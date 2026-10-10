//! The module list, a table rather than a module graph: every module with
//! its level, switches and where its sound, notes or parameter changes go,
//! and the selected module's parameters beside it.

use super::{App, instruments, routing, theme, widgets};
use crate::project::{DEFAULT_SHAPE, DRAWN_SHAPE, MACROS, Module, ModuleKind, OUTPUT_ID, interpolate};
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};

/// The stripe along the top of a module: amber for instruments, green for
/// modules that only pass notes on, blue for effects, grey for the output.
pub fn kind_color(kind: ModuleKind) -> Color32 {
    match kind {
        ModuleKind::Output => Color32::from_gray(150),
        k if k.notes_only() => NOTE_LINK,
        k if k.controls() => CONTROL_LINK,
        k if k.is_instrument() => theme::SELECTED,
        _ => Color32::from_rgb(110, 160, 220),
    }
}

/// A module's own color, or its kind's.
pub fn module_color(m: &Module) -> Color32 {
    m.color.map_or(kind_color(m.kind), |[r, g, b]| Color32::from_rgb(r, g, b))
}

/// Links that carry notes rather than sound, as the MultiSynth's do.
const NOTE_LINK: Color32 = Color32::from_rgb(130, 200, 110);
/// Links that move parameters, as the Modulator's do.
const CONTROL_LINK: Color32 = Color32::from_rgb(190, 130, 230);

pub fn params_panel(app: &mut App, ui: &mut egui::Ui) {
    theme::caption(ui, "MODULE PARAMETERS");
    let Some(id) = app.selected_module else {
        ui.weak("Select a module in the list.");
        return;
    };
    let Some(module) = app.project.module(id).cloned() else {
        app.selected_module = None;
        return;
    };
    // A header like a device panel's: number, name and mute.
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{id:02X}")).monospace().color(theme::PAT_INSTRUMENT));
        let mut name = module.name.clone();
        if ui.add(egui::TextEdit::singleline(&mut name).desired_width(ui.available_width() - 50.0)).changed() {
            app.project.module_mut(id).unwrap().name = name;
            app.mark_layout();
        }
        if theme::toggle(ui, module.mute, "Mute").on_hover_text("Silence this module").clicked() {
            app.project.module_mut(id).unwrap().mute = !module.mute;
            app.mark();
        }
    });
    let mut about = module.kind.name().to_string();
    if module.kind.notes_only() {
        about.push_str(" · passes the notes it gets to the instruments it is connected to");
    } else if module.kind.controls() {
        about.push_str(" · moves a parameter of each module it is connected to");
    } else if module.kind.is_instrument() {
        about.push_str(" · notes you play are sent here");
    } else if module.kind == ModuleKind::Input {
        about.push_str(" · plays the sound card's input, which is open while the song has an Input module");
    }
    ui.label(RichText::new(about).small().color(theme::TEXT_WEAK));
    if module.kind == ModuleKind::Drums {
        let drums = "C kick · D snare · F# closed hat · A# open hat · others tom";
        ui.label(RichText::new(drums).small().color(theme::TEXT_WEAK));
    }
    if module.kind.holds_samples() {
        sample_section(app, ui, &module);
    } else if module.kind.has_modulation() {
        ui.horizontal(|ui| {
            let on = !module.modulation.is_default();
            ui.label(
                RichText::new(if on { "Modulation in use" } else { "No modulation" }).small().color(theme::TEXT_WEAK),
            );
            let tip = "Pitch and filter envelopes, vibrato and tremolo (F4)";
            if ui.button("Modulation…").on_hover_text(tip).clicked() {
                app.selected_module = Some(id);
                app.sampler.synth_tab = super::sampler::SynthTab::Modulation;
                app.view = super::View::Sampler;
            }
        });
    }
    ui.add_space(4.0);

    param_list(app, ui, id);
    ui.add_space(6.0);
    let hint = "Drag to change, with Shift for fine steps. Double-click to reset, right-click to type a value.";
    ui.label(RichText::new(hint).small().color(theme::TEXT_WEAK));
    wiring(app, ui, id, module.kind);
}

/// What a Modulator in `mode` follows from the modules ticked under
/// FOLLOWS.
pub fn follows_hint(mode: f32) -> &'static str {
    match mode.round() as u32 {
        0 => "What it would follow in the other modes; as an LFO it follows nothing",
        1 => "The sounds whose level it follows",
        2 => "The instruments whose last note moves it: up for higher than C-4, down for lower",
        3 => "The instruments whose last note's velocity moves it",
        4 => "The instruments whose notes each start its envelope: up over the attack, down over the release",
        _ => "Nothing: it moves its parameters by its Amount, as one knob for them all",
    }
}

/// Where module `id` gets its sound and sends it, its notes or its
/// parameter changes, as lists to tick.
fn wiring(app: &mut App, ui: &mut egui::Ui, id: u8, kind: ModuleKind) {
    let section = |ui: &mut egui::Ui, title: &str, hint: &str| {
        ui.add_space(6.0);
        theme::caption(ui, title);
        ui.label(RichText::new(hint).small().color(theme::TEXT_WEAK));
    };
    if kind.controls() {
        section(ui, "MOVES", "The parameters it moves");
        routing::control_targets(app, ui, id);
        let params = app.project.module(id).map(|m| m.params.clone()).unwrap_or_default();
        section(ui, "FOLLOWS", follows_hint(params.first().copied().unwrap_or(0.0)));
        routing::sources_checklist(app, ui, id);
    } else if kind.notes_only() {
        section(ui, "PLAYS", "The instruments it passes its notes to");
        routing::sends_checklist(app, ui, id);
    } else {
        if kind.has_input() {
            section(ui, "FED BY", "The modules whose sound comes in");
            routing::sources_checklist(app, ui, id);
        }
        if kind.has_output() {
            section(ui, "SENDS TO", "Where its sound goes");
            routing::sends_checklist(app, ui, id);
        }
    }
}

/// A row's switches and labels need this much room.
const ROW_H: f32 = 22.0;

/// Every module in a table, as a conventional replacement for a module
/// graph: instruments, then effects, then Modulators, and the output last.
pub fn list(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        theme::caption(ui, "MODULES");
        ui.menu_button("+ Add", |ui| {
            ui.label(RichText::new("Instruments and the Input go to the output; effects start unconnected").small().color(theme::TEXT_WEAK));
            for kind in ModuleKind::ADDABLE {
                if ui.button(kind.name()).clicked() {
                    if kind.is_instrument() {
                        instruments::add(app, kind);
                    } else if let Some(id) = app.project.add_module(kind, [0.0, 0.0]) {
                        // A source is heard at once, as instruments are.
                        if !kind.has_input() && kind.makes_sound() {
                            app.project.connect(id, OUTPUT_ID);
                        }
                        app.selected_module = Some(id);
                        app.mark();
                    }
                    ui.close();
                }
            }
        });
        let hint = "Click a module to edit it on the right; right-click for its color, duplicate and delete. Instruments are easier to edit in the Instrument tab (F4).";
        ui.add(egui::Label::new(RichText::new(hint).small().color(theme::TEXT_WEAK)).truncate()).on_hover_text(hint);
    });
    let mut ids: Vec<(u8, u8)> = app
        .project
        .modules
        .iter()
        .map(|m| {
            let group = if m.id == OUTPUT_ID {
                3
            } else if m.kind.is_instrument() {
                0
            } else if m.kind.makes_sound() {
                1
            } else {
                2
            };
            (group, m.id)
        })
        .collect();
    ids.sort();
    let mut delete = None;
    egui::Frame::new().fill(theme::INSET).inner_margin(2).show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("module_list").auto_shrink(false).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 1.0;
            for (_, id) in ids {
                if let Some(d) = row(app, ui, id) {
                    delete = Some(d);
                }
            }
        });
    });
    if let Some(id) = delete {
        app.project.remove_module(id);
        if app.selected_module == Some(id) {
            app.selected_module = None;
        }
        app.mark();
    }
}

/// One module's row: color, number and name, kind, level, where it gets
/// and sends things, and its switches. Returns the module to delete.
fn row(app: &mut App, ui: &mut egui::Ui, id: u8) -> Option<u8> {
    let m = app.project.module(id)?.clone();
    let selected = app.selected_module == Some(id);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
    let painter = ui.painter_at(rect);
    if selected {
        painter.rect_filled(rect, 2.0, theme::SELECTED.gamma_multiply(0.25));
    } else if resp.hovered() {
        painter.rect_filled(rect, 2.0, Color32::from_gray(30));
    }
    let off = m.mute || m.bypass;
    let color = if off { Color32::from_gray(80) } else { module_color(&m) };
    painter.rect_filled(Rect::from_min_size(rect.min + Vec2::new(2.0, 3.0), Vec2::new(5.0, ROW_H - 6.0)), 1.0, color);
    let text = |x: f32, s: String, c: Color32| {
        painter.text(
            egui::pos2(rect.left() + x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            s,
            egui::FontId::proportional(12.0),
            c,
        );
    };
    let name_color = if off { theme::TEXT_WEAK } else { theme::TEXT };
    text(12.0, format!("{id:02X} {}", m.name), name_color);
    text(150.0, m.kind.name().to_string(), theme::TEXT_WEAK);
    // The level it puts out, left above right, as the module graph showed.
    if m.kind.makes_sound() {
        let level = app.level(id);
        for (ch, l) in level.iter().enumerate() {
            let bar = Rect::from_min_size(
                egui::pos2(rect.left() + 230.0, rect.top() + 6.0 + 6.0 * ch as f32),
                Vec2::new(60.0, 3.0),
            );
            painter.rect_filled(bar, 0.0, Color32::from_gray(36));
            let lit = Rect::from_min_size(bar.min, Vec2::new(60.0 * l.min(1.0), 3.0));
            painter.rect_filled(lit, 0.0, if *l > 1.0 { theme::RECORD } else { theme::SCOPE });
        }
    }
    // Where it sends things: sound, notes or parameter changes.
    let sends: Vec<String> = app.project.links.iter().filter(|l| l.0 == id).map(|l| routing::label(app, l.1)).collect();
    let arrow = if m.kind.notes_only() {
        "plays"
    } else if m.kind.controls() {
        "moves"
    } else {
        "to"
    };
    let route = if sends.is_empty() {
        if id == OUTPUT_ID {
            "to sound card".to_string()
        } else if app.project.master.contains(&id) {
            "in the master chain".to_string()
        } else {
            format!("{arrow} nothing")
        }
    } else {
        format!("{arrow} {}", sends.join(", "))
    };
    let route_rect = Rect::from_x_y_ranges(rect.left() + 305.0..=rect.right() - 120.0, rect.y_range());
    painter.with_clip_rect(route_rect).text(
        egui::pos2(route_rect.left(), rect.center().y),
        egui::Align2::LEFT_CENTER,
        route,
        egui::FontId::proportional(11.0),
        if m.kind.notes_only() || m.kind.controls() { color } else { theme::TEXT_WEAK },
    );

    // Switches at the right: mute and solo, and on/off for effects.
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("module_row", id))
            .max_rect(Rect::from_x_y_ranges(rect.right() - 115.0..=rect.right() - 2.0, rect.y_range()))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    if m.kind.has_input() && m.kind.has_output() && m.kind.makes_sound() {
        let label = if m.bypass { "Off" } else { "On" };
        if theme::toggle(&mut child, !m.bypass, label)
            .on_hover_text("Switch the effect off: the sound goes past it")
            .clicked()
        {
            app.project.module_mut(id).unwrap().bypass = !m.bypass;
            app.mark();
        }
    }
    if m.kind.makes_sound() && id != OUTPUT_ID && theme::toggle(&mut child, m.solo, "S").on_hover_text("Solo").clicked()
    {
        app.project.module_mut(id).unwrap().solo = !m.solo;
        app.mark();
    }
    if theme::toggle(&mut child, m.mute, "M").on_hover_text("Mute").clicked() {
        app.project.module_mut(id).unwrap().mute = !m.mute;
        app.mark();
    }

    if resp.clicked() {
        app.selected_module = Some(id);
    }
    if resp.double_clicked() && m.kind.is_instrument() {
        // Hear it, as the module graph did.
        app.send(crate::engine::Cmd::NoteOn { module: id, key: crate::engine::LIVE_KEY + 200, note: 60, vel: 1.0 });
        app.send(crate::engine::Cmd::NoteOff { module: id, key: crate::engine::LIVE_KEY + 200 });
    }
    let mut delete = None;
    resp.on_hover_text("Click to edit; double-click an instrument to hear it").context_menu(|ui| {
        ui.menu_button("Presets", |ui| super::presets::menu(app, ui, id));
        ui.menu_button("Color", |ui| routing::color_menu(app, ui, id));
        if id != OUTPUT_ID && ui.button("Duplicate").clicked() {
            if let Some(new) = app.project.duplicate_module(id, m.pos) {
                app.selected_module = Some(new);
                app.mark();
            }
            ui.close();
        }
        if id != OUTPUT_ID && ui.button("Delete").clicked() {
            delete = Some(id);
            ui.close();
        }
    });
    delete
}

/// Module `id`'s parameters as bars, following envelopes while the song
/// plays, marked where automated and with "Automate" in their menus.
pub fn param_list(app: &mut App, ui: &mut egui::Ui, id: u8) {
    let Some(module) = app.project.module(id).cloned() else { return };
    // While an envelope moves a parameter, its bar follows it.
    let shown: Vec<f32> = (0..module.params.len()).map(|i| app.automated(id, i).unwrap_or(module.params[i])).collect();
    let mut params = shown.clone();
    let mut changed = false;
    let mut automate = None;
    let automated: Vec<usize> = app.pattern().automation.iter().filter(|e| e.module == id).map(|e| e.param).collect();
    // The instrument whose macros can move these, and its macros' names.
    let owner = app.project.macro_owner(id);
    let macro_names: Vec<String> = owner
        .and_then(|o| app.project.module(o))
        .map_or(Vec::new(), |o| (0..MACROS).map(|k| o.macro_name(k)).collect());
    let mut map = None;
    ui.spacing_mut().item_spacing.y = 3.0;
    for (i, (spec, v)) in module.kind.params().iter().zip(&mut params).enumerate() {
        if !in_use(module.kind, &shown, i) {
            continue;
        }
        let resp = widgets::param_bar(ui, spec, v, |ui| {
            if ui.button("Automate in This Pattern").clicked() {
                automate = Some(i);
                ui.close();
            }
            if !macro_names.is_empty() {
                ui.menu_button("Map to Macro", |ui| {
                    for (k, name) in macro_names.iter().enumerate() {
                        if ui.button(name).clicked() {
                            map = Some((k, i));
                            ui.close();
                        }
                    }
                });
            }
        });
        changed |= resp.changed();
        if automated.contains(&i) {
            // A mark on the left of automated parameters.
            let mark =
                Rect::from_min_size(resp.rect.min + Vec2::new(1.0, 3.0), Vec2::new(3.0, resp.rect.height() - 6.0));
            ui.painter().rect_filled(mark, 1.0, theme::SELECTED);
            resp.on_hover_text("Automated in this pattern; the envelope wins while it plays");
        }
    }
    if changed {
        // Only what was dragged changes; the rest keeps the song's value.
        let m = app.project.module_mut(id).unwrap();
        for (i, v) in params.iter().enumerate() {
            if *v != shown[i] {
                m.params[i] = *v;
            }
        }
        app.mark();
    }
    if let (Some(owner), Some((k, i))) = (owner, map) {
        app.project.map_macro(owner, k, id, i);
        app.mark();
    }
    if let Some(i) = automate {
        super::automation::add(app, id, i);
        app.lower = super::Lower::Automation;
    }
    if module.kind == ModuleKind::Modulator && shown[0].round() == 0.0 && shown[1].round() as u32 == DRAWN_SHAPE {
        shape_editor(app, ui, id);
    }
    if module.kind == ModuleKind::Convolver {
        impulse_section(app, ui, &module);
    }
    if module.kind.takes_key() {
        ui.add_space(3.0);
        routing::key_picker(app, ui, id);
    }
}

/// A Convolver's own impulse: the sample loaded as one, to load or clear.
fn impulse_section(app: &mut App, ui: &mut egui::Ui, module: &Module) {
    ui.add_space(4.0);
    let what = match module.samples.first() {
        Some(s) if let Some(d) = &s.data => format!("Impulse: {} ({:.2} s)", s.name, d.seconds()),
        Some(s) => format!("Impulse: {} (missing)", s.name),
        None => "No impulse loaded".to_string(),
    };
    ui.label(RichText::new(what).small().color(theme::TEXT_WEAK));
    ui.horizontal(|ui| {
        let tip = "A recording of a space or a cabinet answering a click: WAV, FLAC or Ogg, up to 6 s";
        if ui.button("Load Impulse…").on_hover_text(tip).clicked() {
            app.pick_file(super::files::Purpose::LoadSample(module.id));
        }
        if ui.add_enabled(!module.samples.is_empty(), egui::Button::new("Clear")).clicked() {
            app.project.module_mut(module.id).unwrap().samples.clear();
            app.mark();
        }
    });
}

/// Whether parameter `i` of a `kind` set to `params` does anything, so
/// the list shows it: a Modulator's LFO settings only as an LFO, its rate,
/// period or beats only as its Sync says, its attack and release only when
/// it follows something.
fn in_use(kind: ModuleKind, params: &[f32], i: usize) -> bool {
    if kind != ModuleKind::Modulator {
        return true;
    }
    let (mode, sync) = (params[0].round() as u32, params[3].round() as u32);
    match i {
        1 | 3 => mode == 0,
        2 => mode == 0 && sync == 0,
        4 => mode == 0 && sync == 1,
        8 => mode == 0 && sync == 2,
        // Manual smooths its knob over the attack only.
        6 => mode != 0,
        7 => !matches!(mode, 0 | 5),
        _ => true,
    }
}

/// A Modulator's drawn LFO shape, a cycle from left to right: click to add
/// a point, drag one to move it, right-click one to take it away.
fn shape_editor(app: &mut App, ui: &mut egui::Ui, id: u8) {
    let Some(m) = app.project.module(id) else { return };
    let mut points = if m.shape.is_empty() { DEFAULT_SHAPE.to_vec() } else { m.shape.clone() };
    let before = points.clone();
    ui.add_space(4.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 80.0), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, theme::INSET);
    let x_of = |x: f32| rect.left() + x * rect.width();
    let y_of = |v: f32| rect.bottom() - v * rect.height();
    painter.line_segment(
        [Pos2::new(rect.left(), y_of(0.5)), Pos2::new(rect.right(), y_of(0.5))],
        (1.0, Color32::from_gray(40)),
    );
    let at = |p: Pos2| {
        (((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0), ((rect.bottom() - p.y) / rect.height()).clamp(0.0, 1.0))
    };
    let hit =
        |p: Pos2, pts: &[(f32, f32)]| pts.iter().position(|&(x, v)| Pos2::new(x_of(x), y_of(v)).distance(p) < 7.0);
    let drag_id = ui.id().with(("shape_drag", id));
    if resp.drag_started()
        && let Some(p) = ui.input(|i| i.pointer.press_origin())
    {
        let i = hit(p, &points).unwrap_or_else(|| {
            // Dragging where there is no point draws one there.
            let (x, v) = at(p);
            let i = points.iter().position(|q| q.0 > x).unwrap_or(points.len());
            points.insert(i, (x, v));
            i
        });
        ui.data_mut(|d| d.insert_temp(drag_id, i));
    }
    if resp.dragged()
        && let (Some(i), Some(p)) = (ui.data(|d| d.get_temp::<usize>(drag_id)), resp.interact_pointer_pos())
        && i < points.len()
    {
        // A point keeps its place among its neighbours.
        let (x, v) = at(p);
        let lo = if i > 0 { points[i - 1].0 } else { 0.0 };
        let hi = points.get(i + 1).map_or(1.0, |n| n.0);
        points[i] = (x.clamp(lo, hi), v);
    }
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
        && hit(p, &points).is_none()
    {
        let (x, v) = at(p);
        let i = points.iter().position(|q| q.0 > x).unwrap_or(points.len());
        points.insert(i, (x, v));
    }
    if resp.secondary_clicked()
        && let Some(i) = resp.interact_pointer_pos().and_then(|p| hit(p, &points))
        && points.len() > 2
    {
        points.remove(i);
    }
    let line: Vec<Pos2> = (0..=64)
        .map(|k| {
            let x = k as f32 / 64.0;
            Pos2::new(x_of(x), y_of(interpolate(&points, x, false, false).unwrap_or(0.5)))
        })
        .collect();
    painter.add(egui::Shape::line(line, Stroke::new(1.5, theme::SELECTED)));
    for &(x, v) in &points {
        painter.rect_filled(
            Rect::from_center_size(Pos2::new(x_of(x), y_of(v)), Vec2::splat(6.0)),
            1.0,
            theme::SELECTED,
        );
    }
    resp.on_hover_text(
        "The LFO's cycle, left to right: click to add a point, drag one to move it, right-click to remove it",
    );
    if points != before {
        app.project.module_mut(id).unwrap().shape = points;
        app.mark();
    }
}

fn sample_section(app: &mut App, ui: &mut egui::Ui, module: &Module) {
    ui.separator();
    let n = module.samples.len();
    let missing = module.samples.iter().filter(|s| s.data.is_none()).count();
    let mut text = match n {
        0 => "No samples.".to_string(),
        1 => "1 sample".to_string(),
        n => format!("{n} samples"),
    };
    if missing > 0 {
        text.push_str(&format!(" ({missing} missing)"));
    }
    ui.horizontal(|ui| {
        ui.label(text);
        if ui.button("Open in Editor").on_hover_text("The instrument editor (F4)").clicked() {
            app.view = super::View::Sampler;
        }
    });
    let offset = "Effect 9xx starts playback xx/256 of the way into the sample.";
    ui.label(RichText::new(offset).small().color(theme::TEXT_WEAK));
}
