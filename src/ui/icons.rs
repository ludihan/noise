//! Simple icons drawn with lines and shapes, for buttons that would
//! otherwise need symbols or emoji from a font: arrows, play, stop, loop,
//! refresh, home, a folder, a music note and a bin.

use super::theme;
use eframe::egui::{self, Color32, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};
use std::f32::consts::{PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Icon {
    Up,
    Down,
    Left,
    Right,
    Play,
    /// A play triangle after a bar: play from a line.
    PlayFrom,
    Stop,
    Loop,
    Refresh,
    Home,
    Folder,
    Music,
    Bin,
    Plus,
    Minus,
    /// Two squares, one over the other: a copy.
    Clone,
}

/// Draws `icon` in `rect` with `color`.
pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let c = rect.center();
    let s = rect.width().min(rect.height()) * 0.5;
    let stroke = Stroke::new(1.5, color);
    let p = |x: f32, y: f32| pos2(c.x + x * s, c.y + y * s);
    let triangle = |points: [Pos2; 3]| Shape::convex_polygon(points.to_vec(), color, Stroke::NONE);
    match icon {
        Icon::Up | Icon::Down | Icon::Left | Icon::Right => {
            // An arrow: a shaft and a head, pointing up, then turned.
            let turn = match icon {
                Icon::Up => 0.0,
                Icon::Right => PI / 2.0,
                Icon::Down => PI,
                _ => -PI / 2.0,
            };
            let r = |x: f32, y: f32| {
                let (sin, cos) = turn.sin_cos();
                p(x * cos - y * sin, x * sin + y * cos)
            };
            painter.line_segment([r(0.0, 0.75), r(0.0, -0.2)], stroke);
            painter.add(triangle([r(0.0, -0.8), r(-0.6, -0.1), r(0.6, -0.1)]));
        }
        Icon::Play => {
            painter.add(triangle([p(-0.55, -0.7), p(-0.55, 0.7), p(0.7, 0.0)]));
        }
        Icon::PlayFrom => {
            painter.rect_filled(Rect::from_min_max(p(-0.75, -0.7), p(-0.5, 0.7)), 0.0, color);
            painter.add(triangle([p(-0.25, -0.7), p(-0.25, 0.7), p(0.85, 0.0)]));
        }
        Icon::Stop => {
            painter.rect_filled(Rect::from_center_size(c, Vec2::splat(s * 1.2)), 1.0, color);
        }
        Icon::Loop | Icon::Refresh => {
            // Most of a circle, with an arrowhead at its end.
            let r = s * 0.7;
            let (from, to) = if icon == Icon::Loop { (0.15 * TAU, 0.95 * TAU) } else { (-0.2 * TAU, 0.6 * TAU) };
            let points: Vec<Pos2> = (0..=20)
                .map(|k| from + (to - from) * k as f32 / 20.0)
                .map(|a| c + vec2(a.cos(), a.sin()) * r)
                .collect();
            let end = *points.last().unwrap();
            painter.add(Shape::line(points, stroke));
            // The head points along the circle at its end.
            let dir = vec2(-to.sin(), to.cos());
            let side = vec2(to.cos(), to.sin());
            let h = s * 0.45;
            painter.add(triangle([end + dir * h, end + side * h * 0.7, end - side * h * 0.7]));
        }
        Icon::Home => {
            // A roof over a box.
            painter.add(Shape::line(vec![p(-0.85, -0.05), p(0.0, -0.85), p(0.85, -0.05)], stroke));
            painter.add(Shape::line(vec![p(-0.55, -0.3), p(-0.55, 0.75), p(0.55, 0.75), p(0.55, -0.3)], stroke));
        }
        Icon::Folder => {
            // A box with a tab on its top left.
            let body = Rect::from_min_max(p(-0.85, -0.45), p(0.85, 0.7));
            painter.rect_stroke(body, 1.0, stroke, egui::StrokeKind::Middle);
            painter.add(Shape::line(vec![p(-0.85, -0.45), p(-0.85, -0.7), p(-0.2, -0.7), p(0.0, -0.45)], stroke));
        }
        Icon::Music => {
            // An eighth note: a head, a stem and a flag.
            painter.circle_filled(p(-0.3, 0.5), s * 0.3, color);
            painter.line_segment([p(-0.03, 0.5), p(-0.03, -0.8)], stroke);
            painter.add(Shape::line(vec![p(-0.03, -0.8), p(0.55, -0.45), p(0.55, -0.1)], stroke));
        }
        Icon::Bin => {
            // A lid, a handle and a body with ribs.
            painter.line_segment([p(-0.8, -0.55), p(0.8, -0.55)], stroke);
            painter.add(Shape::line(vec![p(-0.25, -0.55), p(-0.25, -0.8), p(0.25, -0.8), p(0.25, -0.55)], stroke));
            painter.add(Shape::line(vec![p(-0.6, -0.55), p(-0.45, 0.8), p(0.45, 0.8), p(0.6, -0.55)], stroke));
            painter.line_segment([p(0.0, -0.3), p(0.0, 0.55)], Stroke::new(1.0, color));
        }
        Icon::Plus | Icon::Minus => {
            painter.line_segment([p(-0.6, 0.0), p(0.6, 0.0)], stroke);
            if icon == Icon::Plus {
                painter.line_segment([p(0.0, -0.6), p(0.0, 0.6)], stroke);
            }
        }
        Icon::Clone => {
            let square = |x: f32, y: f32| Rect::from_min_max(p(x - 0.6, y - 0.6), p(x + 0.25, y + 0.25));
            painter.rect_stroke(square(0.0, 0.0), 1.0, stroke, egui::StrokeKind::Middle);
            painter.rect_filled(square(0.35, 0.35), 1.0, color);
        }
    }
}

/// A small button with `icon`, framed like the others.
pub fn button(ui: &mut Ui, icon: Icon) -> Response {
    sized_button(ui, icon, Vec2::splat(ui.spacing().interact_size.y), false)
}

/// A button with `icon` of `size`, lit in the selected color when `on`.
pub fn sized_button(ui: &mut Ui, icon: Icon, size: Vec2, on: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let visuals = ui.style().interact_selectable(&resp, on);
    let (fill, color) =
        if on { (theme::SELECTED, theme::SELECTED_TEXT) } else { (visuals.weak_bg_fill, visuals.fg_stroke.color) };
    let color = if ui.is_enabled() { color } else { color.gamma_multiply(0.4) };
    ui.painter().rect(rect, 2.0, fill, visuals.bg_stroke, egui::StrokeKind::Inside);
    let side = (size.y * 0.62).min(size.x * 0.62);
    paint(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(side)), icon, color);
    resp
}
