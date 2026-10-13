//! The mixer. Audio flows through modules here rather than
//! tracks, so every instrument and send track gets a channel strip listing
//! the effects of its chain, with the output as the master strip on the
//! right.

use super::{App, modules, theme, widgets};
use crate::project::{MIXER_PAN, ModuleKind, OUTPUT_ID, Owner};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};

/// Strip width when they all fit, and the narrowest they get before the
/// mixer scrolls.
const STRIP_W: f32 = 96.0;
const STRIP_MIN_W: f32 = 80.0;
/// The most effects a strip lists, and the height of each.
const MAX_DEVICES: usize = 6;
const DEVICE_H: f32 = 16.0;
/// The space between strips, and between chains; the room around a
/// chain's strips in its frame, and the band of its color above them.
const STRIP_GAP: f32 = 4.0;
const GROUP_GAP: f32 = 6.0;
const GROUP_PAD: f32 = 3.0;
const GROUP_BAND: f32 = 4.0;
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

/// One chain's strips, side by side in a frame of its color: what starts
/// it, an instrument or a send track, then its effects when they are
/// shown. A track's chain and the master's have only their effects here.
struct Group {
    head: Option<u8>,
    effects: Vec<u8>,
    color: Color32,
}

/// The chains: the instruments', by number, the tracks', the send
/// tracks' as they are listed, and the master's effects, beside its strip.
/// Sends have no strips: their amounts say how loud they send.
fn groups(app: &App, show_effects: bool) -> Vec<Group> {
    let p = &app.project;
    let strips = |ids: &[u8]| -> Vec<u8> {
        let shown = ids.iter().filter(|&&e| p.module(e).is_some_and(|m| m.kind != ModuleKind::Send));
        if show_effects { shown.copied().collect() } else { Vec::new() }
    };
    let module = |id: u8| -> Option<Group> {
        let m = p.module(id)?;
        Some(Group { head: Some(id), effects: strips(&p.chain(id).effects), color: modules::module_color(m) })
    };
    let mut instruments: Vec<u8> =
        (p.modules.iter()).filter(|m| m.kind.in_instrument_list() && m.kind.makes_sound()).map(|m| m.id).collect();
    instruments.sort();
    let mut groups: Vec<Group> = instruments.into_iter().filter_map(module).collect();
    for t in 0..p.tracks.len() {
        let color = super::pattern::track_color(p, t);
        groups.push(Group { head: None, effects: strips(&p.tracks[t].effects), color });
    }
    groups.extend(p.sends.iter().filter_map(|s| module(s.module)));
    let master = p.module(OUTPUT_ID).map_or(theme::FRAME_LINE, modules::module_color);
    groups.push(Group { head: None, effects: strips(&p.master), color: master });
    groups.retain(|g| g.head.is_some() || !g.effects.is_empty());
    groups
}

/// The effects strip `id` lists: its chain's, for an instrument, a send
/// track or the master; none for an effect, which is in one.
fn devices(app: &App, id: u8) -> Vec<u8> {
    let p = &app.project;
    match p.module(id) {
        Some(m) if m.kind.is_effect() => Vec::new(),
        Some(_) => p.chain(id).effects,
        None => Vec::new(),
    }
}

pub fn view(app: &mut App, ui: &mut egui::Ui) {
    let effects_id = egui::Id::new("mixer_effects");
    let mut show_effects = ui.data(|d| d.get_temp(effects_id)).unwrap_or(false);
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
    let groups = groups(app, show_effects);
    let count = groups.iter().map(|g| g.effects.len() + g.head.is_some() as usize).sum::<usize>();
    // Every strip lists as many effects as the longest chain, so that the
    // faders line up, and gives up rows before the faders get short; none
    // when the effects have strips of their own beside it.
    let heads = groups.iter().filter_map(|g| g.head).chain([OUTPUT_ID]);
    let longest = if show_effects { 0 } else { heads.map(|id| devices(app, id).len()).max().unwrap_or(0) };
    let rows = longest.min(MAX_DEVICES).min(((ui.available_height() - 320.0) / DEVICE_H).max(0.0) as usize);
    // Frames need room around the strips, kept around every chain so
    // that they line up.
    let pad = if show_effects { GROUP_PAD } else { 0.0 };
    // The master stays on the right, however far the rest scrolls.
    let master_w = STRIP_W + 14.0;
    ui.horizontal_top(|ui| {
        let h = ui.available_height();
        let w = (ui.available_width() - master_w).max(STRIP_W);
        // Narrower strips before scrolling, when there are many.
        let gaps = count as f32 * STRIP_GAP + groups.len() as f32 * (GROUP_GAP + 2.0 * pad);
        let strip_w = ((w - gaps) / count.max(1) as f32).clamp(STRIP_MIN_W, STRIP_W);
        ui.allocate_ui(Vec2::new(w, h), |ui| {
            egui::ScrollArea::horizontal().auto_shrink(false).show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = GROUP_GAP;
                    for g in groups {
                        let framed = !g.effects.is_empty();
                        group(ui, pad, framed.then_some(g.color), |ui| {
                            for id in g.head.into_iter().chain(g.effects) {
                                strip(app, ui, id, strip_w, rows);
                            }
                        });
                    }
                });
            });
        });
        ui.separator();
        group(ui, pad, None, |ui| strip(app, ui, OUTPUT_ID, STRIP_W, rows));
    });
}

/// A chain's strips, added by `add`, `pad` from its edges with a band
/// above them: framed in `color`, if any, as one with effects' strips is.
fn group(ui: &mut egui::Ui, pad: f32, color: Option<Color32>, add: impl FnOnce(&mut egui::Ui)) {
    let band = if pad > 0.0 { GROUP_BAND } else { 0.0 };
    let margin = egui::Margin { top: (pad + band) as i8, ..egui::Margin::same(pad as i8) };
    let mut frame = egui::Frame::new().inner_margin(margin).corner_radius(4.0);
    if let Some(color) = color {
        frame = frame.fill(color.gamma_multiply(0.08)).stroke(Stroke::new(1.0, color.gamma_multiply(0.5)));
    }
    let resp = frame.show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = STRIP_GAP;
        ui.horizontal_top(add);
    });
    if let Some(color) = color {
        let r = resp.response.rect;
        let band = Rect::from_min_size(r.min, Vec2::new(r.width(), band));
        ui.painter().rect_filled(band, egui::CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, color);
    }
}

/// The effects of a strip's chain, `rows` high: each on its color, dull
/// when off, with a light that switches it on and off; past `rows` the
/// last says how many more.
fn device_list(app: &mut App, ui: &mut egui::Ui, ids: &[u8], rows: usize) {
    let (area, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), rows as f32 * DEVICE_H), Sense::hover());
    let row = |k: usize| {
        Rect::from_min_size(area.min + Vec2::new(0.0, k as f32 * DEVICE_H), Vec2::new(area.width(), DEVICE_H))
    };
    let fit = if ids.len() > rows { rows - 1 } else { ids.len() };
    for (k, &id) in ids[..fit].iter().enumerate() {
        let Some(m) = app.project.module(id).cloned() else { continue };
        let row = row(k).shrink2(Vec2::new(0.0, 1.0));
        let light = Rect::from_center_size(row.left_center() + Vec2::new(7.0, 0.0), Vec2::splat(8.0));
        let resp = ui.interact(row, ui.id().with(("device", id)), Sense::click());
        let switch = ui.interact(light.expand(3.0), ui.id().with(("bypass", id)), Sense::click());
        let off = m.mute || m.bypass;
        let selected = app.selected_module == Some(id);
        let color = if off { Color32::from_gray(80) } else { modules::module_color(&m) };
        let fill = if selected { color } else { color.gamma_multiply(if resp.hovered() { 0.5 } else { 0.3 }) };
        let painter = ui.painter();
        painter.rect_filled(row, 2.0, fill);
        painter.rect_filled(light, 1.0, if m.bypass { theme::INSET } else { theme::SCOPE });
        let text = if selected { theme::SELECTED_TEXT } else { theme::TEXT };
        painter.with_clip_rect(row.shrink(1.0)).text(
            Pos2::new(light.right() + 4.0, row.center().y),
            Align2::LEFT_CENTER,
            &m.name,
            FontId::proportional(11.0),
            text,
        );
        let (state, other) = if m.bypass { ("off", "on") } else { ("on", "off") };
        let tip = format!("{id:02X} {} · {}, {state}", m.name, m.kind.name());
        if switch.on_hover_text(format!("{tip}: click to switch it {other}")).clicked() {
            app.project.module_mut(id).unwrap().bypass = !m.bypass;
            app.mark();
            continue;
        }
        let resp = resp.on_hover_text(format!("{tip}: click to select it, double-click to show it in its chain"));
        if resp.double_clicked() {
            reveal(app, id);
        } else if resp.clicked() {
            app.select(Some(id));
        }
    }
    if fit < ids.len() {
        let row = row(fit);
        let resp = ui.interact(row, ui.id().with("more"), Sense::click());
        let more = format!("+{} more", ids.len() - fit);
        ui.painter().text(row.center(), Align2::CENTER_CENTER, more, FontId::proportional(11.0), theme::TEXT_WEAK);
        if resp.on_hover_text("Click to show the whole chain").clicked() {
            reveal(app, ids[fit]);
        }
    }
}

/// Selects module `id` and shows the chain it is in, scrolled to it.
fn reveal(app: &mut App, id: u8) {
    app.select(Some(id));
    app.chain_view.reveal(id);
    if let Some(owner) = app.project.owner_of(id) {
        app.show_chain(owner);
    }
}

fn strip(app: &mut App, ui: &mut egui::Ui, id: u8, w: f32, rows: usize) {
    let Some(m) = app.project.module(id).cloned() else { return };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, ui.available_height()), Sense::hover());
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
    let kind = match m.kind {
        ModuleKind::Output => "Master",
        ModuleKind::Return => "Send track",
        kind => kind.name(),
    };
    let tip = format!("{id:02X} {} · {kind}: click to select it, double-click to show its chain", m.name);
    let resp = resp.on_hover_text(tip);
    if resp.double_clicked() {
        app.select(Some(id));
        if let Some(owner) = app.project.owner_of(id) {
            app.show_chain(owner);
        }
    } else if resp.clicked() {
        app.select(Some(id));
    }

    // Its effects are listed below, so an instrument says where the last
    // of them goes.
    let outputs = if m.kind.is_effect() {
        app.project.links.iter().filter(|l| l.0 == id).map(|l| l.1).collect()
    } else {
        app.project.chain(id).outputs
    };
    let targets: Vec<String> = (outputs.iter().filter_map(|&t| app.project.module(t)))
        .map(|t| if t.id == OUTPUT_ID { "Master".to_string() } else { format!("{:02X} {}", t.id, t.name) })
        .collect();
    let owner = app.project.owner_of(id);
    let route = match (id, targets.is_empty()) {
        (OUTPUT_ID, _) => "to Sound card".to_string(),
        _ if m.kind == ModuleKind::Return => "to Master".to_string(),
        _ if m.kind.is_effect() => match owner {
            // In a frame with what starts its chain it says what it is.
            Some(Owner::Track(t)) => format!("in {}", app.project.track_name(t)),
            Some(Owner::Module(OUTPUT_ID)) => "in Master".to_string(),
            Some(Owner::Module(_)) => m.kind.name().to_string(),
            None => "in no chain".to_string(),
        },
        (_, true) => "to nowhere".to_string(),
        _ => format!("to {}", targets.join(", ")),
    };
    ui.add(egui::Label::new(RichText::new(&route).small()).truncate()).on_hover_text(route);
    if rows > 0 {
        device_list(app, ui, &devices(app, id), rows);
    }

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
    // Against the ticks, clear of the fader however narrow the strip.
    let scale_x = area.right() - 23.0;
    for db in MARKS {
        let y = y_at(area, pos(10f32.powf(db / 20.0)));
        let label = if db > 0.0 { format!("+{db}") } else { format!("{db}") };
        painter.text(Pos2::new(scale_x, y), Align2::RIGHT_CENTER, label, FontId::monospace(9.0), theme::TEXT_WEAK);
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
