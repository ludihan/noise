//! Widgets the panels share: parameter bars with the name and value written
//! inside, chips to switch things, and framed boxes.

use super::theme;
use crate::project::ParamSpec;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, Vec2};

const BAR_H: f32 = 18.0;

/// A parameter as a horizontal bar. Drag to change it (Shift for fine
/// steps), double-click to reset it and right-click to type a value, with
/// `more` adding to that menu. Choices open a menu instead.
pub fn param_bar(ui: &mut Ui, spec: &ParamSpec, value: &mut f32, more: impl FnOnce(&mut Ui)) -> Response {
    named_bar(ui, spec, spec.name, value, more)
}

/// `param_bar` under a name of its own, as a macro's.
pub fn named_bar(ui: &mut Ui, spec: &ParamSpec, name: &str, value: &mut f32, more: impl FnOnce(&mut Ui)) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), BAR_H), Sense::click_and_drag());
    let old = *value;
    if spec.choices.is_empty() {
        drag(ui, &resp, rect, spec, value);
        if resp.double_clicked() {
            *value = spec.default;
        }
        field_context_menu(&resp, |ui| {
            ui.label(egui::RichText::new(name).color(theme::SELECTED));
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
    draw(ui, rect, &resp, spec, name, *value);
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

fn draw(ui: &Ui, rect: Rect, resp: &Response, spec: &ParamSpec, name: &str, value: f32) {
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
    painter.text(Pos2::new(rect.left() + 6.0, y), Align2::LEFT_CENTER, name, font, theme::TEXT);
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

/// A chip in `color` with `text`: lit when `on`, dim otherwise.
pub fn chip(ui: &mut egui::Ui, color: Color32, text: &str, on: bool) -> egui::Response {
    let font = egui::FontId::proportional(12.0);
    let text_color = if on { theme::SELECTED_TEXT } else { theme::TEXT };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, text_color);
    let size = galley.size() + Vec2::new(16.0, 6.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let fill = if on { color } else { color.gamma_multiply(if resp.hovered() { 0.5 } else { 0.3 }) };
    ui.painter().rect_filled(rect, 9.0, fill);
    if on {
        ui.painter().rect_stroke(rect, 9.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Inside);
    }
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, text_color);
    resp
}

/// Allocates `size` and draws a framed box around `add`.
/// `id` keeps the widgets of different boxes apart.
pub fn boxed<R>(ui: &mut egui::Ui, id: &str, size: Vec2, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect(rect, 3.0, theme::FRAME_BG, Stroke::new(1.0, theme::FRAME_LINE), egui::StrokeKind::Inside);
    let inner = rect.shrink(5.0);
    let layout = egui::Layout::top_down(egui::Align::Min);
    let mut child = ui.new_child(egui::UiBuilder::new().id_salt(id).max_rect(inner).layout(layout));
    child.shrink_clip_rect(inner);
    add(&mut child)
}

/// Menus with a field to type in: a click inside one, on the field or its
/// scroll bar, leaves it open. Whatever picks something closes it itself.
fn field_menu_config() -> egui::containers::menu::MenuConfig {
    egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
}

/// A menu button for a menu with a field in it, or in one of its submenus.
pub fn field_menu<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> Response {
    egui::containers::menu::MenuButton::new(label).config(field_menu_config()).ui(ui, add).0
}

/// `resp.context_menu`, for a menu with a field in it, or in one of its
/// submenus.
pub fn field_context_menu(resp: &Response, add: impl FnOnce(&mut Ui)) {
    let config = field_menu_config();
    egui::Popup::context_menu(resp)
        .close_behavior(config.close_behavior)
        .info(
            egui::UiStackInfo::new(egui::UiKind::Menu)
                .with_tag_value(egui::containers::menu::MenuConfig::MENU_CONFIG_TAG, config),
        )
        .show(add);
}

/// Content of a menu that may be longer than the window: it scrolls past
/// most of the window's height rather than running off it.
pub fn menu_scroll<R>(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, add: impl FnOnce(&mut Ui) -> R) -> R {
    let max = (ui.ctx().content_rect().height() * 0.6).max(120.0);
    egui::ScrollArea::vertical().id_salt(id).max_height(max).show(ui, add).inner
}

/// A list of `items` in a menu, as (name, a note on the right), scrolling
/// when long, with a field to find one by name once there are more than a
/// few. Returns the one clicked, or the first one found when Enter is
/// pressed in the field.
pub fn menu_list(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    hint: &str,
    items: &[(String, String)],
) -> Option<usize> {
    let id = egui::Id::new(id);
    let mut find: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let mut picked = None;
    if items.len() > 12 {
        let field = ui.add(egui::TextEdit::singleline(&mut find).hint_text(hint).desired_width(180.0));
        if !field.has_focus() && find.is_empty() {
            field.request_focus();
        }
        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            picked = Some(None);
        }
    }
    let wanted = find.trim().to_lowercase();
    let found: Vec<usize> = (0..items.len()).filter(|&i| items[i].0.to_lowercase().contains(&wanted)).collect();
    if picked == Some(None) {
        picked = Some(found.first().copied());
    }
    menu_scroll(ui, id, |ui| {
        if found.is_empty() {
            ui.label(egui::RichText::new("Nothing by that name").color(theme::TEXT_WEAK));
        }
        for &i in &found {
            let (name, note) = &items[i];
            let note = egui::RichText::new(note).small().color(theme::TEXT_WEAK);
            if ui.add(egui::Button::new(name).right_text(note)).clicked() {
                picked = Some(Some(i));
            }
        }
    });
    let picked = picked.flatten();
    if picked.is_some() {
        find.clear();
    }
    ui.data_mut(|d| d.insert_temp(id, find));
    picked
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;

    #[test]
    fn long_menu_lists_stay_inside_the_window() {
        let items: Vec<(String, String)> = (0..100).map(|i| (format!("Preset {i}"), "Kind".to_string())).collect();
        let height = std::rc::Rc::new(std::cell::Cell::new(0.0));
        let seen = height.clone();
        let mut harness = Harness::builder().with_size([300.0, 600.0]).build_ui(move |ui| {
            menu_list(ui, "test_list", "Find", &items);
            seen.set(ui.min_rect().height());
        });
        harness.run_steps(2);
        assert!(height.get() > 100.0 && height.get() < 600.0, "{}", height.get());
    }

    #[test]
    fn clicking_a_menus_field_leaves_it_open() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;
        let items: Vec<(String, String)> = (0..20).map(|i| (format!("Preset {i}"), String::new())).collect();
        let mut harness = Harness::builder().with_size([300.0, 600.0]).build_ui(move |ui| {
            field_menu(ui, "Open", |ui| menu_list(ui, "test_find", "Find", &items));
        });
        harness.get_by_label("Open").click();
        harness.run();
        assert!(harness.query_by_label_contains("Preset 3").is_some(), "the menu opens");
        harness.get_by_role(Role::TextInput).click();
        harness.run();
        assert!(harness.query_by_label_contains("Preset 3").is_some(), "the menu is still open");
    }
}
