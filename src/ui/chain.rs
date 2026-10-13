//! An instrument's device chain: the instrument itself and its effects,
//! each a panel of parameters, in the order the sound goes through them,
//! and the Modulators moving any of them. Everything about an instrument's
//! sound is edited here. The output's page is the master chain: the
//! effects the whole mix goes through, then the output; a track's page is
//! its own effects, which what it plays goes through on its way to the
//! master; a send track's is the effects what Sends send it goes through.

use super::{App, modules, pattern, routing, theme, widgets};
use crate::project::{MACROS, ModuleKind, OUTPUT_ID, Owner};
use eframe::egui::{self, Align2, Color32, FontId, RichText, Sense, Stroke, StrokeKind, Vec2};

/// Width of a device panel, and of one folded to its name.
const DEVICE_W: f32 = 215.0;
const FOLDED_W: f32 = 26.0;

/// How the chains are shown: which devices are folded to their names,
/// which instruments show their macros, and a device to scroll to.
#[derive(Default)]
pub struct ChainView {
    folded: std::collections::HashSet<u8>,
    /// Instruments whose macro panel was opened or closed by hand; the rest
    /// show it while any macro moves something.
    macros: std::collections::HashMap<u8, bool>,
    scroll_to: Option<u8>,
}

impl ChainView {
    /// Forgets the devices that are `gone`.
    pub fn forget(&mut self, gone: impl Fn(u8) -> bool) {
        self.folded.retain(|&id| !gone(id));
        self.macros.retain(|&id, _| !gone(id));
        self.scroll_to = self.scroll_to.filter(|&id| !gone(id));
    }

    /// Opens device `id` if it is folded, and scrolls to it.
    pub fn reveal(&mut self, id: u8) {
        self.folded.remove(&id);
        self.scroll_to = Some(id);
    }

    /// Folds `ids` to their names.
    #[cfg(test)]
    pub fn fold(&mut self, ids: &[u8]) {
        self.folded.extend(ids);
    }

    #[cfg(test)]
    pub fn is_folded(&self, id: u8) -> bool {
        self.folded.contains(&id)
    }
}

/// What a device is to the instrument whose chain is shown.
#[derive(Clone, Copy, PartialEq)]
enum Place {
    Instrument,
    /// The output, at the end of the master chain.
    Output,
    /// Its own effect `k` of `n`.
    Own(usize, usize),
    Modulator,
}

enum Action {
    Add(usize, ModuleKind),
    Remove(u8),
    Delete(u8),
    Move(u8, i32),
    /// A Modulator, starting on this device.
    AddModulator(u8),
    /// A Send at the end of the chain, to this send track, or to a new one.
    AddSend(usize, Option<u8>),
    Select(u8),
    /// Folds a device to its name, or opens it.
    Fold(u8, bool),
    FoldAll(bool),
    /// An effect dragged by its name and let go at this x.
    Drop(u8, f32),
}

pub fn page(app: &mut App, ui: &mut egui::Ui, owner: impl Into<Owner>) {
    let owner = owner.into();
    let (id, track) = match owner {
        Owner::Module(id) => (Some(id), None),
        Owner::Track(t) => (None, Some(t)),
    };
    let inst = id.and_then(|id| app.project.module(id).cloned());
    if id.is_some() && inst.is_none() {
        return;
    }
    let master = id == Some(OUTPUT_ID);
    let send = id.filter(|&id| app.project.send_track(id).is_some());
    // A MultiSynth's links carry notes: it has no sound to follow.
    let notes_only = inst.as_ref().is_some_and(|m| m.kind.notes_only());
    let chain = app.project.chain(owner);
    // The Modulators moving anything this sound goes through, those added
    // here, and on the master those that have no place.
    let mut reach: Vec<u8> = id.into_iter().collect();
    reach.extend(&chain.effects);
    let links = &app.project.links;
    let modulators: Vec<u8> = (app.project.modules.iter())
        .filter(|m| m.kind.controls())
        .filter(|m| {
            let moves = links.iter().any(|l| l.0 == m.id && reach.contains(&l.1));
            let homeless = m.home.is_none() && !links.iter().any(|l| l.0 == m.id);
            moves || m.home == Some(owner) || (master && homeless)
        })
        .map(|m| m.id)
        .collect();
    let mut action = None;
    // Every device along the way, in order, for the overview.
    let mut path: Vec<u8> = id.filter(|&i| i != OUTPUT_ID && send.is_none()).into_iter().collect();
    if !notes_only {
        path.extend(&chain.effects);
    }
    if master {
        path.push(OUTPUT_ID);
    }
    overview(app, ui, owner, &path, &modulators, chain.effects.len(), !notes_only, &mut action);
    ui.add_space(3.0);
    let h = ui.available_height() - 8.0;
    egui::ScrollArea::horizontal().id_salt("chain").auto_shrink(false).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.set_min_height(h);
            match (id, track) {
                (Some(id), _) if send.is_some() => send_panel(app, ui, id, h),
                (Some(id), _) if !master => {
                    if let Some(m) = inst.as_ref().filter(|m| m.kind.plays_sound()) {
                        let used = m.macros.iter().any(|mac| !mac.targets.is_empty());
                        if app.chain_view.macros.get(&id).copied().unwrap_or(used) {
                            macros_panel(app, ui, id, h);
                        } else if folded_strip(ui, "MACROS", theme::FRAME_LINE, false, h)
                            .on_hover_text("Eight knobs that each move any parameters of the instrument and its effects: click to show them")
                            .clicked()
                        {
                            app.chain_view.macros.insert(id, true);
                        }
                        ui.separator();
                    }
                    device(app, ui, id, Place::Instrument, h, &mut action);
                }
                (_, Some(t)) => track_panel(app, ui, t, h),
                _ => {}
            }
            if !notes_only {
                let mut own = Vec::new();
                for (k, &e) in chain.effects.iter().enumerate() {
                    if !master || k > 0 {
                        arrow(ui, h);
                    }
                    own.push((e, device(app, ui, e, Place::Own(k, chain.effects.len()), h, &mut action)));
                }
                // While an effect is dragged, a line shows where it lands.
                if let (Some(dragged), Some(p)) = (dragging(ui), ui.input(|i| i.pointer.hover_pos())) {
                    let others: Vec<egui::Rect> = own.iter().filter(|o| o.0 != dragged).map(|o| o.1).collect();
                    let x = match others.iter().position(|r| r.center().x > p.x) {
                        Some(i) => others[i].left() - 9.0,
                        None => others.last().map_or(p.x, |r| r.right() + 9.0),
                    };
                    let top = own.first().map_or(p.y, |o| o.1.top());
                    ui.painter()
                        .line_segment([egui::pos2(x, top), egui::pos2(x, top + h)], Stroke::new(3.0, theme::SELECTED));
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                }
                if let Some(Action::Drop(e, x)) = action {
                    let index = own.iter().filter(|o| o.0 != e && o.1.center().x < x).count();
                    action = None;
                    app.project.chain_place(owner, e, index);
                    app.mark();
                }
                if !chain.effects.is_empty() || !master {
                    arrow(ui, h);
                }
                ending(app, ui, owner, chain.effects.len(), h, &mut action);
                if master {
                    arrow(ui, h);
                    device(app, ui, OUTPUT_ID, Place::Output, h, &mut action);
                }
                if let Some(t) = track {
                    arrow(ui, h);
                    to_master(app, ui, t, h);
                }
            }
            ui.separator();
            for &m in &modulators {
                device(app, ui, m, Place::Modulator, h, &mut action);
            }
            // A Modulator starts on the first device it can move.
            if let Some(&target) = reach.first() {
                ui.vertical(|ui| {
                    ui.set_width(150.0);
                    let tip = "A Modulator moves parameters with an LFO, or by following a sound's level";
                    if ui.button("+ Add Modulator").on_hover_text(tip).clicked() {
                        action = Some(Action::AddModulator(target));
                    }
                });
            }
        });
    });
    let name = match (&inst, track) {
        (_, Some(t)) => app.project.track_name(t),
        (Some(m), _) => m.name.clone(),
        _ => String::new(),
    };
    let _ = send;
    match action {
        Some(Action::Add(after, kind)) => {
            if let Some(new) = app.project.chain_insert(owner, after, kind) {
                app.set_status(format!("Added {} {:02X} to {name}", kind.name(), new));
                app.mark();
            } else {
                app.set_status("Too many modules");
            }
        }
        Some(Action::Remove(e)) => {
            app.project.chain_remove(owner, e);
            app.mark();
        }
        Some(Action::Delete(m)) => {
            app.project.remove_module(m);
            app.mark();
        }
        Some(Action::Move(e, delta)) => {
            app.project.chain_move(owner, e, delta);
            app.mark();
        }
        Some(Action::AddModulator(target)) => match app.project.add_module(ModuleKind::Modulator, [0.0, 0.0]) {
            Some(m) => {
                // It starts on the device's first parameter; pick another below.
                app.project.connect(m, target);
                app.project.module_mut(m).unwrap().home = Some(owner);
                app.mark();
            }
            None => app.set_status("Too many modules"),
        },
        Some(Action::AddSend(at, to)) => {
            let to = to.or_else(|| {
                let name = app.project.next_send_name();
                app.project.add_send_track(&name)
            });
            match (to, app.project.chain_insert(owner, at, ModuleKind::Send)) {
                (Some(to), Some(new)) => {
                    let name = app.project.module(to).map(|m| m.name.clone()).unwrap_or_default();
                    let m = app.project.module_mut(new).unwrap();
                    (m.send_to, m.name) = (Some(to), format!("To {name}"));
                    app.set_status(format!("{} sends to {name}", if notes_only { "It" } else { "The chain" }));
                    app.mark();
                }
                _ => app.set_status("Too many modules"),
            }
        }
        Some(Action::Select(m)) => app.select(Some(m)),
        Some(Action::Fold(m, on)) => {
            if on {
                app.chain_view.folded.insert(m);
            } else {
                app.chain_view.folded.remove(&m);
            }
        }
        Some(Action::FoldAll(on)) => {
            if on {
                app.chain_view.folded.extend(path.iter().chain(&modulators));
            } else {
                app.chain_view.folded.clear();
            }
        }
        // Drops are handled where the devices' places are known.
        Some(Action::Drop(..)) | None => {}
    }
}

/// The panel a track's chain starts with: the track on its color, its mute
/// and solo, and the instruments it plays, whose sound comes in here.
fn track_panel(app: &mut App, ui: &mut egui::Ui, t: usize, h: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(DEVICE_W * 0.8, h), Sense::hover());
    ui.painter().rect(rect, 3.0, theme::FRAME_BG, Stroke::new(1.0, theme::FRAME_LINE), StrokeKind::Inside);
    let inner = rect.shrink(5.0);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("track_panel", t))
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    let ui = &mut child;
    let (head, _) = ui.allocate_exact_size(Vec2::new(inner.width(), 20.0), Sense::hover());
    let color = pattern::track_color(&app.project, t);
    ui.painter().rect_filled(head, 2.0, color);
    ui.painter().with_clip_rect(head.shrink(2.0)).text(
        head.left_center() + Vec2::new(5.0, 0.0),
        Align2::LEFT_CENTER,
        format!("{:02} {}", t + 1, app.project.track_name(t)),
        FontId::proportional(12.5),
        theme::SELECTED_TEXT,
    );
    let info = app.project.tracks.get(t).cloned().unwrap_or_default();
    ui.horizontal(|ui| {
        if theme::toggle(ui, info.mute, "Mute").on_hover_text("Silence this track").clicked() {
            app.project.tracks[t].mute = !info.mute;
            app.mark();
        }
        if theme::toggle(ui, info.solo, "Solo").on_hover_text("Hear only this track").clicked() {
            app.project.solo_track(t);
            app.mark();
        }
    });
    ui.add_space(6.0);
    theme::caption(ui, "PLAYS");
    let plays = app.project.track_instruments(t);
    if plays.is_empty() {
        ui.label(RichText::new("No notes yet").small().color(theme::TEXT_WEAK));
    }
    for m in plays {
        ui.label(RichText::new(routing::label(app, m)).small());
    }
    ui.add_space(6.0);
    let hint = "What the track plays goes through its effects after each instrument's own, then on to the master, or through another track's effects first if the track menu's Group picks one.";
    ui.label(RichText::new(hint).small().color(theme::TEXT_WEAK));
}

/// Where track `t`'s chain ends: its group's effects or the master chain,
/// a click away.
fn to_master(app: &mut App, ui: &mut egui::Ui, t: usize, h: f32) {
    if let Some(g) = app.project.group_of(t) {
        ui.vertical(|ui| {
            ui.set_width(110.0);
            ui.set_max_height(h);
            theme::caption(ui, "TO GROUP");
            ui.label(RichText::new(format!("{:02} {}", g + 1, app.project.track_name(g))).small());
            if ui.button("Show Group").clicked() {
                app.show_track_fx(g);
            }
        });
        return;
    }
    ui.vertical(|ui| {
        ui.set_width(110.0);
        ui.set_max_height(h);
        theme::caption(ui, "TO MASTER");
        let n = app.project.master.len();
        let what = if n == 0 {
            "The output".to_string()
        } else {
            format!("{n} master effect{}", if n == 1 { "" } else { "s" })
        };
        ui.label(RichText::new(what).small().color(theme::TEXT_WEAK));
        if ui.button("Show Master").clicked() {
            app.track_fx = super::TrackFx::Master;
        }
    });
}

/// The arrow between two devices.
fn arrow(ui: &mut egui::Ui, h: f32) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(18.0, h), Sense::hover());
    super::icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(r.center(), Vec2::splat(10.0)),
        super::icons::Icon::Right,
        theme::TEXT_WEAK,
    );
}

/// The effect whose name is being dragged, if any.
fn dragging(ui: &egui::Ui) -> Option<u8> {
    ui.data(|d| d.get_temp(egui::Id::new("chain_drag")))
}

/// A device panel: a header with the device's name
/// on its color and its switches, and its parameters below. Returns where
/// it was drawn.
fn device(app: &mut App, ui: &mut egui::Ui, id: u8, place: Place, h: f32, action: &mut Option<Action>) -> egui::Rect {
    let Some(m) = app.project.module(id).cloned() else { return egui::Rect::NOTHING };
    let scroll = app.chain_view.scroll_to == Some(id);
    if app.chain_view.folded.contains(&id) {
        let off = m.mute || m.bypass;
        let color = if off { Color32::from_gray(80) } else { modules::module_color(&m) };
        let text = format!("{id:02X} {}", m.name);
        let resp = folded_strip(ui, &text, color, app.selected_module == Some(id), h);
        if scroll {
            resp.scroll_to_me(Some(egui::Align::Center));
            app.chain_view.scroll_to = None;
        }
        let rect = resp.rect;
        if resp.on_hover_text(format!("{text} · {}: click to open it", m.kind.name())).clicked() {
            *action = Some(Action::Fold(id, false));
        }
        return rect;
    }
    let (rect, _) = ui.allocate_exact_size(Vec2::new(DEVICE_W, h), Sense::hover());
    if scroll {
        ui.scroll_to_rect(rect, Some(egui::Align::Center));
        app.chain_view.scroll_to = None;
    }
    let edge = if app.selected_module == Some(id) { theme::SELECTED } else { theme::FRAME_LINE };
    ui.painter().rect(rect, 3.0, theme::FRAME_BG, Stroke::new(1.0, edge), StrokeKind::Inside);
    let inner = rect.shrink(5.0);
    let mut child = ui.new_child(
        egui::UiBuilder::new().id_salt(("device", id)).max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.shrink_clip_rect(inner);
    let ui = &mut child;

    // Header: number and name on the module's color; an effect of the
    // instrument's own is dragged by it to another place in the chain.
    let own = matches!(place, Place::Own(..));
    let sense = if own { Sense::click_and_drag() } else { Sense::click() };
    let (head, resp) = ui.allocate_exact_size(Vec2::new(inner.width(), 20.0), sense);
    if own && resp.drag_started() {
        ui.data_mut(|d| d.insert_temp(egui::Id::new("chain_drag"), id));
    }
    if own && resp.drag_stopped() {
        ui.data_mut(|d| d.remove::<u8>(egui::Id::new("chain_drag")));
        if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
            *action = Some(Action::Drop(id, p.x));
        }
    }
    let off = m.mute || m.bypass;
    let color = if off { Color32::from_gray(80) } else { modules::module_color(&m) };
    ui.painter().rect_filled(head, 2.0, color);
    ui.painter().with_clip_rect(head.shrink(2.0)).text(
        head.left_center() + Vec2::new(5.0, 0.0),
        Align2::LEFT_CENTER,
        format!("{id:02X} {}", m.name),
        FontId::proportional(12.5),
        theme::SELECTED_TEXT,
    );
    let tag = m.kind.name().to_string();
    ui.painter().text(
        head.right_center() - Vec2::new(5.0, 0.0),
        Align2::RIGHT_CENTER,
        tag,
        FontId::proportional(10.5),
        theme::SELECTED_TEXT,
    );
    let tip = if own {
        "Drag to move it along the chain; click to select it in the mixer and automation; right-click for presets and its color"
    } else {
        "Click to select it in the mixer and automation; right-click for presets and its color"
    };
    let resp = resp.on_hover_text(format!("{tip}; double-click to fold it to its name"));
    if resp.double_clicked() {
        *action = Some(Action::Fold(id, true));
    } else if resp.clicked() {
        *action = Some(Action::Select(id));
    }
    widgets::field_context_menu(&resp, |ui| {
        ui.menu_button("Presets", |ui| super::presets::menu(app, ui, id));
        ui.menu_button("Color", |ui| routing::color_menu(app, ui, id));
    });

    ui.horizontal(|ui| {
        match place {
            Place::Output => {}
            Place::Instrument => {
                if theme::toggle(ui, m.mute, "Mute").on_hover_text("Silence this instrument").clicked() {
                    app.project.module_mut(id).unwrap().mute = !m.mute;
                    app.mark();
                }
                if m.kind.makes_sound()
                    && theme::toggle(ui, m.solo, "Solo")
                        .on_hover_text("Hear only this, and what it goes through")
                        .clicked()
                {
                    app.project.module_mut(id).unwrap().solo = !m.solo;
                    app.mark();
                }
            }
            Place::Own(..) => {
                let label = if m.bypass { "Off" } else { "On" };
                if theme::toggle(ui, !m.bypass, label)
                    .on_hover_text("Switch the effect off: the sound goes past it")
                    .clicked()
                {
                    app.project.module_mut(id).unwrap().bypass = !m.bypass;
                    app.mark();
                }
            }
            Place::Modulator => {
                let label = if m.mute { "Off" } else { "On" };
                if theme::toggle(ui, !m.mute, label).on_hover_text("Switch the Modulator off").clicked() {
                    app.project.module_mut(id).unwrap().mute = !m.mute;
                    app.mark();
                }
            }
        }
        if let Place::Own(k, n) = place {
            if ui
                .add_enabled_ui(k > 0, |ui| super::icons::button(ui, super::icons::Icon::Left))
                .inner
                .on_hover_text("Earlier in the chain")
                .clicked()
            {
                *action = Some(Action::Move(id, -1));
            }
            if ui
                .add_enabled_ui(k + 1 < n, |ui| super::icons::button(ui, super::icons::Icon::Right))
                .inner
                .on_hover_text("Later in the chain")
                .clicked()
            {
                *action = Some(Action::Move(id, 1));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !matches!(place, Place::Instrument | Place::Output)
                && super::icons::button(ui, super::icons::Icon::Bin).on_hover_text("Remove it").clicked()
            {
                *action = Some(if matches!(place, Place::Own(..)) { Action::Remove(id) } else { Action::Delete(id) });
            }
            if super::icons::button(ui, super::icons::Icon::Minus).on_hover_text("Fold it to its name").clicked() {
                *action = Some(Action::Fold(id, true));
            }
        });
    });
    if m.kind == ModuleKind::Send {
        send_target(app, ui, id);
    }
    if m.kind == ModuleKind::Drums {
        let drums = "C kick · D snare · F# closed hat · A# open hat · others tom";
        ui.label(RichText::new(drums).small().color(theme::TEXT_WEAK));
    }
    ui.add_space(3.0);
    egui::ScrollArea::vertical().id_salt(("device_params", id)).auto_shrink(false).show(ui, |ui| {
        modules::param_list(app, ui, id);
        if m.kind.notes_only() {
            ui.add_space(6.0);
            theme::caption(ui, "PLAYS");
            ui.label(RichText::new("The instruments it passes its notes to").small().color(theme::TEXT_WEAK));
            routing::plays_checklist(app, ui, id);
        }
        if place == Place::Modulator {
            ui.add_space(6.0);
            theme::caption(ui, "MOVES");
            routing::control_targets(app, ui, id);
            // An LFO and a Manual knob follow nothing.
            if !matches!(m.params[0].round() as u32, 0 | 5) {
                ui.add_space(6.0);
                theme::caption(ui, "FOLLOWS");
                ui.label(RichText::new(modules::follows_hint(m.params[0])).small().color(theme::TEXT_WEAK));
                routing::sources_checklist(app, ui, id);
            }
        }
    });
    rect
}

/// After a chain's devices: adding an effect or a Send, and where the
/// sound goes next.
fn ending(app: &mut App, ui: &mut egui::Ui, owner: Owner, len: usize, h: f32, action: &mut Option<Action>) {
    ui.vertical(|ui| {
        ui.set_width(150.0);
        ui.set_max_height(h);
        add_effect_menu(ui, "+ Add Effect", len, action);
        if owner != Owner::Module(OUTPUT_ID) {
            add_send_menu(app, ui, "+ Add Send", owner, len, action);
        }
        ui.add_space(8.0);
        let hint = match owner {
            Owner::Module(OUTPUT_ID) => {
                "Effects here process the whole mix, in order, before the output. Drag them by their names to reorder them."
            }
            Owner::Track(_) => {
                "Effects here process what this track plays, in order. Drag them by their names to reorder them."
            }
            Owner::Module(id) if app.project.send_track(id).is_some() => {
                "Effects here process what Sends send this send track, in order, then its fader takes it to the master."
            }
            Owner::Module(_) => {
                "The sound goes on to the effects of the track playing it, then the master. + Add Send sends a copy to a send track, for a reverb or a delay several instruments share."
            }
        };
        ui.label(RichText::new(hint).small().color(theme::TEXT_WEAK));
        // A chain that went nowhere can go to the output again.
        if let Owner::Module(id) = owner
            && app.project.module(id).is_some_and(|m| m.kind.starts_chain())
            && app.project.chain(id).outputs.is_empty()
        {
            ui.add_space(6.0);
            ui.label(RichText::new("It goes nowhere: it is silent.").small().color(theme::RECORD));
            if ui.button("Send to the Output").clicked() {
                let last = app.project.chain(id).effects.last().copied().unwrap_or(id);
                app.project.connect(last, OUTPUT_ID);
                app.mark();
            }
        }
    });
}

/// The effects by what they do, as the add menu groups them.
const EFFECT_GROUPS: &[(&str, &[ModuleKind])] = &[
    (
        "Filters",
        &[
            ModuleKind::Filter,
            ModuleKind::FilterPro,
            ModuleKind::AnalogFilter,
            ModuleKind::CombFilter,
            ModuleKind::VocalFilter,
            ModuleKind::ScreamFilter,
        ],
    ),
    (
        "Drive",
        &[
            ModuleKind::Distortion,
            ModuleKind::WaveShaper,
            ModuleKind::Cabinet,
            ModuleKind::Exciter,
            ModuleKind::RingMod,
        ],
    ),
    (
        "Delay and reverb",
        &[
            ModuleKind::Delay,
            ModuleKind::Echo,
            ModuleKind::Multitap,
            ModuleKind::Reverb,
            ModuleKind::PlateReverb,
            ModuleKind::Convolver,
            ModuleKind::Repeater,
        ],
    ),
    (
        "Modulation",
        &[
            ModuleKind::Chorus,
            ModuleKind::Flanger,
            ModuleKind::Phaser,
            ModuleKind::Vibrato,
            ModuleKind::Lfo,
            ModuleKind::PitchShifter,
            ModuleKind::Vocoder,
        ],
    ),
    ("Dynamics", &[ModuleKind::Compressor, ModuleKind::Gate, ModuleKind::Maximizer, ModuleKind::Amplifier]),
    (
        "EQ and stereo",
        &[ModuleKind::Eq, ModuleKind::Eq5, ModuleKind::Eq10, ModuleKind::StereoExpander, ModuleKind::DcBlocker],
    ),
];

/// A menu of every effect, grouped by what it does, with a field to find
/// one by name, to add at place `at` of the chain.
fn add_effect_menu(ui: &mut egui::Ui, label: &str, at: usize, action: &mut Option<Action>) {
    widgets::field_menu(ui, label, |ui| {
        let key = egui::Id::new("effect_search");
        let mut search: String = ui.data(|d| d.get_temp(key)).unwrap_or_default();
        let field = ui.add(egui::TextEdit::singleline(&mut search).hint_text("Find an effect").desired_width(170.0));
        if !field.has_focus() && search.is_empty() {
            field.request_focus();
        }
        let wanted = search.trim().to_lowercase();
        let mut pick = None;
        widgets::menu_scroll(ui, "effects_to_add", |ui| {
            let mut groups: Vec<(&str, Vec<ModuleKind>)> =
                EFFECT_GROUPS.iter().map(|(name, kinds)| (*name, kinds.to_vec())).collect();
            // Any effect not in a group yet still shows, last.
            let rest: Vec<ModuleKind> = (ModuleKind::ADDABLE.into_iter())
                .filter(|k| k.is_effect() && *k != ModuleKind::Send)
                .filter(|k| !EFFECT_GROUPS.iter().any(|g| g.1.contains(k)))
                .collect();
            groups.push(("Other", rest));
            for (name, kinds) in groups {
                let kinds: Vec<ModuleKind> = kinds
                    .into_iter()
                    .filter(|k| wanted.is_empty() || k.name().to_lowercase().contains(&wanted))
                    .collect();
                if kinds.is_empty() {
                    continue;
                }
                ui.label(RichText::new(name).small().color(theme::TEXT_WEAK));
                for kind in kinds {
                    if ui.button(kind.name()).clicked() {
                        pick = Some(kind);
                    }
                }
            }
        });
        // Enter takes the first one found.
        if pick.is_none() && !wanted.is_empty() && field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            pick = ModuleKind::ADDABLE
                .into_iter()
                .find(|k| k.is_effect() && *k != ModuleKind::Send && k.name().to_lowercase().contains(&wanted));
        }
        if let Some(kind) = pick {
            *action = Some(Action::Add(at, kind));
            search.clear();
            ui.close();
        }
        ui.data_mut(|d| d.insert_temp(key, search));
    })
    .on_hover_text("Add an effect at the end of the chain");
}

/// A menu of the send tracks `owner`'s chain can send to, and a new one,
/// to add a Send at place `at`. The master can't send: what it sends
/// would come round to it.
fn add_send_menu(app: &App, ui: &mut egui::Ui, label: &str, owner: Owner, at: usize, action: &mut Option<Action>) {
    ui.menu_button(label, |ui| {
        let sends: Vec<u8> = app.project.sends.iter().map(|s| s.module).collect();
        widgets::menu_scroll(ui, "sends_to_add", |ui| {
            for to in sends.into_iter().filter(|&to| app.project.can_send(owner, to)) {
                if ui.button(format!("To {}", routing::label(app, to))).clicked() {
                    *action = Some(Action::AddSend(at, Some(to)));
                    ui.close();
                }
            }
        });
        let tip = "A send track of its own, for a reverb or a delay several chains share";
        if ui.button("To a New Send Track").on_hover_text(tip).clicked() {
            *action = Some(Action::AddSend(at, None));
            ui.close();
        }
    })
    .response
    .on_hover_text("Send a copy of the sound here to a send track: for effects several chains share");
}

/// A device folded to its name, written up a strip in its color; `on`
/// when it is the selected one.
fn folded_strip(ui: &mut egui::Ui, text: &str, color: Color32, on: bool, h: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(FOLDED_W, h), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let edge = if on {
        theme::SELECTED
    } else if resp.hovered() {
        theme::TEXT_WEAK
    } else {
        theme::FRAME_LINE
    };
    ui.painter().rect(rect, 3.0, theme::FRAME_BG, Stroke::new(1.0, edge), StrokeKind::Inside);
    let band =
        egui::Rect::from_min_size(rect.min + Vec2::new(3.0, 3.0), Vec2::new(rect.width() - 6.0, rect.height() - 6.0));
    ui.painter().rect_filled(band, 2.0, color.gamma_multiply(if resp.hovered() { 0.55 } else { 0.35 }));
    let galley = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(12.0), theme::TEXT);
    let at = egui::pos2(rect.center().x - galley.size().y / 2.0, rect.top() + 8.0 + galley.size().x);
    let shape = egui::epaint::TextShape::new(at, galley, theme::TEXT).with_angle(-std::f32::consts::FRAC_PI_2);
    ui.painter().with_clip_rect(band).add(shape);
    resp
}

/// The chain at a glance, above it: a chip for each device in order, a
/// click from scrolling to it, the Modulators, adding effects, Sends and
/// Modulators, and folding every device or none.
#[allow(clippy::too_many_arguments)]
fn overview(
    app: &mut App,
    ui: &mut egui::Ui,
    owner: Owner,
    path: &[u8],
    modulators: &[u8],
    len: usize,
    sound: bool,
    action: &mut Option<Action>,
) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        theme::caption(ui, "CHAIN");
        let chip = |app: &mut App, ui: &mut egui::Ui, id: u8| {
            let Some(m) = app.project.module(id) else { return };
            let off = m.mute || m.bypass;
            let color = if off { Color32::from_gray(80) } else { modules::module_color(m) };
            let text = m.name.clone();
            let tip =
                format!("{id:02X} {} · {}{}: click to show it", m.name, m.kind.name(), if off { ", off" } else { "" });
            if widgets::chip(ui, color, &text, app.selected_module == Some(id)).on_hover_text(tip).clicked() {
                app.chain_view.reveal(id);
                app.select(Some(id));
            }
        };
        for (k, &id) in path.iter().enumerate() {
            if k > 0 {
                ui.label(RichText::new("›").color(theme::TEXT_WEAK));
            }
            chip(app, ui, id);
        }
        if sound {
            add_effect_menu(ui, "+ Effect", len, action);
            if owner != Owner::Module(OUTPUT_ID) {
                add_send_menu(app, ui, "+ Send", owner, len, action);
            }
        }
        if !modulators.is_empty() {
            ui.separator();
            ui.label(RichText::new("Modulated by").small().color(theme::TEXT_WEAK));
            for &m in modulators {
                chip(app, ui, m);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let all: Vec<u8> = path.iter().chain(modulators).copied().collect();
            let any_open = all.iter().any(|id| !app.chain_view.folded.contains(id));
            let (label, tip) = if any_open {
                ("Fold All", "Fold every device to its name, to see the whole chain")
            } else {
                ("Open All", "Open every device")
            };
            if ui.small_button(label).on_hover_text(tip).clicked() {
                *action = Some(Action::FoldAll(any_open));
            }
        });
    });
}

/// Where a Send sends: a send track it can send to, picked from a list
/// that can also make a new one, and a way there.
fn send_target(app: &mut App, ui: &mut egui::Ui, id: u8) {
    let Some(owner) = app.project.owner_of(id) else { return };
    let to = app.project.module(id).and_then(|m| m.send_to);
    let shown = to.map_or_else(|| "Nowhere".to_string(), |t| routing::label(app, t));
    let mut picked = to;
    let mut new = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new("To").small());
        egui::ComboBox::from_id_salt(("send_to", id))
            .selected_text(shown)
            .width(ui.available_width() - 30.0)
            .show_ui(ui, |ui| {
                let sends: Vec<u8> = app.project.sends.iter().map(|s| s.module).collect();
                for s in sends.into_iter().filter(|&s| app.project.can_send(owner, s)) {
                    ui.selectable_value(&mut picked, Some(s), routing::label(app, s));
                }
                ui.separator();
                if ui.button("New Send Track").clicked() {
                    new = true;
                }
            })
            .response
            .on_hover_text("The send track it sends to");
        let show = ui.add_enabled_ui(to.is_some(), |ui| super::icons::button(ui, super::icons::Icon::Right)).inner;
        if show.on_hover_text("Show the send track").clicked()
            && let Some(t) = to
        {
            app.show_send(t);
        }
    });
    if new {
        let name = app.project.next_send_name();
        picked = app.project.add_send_track(&name);
    }
    if picked != to {
        let name = picked.and_then(|t| app.project.module(t)).map(|m| m.name.clone());
        let m = app.project.module_mut(id).unwrap();
        m.send_to = picked;
        // A Send named for where it went is named for where it goes.
        if m.name == "Send" || m.name.starts_with("To ") {
            m.name = name.map_or_else(|| "Send".to_string(), |n| format!("To {n}"));
        }
        app.mark();
    }
    if to.is_none() {
        ui.label(RichText::new("Pick a send track for it to send to.").small().color(theme::RECORD));
    }
}

/// The panel a send track's chain starts with: its name on its color, its
/// mute and solo, what sends to it and how much, and deleting it.
fn send_panel(app: &mut App, ui: &mut egui::Ui, id: u8, h: f32) {
    let Some(m) = app.project.module(id).cloned() else { return };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(DEVICE_W, h), Sense::hover());
    ui.painter().rect(rect, 3.0, theme::FRAME_BG, Stroke::new(1.0, theme::FRAME_LINE), StrokeKind::Inside);
    let inner = rect.shrink(5.0);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("send_panel", id))
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.shrink_clip_rect(inner);
    let ui = &mut child;
    let (head, resp) = ui.allocate_exact_size(Vec2::new(inner.width(), 20.0), Sense::click());
    let color = if m.mute { Color32::from_gray(80) } else { modules::module_color(&m) };
    ui.painter().rect_filled(head, 2.0, color);
    ui.painter().with_clip_rect(head.shrink(2.0)).text(
        head.left_center() + Vec2::new(5.0, 0.0),
        Align2::LEFT_CENTER,
        "SEND TRACK",
        FontId::proportional(11.0),
        theme::SELECTED_TEXT,
    );
    resp.on_hover_text("Right-click for its color").context_menu(|ui| {
        ui.menu_button("Color", |ui| routing::color_menu(app, ui, id));
    });
    let mut name = m.name.clone();
    if ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY)).changed() {
        app.project.module_mut(id).unwrap().name = name;
        app.mark_layout();
    }
    ui.horizontal(|ui| {
        if theme::toggle(ui, m.mute, "Mute").on_hover_text("Silence this send track").clicked() {
            app.project.module_mut(id).unwrap().mute = !m.mute;
            app.mark();
        }
        if theme::toggle(ui, m.solo, "Solo").on_hover_text("Hear only this, and what sends to it").clicked() {
            app.project.module_mut(id).unwrap().solo = !m.solo;
            app.mark();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let tip = "Delete this send track and its effects; Sends to it send nowhere";
            if super::icons::button(ui, super::icons::Icon::Bin).on_hover_text(tip).clicked() {
                app.project.remove_module(id);
                app.track_fx = super::TrackFx::Master;
                app.mark();
            }
        });
    });
    ui.add_space(6.0);
    theme::caption(ui, "FED BY");
    let sends: Vec<(u8, f32)> =
        (app.project.modules.iter()).filter(|s| s.send_to == Some(id)).map(|s| (s.id, s.params[0])).collect();
    if sends.is_empty() {
        let hint = "Nothing sends here yet. Add a Send to a chain with + Add Send.";
        ui.label(RichText::new(hint).small().color(theme::TEXT_WEAK));
    }
    egui::ScrollArea::vertical().id_salt(("fed_by", id)).auto_shrink(false).show(ui, |ui| {
        for (s, amount) in sends {
            let Some(owner) = app.project.owner_of(s) else { continue };
            let from = match owner {
                Owner::Track(t) => format!("Track {:02} {}", t + 1, app.project.track_name(t)),
                Owner::Module(o) => routing::label(app, o),
            };
            let text = format!("{from}  {}", super::mixer::db_text(amount));
            let resp = ui.add(egui::Button::new(RichText::new(text).small()).frame(false));
            if resp.on_hover_text("Show the chain this Send is in").clicked() {
                app.show_chain(owner);
            }
        }
    });
}

/// An instrument's macros: knobs that each move parameters of it and its
/// own effects, mapped from those parameters' menus, with what each moves
/// listed under it.
fn macros_panel(app: &mut App, ui: &mut egui::Ui, id: u8, h: f32) {
    let Some(m) = app.project.module(id).cloned() else { return };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(DEVICE_W * 0.9, h), Sense::hover());
    ui.painter().rect(rect, 3.0, theme::FRAME_BG, Stroke::new(1.0, theme::FRAME_LINE), StrokeKind::Inside);
    let inner = rect.shrink(5.0);
    let mut child = ui.new_child(
        egui::UiBuilder::new().id_salt(("macros", id)).max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.shrink_clip_rect(inner);
    let ui = &mut child;
    ui.horizontal(|ui| {
        theme::caption(ui, "MACROS");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if super::icons::button(ui, super::icons::Icon::Minus).on_hover_text("Fold the macros away").clicked() {
                app.chain_view.macros.insert(id, false);
            }
        });
    });
    let hint = "Right-click a parameter of the instrument or its effects and choose Map to Macro.";
    ui.label(RichText::new(hint).small().color(theme::TEXT_WEAK));
    ui.add_space(3.0);
    let n = m.kind.params().len();
    let mut automate = None;
    let mut modulate = None;
    let modulators: Vec<u8> = app.project.modules.iter().filter(|m| m.kind.controls()).map(|m| m.id).collect();
    egui::ScrollArea::vertical().id_salt(("macro_list", id)).auto_shrink(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        for k in 0..MACROS {
            let Some(spec) = m.kind.automatable(n + 2 + k) else { continue };
            let mac = m.macros.get(k).cloned().unwrap_or_default();
            let shown = app.automated(id, n + 2 + k).unwrap_or(mac.value);
            let (mut value, mut name, mut clear) = (shown, mac.name.clone(), false);
            let resp = widgets::named_bar(ui, spec, &m.macro_name(k), &mut value, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut name);
                });
                if ui.button("Automate in This Pattern").clicked() {
                    automate = Some(n + 2 + k);
                    ui.close();
                }
                ui.menu_button("Modulate With", |ui| {
                    if ui.button("New Modulator").clicked() {
                        modulate = Some((None, n + 2 + k));
                        ui.close();
                    }
                    widgets::menu_scroll(ui, "macro_modulate_with", |ui| {
                        for &by in &modulators {
                            if ui.button(routing::label(app, by)).clicked() {
                                modulate = Some((Some(by), n + 2 + k));
                                ui.close();
                            }
                        }
                    });
                });
                if !mac.targets.is_empty() && ui.button("Clear Targets").clicked() {
                    clear = true;
                    ui.close();
                }
            });
            if mac.targets.is_empty() {
                resp.on_hover_text("Moves nothing yet");
            }
            let mut targets = mac.targets.clone();
            let mut remove = None;
            for (t, target) in targets.iter_mut().enumerate() {
                let Some(tm) = app.project.module(target.module) else { continue };
                let Some(tspec) = tm.kind.automatable(target.param) else { continue };
                let what = format!("  {:02X} {} · {}", target.module, tm.name, tm.automatable_name(target.param));
                ui.horizontal(|ui| {
                    let tip = "Right-click to set what it moves the parameter from and to";
                    let label = ui.add(
                        egui::Label::new(RichText::new(what).small().color(theme::TEXT_WEAK))
                            .truncate()
                            .sense(Sense::click()),
                    );
                    widgets::field_context_menu(&label.on_hover_text(tip), |ui| {
                        ui.set_width(180.0);
                        for (end, at) in [("From", &mut target.from), ("To", &mut target.to)] {
                            let mut v = tspec.value_at(*at);
                            widgets::named_bar(ui, tspec, end, &mut v, |_| {});
                            *at = tspec.position(v);
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if super::icons::button(ui, super::icons::Icon::Bin).on_hover_text("Stop moving it").clicked() {
                            remove = Some(t);
                        }
                    });
                });
            }
            if let Some(t) = remove {
                targets.remove(t);
            }
            if clear {
                targets.clear();
            }
            let renamed = name != mac.name;
            if value != shown || renamed || targets != mac.targets {
                let mac = app.project.module_mut(id).unwrap().macro_mut(k);
                if value != shown {
                    mac.value = value;
                }
                (mac.name, mac.targets) = (name, targets);
                app.mark();
            }
            ui.add_space(2.0);
        }
    });
    if let Some(i) = automate {
        super::automation::add(app, id, i);
        app.lower = super::Lower::Automation;
    }
    if let Some((by, i)) = modulate {
        modules::modulate_with(app, by, id, i);
    }
}
