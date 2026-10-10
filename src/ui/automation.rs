//! Automation: envelopes that move module parameters over
//! the course of a pattern, drawn in the lower frame.

use super::{App, theme};
use crate::project::{Envelope, LFO_SHAPES, ParamSpec};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};

#[derive(Default)]
pub struct AutomationView {
    /// The parameter whose envelope is shown, by module and parameter; it
    /// may have none in this pattern yet.
    selected: Option<(u8, usize)>,
    /// The point being dragged, or `None` while drawing.
    dragging: Option<usize>,
    /// Text the parameter browser is narrowed to.
    filter: String,
    /// The browser shows every parameter, not just the automated ones.
    pub all: bool,
    /// The pattern the browser last looked at: a new one starts on its
    /// automated parameters, or on all of them if it has none.
    seen_pattern: Option<usize>,
}

/// The browser's groups: a heading and the modules under it.
fn groups(app: &App) -> Vec<(String, Vec<u8>)> {
    let p = &app.project;
    let mut ids: Vec<u8> = p.modules.iter().map(|m| m.id).collect();
    ids.sort();
    let kind = |id: u8| p.module(id).map(|m| m.kind);
    let effect = |id: u8| kind(id).is_some_and(|k| k.has_input() && k.has_output() && k.makes_sound());
    let pick = |f: &dyn Fn(u8) -> bool| ids.iter().copied().filter(|&id| f(id)).collect::<Vec<u8>>();
    let mut groups = vec![
        ("Instruments".to_string(), pick(&|id| kind(id).is_some_and(|k| k.is_instrument()))),
        ("Effects".to_string(), pick(&|id| effect(id) && !p.placed(id))),
    ];
    for (t, track) in p.tracks.iter().enumerate().filter(|(_, t)| !t.effects.is_empty()) {
        groups.push((format!("Track {:02} · {}", t + 1, p.track_name(t)), track.effects.clone()));
    }
    let mut master = p.master.clone();
    master.push(crate::project::OUTPUT_ID);
    groups.push(("Master".to_string(), master));
    groups.push((
        "Modulators and sources".to_string(),
        pick(&|id| kind(id).is_some_and(|k| k.controls() || (!k.is_instrument() && !k.has_input() && k.makes_sound()))),
    ));
    groups.retain(|g| !g.1.is_empty());
    groups
}

/// "04 Filter · Cutoff".
fn describe(app: &App, module: u8, param: usize) -> String {
    match app.project.module(module) {
        Some(m) => format!("{module:02X} {} · {}", m.name, m.automatable_name(param)),
        None => format!("{module:02X} (deleted)"),
    }
}

/// Automates `param` of `module` in the pattern being edited, starting
/// from its current value, and selects the envelope.
pub fn add(app: &mut App, module: u8, param: usize) {
    app.automation.selected = Some((module, param));
    if app.pattern().automation.iter().any(|e| (e.module, e.param) == (module, param)) {
        return;
    }
    let Some(m) = app.project.module(module) else { return };
    let Some(spec) = m.kind.automatable(param) else { return };
    let start = spec.position(m.automatable_value(param));
    app.pattern_mut().automation.push(Envelope::new(module, param, vec![(0.0, start)]));
    app.mark();
}

pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    let h = ui.available_height();
    ui.horizontal_top(|ui| {
        super::widgets::boxed(ui, "envelopes", Vec2::new(240.0, h), |ui| list(app, ui));
        super::widgets::boxed(ui, "envelope", Vec2::new(ui.available_width(), h), |ui| editor(app, ui));
    });
}

/// The parameter browser: the automated parameters of this pattern, or
/// every module's, grouped and folded, narrowed by a filter.
fn list(app: &mut App, ui: &mut egui::Ui) {
    let pattern = app.current_pattern_index();
    let envelopes: Vec<(u8, usize)> = app.pattern().automation.iter().map(|e| (e.module, e.param)).collect();
    if app.automation.seen_pattern != Some(pattern) {
        app.automation.seen_pattern = Some(pattern);
        app.automation.all = envelopes.is_empty();
        // A pattern opens on its own envelopes.
        app.automation.selected = envelopes.first().copied();
    }
    if app.automation.selected.is_none_or(|(m, _)| app.project.module(m).is_none()) {
        app.automation.selected = envelopes.first().copied();
    }
    theme::caption(ui, &format!("AUTOMATION · PATTERN {pattern:02}"));
    ui.horizontal(|ui| {
        let tip = "Only what is automated in this pattern";
        if theme::toggle(ui, !app.automation.all, format!("Automated {}", envelopes.len())).on_hover_text(tip).clicked()
        {
            app.automation.all = false;
        }
        if theme::toggle(ui, app.automation.all, "All").on_hover_text("Every module's parameters").clicked() {
            app.automation.all = true;
        }
    });
    let edit = egui::TextEdit::singleline(&mut app.automation.filter).hint_text("Filter: a module or parameter");
    ui.add(edit.desired_width(f32::INFINITY));
    let filter = app.automation.filter.trim().to_lowercase();
    let matches = |text: &str| filter.is_empty() || text.to_lowercase().contains(&filter);
    let mut pick = None;
    egui::Frame::new().fill(theme::INSET).inner_margin(2).show(ui, |ui| {
        let list = egui::ScrollArea::vertical().max_height(ui.available_height() - 30.0).auto_shrink(false);
        list.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                if !app.automation.all {
                    for &(m, p) in envelopes.iter().filter(|&&(m, p)| matches(&describe(app, m, p))) {
                        let selected = app.automation.selected == Some((m, p));
                        if ui.add(egui::Button::selectable(selected, describe(app, m, p))).clicked() {
                            pick = Some((m, p));
                        }
                    }
                    if envelopes.is_empty() {
                        let hint = "Nothing is automated in this pattern yet. Pick a parameter under All, or right-click one's bar and choose Automate.";
                        ui.label(RichText::new(hint).small().color(theme::TEXT_WEAK));
                    }
                    return;
                }
                for (heading, ids) in groups(app) {
                    let mut shown_heading = false;
                    for id in ids {
                        let Some(m) = app.project.module(id) else { continue };
                        let (kind, name, m) = (m.kind, m.name.clone(), m.clone());
                        let module_hit = matches(&name) || matches(kind.name());
                        let params: Vec<usize> = (0..kind.num_automatable())
                            .filter(|&i| module_hit || matches(&m.automatable_name(i)))
                            .collect();
                        if params.is_empty() {
                            continue;
                        }
                        if !std::mem::replace(&mut shown_heading, true) {
                            ui.add_space(3.0);
                            ui.label(RichText::new(&heading).small().color(theme::TEXT_WEAK));
                        }
                        let count = envelopes.iter().filter(|e| e.0 == id).count();
                        let label = if count > 0 { format!("{id:02X} {name}  • {count}") } else { format!("{id:02X} {name}") };
                        let header = egui::CollapsingHeader::new(label)
                            .id_salt(("automate", id))
                            .default_open(app.selected_module == Some(id));
                        let header = if filter.is_empty() { header } else { header.open(Some(true)) };
                        header.show(ui, |ui| {
                            for i in params {
                                let on = envelopes.contains(&(id, i));
                                let text = format!("{}{}", if on { "• " } else { "" }, m.automatable_name(i));
                                let selected = app.automation.selected == Some((id, i));
                                // Automated ones are marked in the accent colour, but not
                                // on the selection's own accent fill.
                                let text = match (on, selected) {
                                    (_, true) => RichText::new(text).color(theme::SELECTED_TEXT),
                                    (true, false) => RichText::new(text).color(theme::SELECTED),
                                    _ => RichText::new(text),
                                };
                                if ui.add(egui::Button::selectable(selected, text)).clicked() {
                                    pick = Some((id, i));
                                }
                            }
                        });
                    }
                }
            });
        });
    });
    if let Some(p) = pick {
        app.automation.selected = Some(p);
        app.selected_module = Some(p.0);
    }
    let selected = app.automation.selected.filter(|s| envelopes.contains(s));
    let tip = "Remove the selected envelope from this pattern";
    if ui.add_enabled(selected.is_some(), egui::Button::new("Remove Envelope")).on_hover_text(tip).clicked() {
        app.pattern_mut().automation.retain(|e| Some((e.module, e.param)) != selected);
        app.mark();
    }
}

fn editor(app: &mut App, ui: &mut egui::Ui) {
    let found = app.automation.selected.and_then(|(m, p)| {
        let i = app.pattern().automation.iter().position(|e| (e.module, e.param) == (m, p));
        let module = app.project.module(m)?;
        Some((m, p, i, module.kind.automatable(p)?, module.automatable_value(p)))
    });
    let Some((module, param, index, spec, value)) = found else {
        theme::caption(ui, "ENVELOPE");
        ui.label(RichText::new("Pick a parameter on the left.").color(theme::TEXT_WEAK));
        return;
    };
    // A parameter not automated here yet gets an envelope once it has a
    // point.
    let mut env = match index {
        Some(i) => app.pattern().automation[i].clone(),
        None => Envelope::new(module, param, Vec::new()),
    };
    let before = env.clone();

    ui.horizontal(|ui| {
        theme::caption(ui, &describe(app, env.module, env.param).to_uppercase());
        if theme::toggle(ui, env.steps, "Points").on_hover_text("Hold each point's value until the next").clicked() {
            env.steps = true;
            env.curve = false;
        }
        if theme::toggle(ui, !env.steps && !env.curve, "Lines")
            .on_hover_text("Move in straight lines from point to point")
            .clicked()
        {
            env.steps = false;
            env.curve = false;
        }
        if theme::toggle(ui, !env.steps && env.curve, "Curve")
            .on_hover_text("Move along a smooth curve through the points")
            .clicked()
        {
            env.steps = false;
            env.curve = true;
        }
        if ui.button("Clear").on_hover_text("Remove every point").clicked() {
            env.points.clear();
        }
    });
    // Which lines it takes effect in, on a row of its own.
    let (pattern_lines, lpb) = (app.pattern().lines, app.project.lpb.max(1));
    let around = spec.position(value);
    ui.horizontal(|ui| {
        timing(ui, &mut env, pattern_lines, lpb);
        ui.separator();
        shape_menu(ui, &mut env, pattern_lines, lpb, around);
    });

    let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, theme::INSET);
    let graph = Rect::from_min_max(rect.min + Vec2::new(58.0, 8.0), rect.max - Vec2::new(8.0, 16.0));
    // The graph is the envelope's own lines, the pattern or less of it.
    let pattern_lines = app.pattern().lines;
    let lines = env.span(pattern_lines, lpb);
    let x_of = |pos: f32| graph.left() + pos / lines * graph.width();
    let y_of = |t: f32| graph.bottom() - t * graph.height();

    grid(app, &painter, graph, spec, env.start, lines);
    // Where the song is playing in this pattern, and the edit cursor.
    let (play_slot, play_line) = app.play_position();
    let playing_here =
        app.is_playing() && app.project.order.get(play_slot).is_some_and(|s| s.pattern == app.current_pattern_index());
    let mark = |pos: f32, color: Color32| {
        painter.line_segment([Pos2::new(x_of(pos), graph.top()), Pos2::new(x_of(pos), graph.bottom())], (1.0, color));
    };
    if let Some(at) = env.local(app.cursor.line as f32, pattern_lines, lpb) {
        mark(at, theme::SELECTED.gamma_multiply(0.35));
    }
    if let Some(at) = env.local(play_line as f32, pattern_lines, lpb).filter(|_| playing_here) {
        mark(at, theme::SCOPE);
    }

    // Without points, the parameter's own value, faintly, and how to start.
    if env.points.is_empty() {
        let y = y_of(spec.position(value));
        painter.line_segment(
            [Pos2::new(graph.left(), y), Pos2::new(graph.right(), y)],
            (1.0, theme::SELECTED.gamma_multiply(0.4)),
        );
        let hint = "Not automated in this pattern: click to add the first point";
        painter.text(graph.center(), Align2::CENTER_CENTER, hint, FontId::proportional(13.0), theme::TEXT_WEAK);
    }
    // The envelope, held flat before its first point and after its last.
    if let (Some(first), Some(last)) = (env.points.first(), env.points.last()) {
        let mut line = vec![Pos2::new(graph.left(), y_of(first.1))];
        for (i, &(pos, t)) in env.points.iter().enumerate() {
            if env.steps && i > 0 {
                line.push(Pos2::new(x_of(pos), y_of(env.points[i - 1].1)));
            }
            if env.curve && !env.steps && i > 0 {
                // Trace the curve between the points.
                let from = env.points[i - 1].0;
                for k in 1..24 {
                    let at = from + (pos - from) * k as f32 / 24.0;
                    line.push(Pos2::new(x_of(at), y_of(env.value_at(at).unwrap_or(t))));
                }
            }
            line.push(Pos2::new(x_of(pos), y_of(t)));
        }
        line.push(Pos2::new(graph.right(), y_of(last.1)));
        painter.add(egui::Shape::line(line, Stroke::new(1.5, theme::SELECTED)));
    }

    // Interaction: points are hit within a few pixels. They snap to lines,
    // or to sixteenths of a short envelope.
    let snap = !ui.input(|i| i.modifiers.shift);
    let grid_step = if lines >= 16.0 { 1.0 } else { lines / 16.0 };
    let place = |p: Pos2| {
        let pos = ((p.x - graph.left()) / graph.width() * lines).clamp(0.0, lines);
        let pos = if snap { (pos / grid_step).round() * grid_step } else { pos };
        (pos, ((graph.bottom() - p.y) / graph.height()).clamp(0.0, 1.0))
    };
    let hit = |p: Pos2, env: &Envelope| {
        env.points.iter().position(|&(pos, t)| Pos2::new(x_of(pos), y_of(t)).distance(p) < 7.0)
    };
    if resp.drag_started() {
        app.automation.dragging = ui.input(|i| i.pointer.press_origin()).and_then(|p| hit(p, &env));
    }
    if resp.dragged()
        && let Some(p) = resp.interact_pointer_pos()
    {
        let (pos, t) = place(p);
        match app.automation.dragging {
            // A point keeps its place among its neighbours.
            Some(i) if i < env.points.len() => {
                let step = if snap { grid_step } else { 0.01 };
                let lo = if i > 0 { env.points[i - 1].0 + step } else { 0.0 };
                let hi = env.points.get(i + 1).map_or(lines, |n| n.0 - step);
                let pos = if lo <= hi { pos.clamp(lo, hi) } else { env.points[i].0 };
                env.points[i] = (pos, t);
            }
            _ => env.set(pos, t),
        }
    }
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
        && hit(p, &env).is_none()
    {
        let (pos, t) = place(p);
        env.set(pos, t);
    }
    if (resp.secondary_clicked() || resp.double_clicked())
        && let Some(i) = resp.interact_pointer_pos().and_then(|p| hit(p, &env))
    {
        env.points.remove(i);
    }
    // What is under the pointer now: a point may have just gone.
    let hovered = resp.hover_pos().and_then(|p| hit(p, &env));

    for (i, &(pos, t)) in env.points.iter().enumerate() {
        let r = Rect::from_center_size(Pos2::new(x_of(pos), y_of(t)), Vec2::splat(7.0));
        let hot = hovered == Some(i) || (resp.dragged() && app.automation.dragging == Some(i));
        painter.rect(
            r,
            1.0,
            if hot { Color32::WHITE } else { theme::SELECTED },
            Stroke::new(1.0, theme::SELECTED_TEXT),
            StrokeKind::Inside,
        );
    }
    if let Some(p) = resp.hover_pos()
        && graph.expand(4.0).contains(p)
    {
        let (pos, t) = hovered.map_or_else(|| place(p), |i| env.points[i]);
        let how =
            "Click to add a point, drag to draw or move one, right-click to delete; Shift places between the grid";
        let at = format!("Line {pos:.2} · {}\n{how}", spec.format(spec.value_at(t)));
        resp.clone().on_hover_text_at_pointer(at);
    }

    if env != before {
        match index {
            Some(i) => app.pattern_mut().automation[i] = env,
            None => app.pattern_mut().automation.push(env),
        }
        app.mark();
    }
}

/// Beat and bar lines with their line numbers, and the parameter's value
/// at five heights.
fn grid(app: &App, painter: &egui::Painter, graph: Rect, spec: &ParamSpec, start: f32, lines: f32) {
    let lpb = app.project.lpb.max(1) as usize;
    let font = FontId::monospace(9.5);
    for l in (0..=lines as usize).step_by(lpb) {
        let x = graph.left() + l as f32 / lines * graph.width();
        let bar = l % (lpb * 4) == 0;
        let color = Color32::from_gray(if bar { 52 } else { 32 });
        painter.line_segment([Pos2::new(x, graph.top()), Pos2::new(x, graph.bottom())], (1.0, color));
        if bar {
            painter.text(
                Pos2::new(x, graph.bottom() + 2.0),
                Align2::CENTER_TOP,
                format!("{:03}", l + start as usize),
                font.clone(),
                theme::TEXT_WEAK,
            );
        }
    }
    for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
        let y = graph.bottom() - t * graph.height();
        painter.line_segment([Pos2::new(graph.left(), y), Pos2::new(graph.right(), y)], (1.0, Color32::from_gray(32)));
        let at = Pos2::new(graph.left() - 4.0, y);
        // The ends sit inside the graph's height, clear of the line numbers.
        let align = match t {
            0.0 => Align2::RIGHT_BOTTOM,
            1.0 => Align2::RIGHT_TOP,
            _ => Align2::RIGHT_CENTER,
        };
        painter.text(at, align, spec.format(spec.value_at(t)), font.clone(), theme::TEXT_WEAK);
    }
}

/// How often an envelope can repeat, in beats, and their names.
const EVERY: &[(f32, &str)] = &[
    (0.0625, "1/16 beat"),
    (0.125, "1/8 beat"),
    (0.25, "1/4 beat"),
    (0.5, "1/2 beat"),
    (1.0, "1 beat"),
    (2.0, "2 beats"),
    (4.0, "1 bar"),
    (8.0, "2 bars"),
    (16.0, "4 bars"),
];

/// When envelope `env` takes effect in a pattern of `pattern_lines` at
/// `lpb` lines a beat: from one line to another, or over and over through
/// the whole pattern every so many beats. A change of length stretches
/// the drawing to keep its shape.
fn timing(ui: &mut egui::Ui, env: &mut Envelope, pattern_lines: usize, lpb: u32) {
    let before = env.span(pattern_lines, lpb);
    let repeats = env.every > 0.0;
    let tip = "From one line of the pattern to another; outside them the parameter keeps the song's value";
    if theme::toggle(ui, !repeats, "Once").on_hover_text(tip).clicked() {
        env.every = 0.0;
    }
    let tip = "Over and over through the whole pattern, every so many beats";
    if theme::toggle(ui, repeats, "Repeat").on_hover_text(tip).clicked() && !repeats {
        (env.every, env.start, env.lines) = (1.0, 0.0, 0.0);
    }
    ui.separator();
    if env.every > 0.0 {
        let name = EVERY.iter().find(|e| e.0 == env.every).map_or("", |e| e.1);
        egui::ComboBox::from_id_salt("envelope_every").selected_text(format!("Every {name}")).width(130.0).show_ui(
            ui,
            |ui| {
                for &(beats, name) in EVERY {
                    ui.selectable_value(&mut env.every, beats, name);
                }
            },
        );
    } else {
        let max = pattern_lines as f32;
        let whole = |v: f64, _| if v.fract() == 0.0 { format!("{v:.0}") } else { format!("{v:.2}") };
        ui.label("From");
        let mut to = if env.lines > 0.0 { env.start + env.lines } else { max };
        let from = egui::DragValue::new(&mut env.start).range(0.0..=max - 1.0).speed(0.25).custom_formatter(whole);
        ui.add(from).on_hover_text("The line it starts on");
        ui.label("To");
        let field = egui::DragValue::new(&mut to).range(env.start + 0.25..=max).speed(0.25).custom_formatter(whole);
        ui.add(field).on_hover_text("The line it ends on, where the next starts");
        let to = to.max(env.start + 0.25);
        env.lines = if to >= max { 0.0 } else { to - env.start };
    }
    let after = env.span(pattern_lines, lpb);
    if after != before {
        let scale = after / before;
        env.points.iter_mut().for_each(|p| p.0 *= scale);
    }
}

/// Draws one cycle of an LFO shape (`LFO_SHAPES`) into `env` over its
/// lines, or over each repeat, swinging a quarter of the parameter's range
/// either side of `around`, where the parameter is (0..1 along its bar).
fn shape_menu(ui: &mut egui::Ui, env: &mut Envelope, pattern_lines: usize, lpb: u32, around: f32) {
    ui.menu_button("Shape", |ui| {
        for (k, name) in LFO_SHAPES.iter().enumerate() {
            if ui.button(*name).clicked() {
                let (points, steps, curve) = lfo_points(k, env.span(pattern_lines, lpb), around);
                (env.points, env.steps, env.curve) = (points, steps, curve);
                ui.close();
            }
        }
    })
    .response
    .on_hover_text("Draw one cycle of a shape, as an LFO would play it; on Repeat it plays every interval");
}

/// The points of one cycle of LFO shape `shape` over `span` lines, around
/// `around`, and whether they are steps or a curve.
pub(super) fn lfo_points(shape: usize, span: f32, around: f32) -> (Vec<(f32, f32)>, bool, bool) {
    let (lo, hi) = ((around - 0.25).max(0.0), (around + 0.25).min(1.0));
    let mid = (lo + hi) / 2.0;
    let at = |f: f32, v: f32| (f * span, v);
    match shape {
        0 => (vec![at(0.0, mid), at(0.25, hi), at(0.5, mid), at(0.75, lo), at(1.0, mid)], false, true),
        1 => (vec![at(0.0, lo), at(0.5, hi), at(1.0, lo)], false, false),
        2 => (vec![at(0.0, hi), at(0.5, lo)], true, false),
        3 => (vec![at(0.0, hi), at(1.0, lo)], false, false),
        _ => (vec![at(0.0, lo), at(1.0, hi)], false, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_draw_one_cycle_around_the_value() {
        // A sine over 4 lines around the middle: up a quarter, then down.
        let (points, steps, curve) = lfo_points(0, 4.0, 0.5);
        let mut env = Envelope::new(1, 0, points);
        (env.steps, env.curve) = (steps, curve);
        assert_eq!(env.value_at(1.0), Some(0.75));
        assert_eq!(env.value_at(3.0), Some(0.25));
        assert_eq!(env.value_at(4.0), Some(0.5), "and back where it started");
        // Near the top of the range it stays inside it.
        let (points, ..) = lfo_points(4, 8.0, 0.9);
        assert!(points.iter().all(|p| (0.0..=1.0).contains(&p.1)) && points[1] == (8.0, 1.0));
    }
}
