//! A module's parameters as the device panels show them, with what an LFO
//! or a Modulator does drawn over them, and the colors modules go by.

use super::{App, lfo, routing, theme, widgets};
use crate::project::{
    DEFAULT_SHAPE, DRAWN_SHAPE, MACROS, Module, ModuleKind, RANDOM_SHAPE, SMOOTH_RANDOM_SHAPE, interpolate,
};
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
    // The Modulators that could move these, any but this one.
    let modulators: Vec<u8> =
        app.project.modules.iter().filter(|m| m.kind.controls() && m.id != id).map(|m| m.id).collect();
    let mut modulate = None;
    ui.spacing_mut().item_spacing.y = 3.0;
    wave_view(app, ui, &module, &shown);
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
            ui.menu_button("Modulate With", |ui| {
                if ui.button("New Modulator").on_hover_text("An LFO moving it, to set up in the chain").clicked() {
                    modulate = Some((None, i));
                    ui.close();
                }
                for &m in &modulators {
                    if ui.button(routing::label(app, m)).clicked() {
                        modulate = Some((Some(m), i));
                        ui.close();
                    }
                }
            });
        });
        changed |= resp.changed();
        let by = app.project.modulated_by(id, i);
        if !by.is_empty() {
            // A mark on the right of parameters Modulators move.
            let mark = Rect::from_min_size(
                Pos2::new(resp.rect.right() - 4.0, resp.rect.top() + 3.0),
                Vec2::new(3.0, resp.rect.height() - 6.0),
            );
            ui.painter().rect_filled(mark, 1.0, CONTROL_LINK);
            let names: Vec<String> =
                by.iter().map(|&(m, depth)| format!("{} ({:+.0}%)", routing::label(app, m), depth * 100.0)).collect();
            resp.clone().on_hover_text(format!("Moved by {}", names.join(", ")));
        }
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
    if let Some((by, i)) = modulate {
        modulate_with(app, by, id, i);
    }
    if module.kind == ModuleKind::Modulator && shown[0].round() == 0.0 && shown[1].round() as u32 == DRAWN_SHAPE {
        let phase = (!module.mute).then(|| (app.modulator(id).0 + shown[9]).fract());
        shape_editor(app, ui, id, phase);
    }
    if module.kind == ModuleKind::Convolver {
        impulse_section(app, ui, &module);
    }
    if module.kind.takes_key() {
        ui.add_space(3.0);
        routing::key_picker(app, ui, id);
    }
}

/// Makes Modulator `by`, or a new one, move parameter `param` of module
/// `id`.
pub fn modulate_with(app: &mut App, by: Option<u8>, id: u8, param: usize) {
    let by = match by {
        Some(by) => by,
        None => match app.project.add_module(ModuleKind::Modulator, [0.0, 0.0]) {
            Some(new) => {
                // It shows in the chain of what it moves.
                app.project.module_mut(new).unwrap().home = app.project.owner_of(id);
                new
            }
            None => return app.set_status("Too many modules"),
        },
    };
    routing::add_control(app, by, id, param);
    let what = app.project.module(id).map(|m| m.automatable_name(param)).unwrap_or_default();
    app.set_status(format!("{} moves {} · {what}", routing::label(app, by), routing::label(app, id)));
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

/// What an LFO or a Modulator set to `params` does, drawn over the
/// parameters: an LFO's cycle, with a dot where a Modulator's is now, or
/// what a Modulator following something has put out lately. A drawn shape
/// has its own editor instead.
fn wave_view(app: &App, ui: &mut egui::Ui, module: &Module, params: &[f32]) {
    let specs = module.kind.params();
    let name = |i: usize| specs[i].choices.get(params[i].round() as usize).copied().unwrap_or("");
    match module.kind {
        ModuleKind::Lfo => {
            let shape = params[1].round() as u32;
            let speed = if params[4] >= 0.5 { specs[5].format(params[5]) } else { specs[3].format(params[3]) };
            let about = format!("{} · {} · {speed}", name(0), name(1));
            lfo::cycle(ui, 54.0, &|p| crate::dsp::lfo_shape(shape, p), params[2], None, &about);
        }
        ModuleKind::Modulator => {
            let (mode, shape) = (params[0].round() as u32, params[1].round() as u32);
            let (phase, signal) = app.modulator(module.id);
            let (amount, offset, unipolar) = (params[5], params[9], params[10] >= 0.5);
            let random = matches!(shape, RANDOM_SHAPE | SMOOTH_RANDOM_SHAPE);
            let speed = match params[3].round() as u32 {
                0 => specs[2].format(params[2]),
                1 => specs[4].format(params[4]),
                _ if name(8) == "1" => "1 beat".to_string(),
                _ => format!("{} beats", name(8)),
            };
            let trace = |ui: &mut egui::Ui, value: f32, about: &str| {
                lfo::trace(ui, 54.0, ui.id().with(("modulator_trace", module.id)), value, amount, about);
            };
            if mode != 0 {
                trace(ui, signal, &format!("{} · output", name(0)));
            } else if random {
                // Unipolar, it puts out 0..1, and bipolar half of -1..1.
                trace(ui, if unipolar { signal } else { 2.0 * signal }, &format!("{} · {speed}", name(1)));
            } else if shape != DRAWN_SHAPE {
                let drawn = &module.shape;
                let wave = |p: f32| {
                    let w = lfo::shape_at(shape, drawn, (p + offset).fract());
                    if unipolar { (w + 1.0) * 0.5 } else { w }
                };
                let phase = (!module.mute).then_some(phase);
                lfo::cycle(ui, 54.0, &wave, amount, phase, &format!("{} · {speed}", name(1)));
            } else {
                return;
            }
            ui.add_space(2.0);
        }
        _ => return,
    }
    ui.add_space(2.0);
}

/// Whether parameter `i` of a `kind` set to `params` does anything, so
/// the list shows it: a Modulator's LFO settings only as an LFO, its rate,
/// period or beats only as its Sync says, its attack and release only when
/// it follows something.
fn in_use(kind: ModuleKind, params: &[f32], i: usize) -> bool {
    if kind != ModuleKind::Modulator {
        return true;
    }
    let (mode, sync, shape) = (params[0].round() as u32, params[3].round() as u32, params[1].round() as u32);
    match i {
        1 | 3 | 10 => mode == 0,
        // Where in the cycle it starts means nothing to a random shape.
        9 => mode == 0 && !matches!(shape, RANDOM_SHAPE | SMOOTH_RANDOM_SHAPE),
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
fn shape_editor(app: &mut App, ui: &mut egui::Ui, id: u8, phase: Option<f32>) {
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
    if let Some(p) = phase {
        let at = Pos2::new(x_of(p), y_of(interpolate(&points, p, false, false).unwrap_or(0.5)));
        painter.line_segment([Pos2::new(at.x, rect.top()), Pos2::new(at.x, rect.bottom())], (1.0, theme::SCOPE));
        painter.circle_filled(at, 3.5, Color32::WHITE);
    }
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
