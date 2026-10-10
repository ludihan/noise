//! Wiring modules without a graph: lists of where a module sends its sound,
//! notes or parameter changes, ticked on and off, as menus and panels in
//! the instrument editor and the module list use them.

use super::{App, theme};
use crate::project::OUTPUT_ID;
use eframe::egui::{self, RichText, Vec2};

/// "04 Filter", or "00 Output".
pub fn label(app: &App, id: u8) -> String {
    app.project.module(id).map_or_else(|| format!("{id:02X} ?"), |m| format!("{id:02X} {}", m.name))
}

/// The modules `id` could link to: instruments for a MultiSynth, any
/// module for a Modulator, and modules that take sound for the rest.
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
                !m.kind.controls()
            } else {
                m.kind.has_input()
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

/// A box to tick for every module `id` could send to.
pub fn sends_checklist(app: &mut App, ui: &mut egui::Ui, id: u8) {
    sends_checklist_except(app, ui, id, &[]);
}

/// The same, leaving out `except`, such as the devices of the chain `id`
/// ends, which sending back to would loop.
pub fn sends_checklist_except(app: &mut App, ui: &mut egui::Ui, id: u8, except: &[u8]) {
    let mut toggle = None;
    for to in candidates(app, id).into_iter().filter(|t| !except.contains(t)) {
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
        .filter(|m| m.id != id && m.kind.makes_sound() && m.id != OUTPUT_ID)
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

/// What Modulator `id` moves: each target with the parameter picked for it
/// and a button to let go of it, and a menu to add one.
pub fn control_targets(app: &mut App, ui: &mut egui::Ui, id: u8) {
    let targets: Vec<u8> = app.project.links.iter().filter(|l| l.0 == id).map(|l| l.1).collect();
    if targets.is_empty() {
        ui.label(RichText::new("It moves nothing yet: add a target.").small().color(theme::TEXT_WEAK));
    }
    let mut remove = None;
    for to in targets {
        let Some(m) = app.project.module(to).cloned() else { continue };
        let kind = m.kind;
        let current = app.project.control_param(id, to);
        let param = |i: usize| m.automatable_name(i);
        let mut pick = current;
        ui.horizontal(|ui| {
            ui.label(RichText::new(label(app, to)).small().color(theme::PAT_EFFECT));
            egui::ComboBox::from_id_salt(("control", id, to)).selected_text(param(current)).width(110.0).show_ui(
                ui,
                |ui| {
                    for i in 0..kind.num_automatable() {
                        ui.selectable_value(&mut pick, i, param(i));
                    }
                },
            );
            if super::icons::button(ui, super::icons::Icon::Bin).on_hover_text("Stop moving it").clicked() {
                remove = Some(to);
            }
        });
        if pick != current {
            app.project.set_control_param(id, to, pick);
            app.mark();
        }
    }
    if let Some(to) = remove {
        set_link(app, id, to, false);
    }
    let mut add = None;
    ui.menu_button("+ Add Target", |ui| {
        for to in candidates(app, id) {
            if !app.project.links.contains(&(id, to)) && ui.button(label(app, to)).clicked() {
                add = Some(to);
                ui.close();
            }
        }
    });
    if let Some(to) = add {
        set_link(app, id, to, true);
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
