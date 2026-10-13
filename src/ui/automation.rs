//! Automation: envelopes that move module parameters over
//! the course of a pattern, drawn in the lower frame.

use super::{App, theme};
use crate::project::{Envelope, LFO_SHAPES, ParamSpec};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};

pub struct AutomationView {
    /// The parameter whose envelope is shown, by module and parameter; it
    /// may have none in this pattern yet.
    selected: Option<(u8, usize)>,
    /// What the pointer is doing in the graph, from when it was pressed.
    drag: Option<Drag>,
    /// The points picked, by index, and the envelope (pattern, module and
    /// parameter) they are points of.
    picked: Vec<usize>,
    picked_of: Option<(usize, u8, usize)>,
    /// An envelope copied, to paste onto another parameter or pattern.
    clipboard: Option<Envelope>,
    /// How far the Shape menu's shapes swing either side of the
    /// parameter, of its range, and how many cycles they draw.
    shape_swing: f32,
    shape_cycles: u32,
    /// Text the parameter browser is narrowed to.
    filter: String,
    /// The browser shows every parameter, not just the automated ones.
    pub all: bool,
    /// The pattern the browser last looked at: a new one starts on its
    /// automated parameters, or on all of them if it has none.
    seen_pattern: Option<usize>,
}

impl Default for AutomationView {
    fn default() -> Self {
        Self {
            selected: None,
            drag: None,
            picked: Vec::new(),
            picked_of: None,
            clipboard: None,
            shape_swing: 0.25,
            shape_cycles: 1,
            filter: String::new(),
            all: false,
            seen_pattern: None,
        }
    }
}

impl AutomationView {
    /// Forgets the parameter shown and the points picked of modules that
    /// are `gone`.
    pub fn forget(&mut self, gone: impl Fn(u8) -> bool) {
        self.selected = self.selected.filter(|&(id, _)| !gone(id));
        if self.picked_of.is_some_and(|(_, id, _)| gone(id)) {
            (self.picked_of, self.picked) = (None, Vec::new());
        }
    }
}

/// What dragging in the graph does.
enum Drag {
    /// Moves the picked points, which were at `from` when it began at
    /// `origin` (in lines and 0..1).
    Points { origin: (f32, f32), from: Vec<(f32, f32)>, picked: Vec<usize> },
    /// Picks the points in a box from this corner.
    Box(Pos2),
    /// Draws points along the way.
    Draw,
}

/// The browser's groups: a heading and the modules under it.
fn groups(app: &App) -> Vec<(String, Vec<u8>)> {
    let p = &app.project;
    let mut ids: Vec<u8> = p.modules.iter().map(|m| m.id).collect();
    ids.sort();
    let kind = |id: u8| p.module(id).map(|m| m.kind);
    let effect = |id: u8| kind(id).is_some_and(|k| k.is_effect());
    let pick = |f: &dyn Fn(u8) -> bool| ids.iter().copied().filter(|&id| f(id)).collect::<Vec<u8>>();
    let mut groups = vec![
        ("Instruments".to_string(), pick(&|id| kind(id).is_some_and(|k| k.in_instrument_list()))),
        ("Instrument effects".to_string(), pick(&|id| effect(id) && !p.placed(id))),
    ];
    for (t, track) in p.tracks.iter().enumerate().filter(|(_, t)| !t.effects.is_empty()) {
        groups.push((format!("Track {:02} · {}", t + 1, p.track_name(t)), track.effects.clone()));
    }
    for send in &p.sends {
        let name = p.module(send.module).map_or(String::new(), |m| m.name.clone());
        let ids = send.effects.iter().copied().chain([send.module]).collect();
        groups.push((format!("Send track · {name}"), ids));
    }
    let mut master = p.master.clone();
    master.push(crate::project::OUTPUT_ID);
    groups.push(("Master".to_string(), master));
    groups.push(("Modulators".to_string(), pick(&|id| kind(id).is_some_and(|k| k.controls()))));
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
                                // A Modulator moving it is marked after its name.
                                let moved = !app.project.modulated_by(id, i).is_empty();
                                let text = format!(
                                    "{}{}{}",
                                    if on { "• " } else { "" },
                                    m.automatable_name(i),
                                    if moved { "  ~" } else { "" }
                                );
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
        app.select(Some(p.0));
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
    // Picked points belong to one envelope; another starts with none.
    let of = Some((app.current_pattern_index(), module, param));
    if app.automation.picked_of != of {
        (app.automation.picked_of, app.automation.picked) = (of, Vec::new());
        app.automation.drag = None;
    }
    app.automation.picked.retain(|&i| i < env.points.len());

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
        ui.separator();
        let span = pattern_span(app, &before);
        edit_menu(ui, &mut env, &mut app.automation, span);
        let tip = "Copy this envelope, its points and timing, to paste onto another parameter or pattern";
        if ui.add_enabled(!env.points.is_empty(), egui::Button::new("Copy")).on_hover_text(tip).clicked() {
            app.automation.clipboard = Some(env.clone());
        }
        let tip = "Put the envelope copied in place of this one";
        if ui.add_enabled(app.automation.clipboard.is_some(), egui::Button::new("Paste")).on_hover_text(tip).clicked()
            && let Some(copied) = &app.automation.clipboard
        {
            env = copied.clone();
            (env.module, env.param) = (module, param);
            app.automation.picked.clear();
        }
    });
    // Which lines it takes effect in, on a row of its own.
    let (pattern_lines, lpb) = (app.pattern().lines, app.project.lpb.max(1));
    let around = spec.position(value);
    ui.horizontal(|ui| {
        timing(ui, &mut env, pattern_lines, lpb);
        ui.separator();
        shape_menu(ui, &mut env, &mut app.automation, pattern_lines, lpb, around);
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
    let play_line = play_line as f32 + app.play_line_frac();
    let playing_here =
        app.is_playing() && app.project.order.get(play_slot).is_some_and(|s| s.pattern == app.current_pattern_index());
    let mark = |pos: f32, color: Color32| {
        painter.line_segment([Pos2::new(x_of(pos), graph.top()), Pos2::new(x_of(pos), graph.bottom())], (1.0, color));
    };
    if let Some(at) = env.local(app.cursor.line as f32, pattern_lines, lpb) {
        mark(at, theme::SELECTED.gamma_multiply(0.35));
    }
    if let Some(at) = env.local(play_line, pattern_lines, lpb).filter(|_| playing_here) {
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
    let (snap, add) = ui.input(|i| (!i.modifiers.shift, i.modifiers.command));
    let grid_step = if lines >= 16.0 { 1.0 } else { lines / 16.0 };
    let place = |p: Pos2| {
        let pos = ((p.x - graph.left()) / graph.width() * lines).clamp(0.0, lines);
        let pos = if snap { (pos / grid_step).round() * grid_step } else { pos };
        (pos, ((graph.bottom() - p.y) / graph.height()).clamp(0.0, 1.0))
    };
    let unsnapped = |p: Pos2| ((p.x - graph.left()) / graph.width() * lines, (graph.bottom() - p.y) / graph.height());
    let hit = |p: Pos2, env: &Envelope| {
        env.points.iter().position(|&(pos, t)| Pos2::new(x_of(pos), y_of(t)).distance(p) < 7.0)
    };
    let view = &mut app.automation;
    if resp.drag_started()
        && let Some(p) = ui.input(|i| i.pointer.press_origin())
    {
        view.drag = Some(match hit(p, &env) {
            // A point dragged moves with the others picked, or alone.
            Some(i) => {
                if !view.picked.contains(&i) {
                    view.picked = vec![i];
                }
                Drag::Points { origin: unsnapped(p), from: env.points.clone(), picked: view.picked.clone() }
            }
            None if add => Drag::Box(p),
            None => {
                view.picked.clear();
                Drag::Draw
            }
        });
    }
    if resp.dragged()
        && let Some(p) = resp.interact_pointer_pos()
    {
        match &view.drag {
            Some(Drag::Points { origin, from, picked }) => {
                let now = unsnapped(p);
                let mut dx = now.0 - origin.0;
                if snap {
                    dx = (dx / grid_step).round() * grid_step;
                }
                let dy = now.1 - origin.1;
                (env.points, view.picked) = moved(from, picked, dx, dy, lines);
            }
            Some(Drag::Box(corner)) => {
                let area = Rect::from_two_pos(*corner, p);
                painter.rect(
                    area,
                    0.0,
                    theme::SELECTED.gamma_multiply(0.08),
                    (1.0, theme::SELECTED),
                    StrokeKind::Inside,
                );
                view.picked = (0..env.points.len())
                    .filter(|&i| area.contains(Pos2::new(x_of(env.points[i].0), y_of(env.points[i].1))))
                    .collect();
            }
            _ => {
                let (pos, t) = place(p);
                env.set(pos, t);
            }
        }
    }
    if resp.drag_stopped() {
        view.drag = None;
    }
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        match hit(p, &env) {
            // Ctrl-click picks a point or lets it go; a click picks it alone.
            Some(i) if add => {
                if let Some(k) = view.picked.iter().position(|&j| j == i) {
                    view.picked.remove(k);
                } else {
                    view.picked.push(i);
                }
            }
            Some(i) => view.picked = vec![i],
            None => {
                let (pos, t) = place(p);
                env.set(pos, t);
                view.picked.clear();
            }
        }
    }
    if (resp.secondary_clicked() || resp.double_clicked())
        && let Some(i) = resp.interact_pointer_pos().and_then(|p| hit(p, &env))
    {
        // Right-clicking a picked point deletes every one picked.
        let doomed = if view.picked.contains(&i) { std::mem::take(&mut view.picked) } else { vec![i] };
        env.points = (env.points.iter().enumerate()).filter(|(k, _)| !doomed.contains(k)).map(|(_, p)| *p).collect();
        view.picked.clear();
    }
    let picked = view.picked.clone();
    // What is under the pointer now: a point may have just gone.
    let hovered = resp.hover_pos().and_then(|p| hit(p, &env));

    for (i, &(pos, t)) in env.points.iter().enumerate() {
        let r = Rect::from_center_size(Pos2::new(x_of(pos), y_of(t)), Vec2::splat(7.0));
        let hot = hovered == Some(i) || picked.contains(&i);
        painter.rect(
            r,
            1.0,
            if hot { Color32::WHITE } else { theme::SELECTED },
            Stroke::new(1.0, theme::SELECTED_TEXT),
            StrokeKind::Inside,
        );
    }
    // While it plays here, a dot where the envelope has the parameter.
    if let Some(at) = env.local(play_line, pattern_lines, lpb).filter(|_| playing_here)
        && let Some(t) = env.value_at(at)
    {
        let dot = Pos2::new(x_of(at), y_of(t));
        painter.circle_filled(dot, 4.0, theme::SCOPE);
        painter.text(
            dot + Vec2::new(7.0, -7.0),
            Align2::LEFT_BOTTOM,
            spec.format(spec.value_at(t)),
            FontId::monospace(10.0),
            theme::SCOPE,
        );
    }
    if let Some(p) = resp.hover_pos()
        && graph.expand(4.0).contains(p)
    {
        let (pos, t) = hovered.map_or_else(|| place(p), |i| env.points[i]);
        let how = "Click to add a point, drag to draw or move one, right-click to delete; Ctrl-click or Ctrl-drag picks several to move or delete together; Shift places between the grid";
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

/// Picks points `picked` of the envelope shown, as the pictures do.
#[cfg(test)]
pub(super) fn pick(app: &mut App, picked: &[usize]) {
    let first = app.pattern().automation.first().map(|e| (e.module, e.param));
    app.automation.selected = first;
    app.automation.picked_of = first.map(|(m, p)| (app.current_pattern_index(), m, p));
    app.automation.seen_pattern = Some(app.current_pattern_index());
    app.automation.picked = picked.to_vec();
}

/// Lines a pattern's envelope `env` spans, for the edit menu.
fn pattern_span(app: &App, env: &Envelope) -> f32 {
    env.span(app.pattern().lines, app.project.lpb.max(1))
}

/// `points` with those at `picked` moved `dx` lines and `dy` of the range,
/// as a group kept inside `0..=lines` and `0..=1`, and where the picked
/// ones are once sorted again. A point they land on is replaced.
fn moved(points: &[(f32, f32)], picked: &[usize], dx: f32, dy: f32, lines: f32) -> (Vec<(f32, f32)>, Vec<usize>) {
    let group = || picked.iter().filter_map(|&i| points.get(i));
    let (lo, hi) = group().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.0), hi.max(p.0)));
    let (bottom, top) = group().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
    if lo > hi {
        return (points.to_vec(), Vec::new());
    }
    let dx = dx.clamp(-lo, lines - hi);
    let dy = dy.clamp(-bottom, 1.0 - top);
    let mut all: Vec<((f32, f32), bool)> = points
        .iter()
        .enumerate()
        .map(|(i, &(pos, t))| if picked.contains(&i) { ((pos + dx, t + dy), true) } else { ((pos, t), false) })
        .collect();
    // Picked points win where they land on another.
    let landed: Vec<f32> = all.iter().filter(|p| p.1).map(|p| p.0.0).collect();
    all.retain(|&(p, on)| on || !landed.iter().any(|q| (q - p.0).abs() < 1e-4));
    all.sort_by(|a, b| a.0.0.total_cmp(&b.0.0));
    let picked = (all.iter().enumerate()).filter(|(_, p)| p.1).map(|(i, _)| i).collect();
    (all.into_iter().map(|p| p.0).collect(), picked)
}

/// Ways to change the whole envelope, or the points picked: turn it upside
/// down or back to front, thin out points it doesn't need, repeat it in
/// half the time, pick or delete points.
fn edit_menu(ui: &mut egui::Ui, env: &mut Envelope, view: &mut AutomationView, span: f32) {
    let picked = view.picked.clone();
    let some = !picked.is_empty();
    let on = |i: usize| !some || picked.contains(&i);
    ui.menu_button("Edit", |ui| {
        let which = if some { "the points picked" } else { "the envelope" };
        if ui.button("Invert").on_hover_text(format!("Turn {which} upside down")).clicked() {
            for (_, p) in env.points.iter_mut().enumerate().filter(|(i, _)| on(*i)) {
                p.1 = 1.0 - p.1;
            }
            ui.close();
        }
        if ui.button("Reverse").on_hover_text("Play the envelope back to front").clicked() {
            env.points = reversed(&env.points, span, env.steps);
            view.picked.clear();
            ui.close();
        }
        if ui.button("Thin Out").on_hover_text("Remove the points the envelope would pass through anyway").clicked() {
            env.points = thinned(&env.points, env.steps);
            view.picked.clear();
            ui.close();
        }
        let tip = "Squeeze the envelope into the first half of its lines and play it again in the second";
        if ui.button("Repeat Twice").on_hover_text(tip).clicked() {
            let half: Vec<(f32, f32)> = env.points.iter().map(|&(pos, t)| (pos / 2.0, t)).collect();
            env.points = half.iter().copied().chain(half.iter().map(|&(pos, t)| (pos + span / 2.0, t))).collect();
            env.points.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-4);
            view.picked.clear();
            ui.close();
        }
        ui.separator();
        if ui.button("Pick All").clicked() {
            view.picked = (0..env.points.len()).collect();
            ui.close();
        }
        if ui.add_enabled(some, egui::Button::new("Delete Picked")).clicked() {
            env.points =
                (env.points.iter().enumerate()).filter(|(i, _)| !picked.contains(i)).map(|(_, p)| *p).collect();
            view.picked.clear();
            ui.close();
        }
        if ui.button("Clear").on_hover_text("Remove every point").clicked() {
            env.points.clear();
            view.picked.clear();
            ui.close();
        }
    })
    .response
    .on_hover_text("Invert, reverse, thin out or repeat it; Ctrl-click points to work on only those");
}

/// `points` over `span` lines played back to front. Held points hold from
/// where they are to the next, so each is moved to where its hold ends.
fn reversed(points: &[(f32, f32)], span: f32, steps: bool) -> Vec<(f32, f32)> {
    let mut out: Vec<(f32, f32)> = if steps {
        (points.iter().enumerate()).map(|(i, &(_, t))| (span - points.get(i + 1).map_or(span, |n| n.0), t)).collect()
    } else {
        points.iter().map(|&(pos, t)| (span - pos, t)).collect()
    };
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-4);
    out
}

/// `points` without those the envelope passes through anyway: on the
/// straight line between their neighbours, or holding the value before.
fn thinned(points: &[(f32, f32)], steps: bool) -> Vec<(f32, f32)> {
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(points.len());
    for (i, &p) in points.iter().enumerate() {
        let Some(&prev) = out.last() else {
            out.push(p);
            continue;
        };
        let needless = match points.get(i + 1) {
            _ if steps => (p.1 - prev.1).abs() < 1e-3,
            Some(&next) => {
                let along = prev.1 + (next.1 - prev.1) * (p.0 - prev.0) / (next.0 - prev.0).max(1e-6);
                (p.1 - along).abs() < 1e-3
            }
            None => false,
        };
        if !needless {
            out.push(p);
        }
    }
    out
}

/// Draws cycles of an LFO shape (`LFO_SHAPES`, or random steps) into `env`
/// over its lines, or over each repeat, swinging as far either side of
/// `around`, where the parameter is (0..1 along its bar), as the menu says.
fn shape_menu(
    ui: &mut egui::Ui,
    env: &mut Envelope,
    view: &mut AutomationView,
    pattern_lines: usize,
    lpb: u32,
    around: f32,
) {
    ui.menu_button("Shape", |ui| {
        ui.horizontal(|ui| {
            ui.label("Swing");
            let field = egui::DragValue::new(&mut view.shape_swing)
                .range(0.02..=0.5)
                .speed(0.005)
                .custom_formatter(|v, _| format!("±{:.0}%", v * 100.0))
                .custom_parser(|t| {
                    t.trim_start_matches('±').trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)
                });
            ui.add(field).on_hover_text("How far either side of where the parameter is, of its range");
            ui.label("Cycles");
            ui.add(egui::DragValue::new(&mut view.shape_cycles).range(1..=16))
                .on_hover_text("How many times the shape plays over the lines, or each repeat");
        });
        ui.separator();
        let span = env.span(pattern_lines, lpb);
        let (swing, cycles) = (view.shape_swing, view.shape_cycles);
        for (k, name) in LFO_SHAPES.iter().enumerate() {
            if super::lfo::shape_button(ui, name, false, &|p| crate::dsp::lfo_shape(k as u32, p)).clicked() {
                (env.points, env.steps, env.curve) = shape_points(k, span, around, swing, cycles);
                view.picked.clear();
                ui.close();
            }
        }
        let random = |p: f32| crate::rng::Rng(1 + (p * 8.0) as u32 * 7919).next();
        if super::lfo::shape_button(ui, "Random steps", false, &random)
            .on_hover_text("Eight held values a cycle, picked at random")
            .clicked()
        {
            let seed = ui.input(|i| i.time.to_bits() as u32);
            (env.points, env.steps, env.curve) = random_points(seed, span, around, swing, cycles * 8);
            view.picked.clear();
            ui.close();
        }
    })
    .response
    .on_hover_text("Draw cycles of a shape, as an LFO would play it; on Repeat it plays every interval");
}

/// The points of one cycle of LFO shape `shape` over `span` lines, a
/// quarter of the range either side of `around`, and whether they are
/// steps or a curve.
#[cfg(test)]
pub(super) fn lfo_points(shape: usize, span: f32, around: f32) -> (Vec<(f32, f32)>, bool, bool) {
    shape_points(shape, span, around, 0.25, 1)
}

/// `cycles` of LFO shape `shape` over `span` lines, `swing` either side of
/// `around` (kept inside 0..1), and whether they are steps or a curve.
fn shape_points(shape: usize, span: f32, around: f32, swing: f32, cycles: u32) -> (Vec<(f32, f32)>, bool, bool) {
    let (lo, hi) = ((around - swing).max(0.0), (around + swing).min(1.0));
    let mid = (lo + hi) / 2.0;
    let (one, steps, curve): (&[(f32, f32)], bool, bool) = match shape {
        0 => (&[(0.0, mid), (0.25, hi), (0.5, mid), (0.75, lo)], false, true),
        1 => (&[(0.0, lo), (0.5, hi)], false, false),
        2 => (&[(0.0, hi), (0.5, lo)], true, false),
        // A saw jumps back as the next cycle starts, a hair before it.
        3 => (&[(0.0, hi), (0.999, lo)], false, false),
        _ => (&[(0.0, lo), (0.999, hi)], false, false),
    };
    let cycle = span / cycles.max(1) as f32;
    let mut points: Vec<(f32, f32)> =
        (0..cycles.max(1)).flat_map(|c| one.iter().map(move |&(f, v)| ((c as f32 + f) * cycle, v))).collect();
    // It ends where it began, the next cycle's first point.
    let end = match shape {
        3 => lo,
        4 => hi,
        _ => one[0].1,
    };
    if !steps {
        points.push((span, end));
    }
    if cycles <= 1 {
        // One cycle of a saw is a plain ramp.
        points.retain(|p| (p.0 - 0.999 * span).abs() > 1e-4 || !matches!(shape, 3 | 4));
    }
    (points, steps, curve)
}

/// `n` values held one after another over `span` lines, at random within
/// `swing` either side of `around`, from `seed`.
fn random_points(seed: u32, span: f32, around: f32, swing: f32, n: u32) -> (Vec<(f32, f32)>, bool, bool) {
    let (lo, hi) = ((around - swing).max(0.0), (around + swing).min(1.0));
    let mut rng = crate::rng::Rng(seed);
    let points = (0..n).map(|k| (k as f32 * span / n as f32, lo + (hi - lo) * rng.unit())).collect();
    (points, true, false)
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

    #[test]
    fn picked_points_move_together_inside_the_graph() {
        let points = [(0.0, 0.5), (4.0, 0.2), (8.0, 0.9), (12.0, 0.5)];
        // Two picked, moved 2 lines on and up: the top one stops at the top.
        let (moved, picked) = moved(&points, &[1, 2], 2.0, 0.5, 16.0);
        assert_eq!(moved, [(0.0, 0.5), (6.0, 0.3), (10.0, 1.0), (12.0, 0.5)]);
        assert_eq!(picked, [1, 2]);
        // Dragged over another, they pass it and replace one they land on.
        let (moved, picked) = super::moved(&points, &[1], 8.0, 0.0, 16.0);
        assert_eq!(moved, [(0.0, 0.5), (8.0, 0.9), (12.0, 0.2)]);
        assert_eq!(picked, [2]);
    }

    #[test]
    fn envelopes_reverse_and_thin_out() {
        let ramp = [(0.0, 0.0), (2.0, 0.25), (4.0, 0.5), (8.0, 1.0)];
        assert_eq!(thinned(&ramp, false), [(0.0, 0.0), (8.0, 1.0)], "a straight line needs its ends");
        assert_eq!(reversed(&ramp, 8.0, false)[0], (0.0, 1.0));
        // Held values keep how long they hold.
        let held = [(0.0, 0.1), (2.0, 0.9), (2.5, 0.9)];
        assert_eq!(thinned(&held, true), [(0.0, 0.1), (2.0, 0.9)]);
        assert_eq!(reversed(&[(0.0, 0.1), (2.0, 0.9)], 8.0, true), [(0.0, 0.9), (6.0, 0.1)]);
    }

    #[test]
    fn shapes_draw_as_many_cycles_as_asked() {
        // Two cycles of a triangle over 8 lines, swinging 0.1 around 0.5.
        let (points, ..) = shape_points(1, 8.0, 0.5, 0.1, 2);
        let mut env = Envelope::new(1, 0, points);
        assert!((env.value_at(2.0).unwrap() - 0.6).abs() < 1e-6 && (env.value_at(6.0).unwrap() - 0.6).abs() < 1e-6);
        assert!((env.value_at(4.0).unwrap() - 0.4).abs() < 1e-6);
        // A saw jumps back at each cycle.
        let (points, ..) = shape_points(4, 8.0, 0.5, 0.25, 2);
        env.points = points;
        assert!(env.value_at(3.99).unwrap() > 0.7 && env.value_at(4.0).unwrap() < 0.3);
        // Random steps stay inside their swing.
        let (points, steps, _) = random_points(7, 8.0, 0.5, 0.1, 8);
        assert!(steps && points.len() == 8 && points.iter().all(|p| (0.4..=0.6).contains(&p.1)));
    }
}
