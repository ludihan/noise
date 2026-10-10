//! An instrument's device chain: the
//! instrument itself and the effects only it feeds, each a panel of
//! parameters, in the order the sound goes through them, then the effects
//! it shares with other instruments, where the sound goes, and the
//! Modulators moving any of them. Everything about an instrument's sound is
//! edited here. The output's page is the master chain: the effects the
//! whole mix goes through, then the output; a track's page is its own
//! effects, which what it plays goes through on its way to the master.

use super::{App, modules, pattern, routing, theme, widgets};
use crate::project::{MACROS, ModuleKind, OUTPUT_ID, Owner};
use eframe::egui::{self, Align2, Color32, FontId, RichText, Sense, Stroke, StrokeKind, Vec2};

/// Width of a device panel.
const DEVICE_W: f32 = 215.0;

/// What a device is to the instrument whose chain is shown.
#[derive(Clone, Copy, PartialEq)]
enum Place {
    Instrument,
    /// The output, at the end of the master chain.
    Output,
    /// Its own effect `k` of `n`.
    Own(usize, usize),
    /// An effect other instruments go through too.
    Shared,
    Modulator,
}

enum Action {
    Add(usize, ModuleKind),
    Remove(u8),
    Delete(u8),
    Move(u8, i32),
    /// A Modulator, starting on this device.
    AddModulator(u8),
    Select(u8),
    /// An effect dragged by its name and let go at this x.
    Drop(u8, f32),
}

/// The effects after the chain: those its outputs lead through, in the
/// order the sound reaches them, up to the output.
fn shared_effects(app: &App, outputs: &[u8]) -> Vec<u8> {
    let mut found = Vec::new();
    let mut queue: Vec<u8> = outputs.to_vec();
    while !queue.is_empty() {
        let at = queue.remove(0);
        let Some(m) = app.project.module(at) else { continue };
        if at == OUTPUT_ID || found.contains(&at) || !m.kind.makes_sound() {
            continue;
        }
        found.push(at);
        queue.extend(app.project.links.iter().filter(|l| l.0 == at).map(|l| l.1));
    }
    found
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
    // A MultiSynth's links carry notes: it has no sound to follow.
    let notes_only = inst.as_ref().is_some_and(|m| m.kind.notes_only());
    let chain = app.project.chain(owner);
    let shared = if notes_only || track.is_some() { Vec::new() } else { shared_effects(app, &chain.outputs) };
    // The Modulators moving anything this sound goes through.
    let mut reach: Vec<u8> = id.into_iter().collect();
    reach.extend(&chain.effects);
    reach.extend(&shared);
    let modulators: Vec<u8> = app
        .project
        .modules
        .iter()
        .filter(|m| m.kind.controls() && app.project.links.iter().any(|l| l.0 == m.id && reach.contains(&l.1)))
        .map(|m| m.id)
        .collect();
    let mut action = None;
    let h = ui.available_height() - 8.0;
    egui::ScrollArea::horizontal().id_salt("chain").auto_shrink(false).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.set_min_height(h);
            match (id, track) {
                (Some(id), _) if !master => {
                    if inst.as_ref().is_some_and(|m| m.kind.plays_sound()) {
                        macros_panel(app, ui, id, h);
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
                let last = chain.effects.last().copied().or(id).unwrap_or(OUTPUT_ID);
                ending(app, ui, owner, last, chain.effects.len(), h, &mut action);
                if master {
                    arrow(ui, h);
                    device(app, ui, OUTPUT_ID, Place::Output, h, &mut action);
                }
                if let Some(t) = track {
                    arrow(ui, h);
                    to_master(app, ui, t, h);
                }
                for &e in &shared {
                    arrow(ui, h);
                    device(app, ui, e, Place::Shared, h, &mut action);
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
                    let tip = "A Modulator moves a parameter with an LFO, or by following a sound's level";
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
                app.mark();
            }
            None => app.set_status("Too many modules"),
        },
        Some(Action::Select(m)) => app.selected_module = Some(m),
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
    let (rect, _) = ui.allocate_exact_size(Vec2::new(DEVICE_W, h), Sense::hover());
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
    let tag = if place == Place::Shared { format!("{} · shared", m.kind.name()) } else { m.kind.name().to_string() };
    ui.painter().text(
        head.right_center() - Vec2::new(5.0, 0.0),
        Align2::RIGHT_CENTER,
        tag,
        FontId::proportional(10.5),
        theme::SELECTED_TEXT,
    );
    let tip = if own {
        "Drag to move it along the chain; click to select it in the mixer, module list and automation; right-click for presets and its color"
    } else {
        "Click to select it in the mixer, the module list and automation; right-click for presets and its color"
    };
    let resp = resp.on_hover_text(tip);
    if resp.clicked() {
        *action = Some(Action::Select(id));
    }
    resp.context_menu(|ui| {
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
            Place::Own(..) | Place::Shared => {
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
        if !matches!(place, Place::Instrument | Place::Output) {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let tip = if place == Place::Shared { "Delete this effect, for every instrument" } else { "Remove it" };
                if super::icons::button(ui, super::icons::Icon::Bin).on_hover_text(tip).clicked() {
                    *action =
                        Some(if matches!(place, Place::Own(..)) { Action::Remove(id) } else { Action::Delete(id) });
                }
            });
        }
    });
    if place == Place::Shared {
        let from: Vec<String> =
            app.project.links.iter().filter(|l| l.1 == id).map(|l| routing::label(app, l.0)).collect();
        ui.label(RichText::new(format!("Fed by {}", from.join(", "))).small().color(theme::TEXT_WEAK));
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
            routing::sends_checklist(app, ui, id);
        }
        if place == Place::Shared {
            ui.add_space(6.0);
            theme::caption(ui, "SENDS TO");
            routing::sends_checklist(app, ui, id);
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

/// After the instrument's own devices: adding an effect, and where the
/// sound of `last` goes.
fn ending(app: &mut App, ui: &mut egui::Ui, owner: Owner, last: u8, len: usize, h: f32, action: &mut Option<Action>) {
    ui.vertical(|ui| {
        ui.set_width(150.0);
        ui.set_max_height(h);
        ui.menu_button("+ Add Effect", |ui| {
            for kind in ModuleKind::ADDABLE.into_iter().filter(|k| k.has_input() && k.has_output() && k.makes_sound()) {
                if ui.button(kind.name()).clicked() {
                    *action = Some(Action::Add(len, kind));
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("At the end of the chain");
        let id = match owner {
            Owner::Module(OUTPUT_ID) | Owner::Track(_) => {
                let hint = if owner == Owner::Module(OUTPUT_ID) {
                    "Effects here process the whole mix, in order, before the output. Drag them by their names to reorder them."
                } else {
                    "Effects here process what this track plays, in order. Drag them by their names to reorder them."
                };
                ui.add_space(8.0);
                ui.label(RichText::new(hint).small().color(theme::TEXT_WEAK));
                return;
            }
            Owner::Module(id) => id,
        };
        ui.add_space(8.0);
        theme::caption(ui, "SENDS TO");
        let hint = "The output, or effects other instruments go through too. Several at once split the sound.";
        ui.label(RichText::new(hint).small().color(theme::TEXT_WEAK));
        let mut own = app.project.chain(id).effects;
        own.push(id);
        egui::ScrollArea::vertical().id_salt(("sends", id)).show(ui, |ui| routing::sends_checklist_except(app, ui, last, &own));
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
    theme::caption(ui, "MACROS");
    let hint = "Right-click a parameter of the instrument or its effects and choose Map to Macro.";
    ui.label(RichText::new(hint).small().color(theme::TEXT_WEAK));
    ui.add_space(3.0);
    let n = m.kind.params().len();
    let mut automate = None;
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
                    label.on_hover_text(tip).context_menu(|ui| {
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
}
