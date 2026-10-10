//! A module: its kind and settings, mixer strip, samples, modulation,
//! phrases and macros.

use super::*;

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
    /// The module whose sound a Compressor, Gate or Vocoder listens to, its key
    /// input, instead of its own: the sidechain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<u8>,
    /// An instrument's macros, `MACROS` of them once any is used.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub macros: Vec<Macro>,
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
    pub(super) old_phrase: Option<OldPhrase>,
    /// The pitch and filter envelopes and LFOs of a Sampler, Generator or
    /// FM's voices.
    #[serde(default, skip_serializing_if = "Modulation::is_default")]
    pub modulation: Modulation,
    /// The single sample of Samplers saved before they had sample slots;
    /// read when loading and turned into a slot.
    #[serde(default, skip_serializing)]
    pub(super) sample_path: Option<String>,
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
            i if i == n + 1 => self.pan,
            i => self.macro_value(i - n - 2),
        }
    }

    /// Where macro `k` is turned to, 0..1.
    pub fn macro_value(&self, k: usize) -> f32 {
        self.macros.get(k).map_or(0.0, |m| m.value)
    }

    /// Macro `k`, made with the others the first time one is used.
    pub fn macro_mut(&mut self, k: usize) -> &mut Macro {
        if self.macros.len() < MACROS {
            self.macros.resize(MACROS, Macro::default());
        }
        &mut self.macros[k.min(MACROS - 1)]
    }

    /// Macro `k`'s name: its own, or "Macro 1" and on.
    pub fn macro_name(&self, k: usize) -> String {
        match self.macros.get(k) {
            Some(m) if !m.name.is_empty() => m.name.clone(),
            _ => format!("Macro {}", k + 1),
        }
    }

    /// What automatable parameter `i` is called in lists: its name, the
    /// mixer's "Mixer Fader" and "Mixer Pan", or a macro's name.
    pub fn automatable_name(&self, i: usize) -> String {
        let n = self.kind.params().len();
        match self.kind.automatable(i) {
            Some(spec) if i < n => spec.name.to_string(),
            Some(spec) if i < n + 2 => format!("Mixer {}", spec.name),
            Some(_) => self.macro_name(i - n - 2),
            None => "?".to_string(),
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
            key: None,
            macros: Vec::new(),
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
        self.macros = other.macros.clone();
        (self.phrase_mode, self.selected_phrase) = (other.phrase_mode, other.selected_phrase);
    }
}

/// A knob of an instrument's that moves several parameters at once.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Macro {
    /// Empty for "Macro 1" and on.
    pub name: String,
    /// Where it is turned to, 0..1.
    pub value: f32,
    pub targets: Vec<MacroTarget>,
}

/// A parameter a macro moves: automatable parameter `param` of `module`,
/// from `from` with the macro at 0 to `to` at 1, as places along its
/// range (0..1) as its bar shows them.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MacroTarget {
    pub module: u8,
    pub param: usize,
    pub from: f32,
    pub to: f32,
}
