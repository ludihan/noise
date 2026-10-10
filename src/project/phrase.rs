//! Phrases: short patterns an instrument plays for its notes, and how
//! notes pick one.

use super::*;

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
pub(super) struct OldPhrase {
    #[serde(default)]
    pub(super) on: bool,
    #[serde(flatten)]
    pub(super) phrase: Phrase,
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

    pub(super) fn is_off(&self) -> bool {
        *self == PhraseMode::Off
    }
}

/// The most phrases an instrument has, as `Zxx` can pick from.
pub const MAX_PHRASES: usize = 0x7E;

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

    pub(super) fn normalize(&mut self) {
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
