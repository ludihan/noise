//! The Phrase page of the instrument editor: a
//! short pattern the instrument plays when it gets a note, transposed so
//! that its C-4 is the note played, with effect commands as in the
//! pattern. Edited with the keyboard in edit mode, like the pattern.

use super::{App, pattern, theme};
use crate::project::{Cell, FX_BREAK, FX_PHRASE, FX_WAIT, MAX_PHRASE_LINES, MAX_PHRASES, Note, Phrase, PhraseMode};
use eframe::egui::{self, Align2, FontId, Key, Pos2, Rect, RichText, Sense, Vec2};

/// Where the phrase editor's cursor is: the line, and the note (0), a
/// digit of the volume (1, 2) or of the effect (3 to 5).
#[derive(Default)]
pub struct PhraseView {
    line: usize,
    col: usize,
}

/// Whether the Phrase page is showing, so the keyboard edits it.
pub fn shown(app: &App) -> bool {
    use super::sampler::{SynthTab, Tab};
    let Some(m) = app.instrument().and_then(|id| app.project.module(id)) else { return false };
    app.view == super::View::Sampler
        && if m.kind == crate::project::ModuleKind::Sampler {
            app.sampler.tab == Tab::Phrase
        } else {
            app.sampler.synth_tab == SynthTab::Phrase
        }
}

pub fn page(app: &mut App, ui: &mut egui::Ui, id: u8) {
    let Some(m) = app.project.module(id).cloned() else { return };
    let mut edited = m.clone();
    let playing: Vec<(usize, usize)> =
        app.playing_phrases().into_iter().filter(|p| p.0 == id).map(|p| (p.1, p.2)).collect();
    ui.horizontal(|ui| {
        theme::caption(ui, "PHRASES");
        for k in 0..m.phrases.len() {
            let tip = "Show this phrase; in Program mode, every note plays it. A dot means it is playing.";
            let resp = theme::toggle(ui, k == m.selected_phrase, format!("{:02X}", k + 1)).on_hover_text(tip);
            if playing.iter().any(|p| p.0 == k) {
                let at = egui::pos2(resp.rect.center().x, resp.rect.bottom() - 2.5);
                ui.painter().circle_filled(at, 1.5, theme::SCOPE);
            }
            if resp.clicked() {
                edited.selected_phrase = k;
            }
        }
        if m.phrases.len() < MAX_PHRASES && ui.button("+").on_hover_text("Add a phrase").clicked() {
            edited.phrases.push(Phrase::default());
            edited.selected_phrase = edited.phrases.len() - 1;
        }
        if let Some(phrase) = m.phrase().filter(|_| m.phrases.len() < MAX_PHRASES)
            && ui.button("Dup").on_hover_text("Copy the shown phrase").clicked()
        {
            edited.phrases.insert(m.selected_phrase + 1, phrase.clone());
            edited.selected_phrase += 1;
        }
        if m.phrase().is_some() && ui.button("−").on_hover_text("Delete the shown phrase").clicked() {
            edited.phrases.remove(m.selected_phrase);
            edited.selected_phrase = m.selected_phrase.min(edited.phrases.len().saturating_sub(1));
        }
        ui.separator();
        theme::caption(ui, "PLAY");
        for mode in PhraseMode::ALL {
            let tip = match mode {
                PhraseMode::Off => "Notes play the instrument itself, unless a Zxx picks a phrase",
                PhraseMode::Program => "Every note plays the shown phrase, transposed",
                PhraseMode::Keymap => {
                    "A note plays the phrase whose keys hold it, or the instrument itself outside them"
                }
            };
            if theme::toggle(ui, m.phrase_mode == mode, mode.name()).on_hover_text(tip).clicked() {
                edited.phrase_mode = mode;
            }
        }
    });
    let keymap = m.phrase_mode == PhraseMode::Keymap;
    if let Some(phrase) = edited.phrase_mut() {
        ui.horizontal(|ui| {
            theme::caption(ui, "LINES");
            ui.add(egui::DragValue::new(&mut phrase.lines).range(1..=MAX_PHRASE_LINES));
            theme::caption(ui, "LPB");
            ui.add(egui::DragValue::new(&mut phrase.lpb).range(1..=32)).on_hover_text("Lines per beat of the phrase, at the song's BPM");
            if theme::toggle(ui, phrase.looping, "Loop").on_hover_text("Start again after the last line while the note is held").clicked() {
                phrase.looping = !phrase.looping;
            }
            if keymap {
                theme::caption(ui, "KEYS");
                let [low, high] = &mut phrase.keys;
                ui.add(key_field(low, 0, *high)).on_hover_text("The lowest note that plays this phrase");
                ui.add(key_field(high, *low, 119)).on_hover_text("The highest note that plays this phrase");
            }
            if ui.button("Clear").on_hover_text("Empty every line").clicked() {
                phrase.cells.fill(Cell::default());
            }
            let hint = "Esc for edit mode, then type notes (C-4 is the note played), hex volumes and effect commands, as in the pattern. A or 1 is a note-off, Del clears. Zxx in the pattern picks phrase xx for a note.";
            ui.add(egui::Label::new(RichText::new(hint).small().color(theme::TEXT_WEAK)).truncate()).on_hover_text(hint);
        });
    }
    if (&edited.phrases, edited.phrase_mode, edited.selected_phrase) != (&m.phrases, m.phrase_mode, m.selected_phrase) {
        let module = app.project.module_mut(id).unwrap();
        module.phrases = edited.phrases;
        module.phrase_mode = edited.phrase_mode;
        module.selected_phrase = edited.selected_phrase;
        app.mark();
    }
    let selected = app.project.module(id).map_or(0, |m| m.selected_phrase);
    let Some(phrase) = app.project.module(id).and_then(|m| m.phrase()).cloned() else {
        ui.label(RichText::new("No phrases. + adds one.").color(theme::TEXT_WEAK));
        return;
    };

    let font = FontId::monospace(13.0);
    let char_w = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
    let row_h = 16.0;
    let lines = phrase.lines;
    app.phrase.line = app.phrase.line.min(lines - 1);
    let (outer, resp) = ui.allocate_exact_size(Vec2::new(char_w * 15.5, ui.available_height()), Sense::click());
    let painter = ui.painter_at(outer);
    painter.rect_filled(outer, 2.0, theme::PAT_BG);
    if app.edit_mode {
        painter.rect_stroke(outer.shrink(1.0), 2.0, (2.0, theme::RECORD), egui::StrokeKind::Inside);
    }
    // The cursor line stays in the middle, as in the pattern editor.
    let center = (outer.center().y - row_h / 2.0).round();
    let cur = app.phrase.line;
    if let Some(p) = resp.interact_pointer_pos().filter(|_| resp.clicked()) {
        let line = cur as i64 + ((p.y - center) / row_h).floor() as i64;
        app.phrase.line = line.clamp(0, lines as i64 - 1) as usize;
        let ch = ((p.x - outer.left()) / char_w) as usize;
        app.phrase.col = match ch {
            0..=7 => 0,
            8 => 1,
            9 => 2,
            10..=11 => 3,
            12 => 4,
            _ => 5,
        };
    }
    let cur = app.phrase.line;
    for line in 0..lines {
        let y = center + (line as f32 - cur as f32) * row_h;
        if y + row_h < outer.top() || y > outer.bottom() {
            continue;
        }
        let row = Rect::from_min_size(Pos2::new(outer.left(), y), Vec2::new(outer.width(), row_h));
        let lpb = phrase.lpb.max(1) as usize;
        // The lines playing now, one for each note playing this phrase.
        let bg = if playing.contains(&(selected, line)) {
            theme::PAT_PLAY_ROW
        } else if line == cur {
            if app.edit_mode { theme::PAT_CURSOR_ROW_EDIT } else { theme::PAT_CURSOR_ROW }
        } else if line % lpb == 0 {
            theme::PAT_BEAT
        } else {
            theme::PAT_BG
        };
        painter.rect_filled(row, 0.0, bg);
        let cell = phrase.cells[line];
        let mid = y + row_h / 2.0;
        let text = |x: f32, s: String, c| {
            painter.text(Pos2::new(outer.left() + x * char_w, mid), Align2::LEFT_CENTER, s, font.clone(), c);
        };
        text(0.5, format!("{line:02}"), if line % lpb == 0 { theme::PAT_NOTE } else { theme::PAT_LINE_NUMBER });
        let note = cell.note.map_or("---".into(), |n| n.label());
        text(4.0, note, if cell.note.is_some() { theme::PAT_NOTE } else { theme::PAT_EMPTY });
        let vol = cell.vol.map_or("..".into(), crate::project::vol_text);
        text(8.0, vol, pattern::vol_color(cell.vol));
        let fx = cell.fx.map_or("...".into(), |(c, a)| format!("{}{a:02X}", super::block::fx_command(c)));
        text(11.0, fx, if cell.fx.is_some() { theme::PAT_EFFECT } else { theme::PAT_EMPTY });
        if line == cur {
            let (x, w) = match app.phrase.col {
                0 => (4.0, 3.0),
                c @ 1..=2 => (7.0 + c as f32, 1.0),
                c => (8.0 + c as f32, 1.0),
            };
            let r = Rect::from_min_size(Pos2::new(outer.left() + x * char_w, y), Vec2::new(w * char_w, row_h));
            painter.rect_stroke(r.expand(1.0), 1.0, (1.0, theme::PAT_CURSOR), egui::StrokeKind::Outside);
        }
    }
}

/// Edits the phrase with key `key`, when the Phrase page shows and edit
/// mode is on. Returns whether the key was used.
pub fn handle_key(app: &mut App, key: Key, repeat: bool) -> bool {
    if !shown(app) || !app.edit_mode {
        return false;
    }
    let Some(id) = app.instrument() else { return false };
    let Some(lines) = app.project.module(id).and_then(|m| m.phrase()).map(|p| p.lines) else { return false };
    let cur = &mut app.phrase;
    match key {
        Key::ArrowUp => cur.line = (cur.line + lines - 1) % lines,
        Key::ArrowDown => cur.line = (cur.line + 1) % lines,
        Key::ArrowLeft => cur.col = cur.col.saturating_sub(1),
        Key::ArrowRight => cur.col = (cur.col + 1).min(5),
        Key::Home => cur.line = 0,
        Key::End => cur.line = lines - 1,
        _ => return edit(app, id, key, repeat, lines),
    }
    true
}

/// Writes key `key` into the cell under the cursor.
fn edit(app: &mut App, id: u8, key: Key, repeat: bool, lines: usize) -> bool {
    let (line, col, octave, step) = (app.phrase.line, app.phrase.col, app.octave, app.step);
    let Some(phrase) = app.project.module_mut(id).and_then(|m| m.phrase_mut()) else { return false };
    let cell = &mut phrase.cells[line];
    let used = match (col, key) {
        (_, Key::Delete) => {
            match col {
                0 => *cell = Cell::default(),
                1 | 2 => cell.vol = None,
                _ => cell.fx = None,
            }
            true
        }
        (0, Key::A | Key::Num1) if !repeat => {
            cell.note = Some(Note::Off);
            true
        }
        (1, k) if pattern::vol_key(k).is_some() => {
            cell.vol = pattern::vol_key(k).map(|c| pattern::vol_command_typed(cell.vol, c));
            true
        }
        // Zxx, Jxx and Wxx act on the song, so don't apply in phrases.
        (3, k) if pattern::fx_key(k).is_some_and(|c| ![FX_PHRASE, FX_BREAK, FX_WAIT].contains(&c)) => {
            cell.fx = Some((pattern::fx_key(k).unwrap(), cell.fx.map_or(0, |f| f.1)));
            true
        }
        (0, k) => match pattern::note_offset(k) {
            Some(off) if !repeat => {
                cell.note = Some(Note::On((octave as u32 * 12 + off as u32).min(119) as u8));
                true
            }
            _ => false,
        },
        (_, k) => match pattern::hex_digit(k) {
            Some(d) => {
                let (cmd, arg) = cell.fx.unwrap_or((0, 0));
                match col {
                    1 | 2 => cell.vol = pattern::type_vol(cell.vol, col == 1, d),
                    3 => cell.fx = Some((d, arg)),
                    _ => cell.fx = Some((cmd, pattern::set_nibble(Some(arg), col == 4, d).unwrap())),
                }
                true
            }
            None => false,
        },
    };
    if used {
        app.phrase.line = (line + step) % lines;
        app.mark();
    }
    used
}

/// A note picked by dragging, between `low` and `high`.
fn key_field(note: &mut u8, low: u8, high: u8) -> egui::DragValue<'_> {
    egui::DragValue::new(note)
        .range(low..=high)
        .speed(0.2)
        .custom_formatter(|v, _| Note::On(v as u8).label())
        .custom_parser(|t| (0..120u8).find(|&n| Note::On(n).label().eq_ignore_ascii_case(t.trim())).map(f64::from))
}
