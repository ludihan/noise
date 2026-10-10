//! The Sampler: samples across the keyboard, with loops, slices, keyzones and mute groups.

use super::*;

/// What the sampler needs from a `SampleSlot`, without the strings.
pub(super) struct Zone {
    /// `None` for a slot without audio, which keeps zone and slot indices
    /// lined up.
    pub(super) data: Option<Arc<Sample>>,
    pub(super) gain: f32,
    pub(super) pan: f32,
    /// Semitones to add to the note before comparing with the base note.
    pub(super) tune: f32,
    pub(super) base_note: f32,
    pub(super) loop_mode: u8,
    pub(super) loop_start: f64,
    pub(super) loop_end: f64,
    pub(super) keys: [u8; 2],
    pub(super) velocities: [u8; 2],
    /// Lines the whole sample takes, or 0.
    pub(super) beat_sync: f64,
    pub(super) oneshot: bool,
    pub(super) mute_group: u8,
    pub(super) autoseek: bool,
    /// The sample slot the zone plays, and the frames it plays: all of
    /// them, or one slice.
    pub(super) slot: usize,
    /// Which slice of the slot this is, if one.
    pub(super) slice: Option<usize>,
    pub(super) start: f64,
    pub(super) end: f64,
}

/// How long a voice cut by its mute group takes to fade out, in seconds.
pub(super) const CHOKE_TIME: f32 = 0.005;

#[derive(Clone, Copy, Default)]
pub(super) struct SamplerVoice {
    pub(super) slot: VoiceSlot,
    pub(super) env: Adsr,
    pub(super) zone: usize,
    /// Address of the zone's audio when the voice started; if the audio is
    /// replaced the voice is stopped.
    pub(super) data: usize,
    /// Read position in sample frames.
    pub(super) pos: f64,
    /// Playing backwards inside a backward or ping-pong loop.
    pub(super) backwards: bool,
    /// Playing the whole sample backwards, as Rxx asks, past its loop.
    pub(super) reversed: bool,
    /// Semitones the note moves by to keep its pitch on a slice Sxx
    /// switched to.
    pub(super) shift: f32,
    /// Fading out after its mute group cut it, from 1 down.
    pub(super) choke: Option<f32>,
    /// Output frames to move on by before playing, for autoseek.
    pub(super) seek: f64,
    pub(super) md: VoiceMod,
}

pub(super) struct Sampler {
    pub(super) zones: Vec<Zone>,
    pub(super) voices: [SamplerVoice; MAX_VOICES],
    pub(super) clock: u64,
    /// The last note played: its key, note and velocity, for Sxx.
    pub(super) last_on: Option<(u32, f32, f32)>,
    pub(super) mods: Modulation,
    pub(super) tap: TrackTap,
}

impl Sampler {
    pub(super) fn new() -> Self {
        let voices = [SamplerVoice::default(); MAX_VOICES];
        Self { zones: Vec::new(), voices, clock: 0, last_on: None, mods: Modulation::default(), tap: TrackTap::new() }
    }

    /// Starts a voice playing zone `zi`, `shift` semitones from the note,
    /// cutting the others of its mute group.
    pub(super) fn start(&mut self, zi: usize, key: u32, note: f32, vel: f32, shift: f32) {
        let z = &self.zones[zi];
        let Some(data) = &z.data else { return };
        let data = Arc::as_ptr(data) as usize;
        if z.mute_group > 0 {
            let zones = &self.zones;
            let same_group =
                |v: &SamplerVoice| v.zone != zi && zones.get(v.zone).is_some_and(|o| o.mute_group == z.mute_group);
            for v in self.voices.iter_mut().filter(|v| v.env.active() && v.choke.is_none() && same_group(v)) {
                v.choke = Some(1.0);
            }
        }
        let i = alloc_voice(&mut self.voices, |v| (&v.slot, v.env.active()));
        self.voices[i] = SamplerVoice {
            slot: VoiceSlot::new(key, note, vel, self.clock),
            zone: zi,
            data,
            pos: z.start,
            shift,
            ..Default::default()
        };
        self.voices[i].env.trigger();
    }
}

/// 4-point Hermite interpolation of `frames` at fractional position `pos`.
pub(super) fn hermite(frames: &[Frame], pos: f64) -> Frame {
    let i = pos.floor() as isize;
    let t = (pos - i as f64) as f32;
    let last = frames.len() as isize - 1;
    let at = |k: isize| frames[(i + k).clamp(0, last) as usize];
    let (xm1, x0, x1, x2) = (at(-1), at(0), at(1), at(2));
    let mut out = [0.0; 2];
    for ch in 0..2 {
        let c1 = 0.5 * (x1[ch] - xm1[ch]);
        let c2 = xm1[ch] - 2.5 * x0[ch] + 2.0 * x1[ch] - 0.5 * x2[ch];
        let c3 = 0.5 * (x2[ch] - xm1[ch]) + 1.5 * (x0[ch] - x1[ch]);
        out[ch] = ((c3 * t + c2) * t + c1) * t + x0[ch];
    }
    out
}

impl Dsp for Sampler {
    voice_controls!();

    fn track_tap(&mut self) -> Option<&mut TrackTap> {
        Some(&mut self.tap)
    }

    fn note_on(&mut self, key: u32, note: f32, vel: f32) {
        // Every sample whose keyzone holds the note plays, so overlapping
        // zones layer.
        self.clock += 1;
        self.last_on = Some((key, note, vel));
        let (n, v) = (note.round().clamp(0.0, 127.0) as u8, (vel * 127.0).round().clamp(0.0, 127.0) as u8);
        for zi in 0..self.zones.len() {
            let z = &self.zones[zi];
            if z.data.is_some()
                && (z.keys[0]..=z.keys[1]).contains(&n)
                && (z.velocities[0]..=z.velocities[1]).contains(&v)
            {
                self.start(zi, key, note, vel, 0.0);
            }
        }
    }

    fn sample_offset(&mut self, key: u32, pos: f32) {
        let newest = self.voices.iter().filter(|v| v.slot.key == key && v.env.active()).map(|v| v.slot.age).max();
        for v in self.voices.iter_mut().filter(|v| v.slot.key == key && v.env.active() && Some(v.slot.age) == newest) {
            if let Some(data) = self.zones.get(v.zone).and_then(|z| z.data.as_ref()) {
                let z = &self.zones[v.zone];
                v.pos = z.start + pos as f64 * (z.end - z.start).min(data.len() as f64);
            }
        }
    }

    fn reverse(&mut self, key: u32, on: bool) {
        let newest = self.voices.iter().filter(|v| v.slot.key == key && v.env.active()).map(|v| v.slot.age).max();
        for v in self.voices.iter_mut().filter(|v| v.slot.key == key && v.env.active() && Some(v.slot.age) == newest) {
            let Some(z) = self.zones.get(v.zone) else { continue };
            if on && !v.reversed && v.pos <= z.start {
                let len = z.data.as_ref().map_or(0.0, |d| d.len() as f64);
                v.pos = (z.end.min(len) - 1.0).max(z.start);
            }
            v.reversed = on;
        }
    }

    fn play_slice(&mut self, key: u32, slice: usize) {
        // It follows the note it changes.
        let Some((_, note, vel)) = self.last_on.filter(|l| l.0 == key) else { return };
        let clock = self.clock;
        let mut started = false;
        for v in self.voices.iter_mut().filter(|v| v.slot.key == key && v.env.active() && v.slot.age == clock) {
            started = true;
            let Some(from) = self.zones.get(v.zone) else { continue };
            let Some(to) = self.zones.iter().position(|z| z.slot == from.slot && z.slice == Some(slice)) else {
                continue;
            };
            v.shift += self.zones[to].base_note - from.base_note;
            v.zone = to;
            v.pos = self.zones[to].start;
        }
        // A note that played nothing plays the slice of the first sliced
        // sample, pitched from the sample's base note.
        if !started && let Some(zi) = self.zones.iter().position(|z| z.slice == Some(slice)) {
            let whole = self.zones[self.zones[zi].slot].base_note;
            self.start(zi, key, note, vel, self.zones[zi].base_note - whole);
        }
    }

    fn seek(&mut self, key: u32, frames: f64) {
        let newest = self.voices.iter().filter(|v| v.slot.key == key && v.env.active()).map(|v| v.slot.age).max();
        for v in self.voices.iter_mut().filter(|v| v.slot.key == key && v.env.active() && Some(v.slot.age) == newest) {
            if self.zones.get(v.zone).is_some_and(|z| z.autoseek) {
                v.seek = frames;
            } else {
                v.env = Adsr::default();
            }
        }
    }

    fn note_off(&mut self, key: u32) {
        let zones = &self.zones;
        let oneshot = |v: &SamplerVoice| zones.get(v.zone).is_some_and(|z| z.oneshot);
        for v in self.voices.iter_mut().filter(|v| v.slot.key == key && !v.slot.released && !oneshot(v)) {
            v.slot.released = true;
            v.env.release();
        }
    }

    fn set_samples(&mut self, slots: &[SampleSlot]) {
        // A zone for each slot, lined up with them, then one for each slice.
        self.zones.clear();
        for (i, s) in slots.iter().enumerate() {
            let len = s.len() as f64;
            let loop_end = (s.loop_end as f64).clamp(1.0, len.max(1.0));
            let sliced = !s.slices.is_empty();
            let base = s.base_note;
            self.zones.push(Zone {
                data: s.data.clone(),
                gain: s.volume,
                pan: s.panning,
                tune: s.transpose as f32 + s.finetune as f32 / 100.0,
                base_note: s.base_note as f32,
                loop_mode: s.loop_mode,
                loop_start: (s.loop_start as f64).min(loop_end - 1.0),
                loop_end,
                // A sliced sample plays whole on its base note only.
                keys: if sliced { [base, base] } else { s.keys },
                velocities: s.velocities,
                beat_sync: s.beat_sync as f64,
                oneshot: s.oneshot,
                mute_group: s.mute_group,
                autoseek: s.autoseek,
                slot: i,
                slice: None,
                start: 0.0,
                end: len,
            });
        }
        for (i, s) in slots.iter().enumerate() {
            for (k, (from, to)) in s.slice_ranges().into_iter().enumerate() {
                let note = s.slice_note(k);
                let st = s.slice(k);
                let whole = &self.zones[i];
                let zone = Zone {
                    data: whole.data.clone(),
                    gain: whole.gain * st.volume,
                    pan: whole.pan + st.panning,
                    tune: whole.tune + st.transpose as f32 + st.finetune as f32 / 100.0,
                    base_note: note as f32,
                    // A slice loops over all of itself.
                    loop_mode: st.loop_mode,
                    loop_start: from as f64,
                    loop_end: to as f64,
                    oneshot: whole.oneshot || st.oneshot,
                    keys: [note, note],
                    slice: Some(k),
                    start: from as f64,
                    end: to as f64,
                    ..*whole
                };
                self.zones.push(zone);
            }
        }
        // Voices whose sample was removed or replaced stop; the rest pick
        // up the new settings.
        for v in self.voices.iter_mut().filter(|v| v.env.active()) {
            match self.zones.get(v.zone).and_then(|z| z.data.as_ref()) {
                Some(data) if Arc::as_ptr(data) as usize == v.data => {}
                _ => v.env = Adsr::default(),
            }
        }
    }

    fn set_modulation(&mut self, m: &Modulation) {
        copy_modulation(&mut self.mods, m);
    }

    fn playheads(&self, out: &mut Vec<Playhead>) {
        for v in self.voices.iter().filter(|v| v.env.active()) {
            let envelopes = [v.md.pitch_t, v.md.filter_t];
            let slot = self.zones.get(v.zone).map_or(v.zone, |z| z.slot);
            out.push(Playhead { module: 0, slot, pos: v.pos, level: v.env.level * v.slot.vel, envelopes });
        }
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], _: &[Frame], out: &mut [Frame]) {
        out.fill([0.0; 2]);
        let (vol, pan, transpose) = (p[0], p[1], p[2].round());
        let (a, d, s, r) = (p[3], p[4], p[5], p[6]);

        let m = &self.mods;
        for v in self.voices.iter_mut().filter(|v| v.env.active()) {
            let Some(z) = self.zones.get(v.zone) else { continue };
            let Some(data) = &z.data else { continue };
            let frames = &data.frames[..];
            let len = frames.len() as f64;
            let (pl, pr) = pan_gains(pan + z.pan + v.slot.pan);

            let mut mb = v.md.block(m, &v.slot, out.len(), ctx.sr);
            let semis = v.slot.note + v.shift + transpose + z.tune - z.base_note + mb.bend;
            let pitch = 2f64.powf(semis as f64 / 12.0);
            let rate = if z.beat_sync > 0.0 {
                pitch * len / (z.beat_sync * ctx.samples_per_line as f64)
            } else {
                pitch * (data.sample_rate / ctx.sr) as f64
            };
            let (ls, le) = (z.loop_start, z.loop_end);
            let mut pos = v.pos;
            if v.seek > 0.0 {
                match seek_position(z, pos, rate * std::mem::take(&mut v.seek)) {
                    Some((p, backwards)) => (pos, v.backwards) = (p, backwards),
                    None => {
                        // It would have ended by now.
                        v.env = Adsr::default();
                        continue;
                    }
                }
            }
            for (i, o) in out.iter_mut().enumerate() {
                let env = v.env.next(ctx.sr, a, d, s, r);
                let x = v.md.apply(&mut mb, hermite(frames, pos));
                let g = env * v.slot.vel * vol * z.gain * v.choke.unwrap_or(1.0);
                o[0] += x[0] * g * pl;
                o[1] += x[1] * g * pr;
                self.tap.add(v.slot.key, i, [x[0] * g * pl, x[1] * g * pr]);
                if let Some(c) = &mut v.choke {
                    *c -= 1.0 / (CHOKE_TIME * ctx.sr);
                    if *c <= 0.0 {
                        v.env = Adsr::default();
                        break;
                    }
                }

                if v.reversed {
                    pos -= rate;
                } else if v.backwards {
                    pos -= rate;
                    if pos < ls {
                        if z.loop_mode == 3 {
                            // Ping-pong: bounce off the loop start.
                            v.backwards = false;
                            pos = (2.0 * ls - pos).min(le - 1.0);
                        } else {
                            pos += le - ls;
                        }
                    }
                } else {
                    pos += rate;
                    // Loops only start once playback reaches them; a 9xx
                    // offset past the loop end plays out the rest.
                    if pos >= le && pos - rate < le {
                        match z.loop_mode {
                            1 => pos = ls + (pos - ls) % (le - ls),
                            2 | 3 => {
                                // Turn around at the last frame in the loop.
                                v.backwards = true;
                                pos = (2.0 * (le - 1.0) - pos).max(ls);
                            }
                            _ => {}
                        }
                    }
                }
                if pos >= z.end.min(len) || pos < 0.0 || (v.reversed && pos < z.start) {
                    v.env = Adsr::default();
                    break;
                }
            }
            v.pos = pos.clamp(0.0, len - 1.0);
        }
    }
}

/// Where a voice of zone `z` at `pos` is after playing `dist` frames of
/// the sample forwards, through its loop, and whether it is then playing
/// backwards; `None` if it has played out.
pub(super) fn seek_position(z: &Zone, pos: f64, dist: f64) -> Option<(f64, bool)> {
    let (ls, le) = (z.loop_start, z.loop_end);
    let to = pos + dist;
    let span = le - ls;
    if z.loop_mode == 0 || pos >= le || to < le || span < 1.0 {
        return (to < z.end).then_some((to, false));
    }
    let over = to - le;
    Some(match z.loop_mode {
        1 => (ls + over % span, false),
        // Backward: from the loop end down to its start, over and over.
        2 => (le - 1.0 - over % span, true),
        // Ping-pong: down from the end, then up from the start.
        _ => {
            let k = over % (2.0 * span);
            if k < span { ((le - 1.0 - k).max(ls), true) } else { (ls + (k - span), false) }
        }
    })
}
