//! Pictures of what LFOs and Modulators do: an LFO's cycle with a dot
//! where it is, what a follower puts out over the last few seconds, and
//! small shapes for menus.

use super::theme;
use crate::project::interpolate;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};

/// Where in its cycle an LFO of `shape` (`MODULATOR_SHAPES`) is at `phase`
/// (0..1), -1..1, with `drawn` the points of a drawn shape.
pub fn shape_at(shape: u32, drawn: &[(f32, f32)], phase: f32) -> f32 {
    if shape == crate::project::DRAWN_SHAPE {
        let points = if drawn.is_empty() { &crate::project::DEFAULT_SHAPE[..] } else { drawn };
        interpolate(points, phase, false, false).map_or(0.0, |v| 2.0 * v - 1.0)
    } else {
        crate::dsp::lfo_shape(shape, phase)
    }
}

/// The frame a picture is drawn in, `h` high across the width there is,
/// with its middle line.
fn frame(ui: &mut egui::Ui, h: f32) -> (Rect, egui::Response, egui::Painter) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, theme::INSET);
    for t in [0.25, 0.5, 0.75] {
        let x = rect.left() + t * rect.width();
        painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], (1.0, Color32::from_gray(30)));
    }
    let y = rect.center().y;
    painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], (1.0, Color32::from_gray(44)));
    (rect, resp, painter)
}

/// One cycle of `wave` (phase 0..1 to -1..1) left to right, scaled by
/// `scale` (-1..1, upside down below zero), and while it runs a line and
/// a dot where it is now, at `phase`. `about` is written in a corner.
pub fn cycle(ui: &mut egui::Ui, h: f32, wave: &dyn Fn(f32) -> f32, scale: f32, phase: Option<f32>, about: &str) {
    let (rect, resp, painter) = frame(ui, h);
    let inner = rect.shrink2(Vec2::new(0.0, 5.0));
    let x_of = |p: f32| inner.left() + p * inner.width();
    let y_of = |v: f32| inner.center().y - (v * scale).clamp(-1.0, 1.0) * inner.height() / 2.0;
    let steps = (inner.width() as usize).clamp(32, 400);
    let line: Vec<Pos2> = (0..=steps)
        .map(|k| {
            let p = k as f32 / steps as f32;
            // A jump at the very end of a cycle is drawn at its end.
            let p = if k == steps { 1.0 - 1e-4 } else { p };
            Pos2::new(x_of(p), y_of(wave(p)))
        })
        .collect();
    // The area under the curve, faintly, then the curve.
    let mid = inner.center().y;
    for pair in line.windows(2) {
        let quad = vec![pair[0], pair[1], Pos2::new(pair[1].x, mid), Pos2::new(pair[0].x, mid)];
        painter.add(egui::Shape::convex_polygon(quad, theme::SELECTED.gamma_multiply(0.12), Stroke::NONE));
    }
    painter.add(egui::Shape::line(line, Stroke::new(1.5, theme::SELECTED)));
    if let Some(p) = phase {
        let at = Pos2::new(x_of(p), y_of(wave(p)));
        painter.line_segment([Pos2::new(at.x, rect.top()), Pos2::new(at.x, rect.bottom())], (1.0, theme::SCOPE));
        painter.circle_filled(at, 3.5, Color32::WHITE);
    }
    corner(&painter, rect, about);
    resp.on_hover_text("One cycle of the shape, as far as it swings; the dot is where it is now");
}

/// What a follower or an envelope has put out lately, -1..1 scaled by
/// `scale`, as a trace running right to left; the window keeps the trace
/// under `id`.
pub fn trace(ui: &mut egui::Ui, h: f32, id: egui::Id, value: f32, scale: f32, about: &str) {
    const LEN: usize = 180;
    let history = ui.data_mut(|d| {
        let history = d.get_temp_mut_or_insert_with(id, || vec![0f32; LEN]);
        history.rotate_left(1);
        history[LEN - 1] = value;
        history.clone()
    });
    let (rect, resp, painter) = frame(ui, h);
    let inner = rect.shrink2(Vec2::new(0.0, 5.0));
    let line: Vec<Pos2> = (history.iter().enumerate())
        .map(|(k, v)| {
            let x = inner.left() + k as f32 / (LEN - 1) as f32 * inner.width();
            Pos2::new(x, inner.center().y - (v * scale).clamp(-1.0, 1.0) * inner.height() / 2.0)
        })
        .collect();
    let now = *line.last().unwrap();
    painter.add(egui::Shape::line(line, Stroke::new(1.5, theme::SELECTED)));
    painter.circle_filled(now, 3.5, Color32::WHITE);
    corner(&painter, rect, about);
    resp.on_hover_text("What it has put out over the last few seconds, newest on the right");
}

fn corner(painter: &egui::Painter, rect: Rect, text: &str) {
    painter.text(
        rect.left_top() + Vec2::new(4.0, 2.0),
        egui::Align2::LEFT_TOP,
        text,
        egui::FontId::proportional(10.5),
        theme::TEXT_WEAK,
    );
}

/// A small picture of `wave` for a menu or a label, in `rect`.
pub fn icon(painter: &egui::Painter, rect: Rect, wave: &dyn Fn(f32) -> f32, color: Color32) {
    let line: Vec<Pos2> = (0..=24)
        .map(|k| {
            let p = if k == 24 { 1.0 - 1e-4 } else { k as f32 / 24.0 };
            Pos2::new(rect.left() + p * rect.width(), rect.center().y - wave(p) * rect.height() / 2.0)
        })
        .collect();
    painter.add(egui::Shape::line(line, Stroke::new(1.2, color)));
}

/// A menu button for `label` with a small picture of `wave` before it.
pub fn shape_button(ui: &mut egui::Ui, label: &str, selected: bool, wave: &dyn Fn(f32) -> f32) -> egui::Response {
    let resp = ui.add(egui::Button::selectable(selected, format!("        {label}")));
    let r = Rect::from_min_size(resp.rect.left_center() + Vec2::new(5.0, -5.0), Vec2::new(22.0, 10.0));
    let color = if selected { theme::SELECTED_TEXT } else { theme::SELECTED };
    icon(ui.painter(), r, wave, color);
    resp
}
