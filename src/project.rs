//! Song data: patterns, the order list and the module graph.
//!
//! The UI owns a `Project` and sends immutable snapshots of it to the audio
//! thread whenever it changes. Nothing in here touches DSP state.

use crate::sample::Sample;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const MAX_TRACKS: usize = 32;
/// Note columns a track can have.
pub const MAX_COLUMNS: usize = 8;
/// Effect columns a track can have.
pub const MAX_FX_COLUMNS: usize = 8;
pub const MAX_LINES: usize = 256;
pub const OUTPUT_ID: u8 = 0;

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
    /// How much of the pattern the envelope takes, up to all of it (1).
    #[serde(default = "unity", skip_serializing_if = "is_unity")]
    pub length: f32,
    /// A shorter envelope starts again when it ends, through the rest of
    /// the pattern, rather than holding its last value.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub repeat: bool,
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
        Self { module, param, points, steps: false, curve: false, length: 1.0, repeat: false }
    }

    /// The lines it runs for in a pattern of `lines`, before it holds or
    /// starts again.
    pub fn span(&self, lines: usize) -> f32 {
        (lines as f32 * self.length.clamp(0.0, 1.0)).max(0.25)
    }

    /// Where `pos` lines into the pattern falls in the envelope: past its
    /// end it starts again if it repeats.
    pub fn local(&self, pos: f32, lines: usize) -> f32 {
        let span = self.span(lines);
        if self.repeat && pos >= span { pos.rem_euclid(span) } else { pos }
    }

    /// Its value `pos` lines into a pattern of `lines`.
    pub fn value_in(&self, pos: f32, lines: usize) -> Option<f32> {
        self.value_at(self.local(pos, lines))
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
    fn normalize_columns(&mut self) {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModuleKind {
    Output,
    Generator,
    Fm,
    Drums,
    Sampler,
    Filter,
    Distortion,
    Delay,
    Reverb,
    Amplifier,
    Lfo,
    Flanger,
    Compressor,
    Eq,
    MultiSynth,
    Modulator,
    Phaser,
    VocalFilter,
    Repeater,
    RingMod,
    Gate,
    Kicker,
    SpectraVoice,
    PitchShifter,
    StereoExpander,
    CombFilter,
    Maximizer,
    Exciter,
    DcBlocker,
    ScreamFilter,
    Multitap,
    Eq10,
    Cabinet,
    Vibrato,
    Fmx,
    Input,
    WaveShaper,
    FilterPro,
    Chorus,
    Echo,
    AnalogFilter,
    PlateReverb,
    Eq5,
    Glide,
    Wavetable,
    Granular,
    Convolver,
}

/// How a parameter's value is shown.
#[derive(Clone, Copy, PartialEq)]
pub enum Unit {
    Plain,
    /// A linear gain, shown in decibels.
    Gain,
    Hz,
    /// Seconds, shown in milliseconds below one second.
    Seconds,
    /// 0..1 as a percentage.
    Percent,
    Semitones,
    Cents,
    Lines,
    /// -1..1, left to right.
    Pan,
    /// A frequency ratio.
    Ratio,
    /// A MIDI-style note number, shown as a note name.
    Note,
    Octaves,
    /// 0..4 through the vowels A, E, I, O and U.
    Vowel,
}

pub struct ParamSpec {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// If set, the param is a discrete choice and these are its labels.
    pub choices: &'static [&'static str],
    /// The param only takes whole numbers.
    pub integer: bool,
    pub unit: Unit,
}

const fn p(name: &'static str, min: f32, max: f32, default: f32) -> ParamSpec {
    ParamSpec { name, min, max, default, choices: &[], integer: false, unit: Unit::Plain }
}

const fn c(name: &'static str, default: f32, choices: &'static [&'static str]) -> ParamSpec {
    ParamSpec { name, min: 0.0, max: (choices.len() - 1) as f32, default, choices, integer: false, unit: Unit::Plain }
}

const fn i(name: &'static str, min: f32, max: f32, default: f32) -> ParamSpec {
    ParamSpec { name, min, max, default, choices: &[], integer: true, unit: Unit::Plain }
}

impl ParamSpec {
    const fn unit(self, unit: Unit) -> Self {
        ParamSpec { unit, ..self }
    }

    /// Wide ranges of positive values, like frequencies, move in octaves.
    fn logarithmic(&self) -> bool {
        self.min > 0.0 && self.max / self.min >= 100.0
    }

    /// Envelope times get more room for short values.
    fn squared(&self) -> bool {
        self.unit == Unit::Seconds && !self.logarithmic()
    }

    /// Where `v` sits along the parameter's slider, 0..1.
    pub fn position(&self, v: f32) -> f32 {
        let v = v.clamp(self.min, self.max);
        let t = if self.logarithmic() {
            (v / self.min).ln() / (self.max / self.min).ln()
        } else {
            (v - self.min) / (self.max - self.min)
        };
        if self.squared() { t.sqrt() } else { t }
    }

    /// The value at `t` (0..1) along the parameter's slider.
    pub fn value_at(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let t = if self.squared() { t * t } else { t };
        let v = if self.logarithmic() {
            self.min * (self.max / self.min).powf(t)
        } else {
            self.min + t * (self.max - self.min)
        };
        if self.integer || !self.choices.is_empty() { v.round() } else { v }
    }

    /// The value as shown to the user, with its unit.
    pub fn format(&self, v: f32) -> String {
        if !self.choices.is_empty() {
            return self.choices[(v.round().max(0.0) as usize).min(self.choices.len() - 1)].to_string();
        }
        match self.unit {
            Unit::Plain if self.integer => format!("{}", v.round() as i32),
            Unit::Plain => format!("{v:.2}"),
            Unit::Gain if v <= 0.0 => "-inf dB".into(),
            Unit::Gain => format!("{:+.1} dB", 20.0 * v.log10()),
            Unit::Hz if v >= 1000.0 => format!("{:.2} kHz", v / 1000.0),
            Unit::Hz if v < 10.0 => format!("{v:.2} Hz"),
            Unit::Hz => format!("{v:.0} Hz"),
            Unit::Seconds if v < 1.0 => format!("{:.0} ms", v * 1000.0),
            Unit::Seconds => format!("{v:.2} s"),
            Unit::Percent => format!("{:.0}%", v * 100.0),
            Unit::Semitones if self.integer => format!("{:+} st", v.round() as i32),
            Unit::Semitones => format!("{v:.2} st"),
            Unit::Cents => format!("{v:+.0} ct"),
            Unit::Lines => format!("{v:.2} lines"),
            Unit::Pan if v.abs() < 0.005 => "Center".into(),
            Unit::Pan => format!("{:.0}{}", v.abs() * 100.0, if v < 0.0 { "L" } else { "R" }),
            Unit::Note => Note::On(v.round().clamp(0.0, 119.0) as u8).label(),
            Unit::Octaves => format!("{v:+.2} oct"),
            Unit::Vowel => {
                // The vowel, or the two it is between and how far along.
                let names = ['A', 'E', 'I', 'O', 'U'];
                let (k, t) = (v.clamp(0.0, 4.0).floor() as usize, v.clamp(0.0, 4.0).fract());
                match t {
                    t if t < 0.05 => names[k].to_string(),
                    t if t > 0.95 => names[k + 1].to_string(),
                    t => format!("{}–{} {:.0}%", names[k], names[k + 1], t * 100.0),
                }
            }
            Unit::Ratio if self.integer => format!("×{}", v.round() as i32),
            Unit::Ratio => format!("×{v:.2}"),
        }
    }
}

pub const WAVES: &[&str] = &["Saw", "Square", "Triangle", "Sine", "Noise"];
/// The Wavetable synth's tables: sine to triangle to saw to square, a
/// narrowing pulse, a hard-synced saw, a folded sine, and vowels.
pub const WAVETABLES: &[&str] = &["Basic", "Pulse", "Sync", "Fold", "Vocal"];
/// The Convolver's impulses: made-up spaces and a cabinet, or the sample
/// loaded into it.
pub const IMPULSES: &[&str] = &["Room", "Hall", "Plate", "Spring", "Cabinet", "Sample"];
pub const LOOP_MODES: &[&str] = &["Off", "Forward", "Backward", "Ping-pong"];
pub const FILTER_MODES: &[&str] = &["Lowpass", "Highpass", "Bandpass"];
pub const DISTORTION_TYPES: &[&str] = &["Soft clip", "Hard clip", "Wave fold"];
pub const LFO_MODES: &[&str] = &["Tremolo", "Auto-pan"];
pub const LFO_SHAPES: &[&str] = &["Sine", "Triangle", "Square", "Saw down", "Saw up"];
/// A Modulator's LFO shapes: the LFO's, and one drawn by hand.
pub const MODULATOR_SHAPES: &[&str] = &["Sine", "Triangle", "Square", "Saw down", "Saw up", "Drawn"];
/// The index of the drawn shape in `MODULATOR_SHAPES`.
pub const DRAWN_SHAPE: u32 = 5;
/// What a Modulator's cycle can be timed by: Hz, a period in lines, or a
/// length in beats.
pub const MODULATOR_SYNCS: &[&str] = &["Off (Hz)", "Lines", "Beats"];
/// The lengths in beats a synced Modulator's cycle takes, and their labels.
pub const MODULATOR_BEATS: &[&str] = &["1/16", "1/8", "1/4", "1/2", "1", "2", "4", "8", "16", "32"];
pub const MODULATOR_BEAT_LENGTHS: [f32; 10] = [0.0625, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
/// The shape a drawn LFO starts as: up and down once, as (where in the
/// cycle, value 0..1).
pub const DEFAULT_SHAPE: [(f32, f32); 3] = [(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)];
pub const FLANGER_MODES: &[&str] = &["Flanger", "Chorus"];
pub const MULTISYNTH_MODES: &[&str] = &["All", "Round robin", "Random"];
/// When a Glide slides: into every note, or only one played on top of the
/// last, without a gap, which then doesn't start again.
pub const GLIDE_MODES: &[&str] = &["Always", "Legato"];
/// What a Modulator moves its parameters by: an LFO, or what it follows:
/// the level of its input, the last note played on it, or how hard; an
/// envelope each note starts; or just its Amount, as a knob that turns
/// them all.
pub const MODULATOR_MODES: &[&str] = &["LFO", "Follow input", "Key", "Velocity", "Envelope", "Manual"];
pub const REPEATER_HOLD: &[&str] = &["Off", "Hold"];
/// The Repeater's lengths, and how many lines each is.
pub const REPEATER_LENGTHS: &[&str] = &[
    "8 lines",
    "4 lines",
    "2 lines",
    "1 line",
    "1/2 line",
    "1/3 line",
    "1/4 line",
    "1/6 line",
    "1/8 line",
    "1/16 line",
];
pub const REPEATER_LINES: [f32; 10] = [8.0, 4.0, 2.0, 1.0, 0.5, 1.0 / 3.0, 0.25, 1.0 / 6.0, 0.125, 0.0625];

use Unit::{Cents, Gain, Hz, Lines, Pan, Percent, Ratio, Seconds, Semitones};

static OUTPUT_PARAMS: [ParamSpec; 1] = [p("Volume", 0.0, 1.5, 0.8).unit(Gain)];
static GENERATOR_PARAMS: [ParamSpec; 10] = [
    p("Volume", 0.0, 1.0, 0.5).unit(Gain),
    c("Wave", 0.0, WAVES),
    p("Attack", 0.0, 2.0, 0.005).unit(Seconds),
    p("Decay", 0.0, 2.0, 0.2).unit(Seconds),
    p("Sustain", 0.0, 1.0, 0.6).unit(Percent),
    p("Release", 0.0, 4.0, 0.2).unit(Seconds),
    p("Detune", -100.0, 100.0, 0.0).unit(Cents),
    i("Unison", 1.0, 4.0, 1.0),
    p("Pulse width", 0.05, 0.95, 0.5).unit(Percent),
    p("Pan", -1.0, 1.0, 0.0).unit(Pan),
];
static WAVETABLE_PARAMS: [ParamSpec; 12] = [
    p("Volume", 0.0, 1.0, 0.5).unit(Gain),
    c("Table", 0.0, WAVETABLES),
    p("Position", 0.0, 1.0, 0.0).unit(Percent),
    p("Sweep", -1.0, 1.0, 0.0),
    i("Unison", 1.0, 7.0, 1.0),
    p("Detune", 0.0, 100.0, 15.0).unit(Cents),
    p("Stereo", 0.0, 1.0, 0.5).unit(Percent),
    p("Attack", 0.0, 2.0, 0.005).unit(Seconds),
    p("Decay", 0.0, 2.0, 0.3).unit(Seconds),
    p("Sustain", 0.0, 1.0, 0.7).unit(Percent),
    p("Release", 0.0, 4.0, 0.3).unit(Seconds),
    p("Pan", -1.0, 1.0, 0.0).unit(Pan),
];
static GRANULAR_PARAMS: [ParamSpec; 11] = [
    p("Volume", 0.0, 1.0, 0.6).unit(Gain),
    p("Position", 0.0, 1.0, 0.0).unit(Percent),
    p("Scan", -2.0, 2.0, 0.25),
    p("Size", 0.01, 0.5, 0.08).unit(Seconds),
    p("Density", 1.0, 200.0, 30.0).unit(Hz),
    p("Spray", 0.0, 0.5, 0.02).unit(Seconds),
    p("Random pitch", 0.0, 100.0, 0.0).unit(Cents),
    p("Stereo", 0.0, 1.0, 0.5).unit(Percent),
    p("Reverse", 0.0, 1.0, 0.0).unit(Percent),
    p("Attack", 0.0, 4.0, 0.05).unit(Seconds),
    p("Release", 0.0, 8.0, 0.5).unit(Seconds),
];
static CONVOLVER_PARAMS: [ParamSpec; 3] =
    [c("Impulse", 0.0, IMPULSES), p("Mix", 0.0, 1.0, 0.3).unit(Percent), p("Gain", 0.0, 2.0, 1.0).unit(Gain)];
static FM_PARAMS: [ParamSpec; 9] = [
    p("Volume", 0.0, 1.0, 0.5).unit(Gain),
    p("Ratio", 0.25, 8.0, 2.0).unit(Ratio),
    p("Mod index", 0.0, 10.0, 2.0),
    p("Mod decay", 0.0, 4.0, 0.4).unit(Seconds),
    p("Feedback", 0.0, 1.0, 0.0).unit(Percent),
    p("Attack", 0.0, 2.0, 0.002).unit(Seconds),
    p("Decay", 0.0, 4.0, 0.6).unit(Seconds),
    p("Sustain", 0.0, 1.0, 0.0).unit(Percent),
    p("Release", 0.0, 4.0, 0.3).unit(Seconds),
];
static DRUM_PARAMS: [ParamSpec; 5] = [
    p("Volume", 0.0, 1.0, 0.7).unit(Gain),
    p("Kick tone", 30.0, 120.0, 50.0).unit(Hz),
    p("Kick decay", 0.05, 1.5, 0.45).unit(Seconds),
    p("Snare tone", 0.0, 1.0, 0.5).unit(Percent),
    p("Hat decay", 0.01, 0.5, 0.06).unit(Seconds),
];
pub const KICKER_WAVES: &[&str] = &["Sine", "Triangle", "Square"];
static KICKER_PARAMS: [ParamSpec; 8] = [
    p("Volume", 0.0, 1.0, 0.7).unit(Gain),
    c("Wave", 0.0, KICKER_WAVES),
    p("Pitch drop", 0.0, 6.0, 3.0).unit(Unit::Octaves),
    p("Sweep", 0.005, 0.5, 0.04).unit(Seconds),
    p("Attack", 0.0, 0.05, 0.0).unit(Seconds),
    p("Decay", 0.05, 3.0, 0.5).unit(Seconds),
    p("Boost", 0.0, 1.0, 0.2).unit(Percent),
    p("Pan", -1.0, 1.0, 0.0).unit(Pan),
];
static SPECTRAVOICE_PARAMS: [ParamSpec; 11] = [
    p("Volume", 0.0, 1.0, 0.4).unit(Gain),
    i("Harmonics", 1.0, 32.0, 12.0),
    p("Slope", 0.0, 3.0, 1.0),
    p("Even", 0.0, 1.0, 1.0).unit(Percent),
    p("Stretch", -0.02, 0.02, 0.0).unit(Percent),
    p("Shimmer", 0.0, 1.0, 0.0).unit(Percent),
    p("Attack", 0.0, 4.0, 0.01).unit(Seconds),
    p("Decay", 0.0, 4.0, 0.3).unit(Seconds),
    p("Sustain", 0.0, 1.0, 0.7).unit(Percent),
    p("Release", 0.0, 8.0, 0.4).unit(Seconds),
    p("Pan", -1.0, 1.0, 0.0).unit(Pan),
];
/// The FMX's algorithms: how its four operators feed each other, as the
/// classic four-operator FM synths have them. `>` is "modulates"; an
/// operator that modulates none is heard.
pub const FMX_ALGORITHMS: &[&str] = &[
    "4 > 3 > 2 > 1",
    "3 + 4 > 2 > 1",
    "4 > 1, 3 > 2 > 1",
    "4 > 3 > 1, 2 > 1",
    "4 > 3, 2 > 1: two stacks",
    "4 > 1, 2, 3",
    "4 > 3, 1, 2",
    "1, 2, 3, 4 side by side",
];
static FMX_PARAMS: [ParamSpec; 27] = [
    p("Volume", 0.0, 1.0, 0.5).unit(Gain),
    c("Algorithm", 0.0, FMX_ALGORITHMS),
    p("Feedback", 0.0, 1.0, 0.0).unit(Percent),
    p("Op1 level", 0.0, 1.0, 1.0).unit(Percent),
    p("Op1 ratio", 0.25, 16.0, 1.0).unit(Ratio),
    p("Op1 attack", 0.0, 4.0, 0.005).unit(Seconds),
    p("Op1 decay", 0.0, 8.0, 1.2).unit(Seconds),
    p("Op1 sustain", 0.0, 1.0, 0.5).unit(Percent),
    p("Op1 release", 0.0, 8.0, 0.5).unit(Seconds),
    p("Op2 level", 0.0, 1.0, 0.45).unit(Percent),
    p("Op2 ratio", 0.25, 16.0, 1.0).unit(Ratio),
    p("Op2 attack", 0.0, 4.0, 0.001).unit(Seconds),
    p("Op2 decay", 0.0, 8.0, 0.8).unit(Seconds),
    p("Op2 sustain", 0.0, 1.0, 0.2).unit(Percent),
    p("Op2 release", 0.0, 8.0, 0.4).unit(Seconds),
    p("Op3 level", 0.0, 1.0, 0.3).unit(Percent),
    p("Op3 ratio", 0.25, 16.0, 2.0).unit(Ratio),
    p("Op3 attack", 0.0, 4.0, 0.001).unit(Seconds),
    p("Op3 decay", 0.0, 8.0, 0.5).unit(Seconds),
    p("Op3 sustain", 0.0, 1.0, 0.1).unit(Percent),
    p("Op3 release", 0.0, 8.0, 0.3).unit(Seconds),
    p("Op4 level", 0.0, 1.0, 0.2).unit(Percent),
    p("Op4 ratio", 0.25, 16.0, 3.0).unit(Ratio),
    p("Op4 attack", 0.0, 4.0, 0.001).unit(Seconds),
    p("Op4 decay", 0.0, 8.0, 0.3).unit(Seconds),
    p("Op4 sustain", 0.0, 1.0, 0.0).unit(Percent),
    p("Op4 release", 0.0, 8.0, 0.2).unit(Seconds),
];
/// Instrument-wide settings; each sample has its own in `SampleSlot`.
static SAMPLER_PARAMS: [ParamSpec; 7] = [
    p("Volume", 0.0, 1.5, 0.8).unit(Gain),
    p("Pan", -1.0, 1.0, 0.0).unit(Pan),
    i("Transpose", -48.0, 48.0, 0.0).unit(Semitones),
    p("Attack", 0.0, 2.0, 0.0).unit(Seconds),
    p("Decay", 0.0, 4.0, 0.5).unit(Seconds),
    p("Sustain", 0.0, 1.0, 1.0).unit(Percent),
    p("Release", 0.0, 4.0, 0.1).unit(Seconds),
];
static FILTER_PARAMS: [ParamSpec; 5] = [
    c("Mode", 0.0, FILTER_MODES),
    p("Cutoff", 20.0, 20000.0, 2000.0).unit(Hz),
    p("Resonance", 0.0, 0.97, 0.3).unit(Percent),
    p("LFO rate", 0.0, 20.0, 0.0).unit(Hz),
    p("LFO depth", 0.0, 1.0, 0.0).unit(Percent),
];
static DISTORTION_PARAMS: [ParamSpec; 6] = [
    p("Drive", 1.0, 50.0, 4.0).unit(Gain),
    p("Tone", 0.0, 1.0, 0.7).unit(Percent),
    p("Mix", 0.0, 1.0, 1.0).unit(Percent),
    c("Type", 0.0, DISTORTION_TYPES),
    i("Bits", 1.0, 16.0, 16.0),
    i("Downsample", 1.0, 32.0, 1.0).unit(Ratio),
];
static DELAY_PARAMS: [ParamSpec; 4] = [
    p("Time", 0.25, 16.0, 3.0).unit(Lines),
    p("Feedback", 0.0, 0.95, 0.4).unit(Percent),
    p("Mix", 0.0, 1.0, 0.3).unit(Percent),
    p("Stereo", 0.0, 1.0, 0.5).unit(Percent),
];
static REVERB_PARAMS: [ParamSpec; 3] = [
    p("Room size", 0.0, 1.0, 0.7).unit(Percent),
    p("Damping", 0.0, 1.0, 0.4).unit(Percent),
    p("Mix", 0.0, 1.0, 0.25).unit(Percent),
];
static AMP_PARAMS: [ParamSpec; 3] =
    [p("Volume", 0.0, 4.0, 1.0).unit(Gain), p("Pan", -1.0, 1.0, 0.0).unit(Pan), c("Invert", 0.0, &["Off", "On"])];
static LFO_PARAMS: [ParamSpec; 6] = [
    c("Mode", 0.0, LFO_MODES),
    c("Shape", 0.0, LFO_SHAPES),
    p("Depth", 0.0, 1.0, 0.5).unit(Percent),
    p("Rate", 0.05, 20.0, 2.0).unit(Hz),
    c("Sync", 0.0, &["Off", "Lines"]),
    p("Period", 0.25, 64.0, 4.0).unit(Lines),
];
static FLANGER_PARAMS: [ParamSpec; 6] = [
    c("Mode", 0.0, FLANGER_MODES),
    p("Delay", 0.0005, 0.03, 0.003).unit(Seconds),
    p("Depth", 0.0, 1.0, 0.5).unit(Percent),
    p("Rate", 0.02, 10.0, 0.3).unit(Hz),
    p("Feedback", -0.95, 0.95, 0.5).unit(Percent),
    p("Mix", 0.0, 1.0, 0.5).unit(Percent),
];
static PHASER_PARAMS: [ParamSpec; 7] = [
    p("Rate", 0.02, 10.0, 0.4).unit(Hz),
    p("Depth", 0.0, 1.0, 0.8).unit(Percent),
    p("Floor", 20.0, 5000.0, 250.0).unit(Hz),
    p("Ceiling", 200.0, 18000.0, 4000.0).unit(Hz),
    i("Stages", 1.0, 12.0, 4.0),
    p("Feedback", -0.95, 0.95, 0.4).unit(Percent),
    p("Mix", 0.0, 1.0, 0.5).unit(Percent),
];
static VOCAL_FILTER_PARAMS: [ParamSpec; 6] = [
    p("Vowel", 0.0, 4.0, 0.0).unit(Unit::Vowel),
    p("Shift", -12.0, 12.0, 0.0).unit(Semitones),
    p("Width", 0.25, 4.0, 1.0).unit(Ratio),
    i("Formants", 1.0, 5.0, 5.0),
    p("Gain", 0.0, 4.0, 2.0).unit(Gain),
    p("Mix", 0.0, 1.0, 1.0).unit(Percent),
];
static REPEATER_PARAMS: [ParamSpec; 3] =
    [c("Hold", 0.0, REPEATER_HOLD), c("Length", 3.0, REPEATER_LENGTHS), p("Mix", 0.0, 1.0, 1.0).unit(Percent)];
static RING_MOD_PARAMS: [ParamSpec; 4] = [
    p("Frequency", 1.0, 5000.0, 440.0).unit(Hz),
    c("Shape", 0.0, LFO_SHAPES),
    p("Stereo", 0.0, 0.5, 0.0).unit(Percent),
    p("Mix", 0.0, 1.0, 1.0).unit(Percent),
];
static GATE_PARAMS: [ParamSpec; 5] = [
    p("Threshold", 0.001, 1.0, 0.05).unit(Gain),
    p("Attack", 0.0001, 0.1, 0.001).unit(Seconds),
    p("Hold", 0.0, 1.0, 0.05).unit(Seconds),
    p("Release", 0.005, 2.0, 0.1).unit(Seconds),
    p("Floor", 0.0, 1.0, 0.0).unit(Gain),
];
static PITCH_SHIFTER_PARAMS: [ParamSpec; 5] = [
    i("Pitch", -24.0, 24.0, 12.0).unit(Semitones),
    p("Finetune", -100.0, 100.0, 0.0).unit(Cents),
    p("Grain", 0.01, 0.2, 0.06).unit(Seconds),
    p("Feedback", 0.0, 0.9, 0.0).unit(Percent),
    p("Mix", 0.0, 1.0, 1.0).unit(Percent),
];
static STEREO_EXPANDER_PARAMS: [ParamSpec; 2] =
    [p("Width", 0.0, 2.0, 1.5).unit(Percent), p("Mono bass", 0.0, 500.0, 0.0).unit(Hz)];
static COMB_FILTER_PARAMS: [ParamSpec; 5] = [
    i("Note", 24.0, 108.0, 57.0).unit(Unit::Note),
    p("Finetune", -100.0, 100.0, 0.0).unit(Cents),
    p("Feedback", -0.98, 0.98, 0.8).unit(Percent),
    p("Damping", 0.0, 1.0, 0.2).unit(Percent),
    p("Mix", 0.0, 1.0, 0.5).unit(Percent),
];
static MAXIMIZER_PARAMS: [ParamSpec; 3] = [
    p("Boost", 1.0, 8.0, 2.0).unit(Gain),
    p("Ceiling", 0.1, 1.0, 0.95).unit(Gain),
    p("Release", 0.005, 1.0, 0.1).unit(Seconds),
];
static EXCITER_PARAMS: [ParamSpec; 3] = [
    p("Frequency", 1000.0, 16000.0, 3000.0).unit(Hz),
    p("Drive", 1.0, 10.0, 3.0).unit(Gain),
    p("Amount", 0.0, 1.0, 0.3).unit(Percent),
];
static DC_BLOCKER_PARAMS: [ParamSpec; 1] = [p("Cutoff", 1.0, 100.0, 10.0).unit(Hz)];
static SCREAM_FILTER_PARAMS: [ParamSpec; 5] = [
    c("Mode", 0.0, FILTER_MODES),
    p("Cutoff", 20.0, 20000.0, 1500.0).unit(Hz),
    p("Resonance", 0.0, 1.0, 0.7).unit(Percent),
    p("Scream", 1.0, 20.0, 4.0).unit(Gain),
    p("Mix", 0.0, 1.0, 1.0).unit(Percent),
];
static MULTITAP_PARAMS: [ParamSpec; 14] = [
    p("Tap 1 time", 0.25, 16.0, 1.0).unit(Lines),
    p("Tap 1 level", 0.0, 1.0, 0.6).unit(Gain),
    p("Tap 1 pan", -1.0, 1.0, -0.6).unit(Pan),
    p("Tap 2 time", 0.25, 16.0, 2.0).unit(Lines),
    p("Tap 2 level", 0.0, 1.0, 0.45).unit(Gain),
    p("Tap 2 pan", -1.0, 1.0, 0.6).unit(Pan),
    p("Tap 3 time", 0.25, 16.0, 3.0).unit(Lines),
    p("Tap 3 level", 0.0, 1.0, 0.3).unit(Gain),
    p("Tap 3 pan", -1.0, 1.0, -0.3).unit(Pan),
    p("Tap 4 time", 0.25, 16.0, 4.0).unit(Lines),
    p("Tap 4 level", 0.0, 1.0, 0.2).unit(Gain),
    p("Tap 4 pan", -1.0, 1.0, 0.3).unit(Pan),
    p("Feedback", 0.0, 0.95, 0.3).unit(Percent),
    p("Mix", 0.0, 1.0, 0.4).unit(Percent),
];
/// The EQ 10's band centers, an octave apart.
pub const EQ10_FREQS: [f32; 10] = [31.25, 62.5, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
const fn band(name: &'static str) -> ParamSpec {
    p(name, 0.25, 4.0, 1.0).unit(Gain)
}
static EQ10_PARAMS: [ParamSpec; 10] = [
    band("31 Hz"),
    band("63 Hz"),
    band("125 Hz"),
    band("250 Hz"),
    band("500 Hz"),
    band("1 kHz"),
    band("2 kHz"),
    band("4 kHz"),
    band("8 kHz"),
    band("16 kHz"),
];
pub const CABINETS: &[&str] = &["Small combo", "British 4x12", "Bass 1x15", "Radio"];
static CABINET_PARAMS: [ParamSpec; 3] =
    [c("Cabinet", 0.0, CABINETS), p("Drive", 1.0, 10.0, 1.5).unit(Gain), p("Mix", 0.0, 1.0, 1.0).unit(Percent)];
static VIBRATO_PARAMS: [ParamSpec; 4] = [
    p("Rate", 0.1, 20.0, 5.0).unit(Hz),
    p("Depth", 0.0, 1.0, 0.3).unit(Percent),
    p("Stereo", 0.0, 0.5, 0.0).unit(Percent),
    p("Mix", 0.0, 1.0, 1.0).unit(Percent),
];
pub const INPUT_CHANNELS: &[&str] = &["Stereo", "Left", "Right", "Mono"];
static INPUT_PARAMS: [ParamSpec; 2] = [p("Volume", 0.0, 4.0, 1.0).unit(Gain), c("Channels", 0.0, INPUT_CHANNELS)];
/// The WaveShaper's curve points: what each of these input levels comes out as.
static WAVESHAPER_PARAMS: [ParamSpec; 13] = [
    p("Input", 0.0, 4.0, 1.0).unit(Gain),
    c("Symmetric", 1.0, &["Off", "On"]),
    p("Output", 0.0, 2.0, 1.0).unit(Gain),
    p("Mix", 0.0, 1.0, 1.0).unit(Percent),
    p("At -100%", -1.0, 1.0, -1.0),
    p("At -75%", -1.0, 1.0, -0.75),
    p("At -50%", -1.0, 1.0, -0.5),
    p("At -25%", -1.0, 1.0, -0.25),
    p("At 0%", -1.0, 1.0, 0.0),
    p("At +25%", -1.0, 1.0, 0.25),
    p("At +50%", -1.0, 1.0, 0.5),
    p("At +75%", -1.0, 1.0, 0.75),
    p("At +100%", -1.0, 1.0, 1.0),
];
/// The Filter Pro's types, in the order of `Shape::ALL`.
pub const FILTER_PRO_TYPES: &[&str] =
    &["Lowpass", "Highpass", "Bandpass", "Notch", "Allpass", "Peak", "Low shelf", "High shelf"];
pub const FILTER_SLOPES: &[&str] = &["12 dB", "24 dB", "36 dB", "48 dB"];
static FILTER_PRO_PARAMS: [ParamSpec; 6] = [
    c("Type", 0.0, FILTER_PRO_TYPES),
    p("Frequency", 20.0, 20000.0, 1000.0).unit(Hz),
    p("Resonance", 0.3, 20.0, 0.707),
    p("Gain", 0.25, 4.0, 1.0).unit(Gain),
    c("Slope", 0.0, FILTER_SLOPES),
    p("Mix", 0.0, 1.0, 1.0).unit(Percent),
];
static CHORUS_PARAMS: [ParamSpec; 7] = [
    i("Voices", 1.0, 4.0, 3.0),
    p("Rate", 0.05, 5.0, 0.8).unit(Hz),
    p("Depth", 0.0, 1.0, 0.5).unit(Percent),
    p("Delay", 0.005, 0.04, 0.015).unit(Seconds),
    p("Stereo", 0.0, 0.5, 0.25).unit(Percent),
    p("Feedback", 0.0, 0.8, 0.0).unit(Percent),
    p("Mix", 0.0, 1.0, 0.5).unit(Percent),
];
static ECHO_PARAMS: [ParamSpec; 5] = [
    p("Time", 0.01, 2.0, 0.3).unit(Seconds),
    p("Feedback", 0.0, 0.95, 0.5).unit(Percent),
    p("Damping", 0.0, 1.0, 0.3).unit(Percent),
    p("Stereo", 0.0, 1.0, 0.3).unit(Percent),
    p("Mix", 0.0, 1.0, 0.35).unit(Percent),
];
pub const LADDER_TYPES: &[&str] = &["Lowpass 24 dB", "Lowpass 12 dB", "Bandpass", "Highpass 24 dB"];
static ANALOG_FILTER_PARAMS: [ParamSpec; 5] = [
    c("Type", 0.0, LADDER_TYPES),
    p("Cutoff", 20.0, 18000.0, 1200.0).unit(Hz),
    p("Resonance", 0.0, 1.0, 0.3).unit(Percent),
    p("Drive", 1.0, 10.0, 1.0).unit(Gain),
    p("Mix", 0.0, 1.0, 1.0).unit(Percent),
];
static PLATE_PARAMS: [ParamSpec; 5] = [
    p("Decay", 0.0, 0.98, 0.6).unit(Percent),
    p("Predelay", 0.0, 0.2, 0.02).unit(Seconds),
    p("Damping", 0.0, 1.0, 0.3).unit(Percent),
    p("Width", 0.0, 1.0, 1.0).unit(Percent),
    p("Mix", 0.0, 1.0, 0.3).unit(Percent),
];
static EQ5_PARAMS: [ParamSpec; 15] = [
    p("Low freq", 20.0, 20000.0, 100.0).unit(Hz),
    p("Low gain", 0.25, 4.0, 1.0).unit(Gain),
    p("Low Q", 0.3, 10.0, 0.9),
    p("Low mid freq", 20.0, 20000.0, 400.0).unit(Hz),
    p("Low mid gain", 0.25, 4.0, 1.0).unit(Gain),
    p("Low mid Q", 0.3, 10.0, 0.9),
    p("Mid freq", 20.0, 20000.0, 1200.0).unit(Hz),
    p("Mid gain", 0.25, 4.0, 1.0).unit(Gain),
    p("Mid Q", 0.3, 10.0, 0.9),
    p("High mid freq", 20.0, 20000.0, 3500.0).unit(Hz),
    p("High mid gain", 0.25, 4.0, 1.0).unit(Gain),
    p("High mid Q", 0.3, 10.0, 0.9),
    p("High freq", 20.0, 20000.0, 9000.0).unit(Hz),
    p("High gain", 0.25, 4.0, 1.0).unit(Gain),
    p("High Q", 0.3, 10.0, 0.9),
];
static COMPRESSOR_PARAMS: [ParamSpec; 6] = [
    p("Threshold", 0.01, 1.0, 0.25).unit(Gain),
    p("Ratio", 1.0, 20.0, 4.0).unit(Ratio),
    p("Attack", 0.0001, 0.2, 0.005).unit(Seconds),
    p("Release", 0.01, 2.0, 0.15).unit(Seconds),
    p("Makeup", 1.0, 8.0, 1.0).unit(Gain),
    p("Mix", 0.0, 1.0, 1.0).unit(Percent),
];
static EQ_PARAMS: [ParamSpec; 6] = [
    p("Low", 0.0, 4.0, 1.0).unit(Gain),
    p("Mid", 0.0, 4.0, 1.0).unit(Gain),
    p("High", 0.0, 4.0, 1.0).unit(Gain),
    p("Low freq", 20.0, 2000.0, 200.0).unit(Hz),
    p("Mid freq", 100.0, 10000.0, 1000.0).unit(Hz),
    p("High freq", 500.0, 20000.0, 5000.0).unit(Hz),
];
static MULTISYNTH_PARAMS: [ParamSpec; 8] = [
    c("Mode", 0.0, MULTISYNTH_MODES),
    i("Transpose", -48.0, 48.0, 0.0).unit(Semitones),
    p("Finetune", -100.0, 100.0, 0.0).unit(Cents),
    p("Random pitch", 0.0, 100.0, 0.0).unit(Cents),
    p("Velocity", 0.0, 2.0, 1.0).unit(Percent),
    p("Random velocity", 0.0, 1.0, 0.0).unit(Percent),
    i("Low note", 0.0, 119.0, 0.0).unit(Unit::Note),
    i("High note", 0.0, 119.0, 119.0).unit(Unit::Note),
];
static GLIDE_PARAMS: [ParamSpec; 2] = [c("Mode", 0.0, GLIDE_MODES), p("Time", 0.005, 2.0, 0.12).unit(Seconds)];
static MODULATOR_PARAMS: [ParamSpec; 9] = [
    c("Mode", 0.0, MODULATOR_MODES),
    c("Shape", 0.0, MODULATOR_SHAPES),
    p("Rate", 0.05, 20.0, 1.0).unit(Hz),
    c("Sync", 0.0, MODULATOR_SYNCS),
    p("Period", 0.25, 64.0, 16.0).unit(Lines),
    p("Amount", -1.0, 1.0, 0.5).unit(Percent),
    p("Attack", 0.001, 1.0, 0.01).unit(Seconds),
    p("Release", 0.01, 2.0, 0.2).unit(Seconds),
    c("Beats", 4.0, MODULATOR_BEATS),
];
/// The pan control of a mixer strip.
pub static MIXER_PAN: ParamSpec = p("Pan", -1.0, 1.0, 0.0).unit(Pan);
/// The fader of a mixer strip, as automation sees it.
pub static MIXER_GAIN: ParamSpec = p("Fader", 0.0, 2.0, 1.0).unit(Gain);

impl ModuleKind {
    pub const ADDABLE: [ModuleKind; 46] = [
        ModuleKind::Generator,
        ModuleKind::Wavetable,
        ModuleKind::Fm,
        ModuleKind::Drums,
        ModuleKind::Kicker,
        ModuleKind::SpectraVoice,
        ModuleKind::Fmx,
        ModuleKind::Sampler,
        ModuleKind::Granular,
        ModuleKind::Input,
        ModuleKind::MultiSynth,
        ModuleKind::Glide,
        ModuleKind::Modulator,
        ModuleKind::Filter,
        ModuleKind::Distortion,
        ModuleKind::Delay,
        ModuleKind::Reverb,
        ModuleKind::Amplifier,
        ModuleKind::Lfo,
        ModuleKind::Flanger,
        ModuleKind::Phaser,
        ModuleKind::VocalFilter,
        ModuleKind::Repeater,
        ModuleKind::RingMod,
        ModuleKind::Gate,
        ModuleKind::PitchShifter,
        ModuleKind::StereoExpander,
        ModuleKind::CombFilter,
        ModuleKind::Maximizer,
        ModuleKind::Exciter,
        ModuleKind::DcBlocker,
        ModuleKind::ScreamFilter,
        ModuleKind::Multitap,
        ModuleKind::Eq10,
        ModuleKind::Cabinet,
        ModuleKind::Vibrato,
        ModuleKind::WaveShaper,
        ModuleKind::FilterPro,
        ModuleKind::Chorus,
        ModuleKind::Echo,
        ModuleKind::AnalogFilter,
        ModuleKind::PlateReverb,
        ModuleKind::Convolver,
        ModuleKind::Eq5,
        ModuleKind::Compressor,
        ModuleKind::Eq,
    ];

    /// The kind's name and its parameters.
    fn info(self) -> (&'static str, &'static [ParamSpec]) {
        match self {
            ModuleKind::Output => ("Output", &OUTPUT_PARAMS),
            ModuleKind::Generator => ("Generator", &GENERATOR_PARAMS),
            ModuleKind::Wavetable => ("Wavetable", &WAVETABLE_PARAMS),
            ModuleKind::Fm => ("FM", &FM_PARAMS),
            ModuleKind::Drums => ("Drums", &DRUM_PARAMS),
            ModuleKind::Sampler => ("Sampler", &SAMPLER_PARAMS),
            ModuleKind::Granular => ("Granular", &GRANULAR_PARAMS),
            ModuleKind::Filter => ("Filter", &FILTER_PARAMS),
            ModuleKind::Distortion => ("Distortion", &DISTORTION_PARAMS),
            ModuleKind::Delay => ("Delay", &DELAY_PARAMS),
            ModuleKind::Reverb => ("Reverb", &REVERB_PARAMS),
            ModuleKind::Amplifier => ("Amplifier", &AMP_PARAMS),
            ModuleKind::Lfo => ("LFO", &LFO_PARAMS),
            ModuleKind::Flanger => ("Flanger", &FLANGER_PARAMS),
            ModuleKind::Phaser => ("Phaser", &PHASER_PARAMS),
            ModuleKind::VocalFilter => ("Vocal Filter", &VOCAL_FILTER_PARAMS),
            ModuleKind::Repeater => ("Repeater", &REPEATER_PARAMS),
            ModuleKind::RingMod => ("Ring Mod", &RING_MOD_PARAMS),
            ModuleKind::Gate => ("Gate", &GATE_PARAMS),
            ModuleKind::Kicker => ("Kicker", &KICKER_PARAMS),
            ModuleKind::SpectraVoice => ("SpectraVoice", &SPECTRAVOICE_PARAMS),
            ModuleKind::Fmx => ("FMX", &FMX_PARAMS),
            ModuleKind::PitchShifter => ("Pitch Shifter", &PITCH_SHIFTER_PARAMS),
            ModuleKind::StereoExpander => ("Stereo Expander", &STEREO_EXPANDER_PARAMS),
            ModuleKind::CombFilter => ("Comb Filter", &COMB_FILTER_PARAMS),
            ModuleKind::Maximizer => ("Maximizer", &MAXIMIZER_PARAMS),
            ModuleKind::Exciter => ("Exciter", &EXCITER_PARAMS),
            ModuleKind::DcBlocker => ("DC Blocker", &DC_BLOCKER_PARAMS),
            ModuleKind::ScreamFilter => ("Scream Filter", &SCREAM_FILTER_PARAMS),
            ModuleKind::Multitap => ("Multitap Delay", &MULTITAP_PARAMS),
            ModuleKind::Eq10 => ("EQ 10", &EQ10_PARAMS),
            ModuleKind::Cabinet => ("Cabinet Simulator", &CABINET_PARAMS),
            ModuleKind::Vibrato => ("Vibrato", &VIBRATO_PARAMS),
            ModuleKind::Input => ("Input", &INPUT_PARAMS),
            ModuleKind::WaveShaper => ("WaveShaper", &WAVESHAPER_PARAMS),
            ModuleKind::FilterPro => ("Filter Pro", &FILTER_PRO_PARAMS),
            ModuleKind::Chorus => ("Chorus", &CHORUS_PARAMS),
            ModuleKind::Echo => ("Echo", &ECHO_PARAMS),
            ModuleKind::AnalogFilter => ("Analog Filter", &ANALOG_FILTER_PARAMS),
            ModuleKind::PlateReverb => ("Plate Reverb", &PLATE_PARAMS),
            ModuleKind::Convolver => ("Convolver", &CONVOLVER_PARAMS),
            ModuleKind::Eq5 => ("EQ 5", &EQ5_PARAMS),
            ModuleKind::Compressor => ("Compressor", &COMPRESSOR_PARAMS),
            ModuleKind::Eq => ("EQ", &EQ_PARAMS),
            ModuleKind::MultiSynth => ("MultiSynth", &MULTISYNTH_PARAMS),
            ModuleKind::Glide => ("Glide", &GLIDE_PARAMS),
            ModuleKind::Modulator => ("Modulator", &MODULATOR_PARAMS),
        }
    }

    pub fn name(self) -> &'static str {
        self.info().0
    }

    pub fn params(self) -> &'static [ParamSpec] {
        self.info().1
    }

    /// What envelopes can move: the module's parameters, then its mixer
    /// strip's fader and pan.
    pub fn automatable(self, i: usize) -> Option<&'static ParamSpec> {
        let n = self.params().len();
        match i {
            i if i < n => Some(&self.params()[i]),
            i if i == n => Some(&MIXER_GAIN),
            i if i == n + 1 => Some(&MIXER_PAN),
            _ => None,
        }
    }

    /// How many parameters `automatable` knows.
    pub fn num_automatable(self) -> usize {
        self.params().len() + 2
    }

    /// Whether the module plays notes (as opposed to processing audio).
    pub fn is_instrument(self) -> bool {
        matches!(
            self,
            ModuleKind::Generator
                | ModuleKind::Wavetable
                | ModuleKind::Fm
                | ModuleKind::Drums
                | ModuleKind::Kicker
                | ModuleKind::SpectraVoice
                | ModuleKind::Fmx
                | ModuleKind::Sampler
                | ModuleKind::Granular
                | ModuleKind::MultiSynth
                | ModuleKind::Glide
        )
    }

    /// Whether the module plays samples the instrument editor loads and
    /// edits.
    pub fn holds_samples(self) -> bool {
        matches!(self, ModuleKind::Sampler | ModuleKind::Granular)
    }

    /// Whether the module's voices take a `Modulation`.
    pub fn has_modulation(self) -> bool {
        matches!(self, ModuleKind::Sampler | ModuleKind::Generator | ModuleKind::Fm | ModuleKind::Wavetable)
    }

    /// Whether the module only passes notes on and makes no sound, as the
    /// MultiSynth does.
    pub fn notes_only(self) -> bool {
        matches!(self, ModuleKind::MultiSynth | ModuleKind::Glide)
    }

    /// Whether the module's links move other modules' parameters rather
    /// than carry sound.
    pub fn controls(self) -> bool {
        self == ModuleKind::Modulator
    }

    /// Whether the module outputs sound, and so has a mixer strip.
    pub fn makes_sound(self) -> bool {
        !self.notes_only() && !self.controls()
    }

    pub fn has_input(self) -> bool {
        !self.is_instrument() && self != ModuleKind::Input
    }

    pub fn has_output(self) -> bool {
        self != ModuleKind::Output
    }
}

/// An envelope a Sampler's voices run through from each note, in seconds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceEnvelope {
    pub on: bool,
    /// Points as (seconds since the note started, value 0..1), sorted.
    pub points: Vec<(f32, f32)>,
    /// The point the envelope holds at while the note is held.
    pub sustain: Option<usize>,
    pub curve: bool,
    /// How far the envelope moves its target: semitones for pitch,
    /// octaves for the filter.
    pub amount: f32,
}

impl Default for VoiceEnvelope {
    fn default() -> Self {
        VoiceEnvelope::flat(0.5, 12.0)
    }
}

impl Default for VoiceLfo {
    fn default() -> Self {
        VoiceLfo::new(0.3)
    }
}

impl VoiceEnvelope {
    fn flat(value: f32, amount: f32) -> Self {
        VoiceEnvelope { on: false, points: vec![(0.0, value)], sustain: None, curve: false, amount }
    }

    /// The value `t` seconds into the note; `advance` keeps `t` at the
    /// sustain point while the note is held.
    pub fn value(&self, t: f32) -> f32 {
        interpolate(&self.points, t, false, self.curve).unwrap_or(0.0)
    }

    /// Where a note `t` seconds in moves to after `dt` more seconds.
    pub fn advance(&self, t: f32, dt: f32, held: bool) -> f32 {
        match self.sustain.and_then(|i| self.points.get(i)) {
            Some(&(hold, _)) if held && t <= hold => (t + dt).min(hold),
            _ => t + dt,
        }
    }

    /// The time of the last point.
    pub fn length(&self) -> f32 {
        self.points.last().map_or(0.0, |p| p.0)
    }
}

/// A low-frequency oscillator on each voice.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceLfo {
    pub on: bool,
    /// Index into `LFO_SHAPES`.
    pub shape: u8,
    pub rate: f32,
    /// Semitones for vibrato, 0..1 of the volume for tremolo.
    pub depth: f32,
    /// Seconds before it starts, fading in over as long again.
    pub delay: f32,
}

impl VoiceLfo {
    fn new(depth: f32) -> Self {
        VoiceLfo { on: false, shape: 0, rate: 5.0, depth, delay: 0.0 }
    }
}

/// How a Sampler's voices change while they play: a pitch envelope, a
/// filter with its own envelope, vibrato and tremolo.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Modulation {
    pub pitch: VoiceEnvelope,
    pub filter: bool,
    /// Index into `FILTER_MODES`.
    pub filter_mode: u8,
    pub cutoff: f32,
    pub resonance: f32,
    pub filter_env: VoiceEnvelope,
    pub vibrato: VoiceLfo,
    pub tremolo: VoiceLfo,
}

impl Default for Modulation {
    fn default() -> Self {
        Modulation {
            pitch: VoiceEnvelope::flat(0.5, 12.0),
            filter: false,
            filter_mode: 0,
            cutoff: 2000.0,
            resonance: 0.3,
            filter_env: VoiceEnvelope { points: vec![(0.0, 1.0), (0.4, 0.3)], ..VoiceEnvelope::flat(0.0, 3.0) },
            vibrato: VoiceLfo::new(0.3),
            tremolo: VoiceLfo::new(0.5),
        }
    }
}

impl Modulation {
    pub fn is_default(&self) -> bool {
        *self == Modulation::default()
    }

    /// Keeps hand-edited values in range.
    fn normalize(&mut self) {
        for env in [&mut self.pitch, &mut self.filter_env] {
            for p in &mut env.points {
                *p = (p.0.clamp(0.0, 60.0), p.1.clamp(0.0, 1.0));
            }
            env.points.sort_by(|a, b| a.0.total_cmp(&b.0));
            if env.points.is_empty() {
                env.points.push((0.0, 0.5));
            }
            env.sustain = env.sustain.filter(|&i| i < env.points.len());
        }
        self.pitch.amount = self.pitch.amount.clamp(0.0, 48.0);
        self.filter_env.amount = self.filter_env.amount.clamp(-8.0, 8.0);
        self.filter_mode = self.filter_mode.min(FILTER_MODES.len() as u8 - 1);
        self.cutoff = self.cutoff.clamp(20.0, 20000.0);
        self.resonance = self.resonance.clamp(0.0, 0.97);
        for lfo in [&mut self.vibrato, &mut self.tremolo] {
            lfo.shape = lfo.shape.min(LFO_SHAPES.len() as u8 - 1);
            lfo.rate = lfo.rate.clamp(0.05, 20.0);
            lfo.delay = lfo.delay.clamp(0.0, 10.0);
        }
        self.vibrato.depth = self.vibrato.depth.clamp(0.0, 12.0);
        self.tremolo.depth = self.tremolo.depth.clamp(0.0, 1.0);
    }
}

/// A phrase: a short pattern of its own that an instrument
/// plays when it gets a note, transposed so its C-4 is the note played.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Phrase {
    pub lines: usize,
    /// Lines per beat of the phrase, at the song's BPM.
    pub lpb: u32,
    /// Start again after the last line, while the note is held.
    pub looping: bool,
    /// One cell a line; `MAX_PHRASE_LINES` of them.
    pub cells: Vec<Cell>,
    /// The notes that play this phrase in `PhraseMode::Keymap`, lowest
    /// and highest.
    pub keys: [u8; 2],
}

/// The single phrase of songs saved before instruments had several, and
/// whether it played.
#[derive(Clone, Debug, Deserialize)]
struct OldPhrase {
    #[serde(default)]
    on: bool,
    #[serde(flatten)]
    phrase: Phrase,
}

/// How an instrument's notes pick a phrase. `Zxx` in the pattern picks one
/// for a note whatever the mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhraseMode {
    /// Notes play the instrument itself.
    #[default]
    Off,
    /// Every note plays the selected phrase.
    Program,
    /// A note plays the phrase whose keys hold it, or the instrument
    /// itself outside them.
    Keymap,
}

impl PhraseMode {
    pub const ALL: [PhraseMode; 3] = [PhraseMode::Off, PhraseMode::Program, PhraseMode::Keymap];

    pub fn name(self) -> &'static str {
        match self {
            PhraseMode::Off => "Off",
            PhraseMode::Program => "Program",
            PhraseMode::Keymap => "Keymap",
        }
    }

    fn is_off(&self) -> bool {
        *self == PhraseMode::Off
    }
}

/// The most phrases an instrument has, as `Zxx` can pick from.
pub const MAX_PHRASES: usize = 0x7E;

/// The effect command that picks a phrase, `Z`: base 36 digits write the
/// commands, so this one shows as a letter after the hex ones.
pub const FX_PHRASE: u8 = 35;

/// The effect command that pans the note to and fro, `N`, an auto-pan:
/// speed x, depth y.
pub const FX_AUTOPAN: u8 = 23;

/// The effect command that breaks off the pattern, `J`: the song goes
/// on at line xx of the next slot.
pub const FX_BREAK: u8 = 19;

/// The effect command that sets the track's volume, `L`: 00 silent to
/// 80 full, for every note the track plays until it changes again.
pub const FX_TRACK_VOLUME: u8 = 21;

/// The effect command that plays the sample backwards, `R`: from where
/// it is, or from its end with a note on the line; R00 plays forwards.
pub const FX_REVERSE: u8 = 27;

/// The effect command that plays a slice, `S`: slice xx of the sample
/// the note plays (00 the first), at the note's pitch.
pub const FX_SLICE: u8 = 28;

/// The effect command that stutters the note, `T`, a tremor: on for x
/// ticks, off for y.
pub const FX_TREMOR: u8 = 29;

/// The effect command that holds the song on its line, `W`: for xx
/// lines more, while the line's effects go on.
pub const FX_WAIT: u8 = 32;

/// The effect command letters after the hex digits, by key.
pub fn fx_letter(c: char) -> Option<u8> {
    match c.to_ascii_uppercase() {
        'J' => Some(FX_BREAK),
        'L' => Some(FX_TRACK_VOLUME),
        'N' => Some(FX_AUTOPAN),
        'R' => Some(FX_REVERSE),
        'S' => Some(FX_SLICE),
        'T' => Some(FX_TREMOR),
        'W' => Some(FX_WAIT),
        'Y' => Some(FX_MAYBE),
        'Z' => Some(FX_PHRASE),
        _ => None,
    }
}

/// The volume column's commands, by letter, kept above the volumes (00
/// to 80): sixteen values each from 90 up, the letter's hex digit after it.
pub const VOL_COMMANDS: [char; 7] = ['I', 'O', 'U', 'D', 'G', 'C', 'R'];

/// Where the volume column's commands start.
const VOL_COMMAND_BASE: u8 = 0x90;

/// The volume column command `v` holds, as its letter and digit, if it
/// holds one rather than a volume.
pub fn vol_command(v: u8) -> Option<(char, u8)> {
    let i = v.checked_sub(VOL_COMMAND_BASE)? as usize / 16;
    Some((*VOL_COMMANDS.get(i)?, v & 0xF))
}

/// The volume column value of command `letter` with digit `x`.
pub fn vol_command_value(letter: char, x: u8) -> Option<u8> {
    let i = VOL_COMMANDS.iter().position(|&c| c == letter.to_ascii_uppercase())?;
    Some(VOL_COMMAND_BASE + 16 * i as u8 + (x & 0xF))
}

/// A volume column value as the editor shows it: two hex digits, or a
/// command's letter and digit.
pub fn vol_text(v: u8) -> String {
    match vol_command(v) {
        Some((c, x)) => format!("{c}{x:X}"),
        None => format!("{v:02X}"),
    }
}

/// Reads what `vol_text` writes.
pub fn parse_vol(s: &str) -> Option<u8> {
    let mut chars = s.chars();
    let (a, b) = (chars.next()?, chars.next()?);
    if chars.next().is_some() {
        return None;
    }
    match (vol_command_value(a, 0), b.to_digit(16)) {
        (Some(base), Some(x)) => Some(base + x as u8),
        _ => u8::from_str_radix(s, 16).ok().map(|v| v.min(0x80)),
    }
}

/// The effect command a volume column command stands for: fades are
/// volume slides, pitch slides and glides move x/4 semitone a tick, and
/// cuts and retriggers count x ticks.
pub fn vol_effect(v: u8) -> Option<(u8, u8)> {
    let (c, x) = vol_command(v)?;
    Some(match c {
        'I' => (0xA, x << 4),
        'O' => (0xA, x),
        'U' => (0x1, x * 4),
        'D' => (0x2, x * 4),
        'G' => (0x3, x * 4),
        'C' => (0xC, x),
        _ => (0xE, x),
    })
}

/// The effect command that plays a line's note only sometimes, `Y`: with
/// a chance of xx in FF.
pub const FX_MAYBE: u8 = 34;

/// The longest phrase.
pub const MAX_PHRASE_LINES: usize = 64;

impl Default for Phrase {
    fn default() -> Self {
        Phrase { lines: 16, lpb: 4, looping: false, cells: vec![Cell::default(); MAX_PHRASE_LINES], keys: [0, 119] }
    }
}

impl Phrase {
    pub fn is_default(&self) -> bool {
        *self == Phrase::default()
    }

    fn normalize(&mut self) {
        self.lines = self.lines.clamp(1, MAX_PHRASE_LINES);
        self.lpb = self.lpb.clamp(1, 32);
        self.cells.resize(MAX_PHRASE_LINES, Cell::default());
        self.keys[1] = self.keys[1].min(119);
        self.keys[0] = self.keys[0].min(self.keys[1]);
    }

    /// Whether note `note` plays this phrase in `PhraseMode::Keymap`.
    pub fn has_key(&self, note: f32) -> bool {
        let n = note.round();
        n >= self.keys[0] as f32 && n <= self.keys[1] as f32
    }
}

/// The controls of the Sampler's modulation, for its editor.
pub static PITCH_AMOUNT: ParamSpec = i("Range", 0.0, 48.0, 12.0).unit(Unit::Semitones);
pub static FILTER_CUTOFF: ParamSpec = p("Cutoff", 20.0, 20000.0, 2000.0).unit(Unit::Hz);
pub static FILTER_RESONANCE: ParamSpec = p("Resonance", 0.0, 0.97, 0.3).unit(Unit::Percent);
pub static FILTER_ENV_AMOUNT: ParamSpec = p("Env amount", -8.0, 8.0, 3.0).unit(Unit::Octaves);
pub static LFO_RATE: ParamSpec = p("Rate", 0.05, 20.0, 5.0).unit(Unit::Hz);
pub static LFO_DELAY: ParamSpec = p("Delay", 0.0, 10.0, 0.0).unit(Unit::Seconds);
pub static VIBRATO_DEPTH: ParamSpec = p("Depth", 0.0, 12.0, 0.3).unit(Unit::Semitones);
pub static TREMOLO_DEPTH: ParamSpec = p("Depth", 0.0, 1.0, 0.5).unit(Unit::Percent);

/// How one slice of a sliced sample plays, on top of the sample's own
/// settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SliceSettings {
    /// Linear gain.
    pub volume: f32,
    pub panning: f32,
    pub transpose: i32,
    pub finetune: i32,
    /// Index into `LOOP_MODES`; the loop is the whole slice.
    pub loop_mode: u8,
    pub oneshot: bool,
}

impl Default for SliceSettings {
    fn default() -> Self {
        SliceSettings { volume: 1.0, panning: 0.0, transpose: 0, finetune: 0, loop_mode: 0, oneshot: false }
    }
}

/// One sample of a Sampler, with its own playback settings and keyzone.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SampleSlot {
    pub name: String,
    /// Where the audio is stored. Relative paths are resolved against the
    /// song file's folder.
    pub path: Option<String>,
    /// Linear gain, 0..4.
    pub volume: f32,
    pub panning: f32,
    /// Semitones.
    pub transpose: i32,
    /// Cents.
    pub finetune: i32,
    /// The note that plays the sample at its original pitch.
    pub base_note: u8,
    /// Index into `LOOP_MODES`.
    pub loop_mode: u8,
    /// Loop range in frames, end exclusive.
    pub loop_start: usize,
    pub loop_end: usize,
    /// The notes (inclusive) that play this sample.
    pub keys: [u8; 2],
    /// The velocities (inclusive, 0..=127) that play this sample.
    pub velocities: [u8; 2],
    /// Play the whole sample in this many lines at the song's tempo, with
    /// notes still moving its pitch; 0 is off. Beat sync.
    pub beat_sync: u16,
    /// Note-offs don't stop the sample.
    pub oneshot: bool,
    /// Starting a sample stops the others in the same group (1..=15) of
    /// this Sampler, as a closed hi-hat cuts an open one; 0 is none.
    pub mute_group: u8,
    /// Starting the song partway through plays the sample from where it
    /// would be by then, rather than not at all, for long samples such as
    /// loops and vocals.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub autoseek: bool,
    /// Slice markers in frames: the sample then plays
    /// whole on its base note only, and the slices from each marker to the
    /// next on the notes after it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub slices: Vec<usize>,
    /// The settings of each slice, in order; missing ones are defaults.
    #[serde(skip_serializing_if = "all_default")]
    pub slice_settings: Vec<SliceSettings>,
    /// The audio. Shared, so cloning a project stays cheap.
    #[serde(skip)]
    pub data: Option<Arc<Sample>>,
    /// The audio was edited and has to be written out on save.
    #[serde(skip)]
    pub unsaved: bool,
}

impl Default for SampleSlot {
    fn default() -> Self {
        Self {
            name: "Sample".into(),
            path: None,
            volume: 1.0,
            panning: 0.0,
            transpose: 0,
            finetune: 0,
            base_note: 48,
            loop_mode: 0,
            loop_start: 0,
            loop_end: 0,
            keys: [0, 119],
            velocities: [0, 127],
            beat_sync: 0,
            oneshot: false,
            mute_group: 0,
            autoseek: false,
            slices: Vec::new(),
            slice_settings: Vec::new(),
            data: None,
            unsaved: false,
        }
    }
}

impl SampleSlot {
    pub fn new(sample: Sample, path: Option<String>) -> Self {
        Self {
            name: sample.name.clone(),
            path,
            loop_end: sample.len(),
            data: Some(Arc::new(sample)),
            ..Default::default()
        }
    }

    pub fn len(&self) -> usize {
        self.data.as_ref().map_or(0, |d| d.len())
    }

    /// The frames each slice plays, `start..end`: from the start of the
    /// sample, or a marker, to the next marker or the end. Empty without
    /// markers.
    pub fn slice_ranges(&self) -> Vec<(usize, usize)> {
        if self.slices.is_empty() {
            return Vec::new();
        }
        let len = self.len();
        let mut bounds: Vec<usize> = std::iter::once(0).chain(self.slices.iter().copied()).collect();
        bounds.push(len);
        bounds.windows(2).map(|w| (w[0], w[1])).filter(|r| r.0 < r.1).collect()
    }

    /// How slice `i` plays.
    pub fn slice(&self, i: usize) -> SliceSettings {
        self.slice_settings.get(i).cloned().unwrap_or_default()
    }

    /// Changes how slice `i` plays.
    pub fn slice_mut(&mut self, i: usize) -> &mut SliceSettings {
        if self.slice_settings.len() <= i {
            self.slice_settings.resize(i + 1, SliceSettings::default());
        }
        &mut self.slice_settings[i]
    }

    /// Adds a marker at `frame`, the slice it splits handing its settings
    /// to both halves, so the slices after it keep theirs.
    pub fn add_slice(&mut self, frame: usize) {
        if frame == 0 || frame >= self.len() || self.slices.contains(&frame) {
            return;
        }
        let k = self.slices.iter().filter(|&&m| m < frame).count();
        if k < self.slice_settings.len() {
            let split = self.slice_settings[k].clone();
            self.slice_settings.insert(k + 1, split);
        }
        self.slices.push(frame);
        self.clamp_slices();
    }

    /// Removes marker `n`, joining the slices on either side; the joined
    /// slice keeps the first one's settings.
    pub fn remove_slice(&mut self, n: usize) {
        if n < self.slices.len() {
            self.slices.remove(n);
            if n + 1 < self.slice_settings.len() {
                self.slice_settings.remove(n + 1);
            }
        }
    }

    /// The note slice `i` plays on.
    pub fn slice_note(&self, i: usize) -> u8 {
        (self.base_note as usize + 1 + i).min(119) as u8
    }

    /// Keeps the slice markers sorted, apart and inside the sample.
    pub fn clamp_slices(&mut self) {
        let len = self.len();
        self.slices.retain(|&s| s > 0 && (len == 0 || s < len));
        self.slices.sort_unstable();
        self.slices.dedup();
        // The notes after the base note run out at B-9.
        self.slices.truncate(119usize.saturating_sub(self.base_note as usize + 1));
        let n = if self.slices.is_empty() { 0 } else { self.slices.len() + 1 };
        self.slice_settings.truncate(n);
    }

    /// Keeps the loop inside the sample and at least one frame long.
    pub fn clamp_loop(&mut self) {
        let len = self.len();
        if len == 0 {
            return;
        }
        if self.loop_end == 0 || self.loop_end > len {
            self.loop_end = len;
        }
        self.loop_start = self.loop_start.min(self.loop_end - 1);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Module {
    pub id: u8,
    pub kind: ModuleKind,
    pub name: String,
    pub params: Vec<f32>,
    /// Where the module sat in the module graph of earlier versions; kept
    /// so songs open and save as they were.
    pub pos: [f32; 2],
    #[serde(default)]
    pub mute: bool,
    /// Mixer settings, applied after the module's own processing: the
    /// fader as a linear gain (0..2), panning (-1..1) and solo.
    #[serde(default = "unity")]
    pub gain: f32,
    #[serde(default)]
    pub pan: f32,
    #[serde(default)]
    pub solo: bool,
    /// An effect switched off: its input goes straight through.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bypass: bool,
    /// `None` for the color of its kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
    /// A Modulator's targets: which automatable parameter (see
    /// `ModuleKind::automatable`) of each module it links to it moves.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<(u8, usize)>,
    /// A Sampler's samples.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub samples: Vec<SampleSlot>,
    /// A Modulator's drawn LFO shape, as (where in the cycle 0..1, value
    /// 0..1); empty for `DEFAULT_SHAPE`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shape: Vec<(f32, f32)>,
    /// An instrument's phrases, how its notes pick one, and the selected
    /// one, which `PhraseMode::Program` plays and the Phrase page shows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phrases: Vec<Phrase>,
    #[serde(default, skip_serializing_if = "PhraseMode::is_off")]
    pub phrase_mode: PhraseMode,
    #[serde(default, skip_serializing_if = "is_zero_index")]
    pub selected_phrase: usize,
    /// The single phrase of songs saved before instruments had several;
    /// read when loading and turned into the first.
    #[serde(default, rename = "phrase", skip_serializing)]
    old_phrase: Option<OldPhrase>,
    /// The pitch and filter envelopes and LFOs of a Sampler, Generator or
    /// FM's voices.
    #[serde(default, skip_serializing_if = "Modulation::is_default")]
    pub modulation: Modulation,
    /// The single sample of Samplers saved before they had sample slots;
    /// read when loading and turned into a slot.
    #[serde(default, skip_serializing)]
    sample_path: Option<String>,
}

impl Module {
    /// The selected phrase, which the Phrase page shows.
    pub fn phrase(&self) -> Option<&Phrase> {
        self.phrases.get(self.selected_phrase)
    }

    pub fn phrase_mut(&mut self) -> Option<&mut Phrase> {
        self.phrases.get_mut(self.selected_phrase)
    }

    /// Whether notes play phrases unless a `Zxx` says otherwise.
    pub fn plays_phrases(&self) -> bool {
        self.phrase_mode != PhraseMode::Off && !self.phrases.is_empty()
    }

    /// The value of automatable parameter `i` (see
    /// `ModuleKind::automatable`).
    pub fn automatable_value(&self, i: usize) -> f32 {
        let n = self.params.len();
        match i {
            i if i < n => self.params[i],
            i if i == n => self.gain,
            _ => self.pan,
        }
    }

    pub fn new(id: u8, kind: ModuleKind, pos: [f32; 2]) -> Self {
        Self {
            id,
            kind,
            name: kind.name().to_string(),
            params: kind.params().iter().map(|p| p.default).collect(),
            pos,
            mute: false,
            gain: 1.0,
            pan: 0.0,
            solo: false,
            bypass: false,
            color: None,
            controls: Vec::new(),
            samples: Vec::new(),
            shape: Vec::new(),
            modulation: Modulation::default(),
            phrases: Vec::new(),
            phrase_mode: PhraseMode::Off,
            selected_phrase: 0,
            old_phrase: None,
            sample_path: None,
        }
    }

    /// Takes the sound of `other`, a module of the same kind: its name,
    /// settings, mixer strip, samples, voice modulation and phrases. Its
    /// place in the song (id, position and links) stays.
    pub fn take_sound(&mut self, other: &Module) {
        if other.kind != self.kind {
            return;
        }
        self.name = other.name.clone();
        let specs = self.kind.params();
        self.params = specs
            .iter()
            .enumerate()
            .map(|(i, s)| other.params.get(i).copied().unwrap_or(s.default).clamp(s.min, s.max))
            .collect();
        (self.gain, self.pan, self.bypass, self.color) = (other.gain, other.pan, other.bypass, other.color);
        self.samples = other.samples.clone();
        self.modulation = other.modulation.clone();
        self.phrases = other.phrases.clone();
        (self.phrase_mode, self.selected_phrase) = (other.phrase_mode, other.selected_phrase);
    }
}

fn unity() -> f32 {
    1.0
}

fn is_unity(v: &f32) -> bool {
    *v == 1.0
}

fn all_default(s: &[SliceSettings]) -> bool {
    s.iter().all(|s| *s == SliceSettings::default())
}

fn is_zero_index(v: &usize) -> bool {
    *v == 0
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

fn yes() -> bool {
    true
}

fn default_tpl() -> u32 {
    6
}

/// A position in the song: the pattern it plays and the tracks muted
/// there, in the pattern matrix.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "SlotFile", into = "SlotFile")]
pub struct Slot {
    pub pattern: usize,
    /// Bit `t` mutes track `t`.
    pub muted: u32,
}

const _: () = assert!(MAX_TRACKS <= u32::BITS as usize);

impl Slot {
    pub fn new(pattern: usize) -> Self {
        Slot { pattern, muted: 0 }
    }

    pub fn is_muted(&self, track: usize) -> bool {
        track < MAX_TRACKS && self.muted >> track & 1 == 1
    }

    pub fn toggle_mute(&mut self, track: usize) {
        if track < MAX_TRACKS {
            self.muted ^= 1 << track;
        }
    }
}

/// A slot in a song file: just the pattern number when nothing is muted,
/// as songs were saved before slots could mute tracks.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum SlotFile {
    Pattern(usize),
    Muted { pattern: usize, muted: u32 },
}

impl From<SlotFile> for Slot {
    fn from(f: SlotFile) -> Self {
        match f {
            SlotFile::Pattern(pattern) => Slot::new(pattern),
            SlotFile::Muted { pattern, muted } => Slot { pattern, muted },
        }
    }
}

impl From<Slot> for SlotFile {
    fn from(s: Slot) -> Self {
        if s.muted == 0 { SlotFile::Pattern(s.pattern) } else { SlotFile::Muted { pattern: s.pattern, muted: s.muted } }
    }
}

/// A named part of the song, starting at a slot: a sequencer section.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub start: usize,
    pub name: String,
}

/// One track of one pattern, every note and effect column of it, as the
/// pattern matrix copies it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackCells {
    pub notes: Vec<Vec<Cell>>,
    pub effects: Vec<Vec<Cell>>,
}

/// Whose device chain: an instrument's (or, for the output, the master
/// chain), or a track's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Owner {
    Module(u8),
    Track(usize),
}

impl From<u8> for Owner {
    fn from(id: u8) -> Self {
        Owner::Module(id)
    }
}

/// A copy of a module in the sound's path: the module, and for one copied
/// for a track with effects of its own, that track.
pub type Instance = (u8, Option<usize>);

/// An instrument's device chain: see `Project::chain`.
#[derive(Clone, Debug, PartialEq)]
pub struct Chain {
    pub effects: Vec<u8>,
    /// Where the chain sends its sound: usually the output, or effects that
    /// other instruments feed too.
    pub outputs: Vec<u8>,
}

/// Song-wide settings of a track, shared by every pattern.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Track {
    /// Empty for the default name.
    pub name: String,
    pub mute: bool,
    pub solo: bool,
    /// `None` for the default color.
    pub color: Option<[u8; 3]>,
    /// Show the panning and delay columns.
    pub show_pan: bool,
    pub show_delay: bool,
    /// The track's own effects: what its notes
    /// play goes through them, after the instruments' own effects, on its
    /// way to the output. They are left out of the links, as the master
    /// chain's effects are.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<u8>,
    /// The track whose effects this track's sound goes on through, as a
    /// bus, rather than straight to the master.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    /// The song's title, artist and comments: its Song Comments.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub artist: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comments: String,
    pub bpm: f32,
    /// Lines per beat.
    pub lpb: u32,
    /// Ticks per line: the steps effects take within a line.
    #[serde(default = "default_tpl")]
    pub tpl: u32,
    /// Swing, the groove: every odd line starts this much of half
    /// a line late, 0..1.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub groove: f32,
    /// Songs from before `F00`: off, the song stopped after its last
    /// slot. Read only, and turned into an `F00` on loading.
    #[serde(default = "yes", skip_serializing)]
    loop_song: bool,
    pub patterns: Vec<Pattern>,
    /// The song: which pattern plays at each position.
    pub order: Vec<Slot>,
    /// Named parts of the order list, sorted by where they start.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<Section>,
    pub modules: Vec<Module>,
    /// Audio connections `(from, to)` by module id.
    pub links: Vec<(u8, u8)>,
    /// The master chain: effects the whole mix
    /// goes through, in order, on its way to the output. They are left out
    /// of `links`; `audio_links` puts them in the sound's path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub master: Vec<u8>,
    /// Settings for every track number, `MAX_TRACKS` of them.
    #[serde(default)]
    pub tracks: Vec<Track>,
    /// The mute flags of songs saved before tracks had settings; read when
    /// loading and moved into `tracks`.
    #[serde(default, skip_serializing)]
    track_mute: Vec<bool>,
}

impl Project {
    pub fn empty() -> Self {
        Self {
            title: String::new(),
            artist: String::new(),
            comments: String::new(),
            bpm: 125.0,
            lpb: 4,
            tpl: default_tpl(),
            groove: 0.0,
            loop_song: true,
            patterns: vec![Pattern::new("", 4, 64)],
            order: vec![Slot::new(0)],
            sections: Vec::new(),
            modules: vec![Module::new(OUTPUT_ID, ModuleKind::Output, [580.0, 90.0])],
            links: vec![],
            master: Vec::new(),
            tracks: vec![Track::default(); MAX_TRACKS],
            track_mute: Vec::new(),
        }
    }

    pub fn module(&self, id: u8) -> Option<&Module> {
        self.modules.iter().find(|m| m.id == id)
    }

    pub fn module_mut(&mut self, id: u8) -> Option<&mut Module> {
        self.modules.iter_mut().find(|m| m.id == id)
    }

    pub fn add_module(&mut self, kind: ModuleKind, pos: [f32; 2]) -> Option<u8> {
        let id = (1..=255u8).find(|id| self.module(*id).is_none())?;
        self.modules.push(Module::new(id, kind, pos));
        Some(id)
    }

    /// Copies module `id` (settings, samples and mixer, but not its
    /// connections or solo) to a new module at `pos`.
    pub fn duplicate_module(&mut self, id: u8, pos: [f32; 2]) -> Option<u8> {
        let src = self.module(id)?.clone();
        let new = self.add_module(src.kind, pos)?;
        *self.module_mut(new).unwrap() = Module { id: new, pos, solo: false, ..src };
        Some(new)
    }

    pub fn remove_module(&mut self, id: u8) {
        if id == OUTPUT_ID {
            return;
        }
        self.modules.retain(|m| m.id != id);
        self.links.retain(|&(a, b)| a != id && b != id);
        self.master.retain(|&m| m != id);
        for t in &mut self.tracks {
            t.effects.retain(|&m| m != id);
        }
        for pat in &mut self.patterns {
            pat.automation.retain(|e| e.module != id);
        }
    }

    /// Adds a link unless it already exists, is invalid, or would create a
    /// cycle. Links carry audio into effects, or notes from a MultiSynth
    /// into instruments.
    pub fn connect(&mut self, from: u8, to: u8) -> bool {
        let (Some(a), Some(b)) = (self.module(from), self.module(to)) else {
            return false;
        };
        let fits = if a.kind.notes_only() { b.kind.is_instrument() } else { a.kind.controls() || b.kind.has_input() };
        // The master and track chains' effects are wired by their place in
        // them; only Modulators link to them.
        let master = self.placed(from) || (self.placed(to) && !a.kind.controls());
        if from == to
            || !a.kind.has_output()
            || !fits
            || master
            || self.links.contains(&(from, to))
            || self.reaches(to, from)
        {
            return false;
        }
        self.links.push((from, to));
        true
    }

    /// The parameter Modulator `from` moves on module `to`; the first one
    /// until another is picked.
    pub fn control_param(&self, from: u8, to: u8) -> usize {
        self.module(from).and_then(|m| m.controls.iter().find(|c| c.0 == to)).map_or(0, |c| c.1)
    }

    /// Makes Modulator `from` move parameter `param` of module `to`.
    pub fn set_control_param(&mut self, from: u8, to: u8, param: usize) {
        let Some(m) = self.module_mut(from) else { return };
        match m.controls.iter_mut().find(|c| c.0 == to) {
            Some(c) => c.1 = param,
            None => m.controls.push((to, param)),
        }
    }

    pub fn disconnect(&mut self, from: u8, to: u8) {
        self.links.retain(|&l| l != (from, to));
    }

    fn reaches(&self, from: u8, to: u8) -> bool {
        let mut stack = vec![from];
        let mut seen = Vec::new();
        while let Some(n) = stack.pop() {
            if n == to {
                return true;
            }
            if seen.contains(&n) {
                continue;
            }
            seen.push(n);
            stack.extend(self.links.iter().filter(|l| l.0 == n).map(|l| l.1));
        }
        false
    }

    /// Solos `track` alone: the other tracks fall silent.
    /// Soloing the one track soloed again brings them all back.
    pub fn solo_track(&mut self, track: usize) {
        let alone = self.tracks.iter().enumerate().all(|(t, info)| info.solo == (t == track));
        for (t, info) in self.tracks.iter_mut().enumerate() {
            info.solo = !alone && t == track;
        }
    }

    /// Whether notes on `track` play: it isn't muted, and it is soloed
    /// if any track is.
    pub fn track_audible(&self, track: usize) -> bool {
        let Some(t) = self.tracks.get(track) else { return true };
        !t.mute && (t.solo || !self.tracks.iter().any(|t| t.solo))
    }

    /// Gives the tracks of pattern `index` the note columns they have in
    /// the other patterns.
    pub fn sync_columns(&mut self, index: usize) {
        for t in 0..self.patterns[index].num_tracks() {
            let others =
                || self.patterns.iter().enumerate().filter(|(i, p)| *i != index && t < p.num_tracks()).map(|(_, p)| p);
            let (notes, fx) = (others().map(|p| p.columns(t)).max(), others().map(|p| p.fx_columns(t)).max());
            if let Some(n) = notes {
                self.patterns[index].set_columns(t, n);
            }
            if let Some(n) = fx {
                self.patterns[index].set_fx_columns(t, n);
            }
        }
    }

    /// Shows `n` effect columns on `track` in every pattern.
    pub fn set_fx_columns(&mut self, track: usize, n: usize) {
        for pat in &mut self.patterns {
            if track < pat.num_tracks() {
                pat.set_fx_columns(track, n);
            }
        }
    }

    /// Shows `n` note columns on `track` in every pattern: a track's
    /// columns are the same throughout the song.
    pub fn set_columns(&mut self, track: usize, n: usize) {
        for pat in &mut self.patterns {
            if track < pat.num_tracks() {
                pat.set_columns(track, n);
            }
        }
    }

    /// Instrument `id`'s device chain: the
    /// effects only it feeds, one after another, and where the last one
    /// (or the instrument) sends its sound.
    pub fn chain(&self, owner: impl Into<Owner>) -> Chain {
        let id = match owner.into() {
            Owner::Track(t) => {
                let effects = self.tracks.get(t).map(|t| t.effects.clone()).unwrap_or_default();
                return Chain { effects, outputs: Vec::new() };
            }
            // The output's chain is the master chain.
            Owner::Module(OUTPUT_ID) => return Chain { effects: self.master.clone(), outputs: Vec::new() },
            Owner::Module(id) => id,
        };
        let mut effects = Vec::new();
        let mut at = id;
        loop {
            // A Modulator following the sound only listens: it is no send.
            let listens = |t: u8| self.module(t).is_some_and(|m| m.kind.controls());
            let outs: Vec<u8> = self.links.iter().filter(|l| l.0 == at && !listens(l.1)).map(|l| l.1).collect();
            let exclusive = |t: u8| {
                self.module(t).is_some_and(|m| m.kind.has_input() && m.kind.has_output() && m.kind.makes_sound())
                    && self.links.iter().filter(|l| l.1 == t && self.carries_audio(l.0)).count() == 1
                    && !effects.contains(&t)
            };
            match outs[..] {
                [next] if exclusive(next) => {
                    effects.push(next);
                    at = next;
                }
                _ => return Chain { effects, outputs: outs },
            }
        }
    }

    /// The links as the sound takes them: what goes to the output goes
    /// through the master chain first, effect after effect, then out.
    pub fn audio_links(&self) -> Vec<(u8, u8)> {
        let Some(&first) = self.master.first() else { return self.links.clone() };
        let mut links: Vec<(u8, u8)> = self
            .links
            .iter()
            .map(|&(a, b)| if b == OUTPUT_ID && self.carries_audio(a) { (a, first) } else { (a, b) })
            .collect();
        links.extend(self.master.windows(2).map(|w| (w[0], w[1])));
        links.push((*self.master.last().unwrap(), OUTPUT_ID));
        links
    }

    /// Whether module `id` is in the master chain or a track's, where its
    /// place, not links, says where its sound goes.
    pub fn placed(&self, id: u8) -> bool {
        self.master.contains(&id) || self.tracks.iter().any(|t| t.effects.contains(&id))
    }

    /// The instruments track `t`'s notes play, in any pattern: those its
    /// cells name, and those the MultiSynths among them pass notes to.
    pub fn track_instruments(&self, t: usize) -> Vec<u8> {
        let mut found: Vec<u8> = Vec::new();
        for pat in self.patterns.iter().filter(|p| t < p.num_tracks()) {
            for c in 0..pat.columns(t) {
                for m in pat.column(t, c)[..pat.lines].iter().filter_map(|c| c.module) {
                    if !found.contains(&m) {
                        found.push(m);
                    }
                }
            }
        }
        let mut i = 0;
        while i < found.len() {
            if self.module(found[i]).is_some_and(|m| m.kind.notes_only()) {
                for &(a, b) in &self.links {
                    if a == found[i] && !found.contains(&b) {
                        found.push(b);
                    }
                }
            }
            i += 1;
        }
        found.retain(|&m| self.module(m).is_some_and(|m| m.kind.is_instrument() && !m.kind.notes_only()));
        found
    }

    /// The sound's whole path: every module once, and for each track with
    /// effects, a copy of each instrument it plays with that instrument's
    /// own effects, keeping an instrument's effects apart per track. What a
    /// copy would send to the output goes through the track's effects
    /// first; what it sends to a shared effect still goes there.
    /// All but the links are left out for tracks without effects, so a song
    /// without track effects costs what it did.
    pub fn signal_graph(&self) -> (Vec<Instance>, Vec<(Instance, Instance)>) {
        let mut nodes: Vec<Instance> = self.modules.iter().map(|m| (m.id, None)).collect();
        let mut links: Vec<(Instance, Instance)> =
            self.audio_links().into_iter().map(|(a, b)| ((a, None), (b, None))).collect();
        let out = (self.master.first().copied().unwrap_or(OUTPUT_ID), None);
        for (t, track) in self.tracks.iter().enumerate() {
            // A track in a group without effects of its own goes straight
            // into the group's.
            let Some(first) = self.track_entry(t) else { continue };
            for m in self.track_instruments(t) {
                let chain = self.chain(m);
                let mut at = (m, Some(t));
                nodes.push(at);
                for &e in &chain.effects {
                    nodes.push((e, Some(t)));
                    links.push((at, (e, Some(t))));
                    at = (e, Some(t));
                }
                for &o in &chain.outputs {
                    links.push((at, if o == OUTPUT_ID { (first, None) } else { (o, None) }));
                }
            }
            if let Some(&last) = track.effects.last() {
                links.extend(track.effects.windows(2).map(|w| ((w[0], None), (w[1], None))));
                links.push(((last, None), self.track_after(t).map_or(out, |e| (e, None))));
            }
        }
        (nodes, links)
    }

    /// The group track `t` is in, if it names another track.
    pub fn group_of(&self, t: usize) -> Option<usize> {
        self.tracks.get(t)?.group.filter(|&g| g != t && g < self.tracks.len())
    }

    /// The first effect track `t`'s sound goes through: its own first, or
    /// else its group's (or that group's group's); `None` when there are
    /// none on the way to the master.
    fn track_entry(&self, t: usize) -> Option<u8> {
        let mut at = Some(t);
        for _ in 0..MAX_TRACKS {
            let track = self.tracks.get(at?)?;
            if let Some(&first) = track.effects.first() {
                return Some(first);
            }
            at = self.group_of(at?);
        }
        None
    }

    /// Where track `t`'s sound goes after its own effects: into its
    /// group's, or `None` for the master.
    fn track_after(&self, t: usize) -> Option<u8> {
        self.track_entry(self.group_of(t)?)
    }

    /// Whether track `t` can go through group `g`: not itself, nor a
    /// track whose sound already comes through `t`.
    pub fn can_group(&self, t: usize, g: usize) -> bool {
        let mut at = Some(g);
        for _ in 0..MAX_TRACKS {
            match at {
                Some(x) if x == t => return false,
                Some(x) => at = self.group_of(x),
                None => return g < self.tracks.len(),
            }
        }
        false
    }

    /// Whether links from module `id` carry sound, rather than notes or
    /// parameter changes.
    fn carries_audio(&self, id: u8) -> bool {
        self.module(id).is_some_and(|m| !m.kind.notes_only() && !m.kind.controls())
    }

    /// Rewires instrument `id` through `effects` in order, to `outputs`.
    fn set_chain(&mut self, owner: Owner, effects: &[u8], outputs: &[u8]) {
        let id = match owner {
            Owner::Track(t) => {
                if let Some(track) = self.tracks.get_mut(t) {
                    track.effects = effects.to_vec();
                }
                return;
            }
            Owner::Module(OUTPUT_ID) => {
                self.master = effects.to_vec();
                return;
            }
            Owner::Module(id) => id,
        };
        let old = self.chain(id);
        let mut devices = vec![id];
        devices.extend(&old.effects);
        // Only the links from the devices of the chain change.
        self.links.retain(|l| !devices.contains(&l.0) || !(devices.contains(&l.1) || old.outputs.contains(&l.1)));
        let mut path = vec![id];
        path.extend(effects);
        for w in path.windows(2) {
            self.links.push((w[0], w[1]));
        }
        let last = *path.last().unwrap();
        for &o in outputs {
            self.connect(last, o);
        }
    }

    /// Adds an effect of `kind` to instrument `id`'s chain, after the
    /// device at `after` (0 is the instrument itself).
    pub fn chain_insert(&mut self, owner: impl Into<Owner>, after: usize, kind: ModuleKind) -> Option<u8> {
        let owner = owner.into();
        let chain = self.chain(owner);
        let mut effects = chain.effects.clone();
        let prev = match (after, owner) {
            (0, Owner::Module(id)) => Some(id),
            (0, Owner::Track(_)) => None,
            _ => Some(*effects.get(after - 1)?),
        };
        let pos = prev.and_then(|p| self.module(p)).map_or([0.0; 2], |m| [m.pos[0] + 170.0, m.pos[1] + 20.0]);
        let new = self.add_module(kind, pos)?;
        effects.insert(after.min(effects.len()), new);
        // A chain that went nowhere now goes to the output.
        let outputs = if chain.outputs.is_empty() { vec![OUTPUT_ID] } else { chain.outputs };
        self.set_chain(owner, &effects, &outputs);
        Some(new)
    }

    /// Takes `effect` out of instrument `id`'s chain and deletes it.
    pub fn chain_remove(&mut self, owner: impl Into<Owner>, effect: u8) {
        let owner = owner.into();
        let chain = self.chain(owner);
        if !chain.effects.contains(&effect) {
            return;
        }
        let effects: Vec<u8> = chain.effects.iter().copied().filter(|&e| e != effect).collect();
        self.set_chain(owner, &effects, &chain.outputs);
        self.remove_module(effect);
    }

    /// Puts `effect` at place `index` of instrument `id`'s chain.
    pub fn chain_place(&mut self, owner: impl Into<Owner>, effect: u8, index: usize) {
        let owner = owner.into();
        let chain = self.chain(owner);
        let Some(i) = chain.effects.iter().position(|&e| e == effect) else { return };
        let mut effects = chain.effects.clone();
        effects.remove(i);
        effects.insert(index.min(effects.len()), effect);
        self.set_chain(owner, &effects, &chain.outputs);
    }

    /// Moves `effect` `delta` places along instrument `id`'s chain.
    pub fn chain_move(&mut self, owner: impl Into<Owner>, effect: u8, delta: i32) {
        let owner = owner.into();
        let chain = self.chain(owner);
        let Some(i) = chain.effects.iter().position(|&e| e == effect) else { return };
        let j = i as i32 + delta;
        if j < 0 || j >= chain.effects.len() as i32 {
            return;
        }
        let mut effects = chain.effects.clone();
        effects.swap(i, j as usize);
        self.set_chain(owner, &effects, &chain.outputs);
    }

    /// Inserts `slot` at position `at` of the order list; sections after
    /// it move along.
    pub fn insert_slot(&mut self, at: usize, slot: Slot) {
        self.order.insert(at, slot);
        for s in &mut self.sections {
            if s.start >= at && s.start > 0 {
                s.start += 1;
            }
        }
    }

    /// Removes position `at` of the order list, keeping one; sections
    /// after it move back, and one that would land on another goes.
    pub fn remove_slot(&mut self, at: usize) {
        if self.order.len() <= 1 || at >= self.order.len() {
            return;
        }
        self.order.remove(at);
        for s in &mut self.sections {
            if s.start > at {
                s.start -= 1;
            }
        }
        self.tidy_sections();
    }

    /// Swaps positions `a` and `b` of the order list. Sections stay where
    /// they are, and the slots move between them.
    pub fn swap_slots(&mut self, a: usize, b: usize) {
        self.order.swap(a, b);
    }

    /// Moves the slot at `from` to `to`, the ones between moving up or down
    /// one; sections stay where they are, as with `swap_slots`.
    pub fn move_slot(&mut self, from: usize, to: usize) {
        let last = self.order.len().saturating_sub(1);
        let (from, to) = (from.min(last), to.min(last));
        if from < to {
            self.order[from..=to].rotate_left(1);
        } else {
            self.order[to..=from].rotate_right(1);
        }
    }

    /// Names the section starting at `slot`, adding it if needed.
    pub fn set_section(&mut self, slot: usize, name: String) {
        match self.sections.iter_mut().find(|s| s.start == slot) {
            Some(s) => s.name = name,
            None => self.sections.push(Section { start: slot, name }),
        }
        self.tidy_sections();
    }

    pub fn remove_section(&mut self, slot: usize) {
        self.sections.retain(|s| s.start != slot);
    }

    /// The section starting at `slot`, if any.
    pub fn section_at(&self, slot: usize) -> Option<&Section> {
        self.sections.iter().find(|s| s.start == slot)
    }

    /// The slots of the section starting at `slot`: up to the next one.
    pub fn section_slots(&self, slot: usize) -> (usize, usize) {
        let end = self.sections.iter().map(|s| s.start).filter(|&s| s > slot).min().unwrap_or(self.order.len());
        (slot, end.max(slot + 1) - 1)
    }

    fn tidy_sections(&mut self) {
        let len = self.order.len();
        self.sections.retain(|s| s.start < len);
        self.sections.sort_by_key(|s| s.start);
        self.sections.dedup_by_key(|s| s.start);
    }

    /// Copies tracks `tracks` (inclusive) of the patterns in slots `slots`
    /// (inclusive), as `[slot][track]`; tracks a pattern lacks are empty.
    pub fn copy_matrix(&self, slots: (usize, usize), tracks: (usize, usize)) -> Vec<Vec<TrackCells>> {
        (slots.0..=slots.1)
            .map(|i| {
                let p = &self.patterns[self.order[i].pattern];
                (tracks.0..=tracks.1)
                    .map(|t| if t < p.num_tracks() { p.track_cells(t) } else { TrackCells::default() })
                    .collect()
            })
            .collect()
    }

    /// Writes `clip` from `copy_matrix` with its corner at `slot` and
    /// `track`, cut off at the song's end and each pattern's tracks.
    /// Tracks get the note columns they need. Patterns played in several
    /// slots change in all of them.
    pub fn paste_matrix(&mut self, clip: &[Vec<TrackCells>], slot: usize, track: usize) {
        for (i, row) in clip.iter().enumerate() {
            let Some(s) = self.order.get(slot + i) else { break };
            let p = s.pattern;
            for (k, cells) in row.iter().enumerate() {
                let t = track + k;
                if t >= self.patterns[p].num_tracks() {
                    break;
                }
                self.patterns[p].clear_track(t);
                if cells.notes.len() > self.patterns[p].columns(t) {
                    self.set_columns(t, cells.notes.len());
                }
                if cells.effects.len() > self.patterns[p].fx_columns(t) {
                    self.set_fx_columns(t, cells.effects.len());
                }
                let notes = self.patterns[p].columns(t);
                for (c, col) in cells.notes.iter().enumerate() {
                    self.patterns[p].column_mut(t, c).clone_from(col);
                }
                for (c, col) in cells.effects.iter().enumerate() {
                    self.patterns[p].column_mut(t, notes + c).clone_from(col);
                }
            }
        }
    }

    /// Empties tracks `tracks` of the patterns in slots `slots`.
    pub fn clear_matrix(&mut self, slots: (usize, usize), tracks: (usize, usize)) {
        for i in slots.0..=slots.1.min(self.order.len() - 1) {
            let p = &mut self.patterns[self.order[i].pattern];
            for t in tracks.0..=tracks.1.min(p.num_tracks() - 1) {
                p.clear_track(t);
            }
        }
    }

    /// A song that plays only lines `lines` and lanes `lanes` (both
    /// inclusive) of the pattern in order position `slot`, once, for
    /// rendering a selection to a sample. Envelopes come along, moved to
    /// the new first line.
    pub fn excerpt(&self, slot: usize, lines: (usize, usize), lanes: (usize, usize)) -> Project {
        let mut song = self.clone();
        let s = self.order[slot];
        let mut pat = self.patterns[s.pattern].clone();
        let (from, to) = (lines.0.min(pat.lines - 1), lines.1.min(pat.lines - 1));
        for lane in 0..pat.num_lanes() {
            let col = pat.lane_mut(lane);
            if (lanes.0..=lanes.1).contains(&lane) {
                col.copy_within(from..=to, 0);
                col[to - from + 1..].fill(Cell::default());
            } else {
                col.fill(Cell::default());
            }
        }
        pat.lines = to - from + 1;
        for env in &mut pat.automation {
            let start = env.value_at(from as f32);
            let mut points: Vec<(f32, f32)> = env
                .points
                .iter()
                .filter(|p| p.0 > from as f32 && p.0 <= to as f32 + 1.0)
                .map(|&(x, y)| (x - from as f32, y))
                .collect();
            if let Some(v) = start {
                points.insert(0, (0.0, v));
            }
            env.points = points;
        }
        song.patterns = vec![pat];
        song.order = vec![Slot { pattern: 0, muted: s.muted }];
        song.sections.clear();
        song
    }

    pub fn track_name(&self, track: usize) -> String {
        match self.tracks.get(track) {
            Some(t) if !t.name.is_empty() => t.name.clone(),
            _ => format!("Track {:02}", track + 1),
        }
    }

    /// Inserts an empty track before `track` in every pattern that has
    /// tracks from there on, and in pattern `grow` even if it ends there,
    /// moving the later tracks and their settings along. Refuses when a
    /// pattern would get more than `MAX_TRACKS`.
    pub fn insert_track(&mut self, track: usize, grow: usize) -> bool {
        let gets = |i: usize, p: &Pattern| track < p.tracks.len() || (i == grow && track == p.tracks.len());
        if self.patterns.iter().enumerate().any(|(i, p)| gets(i, p) && p.tracks.len() >= MAX_TRACKS) {
            return false;
        }
        for (i, pat) in self.patterns.iter_mut().enumerate() {
            if gets(i, pat) {
                pat.insert_track(track);
            }
        }
        self.tracks.insert(track, Track::default());
        self.tracks.truncate(MAX_TRACKS);
        for t in &mut self.tracks {
            t.group = t.group.map(|g| if g >= track { g + 1 } else { g }).filter(|&g| g < MAX_TRACKS);
        }
        true
    }

    /// Removes `track` from every pattern that has it, keeping at least one
    /// track in each, and moves the later tracks and their settings back.
    pub fn remove_track(&mut self, track: usize) {
        for pat in &mut self.patterns {
            if track < pat.tracks.len() && pat.tracks.len() > 1 {
                pat.remove_track(track);
            }
        }
        if track < self.tracks.len() {
            let gone = self.tracks.remove(track);
            self.tracks.push(Track::default());
            // Tracks in its group go to the master again.
            for t in &mut self.tracks {
                t.group = match t.group {
                    Some(g) if g == track => None,
                    Some(g) if g > track => Some(g - 1),
                    g => g,
                };
            }
            // Its effects go with it.
            for e in gone.effects {
                self.remove_module(e);
            }
        }
    }

    /// Reads a song file and loads the samples it uses. Problems that don't
    /// prevent opening the song (such as a missing sample) are returned as
    /// warnings.
    /// Puts an `F00` on the last line the song plays, so it stops there
    /// rather than starting over: in the first track whose cell there has
    /// no effect. Returns false when none is free.
    pub fn end_with_stop(&mut self) -> bool {
        let Some(slot) = self.order.last() else { return false };
        let Some(pat) = self.patterns.get_mut(slot.pattern) else { return false };
        let line = pat.lines - 1;
        for t in 0..pat.num_tracks() {
            let cell = pat.cell_mut(t, 0, line);
            if cell.fx.is_none() {
                cell.fx = Some((0xF, 0x00));
                return true;
            }
        }
        false
    }

    pub fn load(path: &str) -> Result<(Project, Vec<String>), String> {
        let s = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut p: Project = serde_json::from_str(&s).map_err(|e| e.to_string())?;
        // Normalize anything a hand-edited file might get wrong.
        p.tpl = p.tpl.clamp(1, 16);
        p.groove = p.groove.clamp(0.0, 1.0);
        p.tracks.resize(MAX_TRACKS, Track::default());
        for (t, mute) in p.tracks.iter_mut().zip(std::mem::take(&mut p.track_mute)) {
            t.mute = mute;
        }
        if p.patterns.is_empty() {
            p.patterns.push(Pattern::new("", 4, 64));
        }
        for pat in &mut p.patterns {
            pat.lines = pat.lines.clamp(1, MAX_LINES);
            pat.tracks.truncate(MAX_TRACKS);
            if pat.tracks.is_empty() {
                pat.tracks.push(Vec::new());
            }
            for t in &mut pat.tracks {
                t.resize(MAX_LINES, Cell::default());
            }
            pat.normalize_columns();
        }
        p.order.retain(|s| s.pattern < p.patterns.len());
        if p.order.is_empty() {
            p.order.push(Slot::new(0));
        }
        if !std::mem::replace(&mut p.loop_song, true) {
            p.end_with_stop();
        }
        p.tidy_sections();
        // The master and track chains hold effects, each in one place once,
        // with no links of their own.
        let mut seen = vec![OUTPUT_ID];
        let mut keep = |p: &Project, chain: &mut Vec<u8>| {
            chain.retain(|&id| {
                let effect =
                    p.module(id).is_some_and(|m| m.kind.has_input() && m.kind.has_output() && m.kind.makes_sound());
                effect && !seen.contains(&id) && {
                    seen.push(id);
                    true
                }
            });
        };
        let mut master = std::mem::take(&mut p.master);
        keep(&p, &mut master);
        let mut tracks = std::mem::take(&mut p.tracks);
        for t in &mut tracks {
            keep(&p, &mut t.effects);
        }
        (p.master, p.tracks) = (master, tracks);
        let stray: Vec<(u8, u8)> =
            (p.links.iter().copied()).filter(|&(a, b)| p.placed(a) || (p.placed(b) && p.carries_audio(a))).collect();
        p.links.retain(|l| !stray.contains(l));
        let dir = Path::new(path).parent().unwrap_or(Path::new("."));
        let mut warnings = Vec::new();
        // A file several slots play is loaded once, and they share it.
        let mut loaded: std::collections::HashMap<PathBuf, Arc<Sample>> = std::collections::HashMap::new();
        for m in &mut p.modules {
            let migrate = m.kind == ModuleKind::Sampler && m.params.len() == 10;
            let old_loop = if migrate { migrate_sampler(m) } else { None };
            for slot in &mut m.samples {
                let Some(sp) = &slot.path else { continue };
                let full = resolve(dir, sp);
                match loaded.get(&full).cloned().map_or_else(|| Sample::load(&full).map(Arc::new), Ok) {
                    Ok(s) => {
                        loaded.insert(full.clone(), s.clone());
                        slot.data = Some(s);
                    }
                    Err(e) => warnings.push(format!("sample {}: {e}", full.display())),
                }
                // Kept whole, so the song can be saved somewhere else.
                slot.path = Some(full.to_string_lossy().into_owned());
                if let Some((start, end)) = old_loop {
                    let len = slot.len() as f32;
                    slot.loop_start = (start * len) as usize;
                    slot.loop_end = (end * len) as usize;
                }
                slot.clamp_loop();
                if slot.data.is_some() {
                    slot.clamp_slices();
                }
            }
        }
        for m in &mut p.modules {
            // Parameters added since the song was saved start at their
            // defaults.
            let specs = m.kind.params();
            m.params.truncate(specs.len());
            m.params.extend(specs[m.params.len()..].iter().map(|s| s.default));
            for (v, s) in m.params.iter_mut().zip(specs) {
                *v = v.clamp(s.min, s.max);
            }
            m.gain = m.gain.clamp(0.0, 2.0);
            m.pan = m.pan.clamp(-1.0, 1.0);
            m.modulation.normalize();
            if let Some(old) = m.old_phrase.take().filter(|p| p.on || !p.phrase.is_default()) {
                if old.on {
                    m.phrase_mode = PhraseMode::Program;
                }
                m.phrases.insert(0, old.phrase);
            }
            m.phrases.truncate(MAX_PHRASES);
            m.phrases.iter_mut().for_each(Phrase::normalize);
            m.selected_phrase = m.selected_phrase.min(m.phrases.len().saturating_sub(1));
        }
        if p.module(OUTPUT_ID).is_none() {
            p.modules.push(Module::new(OUTPUT_ID, ModuleKind::Output, [580.0, 90.0]));
        }
        let kinds: Vec<(u8, ModuleKind)> = p.modules.iter().map(|m| (m.id, m.kind)).collect();
        for pat in &mut p.patterns {
            pat.automation.retain(|e| kinds.iter().any(|&(id, k)| id == e.module && e.param < k.num_automatable()));
            for e in &mut pat.automation {
                for pt in &mut e.points {
                    *pt = (pt.0.clamp(0.0, MAX_LINES as f32), pt.1.clamp(0.0, 1.0));
                }
                e.points.sort_by(|a, b| a.0.total_cmp(&b.0));
                e.points.dedup_by(|a, b| a.0 == b.0);
            }
        }
        let links = std::mem::take(&mut p.links);
        for (a, b) in links {
            p.connect(a, b);
        }
        // Targets a Modulator no longer links to are forgotten.
        let (links, kinds) = (p.links.clone(), kinds);
        for m in &mut p.modules {
            let id = m.id;
            m.controls.retain(|&(to, param)| {
                links.contains(&(id, to)) && kinds.iter().any(|&(k, kind)| k == to && param < kind.num_automatable())
            });
        }
        Ok((p, warnings))
    }

    /// The small song the demo used to be: drums, a bass through a filter
    /// and a bell through a delay and a reverb. Tests count on its layout.
    #[cfg(test)]
    pub fn simple() -> Self {
        let mut p = Self::empty();
        p.title = "Demo".into();
        p.comments = "A short loop to show what noise does. Press Space to play it, F1 for the keys.".into();
        p.bpm = 128.0;
        p.modules[0].params[0] = 0.6;
        let drums = p.add_module(ModuleKind::Drums, [40.0, 20.0]).unwrap();
        let bass = p.add_module(ModuleKind::Generator, [40.0, 90.0]).unwrap();
        let lead = p.add_module(ModuleKind::Fm, [40.0, 160.0]).unwrap();
        let filt = p.add_module(ModuleKind::Filter, [220.0, 90.0]).unwrap();
        let delay = p.add_module(ModuleKind::Delay, [220.0, 160.0]).unwrap();
        let verb = p.add_module(ModuleKind::Reverb, [400.0, 160.0]).unwrap();
        p.module_mut(bass).unwrap().name = "Bass".into();
        p.module_mut(lead).unwrap().name = "Bell".into();
        {
            let f = p.module_mut(filt).unwrap();
            f.params[1] = 900.0;
            f.params[2] = 0.6;
            f.params[3] = 0.25;
            f.params[4] = 0.6;
        }
        {
            let b = p.module_mut(bass).unwrap();
            b.params[0] = 0.45;
            b.params[3] = 0.15;
            b.params[4] = 0.3;
            b.params[5] = 0.05;
            b.params[6] = 8.0;
            b.params[7] = 2.0;
        }
        p.module_mut(lead).unwrap().params[0] = 0.3;
        p.connect(drums, OUTPUT_ID);
        p.connect(bass, filt);
        p.connect(filt, OUTPUT_ID);
        p.connect(lead, delay);
        p.connect(delay, verb);
        p.connect(verb, OUTPUT_ID);

        let mut pat_a = Pattern::new("Intro", 4, 64);
        let mut pat_b = Pattern::new("Main", 4, 64);
        let set = |pat: &mut Pattern, t: usize, l: usize, note: Note, m: u8, vol: Option<u8>| {
            pat.tracks[t][l] = Cell { note: Some(note), module: Some(m), vol, ..Cell::default() };
        };
        for pat in [&mut pat_a, &mut pat_b] {
            for l in (0..64).step_by(4) {
                // C = kick, D = snare, F# = closed hat (see the Drums module).
                set(pat, 0, l, Note::On(48), drums, None);
                set(pat, 1, l + 2, Note::On(54), drums, Some(0x40));
            }
            for l in [4, 12, 20, 28, 36, 44, 52, 60] {
                set(pat, 0, l, Note::On(50), drums, None);
            }
        }
        let bassline = [36, 36, 48, 36, 39, 39, 51, 39, 41, 41, 53, 41, 34, 34, 46, 43];
        for (i, n) in bassline.iter().enumerate() {
            set(&mut pat_b, 2, i * 4, Note::On(*n), bass, None);
            set(&mut pat_b, 2, i * 4 + 2, Note::Off, bass, None);
        }
        let melody = [(0, 72), (6, 75), (12, 79), (16, 77), (24, 75), (32, 72), (38, 70), (44, 67), (48, 70), (56, 72)];
        for (l, n) in melody {
            set(&mut pat_b, 3, l, Note::On(n), lead, None);
            set(&mut pat_a, 3, l, Note::On(n), lead, Some(0x30));
        }
        p.patterns = vec![pat_a, pat_b];
        // The intro holds back the hi-hats.
        let mut intro = Slot::new(0);
        intro.toggle_mute(1);
        p.order = vec![intro, Slot::new(1), Slot::new(1)];
        p.sections = vec![Section { start: 0, name: "Intro".into() }, Section { start: 1, name: "Groove".into() }];
        p
    }
}

/// Converts a Sampler from before sample slots: its parameters were volume,
/// root note, fine tune, attack, release, loop mode, start, loop start, loop
/// end (as fractions of the sample) and pan. Returns the loop, which can
/// only be turned into frames once the sample is loaded.
fn migrate_sampler(m: &mut Module) -> Option<(f32, f32)> {
    let old = std::mem::take(&mut m.params);
    m.params = vec![old[0], old[9], 0.0, old[3], 0.5, 1.0, old[4]];
    if let Some(path) = m.sample_path.take() {
        m.samples.push(SampleSlot {
            name: m.name.clone(),
            path: Some(path),
            base_note: old[1].round().clamp(0.0, 119.0) as u8,
            finetune: old[2].round() as i32,
            // Off, forward and ping-pong; ping-pong has moved up one.
            loop_mode: match old[5].round() as u8 {
                0 => 0,
                1 => 1,
                _ => 3,
            },
            ..Default::default()
        });
        return Some((old[7], old[8]));
    }
    None
}

fn resolve(dir: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() { p.to_path_buf() } else { dir.join(p) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_column_commands_read_back_as_written() {
        for v in 0..=0x80 {
            assert_eq!(parse_vol(&vol_text(v)), Some(v));
        }
        let fade = vol_command_value('O', 0xA).unwrap();
        assert_eq!((vol_text(fade), vol_command(fade)), ("OA".into(), Some(('O', 0xA))));
        assert_eq!(parse_vol("OA"), Some(fade));
        assert_eq!(vol_effect(fade), Some((0xA, 0x0A)));
        assert_eq!(parse_vol("FF"), Some(0x80), "hex past 80 is full volume, not a command");
        assert_eq!(parse_vol("XA"), None);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("noise-test-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_wav(path: &Path, frames: usize, channels: u16) {
        let frames = (0..frames).map(|i| [i as f32 / frames as f32, -(i as f32) / frames as f32]).collect();
        Sample { name: "t".into(), sample_rate: 22050.0, channels, frames }.save(path).unwrap();
    }

    #[test]
    fn params_map_to_sliders_and_text() {
        let specs = ModuleKind::Filter.params();
        let cutoff = &specs[1];
        // Frequencies move in octaves: the middle of 20 Hz..20 kHz is 632 Hz.
        assert!((cutoff.value_at(0.5) - 632.5).abs() < 1.0);
        let attack = &ModuleKind::Generator.params()[2];
        // Envelope times give the short end more room.
        assert!((attack.value_at(0.5) - 0.5).abs() < 1e-6);
        for spec in ModuleKind::ADDABLE.iter().flat_map(|k| k.params()) {
            for v in [spec.min, spec.default, spec.max] {
                let back = spec.value_at(spec.position(v));
                assert!((back - v).abs() <= (spec.max - spec.min) * 1e-4, "{}: {v} -> {back}", spec.name);
            }
        }
        let unison = &ModuleKind::Generator.params()[7];
        assert_eq!(unison.value_at(0.4), 2.0);
        assert_eq!(cutoff.format(2000.0), "2.00 kHz");
        assert_eq!(attack.format(0.005), "5 ms");
        assert_eq!(ModuleKind::Generator.params()[0].format(0.5), "-6.0 dB");
        assert_eq!(ModuleKind::Generator.params()[9].format(-0.25), "25L");
        assert_eq!(ModuleKind::Generator.params()[1].format(3.0), "Sine");
    }

    #[test]
    fn tracks_move_with_their_settings() {
        let mut p = Project::simple();
        p.tracks[1].name = "Hats".into();
        p.tracks[2].mute = true;
        let hats = p.patterns[1].tracks[1].clone();
        assert!(p.insert_track(1, 0));
        assert_eq!(p.tracks[2].name, "Hats");
        assert!(p.tracks[3].mute);
        assert_eq!(p.patterns[1].tracks[2], hats);
        assert_eq!(p.patterns[1].num_tracks(), 5);
        // After the last track, only the pattern being edited grows.
        assert!(p.insert_track(5, 0));
        assert_eq!((p.patterns[0].num_tracks(), p.patterns[1].num_tracks()), (6, 5));
        p.remove_track(5);
        p.remove_track(1);
        assert_eq!(p.track_name(1), "Hats");
        assert_eq!(p.track_name(MAX_TRACKS - 1), format!("Track {MAX_TRACKS}"), "a fresh track at the end");
        assert_eq!(p.patterns[1].tracks[1], hats);
        assert_eq!(p.tracks.len(), MAX_TRACKS);
        p.patterns[0].tracks.resize(MAX_TRACKS, vec![Cell::default(); MAX_LINES]);
        assert!(!p.insert_track(0, 0), "a full pattern refuses");
    }

    #[test]
    fn solo_and_mute_decide_which_tracks_play() {
        let mut p = Project::empty();
        assert!(p.track_audible(0) && p.track_audible(1));
        p.tracks[1].solo = true;
        assert!(!p.track_audible(0) && p.track_audible(1));
        p.tracks[1].mute = true;
        assert!(!p.track_audible(1), "mute wins over solo");
    }

    #[test]
    fn old_track_mutes_are_kept() {
        let dir = temp_dir("mutes");
        let mut json = serde_json::to_value(Project::empty()).unwrap();
        json.as_object_mut().unwrap().remove("tracks");
        json["track_mute"] = serde_json::json!([false, true]);
        let song = dir.join("old.json");
        std::fs::write(&song, json.to_string()).unwrap();
        let (p, _) = Project::load(song.to_str().unwrap()).unwrap();
        assert!(!p.tracks[0].mute && p.tracks[1].mute);
        assert_eq!(p.tracks.len(), MAX_TRACKS);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn note_columns_make_lanes() {
        let mut p = Project::empty();
        p.patterns.push(Pattern::new("b", 4, 64));
        p.set_columns(1, 3);
        let pat = &mut p.patterns[0];
        assert_eq!(pat.num_lanes(), 6);
        assert_eq!(pat.lane_pos(3), (1, 2));
        assert_eq!(pat.lane_of(2, 0), 4);
        pat.cell_mut(1, 2, 5).note = Some(Note::On(60));
        assert_eq!(pat.lane(3)[5].note, Some(Note::On(60)));
        assert!(pat.track_used(1));
        assert_eq!(p.patterns[1].columns(1), 3, "every pattern gets the columns");

        // Tracks move with their columns.
        p.insert_track(0, 0);
        assert_eq!(p.patterns[0].columns(2), 3);
        assert_eq!(p.patterns[0].cell(2, 2, 5).note, Some(Note::On(60)));
        p.remove_track(0);
        assert_eq!(p.patterns[0].columns(1), 3);

        // Hiding a column keeps its notes.
        p.set_columns(1, 1);
        assert!(!p.patterns[0].track_used(1));
        p.set_columns(1, 3);
        assert!(p.patterns[0].track_used(1));

        // A new pattern takes the song's columns.
        p.patterns.push(Pattern::new("c", 4, 64));
        p.sync_columns(2);
        assert_eq!(p.patterns[2].columns(1), 3);
    }

    #[test]
    fn note_columns_are_saved() {
        let dir = temp_dir("columns");
        let mut p = Project::empty();
        p.set_columns(0, 2);
        p.patterns[0].cell_mut(0, 1, 3).note = Some(Note::On(50));
        let song = dir.join("song.json");
        std::fs::write(&song, serde_json::to_string(&p).unwrap()).unwrap();
        let (p, _) = Project::load(song.to_str().unwrap()).unwrap();
        assert_eq!(p.patterns[0].columns(0), 2);
        assert_eq!(p.patterns[0].cell(0, 1, 3).note, Some(Note::On(50)));
        // Songs without extra columns or comments save as before.
        let plain = serde_json::to_string(&Project::empty()).unwrap();
        assert!(!plain.contains("extra") && !plain.contains("columns") && !plain.contains("title"));
        let mut p = Project::empty();
        p.title = "Song".into();
        p.comments = "line one\nline two".into();
        std::fs::write(&song, serde_json::to_string(&p).unwrap()).unwrap();
        let (p, _) = Project::load(song.to_str().unwrap()).unwrap();
        assert_eq!((p.title.as_str(), p.comments.as_str()), ("Song", "line one\nline two"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn curves_pass_through_their_points() {
        let mut e = Envelope { curve: true, ..Envelope::new(1, 0, vec![(0.0, 0.0), (4.0, 1.0), (8.0, 0.0)]) };
        for (pos, v) in [(0.0, 0.0), (4.0, 1.0), (8.0, 0.0)] {
            assert_eq!(e.value_at(pos), Some(v));
        }
        let (curve, line) = (e.value_at(2.0).unwrap(), 0.5);
        assert!(curve > line, "it bows up towards the peak: {curve}");
        assert!(e.value_at(5.0).unwrap() <= 1.0, "and stays in range");
        e.steps = true;
        assert_eq!(e.value_at(2.0), Some(0.0), "points win");
        assert_eq!(MIXER_GAIN.name, ModuleKind::Eq.automatable(6).unwrap().name);
        assert_eq!(ModuleKind::Eq.automatable(7).unwrap().name, "Pan");
        assert!(ModuleKind::Eq.automatable(8).is_none());
    }

    #[test]
    fn modulation_is_saved_only_when_used() {
        let mut p = Project::empty();
        let id = p.add_module(ModuleKind::Sampler, [0.0, 0.0]).unwrap();
        assert!(!serde_json::to_string(&p).unwrap().contains("modulation"));
        let m = &mut p.module_mut(id).unwrap().modulation;
        m.pitch.on = true;
        m.pitch.sustain = Some(7);
        let dir = temp_dir("modulation");
        let song = dir.join("song.json");
        std::fs::write(&song, serde_json::to_string(&p).unwrap()).unwrap();
        let (p, _) = Project::load(song.to_str().unwrap()).unwrap();
        let m = &p.module(id).unwrap().modulation;
        assert!(m.pitch.on);
        assert_eq!(m.pitch.sustain, None, "a sustain point that isn't there goes");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn excerpts_keep_only_the_selection() {
        let mut p = Project::empty();
        p.set_columns(0, 2);
        let pat = &mut p.patterns[0];
        pat.tracks[0][4].note = Some(Note::On(60));
        pat.cell_mut(0, 1, 5).note = Some(Note::On(62));
        pat.tracks[1][5].note = Some(Note::On(64));
        pat.tracks[1][20].note = Some(Note::On(65));
        pat.automation.push(Envelope::new(0, 0, vec![(0.0, 0.0), (8.0, 1.0), (40.0, 0.0)]));
        // Lines 4 to 11 of lanes 1 and 2: track 0's second column and track 1.
        let song = p.excerpt(0, (4, 11), (1, 2));
        let pat = &song.patterns[0];
        assert_eq!((song.patterns.len(), song.order.len(), pat.lines), (1, 1, 8));
        assert_eq!(pat.tracks[0][0].note, None, "the first column wasn't selected");
        assert_eq!(pat.cell(0, 1, 1).note, Some(Note::On(62)));
        assert_eq!(pat.tracks[1][1].note, Some(Note::On(64)));
        assert!(pat.tracks[1].iter().filter(|c| c.note.is_some()).count() == 1, "line 20 is outside");
        assert_eq!(pat.automation[0].points, [(0.0, 0.5), (4.0, 1.0)]);
    }

    #[test]
    fn slice_settings_follow_markers() {
        let sample = Sample { name: "s".into(), sample_rate: 100.0, channels: 1, frames: vec![[0.0; 2]; 100] };
        let mut slot = SampleSlot::new(sample, None);
        slot.slices = vec![50];
        slot.slice_mut(1).transpose = 7;
        // Splitting the first slice: the second keeps its settings.
        slot.add_slice(20);
        assert_eq!(slot.slices, [20, 50]);
        assert_eq!(slot.slice(2).transpose, 7);
        // Splitting the last: both halves get them.
        slot.add_slice(80);
        assert_eq!((slot.slice(2).transpose, slot.slice(3).transpose), (7, 7));
        slot.remove_slice(0);
        assert_eq!(slot.slices, [50, 80]);
        assert_eq!((slot.slice(0).transpose, slot.slice(1).transpose), (0, 7));
        // Only settings that differ are saved.
        let plain = SampleSlot { slice_settings: vec![SliceSettings::default(); 3], ..slot.clone() };
        assert!(
            !serde_json::to_string(&SampleSlot { slice_settings: vec![], ..plain }).unwrap().contains("slice_settings")
        );
        assert!(serde_json::to_string(&slot).unwrap().contains("slice_settings"));
    }

    #[test]
    fn instrument_chains() {
        let mut p = Project::simple();
        // The demo: the bass through its filter, the bell through delay
        // and reverb, the drums straight out.
        let (drums, bass, bell, filter, delay, verb) = (1, 2, 3, 4, 5, 6);
        assert_eq!(p.chain(bass), Chain { effects: vec![filter], outputs: vec![OUTPUT_ID] });
        assert_eq!(p.chain(bell), Chain { effects: vec![delay, verb], outputs: vec![OUTPUT_ID] });
        assert_eq!(p.chain(drums), Chain { effects: vec![], outputs: vec![OUTPUT_ID] });

        let eq = p.chain_insert(bass, 1, ModuleKind::Eq).unwrap();
        assert_eq!(p.chain(bass).effects, [filter, eq]);
        p.chain_move(bass, eq, -1);
        assert_eq!(p.chain(bass).effects, [eq, filter]);
        let amp = p.chain_insert(bass, 2, ModuleKind::Amplifier).unwrap();
        p.chain_place(bass, amp, 0);
        assert_eq!(p.chain(bass).effects, [amp, eq, filter]);
        p.chain_place(bass, amp, 9);
        assert_eq!(p.chain(bass).effects, [eq, filter, amp]);
        p.chain_remove(bass, amp);
        p.chain_remove(bass, eq);
        assert_eq!(p.chain(bass), Chain { effects: vec![filter], outputs: vec![OUTPUT_ID] });
        assert!(p.module(eq).is_none());

        // An effect two instruments feed is shared, not part of either chain.
        p.disconnect(drums, OUTPUT_ID);
        p.connect(drums, verb);
        assert_eq!(p.chain(bell), Chain { effects: vec![delay], outputs: vec![verb] });
        assert_eq!(p.chain(drums).outputs, [verb]);
        p.disconnect(drums, verb);
        p.connect(drums, OUTPUT_ID);
        assert_eq!(p.chain(bell).effects, [delay, verb]);
        // Sending the bell's chain into its own delay would loop: refused.
        assert!(!p.connect(verb, delay));

        // A Modulator moving the filter doesn't break the bass's chain.
        let lfo = p.add_module(ModuleKind::Modulator, [0.0, 0.0]).unwrap();
        p.connect(lfo, filter);
        assert_eq!(p.chain(bass).effects, [filter]);

        // A chain going nowhere gets the output when an effect is added.
        p.links.retain(|l| l.0 != drums);
        let comp = p.chain_insert(drums, 0, ModuleKind::Compressor).unwrap();
        assert_eq!(p.chain(drums), Chain { effects: vec![comp], outputs: vec![OUTPUT_ID] });
    }

    #[test]
    fn phrases_are_saved_only_when_used() {
        let mut p = Project::empty();
        let id = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
        assert!(!serde_json::to_string(&p).unwrap().contains("phrase"));
        let mut phrase = Phrase { lines: 500, ..Phrase::default() };
        phrase.cells[1].note = Some(Note::On(50));
        let m = p.module_mut(id).unwrap();
        m.phrases = vec![Phrase::default(), phrase];
        m.selected_phrase = 1;
        m.phrase_mode = PhraseMode::Keymap;
        let dir = temp_dir("phrase");
        let song = dir.join("song.json");
        std::fs::write(&song, serde_json::to_string(&p).unwrap()).unwrap();
        let (p, _) = Project::load(song.to_str().unwrap()).unwrap();
        let m = p.module(id).unwrap();
        assert_eq!((m.phrases.len(), m.selected_phrase, m.phrase_mode), (2, 1, PhraseMode::Keymap));
        assert_eq!(m.phrases[1].cells[1].note, Some(Note::On(50)));
        assert_eq!(m.phrases[1].lines, MAX_PHRASE_LINES, "kept in range");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_single_phrase_of_older_songs_becomes_the_first() {
        let mut p = Project::empty();
        let id = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
        let mut json: serde_json::Value = serde_json::to_value(&p).unwrap();
        let m = json["modules"].as_array_mut().unwrap().iter_mut().find(|m| m["id"] == id).unwrap();
        m["phrase"] = serde_json::json!({ "on": true, "lines": 4, "lpb": 4, "looping": true, "cells": [] });
        let dir = temp_dir("old_phrase");
        let song = dir.join("song.json");
        std::fs::write(&song, json.to_string()).unwrap();
        let (p, _) = Project::load(song.to_str().unwrap()).unwrap();
        let m = p.module(id).unwrap();
        assert_eq!(m.phrase_mode, PhraseMode::Program);
        assert_eq!((m.phrases.len(), m.phrases[0].lines, m.phrases[0].looping), (1, 4, true));
        assert_eq!(m.phrases[0].keys, [0, 119]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn columns_a_track_does_not_show_read_as_empty() {
        let mut p = Pattern::new("t", 4, 64);
        let note = Cell { note: Some(Note::On(60)), ..Cell::default() };
        p.set_columns(0, 2);
        *p.cell_mut(0, 1, 3) = note;
        // The − under the name: the second note column goes, and a track
        // with no effect columns has nothing past its note columns.
        p.set_columns(0, 1);
        assert_eq!(p.column(0, 1)[3], Cell::default(), "no panic, and empty");
        assert_eq!(p.column(2, 5)[0], Cell::default(), "a track that never had columns");
        assert_eq!(p.column(9, 0)[0], Cell::default(), "a track the pattern lacks");
        // With an effect column, the same index is that column.
        p.set_fx_columns(0, 1);
        p.cell_mut(0, 1, 3).fx = Some((0x4, 0x22));
        assert_eq!(p.column(0, 1)[3].fx, Some((0x4, 0x22)));
        assert_eq!(p.column(0, 2)[3], Cell::default(), "past the effect columns");
    }

    #[test]
    fn note_columns_come_back_with_their_notes() {
        let mut p = Pattern::new("t", 2, 64);
        let note = Cell { note: Some(Note::On(60)), ..Cell::default() };
        for fx in [0, 2] {
            p.set_fx_columns(0, fx);
            p.set_columns(0, 3);
            *p.cell_mut(0, 2, 7) = note;
            p.set_columns(0, 1);
            p.set_columns(0, 3);
            assert_eq!(p.cell(0, 2, 7), note, "with {fx} effect columns");
            // The effect columns weren't touched by any of it.
            for c in 0..fx {
                assert_eq!(p.column(0, 3 + c)[7], Cell::default());
            }
            p.clear_track(0);
        }
    }

    #[test]
    fn lanes_follow_the_columns_shown() {
        let mut p = Pattern::new("t", 3, 64);
        // Any order of + and − leaves a lane for each column shown, and
        // every lane reads without panicking.
        let steps: [(usize, usize, usize); 6] = [(0, 3, 0), (0, 3, 2), (0, 1, 2), (1, 2, 1), (0, 1, 0), (1, 1, 0)];
        for (t, notes, fx) in steps {
            p.set_columns(t, notes);
            p.set_fx_columns(t, fx);
            let widths: usize = (0..3).map(|t| p.width(t)).sum();
            assert_eq!(p.num_lanes(), widths);
            for lane in 0..p.num_lanes() {
                let (track, col) = p.lane_pos(lane);
                assert_eq!(p.lane_of(track, col), lane);
                assert_eq!(p.lane(lane).len(), MAX_LINES);
            }
        }
    }

    #[test]
    fn effect_columns_are_kept_by_the_matrix_and_song_files() {
        let mut p = Project::empty();
        p.set_fx_columns(1, 2);
        p.patterns[0].cell_mut(1, 2, 5).fx = Some((0xA, 0x0F));
        p.order.push(Slot::new(0));
        p.patterns.push(Pattern::new("b", 4, 64));
        p.order[1].pattern = 1;
        let clip = p.copy_matrix((0, 0), (1, 1));
        assert_eq!((clip[0][0].notes.len(), clip[0][0].effects.len()), (1, 2));
        p.paste_matrix(&clip, 1, 1);
        assert_eq!(p.patterns[1].fx_columns(1), 2, "the effect columns come along");
        assert_eq!(p.patterns[1].cell(1, 2, 5).fx, Some((0xA, 0x0F)));
        let dir = temp_dir("fx_columns");
        let song = dir.join("song.json");
        std::fs::write(&song, serde_json::to_string(&p).unwrap()).unwrap();
        let (back, _) = Project::load(song.to_str().unwrap()).unwrap();
        assert_eq!(back.patterns[0].fx_columns(1), 2);
        assert_eq!(back.patterns[0].track_effects(1, 5).collect::<Vec<_>>(), [(0xA, 0x0F)]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn soloing_a_track_silences_the_others_until_it_is_soloed_again() {
        let mut p = Project::empty();
        p.solo_track(2);
        assert!(p.track_audible(2) && !p.track_audible(0));
        p.solo_track(1);
        assert!(p.track_audible(1) && !p.track_audible(2), "the solo moves");
        p.solo_track(1);
        assert!((0..4).all(|t| p.track_audible(t)), "and comes off");
    }

    #[test]
    fn the_demo_saves_and_opens_the_same() {
        let p = Project::demo();
        let dir = temp_dir("demo");
        let song = dir.join("demo.json");
        std::fs::write(&song, serde_json::to_string(&p).unwrap()).unwrap();
        let (back, warnings) = Project::load(song.to_str().unwrap()).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(serde_json::to_string(&back).unwrap(), serde_json::to_string(&p).unwrap());
        // Its strings play chords in three note columns, and the arp phrases.
        assert_eq!(back.patterns[1].columns(3), 3);
        assert!(back.modules.iter().any(|m| m.phrase_mode == PhraseMode::Program && !m.phrases.is_empty()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_mix_goes_through_the_master_chain() {
        let mut p = Project::empty();
        let synth = p.add_module(ModuleKind::Generator, [0.0; 2]).unwrap();
        p.connect(synth, OUTPUT_ID);
        let eq = p.chain_insert(OUTPUT_ID, 0, ModuleKind::Eq).unwrap();
        let limit = p.chain_insert(OUTPUT_ID, 1, ModuleKind::Maximizer).unwrap();
        assert_eq!(p.master, [eq, limit]);
        assert!(!p.links.iter().any(|l| l.0 == eq || l.1 == eq), "no links of their own");
        let path = p.audio_links();
        assert!(path.contains(&(synth, eq)) && path.contains(&(eq, limit)) && path.contains(&(limit, OUTPUT_ID)));
        assert!(!path.contains(&(synth, OUTPUT_ID)), "the synth reaches the output through them");
        assert!(!p.connect(synth, eq), "they aren't wired by hand");
        let lfo = p.add_module(ModuleKind::Modulator, [0.0; 2]).unwrap();
        assert!(p.connect(lfo, eq), "but Modulators move them");
        p.chain_move(OUTPUT_ID, limit, -1);
        assert_eq!(p.master, [limit, eq]);
        p.chain_remove(OUTPUT_ID, limit);
        assert_eq!(p.master, [eq]);
        assert!(p.module(limit).is_none());
        // Saved and loaded, it stays; a stray link to it or a wrong id goes.
        p.links.push((synth, eq));
        p.master.extend([eq, 200, synth]);
        let path = std::env::temp_dir().join(format!("noise-master-{}.json", std::process::id()));
        std::fs::write(&path, serde_json::to_string(&p).unwrap()).unwrap();
        let (back, _) = Project::load(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(path).ok();
        assert_eq!(back.master, [eq]);
        assert!(!back.links.contains(&(synth, eq)));
    }

    #[test]
    fn songs_saved_not_to_loop_end_on_an_f00() {
        let mut p = Project::empty();
        p.patterns[0].lines = 8;
        p.patterns[0].tracks[0][7].fx = Some((0x1, 0x10));
        let mut json: serde_json::Value = serde_json::to_value(&p).unwrap();
        assert!(json.get("loop_song").is_none(), "no longer saved");
        let path = std::env::temp_dir().join(format!("noise-loop-song-{}.json", std::process::id()));
        for looping in [true, false] {
            json["loop_song"] = serde_json::Value::Bool(looping);
            std::fs::write(&path, json.to_string()).unwrap();
            let (back, _) = Project::load(path.to_str().unwrap()).unwrap();
            let last = |t: usize| back.patterns[0].tracks[t][7].fx;
            assert_eq!(last(0), Some((0x1, 0x10)), "an effect there stays");
            assert_eq!(last(1), (!looping).then_some((0xF, 0)), "the next track's cell gets the F00");
        }
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn tracks_keep_their_own_effects() {
        let mut p = Project::empty();
        let synth = p.add_module(ModuleKind::Generator, [0.0; 2]).unwrap();
        p.connect(synth, OUTPUT_ID);
        p.patterns[0].tracks[2][0] = Cell { note: Some(Note::On(60)), module: Some(synth), ..Cell::default() };
        let echo = p.chain_insert(Owner::Track(2), 0, ModuleKind::Echo).unwrap();
        assert_eq!(p.tracks[2].effects, [echo]);
        assert!(p.placed(echo) && !p.connect(synth, echo), "wired by its place, not by hand");
        assert_eq!(p.track_instruments(2), [synth]);
        // A Modulator listening halfway along the synth's own chain doesn't
        // cut it short: the chain goes on to the output past it.
        let amp = p.chain_insert(synth, 0, ModuleKind::Amplifier).unwrap();
        let lim = p.chain_insert(synth, 1, ModuleKind::Maximizer).unwrap();
        let duck = p.add_module(ModuleKind::Modulator, [0.0; 2]).unwrap();
        assert!(p.connect(amp, duck));
        assert_eq!(p.chain(synth).effects, [amp, lim]);
        // The path: the synth's copy for track 2, with its chain, goes
        // through the echo.
        let (nodes, links) = p.signal_graph();
        assert!(nodes.contains(&(synth, Some(2))) && nodes.contains(&(lim, Some(2))));
        assert!(links.contains(&((lim, Some(2)), (echo, None))) && links.contains(&((echo, None), (OUTPUT_ID, None))));
        // Saved and loaded, it stays; a stray link to it goes.
        p.links.push((synth, echo));
        let path = std::env::temp_dir().join(format!("noise-track-fx-{}.json", std::process::id()));
        std::fs::write(&path, serde_json::to_string(&p).unwrap()).unwrap();
        let (mut back, _) = Project::load(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(path).ok();
        assert_eq!(back.tracks[2].effects, [echo]);
        assert!(!back.links.contains(&(synth, echo)));
        // A track that goes takes its effects with it.
        back.remove_track(2);
        assert!(back.module(echo).is_none());
    }

    #[test]
    fn grouped_tracks_go_on_through_the_group_s_effects() {
        let mut p = Project::empty();
        for _ in 0..3 {
            p.insert_track(0, 0);
        }
        let synth = p.add_module(ModuleKind::Generator, [0.0; 2]).unwrap();
        p.connect(synth, OUTPUT_ID);
        for t in [0, 1] {
            p.patterns[0].tracks[t][0] = Cell { note: Some(Note::On(60)), module: Some(synth), ..Cell::default() };
        }
        let echo = p.chain_insert(Owner::Track(0), 0, ModuleKind::Echo).unwrap();
        let bus = p.chain_insert(Owner::Track(2), 0, ModuleKind::Compressor).unwrap();
        p.tracks[0].group = Some(2);
        p.tracks[1].group = Some(2);
        let (nodes, links) = p.signal_graph();
        // Track 0's echo goes into the bus; track 1, without effects of its
        // own, goes straight in; the bus goes to the output.
        assert!(links.contains(&((echo, None), (bus, None))));
        assert!(nodes.contains(&(synth, Some(1))) && links.contains(&((synth, Some(1)), (bus, None))));
        assert!(links.contains(&((bus, None), (OUTPUT_ID, None))));
        // No loops: track 2 can't go through track 0, which goes through it.
        assert!(!p.can_group(2, 0) && !p.can_group(2, 2) && p.can_group(2, 3));
        // Tracks moving keep their groups; the group's going ends them.
        p.insert_track(0, 0);
        assert_eq!((p.tracks[1].group, p.tracks[2].group), (Some(3), Some(3)));
        p.remove_track(3);
        assert_eq!((p.tracks[1].group, p.tracks[2].group), (None, None));
    }

    #[test]
    fn shorter_envelopes_repeat_or_hold() {
        // A ramp over a quarter of a 64-line pattern: 16 lines.
        let mut env = Envelope::new(1, 0, vec![(0.0, 0.0), (16.0, 1.0)]);
        env.length = 0.25;
        assert_eq!(env.span(64), 16.0);
        assert_eq!(env.value_in(8.0, 64), Some(0.5));
        assert_eq!(env.value_in(40.0, 64), Some(1.0), "past its end it holds");
        env.repeat = true;
        assert_eq!(env.value_in(40.0, 64), Some(0.5), "or starts again");
        assert_eq!(env.value_in(56.0, 64), Some(0.5));
        // A whole-pattern one is as it was, and saves as it did.
        let whole = Envelope::new(1, 0, vec![(0.0, 0.0), (64.0, 1.0)]);
        assert_eq!(whole.value_in(32.0, 64), Some(0.5));
        let json = serde_json::to_string(&whole).unwrap();
        assert!(!json.contains("length") && !json.contains("repeat"), "{json}");
    }

    #[test]
    fn slots_move_to_where_they_are_dropped() {
        let mut p = Project::empty();
        p.order = (0..4).map(Slot::new).collect();
        let patterns = |p: &Project| p.order.iter().map(|s| s.pattern).collect::<Vec<_>>();
        p.move_slot(0, 2);
        assert_eq!(patterns(&p), [1, 2, 0, 3]);
        p.move_slot(3, 0);
        assert_eq!(patterns(&p), [3, 1, 2, 0]);
        p.move_slot(1, 9);
        assert_eq!(patterns(&p), [3, 2, 0, 1], "past the end is the end");
    }

    #[test]
    fn sections_follow_their_slots() {
        let mut p = Project::empty();
        for _ in 0..4 {
            p.insert_slot(1, Slot::new(0));
        }
        p.set_section(0, "Intro".into());
        p.set_section(2, "Verse".into());
        assert_eq!(p.section_slots(0), (0, 1));
        assert_eq!(p.section_slots(2), (2, 4));
        p.insert_slot(1, Slot::new(0));
        assert_eq!(p.sections[1].start, 3, "pushed down");
        p.insert_slot(0, Slot::new(0));
        assert_eq!(p.sections[0].start, 0, "the first section stays at the top");
        p.remove_slot(4);
        assert_eq!(p.sections[1].start, 4, "the section's next slot starts it");
        for _ in 0..3 {
            p.remove_slot(0);
        }
        assert_eq!(p.sections.iter().map(|s| s.start).collect::<Vec<_>>(), [0, 1]);
        p.remove_slot(0);
        assert_eq!(p.sections.len(), 1, "sections that meet merge");
        assert_eq!(p.sections[0].name, "Intro");
    }

    #[test]
    fn matrix_blocks_copy_tracks_between_slots() {
        let mut p = Project::empty();
        p.patterns.push(Pattern::new("b", 4, 64));
        p.order.push(Slot::new(1));
        p.set_columns(1, 2);
        p.patterns[0].cell_mut(1, 1, 3).note = Some(Note::On(60));
        p.patterns[0].tracks[2][0].note = Some(Note::On(50));
        let clip = p.copy_matrix((0, 0), (1, 2));
        p.paste_matrix(&clip, 1, 0);
        assert_eq!(p.patterns[1].cell(0, 1, 3).note, Some(Note::On(60)), "columns come along");
        assert_eq!(p.patterns[1].columns(0), 2);
        assert_eq!(p.patterns[1].tracks[1][0].note, Some(Note::On(50)));
        p.clear_matrix((0, 1), (0, 1));
        assert!(!p.patterns[1].track_used(0) && !p.patterns[0].track_used(1));
        assert!(p.patterns[0].track_used(2), "outside the block");
    }

    #[test]
    fn new_params_start_at_their_defaults() {
        let dir = temp_dir("params");
        let mut p = Project::empty();
        let id = p.add_module(ModuleKind::Distortion, [0.0, 0.0]).unwrap();
        p.module_mut(id).unwrap().params.truncate(3);
        let song = dir.join("old.json");
        std::fs::write(&song, serde_json::to_string(&p).unwrap()).unwrap();
        let (p, _) = Project::load(song.to_str().unwrap()).unwrap();
        let defaults: Vec<f32> = DISTORTION_PARAMS.iter().map(|s| s.default).collect();
        assert_eq!(p.module(id).unwrap().params, defaults);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn slots_without_mutes_save_as_numbers() {
        let mut slot = Slot::new(3);
        assert_eq!(serde_json::to_string(&slot).unwrap(), "3");
        slot.toggle_mute(2);
        let json = serde_json::to_string(&slot).unwrap();
        assert_eq!(json, r#"{"pattern":3,"muted":4}"#);
        assert_eq!(serde_json::from_str::<Slot>(&json).unwrap(), slot);
        assert!(slot.is_muted(2) && !slot.is_muted(1));
        let old: Vec<Slot> = serde_json::from_str("[0, 1, 1]").unwrap();
        assert_eq!(old, [Slot::new(0), Slot::new(1), Slot::new(1)]);
    }

    #[test]
    fn envelopes_interpolate_between_points() {
        let mut e = Envelope::new(1, 0, vec![]);
        assert_eq!(e.value_at(3.0), None);
        e.set(4.0, 1.0);
        e.set(0.0, 0.0);
        e.set(8.0, 0.5);
        assert_eq!(e.points, [(0.0, 0.0), (4.0, 1.0), (8.0, 0.5)]);
        assert_eq!(e.value_at(2.0), Some(0.5));
        assert_eq!(e.value_at(6.0), Some(0.75));
        assert_eq!(e.value_at(20.0), Some(0.5), "the last point holds");
        e.set(4.0, 0.25);
        assert_eq!(e.points.len(), 3, "setting a point that is there moves it");
        e.steps = true;
        assert_eq!(e.value_at(7.9), Some(0.25));
    }

    #[test]
    fn sample_files_round_trip() {
        let dir = temp_dir("roundtrip");
        for channels in [1, 2] {
            let path = dir.join(format!("{channels}.wav"));
            write_wav(&path, 1000, channels);
            let s = Sample::load(&path).unwrap();
            assert_eq!((s.len(), s.channels, s.sample_rate), (1000, channels, 22050.0));
            assert_eq!(s.frames[500][0], 0.5);
            // Mono files hold the left channel in both.
            let right = if channels == 1 { 0.5 } else { -0.5 };
            assert_eq!(s.frames[500][1], right);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn old_samplers_become_sample_slots() {
        let dir = temp_dir("migrate");
        write_wav(&dir.join("pad.wav"), 1000, 2);
        let mut p = Project::empty();
        let id = p.add_module(ModuleKind::Sampler, [0.0, 0.0]).unwrap();
        let mut json = serde_json::to_value(&p).unwrap();
        // Volume, root note, fine tune, attack, release, loop (ping-pong),
        // start, loop start, loop end, pan.
        let m = json["modules"].as_array_mut().unwrap().iter_mut().find(|m| m["id"] == id).unwrap();
        m["params"] = serde_json::json!([0.9, 50.0, 12.0, 0.01, 0.2, 2.0, 0.0, 0.25, 0.75, -0.5]);
        m["sample_path"] = "pad.wav".into();
        let song = dir.join("old.json");
        std::fs::write(&song, json.to_string()).unwrap();

        let (p, warnings) = Project::load(song.to_str().unwrap()).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let m = p.module(id).unwrap();
        assert_eq!(m.params, vec![0.9, -0.5, 0.0, 0.01, 0.5, 1.0, 0.2]);
        let s = &m.samples[0];
        assert_eq!((s.base_note, s.finetune, s.loop_mode), (50, 12, 3));
        assert_eq!((s.loop_start, s.loop_end), (250, 750));
        assert_eq!(s.len(), 1000);

        // Saved again, the old field is gone and the slot comes back.
        let again = serde_json::to_string(&p).unwrap();
        assert!(!again.contains("sample_path"));
        std::fs::write(&song, again).unwrap();
        let (p2, _) = Project::load(song.to_str().unwrap()).unwrap();
        assert_eq!(p2.module(id).unwrap().samples[0].loop_end, 750);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_samples_are_warnings() {
        let dir = temp_dir("missing");
        let mut p = Project::empty();
        let id = p.add_module(ModuleKind::Sampler, [0.0, 0.0]).unwrap();
        p.module_mut(id).unwrap().samples.push(SampleSlot { path: Some("gone.wav".into()), ..Default::default() });
        let song = dir.join("song.json");
        std::fs::write(&song, serde_json::to_string(&p).unwrap()).unwrap();
        let (p, warnings) = Project::load(song.to_str().unwrap()).unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(p.module(id).unwrap().samples[0].data.is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
