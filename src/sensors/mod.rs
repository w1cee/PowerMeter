#[cfg(windows)]
mod cpu;
#[cfg(not(windows))]
mod demo;
#[cfg(windows)]
mod gpu;
#[cfg(windows)]
mod pawnio;
#[cfg(windows)]
mod system;

use crate::estimator::Sample;
use crate::settings::Settings;
use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

#[cfg(not(windows))]
pub use demo::spawn;

const AVERAGE_WINDOW: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuVendor {
    Amd,
    Nvidia,
}

#[derive(Clone, Debug)]
pub struct GpuSample {
    pub name: String,
    pub vendor: GpuVendor,
    pub power: Option<f32>,
    pub load: f32,
    pub core_clock: f32,
    pub memory_clock: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Cpu,
    Gpu,
    Memory,
    Storage,
    Board,
    Calibration,
}

#[derive(Clone, Debug)]
pub struct Reading {
    pub component: String,
    pub sensor: String,
    pub watts: f32,
    pub estimated: bool,
    pub kind: Kind,
}

impl Reading {
    pub(crate) fn estimate(component: &str, sensor: String, watts: f32, kind: Kind) -> Self {
        Self { component: component.into(), sensor, watts, estimated: true, kind }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CpuStatus {
    #[default]
    Waiting,
    Ok,
    NoDriver,
    DriverBlocked,
    Unsupported,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub seq: u64,
    pub readings: Vec<Reading>,
    pub avg: Sample,
    pub cpu_name: String,
    pub cpu_status: CpuStatus,
    pub base_clock: f32,
    pub peak_clock: f32,
    pub gpu: Option<GpuSample>,
    pub session_wh: f64,
}

impl Snapshot {
    pub fn wall(&self, settings: &Settings) -> f32 {
        self.avg.total / settings.efficiency()
    }
}

pub struct Shared {
    pub snapshot: Mutex<Snapshot>,
    pub settings: Mutex<Settings>,
    pub reset_average: AtomicBool,
}

#[cfg(windows)]
pub fn spawn(shared: Arc<Shared>, repaint: impl Fn() + Send + 'static) {
    std::thread::Builder::new().name("sampler".into()).spawn(move || run(shared, repaint)).expect("sampler thread");
}

#[cfg(windows)]
fn run(shared: Arc<Shared>, repaint: impl Fn()) {
    use crate::pawnio_setup;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    let vendor = cpu::vendor();
    let cpu_name = cpu::name();
    let base_clock = cpu::base_mhz();
    let mut counters = cpu::CpuCounters::open();
    let mut rapl = cpu::Rapl::open(vendor);
    let mut rapl_retry = Instant::now();
    let mut gpu = gpu::open();
    let estimates = system::estimates();

    let mut window: VecDeque<Sample> = VecDeque::new();
    let mut peak_clock = 0f32;
    let mut session_wh = 0f64;
    let mut last = Instant::now();
    let mut seq = 0u64;

    loop {
        let tick = Instant::now();

        // PawnIO may get installed while we run
        if rapl.is_none() && vendor != cpu::Vendor::Unsupported && rapl_retry.elapsed() > Duration::from_secs(10) {
            rapl = cpu::Rapl::open(vendor);
            rapl_retry = Instant::now();
        }

        let (load, performance) = counters.as_mut().map(|c| c.sample()).unwrap_or((0.0, 0.0));
        let clock = base_clock * performance / 100.0;
        peak_clock = peak_clock.max(clock);

        let cpu_watts = rapl.as_mut().and_then(|r| r.watts());
        let gpu_sample = gpu.as_mut().and_then(|g| g.sample());
        let settings = shared.settings.lock().unwrap().clone();

        let mut readings = Vec::new();
        if let Some(w) = cpu_watts {
            readings.push(Reading {
                component: cpu_name.clone(),
                sensor: "Package (RAPL)".into(),
                watts: w,
                estimated: false,
                kind: Kind::Cpu,
            });
        }
        if let Some(g) = &gpu_sample {
            if let Some(w) = g.power {
                readings.push(Reading {
                    component: g.name.clone(),
                    sensor: "Board power".into(),
                    watts: w,
                    estimated: false,
                    kind: Kind::Gpu,
                });
            }
        }
        readings.extend(estimates.iter().cloned());
        if settings.calibration != 0.0 {
            readings.push(Reading::estimate(
                "Calibration offset",
                "fitted to your wall meter".into(),
                settings.calibration,
                Kind::Calibration,
            ));
        }
        let total = readings.iter().map(|r| r.watts).sum::<f32>().max(0.0);

        if shared.reset_average.swap(false, Ordering::Relaxed) {
            window.clear();
        }
        window.push_back(Sample {
            total,
            cpu: cpu_watts.unwrap_or(0.0),
            clock,
            load,
            gpu_load: gpu_sample.as_ref().map_or(0.0, |g| g.load),
            gpu_memory: gpu_sample.as_ref().map_or(0.0, |g| g.memory_clock),
        });
        while window.len() > AVERAGE_WINDOW {
            window.pop_front();
        }
        let avg = average(&window);

        let now = Instant::now();
        session_wh += (total / settings.efficiency()) as f64 * now.duration_since(last).as_secs_f64() / 3600.0;
        last = now;

        let cpu_status = match (vendor, &rapl, cpu_watts) {
            (cpu::Vendor::Unsupported, _, _) => CpuStatus::Unsupported,
            (_, _, Some(_)) => CpuStatus::Ok,
            (_, Some(_), None) => CpuStatus::Waiting,
            (_, None, _) if pawnio_setup::is_ready() => CpuStatus::DriverBlocked,
            _ => CpuStatus::NoDriver,
        };

        seq += 1;
        *shared.snapshot.lock().unwrap() = Snapshot {
            seq,
            readings,
            avg,
            cpu_name: cpu_name.clone(),
            cpu_status,
            base_clock,
            peak_clock,
            gpu: gpu_sample,
            session_wh,
        };
        repaint();

        std::thread::sleep(Duration::from_secs(1).saturating_sub(tick.elapsed()));
    }
}

pub(crate) fn average(window: &VecDeque<Sample>) -> Sample {
    let n = window.len().max(1) as f32;
    let sum = |f: fn(&Sample) -> f32| window.iter().map(f).sum::<f32>() / n;
    Sample {
        total: sum(|s| s.total),
        cpu: sum(|s| s.cpu),
        clock: sum(|s| s.clock),
        load: sum(|s| s.load),
        gpu_load: sum(|s| s.gpu_load),
        gpu_memory: sum(|s| s.gpu_memory),
    }
}
