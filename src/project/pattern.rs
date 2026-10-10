//! Patterns: notes and effects in cells, lines and columns, and the
//! automation envelopes they carry.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Note {
    On(u8),
    Off,
}

impl Note {
    pub fn label(self) -> String {
        match self {
            Note::Off => "OFF".into(),
            Note::On(n) => {
                const NAMES: [&str; 12] = ["C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-"];
                format!("{}{}", NAMES[(n % 12) as usize], n / 12)
            }
        }
    }
}

/// One line of one track.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    pub note: Option<Note>,
    /// Target module id: notes are sent straight to a module.
    pub module: Option<u8>,
    /// Velocity 00..80 (hex).
    pub vol: Option<u8>,
    /// Effect command and argument, e.g. `F` `8C` sets BPM to 140.
    pub fx: Option<(u8, u8)>,
    /// Panning 00..80 (hex), 40 in the middle: the panning column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pan: Option<u8>,
    /// Delays the note by xx/256 of a line: the delay column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<u8>,
}

impl Cell {
    /// An empty cell, as `Cell::default()` but usable in constants.
    pub const EMPTY: Cell = Cell { note: None, module: None, vol: None, fx: None, pan: None, delay: None };
}

/// A module parameter's course over a pattern: an automation envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub module: u8,
    /// Index into the module's parameters.
    pub param: usize,
    /// Points as (position in lines, value), sorted by position. Values run
    /// 0..1 along the parameter's slider, so frequencies move in octaves.
    pub points: Vec<(f32, f32)>,
    /// Hold each value until the next point instead of moving towards it.
    #[serde(default)]
    pub steps: bool,
    /// Move along a smooth curve through the points rather than straight
    /// lines. `steps` wins.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub curve: bool,
    /// For an envelope from one line to another: the line it starts on
    /// (its points are in lines from there) and how many lines it runs for,
    /// 0 for the rest of the pattern. Outside them the parameter keeps the
    /// song's value.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub start: f32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub lines: f32,
    /// For one that repeats instead: how often, in beats, through the whole
    /// pattern; 0 when it runs from line to line.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub every: f32,
    /// Whether an envelope saved before `every` started again after its
    /// lines, which `Project::load` turns into `every`.
    #[serde(default, rename = "repeat", skip_serializing)]
    pub(super) old_repeat: bool,
    /// How much of the pattern envelopes of earlier versions took, which
    /// `Project::load` turns into `lines`.
    #[serde(default = "unity", rename = "length", skip_serializing)]
    pub(super) old_length: f32,
}

/// The value of the envelope through `points` at `pos`, or `None` without
/// points. Before the first point and after the last the nearest one
/// holds. `steps` holds each value until the next point; `curve` runs
/// smoothly through them instead of in straight lines.
pub fn interpolate(points: &[(f32, f32)], pos: f32, steps: bool, curve: bool) -> Option<f32> {
    let next = points.iter().position(|p| p.0 > pos);
    Some(match next {
        None => points.last()?.1,
        Some(0) => points[0].1,
        Some(i) if steps => points[i - 1].1,
        Some(i) => {
            let ((x0, y0), (x1, y1)) = (points[i - 1], points[i]);
            let u = (pos - x0) / (x1 - x0);
            if curve {
                // Catmull-Rom through the neighbours, which stays on the
                // points and turns smoothly at them.
                let before = if i >= 2 { points[i - 2].1 } else { y0 };
                let after = points.get(i + 1).map_or(y1, |p| p.1);
                let (m0, m1) = ((y1 - before) * 0.5, (after - y0) * 0.5);
                let (u2, u3) = (u * u, u * u * u);
                let y = (2.0 * u3 - 3.0 * u2 + 1.0) * y0
                    + (u3 - 2.0 * u2 + u) * m0
                    + (-2.0 * u3 + 3.0 * u2) * y1
                    + (u3 - u2) * m1;
                y.clamp(0.0, 1.0)
            } else {
                y0 + (y1 - y0) * u
            }
        }
    })
}

/// Sets the value at `pos` in `points`, replacing a point there or adding
/// one; returns its index.
pub fn set_point(points: &mut Vec<(f32, f32)>, pos: f32, value: f32) -> usize {
    let value = value.clamp(0.0, 1.0);
    match points.iter().position(|p| p.0 >= pos) {
        Some(i) if (points[i].0 - pos).abs() < 1e-4 => {
            points[i].1 = value;
            i
        }
        Some(i) => {
            points.insert(i, (pos, value));
            i
        }
        None => {
            points.push((pos, value));
            points.len() - 1
        }
    }
}

impl Envelope {
    /// The value at `pos` lines, or `None` without points. Before the first
    /// point and after the last the nearest one holds.
    pub fn value_at(&self, pos: f32) -> Option<f32> {
        interpolate(&self.points, pos, self.steps, self.curve)
    }

    /// An envelope of `points` over the whole pattern, in lines.
    pub fn new(module: u8, param: usize, points: Vec<(f32, f32)>) -> Self {
        Self {
            module,
            param,
            points,
            steps: false,
            curve: false,
            start: 0.0,
            lines: 0.0,
            every: 0.0,
            old_repeat: false,
            old_length: 1.0,
        }
    }

    /// The lines it runs for in a pattern of `lines` at `lpb` lines a beat,
    /// from its start, before it ends or starts again.
    pub fn span(&self, lines: usize, lpb: u32) -> f32 {
        if self.every > 0.0 {
            return (self.every * lpb.max(1) as f32).max(0.25);
        }
        let rest = (lines as f32 - self.start).max(0.25);
        if self.lines > 0.0 { self.lines.min(rest).max(0.25) } else { rest }
    }

    /// Where `pos` lines into a pattern of `lines` at `lpb` lines a beat
    /// falls in the envelope: in its cycle when it repeats, otherwise from
    /// its start, and `None` outside its lines.
    pub fn local(&self, pos: f32, lines: usize, lpb: u32) -> Option<f32> {
        let span = self.span(lines, lpb);
        if self.every > 0.0 {
            return Some(pos.rem_euclid(span));
        }
        Some(pos - self.start).filter(|at| (0.0..span).contains(at))
    }

    /// Its value `pos` lines into a pattern of `lines` at `lpb` lines a
    /// beat, or `None` where it doesn't take effect.
    pub fn value_in(&self, pos: f32, lines: usize, lpb: u32) -> Option<f32> {
        self.value_at(self.local(pos, lines, lpb)?)
    }

    /// Sets the value at `pos`, replacing a point there or adding one.
    pub fn set(&mut self, pos: f32, value: f32) {
        set_point(&mut self.points, pos, value);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pattern {
    pub name: String,
    pub lines: usize,
    /// `tracks[track][line]`, the first note column of each track; every
    /// column always holds `MAX_LINES` cells so resizing a pattern never
    /// loses data.
    pub tracks: Vec<Vec<Cell>>,
    /// The other note columns: `extra[track][column - 1][line]`. Columns a
    /// track no longer shows keep their notes but don't play.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Vec<Vec<Vec<Cell>>>,
    /// Note columns shown for each track; missing ones have one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<u8>,
    /// Effect columns shown for each track, and their cells:
    /// `effects[track][column][line]`, of which only `fx` is used. Their
    /// commands act on every note column of the track.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fx_columns: Vec<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Vec<Vec<Cell>>>,
    /// Parameter envelopes that play along with the pattern.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub automation: Vec<Envelope>,
}

impl Pattern {
    pub fn new(name: impl Into<String>, tracks: usize, lines: usize) -> Self {
        Self {
            name: name.into(),
            lines,
            tracks: vec![vec![Cell::default(); MAX_LINES]; tracks],
            extra: Vec::new(),
            columns: Vec::new(),
            fx_columns: Vec::new(),
            effects: Vec::new(),
            automation: Vec::new(),
        }
    }

    pub fn num_tracks(&self) -> usize {
        self.tracks.len()
    }

    /// How many note columns `track` shows.
    pub fn columns(&self, track: usize) -> usize {
        self.columns.get(track).map_or(1, |&c| (c as usize).clamp(1, MAX_COLUMNS))
    }

    /// Shows `n` note columns on `track`; hidden ones keep their notes.
    pub fn set_columns(&mut self, track: usize, n: usize) {
        let n = n.clamp(1, MAX_COLUMNS);
        if self.columns.len() <= track {
            self.columns.resize(track + 1, 1);
        }
        self.columns[track] = n as u8;
        if self.extra.len() <= track {
            self.extra.resize(track + 1, Vec::new());
        }
        let extra = &mut self.extra[track];
        if extra.len() < n - 1 {
            extra.resize(n - 1, vec![Cell::default(); MAX_LINES]);
        }
    }

    /// How many effect columns `track` shows.
    pub fn fx_columns(&self, track: usize) -> usize {
        self.fx_columns.get(track).map_or(0, |&c| (c as usize).min(MAX_FX_COLUMNS))
    }

    /// Shows `n` effect columns on `track`; hidden ones keep their cells.
    pub fn set_fx_columns(&mut self, track: usize, n: usize) {
        let n = n.min(MAX_FX_COLUMNS);
        if self.fx_columns.len() <= track {
            self.fx_columns.resize(track + 1, 0);
        }
        self.fx_columns[track] = n as u8;
        if self.effects.len() <= track {
            self.effects.resize(track + 1, Vec::new());
        }
        let fx = &mut self.effects[track];
        if fx.len() < n {
            fx.resize(n, vec![Cell::default(); MAX_LINES]);
        }
    }

    /// The columns of `track`: its note columns, then its effect columns.
    pub fn width(&self, track: usize) -> usize {
        self.columns(track) + self.fx_columns(track)
    }

    /// Whether column `col` of `track` is an effect column.
    pub fn is_fx_column(&self, track: usize, col: usize) -> bool {
        col >= self.columns(track)
    }

    /// The effect commands `track`'s effect columns hold on `line`.
    pub fn track_effects(&self, track: usize, line: usize) -> impl Iterator<Item = (u8, u8)> + '_ {
        let shown = self.fx_columns(track);
        self.effects
            .get(track)
            .into_iter()
            .flat_map(move |cols| cols.iter().take(shown).filter_map(move |c| c[line].fx))
    }

    /// Column `col` of `track`: a note column, or past them an effect
    /// column. A column the track doesn't show reads as empty.
    pub fn column(&self, track: usize, col: usize) -> &[Cell] {
        static EMPTY: [Cell; MAX_LINES] = [Cell::EMPTY; MAX_LINES];
        let notes = self.columns(track);
        let found = match col {
            0 => self.tracks.get(track),
            c if c < notes => self.extra.get(track).and_then(|e| e.get(c - 1)),
            c => self.effects.get(track).and_then(|e| e.get(c - notes)),
        };
        found.map_or(&EMPTY, |c| c.as_slice())
    }

    pub fn column_mut(&mut self, track: usize, col: usize) -> &mut Vec<Cell> {
        let notes = self.columns(track);
        match col {
            0 => &mut self.tracks[track],
            c if c < notes => &mut self.extra[track][c - 1],
            c => &mut self.effects[track][c - notes],
        }
    }

    pub fn cell(&self, track: usize, col: usize, line: usize) -> Cell {
        self.column(track, col)[line]
    }

    pub fn cell_mut(&mut self, track: usize, col: usize, line: usize) -> &mut Cell {
        &mut self.column_mut(track, col)[line]
    }

    /// Every column shown, track by track, note columns before effect
    /// columns: the "lanes" the editor's cursor and blocks move across.
    pub fn num_lanes(&self) -> usize {
        (0..self.num_tracks()).map(|t| self.width(t)).sum()
    }

    /// The lane of column `col` of `track`.
    pub fn lane_of(&self, track: usize, col: usize) -> usize {
        (0..track).map(|t| self.width(t)).sum::<usize>() + col.min(self.width(track) - 1)
    }

    /// The track and column of `lane`.
    pub fn lane_pos(&self, lane: usize) -> (usize, usize) {
        let mut l = lane;
        for t in 0..self.num_tracks() {
            let c = self.width(t);
            if l < c {
                return (t, l);
            }
            l -= c;
        }
        let last = self.num_tracks() - 1;
        (last, self.width(last) - 1)
    }

    pub fn lane(&self, lane: usize) -> &[Cell] {
        let (t, c) = self.lane_pos(lane);
        self.column(t, c)
    }

    pub fn lane_mut(&mut self, lane: usize) -> &mut Vec<Cell> {
        let (t, c) = self.lane_pos(lane);
        self.column_mut(t, c)
    }

    /// Whether any column of `track` holds something on the pattern's lines.
    pub fn track_used(&self, track: usize) -> bool {
        (0..self.width(track)).any(|c| self.column(track, c)[..self.lines].iter().any(|c| *c != Cell::default()))
    }

    /// Removes everything from `track`'s columns, shown or not.
    pub fn clear_track(&mut self, track: usize) {
        self.tracks[track].fill(Cell::default());
        for cols in [self.extra.get_mut(track), self.effects.get_mut(track)].into_iter().flatten() {
            for c in cols {
                c.fill(Cell::default());
            }
        }
    }

    /// Every column of `track`, note columns and effect columns, as the
    /// matrix copies it.
    pub fn track_cells(&self, track: usize) -> TrackCells {
        let col = |c: usize| self.column(track, c).to_vec();
        let notes = self.columns(track);
        TrackCells { notes: (0..notes).map(col).collect(), effects: (notes..self.width(track)).map(col).collect() }
    }

    /// Keeps the column settings in step with the tracks and the columns
    /// stored for every one shown.
    pub(super) fn normalize_columns(&mut self) {
        let n = self.num_tracks();
        self.columns.truncate(n);
        self.extra.truncate(n);
        self.fx_columns.truncate(n);
        self.effects.truncate(n);
        for t in 0..self.columns.len() {
            let c = self.columns(t);
            self.set_columns(t, c);
        }
        for t in 0..self.fx_columns.len() {
            let c = self.fx_columns(t);
            self.set_fx_columns(t, c);
        }
        for cols in self.extra.iter_mut().chain(&mut self.effects) {
            for col in cols {
                col.resize(MAX_LINES, Cell::default());
            }
        }
        let empty = |cols: &Vec<Vec<Vec<Cell>>>| cols.iter().flatten().flatten().all(|c| *c == Cell::default());
        if self.columns.iter().all(|&c| c <= 1) && empty(&self.extra) {
            self.columns.clear();
            self.extra.clear();
        }
        if self.fx_columns.iter().all(|&c| c == 0) && empty(&self.effects) {
            self.fx_columns.clear();
            self.effects.clear();
        }
    }

    pub fn insert_track(&mut self, track: usize) {
        self.tracks.insert(track, vec![Cell::default(); MAX_LINES]);
        if track < self.columns.len() {
            self.columns.insert(track, 1);
        }
        if track < self.extra.len() {
            self.extra.insert(track, Vec::new());
        }
        if track < self.fx_columns.len() {
            self.fx_columns.insert(track, 0);
        }
        if track < self.effects.len() {
            self.effects.insert(track, Vec::new());
        }
    }

    pub fn remove_track(&mut self, track: usize) {
        self.tracks.remove(track);
        if track < self.columns.len() {
            self.columns.remove(track);
        }
        if track < self.extra.len() {
            self.extra.remove(track);
        }
        if track < self.fx_columns.len() {
            self.fx_columns.remove(track);
        }
        if track < self.effects.len() {
            self.effects.remove(track);
        }
    }
}
