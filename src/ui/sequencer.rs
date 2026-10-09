//! The pattern sequencer, with the pattern matrix beside it: a block
//! for every track of every slot in the song, showing which tracks have
//! notes there and which the slot mutes.

use super::icons::Icon;
use super::{App, pattern, theme};
use crate::project::{Pattern, Slot, TrackCells};
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};

/// Size of a matrix block.
const BLOCK: f32 = 15.0;
/// Height of the track header over the matrix: a color bar and the
/// track's name, turned on its side.
const HEADER_H: f32 = 64.0;
/// Width of the slot buttons' column, left of the list.
pub const TOOLBAR_W: f32 = 26.0;

/// Tracks the matrix shows: as many as the widest pattern in the song.
fn matrix_tracks(app: &App) -> usize {
    let used = app.project.order.iter().filter_map(|s| app.project.patterns.get(s.pattern));
    used.map(Pattern::num_tracks).max().unwrap_or(1)
}

/// Width the matrix takes beside the sequencer, in the extended view.
pub fn matrix_width(app: &App) -> f32 {
    if app.show_matrix { matrix_tracks(app) as f32 * BLOCK + 4.0 } else { 0.0 }
}

/// The track header over the matrix columns starting at `x`, in `header`:
/// each track's color and name. Click a
/// track to mute it, right-click it to solo it.
fn track_header(app: &App, ui: &mut egui::Ui, header: Rect, x: f32, tracks: usize) -> Option<Action> {
    let mut action = None;
    for t in 0..tracks {
        let col = Rect::from_min_size(Pos2::new(x + t as f32 * BLOCK, header.top()), Vec2::new(BLOCK - 2.0, HEADER_H));
        let info = app.project.tracks.get(t).cloned().unwrap_or_default();
        let audible = app.project.track_audible(t);
        let color = if audible { pattern::track_color(&app.project, t) } else { Color32::from_gray(60) };
        let painter = ui.painter().with_clip_rect(col);
        painter.rect_filled(col, 1.0, color.gamma_multiply(0.25));
        painter.rect_filled(Rect::from_min_size(col.min, Vec2::new(col.width(), 5.0)), 1.0, color);
        if info.solo {
            painter.rect_stroke(col.shrink(0.5), 1.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
        }
        // The name reads upwards from the bottom of the column.
        let name = app.project.track_name(t);
        let text = if audible { theme::TEXT } else { theme::TEXT_WEAK };
        let galley = painter.layout_no_wrap(name.clone(), egui::FontId::proportional(10.5), text);
        let at = Pos2::new(col.center().x - galley.size().y / 2.0, col.bottom() - 3.0);
        painter.add(egui::epaint::TextShape::new(at, galley, text).with_angle(-std::f32::consts::FRAC_PI_2));
        let state = if info.mute {
            " (muted)"
        } else if info.solo {
            " (solo)"
        } else {
            ""
        };
        let resp = ui.interact(col, ui.id().with(("matrix_track", t)), Sense::click());
        let resp = resp.on_hover_text(format!("{name}{state}\nClick to mute, right-click to solo"));
        if resp.clicked() {
            action = Some(Action::TrackMute(t));
        } else if resp.secondary_clicked() {
            action = Some(Action::TrackSolo(t));
        }
    }
    action
}

/// A bit for each track of the pattern in slot `i`.
fn slot_tracks(app: &App, i: usize) -> u32 {
    let n = app.project.patterns[app.project.order[i].pattern].num_tracks();
    if n >= 32 { u32::MAX } else { (1u32 << n) - 1 }
}

/// Bit `t` is set when track `t` of `p` holds anything.
fn content(p: &Pattern) -> u32 {
    (0..p.num_tracks()).filter(|&t| p.track_used(t)).fold(0, |m, t| m | 1 << t)
}

/// What can be typed into in the sequencer: a section's name, or a slot's
/// pattern number or the pattern's name.
#[derive(Clone, Copy, PartialEq)]
enum Field {
    Section,
    Number,
    Name,
}

/// What the buttons down the sequencer's left, and a slot's menu, do to a
/// slot.
#[derive(Clone, Copy, PartialEq)]
enum SlotOp {
    New,
    Clone,
    Repeat,
    Remove,
    Up,
    Down,
}

impl SlotOp {
    /// Each one's icon, menu entry and tip, in the buttons' order.
    const ALL: [(SlotOp, Icon, &str, &str); 6] = [
        (SlotOp::New, Icon::Plus, "Insert New Pattern", "Insert a new empty pattern"),
        (SlotOp::Clone, Icon::Clone, "Clone Pattern", "Insert a copy of this pattern"),
        (SlotOp::Repeat, Icon::Loop, "Repeat Pattern", "Repeat: play this pattern again in a new slot"),
        (SlotOp::Remove, Icon::Minus, "Remove Slot", "Remove this slot"),
        (SlotOp::Up, Icon::Up, "Move Up", "Move this slot up"),
        (SlotOp::Down, Icon::Down, "Move Down", "Move this slot down"),
    ];

    /// Whether it can be done to slot `i` of `slots`.
    fn allowed(self, i: usize, slots: usize) -> bool {
        match self {
            SlotOp::Remove => slots > 1,
            SlotOp::Up => i > 0,
            SlotOp::Down => i + 1 < slots,
            _ => true,
        }
    }
}

/// Does `op` to the selected slot.
fn slot_op(app: &mut App, op: SlotOp) {
    let i = app.slot;
    match op {
        SlotOp::New => {
            let (tracks, lines) = (app.pattern().num_tracks(), app.pattern().lines);
            let n = app.project.patterns.len();
            app.project.patterns.push(Pattern::new("", tracks, lines));
            app.project.sync_columns(n);
            app.slot += 1;
            app.project.insert_slot(app.slot, Slot::new(n));
        }
        SlotOp::Clone => {
            let mut p = app.pattern().clone();
            if !p.name.is_empty() {
                p.name.push_str(" copy");
            }
            let n = app.project.patterns.len();
            app.project.patterns.push(p);
            app.slot += 1;
            app.project.insert_slot(app.slot, Slot { pattern: n, muted: app.project.order[i].muted });
        }
        SlotOp::Repeat => {
            app.slot += 1;
            app.project.insert_slot(app.slot, app.project.order[i]);
        }
        SlotOp::Remove => app.project.remove_slot(i),
        SlotOp::Up => {
            app.project.swap_slots(i, i - 1);
            app.slot -= 1;
        }
        SlotOp::Down => {
            app.project.swap_slots(i, i + 1);
            app.slot += 1;
        }
    }
    app.clamp_cursor();
    app.mark();
}

/// Types into `text` over `rect`, starting with all of it selected so
/// typing replaces it. Says how the typing ended, once it has: true to
/// keep it (Enter, or clicking elsewhere), false on Escape.
fn edit_text(ui: &mut egui::Ui, rect: Rect, text: &mut String) -> Option<bool> {
    let edit = egui::TextEdit::singleline(text).font(egui::FontId::proportional(11.5));
    let mut out = ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| edit.show(ui)).inner;
    let edit = &out.response;
    if !edit.has_focus() && !edit.lost_focus() {
        edit.request_focus();
        let all = egui::text::CCursorRange::select_all(&out.galley);
        out.state.cursor.set_char_range(Some(all));
        out.state.store(ui.ctx(), edit.id);
    }
    out.response.lost_focus().then(|| !ui.input(|i| i.key_pressed(egui::Key::Escape)))
}

/// The matrix selection, the matrix clipboard and what is being typed.
#[derive(Default)]
pub struct State {
    /// Selected slots and tracks, both inclusive.
    selection: Option<((usize, usize), (usize, usize))>,
    clip: Option<Vec<Vec<TrackCells>>>,
    /// The slot whose section name, pattern number or pattern name is
    /// being typed, and the text so far.
    renaming: Option<(usize, Field, String)>,
    /// How far a Shift+drag on a slot's number has gone towards the next
    /// pattern.
    drag: f32,
    /// The slot being dragged to another place in the list.
    moving: Option<usize>,
}

/// How far a drag on a slot's number goes for each pattern.
const DRAG_STEP: f32 = 8.0;

fn span(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
}

enum Action {
    Select(usize, Option<usize>),
    /// Extend the matrix selection from the cursor to this slot and track.
    Extend(usize, usize),
    Mute(usize, usize),
    MuteEverywhere(usize, bool),
    /// Copy, cut, paste or clear the matrix selection, or the block at
    /// this slot and track when it is outside the selection.
    Copy(usize, usize),
    Cut(usize, usize),
    Paste(usize, usize),
    Clear(usize, usize),
    MuteBlock(usize, usize),
    AddSection(usize),
    RenameSection(usize),
    RemoveSection(usize),
    SelectSection(usize),
    /// Mute or solo a track everywhere, as its header does.
    TrackMute(usize),
    TrackSolo(usize),
    /// Mute or unmute every track in slot `i`.
    MuteSlot(usize, bool),
    /// Keep only track `t` playing in slot `i`.
    SoloHere(usize, usize),
    /// Do something to slot `i`, as the buttons do to the selected one.
    Slot(usize, SlotOp),
    /// Start typing into a field of slot `i`.
    Edit(usize, Field),
    /// Play pattern `n` in slot `i`; one past the last is a new pattern.
    SetPattern(usize, usize),
    /// Move slot `from` to `to`, as dropped there.
    Move(usize, usize),
}

pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    theme::caption(ui, if app.show_matrix { "PATTERN SEQUENCER · MATRIX" } else { "SEQUENCER" });
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(TOOLBAR_W - 4.0);
            buttons(app, ui);
        });
        ui.vertical(|ui| slots(app, ui));
    });
}

/// The sequence: the sections' labels and each slot's pattern, with the
/// pattern names and the matrix in the extended view.
fn slots(app: &mut App, ui: &mut egui::Ui) {
    let extended = app.show_matrix;
    let (play_order, _) = app.play_position();
    let playing = app.is_playing();
    let tracks = matrix_tracks(app);
    let contents: Vec<u32> = app.project.patterns.iter().map(content).collect();
    let mut action = None;
    let mut changed = false;
    egui::Frame::new().fill(theme::INSET).inner_margin(2).show(ui, |ui| {
        // Track colors over the matrix columns, placed once the rows say
        // where the columns are.
        let (header, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), if extended { HEADER_H } else { 0.0 }),
            Sense::hover(),
        );
        let mut columns_x = None;
        let list = egui::ScrollArea::vertical().auto_shrink(false);
        list.show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let n_pat = app.project.patterns.len();
            // Each slot's row, for dropping a dragged slot between them.
            let mut rows = Vec::with_capacity(app.project.order.len());
            for i in 0..app.project.order.len() {
                let slot = app.project.order[i];
                if let Some(a) = section_header(app, ui, i) {
                    action = Some(a);
                }
                let row = ui.horizontal(|ui| {
                    let marker = if playing && i == play_order { "•" } else { " " };
                    ui.label(RichText::new(marker).monospace().color(theme::SCOPE));
                    if extended {
                        ui.label(RichText::new(format!("{i:02}")).monospace().color(theme::TEXT_WEAK));
                    }
                    // The pattern number: a click selects the slot, a drag up
                    // or down picks its pattern and a double-click types it.
                    // In the normal view it fills the row and is lit when
                    // selected.
                    let h = ui.spacing().interact_size.y;
                    let w = if extended { 32.0 } else { ui.available_width() };
                    let typing = app.sequencer.renaming.as_ref().filter(|r| r.0 == i).map(|r| r.1);
                    let number = if typing == Some(Field::Number) {
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
                        if let Some((_, _, text)) = &mut app.sequencer.renaming
                            && let Some(keep) = edit_text(ui, rect, text)
                        {
                            let typed = text.trim().parse::<usize>().ok().filter(|_| keep);
                            app.sequencer.renaming = None;
                            action = typed.map(|n| Action::SetPattern(i, n));
                        }
                        None
                    } else {
                        let lit = !extended && i == app.slot;
                        let text = RichText::new(format!("{:02}", slot.pattern)).monospace();
                        let button = egui::Button::selectable(lit, text).sense(Sense::click_and_drag());
                        let tip = "The pattern this slot plays: click to edit it, drag to move the slot, Shift+drag to pick another pattern, double-click to type its number";
                        Some(ui.add_sized([w, h], button).on_hover_text(tip))
                    };
                    if let Some(number) = &number {
                        if number.drag_started() {
                            // A drag moves the slot; with Shift it picks the pattern.
                            app.sequencer.drag = 0.0;
                            app.sequencer.moving = (!ui.input(|i| i.modifiers.shift)).then_some(i);
                            action = Some(Action::Select(i, None));
                        }
                        if number.dragged() && app.sequencer.moving.is_none() {
                            app.sequencer.drag -= number.drag_delta().y;
                            let steps = (app.sequencer.drag / DRAG_STEP).trunc();
                            if steps != 0.0 {
                                app.sequencer.drag -= steps * DRAG_STEP;
                                let pat = (slot.pattern as i64 + steps as i64).clamp(0, n_pat as i64 - 1);
                                app.project.order[i].pattern = pat as usize;
                                changed = true;
                            }
                        }
                        if number.double_clicked() {
                            action = Some(Action::Edit(i, Field::Number));
                        } else if number.clicked() {
                            action = Some(Action::Select(i, None));
                        }
                    }
                    // The pattern's name, in the extended view: double-click
                    // it to rename the pattern.
                    let name = if !extended {
                        None
                    } else {
                        let matrix_w = tracks as f32 * BLOCK;
                        let name_w = (ui.available_width() - matrix_w - ui.spacing().item_spacing.x).max(12.0);
                        if typing == Some(Field::Name) {
                            let (rect, _) = ui.allocate_exact_size(Vec2::new(name_w, h), Sense::hover());
                            if let Some((_, _, text)) = &mut app.sequencer.renaming
                                && let Some(keep) = edit_text(ui, rect, text)
                            {
                                let typed = text.trim().to_string();
                                app.sequencer.renaming = None;
                                if keep && typed != app.project.patterns[slot.pattern].name {
                                    app.project.patterns[slot.pattern].name = typed;
                                    app.mark_layout();
                                }
                            }
                            None
                        } else {
                            let name = app.project.patterns[slot.pattern].name.as_str();
                            let label = egui::Button::selectable(i == app.slot, name).truncate();
                            let layout = egui::Layout::top_down_justified(egui::Align::Min);
                            let label = label.sense(Sense::click_and_drag());
                            let resp = ui.allocate_ui_with_layout(Vec2::new(name_w, h), layout, |ui| ui.add(label)).inner;
                            if resp.drag_started() {
                                app.sequencer.moving = Some(i);
                                action = Some(Action::Select(i, None));
                            }
                            if resp.double_clicked() {
                                action = Some(Action::Edit(i, Field::Name));
                            } else if resp.clicked() {
                                action = Some(Action::Select(i, None));
                            }
                            Some(resp)
                        }
                    };
                    let has_section = app.project.section_at(i).is_some();
                    let slots = app.project.order.len();
                    for resp in number.iter().chain(&name) {
                        resp.context_menu(|ui| slot_menu(ui, i, slots, has_section, &mut action));
                    }
                    if extended {
                        let matrix_w = tracks as f32 * BLOCK;
                        let has = contents[slot.pattern];
                        let width = app.project.patterns[slot.pattern].num_tracks();
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(matrix_w, BLOCK), Sense::hover());
                        columns_x.get_or_insert(rect.left());
                        for t in 0..tracks {
                            if let Some(a) = block(app, ui, rect, i, t, slot, t < width, has >> t & 1 == 1) {
                                action = Some(a);
                            }
                        }
                    }
                });
                rows.push(row.response.rect);
            }
            if let Some(from) = app.sequencer.moving
                && let Some(a) = drop_slot(ui, &rows, from)
            {
                action = Some(a);
            }
        });
        if let Some(x) = columns_x
            && let Some(a) = track_header(app, ui, header, x, tracks)
        {
            action = Some(a);
        }
    });

    // What a block's menu acts on: the selection if the block is in it.
    let target = |app: &App, i: usize, t: usize| match app.sequencer.selection {
        Some((s, k)) if (s.0..=s.1).contains(&i) && (k.0..=k.1).contains(&t) => (s, k),
        _ => ((i, i), (t, t)),
    };
    match action {
        Some(Action::Select(i, track)) => {
            app.slot = i;
            if let Some(t) = track {
                app.cursor.track = t;
                app.cursor.column = 0;
                app.sequencer.selection = Some(((i, i), (t, t)));
            } else {
                app.sequencer.selection = None;
            }
            app.clamp_cursor();
        }
        Some(Action::Extend(i, t)) => {
            app.sequencer.selection = Some((span(app.slot, i), span(app.cursor.track, t)));
        }
        Some(Action::Copy(i, t) | Action::Cut(i, t)) => {
            let (slots, tracks) = target(app, i, t);
            app.sequencer.clip = Some(app.project.copy_matrix(slots, tracks));
            if matches!(action, Some(Action::Cut(..))) {
                app.project.clear_matrix(slots, tracks);
                app.mark();
            }
            let n = (slots.1 - slots.0 + 1) * (tracks.1 - tracks.0 + 1);
            app.set_status(format!(
                "{} {n} matrix blocks",
                if matches!(action, Some(Action::Cut(..))) { "Cut" } else { "Copied" }
            ));
        }
        Some(Action::Paste(i, t)) => {
            if let Some(clip) = app.sequencer.clip.clone() {
                app.project.paste_matrix(&clip, i, t);
                let h = clip.len().min(app.project.order.len() - i);
                let w = clip.first().map_or(1, Vec::len);
                app.sequencer.selection = Some(((i, i + h - 1), (t, t + w - 1)));
                app.clamp_cursor();
                app.mark();
            }
        }
        Some(Action::Clear(i, t)) => {
            let (slots, tracks) = target(app, i, t);
            app.project.clear_matrix(slots, tracks);
            app.mark();
        }
        Some(Action::MuteBlock(i, t)) => {
            // Mutes the whole block, or unmutes it when it is all muted.
            let (slots, tracks) = target(app, i, t);
            let all = (slots.0..=slots.1).all(|s| (tracks.0..=tracks.1).all(|k| app.project.order[s].is_muted(k)));
            for s in slots.0..=slots.1 {
                for k in tracks.0..=tracks.1 {
                    if app.project.order[s].is_muted(k) == all {
                        app.project.order[s].toggle_mute(k);
                    }
                }
            }
            app.mark();
        }
        Some(Action::AddSection(i)) => {
            app.project.set_section(i, format!("Section {}", app.project.sections.len() + 1));
            app.sequencer.renaming = app.project.section_at(i).map(|s| (i, Field::Section, s.name.clone()));
            app.mark_layout();
        }
        Some(Action::RenameSection(i)) => {
            app.sequencer.renaming = app.project.section_at(i).map(|s| (i, Field::Section, s.name.clone()));
        }
        Some(Action::Slot(i, op)) => {
            app.slot = i;
            app.sequencer.selection = None;
            slot_op(app, op);
        }
        Some(Action::Edit(i, field)) => {
            let pattern = &app.project.patterns[app.project.order[i].pattern];
            let text = if field == Field::Name {
                pattern.name.clone()
            } else {
                format!("{:02}", app.project.order[i].pattern)
            };
            app.slot = i;
            app.clamp_cursor();
            app.sequencer.renaming = Some((i, field, text));
        }
        Some(Action::Move(from, to)) => {
            app.sequencer.moving = None;
            if from != to {
                app.project.move_slot(from, to);
                app.slot = to;
                app.sequencer.selection = None;
                app.clamp_cursor();
                app.mark();
            }
        }
        Some(Action::SetPattern(i, n)) => {
            // A number past the last pattern makes a new empty one.
            let n = n.min(app.project.patterns.len());
            if n == app.project.patterns.len() {
                let (tracks, lines) = (app.pattern().num_tracks(), app.pattern().lines);
                app.project.patterns.push(Pattern::new("", tracks, lines));
                app.project.sync_columns(n);
                app.set_status(format!("Pattern {n:02} is new"));
            }
            app.project.order[i].pattern = n;
            app.slot = i;
            changed = true;
        }
        Some(Action::RemoveSection(i)) => {
            app.project.remove_section(i);
            app.mark_layout();
        }
        Some(Action::SelectSection(i)) => {
            let slots = app.project.section_slots(i);
            app.slot = i;
            app.sequencer.selection = Some((slots, (0, tracks - 1)));
            app.clamp_cursor();
        }
        Some(Action::Mute(i, t)) => {
            app.project.order[i].toggle_mute(t);
            app.mark();
        }
        Some(Action::TrackMute(t)) => {
            if let Some(info) = app.project.tracks.get_mut(t) {
                info.mute = !info.mute;
            }
            app.mark();
        }
        Some(Action::TrackSolo(t)) => {
            app.project.solo_track(t);
            app.mark();
        }
        Some(Action::MuteSlot(i, mute)) => {
            let all = slot_tracks(app, i);
            app.project.order[i].muted = if mute { all } else { 0 };
            app.mark();
        }
        Some(Action::SoloHere(i, t)) => {
            let all = slot_tracks(app, i);
            app.project.order[i].muted = all & !(1u32 << t);
            app.mark();
        }
        Some(Action::MuteEverywhere(t, mute)) => {
            for slot in &mut app.project.order {
                if slot.is_muted(t) != mute {
                    slot.toggle_mute(t);
                }
            }
            app.mark();
        }
        None => {}
    }
    if changed {
        app.clamp_cursor();
        app.mark();
    }
}

/// The matrix block for track `t` of slot `i`, in the row's `rect`.
#[allow(clippy::too_many_arguments)]
fn block(
    app: &App,
    ui: &mut egui::Ui,
    rect: Rect,
    i: usize,
    t: usize,
    slot: Slot,
    exists: bool,
    has_notes: bool,
) -> Option<Action> {
    let r = Rect::from_min_size(Pos2::new(rect.left() + t as f32 * BLOCK, rect.top() + 1.0), Vec2::splat(BLOCK - 2.0));
    let painter = ui.painter();
    if !exists {
        painter.circle_filled(r.center(), 1.0, Color32::from_gray(50));
        return None;
    }
    let color = pattern::track_color(&app.project, t);
    let muted = slot.is_muted(t);
    if muted {
        painter.rect_filled(r, 1.0, Color32::from_gray(45));
        let cross = Stroke::new(1.0, Color32::from_gray(110));
        painter.line_segment([r.left_top(), r.right_bottom()], cross);
        painter.line_segment([r.right_top(), r.left_bottom()], cross);
    } else if has_notes {
        painter.rect_filled(r, 1.0, color.gamma_multiply(0.85));
    } else {
        painter.rect_stroke(r, 1.0, Stroke::new(1.0, color.gamma_multiply(0.35)), StrokeKind::Inside);
    }
    if let Some((s, k)) = app.sequencer.selection
        && (s.0..=s.1).contains(&i)
        && (k.0..=k.1).contains(&t)
    {
        painter.rect_stroke(r.expand(1.0), 1.0, Stroke::new(1.5, theme::SELECTED), StrokeKind::Outside);
    }
    if i == app.slot && t == app.cursor.track {
        painter.rect_stroke(r.expand(1.0), 1.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Outside);
    }

    let resp = ui.interact(r, ui.id().with(("block", i, t)), Sense::click());
    let state = match (muted, has_notes) {
        (true, _) => "muted here",
        (false, true) => "has notes",
        (false, false) => "empty",
    };
    let tip = "Click to edit, Shift+click to select a block, Ctrl+click to mute here";
    let resp = resp.on_hover_text(format!("{}: {state}\n{tip}", app.project.track_name(t)));
    let mut action = None;
    if resp.clicked() {
        let m = ui.input(|i| i.modifiers);
        action = Some(if m.command {
            Action::Mute(i, t)
        } else if m.shift {
            Action::Extend(i, t)
        } else {
            Action::Select(i, Some(t))
        });
    }
    let has_clip = app.sequencer.clip.is_some();
    resp.context_menu(|ui| {
        if ui.button("Copy").clicked() {
            action = Some(Action::Copy(i, t));
            ui.close();
        }
        if ui.button("Cut").clicked() {
            action = Some(Action::Cut(i, t));
            ui.close();
        }
        if ui.add_enabled(has_clip, egui::Button::new("Paste")).clicked() {
            action = Some(Action::Paste(i, t));
            ui.close();
        }
        if ui.button("Clear").clicked() {
            action = Some(Action::Clear(i, t));
            ui.close();
        }
        if ui.button("Mute / Unmute Block").clicked() {
            action = Some(Action::MuteBlock(i, t));
            ui.close();
        }
        ui.separator();
        if ui.button(if muted { "Unmute Here" } else { "Mute Here" }).clicked() {
            action = Some(Action::Mute(i, t));
            ui.close();
        }
        if ui.button("Solo Here").on_hover_text("Mute every other track in this slot").clicked() {
            action = Some(Action::SoloHere(i, t));
            ui.close();
        }
        if ui.button("Mute in Every Slot").clicked() {
            action = Some(Action::MuteEverywhere(t, true));
            ui.close();
        }
        if ui.button("Unmute in Every Slot").clicked() {
            action = Some(Action::MuteEverywhere(t, false));
            ui.close();
        }
    });
    action
}

/// While slot `from` is dragged: a line where it would land among `rows`,
/// and on letting go, the move there.
fn drop_slot(ui: &mut egui::Ui, rows: &[Rect], from: usize) -> Option<Action> {
    let (y, released) = ui.input(|i| (i.pointer.interact_pos().map(|p| p.y), !i.pointer.any_down()));
    // The slot goes before the first row whose middle is below the pointer.
    let before = y.map_or(from, |y| rows.iter().position(|r| r.center().y > y).unwrap_or(rows.len()));
    if released {
        let to = if before > from { before - 1 } else { before };
        return Some(Action::Move(from, to));
    }
    if before != from && before != from + 1 {
        let at = rows.get(before).map_or_else(|| rows.last().map_or(0.0, |r| r.bottom()), |r| r.top());
        let x = rows.first().map_or(ui.max_rect().x_range(), |r| r.x_range());
        ui.painter().hline(x, at, Stroke::new(2.0, theme::SELECTED));
    }
    None
}

/// A slot's right-click menu: its pattern, the buttons' slot edits, its
/// section and its mutes.
fn slot_menu(ui: &mut egui::Ui, i: usize, slots: usize, has_section: bool, action: &mut Option<Action>) {
    let mut item = |ui: &mut egui::Ui, on: bool, text: &str, a: Action| {
        if ui.add_enabled(on, egui::Button::new(text)).clicked() {
            *action = Some(a);
            ui.close();
        }
    };
    item(ui, true, "Rename Pattern", Action::Edit(i, Field::Name));
    item(ui, true, "Type Pattern Number", Action::Edit(i, Field::Number));
    ui.separator();
    for (op, _, text, _) in SlotOp::ALL {
        item(ui, op.allowed(i, slots), text, Action::Slot(i, op));
    }
    ui.separator();
    if has_section {
        item(ui, true, "Rename Section", Action::RenameSection(i));
        item(ui, true, "Remove Section", Action::RemoveSection(i));
    } else {
        item(ui, true, "Add Section Here", Action::AddSection(i));
    }
    ui.separator();
    item(ui, true, "Mute Every Track Here", Action::MuteSlot(i, true));
    item(ui, true, "Unmute Every Track Here", Action::MuteSlot(i, false));
}

/// The buttons that add, copy, move and remove slots, in a column down
/// the left of the sequencer.
fn buttons(app: &mut App, ui: &mut egui::Ui) {
    let add = |ui: &mut egui::Ui, icon: Icon, on: bool, tip: &str| {
        let button = ui.add_enabled_ui(on, |ui| super::icons::button(ui, icon)).inner;
        button.on_hover_text(tip).clicked()
    };
    for (op, icon, _, tip) in SlotOp::ALL {
        if op == SlotOp::Up {
            ui.add_space(6.0);
        }
        if add(ui, icon, op.allowed(app.slot, app.project.order.len()), tip) {
            slot_op(app, op);
        }
    }
    ui.add_space(6.0);
    let (icon, tip) = if app.show_matrix {
        (Icon::Left, "Show less: just the sections and the pattern numbers")
    } else {
        (Icon::Right, "Show more: the pattern names, and the matrix with each track's color and name")
    };
    if add(ui, icon, true, tip) {
        app.show_matrix = !app.show_matrix;
    }
}

/// The header of the section starting at slot `i`, if one does: its name
/// on a bar. Click it to select the
/// section's slots in the matrix, double-click it to rename it.
fn section_header(app: &mut App, ui: &mut egui::Ui, i: usize) -> Option<Action> {
    let name = app.project.section_at(i)?.name.clone();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 16.0), Sense::click());
    if let Some((slot, Field::Section, text)) = &mut app.sequencer.renaming
        && *slot == i
    {
        if let Some(keep) = edit_text(ui, rect, text) {
            let name = text.trim().to_string();
            app.sequencer.renaming = None;
            if keep && !name.is_empty() {
                app.project.set_section(i, name);
                app.mark_layout();
            }
        }
        return None;
    }
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, theme::FRAME_LINE);
    painter.rect_filled(Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())), 1.0, theme::SELECTED);
    painter.text(
        rect.left_center() + Vec2::new(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        &name,
        egui::FontId::proportional(11.5),
        theme::SELECTED,
    );
    let mut action = None;
    let resp = resp.on_hover_text("Click to select the section, double-click to rename, right-click for more");
    if resp.double_clicked() {
        action = Some(Action::RenameSection(i));
    } else if resp.clicked() {
        action = Some(Action::SelectSection(i));
    }
    resp.context_menu(|ui| {
        if ui.button("Rename Section").clicked() {
            action = Some(Action::RenameSection(i));
            ui.close();
        }
        if ui.button("Remove Section").clicked() {
            action = Some(Action::RemoveSection(i));
            ui.close();
        }
    });
    action
}
