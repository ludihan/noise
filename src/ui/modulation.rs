//! The Sampler's Modulation page:
//! a pitch envelope, a filter with its own envelope, and vibrato and
//! tremolo, for every voice.

use super::{App, theme, widgets};
use crate::project::{
    FILTER_CUTOFF, FILTER_ENV_AMOUNT, FILTER_MODES, FILTER_RESONANCE, LFO_DELAY, LFO_RATE, LFO_SHAPES, Modulation,
    PITCH_AMOUNT, ParamSpec, TREMOLO_DEPTH, VIBRATO_DEPTH, VoiceEnvelope, VoiceLfo, set_point,
};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};

#[derive(Clone, Copy, PartialEq, Default)]
pub enum Page {
    #[default]
    Pitch,
    Filter,
    Lfo,
}

/// State of the Modulation page between frames.
#[derive(Default)]
pub struct ModulationView {
    page: Page,
    /// The point being dragged, or `None` while drawing.
    dragging: Option<usize>,
    /// Seconds the graph spans, kept while dragging so it holds still.
    span: Option<f32>,
}

pub fn page(app: &mut App, ui: &mut egui::Ui, id: u8) {
    let Some(mut m) = app.project.module(id).map(|m| m.modulation.clone()) else { return };
    let before = m.clone();
    ui.horizontal(|ui| {
        for (page, label, tip) in [
            (Page::Pitch, "Pitch", "An envelope that bends the pitch of each note"),
            (Page::Filter, "Filter", "A filter on each voice, with an envelope on its cutoff"),
            (Page::Lfo, "LFOs", "Vibrato and tremolo"),
        ] {
            let on = match page {
                Page::Pitch => m.pitch.on,
                Page::Filter => m.filter,
                Page::Lfo => m.vibrato.on || m.tremolo.on,
            };
            let text = if on { format!("{label} •") } else { label.to_string() };
            if theme::toggle(ui, app.modulation.page == page, text).on_hover_text(tip).clicked() {
                app.modulation.page = page;
            }
        }
        let hint = "Play notes to hear it. Every voice of this instrument follows it.";
        ui.add(egui::Label::new(RichText::new(hint).small().color(theme::TEXT_WEAK)).truncate()).on_hover_text(hint);
    });
    ui.add_space(2.0);
    match app.modulation.page {
        Page::Pitch => pitch(app, ui, id, &mut m),
        Page::Filter => filter(app, ui, id, &mut m),
        Page::Lfo => lfos(ui, &mut m),
    }
    if m != before {
        app.project.module_mut(id).unwrap().modulation = m;
        app.mark();
    }
}

/// A parameter bar of a fixed width.
fn bar(ui: &mut egui::Ui, spec: &ParamSpec, value: &mut f32, width: f32) {
    ui.allocate_ui(Vec2::new(width, 18.0), |ui| widgets::param_bar(ui, spec, value, |_| {}));
}

/// Where the Sampler's sounding notes are in envelope `which` (0 pitch,
/// 1 filter), in seconds.
fn voices(app: &App, id: u8, which: usize) -> Vec<f32> {
    app.playheads().iter().filter(|p| p.module == id && p.level > 0.001).map(|p| p.envelopes[which]).collect()
}

fn pitch(app: &mut App, ui: &mut egui::Ui, id: u8, m: &mut Modulation) {
    ui.horizontal(|ui| {
        if theme::toggle(ui, m.pitch.on, "On").on_hover_text("Bend the pitch with this envelope").clicked() {
            m.pitch.on = !m.pitch.on;
        }
        bar(ui, &PITCH_AMOUNT, &mut m.pitch.amount, 200.0);
        envelope_options(ui, &mut m.pitch);
    });
    let range = m.pitch.amount;
    let label = move |t: f32| format!("{:+.1} st", (t - 0.5) * 2.0 * range);
    let notes = voices(app, id, 0);
    graph(app, ui, &mut m.pitch, &label, &notes);
}

fn filter(app: &mut App, ui: &mut egui::Ui, id: u8, m: &mut Modulation) {
    ui.horizontal(|ui| {
        if theme::toggle(ui, m.filter, "On").on_hover_text("Filter every voice").clicked() {
            m.filter = !m.filter;
        }
        egui::ComboBox::from_id_salt("voice_filter_mode")
            .selected_text(FILTER_MODES[m.filter_mode as usize])
            .width(90.0)
            .show_ui(ui, |ui| {
                for (i, mode) in FILTER_MODES.iter().enumerate() {
                    ui.selectable_value(&mut m.filter_mode, i as u8, *mode);
                }
            });
        bar(ui, &FILTER_CUTOFF, &mut m.cutoff, 180.0);
        bar(ui, &FILTER_RESONANCE, &mut m.resonance, 160.0);
    });
    ui.horizontal(|ui| {
        let tip = "Move the cutoff with this envelope";
        if theme::toggle(ui, m.filter_env.on, "Envelope").on_hover_text(tip).clicked() {
            m.filter_env.on = !m.filter_env.on;
        }
        bar(ui, &FILTER_ENV_AMOUNT, &mut m.filter_env.amount, 200.0);
        envelope_options(ui, &mut m.filter_env);
    });
    let (cutoff, amount) = (m.cutoff, m.filter_env.amount);
    let label = move |t: f32| FILTER_CUTOFF.format((cutoff * 2f32.powf(t * amount)).clamp(20.0, 20000.0));
    let notes = voices(app, id, 1);
    graph(app, ui, &mut m.filter_env, &label, &notes);
}

/// The curve switch and the sustain hint above an envelope.
fn envelope_options(ui: &mut egui::Ui, env: &mut VoiceEnvelope) {
    if theme::toggle(ui, env.curve, "Curve").on_hover_text("Move along a smooth curve through the points").clicked() {
        env.curve = !env.curve;
    }
    let hint =
        "Click to add a point, drag to move one, right-click to delete, double-click to make it the sustain point.";
    ui.add(egui::Label::new(RichText::new(hint).small().color(theme::TEXT_WEAK)).truncate()).on_hover_text(hint);
}

fn lfos(ui: &mut egui::Ui, m: &mut Modulation) {
    ui.horizontal_top(|ui| {
        lfo(ui, "VIBRATO", "Wobble the pitch", &mut m.vibrato, &VIBRATO_DEPTH);
        ui.separator();
        lfo(ui, "TREMOLO", "Wobble the volume", &mut m.tremolo, &TREMOLO_DEPTH);
    });
}

fn lfo(ui: &mut egui::Ui, title: &str, tip: &str, lfo: &mut VoiceLfo, depth: &ParamSpec) {
    ui.vertical(|ui| {
        ui.set_width(260.0);
        ui.horizontal(|ui| {
            theme::caption(ui, title);
            if theme::toggle(ui, lfo.on, "On").on_hover_text(tip).clicked() {
                lfo.on = !lfo.on;
            }
            egui::ComboBox::from_id_salt(("voice_lfo_shape", title))
                .selected_text(LFO_SHAPES[lfo.shape as usize])
                .width(90.0)
                .show_ui(ui, |ui| {
                    for (i, shape) in LFO_SHAPES.iter().enumerate() {
                        ui.selectable_value(&mut lfo.shape, i as u8, *shape);
                    }
                });
        });
        ui.spacing_mut().item_spacing.y = 3.0;
        bar(ui, &LFO_RATE, &mut lfo.rate, 260.0);
        bar(ui, depth, &mut lfo.depth, 260.0);
        bar(ui, &LFO_DELAY, &mut lfo.delay, 260.0);
        let hint = "Delay: it starts after this long into each note, fading in over as long again.";
        ui.label(RichText::new(hint).small().color(theme::TEXT_WEAK));
    });
}

/// An envelope over seconds: points to add, drag and delete, and the
/// sustain point a held note waits at. `notes` are where sounding notes
/// are in it.
fn graph(app: &mut App, ui: &mut egui::Ui, env: &mut VoiceEnvelope, label: &dyn Fn(f32) -> String, notes: &[f32]) {
    let (rect, resp) = ui.allocate_exact_size(ui.available_size().max(Vec2::new(200.0, 80.0)), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, theme::INSET);
    let graph = Rect::from_min_max(rect.min + Vec2::new(62.0, 8.0), rect.max - Vec2::new(10.0, 16.0));
    let view = &mut app.modulation;
    // The graph shows a bit past the last point, at least a second.
    let fit = (env.length() * 1.25).max(1.0);
    let span = match view.span {
        Some(s) if resp.dragged() => s,
        _ => {
            view.span = Some(fit);
            fit
        }
    };
    let x_of = |t: f32| graph.left() + t / span * graph.width();
    let y_of = |v: f32| graph.bottom() - v * graph.height();
    let dim = if env.on { 1.0 } else { 0.45 };

    // Seconds along the bottom and values up the side.
    let font = FontId::monospace(9.5);
    let step = [0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0].into_iter().find(|s| span / s <= 12.0).unwrap_or(10.0);
    let mut t = 0.0;
    while t <= span + 1e-4 {
        let x = x_of(t);
        painter.line_segment([Pos2::new(x, graph.top()), Pos2::new(x, graph.bottom())], (1.0, Color32::from_gray(32)));
        let text = if t < 1.0 && t > 0.0 { format!("{:.0} ms", t * 1000.0) } else { format!("{t:.1} s") };
        painter.text(Pos2::new(x, graph.bottom() + 2.0), Align2::CENTER_TOP, text, font.clone(), theme::TEXT_WEAK);
        t += step;
    }
    for v in [0.0, 0.25, 0.5, 0.75, 1.0] {
        let y = y_of(v);
        painter.line_segment([Pos2::new(graph.left(), y), Pos2::new(graph.right(), y)], (1.0, Color32::from_gray(32)));
        painter.text(Pos2::new(graph.left() - 4.0, y), Align2::RIGHT_CENTER, label(v), font.clone(), theme::TEXT_WEAK);
    }

    // The sustain point.
    if let Some(&(t, _)) = env.sustain.and_then(|i| env.points.get(i)) {
        let x = x_of(t);
        let mut y = graph.top();
        while y < graph.bottom() {
            painter
                .line_segment([Pos2::new(x, y), Pos2::new(x, (y + 4.0).min(graph.bottom()))], (1.0, theme::PAT_EFFECT));
            y += 8.0;
        }
        painter.text(Pos2::new(x + 3.0, graph.top()), Align2::LEFT_TOP, "sustain", font.clone(), theme::PAT_EFFECT);
    }

    // A line for each sounding note, as the waveform shows playheads.
    if env.on {
        for &t in notes.iter().filter(|&&t| t <= span) {
            let x = x_of(t);
            painter.line_segment(
                [Pos2::new(x, graph.top()), Pos2::new(x, graph.bottom())],
                (1.0, theme::SCOPE.gamma_multiply(0.7)),
            );
            painter.circle_filled(Pos2::new(x, y_of(env.value(t))), 3.0, theme::SCOPE);
        }
    }

    // The envelope, held flat after its last point.
    let color = theme::SELECTED.gamma_multiply(dim);
    let mut line = Vec::new();
    let samples = 160;
    for k in 0..=samples {
        let t = span * k as f32 / samples as f32;
        line.push(Pos2::new(x_of(t), y_of(env.value(t))));
    }
    painter.add(egui::Shape::line(line, Stroke::new(1.5, color)));

    let hit = |p: Pos2, env: &VoiceEnvelope| {
        env.points.iter().position(|&(t, v)| Pos2::new(x_of(t), y_of(v)).distance(p) < 7.0)
    };
    let place = |p: Pos2| {
        let t = ((p.x - graph.left()) / graph.width() * span).max(0.0);
        (t, ((graph.bottom() - p.y) / graph.height()).clamp(0.0, 1.0))
    };
    let hovered = resp.hover_pos().and_then(|p| hit(p, env));
    if resp.drag_started() {
        view.dragging = ui.input(|i| i.pointer.press_origin()).and_then(|p| hit(p, env));
    }
    if resp.dragged()
        && let Some(p) = resp.interact_pointer_pos()
    {
        let (t, v) = place(p);
        match view.dragging {
            // A point keeps its place among its neighbours; the first stays
            // at the start of the note.
            Some(i) if i < env.points.len() => {
                let lo = if i > 0 { env.points[i - 1].0 + 0.001 } else { 0.0 };
                let hi = env.points.get(i + 1).map_or(f32::MAX, |n| n.0 - 0.001);
                let t = if i == 0 {
                    0.0
                } else if lo <= hi {
                    t.clamp(lo, hi)
                } else {
                    env.points[i].0
                };
                env.points[i] = (t, v);
            }
            _ => add_point(env, t, v),
        }
    }
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
        && hit(p, env).is_none()
    {
        let (t, v) = place(p);
        add_point(env, t, v);
    }
    if resp.double_clicked()
        && let Some(i) = resp.interact_pointer_pos().and_then(|p| hit(p, env))
    {
        env.sustain = if env.sustain == Some(i) { None } else { Some(i) };
    }
    if resp.secondary_clicked()
        && let Some(i) = resp.interact_pointer_pos().and_then(|p| hit(p, env))
        && env.points.len() > 1
    {
        env.points.remove(i);
        env.sustain = match env.sustain {
            Some(s) if s == i => None,
            Some(s) if s > i => Some(s - 1),
            s => s,
        };
    }

    for (i, &(t, v)) in env.points.iter().enumerate() {
        let r = Rect::from_center_size(Pos2::new(x_of(t), y_of(v)), Vec2::splat(7.0));
        let hot = hovered == Some(i) || (resp.dragged() && view.dragging == Some(i));
        let fill = if hot {
            Color32::WHITE
        } else if env.sustain == Some(i) {
            theme::PAT_EFFECT
        } else {
            color
        };
        painter.rect(r, 1.0, fill, Stroke::new(1.0, theme::SELECTED_TEXT), StrokeKind::Inside);
    }
    if let Some(p) = resp.hover_pos()
        && graph.expand(4.0).contains(p)
    {
        let (t, v) = hovered.map_or_else(|| place(p), |i| env.points[i]);
        resp.on_hover_text_at_pointer(format!("{:.0} ms · {}", t * 1000.0, label(v)));
    }
}

/// Adds or moves a point, keeping the sustain point on the same one.
fn add_point(env: &mut VoiceEnvelope, t: f32, v: f32) {
    let held = env.sustain.and_then(|i| env.points.get(i).copied());
    set_point(&mut env.points, t, v);
    if let Some((ht, _)) = held {
        env.sustain = env.points.iter().position(|p| p.0 == ht);
    }
}
