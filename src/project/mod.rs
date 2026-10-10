//! Song data: patterns, the order list and the module graph. The song
//! itself, its tracks and slots and how sound finds its way through the
//! modules are here; patterns, module kinds, modules, voice modulation,
//! phrases, effect commands and sample slots have files of their own.
//!
//! The UI owns a `Project` and sends immutable snapshots of it to the audio
//! thread whenever it changes. Nothing in here touches DSP state.

use crate::sample::Sample;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod commands;
mod kinds;
mod module;
mod pattern;
mod phrase;
mod sample_slot;
mod voice;

pub use commands::*;
pub use kinds::*;
pub use module::*;
pub use pattern::*;
pub use phrase::*;
pub use sample_slot::*;
pub use voice::*;

pub const MAX_TRACKS: usize = 32;
/// Note columns a track can have.
pub const MAX_COLUMNS: usize = 8;
/// Effect columns a track can have.
pub const MAX_FX_COLUMNS: usize = 8;
pub const MAX_LINES: usize = 256;
pub const OUTPUT_ID: u8 = 0;

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
        for m in &mut self.modules {
            m.key = m.key.filter(|&k| k != id);
            for mac in &mut m.macros {
                mac.targets.retain(|t| t.module != id);
            }
        }
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

    /// Whether module `effect` can listen to `source` as its key input: it
    /// takes one, `source` makes sound, and neither the effect's sound nor
    /// what it keys comes back round to `source`, which would loop.
    pub fn can_key(&self, effect: u8, source: u8) -> bool {
        let (Some(e), Some(s)) = (self.module(effect), self.module(source)) else { return false };
        if effect == source || !e.kind.takes_key() || !s.kind.makes_sound() || !s.kind.has_output() {
            return false;
        }
        let mut links: Vec<(u8, u8)> = self.signal_graph().1.iter().map(|(a, b)| (a.0, b.0)).collect();
        links.extend(self.modules.iter().filter(|m| m.id != effect).filter_map(|m| Some((m.key?, m.id))));
        let mut stack = vec![effect];
        let mut seen = Vec::new();
        while let Some(n) = stack.pop() {
            if n == source {
                return false;
            }
            if !seen.contains(&n) {
                seen.push(n);
                stack.extend(links.iter().filter(|l| l.0 == n).map(|l| l.1));
            }
        }
        true
    }

    /// The instrument whose macros can move module `id`: itself, or the
    /// instrument whose own chain it is in.
    pub fn macro_owner(&self, id: u8) -> Option<u8> {
        let m = self.module(id)?;
        if m.kind.plays_sound() {
            return Some(id);
        }
        let mut owners = self.modules.iter().filter(|i| i.kind.plays_sound());
        owners.find(|i| self.chain(i.id).effects.contains(&id)).map(|i| i.id)
    }

    /// Makes macro `k` of instrument `owner` move parameter `param` of
    /// `module`, from where it is now to the far end of its range.
    pub fn map_macro(&mut self, owner: u8, k: usize, module: u8, param: usize) {
        let Some(m) = self.module(module) else { return };
        let Some(spec) = m.kind.automatable(param) else { return };
        let from = spec.position(m.automatable_value(param));
        let to = if from < 0.5 { 1.0 } else { 0.0 };
        let Some(inst) = self.module_mut(owner) else { return };
        let mac = inst.macro_mut(k);
        mac.targets.retain(|t| (t.module, t.param) != (module, param));
        mac.targets.push(MacroTarget { module, param, from, to });
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
        found.retain(|&m| self.module(m).is_some_and(|m| m.kind.plays_sound()));
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
mod tests;
