//! The instrument list: every module that plays notes, by number.

use super::{App, theme};
use crate::project::{ModuleKind, OUTPUT_ID};
use eframe::egui::{self, Key, RichText};

/// State of the instrument list between frames.
#[derive(Default)]
pub struct InstrumentList {
    /// Instrument being renamed and the name typed so far.
    renaming: Option<(u8, String)>,
}

enum Action {
    StartRename,
    FinishRename,
    CancelRename,
    ToggleMute,
    ToggleSolo,
    Duplicate,
    Delete,
}

fn instruments(app: &App) -> Vec<u8> {
    let mut ids: Vec<u8> = app.project.modules.iter().filter(|m| m.kind.is_instrument()).map(|m| m.id).collect();
    ids.sort();
    ids
}

/// Adds an instrument wired to the output and selects it. A MultiSynth is wired to the instrument
/// that was selected instead.
pub fn add(app: &mut App, kind: ModuleKind) {
    let before = app.instrument();
    let y = app.project.modules.iter().filter(|m| m.kind.is_instrument()).map(|m| m.pos[1] + 70.0).fold(20.0, f32::max);
    if let Some(id) = app.project.add_module(kind, [40.0, y]) {
        match before {
            Some(target) if kind.notes_only() => app.project.connect(id, target),
            _ => app.project.connect(id, OUTPUT_ID),
        };
        app.selected_module = Some(id);
        app.mark();
    } else {
        app.set_status("Too many modules");
    }
}

/// Copies an instrument and its outgoing connections.
fn duplicate(app: &mut App, id: u8) {
    let Some(pos) = app.project.module(id).map(|m| m.pos) else { return };
    let Some(new) = app.project.duplicate_module(id, [pos[0] + 20.0, pos[1] + 70.0]) else {
        app.set_status("Too many modules");
        return;
    };
    let targets: Vec<u8> = app.project.links.iter().filter(|l| l.0 == id).map(|l| l.1).collect();
    for t in targets {
        app.project.connect(new, t);
    }
    app.selected_module = Some(new);
    app.mark();
}

fn delete(app: &mut App, id: u8) {
    let list = instruments(app);
    let pos = list.iter().position(|&i| i == id).unwrap_or(0);
    app.project.remove_module(id);
    let rest = instruments(app);
    app.selected_module = rest.get(pos.min(rest.len().saturating_sub(1))).copied();
    app.mark();
}

/// Selects the instrument `delta` places away from the current one.
pub fn step(app: &mut App, delta: i32) {
    let list = instruments(app);
    if list.is_empty() {
        return;
    }
    let cur = app.instrument().and_then(|id| list.iter().position(|&i| i == id));
    let next = match cur {
        Some(i) => (i as i32 + delta).clamp(0, list.len() as i32 - 1) as usize,
        None => 0,
    };
    app.selected_module = Some(list[next]);
}

pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        theme::caption(ui, "INSTRUMENTS");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let selected = app.instrument();
            if ui
                .add_enabled(selected.is_some(), egui::Button::new("−").small())
                .on_hover_text("Delete the selected instrument")
                .clicked()
            {
                delete(app, selected.unwrap());
            }
            if ui
                .add_enabled(selected.is_some(), egui::Button::new("Dup").small())
                .on_hover_text("Duplicate the selected instrument")
                .clicked()
            {
                duplicate(app, selected.unwrap());
            }
            ui.menu_button("+", |ui| {
                for kind in ModuleKind::ADDABLE.into_iter().filter(|k| k.is_instrument()) {
                    if ui.button(kind.name()).clicked() {
                        add(app, kind);
                        ui.close();
                    }
                }
                ui.separator();
                super::library::add_menus(app, ui);
            })
            .response
            .on_hover_text("Add an instrument");
        });
    });

    let list = instruments(app);
    let mut select = None;
    let mut action = None;
    egui::Frame::new().fill(theme::INSET).inner_margin(2).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                for &id in &list {
                    let Some(m) = app.project.module(id) else { continue };
                    if let Some((rid, name)) = &mut app.instrument_list.renaming
                        && *rid == id
                    {
                        let edit = ui.add(egui::TextEdit::singleline(name));
                        if !edit.has_focus() && !edit.lost_focus() {
                            edit.request_focus();
                        }
                        if edit.lost_focus() {
                            let keep = !ui.input(|i| i.key_pressed(Key::Escape));
                            action = Some((id, if keep { Action::FinishRename } else { Action::CancelRename }));
                        }
                        continue;
                    }
                    let selected = app.selected_module == Some(id);
                    let mut text = RichText::new(format!("{:02X}  {}", id, m.name));
                    if m.mute {
                        text = text.strikethrough();
                    }
                    let kind = RichText::new(m.kind.name()).small().color(if selected {
                        theme::SELECTED_TEXT
                    } else {
                        theme::TEXT_WEAK
                    });
                    // The row, with mute and solo at its right.
                    let (muted, soloed, sounds) = (m.mute, m.solo, m.kind.makes_sound());
                    let resp = ui
                        .horizontal(|ui| {
                            let w = ui.available_width() - 60.0;
                            let row =
                                ui.add_sized([w, 18.0], egui::Button::selectable(selected, text).right_text(kind));
                            if theme::toggle(ui, muted, "M").on_hover_text("Mute").clicked() {
                                action = Some((id, Action::ToggleMute));
                            }
                            if sounds
                                && theme::toggle(ui, soloed, "S")
                                    .on_hover_text("Solo: hear only this, and what it goes through")
                                    .clicked()
                            {
                                action = Some((id, Action::ToggleSolo));
                            }
                            row
                        })
                        .inner;
                    if resp.clicked() {
                        select = Some(id);
                    }
                    if resp.double_clicked() {
                        action = Some((id, Action::StartRename));
                    }
                    let muted = m.mute;
                    resp.context_menu(|ui| {
                        if ui.button("Rename").clicked() {
                            action = Some((id, Action::StartRename));
                            ui.close();
                        }
                        if ui.button(if muted { "Unmute" } else { "Mute" }).clicked() {
                            action = Some((id, Action::ToggleMute));
                            ui.close();
                        }
                        if sounds && ui.button(if soloed { "Unsolo" } else { "Solo" }).clicked() {
                            action = Some((id, Action::ToggleSolo));
                            ui.close();
                        }
                        if ui.button("Duplicate").clicked() {
                            action = Some((id, Action::Duplicate));
                            ui.close();
                        }
                        if ui.button("Delete").clicked() {
                            action = Some((id, Action::Delete));
                            ui.close();
                        }
                        if sounds {
                            ui.separator();
                            super::library::save_menu(app, ui, id);
                        }
                    });
                }
                if list.is_empty() {
                    ui.label(RichText::new("No instruments. Press + to add one.").color(theme::TEXT_WEAK));
                }
            });
        });
    });

    if let Some(id) = select {
        app.selected_module = Some(id);
    }
    let Some((id, action)) = action else { return };
    match action {
        Action::StartRename => {
            let name = app.project.module(id).map(|m| m.name.clone()).unwrap_or_default();
            app.instrument_list.renaming = Some((id, name));
            app.selected_module = Some(id);
        }
        Action::FinishRename => {
            if let Some((_, name)) = app.instrument_list.renaming.take()
                && let Some(m) = app.project.module_mut(id)
            {
                m.name = name;
                app.mark_layout();
            }
        }
        Action::CancelRename => app.instrument_list.renaming = None,
        Action::ToggleMute => {
            if let Some(m) = app.project.module_mut(id) {
                m.mute = !m.mute;
            }
            app.mark();
        }
        Action::ToggleSolo => {
            if let Some(m) = app.project.module_mut(id) {
                m.solo = !m.solo;
            }
            app.mark();
        }
        Action::Duplicate => duplicate(app, id),
        Action::Delete => delete(app, id),
    }
}
