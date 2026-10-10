//! The tracker pattern editor.

use super::block::{self, Block};
use super::{App, View, theme};
use crate::project::{
    MAX_COLUMNS, MAX_FX_COLUMNS, MAX_LINES, MAX_TRACKS, ModuleKind, Note, Pattern, Project, SampleSlot,
};
use eframe::egui::{self, Align2, Color32, FontId, Key, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

/// Fields of a cell the cursor moves across: note, module (2 hex digits),
/// volume (2), panning (2), delay (2) and effect (3). Panning and delay
/// only show on tracks that have them turned on.
const COLUMNS: usize = 12;
const PAN_COL: usize = 5;
const DELAY_COL: usize = 7;

/// Where each field starts in a cell's text, `None` where hidden, and how
/// wide the text is.
#[derive(Clone, Copy)]
struct SubColumns {
    chars: [Option<usize>; COLUMNS],
    width: usize,
}

impl SubColumns {
    fn new(pan: bool, delay: bool) -> Self {
        let mut chars = [None; COLUMNS];
        chars[0] = Some(0);
        let mut x = 4;
        for (col, shown) in [(1, true), (3, true), (PAN_COL, pan), (DELAY_COL, delay)] {
            if shown {
                chars[col] = Some(x);
                chars[col + 1] = Some(x + 1);
                x += 3;
            }
        }
        for i in 0..3 {
            chars[9 + i] = Some(x + i);
        }
        SubColumns { chars, width: x + 3 }
    }

    fn of(project: &Project, track: usize) -> Self {
        let t = &project.tracks[track];
        Self::new(t.show_pan, t.show_delay)
    }

    /// An effect column: only the effect's three characters.
    fn effect() -> Self {
        let mut chars = [None; COLUMNS];
        for i in 0..3 {
            chars[9 + i] = Some(i);
        }
        SubColumns { chars, width: 3 }
    }

    /// The fields of column `column` of `track`.
    fn of_column(project: &Project, pattern: &Pattern, track: usize, column: usize) -> Self {
        if pattern.is_fx_column(track, column) { Self::effect() } else { Self::of(project, track) }
    }

    fn first(&self) -> usize {
        self.next(0).filter(|_| !self.shown(0)).unwrap_or(0)
    }

    fn shown(&self, col: usize) -> bool {
        self.chars.get(col).is_some_and(Option::is_some)
    }

    fn prev(&self, col: usize) -> Option<usize> {
        (0..col).rev().find(|&c| self.shown(c))
    }

    fn next(&self, col: usize) -> Option<usize> {
        (col + 1..COLUMNS).find(|&c| self.shown(c))
    }

    /// The field at character `ch` of the cell's text.
    fn at(&self, ch: usize) -> usize {
        (0..COLUMNS).filter(|&c| self.chars[c].is_some_and(|x| x <= ch)).max_by_key(|&c| self.chars[c]).unwrap_or(0)
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
pub struct Cursor {
    pub line: usize,
    pub track: usize,
    /// The field inside the cell, from the note to the effect.
    pub col: usize,
    /// The track's note column.
    pub column: usize,
}

/// The lane the cursor is in.
fn lane(app: &App) -> usize {
    app.pattern().lane_of(app.cursor.track, app.cursor.column)
}

/// The first and last lane of `track`.
fn track_lanes(app: &App, track: usize) -> (usize, usize) {
    let p = app.pattern();
    (p.lane_of(track, 0), p.lane_of(track, p.width(track) - 1))
}

/// The effect command a letter key past F writes, as Nxy or Zxx.
pub(super) fn fx_key(key: Key) -> Option<u8> {
    crate::project::fx_letter(key.name().chars().next()?)
}

/// The volume column command a letter key writes.
pub(super) fn vol_key(key: Key) -> Option<char> {
    let c = key.name().chars().next()?;
    (key.name().len() == 1 && crate::project::VOL_COMMANDS.contains(&c)).then_some(c)
}

/// The volume column with command `letter` typed: its digit kept if it
/// held a command already.
pub(super) fn vol_command_typed(vol: Option<u8>, letter: char) -> u8 {
    let x = vol.and_then(crate::project::vol_command).map_or(0, |c| c.1);
    crate::project::vol_command_value(letter, x).unwrap_or(0x80)
}

/// The volume column with hex digit `d` typed, high or low: a volume up
/// to 80, or a command's digit.
pub(super) fn type_vol(vol: Option<u8>, hi: bool, d: u8) -> Option<u8> {
    match vol.and_then(crate::project::vol_command) {
        Some(_) if !hi => set_nibble(vol, false, d),
        _ => set_nibble(vol.filter(|&v| v <= 0x80), hi, d).map(|v| v.min(0x80)),
    }
}

/// The color the volume column shows `vol` in: commands as effects.
pub(super) fn vol_color(vol: Option<u8>) -> Color32 {
    match vol {
        None => theme::PAT_EMPTY,
        Some(v) if crate::project::vol_command(v).is_some() => theme::PAT_EFFECT,
        Some(_) => theme::PAT_VOLUME,
    }
}

pub(super) fn hex_digit(key: Key) -> Option<u8> {
    Some(match key {
        Key::Num0 => 0,
        Key::Num1 => 1,
        Key::Num2 => 2,
        Key::Num3 => 3,
        Key::Num4 => 4,
        Key::Num5 => 5,
        Key::Num6 => 6,
        Key::Num7 => 7,
        Key::Num8 => 8,
        Key::Num9 => 9,
        Key::A => 10,
        Key::B => 11,
        Key::C => 12,
        Key::D => 13,
        Key::E => 14,
        Key::F => 15,
        _ => return None,
    })
}

/// The two-row piano layout; returns semitones above the octave.
pub(super) fn note_offset(key: Key) -> Option<u8> {
    Some(match key {
        Key::Z => 0,
        Key::S => 1,
        Key::X => 2,
        Key::D => 3,
        Key::C => 4,
        Key::V => 5,
        Key::G => 6,
        Key::B => 7,
        Key::H => 8,
        Key::N => 9,
        Key::J => 10,
        Key::M => 11,
        Key::Comma => 12,
        Key::L => 13,
        Key::Period => 14,
        Key::Semicolon => 15,
        Key::Slash => 16,
        Key::Q => 12,
        Key::Num2 => 13,
        Key::W => 14,
        Key::Num3 => 15,
        Key::E => 16,
        Key::R => 17,
        Key::Num5 => 18,
        Key::T => 19,
        Key::Num6 => 20,
        Key::Y => 21,
        Key::Num7 => 22,
        Key::U => 23,
        Key::I => 24,
        Key::Num9 => 25,
        Key::O => 26,
        Key::Num0 => 27,
        Key::P => 28,
        _ => return None,
    })
}

pub(super) fn set_nibble(v: Option<u8>, hi: bool, d: u8) -> Option<u8> {
    let v = v.unwrap_or(0);
    Some(if hi { (v & 0x0F) | (d << 4) } else { (v & 0xF0) | d })
}

/// Edits on the selected block, or on the cell under the cursor when
/// nothing is selected.
#[derive(Clone, Copy, PartialEq)]
pub enum Op {
    Cut,
    Copy,
    Paste,
    MixPaste,
    Delete,
    Transpose(i32),
    Interpolate,
    Humanize,
    SetModule,
    Expand,
    Shrink,
    SelectTrack,
    SelectAll,
    RenderToSample,
}

/// The operations as the Edit menu and the right-click menu list them,
/// with `None` between groups.
const MENU: [Option<(Op, &str, &str)>; 20] = [
    Some((Op::Cut, "Cut", "Ctrl+X")),
    Some((Op::Copy, "Copy", "Ctrl+C")),
    Some((Op::Paste, "Paste", "Ctrl+V")),
    Some((Op::MixPaste, "Mix Paste", "Ctrl+Shift+V")),
    Some((Op::Delete, "Delete", "Del")),
    None,
    Some((Op::Transpose(1), "Transpose +1", "Ctrl+F2")),
    Some((Op::Transpose(-1), "Transpose −1", "Ctrl+F1")),
    Some((Op::Transpose(12), "Transpose +12", "Ctrl+F12")),
    Some((Op::Transpose(-12), "Transpose −12", "Ctrl+F11")),
    Some((Op::Interpolate, "Interpolate", "Ctrl+I")),
    Some((Op::Humanize, "Humanize", "")),
    Some((Op::SetModule, "Use Selected Instrument", "")),
    Some((Op::Expand, "Expand", "")),
    Some((Op::Shrink, "Shrink", "")),
    None,
    Some((Op::SelectTrack, "Select Track", "Ctrl+A")),
    Some((Op::SelectAll, "Select All", "Ctrl+A ×2")),
    None,
    Some((Op::RenderToSample, "Render to Sample", "")),
];

/// Lists the block operations in a menu.
pub fn edit_menu(app: &mut App, ui: &mut egui::Ui) {
    for item in MENU {
        match item {
            None => {
                ui.separator();
            }
            Some((op, label, keys)) => {
                if ui.add(egui::Button::new(label).shortcut_text(keys)).clicked() {
                    apply(app, op);
                    ui.close();
                }
            }
        }
    }
}

/// The selection, if it belongs to the pattern being edited.
pub fn selection(app: &App) -> Option<Block> {
    let (pattern, b) = app.selection?;
    (pattern == app.current_pattern_index()).then(|| b.within(app.pattern())).flatten()
}

fn select(app: &mut App, b: Block) {
    app.selection = Some((app.current_pattern_index(), b));
}

fn deselect(app: &mut App) {
    app.selection = None;
    app.anchor = None;
}

/// What block operations act on.
fn target(app: &App) -> Block {
    let at = (app.cursor.line, lane(app));
    selection(app).unwrap_or(Block::between(at, at))
}

pub fn apply(app: &mut App, op: Op) {
    let b = target(app);
    let size = |b: Block| format!("{} lines × {} columns", b.len(), b.width());
    match op {
        Op::Copy | Op::Cut => {
            let clip = block::copy(app.pattern(), b);
            // Also as text, which other windows can paste and which makes
            // Ctrl+V reach the editor.
            app.ctx.copy_text(block::to_text(&clip));
            app.clip = Some(clip);
            if op == Op::Cut {
                block::clear(app.pattern_mut(), b);
                app.mark();
                app.set_status(format!("Cut {}", size(b)));
            } else {
                app.set_status(format!("Copied {}", size(b)));
            }
        }
        Op::Paste | Op::MixPaste => {
            if let Some(clip) = app.clip.clone() {
                paste(app, &clip, op == Op::MixPaste);
            }
        }
        Op::Delete => {
            block::clear(app.pattern_mut(), b);
            app.mark();
        }
        Op::Transpose(semis) => {
            block::transpose(app.pattern_mut(), b, semis);
            app.mark();
        }
        Op::Interpolate => {
            if block::interpolate(app.pattern_mut(), b) {
                app.mark();
            } else {
                app.set_status("Interpolate needs volumes or effects on the first and last line of a selection");
            }
        }
        Op::Humanize => {
            let delays: Vec<bool> = app.project.tracks.iter().map(|t| t.show_delay).collect();
            let seed =
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos());
            block::humanize(app.pattern_mut(), b, 0.1, &delays, seed);
            app.mark();
            app.set_status(format!(
                "Humanized {}: volumes moved up to 10%, and delays where the column is shown",
                size(b)
            ));
        }
        Op::SetModule => match app.instrument() {
            Some(id) => {
                block::set_module(app.pattern_mut(), b, id);
                app.mark();
            }
            None => app.set_status("No instrument selected"),
        },
        Op::Expand => {
            block::expand(app.pattern_mut(), b);
            app.mark();
        }
        Op::Shrink => {
            block::shrink(app.pattern_mut(), b);
            app.mark();
        }
        Op::SelectTrack => {
            let lines = app.pattern().lines;
            let lanes = track_lanes(app, app.cursor.track);
            select(app, Block { lines: (0, lines - 1), tracks: lanes });
        }
        Op::SelectAll => {
            let (lines, lanes) = (app.pattern().lines, app.pattern().num_lanes());
            select(app, Block { lines: (0, lines - 1), tracks: (0, lanes - 1) });
        }
        Op::RenderToSample => render_to_sample(app, b),
    }
}

/// Render Selection to Sample: plays the block offline, with its
/// tail, and loads it into a new Sampler.
fn render_to_sample(app: &mut App, b: Block) {
    let sr = app.audio.as_ref().map_or(44100, |a| a.sample_rate);
    let song = std::sync::Arc::new(app.project.excerpt(app.slot, b.lines, b.tracks));
    let name = format!("Render {:02} {:03}-{:03}", app.current_pattern_index(), b.lines.0, b.lines.1);
    super::jobs::spawn(app, "Rendering the selection", move |progress| {
        let frames = crate::audio::render_with(song, sr, 4.0, true, &mut |f| progress.set(f));
        Box::new(move |app: &mut App| match frames {
            Some(frames) => rendered(app, name, sr, frames),
            None => app.set_status("Render cancelled"),
        })
    });
}

/// Loads `frames`, rendered at `sr`, into a new Sampler called `name`.
fn rendered(app: &mut App, name: String, sr: u32, frames: Vec<crate::dsp::Frame>) {
    if frames.iter().all(|f| f[0].abs().max(f[1].abs()) < 1e-4) {
        app.set_status("Nothing to render: the selection makes no sound");
        return;
    }
    let secs = frames.len() as f32 / sr as f32;
    let sample = crate::sample::Sample { name: name.clone(), sample_rate: sr as f32, channels: 2, frames };
    let before = app.project.modules.len();
    super::instruments::add(app, ModuleKind::Sampler);
    if app.project.modules.len() == before {
        return;
    }
    let Some(id) = app.instrument() else { return };
    let mut slot = SampleSlot::new(sample, None);
    // Written next to the song when it is saved, like edited samples.
    slot.unsaved = true;
    if let Some(m) = app.project.module_mut(id) {
        m.name = name;
        m.samples.push(slot);
    }
    app.mark();
    app.set_status(format!("Rendered {secs:.2} s into instrument {id:02X}"));
}

/// Pastes at the cursor and selects what was pasted.
fn paste(app: &mut App, clip: &block::Clip, mix: bool) {
    let (line, lane) = (app.cursor.line, lane(app));
    block::paste(app.pattern_mut(), clip, line, lane, mix);
    let len = clip.first().map_or(1, Vec::len);
    select(app, Block::between((line, lane), (line + len - 1, lane + clip.len() - 1)));
    app.mark();
}

pub fn handle_keys(app: &mut App, ctx: &egui::Context) {
    let events = ctx.input(|i| i.events.clone());
    let pattern = app.view == View::Pattern;
    for ev in events {
        let (key, pressed, repeat, modifiers) = match ev {
            egui::Event::Copy if pattern => {
                apply(app, Op::Copy);
                continue;
            }
            egui::Event::Cut if pattern => {
                apply(app, Op::Cut);
                continue;
            }
            egui::Event::Paste(text) if pattern => {
                // Text copied in another window comes first.
                if let Some(clip) = block::from_text(&text).or_else(|| app.clip.clone()) {
                    paste(app, &clip, ctx.input(|i| i.modifiers.shift));
                }
                continue;
            }
            egui::Event::Key { key, pressed, repeat, modifiers, .. } => (key, pressed, repeat, modifiers),
            _ => continue,
        };
        if !pressed {
            app.preview_off(super::Held::Key(key));
            continue;
        }
        if modifiers.command && pattern {
            let op = match key {
                // Once for the track, again for the whole pattern.
                Key::A
                    if selection(app).is_some_and(|b| {
                        b.tracks == track_lanes(app, app.cursor.track) && b.len() == app.pattern().lines
                    }) =>
                {
                    Some(Op::SelectAll)
                }
                Key::A => Some(Op::SelectTrack),
                Key::I => Some(Op::Interpolate),
                Key::F1 => Some(Op::Transpose(-1)),
                Key::F2 => Some(Op::Transpose(1)),
                Key::F11 => Some(Op::Transpose(-12)),
                Key::F12 => Some(Op::Transpose(12)),
                _ => None,
            };
            if let Some(op) = op {
                apply(app, op);
            }
            continue;
        }
        if modifiers.alt && !modifiers.command && pattern {
            alt_key(app, key);
            continue;
        }
        if modifiers.command || modifiers.alt {
            continue;
        }
        if app.view != View::Pattern {
            // The phrase editor takes keys in edit mode; elsewhere the
            // keyboard only plays notes.
            if super::phrase::handle_key(app, key, repeat) {
                continue;
            }
            match key {
                Key::Space if !repeat && modifiers.shift => app.play_from_cursor(),
                Key::Space if !repeat => app.toggle_play(),
                Key::Escape => app.edit_mode = !app.edit_mode,
                Key::Minus => app.octave = app.octave.saturating_sub(1),
                Key::Equals | Key::Plus => app.octave = (app.octave + 1).min(9),
                _ => note_key(app, key, repeat, false),
            }
            continue;
        }
        let lines = app.pattern().lines;
        let tracks = app.pattern().num_tracks();
        let before = (app.cursor.line, lane(app));
        let widths: Vec<usize> = (0..tracks).map(|t| app.pattern().width(t)).collect();
        let subs = |t: usize, column: usize| SubColumns::of_column(&app.project, app.pattern(), t, column);
        let last = |t: usize, column: usize| subs(t, column).prev(COLUMNS).unwrap_or(0);
        let first = |t: usize, column: usize| subs(t, column).first();
        let mut c = app.cursor;
        let mut moved = true;
        match key {
            Key::ArrowUp => c.line = (c.line + lines - 1) % lines,
            Key::ArrowDown => c.line = (c.line + 1) % lines,
            Key::PageUp => c.line = c.line.saturating_sub(16),
            Key::PageDown => c.line = (c.line + 16).min(lines - 1),
            Key::Home => c.line = 0,
            Key::End => c.line = lines - 1,
            Key::ArrowLeft => {
                if let Some(p) = subs(c.track, c.column).prev(c.col) {
                    c.col = p;
                } else if c.column > 0 {
                    c.column -= 1;
                    c.col = last(c.track, c.column);
                } else {
                    c.track = (c.track + tracks - 1) % tracks;
                    c.column = widths[c.track] - 1;
                    c.col = last(c.track, c.column);
                }
            }
            Key::ArrowRight => {
                if let Some(n) = subs(c.track, c.column).next(c.col) {
                    c.col = n;
                } else if c.column + 1 < widths[c.track] {
                    c.column += 1;
                    c.col = first(c.track, c.column);
                } else {
                    c.track = (c.track + 1) % tracks;
                    c.column = 0;
                    c.col = 0;
                }
            }
            Key::Tab if modifiers.shift => {
                c.track = (c.track + tracks - 1) % tracks;
                c.column = 0;
                c.col = 0;
            }
            Key::Tab => {
                c.track = (c.track + 1) % tracks;
                c.column = 0;
                c.col = 0;
            }
            _ => moved = false,
        }
        app.cursor = c;
        if moved {
            // Shift extends the selection from where it started; other
            // moves drop it.
            if modifiers.shift && key != Key::Tab {
                let anchor = *app.anchor.get_or_insert(before);
                select(app, Block::between(anchor, (app.cursor.line, lane(app))));
            } else {
                deselect(app);
            }
            continue;
        }

        match key {
            Key::Space if !repeat && modifiers.shift => app.play_from_cursor(),
            Key::Space if !repeat => app.toggle_play(),
            Key::Escape => app.edit_mode = !app.edit_mode,
            Key::Minus => app.octave = app.octave.saturating_sub(1),
            Key::Equals | Key::Plus => app.octave = (app.octave + 1).min(9),
            Key::Delete if selection(app).is_some() => apply(app, Op::Delete),
            Key::Delete if app.edit_mode => {
                let cur = app.cursor;
                let cell = app.pattern_mut().cell_mut(cur.track, cur.column, cur.line);
                match cur.col {
                    0 => *cell = Default::default(),
                    1 | 2 => cell.module = None,
                    3 | 4 => cell.vol = None,
                    5 | 6 => cell.pan = None,
                    7 | 8 => cell.delay = None,
                    _ => cell.fx = None,
                }
                app.mark();
                advance(app);
            }
            Key::Insert if app.edit_mode => {
                let cur = app.cursor;
                let lines = app.pattern().lines;
                let t = app.pattern_mut().column_mut(cur.track, cur.column);
                t[cur.line..lines].rotate_right(1);
                t[cur.line] = Default::default();
                app.mark();
            }
            Key::Backspace if app.edit_mode && app.cursor.line > 0 && !modifiers.shift => {
                app.cursor.line -= 1;
                let cur = app.cursor;
                *app.pattern_mut().cell_mut(cur.track, cur.column, cur.line) = Default::default();
                app.mark();
            }
            Key::Backspace if app.edit_mode && app.cursor.line > 0 => {
                let cur = app.cursor;
                let lines = app.pattern().lines;
                let t = app.pattern_mut().column_mut(cur.track, cur.column);
                t[cur.line - 1..lines].rotate_left(1);
                t[lines - 1] = Default::default();
                app.cursor.line -= 1;
                app.mark();
            }
            _ if app.cursor.col == 0 => note_key(app, key, repeat, app.edit_mode),
            // A volume column command's letter, in the volume's first digit.
            _ if app.edit_mode && app.cursor.col == 3 && vol_key(key).is_some() => {
                let cur = app.cursor;
                let letter = vol_key(key).unwrap();
                let cell = app.pattern_mut().cell_mut(cur.track, cur.column, cur.line);
                cell.vol = Some(vol_command_typed(cell.vol, letter));
                app.mark();
                advance(app);
            }
            // The commands written with letters past F, in the effect
            // command's place.
            _ if app.edit_mode && app.cursor.col == 9 && fx_key(key).is_some() => {
                let cur = app.cursor;
                let cmd = fx_key(key).unwrap();
                let cell = app.pattern_mut().cell_mut(cur.track, cur.column, cur.line);
                cell.fx = Some((cmd, cell.fx.map_or(0, |f| f.1)));
                app.mark();
                advance(app);
            }
            _ => {
                if let (Some(d), true) = (hex_digit(key), app.edit_mode) {
                    let cur = app.cursor;
                    let cell = app.pattern_mut().cell_mut(cur.track, cur.column, cur.line);
                    match cur.col {
                        1 => cell.module = set_nibble(cell.module, true, d),
                        2 => cell.module = set_nibble(cell.module, false, d),
                        3 => cell.vol = type_vol(cell.vol, true, d),
                        4 => cell.vol = type_vol(cell.vol, false, d),
                        5 => cell.pan = set_nibble(cell.pan, true, d).map(|v| v.min(0x80)),
                        6 => cell.pan = set_nibble(cell.pan, false, d).map(|v| v.min(0x80)),
                        7 => cell.delay = set_nibble(cell.delay, true, d),
                        8 => cell.delay = set_nibble(cell.delay, false, d),
                        9 => cell.fx = Some((d, cell.fx.map_or(0, |f| f.1))),
                        10 => {
                            let (cmd, arg) = cell.fx.unwrap_or((0, 0));
                            cell.fx = Some((cmd, set_nibble(Some(arg), true, d).unwrap()));
                        }
                        _ => {
                            let (cmd, arg) = cell.fx.unwrap_or((0, 0));
                            cell.fx = Some((cmd, set_nibble(Some(arg), false, d).unwrap()));
                        }
                    }
                    app.mark();
                    advance(app);
                }
            }
        }
    }
}

/// Plays the note for `key`, and with `write` enters it in the pattern.
fn note_key(app: &mut App, key: Key, repeat: bool, write: bool) {
    if key == Key::A || key == Key::Num1 {
        if write && !repeat {
            let cur = app.cursor;
            let cell = app.pattern_mut().cell_mut(cur.track, cur.column, cur.line);
            cell.note = Some(Note::Off);
            app.mark();
            advance(app);
        }
        return;
    }
    let Some(off) = note_offset(key) else { return };
    if repeat {
        return;
    }
    let note = (app.octave as u32 * 12 + off as u32).min(119) as u8;
    let vol = (app.entry_vol < 0x80).then_some(app.entry_vol);
    play_note(app, super::Held::Key(key), note, 1.0, vol, write);
}

/// A note from the MIDI keyboard: played, and written in edit mode with
/// its velocity when that is recorded.
pub fn midi_note(app: &mut App, note: u8, velocity: u8) {
    let vol = if app.midi_velocity {
        Some(((velocity as u32 * 0x80 + 63) / 127).min(0x80) as u8)
    } else {
        (app.entry_vol < 0x80).then_some(app.entry_vol)
    };
    let write = app.edit_mode && app.view == View::Pattern;
    play_note(app, super::Held::Midi(note), note, velocity as f32 / 127.0, vol, write);
}

/// Plays `note` while `held`, and with `write` enters it at the cursor
/// with volume `vol`.
fn play_note(app: &mut App, held: super::Held, note: u8, vel: f32, vol: Option<u8>, write: bool) {
    let cur = app.cursor;
    // Without a selected instrument, preview with the module already in the cell.
    let module = app.instrument().or_else(|| app.pattern().cell(cur.track, cur.column, cur.line).module);
    if let Some(m) = module {
        app.preview_on(held, m, note, vel);
    }
    // Recording with the song playing: at the play position, a note held
    // with others in the next free column, as a chord.
    if write && let Some((line, delay)) = live_position(app) {
        let col = free_column(app, cur.track, cur.column);
        let pattern = app.current_pattern_index();
        let cell = app.pattern_mut().cell_mut(cur.track, col, line);
        cell.note = Some(Note::On(note));
        cell.delay = delay;
        if vol.is_some() {
            cell.vol = vol;
        }
        if module.is_some() {
            cell.module = module;
        }
        app.recorded.retain(|r| r.0 != held);
        app.recorded.push((held, pattern, cur.track, col));
        app.mark();
        return;
    }
    if write {
        let cell = app.pattern_mut().cell_mut(cur.track, cur.column, cur.line);
        cell.note = Some(Note::On(note));
        if vol.is_some() {
            cell.vol = vol;
        }
        if module.is_some() {
            cell.module = module;
        }
        app.mark();
        advance(app);
    }
}

/// Where a note recorded now goes while the song plays the pattern shown
/// and the cursor follows it: the nearest multiple of the record quantize,
/// or else the line playing, with how far into it in the delay column if
/// the track shows that. `None` when not playing there.
fn live_position(app: &App) -> Option<(usize, Option<u8>)> {
    let (order, line) = app.play_position();
    if !(app.is_playing() && app.follow && order == app.slot) {
        return None;
    }
    let lines = app.pattern().lines;
    let frac = app.play_line_frac();
    if app.record_quantize > 0 {
        let q = app.record_quantize as f32;
        return Some((((line as f32 + frac) / q).round() as usize * app.record_quantize % lines, None));
    }
    let shows_delay = app.project.tracks.get(app.cursor.track).is_some_and(|t| t.show_delay);
    let delay = shows_delay.then_some((frac * 256.0) as u8).filter(|&d| d > 0);
    Some((line.min(lines - 1), delay))
}

/// The first column of `track`, from `from` on and then round, that no key
/// still held is recording into; with all of them taken, a new one.
fn free_column(app: &mut App, track: usize, from: usize) -> usize {
    let pattern = app.current_pattern_index();
    let busy = |app: &App, c: usize| {
        app.recorded.iter().any(|r| (r.1, r.2, r.3) == (pattern, track, c) && app.held.iter().any(|h| h.0 == r.0))
    };
    let cols = app.pattern().columns(track);
    if let Some(c) = (from..cols).chain(0..from).find(|&c| !busy(app, c)) {
        return c;
    }
    if cols < crate::project::MAX_COLUMNS {
        app.project.set_columns(track, cols + 1);
        return cols;
    }
    from
}

/// A key recorded with the song playing was let go: a note-off where
/// playback is, in the note's column, unless a note is there.
pub fn record_release(app: &mut App, held: super::Held) {
    let Some(i) = app.recorded.iter().position(|r| r.0 == held) else { return };
    let (_, pattern, track, col) = app.recorded.remove(i);
    if !app.record_note_offs || !app.edit_mode || pattern != app.current_pattern_index() {
        return;
    }
    let Some((line, delay)) = live_position(app) else { return };
    let cell = app.pattern_mut().cell_mut(track, col, line);
    if cell.note.is_some() {
        return;
    }
    cell.note = Some(Note::Off);
    cell.delay = delay;
    app.mark();
}

fn advance(app: &mut App) {
    let lines = app.pattern().lines;
    app.cursor.line = (app.cursor.line + app.step) % lines;
}

/// A cell's fields that `subs` shows, each in its column's color, or grey
/// when empty.
fn cell_text(cell: &crate::project::Cell, subs: SubColumns) -> Vec<(String, Color32)> {
    use theme::{PAT_DELAY, PAT_EFFECT, PAT_EMPTY, PAT_INSTRUMENT, PAT_NOTE, PAT_PAN};
    let [note, module, vol, fx] = block::fields(cell);
    let [pan, delay] = block::mixer_fields(cell);
    let all = [
        (0, note, cell.note.is_some(), PAT_NOTE),
        (1, module, cell.module.is_some(), PAT_INSTRUMENT),
        (3, vol, cell.vol.is_some(), vol_color(cell.vol)),
        (PAN_COL, pan, cell.pan.is_some(), PAT_PAN),
        (DELAY_COL, delay, cell.delay.is_some(), PAT_DELAY),
        (9, fx, cell.fx.is_some(), PAT_EFFECT),
    ];
    all.into_iter()
        .filter(|f| subs.shown(f.0))
        .map(|(_, text, filled, color)| (text, if filled { color } else { PAT_EMPTY }))
        .collect()
}

/// Where tracks and their note columns sit across the editor, in points
/// from its left edge.
struct Layout {
    num_w: f32,
    /// Each track's left edge and width.
    tracks: Vec<(f32, f32)>,
    /// Each lane's left edge, track, column, fields and width.
    lanes: Vec<(f32, usize, usize, SubColumns, f32)>,
    width: f32,
}

impl Layout {
    fn new(project: &Project, p: &crate::project::Pattern, char_w: f32) -> Self {
        let num_w = char_w * 4.0;
        let pad = char_w;
        let (mut tracks, mut lanes, mut x) = (Vec::new(), Vec::new(), num_w);
        for t in 0..p.num_tracks() {
            let start = x;
            for c in 0..p.width(t) {
                let subs = SubColumns::of_column(project, p, t, c);
                let w = char_w * (subs.width as f32 + 1.0);
                lanes.push((x, t, c, subs, w));
                x += w;
            }
            x += pad;
            tracks.push((start, x - start));
        }
        Layout { num_w, tracks, lanes, width: x }
    }

    /// The lane under `x`, or the nearest one.
    fn lane_at(&self, x: f32) -> usize {
        self.lanes.iter().rposition(|l| l.0 <= x).unwrap_or(0)
    }
}

pub fn editor(app: &mut App, ui: &mut egui::Ui) {
    header(app, ui);
    ui.add_space(3.0);

    let font = FontId::monospace(13.0);
    let char_w = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
    let row_h = 16.0;

    let playing = app.is_playing();
    let (play_order, play_line) = app.play_position();
    if playing && app.follow && play_order != app.slot {
        app.slot = play_order.min(app.project.order.len() - 1);
        app.clamp_cursor();
    }
    let showing_played = playing && play_order == app.slot;
    if showing_played && app.follow {
        app.cursor.line = play_line.min(app.pattern().lines - 1);
    }

    let lines = app.pattern().lines;
    let lpb = app.project.lpb.max(1) as usize;
    let layout = Layout::new(&app.project, app.pattern(), char_w);
    // A field that was hidden takes the cursor to the first one shown.
    let subs = SubColumns::of_column(&app.project, app.pattern(), app.cursor.track, app.cursor.column);
    if !subs.shown(app.cursor.col) {
        app.cursor.col = subs.first();
    }
    let num_w = layout.num_w;

    let outer = ui.available_rect_before_wrap();
    ui.painter().rect_filled(outer, 2.0, theme::PAT_BG);
    egui::ScrollArea::horizontal().auto_shrink(false).show(ui, |ui| {
        track_headers(app, ui, &layout, row_h + 4.0);
        // The headers may have added or removed columns or tracks: lay the
        // rows out for what the pattern has now.
        let layout = Layout::new(&app.project, app.pattern(), char_w);
        let tracks = app.pattern().num_tracks();
        let width = layout.width;

        // The cursor line stays in the middle and the pattern scrolls
        // underneath it.
        let size = Vec2::new(width, ui.available_height());
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let center_top = (rect.center().y - row_h / 2.0).round();
        let cur = app.cursor;
        // When the cursor moves to a column out of view, the editor scrolls
        // sideways to show all of it.
        let at = (cur.track, cur.column, cur.col);
        if app.shown_cursor != Some(at)
            && let Some(&(lx, _, _, _, w)) = layout.lanes.iter().find(|l| l.1 == cur.track && l.2 == cur.column)
        {
            let x = rect.left() + lx;
            let column = Rect::from_x_y_ranges(x - char_w..=x + w + char_w, center_top..=center_top + row_h);
            ui.scroll_to_rect(column, None);
            app.shown_cursor = Some(at);
            app.scrolling_to_cursor = ui.input(|i| i.time) + 0.4;
        }
        // And the other way round: when the editor is
        // scrolled sideways and the cursor's column leaves the view, the
        // cursor moves to the nearest column still in it.
        let view = ui.clip_rect().x_range();
        if ui.input(|i| i.time) > app.scrolling_to_cursor
            && let Some(&(lx, _, _, _, w)) = layout.lanes.iter().find(|l| l.1 == cur.track && l.2 == cur.column)
        {
            let (left, right) = (rect.left() + lx, rect.left() + lx + w);
            let inside = |l: &&(f32, usize, usize, SubColumns, f32)| {
                rect.left() + l.0 >= view.min && rect.left() + l.0 + l.4 <= view.max
            };
            let to = if left < view.min {
                layout.lanes.iter().find(inside)
            } else if right > view.max {
                layout.lanes.iter().rev().find(inside)
            } else {
                None
            };
            if let Some(&(_, track, column, subs, _)) = to {
                app.cursor = Cursor { track, column, col: subs.first(), ..app.cursor };
                app.shown_cursor = Some((track, column, subs.first()));
            }
        }
        let cur = app.cursor;

        if resp.hovered() {
            app.wheel += ui.input(|i| i.smooth_scroll_delta.y);
            let steps = (app.wheel / row_h).trunc();
            if steps != 0.0 && !(showing_played && app.follow) {
                app.wheel -= steps * row_h;
                let l = app.cursor.line as i64 - steps as i64;
                app.cursor.line = l.clamp(0, lines as i64 - 1) as usize;
            }
        }
        // The cell under a point; left of the tracks, the cursor's track.
        let cell_at = |pos: Pos2| {
            let dl = ((pos.y - center_top) / row_h).floor() as i64;
            let line = (cur.line as i64 + dl).clamp(0, lines as i64 - 1) as usize;
            let x = pos.x - rect.left();
            if x < num_w {
                return Cursor { line, ..cur };
            }
            let (lx, track, column, subs, _) = layout.lanes[layout.lane_at(x)];
            let ch = ((x - lx) / char_w - 0.5).max(0.0) as usize;
            Cursor { line, track, column, col: subs.at(ch) }
        };
        let lane_of = |c: Cursor| layout.lanes.iter().position(|l| l.1 == c.track && l.2 == c.column).unwrap_or(0);
        // Click to move the cursor, Shift+click or drag to select.
        if resp.clicked()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let at = cell_at(pos);
            if ui.input(|i| i.modifiers.shift) {
                let anchor = *app.anchor.get_or_insert((cur.line, lane_of(cur)));
                select(app, Block::between(anchor, (at.line, lane_of(at))));
            } else {
                deselect(app);
            }
            app.cursor = at;
        }
        if resp.drag_started()
            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        {
            let at = cell_at(origin);
            app.cursor = at;
            app.anchor = Some((at.line, lane_of(at)));
        }
        if resp.dragged()
            && let (Some(anchor), Some(pos)) = (app.anchor, resp.interact_pointer_pos())
        {
            let at = cell_at(pos);
            select(app, Block::between(anchor, (at.line, lane_of(at))));
        }
        if resp.secondary_clicked()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            // Right-clicking outside the selection moves the cursor there.
            let at = cell_at(pos);
            if !selection(app).is_some_and(|b| b.contains(at.line, lane_of(at))) {
                deselect(app);
                app.cursor = at;
            }
        }
        resp.context_menu(|ui| edit_menu(app, ui));

        let cur = app.cursor;
        let visible = |y: f32| ((y - center_top) / row_h).floor() as i64 + cur.line as i64;
        let first = visible(rect.top()).max(0) as usize;
        let last = ((visible(rect.bottom()) + 1).max(0) as usize).min(lines);
        let selected = selection(app);
        let tints: Vec<Color32> = (0..tracks).map(|t| track_color(&app.project, t).gamma_multiply(0.07)).collect();
        // Tracks the slot mutes in the pattern matrix are dimmed too.
        let slot = app.project.order[app.slot];
        let audible: Vec<bool> = (0..tracks).map(|t| app.project.track_audible(t) && !slot.is_muted(t)).collect();
        let block_loop = app.block_loop.filter(|b| b.0 == app.slot);
        let pattern = app.pattern();

        for line in first..last {
            let y = center_top + (line as f32 - cur.line as f32) * row_h;
            let row = Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(width, row_h));
            let bg = if showing_played && line == play_line {
                theme::PAT_PLAY_ROW
            } else if line == cur.line {
                if app.edit_mode { theme::PAT_CURSOR_ROW_EDIT } else { theme::PAT_CURSOR_ROW }
            } else if line % (lpb * 4) == 0 {
                theme::PAT_BAR
            } else if line % lpb == 0 {
                theme::PAT_BEAT
            } else {
                theme::PAT_BG
            };
            painter.rect_filled(row, 0.0, bg);
            // Each track is tinted with its color.
            for (&(x, w), tint) in layout.tracks.iter().zip(&tints) {
                let x = rect.left() + x;
                painter.rect_filled(Rect::from_x_y_ranges(x..=x + w - 3.0, y..=y + row_h), 0.0, *tint);
            }
            if let Some(b) = selected
                && (b.lines.0..=b.lines.1).contains(&line)
            {
                let x0 = rect.left() + layout.lanes[b.tracks.0].0;
                let end = layout.lanes[b.tracks.1];
                let x1 = rect.left() + end.0 + end.4 - char_w * 0.5;
                painter.rect_filled(Rect::from_x_y_ranges(x0..=x1, y..=y + row_h), 0.0, theme::PAT_SELECTION);
            }
            // The block loop's lines, marked in the line numbers.
            if let Some((_, from, to)) = block_loop
                && (from..=to).contains(&line)
            {
                let r = Rect::from_x_y_ranges(rect.left()..=rect.left() + num_w - 3.0, y..=y + row_h);
                painter.rect_filled(r, 0.0, theme::SCOPE.gamma_multiply(0.18));
                painter.rect_filled(Rect::from_x_y_ranges(r.left()..=r.left() + 2.0, r.y_range()), 0.0, theme::SCOPE);
            }
            let num_color = if line % lpb == 0 { theme::PAT_NOTE } else { theme::PAT_LINE_NUMBER };
            painter.text(
                Pos2::new(rect.left() + 4.0, y + row_h / 2.0),
                Align2::LEFT_CENTER,
                format!("{line:03}"),
                font.clone(),
                num_color,
            );
            for &(lx, t, c, subs, _) in &layout.lanes {
                let x0 = rect.left() + lx + char_w * 0.5;
                let parts = cell_text(&pattern.cell(t, c, line), subs);
                let mut x = x0;
                for (text, color) in &parts {
                    let color = if audible[t] { *color } else { color.gamma_multiply(0.4) };
                    painter.text(Pos2::new(x, y + row_h / 2.0), Align2::LEFT_CENTER, text, font.clone(), color);
                    x += (text.len() + 1) as f32 * char_w;
                }
                if line == cur.line && t == cur.track && c == cur.column {
                    // An amber block with the characters under it redrawn dark.
                    let full: Vec<char> =
                        parts.iter().map(|p| p.0.as_str()).collect::<Vec<_>>().join(" ").chars().collect();
                    let (c0, w) = (subs.chars[cur.col].unwrap_or(0), if cur.col == 0 { 3 } else { 1 });
                    let r =
                        Rect::from_min_size(Pos2::new(x0 + c0 as f32 * char_w, y), Vec2::new(char_w * w as f32, row_h));
                    painter.rect_filled(r.expand2(Vec2::new(1.0, 0.0)), 1.0, theme::PAT_CURSOR);
                    let text: String = full[c0..c0 + w].iter().collect();
                    painter.text(r.left_center(), Align2::LEFT_CENTER, text, font.clone(), theme::SELECTED_TEXT);
                }
                // A solid line between tracks, a faint one between columns.
                let sep = rect.left() + lx - 2.0;
                let shade = if c == 0 { 40 } else { 28 };
                painter.line_segment([Pos2::new(sep, y), Pos2::new(sep, y + row_h)], (1.0, Color32::from_gray(shade)));
            }
        }
    });
    if app.edit_mode {
        // The pattern editor is outlined in red while editing.
        ui.painter().rect_stroke(outer.shrink(1.0), 2.0, (2.0, theme::RECORD), egui::StrokeKind::Inside);
    }
}

/// A track's color: its own, or the default for its number.
pub fn track_color(project: &Project, track: usize) -> Color32 {
    match project.tracks.get(track).and_then(|t| t.color) {
        Some([r, g, b]) => Color32::from_rgb(r, g, b),
        None => theme::track_color(track),
    }
}

enum TrackAction {
    Rename,
    Mute,
    Solo,
    Color(Option<[u8; 3]>),
    InsertBefore,
    InsertAfter,
    Delete,
    Clear,
    Columns(usize),
    FxColumns(usize),
    ShowPan,
    ShowDelay,
    /// Open its effects in the lower frame's Track FX tab.
    Effects,
    /// Send its sound on through another track's effects, or the master.
    Group(Option<usize>),
}

/// Track headers: the name on the track's color, with a
/// mute light on the left and a solo button on the right.
fn track_headers(app: &mut App, ui: &mut egui::Ui, layout: &Layout, h: f32) {
    let tracks = app.pattern().num_tracks();
    let (all, _) = ui.allocate_exact_size(Vec2::new(layout.width, h + 5.0 + COLUMN_BUTTONS_H), Sense::hover());
    let mut action = None;
    for t in 0..tracks {
        let (x, w) = layout.tracks[t];
        let r = Rect::from_min_size(Pos2::new(all.left() + x, all.top() + 2.0), Vec2::new(w - 3.0, h));
        let info = app.project.tracks[t].clone();
        let audible = app.project.track_audible(t);
        let id = ui.id().with(("track", t));
        let painter = ui.painter().clone();
        painter.rect_filled(r, 2.0, if info.mute { Color32::from_gray(60) } else { track_color(&app.project, t) });

        let light = Rect::from_center_size(Pos2::new(r.left() + 9.0, r.center().y), Vec2::splat(9.0));
        let fill = if audible { theme::SCOPE } else { Color32::from_gray(34) };
        painter.rect(light, 2.0, fill, Stroke::new(1.0, Color32::from_gray(20)), StrokeKind::Outside);
        // The mute button: click to mute, right-click to solo.
        let mute = ui
            .interact(light.expand(3.0), id.with("mute"), Sense::click())
            .on_hover_text("Click to mute, right-click to solo");
        if mute.clicked() {
            action = Some((t, TrackAction::Mute));
        } else if mute.secondary_clicked() {
            action = Some((t, TrackAction::Solo));
        }
        let solo = Rect::from_center_size(Pos2::new(r.right() - 10.0, r.center().y), Vec2::new(14.0, h - 4.0));
        let (solo_fill, solo_text) = if info.solo {
            (Color32::WHITE, theme::SELECTED_TEXT)
        } else {
            (Color32::from_black_alpha(90), Color32::from_gray(200))
        };
        painter.rect_filled(solo, 2.0, solo_fill);
        painter.text(solo.center(), Align2::CENTER_CENTER, "S", FontId::proportional(10.0), solo_text);
        if ui
            .interact(solo, id.with("solo"), Sense::click())
            .on_hover_text("Solo: only this track plays, until it is soloed again")
            .clicked()
        {
            action = Some((t, TrackAction::Solo));
        }

        // A track with effects of its own shows how many beside its solo;
        // a click opens them.
        let mut name_end = solo.left() - 3.0;
        if !info.effects.is_empty() {
            let badge = Rect::from_center_size(Pos2::new(solo.left() - 17.0, r.center().y), Vec2::new(26.0, h - 4.0));
            painter.rect_filled(badge, 2.0, Color32::from_black_alpha(110));
            let text = format!("FX {}", info.effects.len());
            painter.text(
                badge.center(),
                Align2::CENTER_CENTER,
                text,
                FontId::proportional(9.5),
                Color32::from_gray(225),
            );
            let n = info.effects.len();
            let tip = format!("{n} track effect{}: click to edit them", if n == 1 { "" } else { "s" });
            if ui.interact(badge, id.with("fx"), Sense::click()).on_hover_text(tip).clicked() {
                action = Some((t, TrackAction::Effects));
            }
            name_end = badge.left() - 3.0;
        }
        let name = Rect::from_x_y_ranges(light.right() + 5.0..=name_end.max(light.right() + 6.0), r.y_range());
        if let Some((rt, text)) = &mut app.track_rename
            && *rt == t
        {
            let edit = ui.put(
                name,
                egui::TextEdit::singleline(text).font(FontId::proportional(12.0)).margin(Vec2::new(2.0, 0.0)),
            );
            if !edit.has_focus() && !edit.lost_focus() {
                edit.request_focus();
            }
            if edit.lost_focus() {
                if !ui.input(|i| i.key_pressed(Key::Escape)) {
                    app.project.tracks[t].name = text.trim().to_string();
                    app.mark_layout();
                }
                app.track_rename = None;
            }
            continue;
        }
        let text_color = if info.mute { theme::TEXT_WEAK } else { theme::SELECTED_TEXT };
        let label = app.project.track_name(t);
        painter.with_clip_rect(name).text(
            name.left_center(),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(12.0),
            text_color,
        );
        let resp = ui.interact(name, id.with("name"), Sense::click());
        if resp.clicked() {
            app.cursor = Cursor { track: t, col: 0, column: 0, ..app.cursor };
            deselect(app);
        }
        if resp.double_clicked() {
            action = Some((t, TrackAction::Rename));
        }
        let resp = resp.on_hover_text("Double-click to rename, right-click for more");
        let (columns, fx) = (app.pattern().columns(t), app.pattern().fx_columns(t));
        // Below the name: − and + for the track's
        // note columns on the left, and for its effect columns on the right.
        let row = Rect::from_min_size(Pos2::new(r.left(), r.bottom() + 1.0), Vec2::new(r.width(), COLUMN_BUTTONS_H));
        let buttons = [
            (row.left(), "−", "Remove a note column", columns > 1, TrackAction::Columns(columns.saturating_sub(1))),
            (row.left() + 13.0, "+", "Add a note column", columns < MAX_COLUMNS, TrackAction::Columns(columns + 1)),
            (row.right() - 26.0, "−", "Remove an effect column", fx > 0, TrackAction::FxColumns(fx.saturating_sub(1))),
            (row.right() - 13.0, "+", "Add an effect column", fx < MAX_FX_COLUMNS, TrackAction::FxColumns(fx + 1)),
        ];
        let font = FontId::proportional(10.0);
        painter.text(
            Pos2::new(row.left() + 28.0, row.center().y),
            Align2::LEFT_CENTER,
            format!("{columns}"),
            font.clone(),
            theme::TEXT_WEAK,
        );
        painter.text(
            Pos2::new(row.right() - 30.0, row.center().y),
            Align2::RIGHT_CENTER,
            format!("fx {fx}"),
            font.clone(),
            theme::TEXT_WEAK,
        );
        for (k, (bx, label, tip, enabled, a)) in buttons.into_iter().enumerate() {
            let b = Rect::from_min_size(Pos2::new(bx, row.top()), Vec2::new(12.0, COLUMN_BUTTONS_H));
            let resp = ui.interact(b, id.with(("columns", k)), if enabled { Sense::click() } else { Sense::hover() });
            let fill = if resp.hovered() && enabled { theme::BUTTON_HOVER } else { theme::BUTTON };
            painter.rect_filled(b, 2.0, fill);
            let color = if enabled { theme::TEXT } else { theme::TEXT_WEAK.gamma_multiply(0.5) };
            painter.text(b.center(), Align2::CENTER_CENTER, label, font.clone(), color);
            if resp.on_hover_text(tip).clicked() {
                action = Some((t, a));
            }
        }
        // The tracks it can go through, with their names.
        let groups: Vec<(usize, String)> =
            (0..tracks).filter(|&g| app.project.can_group(t, g)).map(|g| (g, app.project.track_name(g))).collect();
        resp.context_menu(|ui| {
            if let Some(a) = track_menu(ui, &info, &groups, columns, fx) {
                action = Some((t, a));
                ui.close();
            }
        });
    }
    let Some((t, action)) = action else { return };
    match action {
        TrackAction::Rename => app.track_rename = Some((t, app.project.tracks[t].name.clone())),
        TrackAction::Mute => app.project.tracks[t].mute ^= true,
        TrackAction::Solo => app.project.solo_track(t),
        TrackAction::Color(c) => app.project.tracks[t].color = c,
        TrackAction::InsertBefore | TrackAction::InsertAfter => {
            let at = if matches!(action, TrackAction::InsertBefore) { t } else { t + 1 };
            let grow = app.current_pattern_index();
            if !app.project.insert_track(at, grow) {
                app.set_status(format!("Patterns can't have more than {MAX_TRACKS} tracks"));
                return;
            }
        }
        TrackAction::Delete => app.project.remove_track(t),
        TrackAction::Clear => app.pattern_mut().clear_track(t),
        TrackAction::ShowPan => app.project.tracks[t].show_pan ^= true,
        TrackAction::ShowDelay => app.project.tracks[t].show_delay ^= true,
        TrackAction::Columns(n) => app.project.set_columns(t, n),
        TrackAction::FxColumns(n) => app.project.set_fx_columns(t, n),
        TrackAction::Effects => {
            app.show_track_fx(t);
            return;
        }
        TrackAction::Group(g) => app.project.tracks[t].group = g.filter(|&g| app.project.can_group(t, g)),
    }
    if !matches!(action, TrackAction::Rename) {
        app.clamp_cursor();
        app.mark();
    }
}

/// The height of the row of column buttons under each track's name.
const COLUMN_BUTTONS_H: f32 = 12.0;

fn track_menu(
    ui: &mut egui::Ui,
    info: &crate::project::Track,
    groups: &[(usize, String)],
    columns: usize,
    fx: usize,
) -> Option<TrackAction> {
    let mut action = None;
    if ui.button("Rename").clicked() {
        action = Some(TrackAction::Rename);
    }
    let n = info.effects.len();
    let label = if n == 0 { "Track Effects…".to_string() } else { format!("Track Effects ({n})…") };
    if ui.button(label).on_hover_text("Effects for what this track plays, in the lower frame").clicked() {
        action = Some(TrackAction::Effects);
    }
    ui.menu_button("Group", |ui| {
        let tip = "Send this track's sound on through another track's effects, as a bus";
        ui.label(egui::RichText::new(tip).small().color(theme::TEXT_WEAK));
        if ui.add(egui::Button::selectable(info.group.is_none(), "None (to the master)")).clicked() {
            action = Some(TrackAction::Group(None));
        }
        for (g, name) in groups {
            if ui.add(egui::Button::selectable(info.group == Some(*g), format!("{:02} {name}", g + 1))).clicked() {
                action = Some(TrackAction::Group(Some(*g)));
            }
        }
    });
    if ui.button(if info.mute { "Unmute" } else { "Mute" }).clicked() {
        action = Some(TrackAction::Mute);
    }
    if ui.button(if info.solo { "Unsolo" } else { "Solo" }).clicked() {
        action = Some(TrackAction::Solo);
    }
    ui.menu_button("Color", |ui| {
        ui.horizontal(|ui| {
            for c in theme::TRACK_COLORS {
                if ui.add(egui::Button::new("").fill(c).min_size(Vec2::splat(18.0))).clicked() {
                    action = Some(TrackAction::Color(Some([c.r(), c.g(), c.b()])));
                }
            }
        });
        if ui.button("Default").clicked() {
            action = Some(TrackAction::Color(None));
        }
    });
    ui.separator();
    if ui.add_enabled(columns < MAX_COLUMNS, egui::Button::new("Add Note Column")).clicked() {
        action = Some(TrackAction::Columns(columns + 1));
    }
    if ui.add_enabled(columns > 1, egui::Button::new("Remove Note Column")).clicked() {
        action = Some(TrackAction::Columns(columns - 1));
    }
    if ui.add_enabled(fx < MAX_FX_COLUMNS, egui::Button::new("Add Effect Column")).clicked() {
        action = Some(TrackAction::FxColumns(fx + 1));
    }
    if ui.add_enabled(fx > 0, egui::Button::new("Remove Effect Column")).clicked() {
        action = Some(TrackAction::FxColumns(fx - 1));
    }
    if ui.add(egui::Button::selectable(info.show_pan, "Panning Column")).clicked() {
        action = Some(TrackAction::ShowPan);
    }
    if ui.add(egui::Button::selectable(info.show_delay, "Delay Column")).clicked() {
        action = Some(TrackAction::ShowDelay);
    }
    ui.separator();
    if ui.button("Insert Track Before").clicked() {
        action = Some(TrackAction::InsertBefore);
    }
    if ui.button("Insert Track After").clicked() {
        action = Some(TrackAction::InsertAfter);
    }
    if ui.button("Delete Track").on_hover_text("In every pattern").clicked() {
        action = Some(TrackAction::Delete);
    }
    if ui.button("Clear Track").on_hover_text("In this pattern").clicked() {
        action = Some(TrackAction::Clear);
    }
    action
}

/// Block loop sizes: a fraction of the pattern, or the selection (0).
const BLOCK_SIZES: [(usize, &str); 6] =
    [(0, "Selection"), (2, "1/2"), (4, "1/4"), (6, "1/6"), (8, "1/8"), (16, "1/16")];

/// The lines a block loop of `app.block_size` covers: the selection, or
/// the stretch of that size around the cursor.
fn block_range(app: &App) -> (usize, usize) {
    let lines = app.pattern().lines;
    match selection(app) {
        Some(b) if app.block_size == 0 => b.lines,
        _ => {
            let len = (lines / app.block_size.max(2)).max(1);
            let from = app.cursor.line / len * len;
            (from, (from + len - 1).min(lines - 1))
        }
    }
}

/// `Alt` keys in the pattern editor: jump between note columns, and turn
/// and move the block loop.
fn alt_key(app: &mut App, key: Key) {
    match key {
        Key::ArrowLeft | Key::ArrowRight => {
            // Every note column in turn, across the tracks.
            let lanes = app.pattern().num_lanes();
            let lane = lane(app);
            let to = if key == Key::ArrowRight { (lane + 1) % lanes } else { (lane + lanes - 1) % lanes };
            let (track, column) = app.pattern().lane_pos(to);
            app.cursor = Cursor { track, column, col: 0, ..app.cursor };
            deselect(app);
        }
        Key::L => set_block_loop(app, app.block_loop.is_none()),
        Key::ArrowUp | Key::ArrowDown => move_block_loop(app, key == Key::ArrowDown),
        _ => {}
    }
}

/// Moves the block loop by its own length, staying in the pattern; turns
/// it on where the cursor is if it was off.
fn move_block_loop(app: &mut App, down: bool) {
    let Some((slot, from, to)) = app.block_loop.filter(|b| b.0 == app.slot) else {
        set_block_loop(app, true);
        return;
    };
    let (len, lines) = (to - from + 1, app.pattern().lines);
    let from = if down { (from + len).min(lines.saturating_sub(len)) } else { from.saturating_sub(len) };
    app.block_loop = Some((slot, from, (from + len - 1).min(lines - 1)));
    app.send(crate::engine::Cmd::BlockLoop(app.block_loop));
}

/// Turns the block loop on (at the cursor or selection) or off, and tells
/// the engine.
pub fn set_block_loop(app: &mut App, on: bool) {
    app.block_loop = on.then(|| {
        let (from, to) = block_range(app);
        (app.slot, from, to)
    });
    app.send(crate::engine::Cmd::BlockLoop(app.block_loop));
}

fn header(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        let idx = app.current_pattern_index();
        theme::caption(ui, &format!("PATTERN {idx:02}"));
        let mut name = app.pattern().name.clone();
        let edit = egui::TextEdit::singleline(&mut name).desired_width(140.0).hint_text("Name (optional)");
        if ui.add(edit).on_hover_text("A name for the pattern, shown in the extended sequencer").changed() {
            app.pattern_mut().name = name;
            app.mark_layout();
        }
        theme::caption(ui, "LINES");
        let mut lines = app.pattern().lines;
        if ui.add(egui::DragValue::new(&mut lines).range(1..=MAX_LINES)).changed() {
            app.pattern_mut().lines = lines;
            app.clamp_cursor();
            app.mark();
        }
        theme::caption(ui, "TRACKS");
        if ui.small_button("−").clicked() && app.pattern().num_tracks() > 1 {
            let last = app.pattern().num_tracks() - 1;
            app.pattern_mut().remove_track(last);
            app.clamp_cursor();
            app.mark();
        }
        ui.label(app.pattern().num_tracks().to_string());
        if ui.small_button("+").clicked() && app.pattern().num_tracks() < MAX_TRACKS {
            let n = app.pattern().num_tracks();
            app.pattern_mut().insert_track(n);
            let index = app.current_pattern_index();
            app.project.sync_columns(index);
            app.mark();
        }
        // The block loop, beside the pattern's size.
        let on = app.block_loop.is_some();
        let tip = "Block loop: repeat these lines while playing (the size around the cursor, or the selection)";
        if theme::toggle(ui, on, "LOOP").on_hover_text(tip).clicked() {
            set_block_loop(app, !on);
        }
        let size = BLOCK_SIZES.iter().find(|s| s.0 == app.block_size).map_or("1/4", |s| s.1);
        let mut pick = app.block_size;
        egui::ComboBox::from_id_salt("block_size").selected_text(size).width(70.0).show_ui(ui, |ui| {
            for (n, label) in BLOCK_SIZES {
                ui.selectable_value(&mut pick, n, label);
            }
        });
        if pick != app.block_size {
            app.block_size = pick;
            if on {
                set_block_loop(app, true);
            }
        }
        // The note columns of the cursor's track, above the pattern
        // editor.
        theme::caption(ui, "COLUMNS");
        let (t, columns) = (app.cursor.track, app.pattern().columns(app.cursor.track));
        let hint = "Note columns of the track under the cursor";
        if ui.small_button("−").on_hover_text(hint).clicked() && columns > 1 {
            app.project.set_columns(t, columns - 1);
            app.clamp_cursor();
            app.mark();
        }
        ui.label(columns.to_string()).on_hover_text(hint);
        if ui.small_button("+").on_hover_text(hint).clicked() && columns < MAX_COLUMNS {
            app.project.set_columns(t, columns + 1);
            app.mark();
        }
        theme::caption(ui, "FX");
        let fx = app.pattern().fx_columns(t);
        let hint = "Effect columns of the track under the cursor: their commands act on all its notes";
        if ui.small_button("−").on_hover_text(hint).clicked() && fx > 0 {
            app.project.set_fx_columns(t, fx - 1);
            app.clamp_cursor();
            app.mark();
        }
        ui.label(fx.to_string()).on_hover_text(hint);
        if ui.small_button("+").on_hover_text(hint).clicked() && fx < MAX_FX_COLUMNS {
            app.project.set_fx_columns(t, fx + 1);
            app.mark();
        }
        // The track's switches for the panning and delay columns.
        let info = &app.project.tracks[t];
        let (pan, delay) = (info.show_pan, info.show_delay);
        if theme::toggle(ui, pan, "PAN")
            .on_hover_text("Show the panning column of the track under the cursor")
            .clicked()
        {
            app.project.tracks[t].show_pan = !pan;
            app.mark_layout();
        }
        if theme::toggle(ui, delay, "DLY")
            .on_hover_text("Show the delay column of the track under the cursor")
            .clicked()
        {
            app.project.tracks[t].show_delay = !delay;
            app.mark_layout();
        }
        ui.separator();
        match app.instrument().and_then(|id| app.project.module(id)) {
            Some(m) => {
                let text = format!("Instrument {:02X}  {}", m.id, m.name);
                ui.label(egui::RichText::new(text).color(theme::PAT_INSTRUMENT))
            }
            None => ui.label(egui::RichText::new("No instrument selected").color(theme::TEXT_WEAK)),
        };
    });
}
