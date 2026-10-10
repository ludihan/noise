//! A note column's state as the sequencer plays it: its note, instrument
//! and volume, and the effect commands working on them.

use super::*;

#[derive(Clone, Copy, Default)]
pub(super) struct Track {
    pub(super) module: Option<u8>,
    /// Module currently holding a note on this track.
    pub(super) sounding: Option<u8>,
    /// The note's pitch and velocity without this line's arpeggio,
    /// vibrato and tremolo, and the pitch and velocity last sent.
    pub(super) base: f32,
    pub(super) pitch: f32,
    pub(super) vel: f32,
    pub(super) sent_vel: f32,
    /// Panning set with 8xx, -1..1; it stays until changed.
    pub(super) pan: f32,
    pub(super) porta_target: Option<f32>,
    pub(super) porta_speed: f32,
    // Per-line effect state.
    pub(super) arp: Option<(u8, u8)>,
    pub(super) slide: f32,
    pub(super) cut_at: Option<u32>,
    pub(super) delayed: Option<(u32, Cell, Effects)>,
    /// Vibrato and tremolo speed (cycles per tick) and depth.
    pub(super) vibrato: Option<(f32, f32)>,
    pub(super) tremolo: Option<(f32, f32)>,
    pub(super) vibrato_phase: f32,
    pub(super) tremolo_phase: f32,
    /// The last speed and depth given, for parameters left at zero.
    pub(super) vibrato_memory: (u8, u8),
    pub(super) tremolo_memory: (u8, u8),
    /// Auto-pan: speed (cycles per tick) and depth, its phase, its last
    /// parameters, and the panning last sent.
    pub(super) autopan: Option<(f32, f32)>,
    pub(super) autopan_phase: f32,
    pub(super) autopan_memory: (u8, u8),
    pub(super) sent_pan: f32,
    /// Velocity added each tick.
    pub(super) vol_slide: f32,
    /// Play the note again every this many ticks.
    pub(super) retrigger: Option<u32>,
    /// Txy: ticks on, then off.
    pub(super) tremor: Option<(u32, u32)>,
}

/// An effect's two nibbles, where a zero picks up the last value given,
/// as in ProTracker.
pub(super) fn remember(memory: &mut (u8, u8), arg: u8) -> (u8, u8) {
    if arg >> 4 != 0 {
        memory.0 = arg >> 4;
    }
    if arg & 0xF != 0 {
        memory.1 = arg & 0xF;
    }
    *memory
}

/// A line's effect commands for one note column: its volume column's,
/// its own, then those of its track's effect columns, which act on every
/// note column. Later ones win.
#[derive(Clone, Copy, Default)]
pub(super) struct Effects([Option<(u8, u8)>; 2 + MAX_FX_COLUMNS]);

impl Effects {
    /// The commands `cell` holds, and `more`.
    pub(super) fn new(cell: &Cell, more: impl Iterator<Item = (u8, u8)>) -> Self {
        let mut e = Effects::default();
        let own = cell.vol.and_then(crate::project::vol_effect).into_iter().chain(cell.fx);
        for (slot, fx) in e.0.iter_mut().zip(own.chain(more)) {
            *slot = Some(fx);
        }
        e
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (u8, u8)> + '_ {
        self.0.iter().flatten().copied()
    }

    /// The argument of the last `cmd`.
    pub(super) fn find(&self, cmd: u8) -> Option<u8> {
        self.iter().filter(|e| e.0 == cmd).last().map(|e| e.1)
    }
}

impl Track {
    /// Ends the line's effects, putting back the pitch and velocity that
    /// arpeggio, vibrato and tremolo moved. `send` gets the events for the
    /// module sounding.
    pub(super) fn end_line(&mut self, send: &mut impl FnMut(u8, NoteEv)) {
        self.arp = None;
        self.slide = 0.0;
        self.cut_at = None;
        self.delayed = None;
        self.vibrato = None;
        self.tremolo = None;
        self.vol_slide = 0.0;
        self.retrigger = None;
        self.tremor = None;
        self.autopan = None;
        let pan = (self.sent_pan != self.pan).then_some(self.pan);
        self.sent_pan = self.pan;
        if let (Some(m), Some(p)) = (self.sounding, pan) {
            send(m, NoteEv::Pan(p));
        }
        let (pitch, vel) =
            ((self.pitch != self.base).then_some(self.base), (self.sent_vel != self.vel).then_some(self.vel));
        self.pitch = self.base;
        self.sent_vel = self.vel;
        if let Some(m) = self.sounding {
            if let Some(p) = pitch {
                send(m, NoteEv::Pitch(p));
            }
            if let Some(v) = vel {
                send(m, NoteEv::Vel(v));
            }
        }
    }

    /// Plays a line's cell: its note, volume, panning and `effects`, those
    /// that act on the note. Effects on the song (Bxx, Fxx) are left to
    /// the caller. `plays` says whether the module the note goes to takes
    /// notes.
    pub(super) fn trigger(&mut self, cell: Cell, effects: &Effects, plays: bool, send: &mut impl FnMut(u8, NoteEv)) {
        if let Some(m) = cell.module {
            self.module = Some(m);
        }
        // Above 80 the volume column holds a command, which `effects` has.
        let vel = cell.vol.filter(|&v| v <= 0x80).map(|v| v as f32 / 128.0);
        let pan_fx = effects.find(0x8);
        if let Some(p) = pan_fx {
            // 00 is left, 80 the middle and FF right.
            self.pan = ((p as f32 - 128.0) / 127.0).clamp(-1.0, 1.0);
        }
        // The panning column: 00 left, 40 the middle, 80 right.
        let pan_set = pan_fx.is_some() || cell.pan.is_some();
        if let Some(p) = cell.pan {
            self.pan = ((p.min(0x80) as f32 - 64.0) / 64.0).clamp(-1.0, 1.0);
        }

        match cell.note {
            Some(Note::On(n)) => {
                let porta = effects.find(0x3).filter(|_| self.sounding.is_some() && self.sounding == self.module);
                if let Some(speed) = porta {
                    self.porta_target = Some(n as f32);
                    self.porta_speed = speed.max(1) as f32 / 16.0;
                } else {
                    if let Some(m) = self.sounding.take() {
                        send(m, NoteEv::Off);
                    }
                    self.vel = vel.unwrap_or(1.0);
                    if let Some(m) = self.module.filter(|_| plays) {
                        send(m, NoteEv::On(n as f32, self.vel));
                        if self.pan != 0.0 {
                            send(m, NoteEv::Pan(self.pan));
                        }
                        if let Some(k) = effects.find(FX_SLICE) {
                            send(m, NoteEv::Slice(k));
                        }
                        if let Some(offset) = effects.find(0x9) {
                            send(m, NoteEv::Offset(offset as f32 / 256.0));
                        }
                        self.sounding = Some(m);
                    }
                    self.base = n as f32;
                    self.pitch = n as f32;
                    self.sent_vel = self.vel;
                    self.porta_target = None;
                    self.vibrato_phase = 0.0;
                    self.tremolo_phase = 0.0;
                }
            }
            Some(Note::Off) => {
                if let Some(m) = self.sounding.take() {
                    send(m, NoteEv::Off);
                }
            }
            None => {
                if let Some(v) = vel {
                    self.vel = v;
                    self.sent_vel = v;
                    if let Some(m) = self.sounding {
                        send(m, NoteEv::Vel(v));
                    }
                }
            }
        }
        // Panning moves a note that is already playing too.
        if pan_set && let Some(m) = self.sounding {
            send(m, NoteEv::Pan(self.pan));
        }
        // So does Rxx, from the end for a note just started.
        if let (Some(r), Some(m)) = (effects.find(FX_REVERSE), self.sounding) {
            send(m, NoteEv::Reverse(r != 0));
        }
        self.sent_pan = self.pan;

        for fx in effects.iter() {
            self.effect(fx);
        }
        if self.cut_at == Some(0) {
            self.cut(send);
        }
    }

    /// Sets up the per-line effect `fx` for the ticks to come.
    pub(super) fn effect(&mut self, fx: (u8, u8)) {
        match fx {
            (0x0, a) if a != 0 => self.arp = Some((a >> 4, a & 0xF)),
            (0x1, a) => self.slide = a as f32 / 16.0,
            (0x2, a) => self.slide = -(a as f32) / 16.0,
            (0x4, a) => {
                let (speed, depth) = remember(&mut self.vibrato_memory, a);
                self.vibrato = Some((speed as f32 / 32.0, depth as f32 / 8.0));
            }
            (0x7, a) => {
                let (speed, depth) = remember(&mut self.tremolo_memory, a);
                self.tremolo = Some((speed as f32 / 32.0, depth as f32 / 16.0));
            }
            (0xA, a) => {
                let (up, down) = (a >> 4, a & 0xF);
                self.vol_slide = if up != 0 { up as f32 } else { -(down as f32) } / 128.0;
            }
            (FX_AUTOPAN, a) => {
                let (speed, depth) = remember(&mut self.autopan_memory, a);
                self.autopan = Some((speed as f32 / 32.0, depth as f32 / 15.0));
            }
            (0xC, a) => self.cut_at = Some(a as u32),
            (0xE, a) if a > 0 => self.retrigger = Some(a as u32),
            (FX_TREMOR, a) => self.tremor = Some(((a >> 4).max(1) as u32, (a & 0xF) as u32)),
            _ => {}
        }
    }

    pub(super) fn cut(&mut self, send: &mut impl FnMut(u8, NoteEv)) {
        if let Some(m) = self.sounding.take() {
            send(m, NoteEv::Off);
        }
        self.cut_at = None;
    }

    /// Runs the line's effects on tick `tick` after its first.
    pub(super) fn tick_effects(&mut self, tick: u32, send: &mut impl FnMut(u8, NoteEv)) {
        let Some(m) = self.sounding else { return };
        if self.cut_at == Some(tick) {
            self.cut(send);
            return;
        }
        // Slides and glides move the note for good; arpeggio and vibrato
        // only for this line.
        self.base += self.slide;
        if let Some(target) = self.porta_target {
            let step = self.porta_speed;
            self.base =
                if self.base < target { (self.base + step).min(target) } else { (self.base - step).max(target) };
            if self.base == target {
                self.porta_target = None;
            }
        }
        let mut pitch = self.base;
        if let Some((x, y)) = self.arp {
            pitch += [0, x, y][(tick % 3) as usize] as f32;
        }
        if let Some((speed, depth)) = self.vibrato {
            self.vibrato_phase = (self.vibrato_phase + speed).fract();
            pitch += (self.vibrato_phase * TAU).sin() * depth;
        }
        // Likewise a volume slide changes the velocity for good, tremolo
        // only for this line.
        self.vel = (self.vel + self.vol_slide).clamp(0.0, 1.0);
        let mut vel = self.vel;
        if let Some((speed, depth)) = self.tremolo {
            self.tremolo_phase = (self.tremolo_phase + speed).fract();
            vel *= 1.0 - depth * (0.5 - 0.5 * (self.tremolo_phase * TAU).cos());
        }
        if let Some((on, off)) = self.tremor
            && tick % (on + off) >= on
        {
            vel = 0.0;
        }
        if let Some((speed, depth)) = self.autopan {
            self.autopan_phase = (self.autopan_phase + speed).fract();
            let pan = (self.pan + (self.autopan_phase * TAU).sin() * depth).clamp(-1.0, 1.0);
            if pan != self.sent_pan {
                self.sent_pan = pan;
                send(m, NoteEv::Pan(pan));
            }
        }
        let retrigger = self.retrigger.is_some_and(|r| tick.is_multiple_of(r));
        let (moved, louder) = (pitch != self.pitch, vel != self.sent_vel);
        self.pitch = pitch;
        self.sent_vel = vel;
        if retrigger {
            send(m, NoteEv::Off);
            send(m, NoteEv::On(pitch, vel));
            if self.pan != 0.0 {
                send(m, NoteEv::Pan(self.pan));
            }
            return;
        }
        if moved {
            send(m, NoteEv::Pitch(pitch));
        }
        if louder {
            send(m, NoteEv::Vel(vel));
        }
    }
}
