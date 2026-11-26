use crate::theme::*;
use eframe::egui::text::{LayoutJob, TextWrapping};
use eframe::egui::{
    Align2, Color32, CursorIcon, FontId, Id, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};

pub fn card(p: &Painter, r: Rect) {
    p.rect_filled(r, 14.0, CARD);
    p.rect_stroke(r, 14.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
}

pub fn measure(p: &Painter, s: &str, font: FontId) -> Vec2 {
    p.layout_no_wrap(s.to_owned(), font, Color32::WHITE).size()
}

pub fn text(p: &Painter, pos: Pos2, s: impl ToString, font: FontId, color: Color32) -> Rect {
    p.text(pos, Align2::LEFT_TOP, s, font, color)
}

pub fn text_right(p: &Painter, right_top: Pos2, s: impl ToString, font: FontId, color: Color32) -> Rect {
    p.text(right_top, Align2::RIGHT_TOP, s, font, color)
}

pub fn text_center(p: &Painter, r: Rect, s: impl ToString, font: FontId, color: Color32) {
    p.text(r.center(), Align2::CENTER_CENTER, s, font, color);
}

/// Single line, cut with an ellipsis at `width`.
pub fn text_clip(p: &Painter, pos: Pos2, width: f32, s: &str, font: FontId, color: Color32) {
    if width <= 4.0 {
        return;
    }
    let mut job = LayoutJob::simple_singleline(s.to_owned(), font, color);
    job.wrap = TextWrapping::truncate_at_width(width);
    let galley = p.layout_job(job);
    p.galley(pos, galley, color);
}

pub fn pill(p: &Painter, pos: Pos2, s: &str, fg: Color32, bg: Color32) -> Rect {
    let size = measure(p, s, semi(11.0)) + vec2(14.0, 6.0);
    let r = Rect::from_min_size(pos, size);
    p.rect_filled(r, size.y / 2.0, bg);
    text_center(p, r, s, semi(11.0), fg);
    r
}

pub fn pill_width(p: &Painter, s: &str) -> f32 {
    measure(p, s, semi(11.0)).x + 14.0
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Primary,
    Secondary,
    Selected,
    Outline(Color32Key),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Color32Key(pub [u8; 4]);

impl From<Color32> for Color32Key {
    fn from(c: Color32) -> Self {
        Self(c.to_array())
    }
}

pub fn button(ui: &Ui, r: Rect, id: impl std::hash::Hash + std::fmt::Debug, label: &str, style: Style) -> bool {
    let resp = ui.interact(r, Id::unique(("button", id)), Sense::click());
    let hover = resp.hovered();
    if hover {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let p = ui.painter();
    match style {
        Style::Primary => {
            p.rect_filled(r, 8.0, if hover { mix(ACCENT, Color32::WHITE, 0.15) } else { ACCENT });
            text_center(p, r, label, semi(14.0), INK);
        }
        Style::Selected => {
            p.rect_filled(r, 8.0, ACCENT);
            text_center(p, r, label, semi(14.0), INK);
        }
        Style::Secondary => {
            p.rect_filled(r, 8.0, mix(ROW, Color32::WHITE, if hover { 0.12 } else { 0.05 }));
            p.rect_stroke(r, 8.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
            text_center(p, r, label, semi(14.0), TEXT);
        }
        Style::Outline(c) => {
            let c = Color32::from_rgba_premultiplied(c.0[0], c.0[1], c.0[2], c.0[3]);
            p.rect_filled(r, 8.0, mix(ROW, Color32::WHITE, if hover { 0.12 } else { 0.0 }));
            p.rect_stroke(r, 8.0, Stroke::new(1.5, c), StrokeKind::Inside);
            text_center(p, r, label, semi(14.0), c);
        }
    }
    resp.clicked()
}

pub fn button_width(p: &Painter, label: &str) -> f32 {
    measure(p, label, semi(14.0)).x + 30.0
}

/// Thick arc from `a0` to `a1` (radians, 0 = 3 o'clock, clockwise).
pub fn arc(p: &Painter, center: Pos2, radius: f32, a0: f32, a1: f32, width: f32, color: Color32) {
    let steps = (((a1 - a0).abs() * radius) / 3.0).ceil().max(2.0) as usize;
    let points: Vec<Pos2> = (0..=steps)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / steps as f32;
            pos2(center.x + radius * a.cos(), center.y + radius * a.sin())
        })
        .collect();
    p.line(points, Stroke::new(width, color));
}
