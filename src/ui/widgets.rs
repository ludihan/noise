//! Parameter widgets: a bar with the name and value written inside.

use super::theme;
use crate::project::ParamSpec;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, Vec2};

const BAR_H: f32 = 18.0;

/// A parameter as a horizontal bar. Drag to change it (Shift for fine
/// steps), double-click to reset it and right-click to type a value, with
/// `more` adding to that menu. Choices open a menu instead.
pub fn param_bar(ui: &mut Ui, spec: &ParamSpec, value: &mut f32, more: impl FnOnce(&mut Ui)) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), BAR_H), Sense::click_and_drag());
    let old = *value;
    if spec.choices.is_empty() {
        drag(ui, &resp, rect, spec, value);
        if resp.double_clicked() {
            *value = spec.default;
        }
        resp.context_menu(|ui| {
            ui.label(egui::RichText::new(spec.name).color(theme::SELECTED));
            ui.horizontal(|ui| {
                let speed = (spec.max - spec.min) / 400.0;
                let mut field = egui::DragValue::new(value).range(spec.min..=spec.max).speed(speed);
                if spec.integer {
                    field = field.fixed_decimals(0);
                }
                ui.add(field);
                ui.label(egui::RichText::new(spec.format(*value)).color(theme::TEXT_WEAK));
            });
            if ui.button("Reset to default").clicked() {
                *value = spec.default;
                ui.close();
            }
            more(ui);
        });
    } else {
        egui::Popup::menu(&resp).width(rect.width()).show(|ui| {
            for (i, label) in spec.choices.iter().enumerate() {
                if ui.add(egui::Button::selectable(value.round() as usize == i, *label)).clicked() {
                    *value = i as f32;
                }
            }
        });
    }
    draw(ui, rect, &resp, spec, *value);
    if *value != old {
        resp.mark_changed();
    }
    resp
}

/// Relative dragging. The unrounded position is kept while dragging so
/// whole-number parameters still move with small steps.
fn drag(ui: &Ui, resp: &Response, rect: Rect, spec: &ParamSpec, value: &mut f32) {
    if resp.drag_started() {
        ui.data_mut(|d| d.insert_temp(resp.id, spec.position(*value)));
    }
    if resp.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let dx = resp.drag_delta().x / rect.width() * if fine { 0.1 } else { 1.0 };
        let t = ui.data_mut(|d| {
            let t = d.get_temp_mut_or(resp.id, spec.position(*value));
            *t = (*t + dx).clamp(0.0, 1.0);
            *t
        });
        *value = spec.value_at(t);
    }
}

fn draw(ui: &Ui, rect: Rect, resp: &Response, spec: &ParamSpec, value: f32) {
    let painter = ui.painter();
    let hot = resp.hovered() || resp.dragged();
    painter.rect_filled(rect, 2.0, theme::INSET);
    let y = rect.center().y;
    if spec.choices.is_empty() {
        let t = spec.position(value);
        let x = |t: f32| rect.left() + t * rect.width();
        let bar = if hot { theme::BAR_HOT } else { theme::BAR };
        // Ranges around zero, like pan and detune, fill from the middle,
        // which is marked on the edges.
        let zero = if spec.min < 0.0 && spec.max > 0.0 { spec.position(0.0) } else { 0.0 };
        if zero > 0.0 {
            for (a, b) in [(rect.top(), rect.top() + 3.0), (rect.bottom() - 3.0, rect.bottom())] {
                painter.line_segment([Pos2::new(x(zero), a), Pos2::new(x(zero), b)], (1.0, bar));
            }
        }
        painter.rect_filled(Rect::from_x_y_ranges(x(zero.min(t))..=x(zero.max(t)), rect.y_range()), 2.0, bar);
    } else {
        // A small arrow says a menu opens.
        let at = Pos2::new(rect.right() - 9.0, y);
        let arrow = vec![at + Vec2::new(-4.0, -2.0), at + Vec2::new(4.0, -2.0), at + Vec2::new(0.0, 3.0)];
        painter.add(Shape::convex_polygon(arrow, theme::TEXT_WEAK, Stroke::NONE));
    }
    let line = if hot { Color32::from_gray(110) } else { theme::FRAME_LINE };
    painter.rect_stroke(rect, 2.0, Stroke::new(1.0, line), StrokeKind::Inside);
    let font = FontId::proportional(12.0);
    painter.text(Pos2::new(rect.left() + 6.0, y), Align2::LEFT_CENTER, spec.name, font, theme::TEXT);
    let right = if spec.choices.is_empty() { 6.0 } else { 18.0 };
    let value_font = FontId::monospace(11.5);
    painter.text(
        Pos2::new(rect.right() - right, y),
        Align2::RIGHT_CENTER,
        spec.format(value),
        value_font,
        Color32::WHITE,
    );
}
