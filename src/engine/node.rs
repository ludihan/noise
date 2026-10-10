//! A module's place in the sound's path: its DSP, inputs, buffer and the
//! parameters envelopes, Modulators and macros set for the block.

use super::*;

pub(super) struct Node {
    pub(super) id: u8,
    /// For a copy made for a track with effects of its own, that track.
    pub(super) track: Option<usize>,
    pub(super) kind: ModuleKind,
    /// Index into `project.modules`.
    pub(super) module: usize,
    pub(super) dsp: Box<dyn Dsp>,
    pub(super) inputs: Vec<usize>,
    /// For an effect with a key input, the nodes whose sound it listens to:
    /// the module keying it and its copies for tracks.
    pub(super) key: Vec<usize>,
    pub(super) buf: Vec<Frame>,
    /// For a module an effect listens to as its key input: its sound
    /// before its mixer strip, so a muted or faded-down kick still keys.
    pub(super) dry: Vec<Frame>,
    /// Off while another module is soloed and this one neither feeds it
    /// nor is fed by it.
    pub(super) audible: bool,
    /// Output peak since the meters were last published.
    pub(super) peak: [f32; 2],
    /// The module's parameters with the playing pattern's automation
    /// applied, used instead of the song's while `automated`.
    pub(super) params: Vec<f32>,
    pub(super) automated: bool,
    /// The mixer fader and pan envelopes set, used instead of the song's.
    pub(super) mix_gain: Option<f32>,
    pub(super) mix_pan: Option<f32>,
    /// For an instrument, where envelopes and Modulators turn its macros,
    /// used instead of the song's, and what the macros move: macro, node,
    /// automatable parameter and the range it moves it over.
    pub(super) macros: [Option<f32>; MACROS],
    pub(super) macro_targets: Vec<(usize, usize, usize, f32, f32)>,
    /// For a MultiSynth: the nodes it passes notes to, the notes it holds
    /// and the next round-robin target.
    pub(super) targets: Vec<usize>,
    pub(super) held: Vec<Held>,
    pub(super) next_target: usize,
    /// For a Glide: the last note on each key, sliding.
    pub(super) glides: Vec<GlideVoice>,
    /// For a Modulator: the nodes and automatable parameters it moves,
    /// its LFO's phase and the level it follows.
    pub(super) controls: Vec<(usize, usize)>,
    pub(super) phase: f32,
    pub(super) follow: f32,
    /// The last note an instrument was played, how hard (0..1), and when
    /// (by `Engine::notes_played`), for Modulators tracking it. A Modulator
    /// in Envelope mode keeps the when of the note that started it here.
    pub(super) last_note: (f32, f32, u64),
}

/// `into`, cleared, with the sound of nodes `from` added up in it: what
/// `sound` picks of each, its output or its dry sound.
pub(super) fn mix(into: &mut [Frame], nodes: &[Node], from: &[usize], sound: fn(&Node) -> &[Frame]) {
    into.fill([0.0; 2]);
    for &j in from {
        for (s, x) in into.iter_mut().zip(sound(&nodes[j])) {
            s[0] += x[0];
            s[1] += x[1];
        }
    }
}

impl Node {
    /// Automatable parameter `param` of the node's module `m` (see
    /// `ModuleKind::automatable`) as it is this block: the song's value
    /// or what an envelope, Modulator or macro set.
    pub(super) fn param(&self, m: &crate::project::Module, param: usize) -> f32 {
        let n = m.params.len();
        match param {
            p if p < n && self.automated => self.params[p],
            p if p < n => m.params[p],
            p if p == n => self.mix_gain.unwrap_or(m.gain),
            p if p == n + 1 => self.mix_pan.unwrap_or(m.pan),
            p => self.macros.get(p - n - 2).copied().flatten().unwrap_or_else(|| m.macro_value(p - n - 2)),
        }
    }

    /// Sets automatable parameter `param` for this block.
    pub(super) fn set_param(&mut self, m: &crate::project::Module, param: usize, value: f32) {
        let n = m.params.len();
        match param {
            p if p < n => {
                if !self.automated {
                    self.params.clear();
                    self.params.extend_from_slice(&m.params);
                    self.automated = true;
                }
                self.params[p] = value;
            }
            p if p == n => self.mix_gain = Some(value),
            p if p == n + 1 => self.mix_pan = Some(value),
            p => {
                if let Some(v) = self.macros.get_mut(p - n - 2) {
                    *v = Some(value);
                }
            }
        }
    }
}
