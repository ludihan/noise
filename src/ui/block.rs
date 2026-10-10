//! Blocks of pattern data: the selection, the clipboard and the edits
//! made on them.

use crate::project::{Cell, Note, Pattern};

/// A rectangle of a pattern: lines and lanes (note columns, counted across
/// the tracks; see `Pattern::lane_pos`), both inclusive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Block {
    pub lines: (usize, usize),
    pub tracks: (usize, usize),
}

impl Block {
    /// The block with corners `a` and `b`, each given as (line, lane).
    pub fn between(a: (usize, usize), b: (usize, usize)) -> Self {
        Block { lines: (a.0.min(b.0), a.0.max(b.0)), tracks: (a.1.min(b.1), a.1.max(b.1)) }
    }

    pub fn contains(&self, line: usize, track: usize) -> bool {
        (self.lines.0..=self.lines.1).contains(&line) && (self.tracks.0..=self.tracks.1).contains(&track)
    }

    /// The part of the block inside `p`, if any.
    pub fn within(self, p: &Pattern) -> Option<Block> {
        if self.lines.0 >= p.lines || self.tracks.0 >= p.num_lanes() {
            return None;
        }
        let lines = (self.lines.0, self.lines.1.min(p.lines - 1));
        Some(Block { lines, tracks: (self.tracks.0, self.tracks.1.min(p.num_lanes() - 1)) })
    }

    pub fn len(&self) -> usize {
        self.lines.1 - self.lines.0 + 1
    }

    pub fn width(&self) -> usize {
        self.tracks.1 - self.tracks.0 + 1
    }
}

/// Cells copied from a pattern, as `clip[lane][line]`.
pub type Clip = Vec<Vec<Cell>>;

/// Each lane's lines of the block, in order.
fn columns(p: &mut Pattern, b: Block) -> Vec<&mut [Cell]> {
    let lanes: Vec<(usize, usize)> = (b.tracks.0..=b.tracks.1).map(|l| p.lane_pos(l)).collect();
    let rank = |pos: (usize, usize)| lanes.iter().position(|&l| l == pos);
    let shown: Vec<(usize, usize)> = (0..p.num_tracks()).map(|t| (p.columns(t), p.fx_columns(t))).collect();
    let Pattern { tracks, extra, effects, .. } = p;
    let mut out: Vec<(usize, &mut [Cell])> = Vec::new();
    for (t, col) in tracks.iter_mut().enumerate() {
        if let Some(r) = rank((t, 0)) {
            out.push((r, &mut col[b.lines.0..=b.lines.1]));
        }
    }
    // Only the columns shown are lanes: a hidden note column keeps its
    // cells out of the way of the effect columns after it.
    for (t, cols) in extra.iter_mut().enumerate() {
        for (c, col) in cols.iter_mut().enumerate().take(shown[t].0 - 1) {
            if let Some(r) = rank((t, c + 1)) {
                out.push((r, &mut col[b.lines.0..=b.lines.1]));
            }
        }
    }
    for (t, cols) in effects.iter_mut().enumerate() {
        for (c, col) in cols.iter_mut().enumerate().take(shown[t].1) {
            if let Some(r) = rank((t, shown[t].0 + c)) {
                out.push((r, &mut col[b.lines.0..=b.lines.1]));
            }
        }
    }
    out.sort_by_key(|o| o.0);
    out.into_iter().map(|o| o.1).collect()
}

pub fn copy(p: &Pattern, b: Block) -> Clip {
    (b.tracks.0..=b.tracks.1).map(|l| p.lane(l)[b.lines.0..=b.lines.1].to_vec()).collect()
}

pub fn clear(p: &mut Pattern, b: Block) {
    for col in columns(p, b) {
        col.fill(Cell::default());
    }
}

/// Pastes `clip` with its top left corner at `line` and `lane`, cut off
/// at the pattern's edges. A mix paste only fills fields that are empty.
pub fn paste(p: &mut Pattern, clip: &Clip, line: usize, lane: usize, mix: bool) {
    let lines = p.lines;
    for (i, src) in clip.iter().enumerate().take(p.num_lanes().saturating_sub(lane)) {
        let (t, c) = p.lane_pos(lane + i);
        // An effect column takes only the effects.
        let fx_only = p.is_fx_column(t, c);
        let dst = p.lane_mut(lane + i);
        for (cell, to) in src.iter().zip(dst[line.min(lines)..lines].iter_mut()) {
            let cell = &if fx_only { Cell { fx: cell.fx, ..Cell::default() } } else { *cell };
            if mix {
                to.note = to.note.or(cell.note);
                to.module = to.module.or(cell.module);
                to.vol = to.vol.or(cell.vol);
                to.fx = to.fx.or(cell.fx);
                to.pan = to.pan.or(cell.pan);
                to.delay = to.delay.or(cell.delay);
            } else {
                *to = *cell;
            }
        }
    }
}

/// Moves the notes in the block by `semis` semitones, within C-0..B-9.
pub fn transpose(p: &mut Pattern, b: Block, semis: i32) {
    for cell in columns(p, b).into_iter().flatten() {
        if let Some(Note::On(n)) = cell.note {
            cell.note = Some(Note::On((n as i32 + semis).clamp(0, 119) as u8));
        }
    }
}

/// Moves the notes in the block by up to `amount` (0..1) of their volume
/// and, in the tracks `delays` says show their delay column, up to an
/// eighth of a line late, at random from `seed`, so a played part sounds
/// less mechanical.
pub fn humanize(p: &mut Pattern, b: Block, amount: f32, delays: &[bool], seed: u32) {
    let mut rng = crate::rng::Rng(seed);
    let mut random = move || rng.unit();
    let tracks: Vec<usize> = (b.tracks.0..=b.tracks.1).map(|l| p.lane_pos(l).0).collect();
    for (col, t) in columns(p, b).into_iter().zip(tracks) {
        // Volume column commands are left as they are.
        let humanized = |c: &Cell| matches!(c.note, Some(Note::On(_))) && c.vol.is_none_or(|v| v <= 0x80);
        for cell in col.iter_mut().filter(|c| humanized(c)) {
            let vol = cell.vol.unwrap_or(0x80) as f32 * (1.0 + amount * (2.0 * random() - 1.0));
            cell.vol = Some(vol.round().clamp(1.0, 128.0) as u8);
            if delays.get(t).copied().unwrap_or(false) {
                let late = cell.delay.unwrap_or(0) as f32 + amount * 32.0 * random();
                cell.delay = Some(late.round().min(255.0) as u8).filter(|&d| d > 0);
            }
        }
    }
}

/// Sends the notes in the block to `module`.
pub fn set_module(p: &mut Pattern, b: Block, module: u8) {
    for cell in columns(p, b).into_iter().flatten() {
        if matches!(cell.note, Some(Note::On(_))) {
            cell.module = Some(module);
        }
    }
}

/// In each lane of the block, fills the volumes, panning, delays and effect values between
/// its first and last line along a straight line from one to the other.
/// Effects are filled only where both ends use the same command. Returns
/// whether there was anything to fill.
pub fn interpolate(p: &mut Pattern, b: Block) -> bool {
    if b.len() < 3 {
        return false;
    }
    let n = b.len() - 1;
    let mut any = false;
    for col in columns(p, b) {
        let (first, last) = (col[0], col[n]);
        let at = |a: u8, z: u8, i: usize| (a as f32 + (z as f32 - a as f32) * i as f32 / n as f32).round() as u8;
        if let (Some(a @ ..=0x80), Some(z @ ..=0x80)) = (first.vol, last.vol) {
            for (i, c) in col.iter_mut().enumerate() {
                c.vol = Some(at(a, z, i));
            }
            any = true;
        }
        if let (Some(a), Some(z)) = (first.pan, last.pan) {
            for (i, c) in col.iter_mut().enumerate() {
                c.pan = Some(at(a, z, i));
            }
            any = true;
        }
        if let (Some(a), Some(z)) = (first.delay, last.delay) {
            for (i, c) in col.iter_mut().enumerate() {
                c.delay = Some(at(a, z, i));
            }
            any = true;
        }
        if let (Some((cmd, a)), Some((cmd_z, z))) = (first.fx, last.fx)
            && cmd == cmd_z
        {
            for (i, c) in col.iter_mut().enumerate() {
                c.fx = Some((cmd, at(a, z, i)));
            }
            any = true;
        }
    }
    any
}

/// Spreads the block's lines twice as far apart; what would land past its
/// end is dropped.
pub fn expand(p: &mut Pattern, b: Block) {
    for col in columns(p, b) {
        let old = col.to_vec();
        col.fill(Cell::default());
        for (i, cell) in old.into_iter().enumerate().take(col.len().div_ceil(2)) {
            col[2 * i] = cell;
        }
    }
}

/// Packs every other line of the block into its first half.
pub fn shrink(p: &mut Pattern, b: Block) {
    for col in columns(p, b) {
        let old = col.to_vec();
        col.fill(Cell::default());
        for (to, cell) in col.iter_mut().zip(old.into_iter().step_by(2)) {
            *to = cell;
        }
    }
}

/// The note, module, volume and effect of a cell as the editor shows them.
pub fn fields(cell: &Cell) -> [String; 4] {
    [
        cell.note.map_or("---".into(), |n| n.label()),
        cell.module.map_or("..".into(), |m| format!("{m:02X}")),
        cell.vol.map_or("..".into(), crate::project::vol_text),
        cell.fx.map_or("...".into(), |(c, a)| format!("{}{a:02X}", fx_command(c))),
    ]
}

/// An effect command as the editor shows it: a hex digit, or `Z` for
/// `FX_PHRASE`.
pub fn fx_command(c: u8) -> char {
    char::from_digit(c as u32, 36).map_or('?', |c| c.to_ascii_uppercase())
}

/// The panning and delay of a cell as the editor shows them.
pub fn mixer_fields(cell: &Cell) -> [String; 2] {
    let hex = |v: Option<u8>| v.map_or("..".into(), |v| format!("{v:02X}"));
    [hex(cell.pan), hex(cell.delay)]
}

/// The clip as tracker text: a text line per pattern line, with the lanes
/// separated by `|`. Panning and delay follow the effect when any cell has
/// them.
pub fn to_text(clip: &Clip) -> String {
    let lines = clip.first().map_or(0, Vec::len);
    let wide = clip.iter().flatten().any(|c| c.pan.is_some() || c.delay.is_some());
    let text = |c: &Cell| {
        let mut f = fields(c).to_vec();
        if wide {
            f.extend(mixer_fields(c));
        }
        f.join(" ")
    };
    let line = |l: usize| clip.iter().map(|t| text(&t[l])).collect::<Vec<_>>().join(" | ");
    (0..lines).map(line).collect::<Vec<_>>().join("\n")
}

/// Reads text written by `to_text`, so blocks can be pasted between songs
/// open in different windows.
pub fn from_text(text: &str) -> Option<Clip> {
    let rows: Vec<Vec<Cell>> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split('|').map(parse_cell).collect::<Option<Vec<_>>>())
        .collect::<Option<_>>()?;
    let width = rows.first()?.len();
    if rows.iter().any(|r| r.len() != width) {
        return None;
    }
    Some((0..width).map(|t| rows.iter().map(|r| r[t]).collect()).collect())
}

fn parse_cell(text: &str) -> Option<Cell> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let (note, module, vol, fx, pan, delay) = match words[..] {
        [n, m, v, f] => (n, m, v, f, "..", ".."),
        [n, m, v, f, p, d] => (n, m, v, f, p, d),
        _ => return None,
    };
    let hex = |s: &str| if s == ".." { Some(None) } else { u8::from_str_radix(s, 16).ok().map(Some) };
    Some(Cell {
        note: match note {
            "---" => None,
            "OFF" => Some(Note::Off),
            n => Some(Note::On((0..120).find(|&i| Note::On(i).label() == n)?)),
        },
        module: hex(module)?,
        vol: if vol == ".." { None } else { Some(crate::project::parse_vol(vol)?) },
        fx: match fx {
            "..." => None,
            f if f.len() == 3 && f.is_ascii() => {
                Some((f[..1].chars().next()?.to_digit(36)? as u8, u8::from_str_radix(&f[1..], 16).ok()?))
            }
            _ => return None,
        },
        pan: hex(pan)?.map(|v| v.min(0x80)),
        delay: hex(delay)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_column_commands_are_kept_through_edits_and_text() {
        let fade = crate::project::vol_command_value('I', 3);
        let mut p = Pattern::new("t", 1, 8);
        p.tracks[0][0] = Cell { note: Some(Note::On(48)), vol: fade, ..Cell::default() };
        p.tracks[0][4] = Cell { note: Some(Note::On(48)), vol: Some(0x20), ..Cell::default() };
        p.tracks[0][7] = Cell { vol: Some(0x40), ..Cell::default() };
        let all = Block { tracks: (0, 0), lines: (0, 7) };
        humanize(&mut p, all, 0.5, &[], 7);
        assert_eq!(p.tracks[0][0].vol, fade, "humanize leaves commands");
        assert!(!interpolate(&mut p, all), "nor does interpolating from one");
        let clip = vec![p.tracks[0].clone()];
        assert_eq!(from_text(&to_text(&clip)).unwrap()[0][0].vol, fade);
    }

    #[test]
    fn humanize_moves_note_volumes_a_little_and_delays_where_shown() {
        let mut p = Pattern::new("t", 2, 16);
        for line in 0..16 {
            p.tracks[0][line] = Cell { note: Some(Note::On(48)), vol: Some(0x40), ..Cell::default() };
            p.tracks[1][line] = Cell { note: Some(Note::On(48)), ..Cell::default() };
        }
        p.tracks[0][3] = Cell { note: Some(Note::Off), ..Cell::default() };
        let b = Block { lines: (0, 15), tracks: (0, 1) };
        humanize(&mut p, b, 0.1, &[false, true], 1);
        let vols: Vec<u8> = p.tracks[0].iter().filter_map(|c| c.vol).collect();
        assert_eq!(vols.len(), 15, "notes only");
        assert!(vols.iter().all(|&v| (0x3A..=0x46).contains(&v)), "{vols:?}");
        assert!(vols.iter().any(|&v| v != 0x40), "they move");
        assert!(p.tracks[0].iter().all(|c| c.delay.is_none()), "no delays where the column is hidden");
        assert!(p.tracks[1].iter().any(|c| c.delay.is_some()), "late here");
        assert!(p.tracks[1][..16].iter().all(|c| c.vol.is_some_and(|v| v <= 0x80)), "full volume moves down only");
    }

    fn note(n: u8) -> Cell {
        Cell { note: Some(Note::On(n)), module: Some(1), ..Cell::default() }
    }

    fn pattern() -> Pattern {
        let mut p = Pattern::new("t", 3, 8);
        for l in 0..8 {
            p.tracks[0][l] = note(48 + l as u8);
        }
        p
    }

    #[test]
    fn effect_columns_are_lanes_that_hold_only_effects() {
        let mut p = pattern();
        // Track 0: a note column with a hidden second one, then an effect column.
        p.set_columns(0, 2);
        p.cell_mut(0, 1, 0).fx = Some((0x4, 0x11));
        p.set_columns(0, 1);
        p.set_fx_columns(0, 1);
        assert_eq!((p.num_lanes(), p.lane_pos(1)), (4, (0, 1)));
        // Pasting notes into the effect column keeps only their effects.
        let clip = vec![vec![Cell { fx: Some((0xC, 0x02)), ..note(60) }]];
        paste(&mut p, &clip, 2, 1, false);
        assert_eq!(p.cell(0, 1, 2), Cell { fx: Some((0xC, 0x02)), ..Cell::default() });
        // Clearing the effect lane leaves the hidden note column alone.
        clear(&mut p, Block::between((0, 1), (7, 1)));
        assert_eq!(p.cell(0, 1, 2), Cell::default());
        assert_eq!(p.extra[0][0][0].fx, Some((0x4, 0x11)), "the hidden column kept its cell");
    }

    #[test]
    fn copy_and_paste_stop_at_the_edges() {
        let mut p = pattern();
        let clip = copy(&p, Block::between((0, 0), (3, 1)));
        assert_eq!((clip.len(), clip[0].len()), (2, 4));
        // Pasted two lines from the bottom of the last track, only the
        // first track and two lines of the clip fit.
        paste(&mut p, &clip, 6, 2, false);
        assert_eq!(p.tracks[2][6], note(48));
        assert_eq!(p.tracks[2][7], note(49));
        assert_eq!(p.tracks[2][8], Cell::default(), "nothing past the pattern's lines");
    }

    #[test]
    fn blocks_cross_note_columns() {
        let mut p = pattern();
        p.set_columns(0, 2);
        // Lanes: track 0 column 0, track 0 column 1, track 1, track 2.
        let clip = copy(&p, Block::between((0, 0), (1, 0)));
        paste(&mut p, &clip, 0, 1, false);
        assert_eq!(p.cell(0, 1, 1), note(49));
        transpose(&mut p, Block::between((0, 1), (1, 2)), 1);
        assert_eq!(p.cell(0, 1, 0).note, Some(Note::On(49)));
        assert_eq!(p.cell(0, 0, 0).note, Some(Note::On(48)), "outside the block");
        paste(&mut p, &clip, 0, 3, false);
        assert_eq!(p.tracks[2][0], note(48), "the last lane is track 2");
    }

    #[test]
    fn mix_paste_keeps_what_is_there() {
        let mut p = pattern();
        let clip = vec![vec![Cell { note: Some(Note::Off), vol: Some(0x20), ..Cell::default() }]];
        paste(&mut p, &clip, 0, 0, true);
        assert_eq!(p.tracks[0][0], Cell { vol: Some(0x20), ..note(48) });
        paste(&mut p, &clip, 0, 0, false);
        assert_eq!(p.tracks[0][0].note, Some(Note::Off));
    }

    #[test]
    fn transpose_stays_on_the_keyboard() {
        let mut p = pattern();
        p.tracks[0][1].note = Some(Note::On(118));
        p.tracks[0][2].note = Some(Note::Off);
        transpose(&mut p, Block::between((0, 0), (2, 0)), 12);
        assert_eq!(p.tracks[0][0].note, Some(Note::On(60)));
        assert_eq!(p.tracks[0][1].note, Some(Note::On(119)));
        assert_eq!(p.tracks[0][2].note, Some(Note::Off));
        assert_eq!(p.tracks[0][3].note, Some(Note::On(51)), "outside the block");
    }

    #[test]
    fn interpolate_volumes_and_effects() {
        let mut p = pattern();
        p.tracks[1][0] = Cell { vol: Some(0x00), fx: Some((0xF, 0x10)), ..Cell::default() };
        p.tracks[1][4] = Cell { vol: Some(0x40), fx: Some((0xF, 0x20)), ..Cell::default() };
        assert!(interpolate(&mut p, Block::between((0, 1), (4, 1))));
        let vols: Vec<_> = (0..5).map(|l| p.tracks[1][l].vol.unwrap()).collect();
        assert_eq!(vols, [0x00, 0x10, 0x20, 0x30, 0x40]);
        assert_eq!(p.tracks[1][2].fx, Some((0xF, 0x18)));
        assert!(!interpolate(&mut p, Block::between((0, 2), (4, 2))), "nothing at the ends");
    }

    #[test]
    fn expand_and_shrink() {
        let mut p = pattern();
        let all = Block::between((0, 0), (7, 0));
        expand(&mut p, all);
        let notes: Vec<_> = (0..8).map(|l| p.tracks[0][l].note).collect();
        let on = |n| Some(Note::On(n));
        assert_eq!(notes, [on(48), None, on(49), None, on(50), None, on(51), None]);
        shrink(&mut p, all);
        let notes: Vec<_> = (0..8).map(|l| p.tracks[0][l].note).collect();
        assert_eq!(notes, [on(48), on(49), on(50), on(51), None, None, None, None]);
    }

    #[test]
    fn text_round_trip() {
        let mut p = pattern();
        p.tracks[1][1] = Cell { note: Some(Note::Off), vol: Some(0x7F), fx: Some((0xC, 0x03)), ..Cell::default() };
        p.tracks[2][1].fx = Some((crate::project::FX_PHRASE, 0x02));
        let clip = copy(&p, Block::between((0, 0), (2, 2)));
        let text = to_text(&clip);
        assert_eq!(text.lines().nth(1).unwrap(), "C#4 01 .. ... | OFF .. 7F C03 | --- .. .. Z02");
        assert_eq!(from_text(&text), Some(clip));
        assert_eq!(from_text("hello"), None);
        // With panning and delay, every cell gets them.
        p.tracks[0][0].pan = Some(0x10);
        p.tracks[2][2].delay = Some(0x80);
        let clip = copy(&p, Block::between((0, 0), (2, 2)));
        let text = to_text(&clip);
        assert_eq!(text.lines().next().unwrap(), "C-4 01 .. ... 10 .. | --- .. .. ... .. .. | --- .. .. ... .. ..");
        assert_eq!(from_text(&text), Some(clip));
    }
}
