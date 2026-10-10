//! Playing the song: ticks and lines, the slots of the order list, and
//! each line's notes and effects.

use super::*;

impl Engine {
    /// How long until the next tick, after `run_tick` has moved on to it.
    /// With groove, an odd line starts late and its ticks are shorter, so
    /// the even line after it is on time.
    pub(super) fn tick_wait(&self) -> f64 {
        let tick = self.samples_per_tick();
        let delay = self.project.groove.clamp(0.0, 1.0) as f64 * 0.5 * tick * self.tpl.max(1) as f64;
        if delay == 0.0 {
            return tick;
        }
        // The line just played, and the tick coming up.
        let played_odd = self.shown.1 % 2 == 1;
        let base = if played_odd { tick - delay / self.tpl.max(1) as f64 } else { tick };
        let late = self.tick == 0 && self.pos_line % 2 == 1;
        base + if late { delay } else { 0.0 }
    }

    /// Ticks this line lasts: TPL, and as many again for each line a
    /// Wxx holds it.
    pub(super) fn line_ticks(&self) -> u32 {
        self.tpl.max(1) * (1 + self.wait)
    }

    pub(super) fn samples_per_tick(&self) -> f64 {
        self.sr as f64 * 60.0 / (self.bpm.max(1.0) as f64 * self.project.lpb.max(1) as f64 * self.tpl.max(1) as f64)
    }

    pub(super) fn run_tick(&mut self) {
        let project = self.project.clone();
        let Some(&slot) = project.order.get(self.pos_order) else {
            self.playing = false;
            return;
        };
        let Some(pattern) = project.patterns.get(slot.pattern) else {
            self.playing = false;
            return;
        };
        if self.pos_line >= pattern.lines {
            self.pos_line = 0;
        }
        self.shown_tick = self.tick;
        if self.tick == 0 {
            self.shown = (self.pos_order, self.pos_line);
            self.wait = 0;
            let lpb = self.project.lpb.max(1) as usize;
            if self.metronome && self.pos_line.is_multiple_of(lpb) {
                let accent = self.pos_line.is_multiple_of(lpb * BEATS_PER_BAR);
                self.click = Some((0.0, if accent { 1760.0 } else { 880.0 }));
            }
        }

        for ch in 0..MAX_TRACKS * MAX_COLUMNS {
            let (t, col) = (ch % MAX_TRACKS, ch / MAX_TRACKS);
            let key = ch as u32;
            let shown = t < pattern.num_tracks() && col < pattern.columns(t);
            if !shown {
                // A column hidden while it played lets go of its note.
                if self.tick == 0
                    && let Some(m) = self.tracks[ch].sounding.take()
                {
                    self.send_off(m, key);
                }
                continue;
            }
            if self.tick == 0 {
                let cell = pattern.cell(t, col, self.pos_line);
                let effects = Effects::new(&cell, pattern.track_effects(t, self.pos_line));
                let cell = self.maybe(cell, &effects);
                self.end_line_effects(ch);
                if !project.track_audible(t) || slot.is_muted(t) {
                    if let Some(m) = self.tracks[ch].sounding.take() {
                        self.send_off(m, key);
                    }
                    continue;
                }
                // The delay column, in 256ths of a line, wins over Dxx.
                let delay = match (cell.delay, effects.find(0xD)) {
                    (Some(d), _) => d as u32 * self.tpl.max(1) / 256,
                    (None, Some(d)) => d as u32,
                    _ => 0,
                };
                if delay > 0 {
                    self.tracks[ch].delayed = Some((delay, cell, effects));
                } else {
                    self.trigger(ch, cell, &effects);
                }
            } else {
                let track = self.tracks[ch];
                if let Some((at, cell, effects)) = track.delayed
                    && at == self.tick
                {
                    self.tracks[ch].delayed = None;
                    self.trigger(ch, cell, &effects);
                }
                self.tick_effects(ch);
            }
        }

        self.tick += 1;
        let line_ticks = self.line_ticks();
        if self.tick >= line_ticks && std::mem::take(&mut self.stop_after_line) {
            // The song's end: back to its start for the next play, its last
            // notes let go to ring out.
            self.tick = 0;
            self.playing = false;
            self.song_ended = true;
            self.pos_order = 0;
            self.pos_line = 0;
            self.stop_notes();
            return;
        }
        if self.tick >= line_ticks {
            self.tick = 0;
            self.pos_line += 1;
            // Leaving the end of the block loop goes back to its start.
            let looped = self.block_loop.filter(|&(order, from, to)| {
                order == self.pos_order && from <= to && self.pos_line == to.min(pattern.lines - 1) + 1
            });
            let (jump, break_to) = (self.jump.take().filter(|_| !self.loop_pattern), self.break_to.take());
            if let Some((_, from, _)) = looped {
                self.pos_line = from;
            } else if jump.is_some() || break_to.is_some() {
                // Bxx picks the slot, Jxx the line; a break alone goes on
                // to the next slot, or stays in a looping pattern.
                let next = if self.loop_pattern { self.pos_order } else { self.pos_order + 1 };
                let mut to = jump.map_or(next, |to| to.min(project.order.len() - 1));
                if to >= project.order.len() {
                    to = 0;
                    self.song_ended = true;
                }
                // Jumping back to where the song has been ends it for rendering.
                self.song_ended |= jump.is_some() && to <= self.pos_order;
                self.pos_order = to;
                self.pos_line = break_to.unwrap_or(0);
            } else if self.pos_line >= pattern.lines {
                self.pos_line = 0;
                if !self.loop_pattern {
                    self.pos_order += 1;
                    if self.pos_order >= project.order.len() {
                        // The song starts over; rendering
                        // stops here.
                        self.pos_order = 0;
                        self.song_ended = true;
                    }
                }
            }
        }
    }

    pub(super) fn send_off(&mut self, module: u8, key: u32) {
        self.note(module, key, NoteEv::Off);
    }

    /// Ends track `t`'s per-line effects.
    pub(super) fn end_line_effects(&mut self, t: usize) {
        let mut track = self.tracks[t];
        track.end_line(&mut |m, ev| self.track_note(m, t, ev));
        self.tracks[t] = track;
    }

    /// Sends a note event of channel `ch` to module `m`, its velocity
    /// scaled by its track's volume.
    pub(super) fn track_note(&mut self, m: u8, ch: usize, ev: NoteEv) {
        let v = self.track_volume[ch % MAX_TRACKS];
        let ev = match ev {
            NoteEv::On(note, vel) => NoteEv::On(note, vel * v),
            NoteEv::Vel(vel) => NoteEv::Vel(vel * v),
            ev => ev,
        };
        self.note(m, ch as u32, ev);
    }

    /// Sets track `t`'s volume from an Lxx, moving the notes it has
    /// sounding.
    pub(super) fn set_track_volume(&mut self, t: usize, arg: u8) {
        let v = arg.min(0x80) as f32 / 128.0;
        if self.track_volume[t] == v {
            return;
        }
        self.track_volume[t] = v;
        for ch in (0..MAX_COLUMNS).map(|c| c * MAX_TRACKS + t) {
            if let Some(m) = self.tracks[ch].sounding {
                self.track_note(m, ch, NoteEv::Vel(self.tracks[ch].sent_vel));
            }
        }
    }

    /// Playback starting partway through the song: each column's last
    /// note before the start plays from where it would be by now, if it
    /// goes to a sample with autoseek.
    pub(super) fn autoseek(&mut self) {
        let project = self.project.clone();
        let Some(start) = project.order.get(self.pos_order) else { return };
        let Some(pattern) = project.patterns.get(start.pattern) else { return };
        let samples_per_line = self.samples_per_tick() * self.tpl.max(1) as f64;
        let wants = |m: u8| {
            project.module(m).is_some_and(|m| m.kind == ModuleKind::Sampler && m.samples.iter().any(|s| s.autoseek))
        };
        if !project.modules.iter().any(|m| wants(m.id)) {
            return;
        }
        for t in 0..pattern.num_tracks() {
            if !project.track_audible(t) || start.is_muted(t) {
                continue;
            }
            for col in 0..pattern.columns(t) {
                let Some((cell, ago)) = self.last_note(t, col) else { continue };
                if !cell.module.is_some_and(wants) {
                    continue;
                }
                let ch = col * MAX_TRACKS + t;
                // The note alone, with its sample offset: its other effects
                // have had their time.
                let effects = Effects::new(&Cell { fx: cell.fx.filter(|f| f.0 == 0x9), ..cell }, std::iter::empty());
                self.trigger(ch, cell, &effects);
                if let Some(m) = self.tracks[ch].sounding {
                    self.note(m, ch as u32, NoteEv::Seek(ago as f64 * samples_per_line));
                }
            }
        }
    }

    /// The last note in column `col` of track `t` before the play position,
    /// with the instrument it plays (from an earlier line if it names none),
    /// and how many lines ago it was; `None` if a note-off came after it.
    pub(super) fn last_note(&self, t: usize, col: usize) -> Option<(Cell, usize)> {
        let project = &self.project;
        let mut note: Option<(Cell, usize)> = None;
        let mut ago = 0;
        for order in (0..=self.pos_order).rev() {
            let pattern = project.patterns.get(project.order.get(order)?.pattern)?;
            let end = if order == self.pos_order { self.pos_line.min(pattern.lines) } else { pattern.lines };
            for line in (0..end).rev() {
                ago += 1;
                if t >= pattern.num_tracks() || col >= pattern.columns(t) {
                    continue;
                }
                let cell = pattern.cell(t, col, line);
                match (&mut note, cell.note) {
                    (None, Some(Note::Off)) => return None,
                    (None, Some(Note::On(_))) => note = Some((cell, ago)),
                    _ => {}
                }
                if let Some((found, _)) = &mut note {
                    found.module = found.module.or(cell.module);
                    if found.module.is_some() {
                        return note;
                    }
                }
            }
        }
        None
    }

    pub(super) fn trigger(&mut self, t: usize, cell: Cell, effects: &Effects) {
        let key = t as u32;
        let mut track = self.tracks[t];
        // Notes sent to effects are ignored.
        let plays = cell.module.or(track.module).is_some_and(|m| self.plays_notes(m));
        let picked = effects.find(FX_PHRASE);
        if let Some(l) = effects.find(FX_TRACK_VOLUME) {
            self.set_track_volume(t % MAX_TRACKS, l);
        }
        track.trigger(cell, effects, plays, &mut |m, ev| {
            if let NoteEv::On(..) = ev {
                self.picked_phrase = picked;
            }
            self.track_note(m, key as usize, ev);
            self.picked_phrase = None;
        });
        self.tracks[t] = track;
        for fx in effects.iter() {
            match fx {
                (0xB, a) => self.jump = Some(a as usize),
                (FX_BREAK, a) => self.break_to = Some(a as usize),
                (FX_WAIT, a) => self.wait = self.wait.max(a as u32),
                // F00 ends the song after this line, as in ProTracker.
                (0xF, 0) => self.stop_after_line = true,
                (0xF, a) if a >= 0x20 => self.bpm = a as f32,
                (0xF, a) if a > 0 => self.tpl = a as u32,
                _ => {}
            }
        }
    }

    pub(super) fn tick_effects(&mut self, t: usize) {
        let (tick, mut track) = (self.tick, self.tracks[t]);
        track.tick_effects(tick, &mut |m, ev| self.track_note(m, t, ev));
        self.tracks[t] = track;
    }

    /// How far through the song playback is, 0..1, by slots and the lines
    /// of the one playing.
    pub fn song_progress(&self) -> f32 {
        let project = &self.project;
        let lines =
            project.order.get(self.shown.0).and_then(|s| project.patterns.get(s.pattern)).map_or(1, |p| p.lines);
        let into = (self.pattern_line() / lines.max(1) as f64).min(1.0) as f32;
        ((self.shown.0 as f32 + into) / project.order.len().max(1) as f32).min(1.0)
    }

    /// Where in the pattern playing this block starts, in lines.
    pub(super) fn pattern_line(&self) -> f64 {
        let into_tick = (1.0 - self.samples_to_tick / self.samples_per_tick()).clamp(0.0, 1.0);
        self.shown.1 as f64 + (self.shown_tick as f64 + into_tick) / self.line_ticks() as f64
    }

    /// While the song plays, where this block starts in it, in lines from
    /// its start: the slots before the one playing in full, then the lines
    /// of its pattern played. LFOs synced to lines or beats follow it, so
    /// they keep to the beat wherever playback started.
    pub(super) fn song_line(&self) -> Option<f64> {
        if !self.playing {
            return None;
        }
        let project = &self.project;
        let before = project.order.iter().take(self.shown.0).filter_map(|s| project.patterns.get(s.pattern));
        Some(before.map(|p| p.lines).sum::<usize>() as f64 + self.pattern_line())
    }
}
