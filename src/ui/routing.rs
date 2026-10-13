//! Wiring modules without a graph, for the modules that need more than a
//! chain: the instruments a MultiSynth plays, what a Modulator moves and
//! follows, and what an effect listens to as its key input.

use super::{App, theme};
use crate::project::{ModuleKind, OUTPUT_ID};
use eframe::egui::{self, RichText, Vec2};

/// "04 Filter", or "00 Output".
pub fn label(app: &App, id: u8) -> String {
    app.project.module(id).map_or_else(|| format!("{id:02X} ?"), |m| format!("{id:02X} {}", m.name))
}

/// The modules `id` could link to: instruments for a MultiSynth and any
/// module with parameters for a Modulator. Sound goes along chains.
pub fn candidates(app: &App, id: u8) -> Vec<u8> {
    let Some(kind) = app.project.module(id).map(|m| m.kind) else { return Vec::new() };
    let mut ids: Vec<u8> = app
        .project
        .modules
        .iter()
        .filter(|m| m.id != id)
        .filter(|m| {
            if kind.notes_only() {
                m.kind.is_instrument()
            } else if kind.controls() {
                // Anything with parameters, other Modulators too.
                m.kind.num_automatable() > 0
            } else {
                false
            }
        })
        .map(|m| m.id)
        .collect();
    // The output first, then by number.
    ids.sort_by_key(|&m| (m != OUTPUT_ID, m));
    ids
}

/// Links `from` to `to`, or unlinks it, saying why when it can't.
pub fn set_link(app: &mut App, from: u8, to: u8, on: bool) {
    if on {
        if app.project.connect(from, to) {
            app.mark();
        } else {
            app.set_status(format!("{} can't go to {}: that would make a loop", label(app, from), label(app, to)));
        }
    } else {
        app.project.disconnect(from, to);
        app.mark();
    }
}

/// A box to tick for every instrument MultiSynth `id` could play.
pub fn plays_checklist(app: &mut App, ui: &mut egui::Ui, id: u8) {
    let mut toggle = None;
    for to in candidates(app, id) {
        let on = app.project.links.contains(&(id, to));
        let mut ticked = on;
        if ui.checkbox(&mut ticked, label(app, to)).changed() {
            toggle = Some((to, ticked));
        }
    }
    if let Some((to, on)) = toggle {
        set_link(app, id, to, on);
    }
}

/// The modules whose sound a Modulator following its input listens to.
pub fn sources_checklist(app: &mut App, ui: &mut egui::Ui, id: u8) {
    let sources: Vec<u8> = app
        .project
        .modules
        .iter()
        .filter(|m| m.id != id && m.kind.makes_sound() && m.id != OUTPUT_ID && m.kind != ModuleKind::Send)
        // Devices placed in a track, send track or the master can't be.
        .filter(|m| !app.project.placed(m.id))
        .map(|m| m.id)
        .collect();
    let mut toggle = None;
    for from in sources {
        let mut ticked = app.project.links.contains(&(from, id));
        if ui.checkbox(&mut ticked, label(app, from)).changed() {
            toggle = Some((from, ticked));
        }
    }
    if let Some((from, on)) = toggle {
        set_link(app, from, id, on);
    }
}

/// What Modulator `id` moves: each module it links to with the parameters
/// it moves there, how far and a button to let go of each, and menus to
/// move more of them or another module's.
pub fn control_targets(app: &mut App, ui: &mut egui::Ui, id: u8) {
    let targets: Vec<u8> = app.project.links.iter().filter(|l| l.0 == id).map(|l| l.1).collect();
    if targets.is_empty() {
        ui.label(RichText::new("It moves nothing yet: add a target.").small().color(theme::TEXT_WEAK));
    }
    let mut changed = None;
    let mut add = None;
    for to in targets {
        let Some(m) = app.project.module(to).cloned() else { continue };
        let controls = app.project.controls_of(id, to);
        let mut edited = controls.clone();
        ui.horizontal(|ui| {
            ui.label(RichText::new(label(app, to)).small().color(theme::PAT_EFFECT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if super::icons::button(ui, super::icons::Icon::Bin).on_hover_text("Stop moving this module").clicked()
                {
                    edited.clear();
                }
                ui.menu_button("+", |ui| {
                    super::widgets::menu_scroll(ui, "add_param", |ui| {
                        for i in (0..m.kind.num_automatable()).filter(|i| !controls.iter().any(|c| c.param == *i)) {
                            if ui.button(m.automatable_name(i)).clicked() {
                                add = Some((to, i));
                                ui.close();
                            }
                        }
                    });
                })
                .response
                .on_hover_text("Move another of its parameters too");
            });
        });
        let mut remove = None;
        for (k, c) in edited.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let mut param = c.param;
                egui::ComboBox::from_id_salt(("control", id, to, k))
                    .selected_text(m.automatable_name(c.param))
                    .width(84.0)
                    .show_ui(ui, |ui| {
                        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                            for i in 0..m.kind.num_automatable() {
                                ui.selectable_value(&mut param, i, m.automatable_name(i));
                            }
                        });
                    });
                c.param = param;
                let w = (ui.available_width() - 22.0).max(30.0);
                ui.allocate_ui(Vec2::new(w, 18.0), |ui| {
                    let spec = &crate::project::CONTROL_DEPTH;
                    super::widgets::named_bar(ui, spec, "", &mut c.depth, |_| {}).on_hover_text(
                        "How far it moves this parameter, of its swing; below zero, the other way. Double-click for all of it",
                    );
                });
                if super::icons::button(ui, super::icons::Icon::Bin).on_hover_text("Stop moving it").clicked() {
                    remove = Some(k);
                }
            });
        }
        if let Some(k) = remove {
            edited.remove(k);
        }
        // A parameter picked twice is moved once.
        let mut seen = Vec::new();
        edited.retain(|c| {
            !seen.contains(&c.param) && {
                seen.push(c.param);
                true
            }
        });
        if edited != controls {
            changed = Some((to, edited));
        }
    }
    if let Some((to, controls)) = changed {
        app.project.set_controls(id, to, controls);
        app.mark();
    }
    ui.menu_button("+ Add Target", |ui| {
        super::widgets::menu_scroll(ui, "add_target", |ui| {
            for to in candidates(app, id) {
                let Some(m) = app.project.module(to) else { continue };
                let moved = app.project.controls_of(id, to);
                ui.menu_button(label(app, to), |ui| {
                    super::widgets::menu_scroll(ui, ("target_params", to), |ui| {
                        for i in (0..m.kind.num_automatable()).filter(|i| !moved.iter().any(|c| c.param == *i)) {
                            if ui.button(m.automatable_name(i)).clicked() {
                                add = Some((to, i));
                                ui.close();
                            }
                        }
                    });
                });
            }
        });
    })
    .response
    .on_hover_text("A parameter of any module for it to move, Modulators' included");
    if let Some((to, param)) = add {
        add_control(app, id, to, param);
    }
}

/// Makes Modulator `id` move parameter `param` of module `to` too, saying
/// why when it can't.
pub fn add_control(app: &mut App, id: u8, to: u8, param: usize) {
    if app.project.add_control(id, to, param) {
        app.mark();
    } else if !app.project.links.contains(&(id, to)) {
        app.set_status(format!("{} can't move {}: that would make a loop", label(app, id), label(app, to)));
    }
}

/// A module's color, from the colors tracks use, or its kind's.
pub fn color_menu(app: &mut App, ui: &mut egui::Ui, id: u8) {
    let mut pick = None;
    ui.horizontal(|ui| {
        for c in theme::TRACK_COLORS {
            if ui.add(egui::Button::new("").fill(c).min_size(Vec2::splat(18.0))).clicked() {
                pick = Some(Some([c.r(), c.g(), c.b()]));
            }
        }
    });
    if ui.button("Default").clicked() {
        pick = Some(None);
    }
    if let Some(c) = pick {
        if let Some(m) = app.project.module_mut(id) {
            m.color = c;
        }
        app.mark_layout();
        ui.close();
    }
}

/// What effect `id` listens to: its own input, or another module's sound
/// as its key input, picked from those that wouldn't loop.
pub fn key_picker(app: &mut App, ui: &mut egui::Ui, id: u8) {
    let Some(key) = app.project.module(id).map(|m| m.key) else { return };
    let shown = key.map_or_else(|| "Its own input".to_string(), |k| label(app, k));
    let mut picked = key;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Key").small());
        egui::ComboBox::from_id_salt(("key_input", id))
            .selected_text(shown)
            .width(ui.available_width() - 4.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut picked, None, "Its own input");
                let ids: Vec<u8> = app.project.modules.iter().map(|m| m.id).collect();
                for k in ids.into_iter().filter(|&k| app.project.can_key(id, k)) {
                    ui.selectable_value(&mut picked, Some(k), label(app, k));
                }
            })
            .response
            .on_hover_text("The sound it listens to, its sidechain: a kick here ducks what goes through it");
    });
    if picked != key {
        app.project.module_mut(id).unwrap().key = picked;
        app.mark();
    }
}
