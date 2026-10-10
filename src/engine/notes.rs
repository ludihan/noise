//! Notes on their way to instruments: MultiSynths passing them on, Glides
//! sliding them and phrases playing them.

use super::*;

impl Engine {
    /// Whether module `id` exists and takes notes.
    pub(super) fn plays_notes(&self, id: u8) -> bool {
        self.nodes.iter().any(|n| n.id == id && n.kind.is_instrument())
    }

    /// Sends a note event to module `id`.
    pub(super) fn note(&mut self, id: u8, key: u32, ev: NoteEv) {
        if let Some(i) = self.nodes.iter().position(|n| n.id == id && n.track.is_none()) {
            self.note_to(self.copy_for(i, key), key, ev, 0);
        }
    }

    /// Node `i`'s copy for the track playing `key`, if that track has
    /// effects and so a copy; otherwise `i`.
    pub(super) fn copy_for(&self, i: usize, key: u32) -> usize {
        let key = key as usize;
        if key >= MAX_TRACKS * MAX_COLUMNS {
            return i;
        }
        self.copies.get(i).and_then(|c| c[key % MAX_TRACKS]).unwrap_or(i)
    }

    pub(super) fn random(&mut self) -> f32 {
        self.rng.unit()
    }

    /// Sends a note event to node `i`; a MultiSynth changes it and passes
    /// it on to the instruments it is connected to.
    pub(super) fn note_to(&mut self, i: usize, key: u32, ev: NoteEv, depth: u8) {
        // An instrument with phrases may play one instead.
        let project = self.project.clone();
        let module = &project.modules[self.nodes[i].module];
        if !module.phrases.is_empty() && module.kind.makes_sound() && self.phrase_event(i, module, key, ev) {
            return;
        }
        let node = &mut self.nodes[i];
        if !node.kind.notes_only() {
            match ev {
                NoteEv::On(note, vel) => {
                    self.notes_played += 1;
                    node.last_note = (note, vel, self.notes_played);
                }
                NoteEv::Pitch(note) => node.last_note.0 = note,
                _ => {}
            }
            ev.apply(node.dsp.as_mut(), key);
            return;
        }
        if node.kind == ModuleKind::Glide {
            self.glide_event(i, key, ev, depth);
            return;
        }
        if depth >= MAX_NOTE_DEPTH || node.targets.is_empty() {
            return;
        }
        let project = &self.project;
        let p = if node.automated { &node.params } else { &project.modules[node.module].params };
        let (mode, transpose, finetune, spread, vel_scale, vel_spread, low, high) =
            (p[0].round() as u32, p[1], p[2], p[3], p[4], p[5], p[6], p[7]);
        let held = node.held.iter().position(|h| h.key == key);
        let held = match ev {
            NoteEv::On(note, _) => {
                if let Some(h) = held {
                    node.held.swap_remove(h);
                }
                if note < low.round() || note > high.round() || node.held.len() >= MAX_HELD {
                    return;
                }
                let n = self.nodes[i].targets.len();
                let target = match mode {
                    0 => None,
                    1 => {
                        let t = self.nodes[i].next_target % n;
                        self.nodes[i].next_target = (t + 1) % n;
                        Some(t)
                    }
                    _ => Some(((self.random() * n as f32) as usize).min(n - 1)),
                };
                let detune = transpose + (finetune + spread * (2.0 * self.random() - 1.0)) / 100.0;
                let vel = vel_scale * (1.0 - vel_spread * self.random());
                let h = Held { key, detune, vel, target };
                self.nodes[i].held.push(h);
                h
            }
            _ => match held {
                Some(h) => node.held[h],
                None => return,
            },
        };
        if ev == NoteEv::Off {
            self.nodes[i].held.retain(|h| h.key != key);
        }
        let ev = ev.moved(held.detune, held.vel);
        let n = self.nodes[i].targets.len();
        for t in 0..n {
            if held.target.is_none_or(|h| h == t) {
                let j = self.copy_for(self.nodes[i].targets[t], key);
                self.note_to(j, key, ev, depth + 1);
            }
        }
    }

    /// Passes `ev` from note module `i` on to every instrument it is
    /// connected to.
    pub(super) fn pass_on(&mut self, i: usize, key: u32, ev: NoteEv, depth: u8) {
        if depth >= MAX_NOTE_DEPTH {
            return;
        }
        for t in 0..self.nodes[i].targets.len() {
            let j = self.copy_for(self.nodes[i].targets[t], key);
            self.note_to(j, key, ev, depth + 1);
        }
    }

    /// A note event for Glide node `i`: a new note starts where the last
    /// one on its key was and slides to its own pitch, in Time whatever
    /// the distance. In Legato mode only a note played over the last one
    /// slides, and goes on sounding rather than starting again.
    pub(super) fn glide_event(&mut self, i: usize, key: u32, ev: NoteEv, depth: u8) {
        let node = &self.nodes[i];
        let p = if node.automated { &node.params } else { &self.project.modules[node.module].params };
        let (legato, time) = (p[0] >= 0.5, p[1].max(0.001));
        let at = node.glides.iter().position(|g| g.key == key);
        let glides = &mut self.nodes[i].glides;
        match ev {
            NoteEv::On(note, vel) => {
                let last = at.map(|a| glides[a]);
                let tied = last.is_some_and(|g| g.sounding);
                let from = match last {
                    Some(g) if tied || !legato => g.at,
                    _ => note,
                };
                let g = GlideVoice {
                    key,
                    at: from,
                    to: note,
                    rate: (note - from).abs() / time,
                    sounding: true,
                    off_pending: false,
                };
                match at {
                    Some(a) => glides[a] = g,
                    None if glides.len() < MAX_HELD => glides.push(g),
                    None => {}
                }
                if legato && tied {
                    self.pass_on(i, key, NoteEv::Vel(vel), depth);
                } else {
                    // Starting again, with the last note let go first if it
                    // was held back.
                    if last.is_some_and(|g| g.off_pending) {
                        self.pass_on(i, key, NoteEv::Off, depth);
                    }
                    self.pass_on(i, key, NoteEv::On(from, vel), depth);
                }
            }
            NoteEv::Off => match at {
                Some(a) if legato => glides[a].off_pending = glides[a].sounding,
                _ => {
                    if let Some(a) = at {
                        glides[a].sounding = false;
                    }
                    self.pass_on(i, key, ev, depth);
                }
            },
            NoteEv::Pitch(note) => {
                // Slides and vibrato move the note it goes to; while it
                // still glides there, it keeps gliding.
                match at.map(|a| &mut glides[a]) {
                    Some(g) if g.at != g.to => g.to = note,
                    Some(g) => {
                        (g.at, g.to) = (note, note);
                        self.pass_on(i, key, ev, depth);
                    }
                    None => self.pass_on(i, key, ev, depth),
                }
            }
            ev => self.pass_on(i, key, ev, depth),
        }
    }

    /// Moves every Glide's notes on by `frames`, and lets go of the notes
    /// it held back that no new note followed.
    pub(super) fn run_glides(&mut self, frames: usize) {
        let dt = frames as f32 / self.sr;
        for i in 0..self.nodes.len() {
            if self.nodes[i].glides.is_empty() {
                continue;
            }
            for k in 0..self.nodes[i].glides.len() {
                let g = &mut self.nodes[i].glides[k];
                let key = g.key;
                if std::mem::take(&mut g.off_pending) {
                    g.sounding = false;
                    self.pass_on(i, key, NoteEv::Off, 0);
                    continue;
                }
                let g = &mut self.nodes[i].glides[k];
                if g.at != g.to {
                    let step = g.rate.max(1e-3) * dt;
                    g.at = if g.at < g.to { (g.at + step).min(g.to) } else { (g.at - step).max(g.to) };
                    let at = g.at;
                    self.pass_on(i, key, NoteEv::Pitch(at), 0);
                }
            }
        }
    }

    /// Whether a Glide is sliding a note, so rendering goes in small steps.
    pub(super) fn gliding(&self) -> bool {
        self.nodes.iter().any(|n| n.glides.iter().any(|g| g.at != g.to || g.off_pending))
    }

    /// Forgets the notes MultiSynths and Glides hold, and the phrases
    /// playing.
    pub(super) fn forget_held(&mut self) {
        for n in &mut self.nodes {
            n.held.clear();
            n.glides.clear();
        }
        self.phrases.clear();
    }

    /// Samples a line of `phrase` takes at the song's tempo.
    pub(super) fn phrase_line(&self, lpb: u32) -> f64 {
        self.samples_per_tick() * self.tpl.max(1) as f64 * self.project.lpb.max(1) as f64 / lpb.max(1) as f64
    }

    /// The phrase of `module` that note `note` plays: the one a `Zxx`
    /// picked, or else the one its phrase mode picks.
    pub(super) fn pick_phrase(&self, module: &crate::project::Module, note: f32) -> Option<usize> {
        let phrase = match self.picked_phrase {
            Some(0) => return None,
            Some(z) => z as usize - 1,
            None => match module.phrase_mode {
                PhraseMode::Off => return None,
                PhraseMode::Program => module.selected_phrase,
                PhraseMode::Keymap => module.phrases.iter().position(|p| p.has_key(note))?,
            },
        };
        (phrase < module.phrases.len()).then_some(phrase)
    }

    /// Handles a note event for node `i` (instrument `module`, which has
    /// phrases). Returns false for events that go to the instrument as
    /// they are: notes that play no phrase, and what follows them.
    pub(super) fn phrase_event(&mut self, i: usize, module: &crate::project::Module, key: u32, ev: NoteEv) -> bool {
        let id = module.id;
        let at = self.phrases.iter().position(|p| p.key == key && p.module == id);
        match ev {
            NoteEv::On(note, vel) => {
                if let Some(at) = at {
                    let old = self.phrases.swap_remove(at);
                    if old.track.sounding.is_some() {
                        self.nodes[i].dsp.note_off(key);
                    }
                }
                let Some(phrase) = self.pick_phrase(module, note) else { return false };
                if self.phrases.len() < MAX_PHRASES {
                    let track = Track { module: Some(id), ..Track::default() };
                    let player = PhrasePlayer {
                        module: id,
                        phrase,
                        key,
                        transpose: note - 48.0,
                        vel,
                        playing: 0,
                        line: 0,
                        tick: 0,
                        wait: 0.0,
                        track,
                    };
                    self.phrases.push(player);
                    // The first line plays at once.
                    let last = self.phrases.len() - 1;
                    self.advance_phrase(last, 0.0);
                }
                true
            }
            NoteEv::Off => {
                // The phrase's last note may still sound after it ended, so
                // the instrument gets the note-off either way.
                if let Some(at) = at {
                    self.phrases.swap_remove(at);
                }
                false
            }
            NoteEv::Pitch(note) => {
                let Some(at) = at else { return false };
                let p = &mut self.phrases[at];
                p.transpose = note - 48.0;
                if p.track.sounding.is_some() {
                    let p = *p;
                    p.send(self.nodes[i].dsp.as_mut(), NoteEv::Pitch(p.track.pitch));
                }
                true
            }
            NoteEv::Vel(vel) => {
                let Some(at) = at else { return false };
                let p = &mut self.phrases[at];
                p.vel = vel;
                if p.track.sounding.is_some() {
                    let p = *p;
                    p.send(self.nodes[i].dsp.as_mut(), NoteEv::Vel(p.track.sent_vel));
                }
                true
            }
            NoteEv::Seek(_) => {
                // Phrases don't seek: one started before playback did
                // stays silent, as other instruments do.
                let Some(at) = at else { return false };
                self.phrases.swap_remove(at);
                self.nodes[i].dsp.note_off(key);
                true
            }
            NoteEv::Pan(_) | NoteEv::Offset(_) | NoteEv::Reverse(_) | NoteEv::Slice(_) => false,
        }
    }

    /// `cell`, without its note when a `Yxx` in `effects` decides it
    /// doesn't play this time: it plays with a chance of xx in FF.
    pub(super) fn maybe(&mut self, mut cell: Cell, effects: &Effects) -> Cell {
        if let Some(chance) = effects.find(FX_MAYBE)
            && matches!(cell.note, Some(Note::On(_)))
            && self.random() * 255.0 >= chance as f32
        {
            cell.note = None;
        }
        cell
    }

    /// Moves phrase player `k` on by `frames`, playing the lines and
    /// ticks it reaches. Returns false once a phrase that doesn't loop is
    /// over.
    pub(super) fn advance_phrase(&mut self, k: usize, frames: f64) -> bool {
        let project = self.project.clone();
        let mut p = self.phrases[k];
        let base = self.nodes.iter().position(|n| n.id == p.module && n.track.is_none());
        let (Some(i), Some(module)) = (base.map(|b| self.copy_for(b, p.key)), project.module(p.module)) else {
            return false;
        };
        let Some(phrase) = module.phrases.get(p.phrase) else { return false };
        let tpl = self.tpl.max(1);
        let tick_len = self.phrase_line(phrase.lpb) / tpl as f64;
        p.wait -= frames;
        while p.wait <= 0.0 {
            if p.tick == 0 && p.line >= phrase.lines {
                if !phrase.looping {
                    self.phrases[k] = p;
                    return false;
                }
                p.line = 0;
            }
            // The phrase's cells play on its instrument, and effects on the
            // song (Bxx, Fxx, Zxx) don't apply.
            let line = (p.tick == 0).then(|| {
                let cell = Cell { module: None, ..phrase.cells[p.line] };
                let effects = Effects::new(&cell, std::iter::empty());
                (self.maybe(cell, &effects), effects)
            });
            let dsp = self.nodes[i].dsp.as_mut();
            let mut track = p.track;
            let player = p;
            let mut send = |_, ev| player.send(dsp, ev);
            if let Some((cell, effects)) = line {
                p.playing = p.line;
                track.end_line(&mut send);
                match effects.find(0xD) {
                    Some(d) if d > 0 => track.delayed = Some((d as u32, cell, effects)),
                    _ => track.trigger(cell, &effects, true, &mut send),
                }
            } else {
                if let Some((at, cell, effects)) = track.delayed
                    && at == p.tick
                {
                    track.delayed = None;
                    track.trigger(cell, &effects, true, &mut send);
                }
                track.tick_effects(p.tick, &mut send);
            }
            p.track = track;
            p.tick += 1;
            if p.tick >= tpl {
                p.tick = 0;
                p.line += 1;
            }
            p.wait += tick_len;
        }
        self.phrases[k] = p;
        true
    }

    /// Plays the phrases on for `frames` more frames.
    pub(super) fn run_phrases(&mut self, frames: usize) {
        let mut k = 0;
        while k < self.phrases.len() {
            if self.advance_phrase(k, frames as f64) {
                k += 1;
            } else {
                // Over: its last note sounds on until the key lets go.
                self.phrases.swap_remove(k);
            }
        }
    }
}
