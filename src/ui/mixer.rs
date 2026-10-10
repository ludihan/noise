//! The mixer. Audio flows through modules here rather than
//! tracks, so every module gets a channel strip, with the output as the
//! master strip on the right.

use super::{App, modules, theme, widgets};
use crate::project::{MIXER_PAN, ModuleKind, OUTPUT_ID, Owner};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};

const STRIP_W: f32 = 96.0;
/// The marks on the scale between fader and meters, in dB.
const MARKS: [f32; 6] = [6.0, 0.0, -6.0, -12.0, -24.0, -48.0];

/// Fader and meter position (0..1) of a linear gain or level. Amplitude
/// grows with the square of the position, so 0 dB sits at 71% and the top
/// is +6 dB.
fn pos(gain: f32) -> f32 {
    (gain.max(0.0) / 2.0).sqrt().min(1.0)
}

fn gain_at(t: f32) -> f32 {
    2.0 * t.clamp(0.0, 1.0).powi(2)
}

pub fn db_text(gain: f32) -> String {
    if gain < 1e-4 { "-inf dB".into() } else { format!("{:+.1} dB", 20.0 * gain.log10()) }
}

/// The strips in their sections: the instruments, by number; the effects,
/// chain by chain (the instruments' in their order, then the tracks', the
/// send tracks' and the master's); and the send tracks as they are
/// listed. Sends have no strips: their amounts say how loud they send.
fn sections(app: &App) -> [(&'static str, &'static str, Vec<u8>); 3] {
    let p = &app.project;
    let mut instruments: Vec<u8> =
        (p.modules.iter()).filter(|m| m.kind.in_instrument_list() && m.kind.makes_sound()).map(|m| m.id).collect();
    instruments.sort();
    let mut effects: Vec<u8> = instruments.iter().flat_map(|&i| p.chain(i).effects).collect();
    effects.extend(p.tracks.iter().flat_map(|t| t.effects.iter().copied()));
    effects.extend(p.sends.iter().flat_map(|s| s.effects.iter().copied()));
    effects.extend(&p.master);
    effects.retain(|&e| p.module(e).is_some_and(|m| m.kind != ModuleKind::Send));
    let sends = p.sends.iter().map(|s| s.module).collect();
    [
        ("INSTRUMENTS", "What each instrument plays, before its effects", instruments),
        ("EFFECTS", "Each effect's output, chain by chain", effects),
        ("SEND TRACKS", "What comes back from each send track, on its way to the master", sends),
    ]
}

pub fn view(app: &mut App, ui: &mut egui::Ui) {
    let effects_id = egui::Id::new("mixer_effects");
    let mut show_effects = ui.data(|d| d.get_temp(effects_id)).unwrap_or(true);
    ui.horizontal(|ui| {
        theme::caption(ui, "MIXER");
        let help = "Click a name to select it, double-click to show its chain; double-click a fader to reset it.";
        ui.label(RichText::new(help).small().color(theme::TEXT_WEAK));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let tip = "Show a strip for every effect too, or only the instruments, send tracks and master";
            if theme::toggle(ui, show_effects, "Effects").on_hover_text(tip).clicked() {
                show_effects = !show_effects;
            }
        });
    });
    ui.data_mut(|d| d.insert_temp(effects_id, show_effects));
    let sections = sections(app);
    // The master stays on the right, however far the rest scrolls.
    let master_w = STRIP_W + 14.0;
    ui.horizontal_top(|ui| {
        let h = ui.available_height();
        let w = (ui.available_width() - master_w).max(STRIP_W);
        ui.allocate_ui(Vec2::new(w, h), |ui| {
            egui::ScrollArea::horizontal().auto_shrink(false).show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    for (title, tip, ids) in sections {
                        if ids.is_empty() || (title == "EFFECTS" && !show_effects) {
                            continue;
                        }
                        ui.vertical(|ui| {
                            ui.label(RichText::new(title).small().color(theme::TEXT_WEAK)).on_hover_text(tip);
                            ui.horizontal_top(|ui| {
                                for id in ids {
                                    strip(app, ui, id);
                                }
                            });
                        });
                        ui.add_space(10.0);
                    }
                });
            });
        });
        ui.separator();
        ui.vertical(|ui| {
            ui.label(RichText::new("MASTER").small().color(theme::TEXT_WEAK));
            strip(app, ui, OUTPUT_ID);
        });
    });
}

fn strip(app: &mut App, ui: &mut egui::Ui, id: u8) {
    let Some(m) = app.project.module(id).cloned() else { return };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(STRIP_W, ui.available_height()), Sense::hover());
    let selected = app.selected_module == Some(id);
    let edge = if selected { theme::SELECTED } else { theme::FRAME_LINE };
    ui.painter().rect(rect, 3.0, theme::FRAME_BG, Stroke::new(1.0, edge), StrokeKind::Inside);
    let inner = rect.shrink(4.0);
    let layout = egui::Layout::top_down(egui::Align::Center);
    let mut child = ui.new_child(egui::UiBuilder::new().id_salt(("strip", id)).max_rect(inner).layout(layout));
    let ui = &mut child;
    ui.spacing_mut().item_spacing.y = 4.0;

    // The name on the module's color, as a track header has it.
    let (head, resp) = ui.allocate_exact_size(Vec2::new(inner.width(), 20.0), Sense::click());
    let color = if m.mute { Color32::from_gray(80) } else { modules::module_color(&m) };
    ui.painter().rect_filled(head, 2.0, color);
    ui.painter().with_clip_rect(head.shrink(2.0)).text(
        head.left_center() + Vec2::new(4.0, 0.0),
        Align2::LEFT_CENTER,
        if id == OUTPUT_ID { "Master".to_string() } else { format!("{id:02X} {}", m.name) },
        FontId::proportional(12.0),
        theme::SELECTED_TEXT,
    );
    let resp = resp.on_hover_text(format!("{id:02X} {}: click to select it, double-click to show its chain", m.name));
    if resp.double_clicked() {
        app.selected_module = Some(id);
        if let Some(owner) = app.project.owner_of(id) {
            app.show_chain(owner);
        }
    } else if resp.clicked() {
        app.selected_module = Some(id);
    }

    let kind = match m.kind {
        ModuleKind::Output => "MASTER",
        ModuleKind::Return => "SEND TRACK",
        kind => kind.name(),
    };
    ui.label(RichText::new(kind).small().color(theme::TEXT_WEAK));
    let targets: Vec<String> = app
        .project
        .links
        .iter()
        .filter(|l| l.0 == id)
        .filter_map(|l| app.project.module(l.1))
        .map(|t| if t.id == OUTPUT_ID { "Output".to_string() } else { format!("{:02X} {}", t.id, t.name) })
        .collect();
    let owner = app.project.owner_of(id);
    let route = match (id, targets.is_empty()) {
        (OUTPUT_ID, _) => "to Sound card".to_string(),
        _ if m.kind == ModuleKind::Return => "to Master".to_string(),
        _ if m.kind.is_effect() => match owner {
            Some(Owner::Track(t)) => format!("in Track {:02}", t + 1),
            Some(Owner::Module(OUTPUT_ID)) => "in Master".to_string(),
            Some(Owner::Module(o)) => format!("in {}", app.project.module(o).map_or("?", |m| m.name.as_str())),
            None => "in no chain".to_string(),
        },
        (_, true) => "to nowhere".to_string(),
        _ => format!("to {}", targets.join(", ")),
    };
    ui.add(egui::Label::new(RichText::new(&route).small()).truncate()).on_hover_text(route);

    // While an envelope moves them, the controls follow it.
    let n = m.kind.params().len();
    let mut pan = app.automated(id, n + 1).unwrap_or(m.pan);
    if widgets::param_bar(ui, &MIXER_PAN, &mut pan, |_| {}).changed() {
        app.project.module_mut(id).unwrap().pan = pan;
        app.mark();
    }

    // Fader, scale and meters fill the rest, above the level and buttons.
    let h = (ui.available_height() - 50.0).max(60.0);
    let (area, _) = ui.allocate_exact_size(Vec2::new(inner.width(), h), Sense::hover());
    let level = app.level(id);
    let gain = fader(app, ui, id, area, app.automated(id, n).unwrap_or(m.gain));
    meters(ui, area, level);

    ui.label(RichText::new(db_text(gain)).monospace().small());
    ui.horizontal(|ui| {
        let w = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        let mute = ui.add_sized([w, 20.0], egui::Button::selectable(m.mute, "M")).on_hover_text("Mute");
        if mute.clicked() {
            app.project.module_mut(id).unwrap().mute = !m.mute;
            app.mark();
        }
        let solo = ui.add_sized([w, 20.0], egui::Button::selectable(m.solo, "S"));
        if solo.on_hover_text("Solo: hear only this, what feeds it and what it feeds").clicked() {
            app.project.module_mut(id).unwrap().solo = !m.solo;
            app.mark();
        }
    });
}

/// Height of `t` (0..1) in the fader and meter column.
fn y_at(area: Rect, t: f32) -> f32 {
    let (top, bottom) = (area.top() + 6.0, area.bottom() - 6.0);
    bottom - t * (bottom - top)
}

/// The vertical fader on the left of `area`; returns the gain it shows.
fn fader(app: &mut App, ui: &mut egui::Ui, id: u8, area: Rect, gain: f32) -> f32 {
    let column = Rect::from_min_size(area.min, Vec2::new(34.0, area.height()));
    let resp = ui.interact(column, ui.id().with("fader"), Sense::click_and_drag());
    let mut new = gain;
    if resp.drag_started() {
        ui.data_mut(|d| d.insert_temp(resp.id, pos(gain)));
    }
    if resp.dragged() {
        let fine = if ui.input(|i| i.modifiers.shift) { 0.1 } else { 1.0 };
        let dt = -resp.drag_delta().y / (column.height() - 12.0) * fine;
        let t = ui.data_mut(|d| {
            let t = d.get_temp_mut_or(resp.id, pos(gain));
            *t = (*t + dt).clamp(0.0, 1.0);
            *t
        });
        new = gain_at(t);
    }
    if resp.double_clicked() {
        new = 1.0;
    }
    if new != gain {
        app.project.module_mut(id).unwrap().gain = new;
        app.mark();
    }

    let painter = ui.painter();
    let x = column.center().x;
    let groove = Rect::from_x_y_ranges(x - 2.0..=x + 2.0, y_at(area, 1.0)..=y_at(area, 0.0));
    painter.rect_filled(groove, 2.0, theme::INSET);
    let y = y_at(area, pos(new));
    painter.rect_filled(Rect::from_x_y_ranges(groove.x_range(), y..=groove.bottom()), 2.0, theme::BAR);
    let hot = resp.hovered() || resp.dragged();
    let handle = Rect::from_center_size(Pos2::new(x, y), Vec2::new(26.0, 12.0));
    painter.rect(
        handle,
        2.0,
        Color32::from_gray(if hot { 120 } else { 92 }),
        Stroke::new(1.0, Color32::from_gray(30)),
        StrokeKind::Inside,
    );
    painter
        .line_segment([Pos2::new(handle.left() + 3.0, y), Pos2::new(handle.right() - 3.0, y)], (1.0, Color32::WHITE));
    new
}

/// The dB scale and the left and right peak meters, right of the fader.
fn meters(ui: &egui::Ui, area: Rect, level: [f32; 2]) {
    let painter = ui.painter();
    let scale_x = area.left() + 48.0;
    for db in MARKS {
        let y = y_at(area, pos(10f32.powf(db / 20.0)));
        let label = if db > 0.0 { format!("+{db}") } else { format!("{db}") };
        painter.text(Pos2::new(scale_x, y), Align2::CENTER_CENTER, label, FontId::monospace(9.0), theme::TEXT_WEAK);
        let tick = [Pos2::new(area.right() - 20.0, y), Pos2::new(area.right() - 17.0, y)];
        painter.line_segment(tick, (1.0, theme::TEXT_WEAK));
    }
    // Green up to -6 dB, amber up to 0 dB, red above.
    let bands = [(0.0, pos(0.5), theme::SCOPE), (pos(0.5), pos(1.0), theme::SELECTED), (pos(1.0), 1.0, theme::RECORD)];
    for (ch, peak) in level.into_iter().enumerate() {
        let x0 = area.right() - 15.0 + ch as f32 * 8.0;
        let bar = Rect::from_x_y_ranges(x0..=x0 + 6.0, y_at(area, 1.0)..=y_at(area, 0.0));
        painter.rect_filled(bar, 1.0, theme::INSET);
        let t = pos(peak);
        for (lo, hi, color) in bands {
            if t > lo {
                let r = Rect::from_x_y_ranges(bar.x_range(), y_at(area, t.min(hi))..=y_at(area, lo));
                painter.rect_filled(r, 0.0, color);
            }
        }
    }
}
