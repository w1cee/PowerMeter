use crate::catalog::{self, PlanId, PlanKind};
use crate::display::{self, DisplayInfo};
use crate::estimator::{self, Sample};
use crate::pawnio_setup;
use crate::power::{self, PowerError, PowerPlan};
use crate::sensors::{self, CpuStatus, GpuVendor, Kind, Shared, Snapshot};
use crate::settings::Settings;
use crate::theme::*;
use crate::tray;
use crate::ui::{self, Style};
use crate::win;
use eframe::egui::{self, Color32, Id, Mesh, Pos2, Rect, ScrollArea, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use std::collections::VecDeque;
use std::f32::consts::{FRAC_PI_2, TAU};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const HISTORY: usize = 90;
const STUCK_MEMORY_MHZ: f32 = 1000.0;
const BUSY_GPU_LOAD: f32 = 20.0;

#[derive(Clone)]
enum Action {
    Activate(PlanId),
    Install(PlanId),
    Restore(PlanId),
    Delete(PlanId, String),
    Refresh(u32),
    PrimaryOnly,
    ExtendAll,
    InstallDriver,
    Unlock,
    OpenSettings,
}

#[derive(Clone)]
struct PlanRow {
    id: PlanId,
    name: String,
    description: String,
    kind: PlanKind,
    installed: bool,
    active: bool,
    plan: Option<PowerPlan>,
    forecast: String,
    sign: i8,
}

#[derive(Default)]
struct Job {
    running: Option<String>,
    error: Option<String>,
}

pub struct App {
    shared: Arc<Shared>,
    snap: Snapshot,
    settings: Settings,
    last_seq: u64,
    plans: Vec<PowerPlan>,
    display: DisplayInfo,
    display_at: Instant,
    locked: bool,
    pawnio: Option<(u32, u32)>,
    job: Arc<Mutex<Job>>,
    history: VecDeque<f32>,
    hwnd: Option<win::Window>,
    hidden: bool,
    settings_open: bool,
    draft: Settings,
    meter: f32,
    pending: Option<Action>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_theme(&cc.egui_ctx);
        let settings = Settings::load();
        let shared = Arc::new(Shared {
            snapshot: Mutex::new(Snapshot::default()),
            settings: Mutex::new(settings.clone()),
            reset_average: Default::default(),
        });
        let ctx = cc.egui_ctx.clone();
        sensors::spawn(shared.clone(), move || ctx.request_repaint());
        tray::spawn();

        Self {
            shared,
            snap: Snapshot::default(),
            draft: settings.clone(),
            settings,
            last_seq: 0,
            plans: power::load(),
            display: display::read(),
            display_at: Instant::now(),
            locked: power::locked_scheme().is_some(),
            pawnio: pawnio_setup::installed_version(),
            job: Default::default(),
            history: VecDeque::new(),
            hwnd: None,
            hidden: false,
            settings_open: false,
            meter: 0.0,
            pending: None,
        }
    }

    fn wall(&self) -> f32 {
        self.snap.wall(&self.settings)
    }

    fn pull(&mut self) {
        let snap = self.shared.snapshot.lock().unwrap().clone();
        if snap.seq == self.last_seq {
            return;
        }
        self.last_seq = snap.seq;
        self.snap = snap;
        self.plans = power::load();
        self.locked = power::locked_scheme().is_some();
        self.pawnio = pawnio_setup::installed_version();
        if self.display_at.elapsed() > Duration::from_secs(3) {
            self.display = display::read();
            self.display_at = Instant::now();
        }

        let wall = self.wall();
        self.history.push_back(wall);
        while self.history.len() > HISTORY {
            self.history.pop_front();
        }

        let mut tip = format!("PowerMeter: ≈ {wall:.0} W from the wall");
        if self.settings.station_wh > 0 {
            let h = estimator::runtime_hours(self.settings.station_wh, self.settings.inverter_efficiency, wall);
            tip += &format!("\nStation: ≈ {}", estimator::format_duration(h));
        }
        tray::set_tooltip(tip);
    }

    fn report_job(&mut self) {
        let error = self.job.lock().unwrap().error.take();
        if let Some(e) = error {
            win::error(self.hwnd, "PowerMeter", &e);
        }
    }

    fn run(&mut self, action: Action) {
        let result: Result<(), String> = match action {
            Action::Activate(id) => power::set_active(id).map_err(plan_error).map(|_| {
                self.shared.reset_average.store(true, Ordering::Relaxed);
            }),
            Action::Install(id) => match catalog::find_ours(id) {
                Some(t) => power::install(t).map_err(plan_error),
                None => Ok(()),
            },
            Action::Restore(id) => power::restore(id).map_err(plan_error),
            Action::Delete(id, name) => {
                if win::confirm(self.hwnd, &format!("Delete \"{name}\"?")) {
                    power::delete(id).map_err(plan_error)
                } else {
                    Ok(())
                }
            }
            Action::Refresh(hz) => display::set_refresh(hz),
            Action::PrimaryOnly => display::keep_primary_only(),
            Action::ExtendAll => display::extend_all(),
            Action::InstallDriver => {
                self.background("Downloading PawnIO…", |job| {
                    pawnio_setup::install(|m| job.lock().unwrap().running = Some(m.into()))
                });
                Ok(())
            }
            Action::Unlock => {
                self.background("Removing the policy and running gpupdate…", |_| {
                    power::unlock()?;
                    if power::locked_scheme().is_some() {
                        return Err("The policy came straight back after gpupdate, so it's set in the Group Policy editor.\n\n\
                            Open gpedit.msc → Computer Configuration → Administrative Templates → System → Power Management → \
                            \"Specify a custom active power plan\" → Not Configured."
                            .into());
                    }
                    Ok(())
                });
                Ok(())
            }
            Action::OpenSettings => {
                self.draft = self.settings.clone();
                self.meter = self.wall().round();
                self.settings_open = true;
                Ok(())
            }
        };
        if let Err(e) = result {
            win::error(self.hwnd, "That didn't work", &e);
        }
        self.plans = power::load();
        self.display = display::read();
        self.display_at = Instant::now();
    }

    fn background(&self, message: &str, work: impl FnOnce(&Arc<Mutex<Job>>) -> Result<(), String> + Send + 'static) {
        let job = self.job.clone();
        if job.lock().unwrap().running.is_some() {
            return;
        }
        job.lock().unwrap().running = Some(message.into());
        std::thread::spawn(move || {
            let result = work(&job);
            let mut j = job.lock().unwrap();
            j.running = None;
            j.error = result.err();
        });
    }

    fn status(&self) -> String {
        let s = &self.snap;
        match s.cpu_status {
            CpuStatus::Ok => format!(
                "{}  ·  load {:.0}%  ·  {:.0} MHz (base {:.0}, peak {:.0})",
                s.cpu_name, s.avg.load, s.avg.clock, s.base_clock, s.peak_clock
            ),
            CpuStatus::Waiting => "Warming up the sensors…".into(),
            CpuStatus::NoDriver => "No CPU power reading — PawnIO driver needed (button above)".into(),
            CpuStatus::DriverBlocked => "PawnIO is installed but won't talk to us — a reboot usually fixes it".into(),
            CpuStatus::Unsupported => "This CPU has no RAPL counter we know of, so CPU power isn't counted".into(),
        }
    }

    // ---------------------------------------------------------------- layout

    fn layout(&mut self, ui: &mut Ui) {
        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0.0, BACK);
        let area = full.shrink(10.0);
        let mut top = area.top();

        let running = self.job.lock().unwrap().running.clone();
        let driver_missing = !self.pawnio.is_some_and(|v| v >= (2, 0));
        if driver_missing || running.as_deref().is_some_and(|m| m.contains("PawnIO")) {
            let msg = match (&running, self.pawnio) {
                (Some(m), _) => m.clone(),
                (None, None) => "PawnIO driver isn't installed — no CPU power without it".into(),
                (None, Some((a, b))) => format!("PawnIO {a}.{b} is too old, 2.0+ needed"),
            };
            let label = if self.pawnio.is_some() { "Update" } else { "Download & install" };
            self.banner(
                ui,
                Rect::from_min_size(pos2(area.left(), top), vec2(area.width(), 52.0)),
                &msg,
                label,
                running.is_none(),
                Action::InstallDriver,
            );
            top += 60.0;
        }
        if self.locked || running.as_deref().is_some_and(|m| m.contains("policy")) {
            let msg = running
                .clone()
                .filter(|m| m.contains("policy"))
                .unwrap_or_else(|| "A group policy pins your power plan — Windows won't let anything switch it".into());
            self.banner(
                ui,
                Rect::from_min_size(pos2(area.left(), top), vec2(area.width(), 52.0)),
                &msg,
                "Unlock",
                running.is_none(),
                Action::Unlock,
            );
            top += 60.0;
        }

        let hero = Rect::from_min_max(pos2(area.left(), top), pos2(area.right(), top + 240.0));
        self.hero(ui, hero);
        top = hero.bottom() + 10.0;

        let bottom = Rect::from_min_max(pos2(area.left(), top), area.max);
        let split = bottom.left() + bottom.width() * 0.42;
        let left = Rect::from_min_max(bottom.min, pos2(split - 5.0, bottom.bottom()));
        let right = Rect::from_min_max(pos2(split + 5.0, bottom.top()), bottom.max);
        let gpu_height = 290.0;
        let components = Rect::from_min_max(left.min, pos2(left.right(), left.bottom() - gpu_height - 10.0));
        let gpu = Rect::from_min_max(pos2(left.left(), left.bottom() - gpu_height), left.max);

        self.components(ui, components);
        self.gpu(ui, gpu);
        self.plans(ui, right);
    }

    fn banner(&mut self, ui: &mut Ui, r: Rect, message: &str, label: &str, enabled: bool, action: Action) {
        let p = ui.painter();
        p.rect_filled(r, 12.0, mix(BACK, WARNING, 0.09));
        p.rect_stroke(r, 12.0, Stroke::new(1.0, mix(BACK, WARNING, 0.45)), egui::StrokeKind::Inside);
        p.circle_filled(pos2(r.left() + 22.0, r.center().y), 5.0, WARNING);
        let bw = ui::button_width(p, label);
        let btn = Rect::from_min_size(pos2(r.right() - bw - 12.0, r.center().y - 16.0), vec2(bw, 32.0));
        ui::text_clip(p, pos2(r.left() + 40.0, r.center().y - 10.0), btn.left() - r.left() - 56.0, message, semi(14.0), TEXT);
        if enabled {
            if ui::button(ui, btn, label, label, Style::Primary) {
                self.pending = Some(action);
            }
        } else {
            ui.painter().rect_filled(btn, 8.0, ROW);
            ui::text_center(ui.painter(), btn, "working…", semi(14.0), FAINT);
        }
    }

    // ---------------------------------------------------------------- hero

    fn hero(&mut self, ui: &mut Ui, r: Rect) {
        let p = ui.painter().clone();
        ui::card(&p, r);

        let gear = Rect::from_min_size(pos2(r.right() - 46.0, r.top() + 14.0), vec2(32.0, 32.0));
        let resp = ui.interact(gear, Id::unique("gear"), Sense::click());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        p.rect_filled(gear, 8.0, mix(CARD, Color32::WHITE, if resp.hovered() { 0.1 } else { 0.04 }));
        ui::text_center(&p, gear, "⚙", regular(18.0), if resp.hovered() { TEXT } else { MUTED });
        if resp.clicked() {
            self.pending = Some(Action::OpenSettings);
        }

        let ctx = ui.ctx().clone();
        let total = ctx.animate_value_with_time(Id::unique("total"), self.snap.avg.total, 0.4);
        let mut slices: Vec<(Kind, f32)> = Vec::new();
        for reading in self.snap.readings.iter().filter(|r| r.watts > 0.0 && r.kind != Kind::Calibration) {
            match slices.iter_mut().find(|(k, _)| *k == reading.kind) {
                Some((_, w)) => *w += reading.watts,
                None => slices.push((reading.kind, reading.watts)),
            }
        }
        slices.sort_by_key(|(k, _)| *k);

        // donut
        let d = (r.height() - 44.0).min(180.0);
        let center = pos2(r.left() + 28.0 + d / 2.0, r.center().y);
        let radius = d / 2.0 - 7.0;
        p.circle_stroke(center, radius, Stroke::new(14.0, ROW));
        let sum: f32 = slices.iter().map(|(_, w)| w).sum();
        if sum > 0.0 {
            let mut a = -FRAC_PI_2;
            for (kind, w) in &slices {
                let target = w / sum * TAU;
                let sweep = ctx.animate_value_with_time(Id::unique(("slice", *kind)), target, 0.4);
                if sweep > 0.06 {
                    ui::arc(&p, center, radius, a + 0.025, a + sweep - 0.025, 14.0, kind_color(*kind));
                }
                a += sweep;
            }
        }
        p.text(center - vec2(0.0, 8.0), egui::Align2::CENTER_CENTER, format!("{total:.0}"), semi(50.0), TEXT);
        p.text(center + vec2(0.0, 26.0), egui::Align2::CENTER_CENTER, "watts", regular(12.0), MUTED);

        let spark_w = 300f32.min(r.width() * 0.28);
        let spark = Rect::from_min_size(pos2(r.right() - spark_w - 60.0, r.top() + 20.0), vec2(spark_w, r.height() - 40.0));
        let x = center.x + d / 2.0 + 36.0;
        let max_x = spark.left() - 24.0;
        let mut y = r.top() + 24.0;

        ui::text(&p, pos2(x, y), "POWER DRAW", semi(11.0), MUTED);
        y += 18.0;
        let wall = total / self.settings.efficiency();
        ui::text(&p, pos2(x, y), format!("≈ {wall:.0} W from the wall"), semi(22.0), TEXT);
        y += 34.0;

        let mut meta = format!("PSU efficiency {}%", self.settings.psu_efficiency);
        if self.settings.calibration != 0.0 {
            meta += &format!("  ·  calibration {:+.0} W", self.settings.calibration);
        }
        ui::text_clip(&p, pos2(x, y), max_x - x, &meta, regular(12.0), MUTED);
        y += 26.0;

        let mut cx = x;
        for (kind, w) in &slices {
            let label = format!("{}  {:.0} W", kind_name(*kind), w);
            let width = ui::measure(&p, &label, regular(12.0)).x + 30.0;
            if cx + width > max_x && cx > x {
                cx = x;
                y += 30.0;
            }
            let chip = Rect::from_min_size(pos2(cx, y), vec2(width, 24.0));
            p.rect_filled(chip, 12.0, ROW);
            p.circle_filled(pos2(chip.left() + 13.0, chip.center().y), 4.0, kind_color(*kind));
            ui::text(&p, pos2(chip.left() + 23.0, chip.top() + 4.0), label, regular(12.0), TEXT);
            cx += width + 6.0;
        }
        y += 34.0;

        let kwh = self.snap.session_wh / 1000.0;
        let cost = kwh * self.settings.tariff as f64;
        ui::text_clip(
            &p,
            pos2(x, y),
            max_x - x,
            &format!("This session: {kwh:.3} kWh  ·  ≈ {cost:.2} {}", self.settings.currency),
            regular(12.0),
            MUTED,
        );
        y += 20.0;
        if self.settings.station_wh > 0 && wall > 1.0 {
            let h = estimator::runtime_hours(self.settings.station_wh, self.settings.inverter_efficiency, wall);
            ui::text_clip(
                &p,
                pos2(x, y),
                max_x - x,
                &format!("A {} Wh station lasts ≈ {}", self.settings.station_wh, estimator::format_duration(h)),
                semi(14.0),
                ACCENT,
            );
        }

        ui::text_clip(&p, pos2(x, r.bottom() - 30.0), max_x - x, &self.status(), regular(12.0), FAINT);
        self.spark(&p, spark);
    }

    fn spark(&self, p: &egui::Painter, r: Rect) {
        ui::text(p, r.min + vec2(0.0, 4.0), "FROM THE WALL, LAST 90 S", semi(11.0), MUTED);
        let area = Rect::from_min_max(r.min + vec2(0.0, 24.0), r.max);
        p.rect_filled(area, 12.0, ROW);
        if self.history.len() < 2 {
            return;
        }
        let hi = self.history.iter().cloned().fold(f32::MIN, f32::max);
        let lo = self.history.iter().cloned().fold(f32::MAX, f32::min);
        let (top, mut bottom) = (hi * 1.1, lo * 0.9);
        if hi * 1.1 - bottom < 10.0 {
            bottom = top - 10.0;
        }
        let plot = area.shrink2(vec2(10.0, 14.0));
        let offset = HISTORY - self.history.len();
        let pts: Vec<Pos2> = self
            .history
            .iter()
            .enumerate()
            .map(|(i, v)| {
                pos2(
                    plot.left() + (i + offset) as f32 * plot.width() / (HISTORY - 1) as f32,
                    plot.bottom() - (v - bottom) / (top - bottom) * plot.height(),
                )
            })
            .collect();

        let mut mesh = Mesh::default();
        let fill_top = Color32::from_rgba_unmultiplied(ACCENT.r(), ACCENT.g(), ACCENT.b(), 60);
        for (i, pt) in pts.iter().enumerate() {
            mesh.colored_vertex(*pt, fill_top);
            mesh.colored_vertex(pos2(pt.x, plot.bottom()), Color32::TRANSPARENT);
            if i > 0 {
                let b = (i as u32) * 2;
                mesh.add_triangle(b - 2, b - 1, b);
                mesh.add_triangle(b - 1, b + 1, b);
            }
        }
        p.add(mesh);
        p.line(pts, Stroke::new(2.0, ACCENT));
        ui::text_right(p, pos2(area.right() - 10.0, area.top() + 6.0), format!("max {hi:.0}"), semi(11.0), MUTED);
        ui::text_right(p, pos2(area.right() - 10.0, area.bottom() - 20.0), format!("min {lo:.0}"), semi(11.0), MUTED);
    }

    // ---------------------------------------------------------------- components

    fn components(&mut self, ui: &mut Ui, r: Rect) {
        let p = ui.painter().clone();
        ui::card(&p, r);
        ui::text(&p, r.min + vec2(20.0, 18.0), "Components", semi(17.0), TEXT);
        let sensors = self.snap.readings.iter().filter(|x| !x.estimated).count();
        let estimates = self.snap.readings.len() - sensors;
        ui::text_right(
            &p,
            pos2(r.right() - 20.0, r.top() + 22.0),
            format!("{sensors} measured  ·  {estimates} guessed"),
            regular(12.0),
            MUTED,
        );

        let inner = Rect::from_min_max(r.min + vec2(16.0, 56.0), r.max - vec2(10.0, 10.0));
        let readings = self.snap.readings.clone();
        let max = readings.iter().map(|x| x.watts.abs()).fold(1.0f32, f32::max);
        ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
            ScrollArea::vertical().id_salt("components").auto_shrink([false, false]).show(ui, |ui| {
                if readings.is_empty() {
                    ui.label(egui::RichText::new("Waiting for the first sample…").color(MUTED));
                }
                for reading in &readings {
                    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width() - 8.0, 56.0), Sense::hover());
                    component_row(ui.painter(), row, reading, max);
                }
            });
        });
    }

    // ---------------------------------------------------------------- gpu & displays

    fn gpu(&mut self, ui: &mut Ui, r: Rect) {
        let p = ui.painter().clone();
        ui::card(&p, r);
        let (x, right) = (r.left() + 20.0, r.right() - 20.0);
        let mut y = r.top() + 18.0;
        ui::text(&p, pos2(x, y), "GPU & display", semi(17.0), TEXT);

        let g = self.snap.gpu.clone();
        let display = self.display.clone();
        let state = match &g {
            None => Vram::Unknown,
            Some(g) if g.memory_clock <= 0.0 => Vram::Unknown,
            Some(g) if g.load >= BUSY_GPU_LOAD => Vram::Busy,
            Some(_) if self.snap.avg.gpu_memory > STUCK_MEMORY_MHZ => Vram::Stuck,
            Some(_) => Vram::Idle,
        };
        let (pill, color) = match state {
            Vram::Idle => ("VRAM resting", POSITIVE),
            Vram::Stuck => ("VRAM stuck at max", NEGATIVE),
            Vram::Busy => ("busy", WARNING),
            Vram::Unknown => ("no data", MUTED),
        };
        let pw = ui::pill_width(&p, pill);
        ui::pill(&p, pos2(right - pw, y + 2.0), pill, color, mix(CARD, color, 0.16));

        y += 34.0;
        let name = g.as_ref().map_or("No AMD or NVIDIA card found".to_string(), |g| g.name.clone());
        ui::text_clip(&p, pos2(x, y), right - x, &name, regular(12.0), MUTED);
        y += 22.0;

        let col = (right - x) / 3.0;
        let mem_color = match state {
            Vram::Stuck => NEGATIVE,
            Vram::Idle => POSITIVE,
            _ => TEXT,
        };
        let fmt = |v: Option<f32>, unit: &str| v.map_or("—".to_string(), |v| format!("{v:.0} {unit}"));
        metric(&p, pos2(x, y), "CORE", &fmt(g.as_ref().map(|g| g.core_clock), "MHz"), TEXT);
        metric(&p, pos2(x + col, y), "MEMORY", &fmt(g.as_ref().map(|_| self.snap.avg.gpu_memory), "MHz"), mem_color);
        metric(&p, pos2(x + col * 2.0, y), "LOAD", &fmt(g.as_ref().map(|_| self.snap.avg.gpu_load), "%"), TEXT);
        y += 50.0;

        let sync = match g.as_ref().map(|g| g.vendor) {
            Some(GpuVendor::Nvidia) => "G-SYNC in the NVIDIA app",
            Some(GpuVendor::Amd) => "FreeSync in Adrenalin",
            None => "adaptive sync in your driver",
        };
        let hint = match state {
            Vram::Stuck if display.screens > 1 => "The second screen keeps VRAM awake — try one and watch the watts".to_string(),
            Vram::Stuck if display.hz > 120 => format!("Try 120 Hz and turn on {sync}"),
            Vram::Stuck => format!("Turn on {sync}, close videos and overlays"),
            Vram::Idle => "All good: memory clocks down when idle".into(),
            Vram::Busy => "Card is busy — idle check resumes when the load drops".into(),
            Vram::Unknown => "Memory clock sensor isn't available".into(),
        };
        ui::text_clip(&p, pos2(x, y), right - x, &hint, regular(12.0), if state == Vram::Stuck { WARNING } else { MUTED });
        y += 28.0;

        ui::text(&p, pos2(x, y), "DISPLAYS", semi(11.0), MUTED);
        ui::text(
            &p,
            pos2(x, y + 17.0),
            format!("{} active  ·  primary {}×{}", display.screens, display.width, display.height),
            regular(12.0),
            TEXT,
        );
        let (label, style, action) = if display.screens > 1 {
            ("Keep primary only", Style::Primary, Action::PrimaryOnly)
        } else {
            ("Bring all back", Style::Secondary, Action::ExtendAll)
        };
        let bw = ui::button_width(&p, label);
        if ui::button(ui, Rect::from_min_size(pos2(right - bw, y + 2.0), vec2(bw, 30.0)), "topology", label, style) {
            self.pending = Some(action);
        }
        y += 46.0;

        ui::text(&p, pos2(x, y), "REFRESH RATE", semi(11.0), MUTED);
        y += 20.0;
        let recommended = if state == Vram::Stuck && display.hz > 120 {
            display.rates.iter().copied().filter(|&r| r <= 120).max().unwrap_or(0)
        } else {
            0
        };
        let mut cx = x;
        for &hz in &display.rates {
            let t = format!("{hz} Hz");
            let w = ui::measure(&p, &t, semi(14.0)).x + 24.0;
            if cx + w > right && cx > x {
                cx = x;
                y += 34.0;
            }
            let chip = Rect::from_min_size(pos2(cx, y), vec2(w, 28.0));
            let style = if hz == display.hz {
                Style::Selected
            } else if hz == recommended {
                Style::Outline(POSITIVE.into())
            } else {
                Style::Secondary
            };
            if ui::button(ui, chip, ("hz", hz), &t, style) && hz != display.hz {
                self.pending = Some(Action::Refresh(hz));
            }
            cx += w + 6.0;
        }
    }

    // ---------------------------------------------------------------- plans

    fn plan_rows(&self) -> Vec<PlanRow> {
        let cur = self.plans.iter().find(|p| p.active).cloned();
        let mut installed = self.plans.clone();
        installed.sort_by_key(|p| (!p.active, catalog::kind_of(p)));

        let mut rows: Vec<PlanRow> = installed
            .iter()
            .map(|p| {
                let kind = catalog::kind_of(p);
                let description = match kind {
                    PlanKind::Ours => catalog::find_ours(p.id).map(|t| t.description),
                    PlanKind::Windows => catalog::find_standard(p).map(|s| s.description),
                    PlanKind::Other => None,
                }
                .unwrap_or("Third-party plan: from a driver, a tweaker or you")
                .to_string();
                let (forecast, sign) = self.forecast(cur.as_ref(), p);
                PlanRow {
                    id: p.id,
                    name: p.name.clone(),
                    description,
                    kind,
                    installed: true,
                    active: p.active,
                    plan: Some(p.clone()),
                    forecast,
                    sign,
                }
            })
            .collect();

        for s in catalog::STANDARD {
            if !self.plans.iter().any(|p| catalog::find_standard(p).is_some_and(|x| x.id == s.id)) {
                rows.push(PlanRow {
                    id: s.id,
                    name: s.name.into(),
                    description: s.description.into(),
                    kind: PlanKind::Windows,
                    installed: false,
                    active: false,
                    plan: None,
                    forecast: "add it to see".into(),
                    sign: 0,
                });
            }
        }
        for t in catalog::OURS {
            if !self.plans.iter().any(|p| p.id == t.id) {
                let plan = t.to_plan();
                let (forecast, sign) = self.forecast(cur.as_ref(), &plan);
                rows.push(PlanRow {
                    id: t.id,
                    name: t.name.into(),
                    description: t.description.into(),
                    kind: PlanKind::Ours,
                    installed: false,
                    active: false,
                    plan: Some(plan),
                    forecast,
                    sign,
                });
            }
        }
        rows
    }

    fn forecast(&self, cur: Option<&PowerPlan>, target: &PowerPlan) -> (String, i8) {
        if target.active {
            return ("now".into(), 0);
        }
        let s: &Sample = &self.snap.avg;
        let Some(cur) = cur else { return ("—".into(), 0) };
        if s.total <= 0.0 {
            return ("—".into(), 0);
        }
        let delta = estimator::delta(s, self.snap.base_clock, self.snap.peak_clock, cur, target);
        let pct = delta / s.total * 100.0;
        if pct.abs() < 0.5 {
            return ("≈ 0%".into(), 0);
        }
        let mut text =
            if pct < 0.0 { format!("−{:.0}%  ·  −{:.0} W", -pct, -delta) } else { format!("+{pct:.0}%  ·  +{delta:.0} W") };
        if self.settings.station_wh > 0 {
            let eff = self.settings.efficiency();
            let before = estimator::runtime_hours(self.settings.station_wh, self.settings.inverter_efficiency, s.total / eff);
            let after =
                estimator::runtime_hours(self.settings.station_wh, self.settings.inverter_efficiency, (s.total + delta) / eff);
            let gain = after - before;
            if gain.abs() * 60.0 >= 1.0 {
                text += &if gain > 0.0 {
                    format!("  ·  +{}", estimator::format_duration(gain))
                } else {
                    format!("  ·  −{}", estimator::format_duration(-gain))
                };
            }
        }
        (text, if pct < 0.0 { -1 } else { 1 })
    }

    fn plans(&mut self, ui: &mut Ui, r: Rect) {
        let p = ui.painter().clone();
        ui::card(&p, r);
        ui::text(&p, r.min + vec2(20.0, 18.0), "Power plans", semi(17.0), TEXT);
        let mut lx = r.right() - 20.0;
        for (kind, label) in [(PlanKind::Other, "third-party"), (PlanKind::Ours, "PowerMeter"), (PlanKind::Windows, "Windows")] {
            let tr = ui::text_right(&p, pos2(lx, r.top() + 22.0), label, regular(12.0), MUTED);
            p.circle_filled(pos2(tr.left() - 9.0, tr.center().y), 4.0, plan_color(kind));
            lx = tr.left() - 26.0;
        }

        let rows = self.plan_rows();
        let inner = Rect::from_min_max(r.min + vec2(16.0, 56.0), r.max - vec2(10.0, 10.0));
        let mut clicked = None;
        ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
            ScrollArea::vertical().id_salt("plans").auto_shrink([false, false]).show(ui, |ui| {
                for (title, installed) in [("INSTALLED", true), ("AVAILABLE", false)] {
                    let section: Vec<&PlanRow> = rows.iter().filter(|x| x.installed == installed).collect();
                    if section.is_empty() {
                        continue;
                    }
                    let (head, _) = ui.allocate_exact_size(vec2(ui.available_width() - 8.0, 20.0), Sense::hover());
                    ui::text(ui.painter(), head.min + vec2(2.0, 2.0), title, semi(11.0), MUTED);
                    for row in section {
                        let (rr, _) = ui.allocate_exact_size(vec2(ui.available_width() - 8.0, 84.0), Sense::hover());
                        if let Some(a) = plan_row(ui, rr, row) {
                            clicked = Some(a);
                        }
                    }
                    ui.add_space(6.0);
                }
            });
        });
        if clicked.is_some() {
            self.pending = clicked;
        }
    }

    // ---------------------------------------------------------------- settings

    fn settings_modal(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let raw = self.snap.avg.total - self.settings.calibration;
        let mut save = false;
        let mut close = false;
        let draft = &mut self.draft;
        let meter = &mut self.meter;

        let modal = egui::Modal::new(Id::unique("settings")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading("Settings");
            ui.add_space(6.0);

            section(ui, "Power supply");
            grid_row(ui, "PSU efficiency", |ui| {
                ui.add(egui::DragValue::new(&mut draft.psu_efficiency).range(60..=98).suffix(" %"));
            });
            hint(ui, "At low load it's below the sticker: Bronze ≈ 80%, Gold ≈ 85%, Platinum ≈ 88%.");

            section(ui, "Calibrate against a wall meter");
            grid_row(ui, "Your meter says", |ui| {
                ui.add(egui::DragValue::new(meter).range(0.0..=3000.0).speed(1.0).suffix(" W"));
                if ui.button("Fit").clicked() {
                    let eff = draft.psu_efficiency as f32 / 100.0;
                    draft.calibration = ((*meter * eff - raw) * 10.0).round() / 10.0;
                    draft.calibration = draft.calibration.clamp(-300.0, 300.0);
                }
            });
            grid_row(ui, "Offset", |ui| {
                ui.add(egui::DragValue::new(&mut draft.calibration).range(-300.0..=300.0).speed(0.5).suffix(" W"));
            });
            hint(
                ui,
                "Read your EcoFlow or plug-in meter, type the number, hit Fit. The difference lands in its own \
                      \"Calibration offset\" row — fans, VRM losses, RGB, everything we can't see.",
            );

            section(ui, "Power station");
            grid_row(ui, "Capacity (0 hides it)", |ui| {
                ui.add(egui::DragValue::new(&mut draft.station_wh).range(0..=50000).speed(8.0).suffix(" Wh"));
            });
            grid_row(ui, "Inverter efficiency", |ui| {
                ui.add(egui::DragValue::new(&mut draft.inverter_efficiency).range(60..=98).suffix(" %"));
            });
            hint(ui, "EcoFlow River 2 — 256, River 2 Pro — 768, Delta 2 — 1024, Delta 2 Max — 2048.");

            section(ui, "Electricity");
            grid_row(ui, "Price per kWh", |ui| {
                ui.add(egui::DragValue::new(&mut draft.tariff).range(0.0..=1000.0).speed(0.01).max_decimals(2));
                ui.add(egui::TextEdit::singleline(&mut draft.currency).desired_width(64.0).char_limit(6));
            });

            ui.add_space(8.0);
            ui.checkbox(&mut draft.minimize_to_tray, "Minimize to tray");
            hint(ui, &format!("Saved to {}", Settings::location()));

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(egui::RichText::new("Save").color(INK)).fill(ACCENT)).clicked() {
                    save = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });

        if save {
            match self.draft.save() {
                Ok(_) => {}
                Err(e) => win::error(self.hwnd, "Couldn't save settings", &e),
            }
            self.settings = self.draft.clone();
            *self.shared.settings.lock().unwrap() = self.settings.clone();
            self.shared.reset_average.store(true, Ordering::Relaxed);
            self.settings_open = false;
        }
        if close || modal.should_close() {
            self.settings_open = false;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Vram {
    Unknown,
    Busy,
    Idle,
    Stuck,
}

fn plan_error(e: PowerError) -> String {
    e.to_string()
}

fn plan_color(kind: PlanKind) -> Color32 {
    match kind {
        PlanKind::Windows => ACCENT,
        PlanKind::Ours => POSITIVE,
        PlanKind::Other => MUTED,
    }
}

fn metric(p: &egui::Painter, pos: Pos2, label: &str, value: &str, color: Color32) {
    ui::text(p, pos, label, semi(11.0), MUTED);
    ui::text(p, pos + vec2(0.0, 16.0), value, semi(22.0), color);
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(egui::RichText::new(title).font(semi(15.0)).color(TEXT));
}

fn grid_row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(190.0, 24.0), Sense::hover());
        ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, label, regular(14.0), TEXT);
        add(ui);
    });
}

fn hint(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(regular(12.0)).color(MUTED));
}

fn component_row(p: &egui::Painter, r: Rect, reading: &sensors::Reading, max: f32) {
    let color = kind_color(reading.kind);
    let icon = Rect::from_min_size(r.min + vec2(0.0, 4.0), vec2(40.0, 40.0));
    p.rect_filled(icon, 10.0, ROW);
    ui::text_center(p, icon, kind_short(reading.kind), semi(11.0), color);

    let tx = r.left() + 54.0;
    let value = format!("{:.1} W", reading.watts);
    let vw = ui::measure(p, &value, semi(17.0)).x;
    ui::text_right(p, pos2(r.right(), r.top() + 3.0), &value, semi(17.0), TEXT);

    let tag = if reading.estimated { "guess" } else { "sensor" };
    let tag_color = if reading.estimated { MUTED } else { ACCENT };
    let tw = ui::pill_width(p, tag);
    ui::pill(p, pos2(r.right() - tw, r.top() + 27.0), tag, tag_color, ROW);

    let text_w = r.right() - tx - vw.max(tw) - 12.0;
    ui::text_clip(p, pos2(tx, r.top() + 4.0), text_w, &reading.component, semi(14.0), TEXT);
    ui::text_clip(p, pos2(tx, r.top() + 25.0), text_w, &reading.sensor, regular(12.0), MUTED);

    let bar = Rect::from_min_size(pos2(tx, r.top() + 48.0), vec2(r.right() - tx, 3.0));
    p.rect_filled(bar, 1.5, ROW);
    let frac = (reading.watts.abs() / max).clamp(0.0, 1.0);
    if frac > 0.01 {
        p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * frac, 3.0)), 1.5, color);
    }
}

fn plan_row(ui: &Ui, r: Rect, row: &PlanRow) -> Option<Action> {
    let p = ui.painter();
    let accent = plan_color(row.kind);
    p.rect_filled(r, 12.0, if row.active { mix(ROW, ACCENT, 0.08) } else { ROW });
    p.rect_stroke(r, 12.0, Stroke::new(1.0, if row.active { mix(ROW, ACCENT, 0.6) } else { LINE }), egui::StrokeKind::Inside);
    p.rect_filled(Rect::from_min_size(r.min + vec2(1.0, 18.0), vec2(3.0, r.height() - 36.0)), 1.5, accent);

    let right = r.right() - 14.0;
    let tx = r.left() + 18.0;
    let text_right = r.right() - 230.0;

    let badge = match row.kind {
        PlanKind::Windows => "Windows",
        PlanKind::Ours => "PowerMeter",
        PlanKind::Other => "third-party",
    };
    let br = ui::pill(p, pos2(tx, r.top() + 12.0), badge, accent, mix(ROW, accent, 0.18));
    ui::text_clip(p, pos2(br.right() + 8.0, r.top() + 10.0), text_right - br.right() - 8.0, &row.name, semi(14.0), TEXT);
    ui::text_clip(p, pos2(tx, r.top() + 36.0), text_right - tx, &row.description, regular(12.0), MUTED);
    ui::text_clip(p, pos2(tx, r.top() + 57.0), text_right - tx, &specs(row.plan.as_ref()), regular(12.0), FAINT);

    let fc = match row.sign {
        -1 => POSITIVE,
        1 => NEGATIVE,
        _ => MUTED,
    };
    let fw = ui::pill_width(p, &row.forecast);
    ui::pill(p, pos2(right - fw, r.top() + 12.0), &row.forecast, fc, mix(ROW, fc, 0.14));

    let btn_y = r.bottom() - 40.0;
    if row.active {
        let w = ui::measure(p, "active", semi(14.0)).x + 28.0;
        let a = Rect::from_min_size(pos2(right - w, btn_y), vec2(w, 28.0));
        p.rect_stroke(a, 8.0, Stroke::new(1.0, mix(ROW, ACCENT, 0.6)), egui::StrokeKind::Inside);
        ui::text_center(p, a, "active", semi(14.0), ACCENT);
        return None;
    }

    let label = if row.installed { "Switch" } else { "Add" };
    let w = ui::button_width(p, label);
    let btn = Rect::from_min_size(pos2(right - w, btn_y), vec2(w, 28.0));
    let style = if row.installed { Style::Secondary } else { Style::Primary };
    let mut action = None;
    if ui::button(ui, btn, ("plan", row.id.0), label, style) {
        action = Some(match (row.installed, row.kind) {
            (true, _) => Action::Activate(row.id),
            (false, PlanKind::Ours) => Action::Install(row.id),
            (false, _) => Action::Restore(row.id),
        });
    }

    if row.kind == PlanKind::Ours && row.installed {
        let size = ui::measure(p, "delete", regular(12.0));
        let del = Rect::from_min_size(pos2(btn.left() - size.x - 16.0, btn.center().y - size.y / 2.0), size);
        let resp = ui.interact(del, Id::unique(("delete", row.id.0)), Sense::click());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        ui::text(p, del.min, "delete", regular(12.0), if resp.hovered() { NEGATIVE } else { FAINT });
        if resp.clicked() {
            action = Some(Action::Delete(row.id, row.name.clone()));
        }
    }
    action
}

fn specs(p: Option<&PowerPlan>) -> String {
    let Some(p) = p else { return "settings show up once it's added".into() };
    let turbo = match (p.turbo_allowed(), p.boost_mode) {
        (false, _) => "turbo off",
        (true, 3 | 4) => "turbo efficient",
        _ => "turbo on",
    };
    let pcie = match p.aspm {
        0 => "PCIe no saving",
        1 => "PCIe moderate saving",
        _ => "PCIe max saving",
    };
    format!("CPU {}–{}%   ·   {turbo}   ·   {pcie}", p.min_state, p.max_state)
}

fn install_theme(ctx: &egui::Context) {
    crate::theme::install(ctx);
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pull();
        self.report_job();
        if self.job.lock().unwrap().running.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }

        let minimized = ctx.input(|i| i.viewport().minimized).unwrap_or(false);
        if minimized && self.settings.minimize_to_tray && !self.hidden && self.hwnd.is_some() {
            self.hidden = true;
            tray::hide_main();
        }
    }

    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        if self.hwnd.is_none() {
            if let Some(window) = win::window_from(frame) {
                win::style_window(window);
                tray::set_main(window);
                self.hwnd = Some(window);
            }
        }
        self.hidden = false;

        self.layout(ui);
        self.settings_modal(ui.ctx());

        if let Some(action) = self.pending.take() {
            self.run(action);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        tray::remove();
    }
}
