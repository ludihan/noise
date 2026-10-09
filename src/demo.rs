//! The demo song, "Last Light": two minutes in D minor that uses nearly
//! everything noise has, so there is something to look at everywhere.
//!
//! A felt piano opens it alone. Strings, a sub bass and a rippling arp
//! join for the verse; the build stutters into a chorus with a choir-like
//! lead; the bridge drops to voices that talk and bells that may or may
//! not ring; the last chorus adds a harmony and bell arpeggios, and the
//! piano slows down to end on a major chord.
//!
//! The piano and the swell are rendered here, so the Sampler has samples
//! without audio files; they are written next to the song when it is
//! saved.

use crate::dsp::Frame;
use crate::project::*;
use crate::sample::Sample;
use std::f32::consts::TAU;

/// A chord: the bass root, three notes for the pad and the piano, and the
/// root the arp plays its phrase from.
#[derive(Clone, Copy)]
struct Chord {
    root: u8,
    tones: [u8; 3],
    arp: u8,
}

const fn chord(root: u8, tones: [u8; 3]) -> Chord {
    Chord { root, tones, arp: root + 24 }
}

// Notes are MIDI numbers: 48 is the tracker's C-4.
const DM: Chord = chord(26, [45, 50, 53]);
const BB: Chord = chord(22, [46, 50, 53]);
const F: Chord = chord(29, [45, 48, 53]);
const C: Chord = chord(24, [43, 48, 52]);
const GM: Chord = chord(31, [46, 50, 55]);
const A: Chord = chord(33, [45, 49, 52]);
/// The last chord: D major, the light at the end.
const D: Chord = chord(26, [45, 50, 54]);

const VERSE: [Chord; 4] = [DM, BB, F, C];
const CHORUS: [Chord; 4] = [BB, F, C, DM];
const BUILD: [Chord; 4] = [GM, BB, F, A];
const BRIDGE: [Chord; 4] = [BB, F, GM, A];
const OUTRO: [Chord; 4] = [DM, BB, C, D];

/// The piano's tune, over the verse's chords: (line, note).
const MOTIF: [(usize, u8); 12] = [
    (0, 69),
    (6, 67),
    (8, 65),
    (12, 64),
    (16, 62),
    (24, 65),
    (28, 67),
    (32, 69),
    (38, 72),
    (40, 69),
    (44, 67),
    (48, 64),
];

/// The chorus melody: (line, note, length in lines).
const MELODY: [(usize, u8, usize); 15] = [
    (0, 74, 8),
    (8, 72, 4),
    (12, 70, 4),
    (16, 69, 4),
    (20, 72, 4),
    (24, 77, 8),
    (32, 76, 6),
    (38, 74, 2),
    (40, 72, 4),
    (44, 76, 4),
    (48, 74, 8),
    (56, 72, 2),
    (58, 69, 2),
    (60, 74, 3),
    (63, 74, 1),
];

/// The note a third below `n` in D minor, for the last chorus's harmony.
fn third_below(n: u8) -> u8 {
    const SCALE: [u8; 7] = [2, 4, 5, 7, 9, 10, 0];
    let mut steps = 0;
    let mut m = n;
    while steps < 2 {
        m -= 1;
        if SCALE.contains(&(m % 12)) {
            steps += 1;
        }
    }
    m
}

pub(crate) struct Rng(pub(crate) u32);

impl Rng {
    pub(crate) fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32 * 2.0 - 1.0
    }
}

/// Scales `frames` so their peak is `peak`.
pub(crate) fn normalize(frames: &mut [Frame], peak: f32) {
    let max = frames.iter().fold(0f32, |m, f| m.max(f[0].abs()).max(f[1].abs()));
    if max > 0.0 {
        frames.iter_mut().for_each(|f| *f = [f[0] * peak / max, f[1] * peak / max]);
    }
}

/// A felt piano's C-4 (MIDI 48): two strings a hair apart, each a stack
/// of slightly stretched harmonics that die away faster the higher they
/// are, with the soft knock of the hammer. `bright` is a harder strike.
pub(crate) fn felt_piano(sr: f32, bright: bool) -> Sample {
    let f0 = 440.0 * 2f32.powf((48.0 - 69.0) / 12.0);
    let len = (sr * 3.5) as usize;
    let mut frames = vec![[0.0f32; 2]; len];
    let (tilt, damp) = if bright { (1.15, 0.55) } else { (1.9, 0.8) };
    for h in 1..=18 {
        let hf = h as f32;
        let f = hf * f0 * (1.0 + 0.0004 * hf * hf).sqrt();
        if f > sr * 0.45 {
            break;
        }
        let amp = hf.powf(-tilt) * if h > 9 { 0.5 } else { 1.0 };
        let decay = damp * (0.6 + 0.45 * hf);
        // The two strings beat slowly against each other, and sit a little
        // apart in the stereo field.
        for (string, cents, pan) in [(0, -0.9f32, 0.65f32), (1, 0.9, 0.35)] {
            let w = std::f32::consts::TAU * f * 2f32.powf(cents / 1200.0) / sr;
            let phase = (string * h) as f32 * 0.7;
            for (k, fr) in frames.iter_mut().enumerate() {
                let t = k as f32 / sr;
                let v = amp * (-decay * t).exp() * (w * k as f32 + phase).sin();
                fr[0] += v * (1.0 - pan);
                fr[1] += v * pan;
            }
        }
    }
    // The hammer: a short, muffled knock.
    let mut rng = Rng(0x9E37_79B9);
    let (mut lp, knock) = (0.0f32, if bright { 0.25 } else { 0.1 });
    for (k, fr) in frames.iter_mut().take((sr * 0.03) as usize).enumerate() {
        lp += 0.15 * (rng.next() - lp);
        let v = lp * knock * (-(k as f32) / (sr * 0.006)).exp();
        fr[0] += v;
        fr[1] += v;
    }
    // A soft onset, felt rather than struck, and a gentle end.
    let attack = (sr * if bright { 0.002 } else { 0.006 }) as usize;
    let fade = (sr * 0.3) as usize;
    for (k, f) in frames.iter_mut().take(attack).enumerate() {
        let g = k as f32 / attack as f32;
        *f = [f[0] * g, f[1] * g];
    }
    for (k, f) in frames.iter_mut().rev().take(fade).enumerate() {
        let g = k as f32 / fade as f32;
        *f = [f[0] * g, f[1] * g];
    }
    normalize(&mut frames, 0.8);
    let name = if bright { "Felt Piano (hard)" } else { "Felt Piano (soft)" };
    Sample { name: name.into(), sample_rate: sr, channels: 2, frames }
}

/// A reversed-cymbal swell: noise that grows louder and brighter, then
/// stops dead, to throw the song into its next part.
fn swell(sr: f32) -> Sample {
    let len = (sr * 2.5) as usize;
    let mut rng = Rng(0x2545_F491);
    let mut lp = [0.0f32; 2];
    let mut frames = (0..len)
        .map(|k| {
            let t = k as f32 / len as f32;
            let c = 0.03 + 0.6 * t * t;
            let mut f = [0.0; 2];
            for ch in 0..2 {
                lp[ch] += c * (rng.next() - lp[ch]);
                f[ch] = lp[ch] * t.powi(3) * (1.0 - ((t - 0.997).max(0.0) / 0.003));
            }
            f
        })
        .collect::<Vec<Frame>>();
    normalize(&mut frames, 0.7);
    Sample { name: "Swell".into(), sample_rate: sr, channels: 2, frames }
}

/// A slot holding audio rendered here, to be written out when saved.
pub(crate) fn rendered(sample: Sample) -> SampleSlot {
    SampleSlot { unsaved: true, ..SampleSlot::new(sample, None) }
}

pub(crate) fn n(note: u8, module: u8, vol: u8) -> Cell {
    Cell { note: Some(Note::On(note)), module: Some(module), vol: Some(vol), ..Cell::default() }
}

pub(crate) fn fx(cell: Cell, cmd: u8, arg: u8) -> Cell {
    Cell { fx: Some((cmd, arg)), ..cell }
}

pub(crate) fn off() -> Cell {
    Cell { note: Some(Note::Off), ..Cell::default() }
}

/// An envelope for parameter `param` of module `m` of kind `kind`, from
/// (line, value) points in the parameter's own units.
pub(crate) fn envelope(
    m: u8,
    kind: ModuleKind,
    param: usize,
    points: &[(f32, f32)],
    steps: bool,
    curve: bool,
) -> Envelope {
    let spec = kind.automatable(param).expect("an automatable parameter");
    let points = points.iter().map(|&(l, v)| (l, spec.position(v))).collect();
    Envelope { steps, curve, ..Envelope::new(m, param, points) }
}

// ---------------------------------------------------------------- shared by the demo songs

/// Names module `id` and sets its parameters, as (index, value).
pub(crate) fn set(p: &mut Project, id: u8, name: &str, params: &[(usize, f32)]) {
    let m = p.module_mut(id).unwrap();
    m.name = name.into();
    for &(i, v) in params {
        m.params[i] = v;
    }
}

/// An effect to add: its kind, name and parameters, as `set` takes them.
pub(crate) type Effect<'a> = (ModuleKind, &'a str, &'a [(usize, f32)]);

/// Adds `list` to the end of `owner`'s chain; returns the effects' ids.
pub(crate) fn effects<const N: usize>(p: &mut Project, owner: impl Into<Owner>, list: [Effect; N]) -> [u8; N] {
    let owner = owner.into();
    let start = p.chain(owner).effects.len();
    std::array::from_fn(|k| {
        let (kind, name, params) = list[k];
        let id = p.chain_insert(owner, start + k, kind).unwrap();
        set(p, id, name, params);
        id
    })
}

/// A roll: `cell`'s note on each of `lines` lines from `line`, played again
/// every `rate` ticks (Exx) in between, rising from its volume to `to`.
pub(crate) fn roll(pat: &mut Pattern, track: usize, line: usize, lines: usize, cell: Cell, rate: u8, to: u8) {
    let from = cell.vol.unwrap_or(0x40);
    for k in 0..lines {
        let vol = from + (to.saturating_sub(from) as usize * k / lines) as u8;
        pat.tracks[track][line + k] = fx(Cell { vol: Some(vol), ..cell }, 0xE, rate);
    }
}

/// The Repeater `id` holds during each of `holds` (from, to), its loop as
/// long as `lengths` say: (line, index into `REPEATER_LENGTHS`).
pub(crate) fn hold(pat: &mut Pattern, id: u8, holds: &[(f32, f32)], lengths: &[(f32, f32)]) {
    let r = ModuleKind::Repeater;
    let mut points = Vec::new();
    if holds.first().is_none_or(|h| h.0 > 0.0) {
        points.push((0.0, 0.0));
    }
    for &(from, to) in holds {
        points.extend([(from, 1.0), (to, 0.0)]);
    }
    pat.automation.push(envelope(id, r, 0, &points, true, false));
    pat.automation.push(envelope(id, r, 1, lengths, true, false));
}

/// `notes` (line, note) on column `col` of `track`.
pub(crate) fn tune(pat: &mut Pattern, track: usize, col: usize, id: u8, notes: &[(usize, u8)], vol: u8) {
    for &(l, note) in notes {
        *pat.cell_mut(track, col, l) = n(note, id, vol);
    }
}

/// Lets go of the notes still sounding on the note columns of `tracks`,
/// as (track, columns), at the pattern's start.
pub(crate) fn let_go(pat: &mut Pattern, tracks: &[(usize, usize)]) {
    for &(t, cols) in tracks {
        for c in 0..cols {
            *pat.cell_mut(t, c, 0) = off();
        }
    }
}

/// What a drum hit is.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Hit {
    Kick,
    Snare,
    Ghost,
    Rim,
    Hat,
    Open,
    Crash,
    Clap,
    Ride,
}

use Hit::*;

/// One hit, mono, until it dies away; `room` is how long a snare's room
/// rings on, in seconds.
pub(crate) fn hit(sr: f32, kind: Hit, seed: u32, room: f32) -> Vec<f32> {
    let secs = match kind {
        Kick => 0.4,
        Snare | Ghost => 0.3 + 3.0 * room,
        Rim => 0.08,
        Hat => 0.15,
        Open => 0.6,
        Crash => 2.0,
        Clap => 0.5,
        Ride => 1.2,
    };
    let mut noise = Rng(seed);
    let (mut phase, mut lp, mut hp) = (0.0f32, 0.0f32, [0.0f32; 2]);
    (0..(sr * secs) as usize)
        .map(|k| {
            let t = k as f32 / sr;
            let w = noise.next();
            match kind {
                Kick => {
                    phase += (50.0 + 140.0 * (-t / 0.03).exp()) / sr;
                    (TAU * phase).sin() * (-t / 0.16).exp() + w * 0.35 * (-t / 0.0015).exp()
                }
                Snare | Ghost => {
                    // A drum's ring, the snares' rattle (the noise less its
                    // lows) and the room (its lows).
                    lp += 0.4 * (w - lp);
                    let (snap, body) = if kind == Ghost { (0.05, 0.5) } else { (0.12, 1.0) };
                    let tone = (TAU * 185.0 * t).sin() * (-t / 0.05).exp() * 0.6
                        + (TAU * 330.0 * t).sin() * (-t / 0.03).exp() * 0.25;
                    tone * body + (w - lp) * (-t / snap).exp() + lp * 0.35 * (-t / room).exp()
                }
                Rim => {
                    (TAU * 820.0 * t).sin() * (-t / 0.012).exp()
                        + (TAU * 1650.0 * t).sin() * 0.5 * (-t / 0.007).exp()
                        + w * 0.5 * (-t / 0.003).exp()
                }
                Hat | Open | Crash | Ride => {
                    // Noise through two highpasses, thin and metallic; the
                    // crash rings with a few partials besides, and the
                    // ride's bell louder.
                    let h1 = w - hp[0];
                    hp[0] += 0.6 * h1;
                    let h2 = h1 - hp[1];
                    hp[1] += 0.6 * h2;
                    let decay = match kind {
                        Hat => 0.022,
                        Open => 0.17,
                        Ride => 0.35,
                        _ => 0.7,
                    };
                    let partials = || [3150.0, 4423.0, 5210.0, 6830.0].iter().map(|f| (TAU * f * t).sin()).sum::<f32>();
                    let ring = match kind {
                        Crash => partials() * 0.08,
                        Ride => partials() * 0.2,
                        _ => 0.0,
                    };
                    (h2 * 0.9 + ring) * (-t / decay).exp()
                }
                Clap => {
                    // Three claps a few milliseconds apart, then the room:
                    // noise between a highpass and a lowpass.
                    let h = w - hp[0];
                    hp[0] += 0.25 * h;
                    lp += 0.55 * (h - lp);
                    let claps: f32 =
                        [0.0, 0.011, 0.023].iter().filter(|&&s| t >= s).map(|&s| (-(t - s) / 0.005).exp()).sum();
                    let tail = if t >= 0.023 { 0.45 * (-(t - 0.023) / 0.11).exp() } else { 0.0 };
                    lp * 1.6 * (claps * 0.7 + tail)
                }
            }
        })
        .collect()
}

/// The tracks, in order.
const DRUMS: usize = 0;
const HATS: usize = 1;
const BASS: usize = 2;
const PAD: usize = 3;
const PIANO: usize = 4;
const ARP: usize = 5;
const LEAD: usize = 6;
const BELLS: usize = 7;

/// The instruments and effects patterns refer to.
struct Ids {
    drums: u8,
    sub_kick: u8,
    bass: u8,
    pad: u8,
    voices: u8,
    piano: u8,
    swell: u8,
    arp: u8,
    lead: u8,
    bells: u8,
    glass: u8,
    keys: u8,
    bass_filter: u8,
    stutter: u8,
    choir: u8,
    voice: u8,
}

impl Project {
    /// "Last Light", the song noise opens with.
    pub fn demo() -> Self {
        let mut p = Self::empty();
        p.title = "Last Light".into();
        p.artist = "noise".into();
        p.comments = COMMENTS.into();
        p.bpm = 84.0;
        p.groove = 0.08;
        p.modules[0].params[0] = 0.45;
        let names = ["Drums", "Hats", "Bass", "Strings", "Piano", "Arp", "Lead", "Bells"];
        let colors = [
            [196, 92, 76],
            [214, 150, 80],
            [120, 96, 196],
            [88, 160, 196],
            [226, 204, 150],
            [104, 186, 140],
            [222, 120, 170],
            [170, 200, 230],
        ];
        for (t, (name, color)) in names.iter().zip(colors).enumerate() {
            p.tracks[t].name = name.to_string();
            p.tracks[t].color = Some(color);
        }
        p.tracks[HATS].show_pan = true;
        p.tracks[PIANO].show_delay = true;
        let ids = instruments(&mut p);
        let mut patterns = vec![
            intro(&ids),
            verse(&ids),
            build(&ids),
            chorus(&ids, false),
            bridge(&ids),
            chorus(&ids, true),
            outro(&ids),
        ];
        let names = ["Intro", "Verse", "Build", "Chorus", "Bridge", "Last Chorus", "Outro"];
        for (pat, name) in patterns.iter_mut().zip(names) {
            pat.name = name.into();
        }
        p.patterns = patterns;
        for (t, cols) in [(DRUMS, 2), (PAD, 3), (PIANO, 5), (LEAD, 2)] {
            p.set_columns(t, cols);
        }
        // The first verse holds back the hats and the arp, and the first
        // chorus the arp, in the matrix.
        let muted = |pattern: usize, tracks: &[usize]| {
            let mut s = Slot::new(pattern);
            tracks.iter().for_each(|&t| s.toggle_mute(t));
            s
        };
        p.order = vec![
            Slot::new(0),
            muted(1, &[HATS, ARP]),
            Slot::new(1),
            Slot::new(2),
            muted(3, &[ARP]),
            Slot::new(3),
            Slot::new(4),
            Slot::new(5),
            Slot::new(5),
            Slot::new(6),
        ];
        p.sections =
            [("Intro", 0), ("Verse", 1), ("Build", 3), ("Chorus", 4), ("Bridge", 6), ("Last Chorus", 7), ("Outro", 9)]
                .into_iter()
                .map(|(name, start)| Section { start, name: name.into() })
                .collect();
        // It ends on its last chord rather than starting over.
        p.end_with_stop();
        p
    }
}

/// Makes every module and wires them up.
fn instruments(p: &mut Project) -> Ids {
    use ModuleKind as K;
    let set = |p: &mut Project, id: u8, name: &str, params: &[(usize, f32)]| {
        let m = p.module_mut(id).unwrap();
        m.name = name.into();
        for &(i, v) in params {
            m.params[i] = v;
        }
    };
    let add = |p: &mut Project, kind: ModuleKind| p.add_module(kind, [0.0, 0.0]).unwrap();
    // Sends the end of `id`'s chain to `to` rather than the output.
    let send = |p: &mut Project, id: u8, to: u8| {
        let last = p.chain(id).effects.last().copied().unwrap_or(id);
        p.disconnect(last, OUTPUT_ID);
        p.connect(last, to);
    };

    // The shared hall everything but the drums and bass goes through.
    let hall = add(p, K::Reverb);
    set(p, hall, "Hall", &[(0, 0.9), (1, 0.45), (2, 0.38)]);
    p.connect(hall, OUTPUT_ID);
    // Track effects: the hats echo off into the distance, though they play
    // the same Drums as the kicks and snares, which stay dry.
    let hat_echo = p.chain_insert(Owner::Track(HATS), 0, K::Echo).unwrap();
    set(p, hat_echo, "Hat Echo", &[(0, 0.18), (1, 0.3), (2, 0.5), (3, 0.6), (4, 0.25)]);

    // The master chain: a limiter on the whole mix.
    let limit = p.chain_insert(OUTPUT_ID, 0, K::Maximizer).unwrap();
    set(p, limit, "Master Limit", &[(0, 1.0), (1, 0.95), (2, 0.15)]);

    // The hall's mud and fizz taken out with a graphic EQ.
    let hall_tone = p.chain_insert(hall, 0, K::Eq10).unwrap();
    set(p, hall_tone, "Hall Tone", &[(0, 0.4), (1, 0.5), (2, 0.7), (8, 0.8), (9, 0.6)]);

    // Drums through a gate that tightens their tails, a bus compressor,
    // the build's stutter and a maximizer.
    let drums = add(p, K::Drums);
    set(p, drums, "Drums", &[(0, 0.8), (1, 48.0), (2, 0.6), (3, 0.35), (4, 0.05)]);
    let tight = p.chain_insert(drums, 0, K::Gate).unwrap();
    set(p, tight, "Tight", &[(0, 0.02), (2, 0.03), (3, 0.08), (4, 0.15)]);
    let bus = p.chain_insert(drums, 1, K::Compressor).unwrap();
    set(p, bus, "Drum Bus", &[(0, 0.35), (1, 3.0), (2, 0.008), (3, 0.12), (4, 1.3)]);
    let stutter = p.chain_insert(drums, 2, K::Repeater).unwrap();
    set(p, stutter, "Stutter", &[]);
    let punch = p.chain_insert(drums, 3, K::Maximizer).unwrap();
    set(p, punch, "Punch", &[(0, 1.25), (1, 0.9), (2, 0.06)]);
    // A Kicker tuned to D under the chorus's kicks.
    let sub_kick = add(p, K::Kicker);
    set(p, sub_kick, "Sub Kick", &[(0, 0.45), (2, 2.5), (5, 0.7), (6, 0.35)]);
    p.connect(sub_kick, OUTPUT_ID);
    // Rounded off by a WaveShaper bent into a soft clip.
    let round = p.chain_insert(sub_kick, 0, K::WaveShaper).unwrap();
    set(p, round, "Round", &[(0, 1.3), (3, 0.6), (9, 0.3), (10, 0.55), (11, 0.72), (12, 0.8)]);

    // A warm sub bass: filtered and gently driven, with the drive's
    // offset taken away, and a little of a bass cabinet.
    let bass = add(p, K::Generator);
    set(p, bass, "Sub Bass", &[(0, 0.5), (1, 0.0), (2, 0.01), (3, 0.4), (4, 0.7), (5, 0.25), (6, 6.0), (7, 2.0)]);
    let bass_filter = p.chain_insert(bass, 0, K::Filter).unwrap();
    set(p, bass_filter, "Bass Filter", &[(1, 500.0), (2, 0.35)]);
    let drive = p.chain_insert(bass, 1, K::Distortion).unwrap();
    set(p, drive, "Warmth", &[(0, 1.6), (1, 0.5), (2, 0.35)]);
    let centre = p.chain_insert(bass, 2, K::DcBlocker).unwrap();
    set(p, centre, "Centre", &[]);
    let speaker = p.chain_insert(bass, 3, K::Cabinet).unwrap();
    set(p, speaker, "Speaker", &[(0, 2.0), (1, 1.2), (2, 0.35)]);

    // Strings: a wide saw pad, chorused, phased, darkened and widened.
    let pad = add(p, K::Generator);
    set(p, pad, "Strings", &[(0, 0.13), (1, 0.0), (2, 0.9), (3, 1.0), (4, 0.85), (5, 2.2), (6, 16.0), (7, 4.0)]);
    let chorus = p.chain_insert(pad, 0, K::Flanger).unwrap();
    set(p, chorus, "Ensemble", &[(0, 1.0), (2, 0.7), (3, 0.2), (5, 0.6)]);
    let phase = p.chain_insert(pad, 1, K::Phaser).unwrap();
    set(p, phase, "Pad Phase", &[(0, 0.12), (1, 0.7), (2, 300.0), (3, 3000.0), (5, 0.3), (6, 0.35)]);
    let pad_eq = p.chain_insert(pad, 2, K::Eq).unwrap();
    set(p, pad_eq, "Pad Tone", &[(0, 0.4), (2, 0.7), (3, 180.0)]);
    let wide = p.chain_insert(pad, 3, K::StereoExpander).unwrap();
    set(p, wide, "Wide", &[(0, 1.4), (1, 150.0)]);
    send(p, pad, hall);

    // Voices for the bridge: a saw that the Vocal Filter makes talk, and
    // a comb filter makes ring in D.
    let voices = add(p, K::Generator);
    set(p, voices, "Voices", &[(0, 0.22), (1, 0.0), (2, 0.5), (3, 1.0), (4, 0.9), (5, 1.5), (6, 10.0), (7, 3.0)]);
    let voice = p.chain_insert(voices, 0, K::VocalFilter).unwrap();
    set(p, voice, "Voice", &[(2, 1.3), (4, 2.5)]);
    let ring_d = p.chain_insert(voices, 1, K::CombFilter).unwrap();
    set(p, ring_d, "Ring in D", &[(0, 50.0), (2, 0.7), (3, 0.4), (4, 0.3)]);
    let far = p.chain_insert(voices, 2, K::Echo).unwrap();
    set(p, far, "Far Echo", &[(0, 0.43), (1, 0.4), (2, 0.5), (3, 0.5), (4, 0.2)]);
    send(p, voices, hall);

    // The felt piano: two velocity layers, a filter that closes as notes
    // ring, and a little tone shaping on the way to the hall.
    let piano = add(p, K::Sampler);
    set(p, piano, "Felt Piano", &[(0, 0.3), (3, 0.0), (4, 4.0), (5, 1.0), (6, 0.6)]);
    let sr = 44100.0;
    let mut soft = rendered(felt_piano(sr, false));
    soft.velocities = [0, 84];
    let mut hard = rendered(felt_piano(sr, true));
    hard.velocities = [85, 127];
    let m = p.module_mut(piano).unwrap();
    m.samples = vec![soft, hard];
    m.modulation.filter = true;
    m.modulation.cutoff = 5000.0;
    m.modulation.resonance = 0.1;
    m.modulation.filter_env =
        VoiceEnvelope { on: true, points: vec![(0.0, 1.0), (1.5, 0.55)], sustain: None, curve: true, amount: 1.5 };
    let piano_eq = p.chain_insert(piano, 0, K::Eq).unwrap();
    set(p, piano_eq, "Piano Tone", &[(0, 0.8), (1, 1.1), (2, 0.9)]);
    let air = p.chain_insert(piano, 1, K::Exciter).unwrap();
    set(p, air, "Air", &[(0, 5000.0), (1, 2.0), (2, 0.15)]);
    send(p, piano, hall);

    // The swell: one shot, stretched over four beats whatever the tempo.
    let swell_id = add(p, K::Sampler);
    set(p, swell_id, "Swell", &[(0, 0.7), (6, 0.05)]);
    let mut s = rendered(swell(sr));
    s.oneshot = true;
    s.beat_sync = 16;
    p.module_mut(swell_id).unwrap().samples = vec![s];
    let level = p.chain_insert(swell_id, 0, K::Amplifier).unwrap();
    set(p, level, "Swell Level", &[(0, 0.6), (1, -0.2)]);
    let lows_out = p.chain_insert(swell_id, 1, K::FilterPro).unwrap();
    set(p, lows_out, "Swell Cut", &[(0, 1.0), (1, 140.0), (4, 1.0)]);
    let plate = p.chain_insert(swell_id, 2, K::PlateReverb).unwrap();
    set(p, plate, "Swell Plate", &[(0, 0.8), (1, 0.03), (4, 0.4)]);
    send(p, swell_id, hall);

    // The arp: a soft pulse with a filter envelope, playing its phrases,
    // with a little bite from a scream filter before its echo.
    let arp = add(p, K::Generator);
    set(p, arp, "Ripple", &[(0, 0.07), (1, 1.0), (2, 0.002), (3, 0.15), (4, 0.15), (5, 0.2), (8, 0.25)]);
    let m = p.module_mut(arp).unwrap();
    m.modulation.filter = true;
    m.modulation.cutoff = 1100.0;
    m.modulation.resonance = 0.35;
    m.modulation.filter_env =
        VoiceEnvelope { on: true, points: vec![(0.0, 1.0), (0.2, 0.25)], sustain: None, curve: true, amount: 2.5 };
    m.phrases = vec![ripple(), sparkle()];
    m.phrase_mode = PhraseMode::Program;
    let arp_echo = p.chain_insert(arp, 0, K::Delay).unwrap();
    set(p, arp_echo, "Arp Echo", &[(0, 3.0), (1, 0.4), (2, 0.3), (3, 0.8)]);
    let bite = p.chain_insert(arp, 0, K::ScreamFilter).unwrap();
    set(p, bite, "Bite", &[(1, 2500.0), (2, 0.6), (3, 2.0), (4, 0.35)]);
    send(p, arp, hall);

    // The lead: a MultiSynth playing an FM voice with a late vibrato, and
    // a saw the Vocal Filter turns into a choir, into one echo.
    let lead = add(p, K::MultiSynth);
    set(p, lead, "Lead", &[(3, 5.0), (5, 0.1)]);
    let lead_fm = add(p, K::Fm);
    set(
        p,
        lead_fm,
        "Lead FM",
        &[(0, 0.2), (1, 1.0), (2, 1.6), (3, 0.8), (4, 0.15), (5, 0.03), (6, 1.0), (7, 0.6), (8, 0.6)],
    );
    let m = p.module_mut(lead_fm).unwrap();
    m.modulation.vibrato = VoiceLfo { on: true, shape: 0, rate: 5.2, depth: 0.18, delay: 0.35 };
    let lead_saw = add(p, K::Generator);
    set(
        p,
        lead_saw,
        "Lead Choir",
        &[(0, 0.12), (1, 0.0), (2, 0.12), (3, 0.6), (4, 0.8), (5, 0.7), (6, 12.0), (7, 3.0)],
    );
    let choir = p.chain_insert(lead_saw, 0, K::VocalFilter).unwrap();
    set(p, choir, "Choir", &[(4, 2.2), (5, 0.85)]);
    let waver = p.chain_insert(lead_saw, 1, K::Vibrato).unwrap();
    set(p, waver, "Waver", &[(0, 5.2), (1, 0.12), (2, 0.25)]);
    let lead_echo = add(p, K::Delay);
    set(p, lead_echo, "Lead Echo", &[(0, 6.0), (1, 0.35), (2, 0.25), (3, 0.6)]);
    // The choir slides between notes played over each other.
    let slide = add(p, K::Glide);
    set(p, slide, "Choir Slide", &[(0, 1.0), (1, 0.07)]);
    p.connect(lead, lead_fm);
    p.connect(lead, slide);
    p.connect(slide, lead_saw);
    p.connect(lead_fm, lead_echo);
    let warm = p.chain_insert(lead_fm, 0, K::AnalogFilter).unwrap();
    set(p, warm, "Warm", &[(1, 3800.0), (2, 0.25), (3, 1.4)]);
    send(p, lead_saw, lead_echo);
    p.connect(lead_echo, hall);

    // Bells: FM, a touch of ring modulation, panned to and fro, with a
    // shimmer an octave up.
    let bells = add(p, K::Fm);
    set(p, bells, "Bells", &[(0, 0.12), (1, 3.5), (2, 3.0), (3, 1.2), (5, 0.001), (6, 2.0), (7, 0.0), (8, 1.5)]);
    let ring = p.chain_insert(bells, 0, K::RingMod).unwrap();
    set(p, ring, "Shimmer", &[(0, 1180.0), (2, 0.25), (3, 0.2)]);
    let sway = p.chain_insert(bells, 1, K::Lfo).unwrap();
    set(p, sway, "Sway", &[(0, 1.0), (2, 0.6), (4, 1.0), (5, 8.0)]);
    let bell_echo = p.chain_insert(bells, 2, K::Delay).unwrap();
    set(p, bell_echo, "Bell Echo", &[(0, 3.0), (1, 0.5), (2, 0.35), (3, 1.0)]);
    let bell_eq = p.chain_insert(bells, 3, K::Eq5).unwrap();
    set(p, bell_eq, "Bell Tone", &[(1, 0.5), (7, 1.3), (13, 0.7)]);
    let octave = p.chain_insert(bells, 4, K::PitchShifter).unwrap();
    set(p, octave, "Octave Up", &[(4, 0.3)]);
    send(p, bells, hall);

    // Glass: a hollow SpectraVoice, its odd harmonics shimmering, for the
    // outro's high notes, scattered by a multitap delay.
    let glass = add(p, K::SpectraVoice);
    set(p, glass, "Glass", &[(0, 0.1), (1, 16.0), (2, 1.3), (3, 0.15), (4, 0.002), (5, 0.6), (6, 1.5), (9, 3.0)]);
    let scatter = p.chain_insert(glass, 0, K::Multitap).unwrap();
    set(p, scatter, "Scatter", &[(0, 3.0), (3, 5.0), (6, 7.0), (9, 11.0), (12, 0.2), (13, 0.35)]);
    send(p, glass, hall);

    // Keys: an FMX electric piano, two stacks with a bright tine, for the
    // outro's chords.
    let keys = add(p, K::Fmx);
    let tine = [(3, 1.0), (4, 1.0), (6, 2.5), (7, 0.2), (9, 0.35), (10, 14.0), (13, 0.0), (15, 0.8)];
    set(p, keys, "Keys", &[&[(0, 0.35), (1, 4.0)], &tine[..], &[(16, 1.0), (21, 0.4), (22, 1.0), (24, 1.5)]].concat());
    let shimmer = p.chain_insert(keys, 0, K::Chorus).unwrap();
    set(p, shimmer, "Keys Chorus", &[(1, 0.6), (2, 0.4), (6, 0.4)]);
    send(p, keys, hall);

    // Modulators: the strings breathe brighter and darker over four bars,
    // and the strings and arp duck under the drums.
    let breath = add(p, K::Modulator);
    set(p, breath, "Breath", &[(3, 1.0), (4, 64.0), (5, 0.35)]);
    p.connect(breath, pad_eq);
    p.set_control_param(breath, pad_eq, 2);
    let duck = add(p, K::Modulator);
    set(p, duck, "Duck", &[(0, 1.0), (5, -0.55), (6, 0.004), (7, 0.25)]);
    p.connect(stutter, duck);
    let fader = K::Generator.params().len();
    for target in [pad, arp] {
        p.connect(duck, target);
        p.set_control_param(duck, target, fader);
    }

    Ids {
        drums,
        sub_kick,
        bass,
        pad,
        voices,
        piano,
        swell: swell_id,
        arp,
        lead,
        bells,
        glass,
        keys,
        bass_filter,
        stutter,
        choir,
        voice,
    }
}

/// The verse's phrase: root, fifth and octaves, rising and falling.
fn ripple() -> Phrase {
    let mut ph = Phrase { lines: 8, lpb: 8, looping: true, ..Phrase::default() };
    for (l, note) in [48, 55, 60, 67, 72, 67, 60, 55].into_iter().enumerate() {
        ph.cells[l] =
            Cell { note: Some(Note::On(note)), vol: Some(if l % 4 == 0 { 0x80 } else { 0x50 }), ..Cell::default() };
    }
    ph
}

/// The chorus's phrase: higher, clipped short with Cxx, and one note
/// left to chance with Yxx.
fn sparkle() -> Phrase {
    let mut ph = Phrase { lines: 16, lpb: 8, looping: true, keys: [0, 119], ..Phrase::default() };
    for (l, note) in [60, 72, 67, 74, 79, 74, 67, 72, 60, 72, 67, 74, 79, 84, 79, 74].into_iter().enumerate() {
        let cmd = if l == 13 { (FX_MAYBE, 0x60) } else { (0xC, 0x03) };
        ph.cells[l] = Cell {
            note: Some(Note::On(note)),
            vol: Some(if l % 2 == 0 { 0x70 } else { 0x48 }),
            fx: Some(cmd),
            ..Cell::default()
        };
    }
    ph
}

/// Piano chords: the left hand an octave under the root, the right the
/// chord, strummed with the delay column. `every` repeats them within
/// the bar.
fn piano_chords(pat: &mut Pattern, piano: u8, chords: &[Chord; 4], vol: u8, every: usize, lift: u8) {
    for (bar, ch) in chords.iter().enumerate() {
        for at in (0..16).step_by(every).map(|l| bar * 16 + l) {
            let accent = if at % 16 == 0 { vol } else { vol.saturating_sub(0x18) };
            *pat.cell_mut(PIANO, 0, at) = n(ch.root + 12, piano, accent);
            for (c, &tone) in ch.tones.iter().enumerate() {
                *pat.cell_mut(PIANO, c + 1, at) =
                    Cell { delay: Some(0x14 * (c as u8 + 1)), ..n(tone + lift, piano, accent) };
            }
        }
    }
}

fn pad_chords(pat: &mut Pattern, pad: u8, chords: &[Chord; 4], vol: u8) {
    for (bar, ch) in chords.iter().enumerate() {
        for (c, &tone) in ch.tones.iter().enumerate() {
            *pat.cell_mut(PAD, c, bar * 16) = n(tone, pad, vol);
        }
    }
}

/// The arp's root on each bar, playing phrase `phrase` with Zxx.
fn arp_line(pat: &mut Pattern, arp: u8, chords: &[Chord; 4], phrase: u8) {
    for (bar, ch) in chords.iter().enumerate() {
        pat.tracks[ARP][bar * 16] = fx(n(ch.arp, arp, 0x60), FX_PHRASE, phrase);
        pat.tracks[ARP][bar * 16 + 15] = off();
    }
}

/// Closed hats on the off-beats, panned left and right with the panning
/// column, and an open hat cut short with Cxx at the end of each half.
fn hats(pat: &mut Pattern, drums: u8, every: usize, vol: u8) {
    for l in (every / 2..64).step_by(every) {
        let pan = if (l / every).is_multiple_of(2) { 0x28 } else { 0x58 };
        pat.tracks[HATS][l] = Cell { pan: Some(pan), ..n(54, drums, vol) };
    }
    for l in [30, 62] {
        pat.tracks[HATS][l] = fx(n(58, drums, vol + 0x10), 0xC, 0x04);
    }
}

fn intro(ids: &Ids) -> Pattern {
    let mut pat = Pattern::new("", 8, 64);
    pat.set_columns(PIANO, 5);
    piano_chords(&mut pat, ids.piano, &VERSE, 0x44, 16, 0);
    // The tune, softly, in the piano's fifth column.
    for (l, note) in MOTIF {
        *pat.cell_mut(PIANO, 4, l) = n(note, ids.piano, 0x4C);
    }
    // The swell throws the song into the verse.
    pat.tracks[BELLS][48] = n(48, ids.swell, 0x70);
    pat
}

fn verse(ids: &Ids) -> Pattern {
    let mut pat = Pattern::new("", 8, 64);
    pat.set_columns(PAD, 3);
    pat.set_columns(PIANO, 5);
    pad_chords(&mut pat, ids.pad, &VERSE, 0x58);
    piano_chords(&mut pat, ids.piano, &VERSE, 0x50, 8, 0);
    arp_line(&mut pat, ids.arp, &VERSE, 1);
    for (bar, ch) in VERSE.iter().enumerate() {
        pat.tracks[BASS][bar * 16] = n(ch.root, ids.bass, 0x60);
        pat.tracks[BASS][bar * 16 + 10] = n(ch.root + 12, ids.bass, 0x40);
        pat.tracks[BASS][bar * 16 + 14] = off();
    }
    // A heartbeat kick, and soft hats.
    for l in [0, 6, 32, 38] {
        pat.tracks[DRUMS][l] = n(48, ids.drums, if l % 32 == 0 { 0x70 } else { 0x48 });
    }
    hats(&mut pat, ids.drums, 4, 0x28);
    pat
}

fn build(ids: &Ids) -> Pattern {
    let mut pat = Pattern::new("", 8, 64);
    pat.set_columns(PAD, 3);
    pat.set_columns(PIANO, 5);
    pad_chords(&mut pat, ids.pad, &BUILD, 0x60);
    arp_line(&mut pat, ids.arp, &BUILD, 1);
    // The piano pulses in eighths, louder and louder.
    for (bar, ch) in BUILD.iter().enumerate() {
        for k in 0..8 {
            let at = bar * 16 + k * 2;
            let vol = 0x38 + (at as u8) / 2;
            *pat.cell_mut(PIANO, 0, at) = n(ch.root + 12, ids.piano, vol);
            for (c, &tone) in ch.tones.iter().enumerate() {
                *pat.cell_mut(PIANO, c + 1, at) = n(tone, ids.piano, vol);
            }
        }
        // The bass pulses too, and in the last bar slides up with 1xx.
        for k in 0..8 {
            let at = bar * 16 + k * 2;
            let note = n(ch.root, ids.bass, 0x58);
            pat.tracks[BASS][at] = if bar == 3 { fx(note, 0x1, 0x02) } else { note };
            if bar == 3 {
                pat.tracks[BASS][at + 1].fx = Some((0x1, 0x02));
            }
        }
    }
    // Four on the floor from the second bar, the snare closing in, a
    // flam with Dxx and a roll with Exx.
    for l in (16..64).step_by(4) {
        pat.tracks[DRUMS][l] = n(48, ids.drums, 0x60);
    }
    for l in [24, 40, 44] {
        pat.tracks[DRUMS][l + 2] = n(50, ids.drums, 0x50);
    }
    pat.tracks[DRUMS][46] = fx(n(50, ids.drums, 0x30), 0xD, 0x03);
    for (l, rate) in [(48, 0x06), (52, 0x03), (56, 0x02), (60, 0x01)] {
        pat.tracks[DRUMS][l] = fx(n(50, ids.drums, 0x40 + (l as u8 - 48) * 3), 0xE, rate);
        for k in 1..4 {
            pat.tracks[DRUMS][l + k].fx = Some((0xE, rate));
        }
    }
    hats(&mut pat, ids.drums, 2, 0x30);
    pat.tracks[BELLS][48] = n(48, ids.swell, 0x78);
    // The bass filter opens over the pattern, and the drums stutter in
    // shorter and shorter loops at its end.
    let k = ModuleKind::Filter;
    pat.automation.push(envelope(ids.bass_filter, k, 1, &[(0.0, 300.0), (64.0, 4000.0)], false, true));
    let r = ModuleKind::Repeater;
    pat.automation.push(envelope(ids.stutter, r, 0, &[(0.0, 0.0), (56.0, 1.0)], true, false));
    pat.automation.push(envelope(ids.stutter, r, 1, &[(56.0, 6.0), (60.0, 8.0), (62.0, 9.0)], true, false));
    pat
}

fn chorus(ids: &Ids, last: bool) -> Pattern {
    let mut pat = Pattern::new("", 8, 64);
    pat.set_columns(PAD, 3);
    pat.set_columns(PIANO, 5);
    pat.set_columns(LEAD, 2);
    pat.set_columns(DRUMS, 2);
    pad_chords(&mut pat, ids.pad, &CHORUS, 0x70);
    piano_chords(&mut pat, ids.piano, &CHORUS, 0x68, 8, if last { 12 } else { 0 });
    arp_line(&mut pat, ids.arp, &CHORUS, 2);
    // The bass in eighths, gliding into each bar with 3xx.
    for (bar, ch) in CHORUS.iter().enumerate() {
        for k in 0..8 {
            let at = bar * 16 + k * 2;
            let note = if k == 3 || k == 7 { ch.root + 12 } else { ch.root };
            let cell = n(note, ids.bass, if k % 2 == 0 { 0x68 } else { 0x48 });
            pat.tracks[BASS][at] = if k == 0 { fx(cell, 0x3, 0x30) } else { cell };
        }
    }
    // A half-time beat with a ghost snare, the kicks doubled by the sub
    // kick in the second column.
    for l in [0, 10, 32, 42] {
        pat.tracks[DRUMS][l] = n(48, ids.drums, 0x78);
        *pat.cell_mut(DRUMS, 1, l) = n(26, ids.sub_kick, 0x70);
    }
    for l in [16, 48] {
        pat.tracks[DRUMS][l] = n(50, ids.drums, 0x70);
    }
    pat.tracks[DRUMS][29] = fx(n(50, ids.drums, 0x20), 0xD, 0x02);
    if last {
        for l in [20, 52] {
            pat.tracks[DRUMS][l] = n(48, ids.drums, 0x60);
        }
    }
    hats(&mut pat, ids.drums, 2, 0x38);
    // The melody, with vibrato on its long notes and a glide up to the
    // high F; the last chorus adds a harmony a third below.
    for (l, note, len) in MELODY {
        let mut cell = n(note, ids.lead, 0x70);
        if note == 77 {
            cell = fx(cell, 0x3, 0x18);
        }
        pat.tracks[LEAD][l] = cell;
        if len >= 6 {
            for k in l + 3..l + len {
                pat.tracks[LEAD][k].fx = Some((0x4, 0x35));
            }
        }
        if last {
            *pat.cell_mut(LEAD, 1, l) = n(third_below(note), ids.lead, 0x50);
        }
    }
    // The choir sings A, opens to O and closes to E over the pattern.
    let v = ModuleKind::VocalFilter;
    pat.automation.push(envelope(ids.choir, v, 0, &[(0.0, 0.0), (24.0, 3.0), (48.0, 1.0), (64.0, 0.0)], false, true));
    if last {
        // Bell arpeggios on the off-beats, with 0xy.
        for (bar, ch) in CHORUS.iter().enumerate() {
            for k in [4, 12] {
                pat.tracks[BELLS][bar * 16 + k] = fx(n(ch.tones[1] + 24, ids.bells, 0x40), 0x0, 0x7C);
            }
        }
    }
    pat
}

fn bridge(ids: &Ids) -> Pattern {
    let mut pat = Pattern::new("", 8, 64);
    pat.tracks[LEAD][0] = off();
    pat.set_columns(PAD, 3);
    pat.set_columns(PIANO, 5);
    // Voices instead of strings, trembling with 7xy, and talking: their
    // Vocal Filter goes through the vowels.
    for (bar, ch) in BRIDGE.iter().enumerate() {
        for (c, &tone) in ch.tones.iter().enumerate() {
            let at = bar * 16;
            *pat.cell_mut(PAD, c, at) = n(tone, ids.voices, 0x60);
            for k in 8..16 {
                pat.cell_mut(PAD, c, at + k).fx = Some((0x7, 0x44));
            }
        }
        pat.tracks[BASS][bar * 16] = n(ch.root, ids.bass, 0x50);
        pat.tracks[BASS][bar * 16 + 15] = off();
        // A few piano notes from the tune.
        *pat.cell_mut(PIANO, 4, bar * 16) = n(ch.tones[2] + 12, ids.piano, 0x40);
        *pat.cell_mut(PIANO, 4, bar * 16 + 6) = n(ch.tones[1] + 12, ids.piano, 0x34);
        // Bells that may or may not ring (Yxx), swinging left and right
        // (Nxy) from where 8xx puts them.
        for k in (2..16).step_by(2) {
            let note = ch.tones[k / 2 % 3] + 24 + if k > 8 { 12 } else { 0 };
            let cell = fx(n(note, ids.bells, 0x38 + k as u8), FX_MAYBE, 0x70);
            pat.tracks[BELLS][bar * 16 + k] = cell;
            pat.tracks[BELLS][bar * 16 + k + 1].fx =
                Some(if k % 4 == 0 { (FX_AUTOPAN, 0x48) } else { (0x8, if k % 8 == 2 { 0x40 } else { 0xC0 }) });
        }
    }
    let v = ModuleKind::VocalFilter;
    let vowels: Vec<(f32, f32)> =
        [0.0, 1.0, 2.0, 3.0, 4.0, 2.0, 0.0].iter().enumerate().map(|(k, &x)| (k as f32 * 10.0, x)).collect();
    pat.automation.push(envelope(ids.voice, v, 0, &vowels, false, true));
    // The swell comes in late, from a quarter of the way in with 9xx.
    pat.tracks[BELLS][52] = fx(n(48, ids.swell, 0x78), 0x9, 0x40);
    pat
}

fn outro(ids: &Ids) -> Pattern {
    let mut pat = Pattern::new("", 8, 64);
    pat.set_columns(LEAD, 2);
    pat.tracks[LEAD][0] = off();
    *pat.cell_mut(LEAD, 1, 0) = off();
    pat.set_columns(PAD, 3);
    pat.set_columns(PIANO, 5);
    piano_chords(&mut pat, ids.piano, &OUTRO, 0x4C, 16, 0);
    for (l, note) in MOTIF.into_iter().take(8) {
        *pat.cell_mut(PIANO, 4, l) = n(note, ids.piano, 0x48);
    }
    *pat.cell_mut(PIANO, 4, 32) = n(64, ids.piano, 0x44);
    *pat.cell_mut(PIANO, 4, 48) = n(66, ids.piano, 0x50);
    for (bar, ch) in OUTRO.iter().enumerate() {
        for (c, &tone) in ch.tones.iter().enumerate() {
            *pat.cell_mut(PAD, c, bar * 16) = n(tone, ids.pad, 0x40);
        }
    }
    // Glass rings an octave over the top of each chord after the first,
    // the Keys a fifth below it.
    for (bar, ch) in OUTRO.iter().enumerate().skip(1) {
        pat.tracks[LEAD][bar * 16] = n(ch.tones[2] + 12, ids.glass, 0x50);
        *pat.cell_mut(LEAD, 1, bar * 16) = n(ch.tones[0] + 12, ids.keys, 0x48);
    }
    // The bass holds the last D and sinks away with 2xx.
    pat.tracks[BASS][48] = n(26, ids.bass, 0x40);
    for l in 56..64 {
        pat.tracks[BASS][l].fx = Some((0x2, 0x01));
    }
    // Slowing down with Fxx, the last chord fading with Axy.
    for (l, bpm) in [(24, 80), (32, 76), (40, 70), (46, 64), (52, 58)] {
        pat.tracks[DRUMS][l].fx = Some((0xF, bpm));
    }
    for l in 54..64 {
        for c in 0..5 {
            pat.cell_mut(PIANO, c, l).fx = Some((0xA, 0x01));
        }
    }
    let out = ModuleKind::Output;
    pat.automation.push(envelope(OUTPUT_ID, out, 0, &[(40.0, 0.5), (64.0, 0.2)], false, true));
    pat
}

const COMMENTS: &str = "Last Light — the demo song. Press Space to play it and F1 for the manual.\n\n\
    A felt piano plays alone, then strings, a sub bass and an arp come in. A build stutters into the chorus, \
    where a choir-like lead sings; the bridge drops to voices that talk and bells that may or may not ring; \
    the last chorus adds a harmony and bell arpeggios, and the piano slows down to end on D major.\n\n\
    Where to look:\n\
    • Piano (F4): a Sampler with two velocity layers, rendered by the demo itself, and a filter envelope. \
    Its chords are strummed with the delay column.\n\
    • Strings: a chain of Flanger, Phaser and EQ into the shared Hall; the Breath Modulator moves the EQ, \
    and Duck, following the drums, pulls the strings and the arp down under each hit.\n\
    • Arp: two phrases, picked with Z01 and Z02; the second clips its notes with Cxx and leaves one to chance with Yxx.\n\
    • Lead: a MultiSynth playing an FM voice (with a late vibrato on its Modulation page) and, through a \
    Glide that slides tied notes, a saw through a Vocal Filter, whose vowel is automated in the chorus.\n\
    • Build: the bass filter opens, the bass slides up with 1xx, the snare rolls with Exx, and the Stutter \
    Repeater's Hold is automated at the end.\n\
    • Bridge: Voices talk through a Vocal Filter and tremble with 7xy; Bells use Yxx, Nxy and 8xx, through a Ring Mod and an LFO.\n\
    • The swell is a one-shot, beat-synced sample; in the bridge it starts a quarter in with 9xx.\n\
    • Outro: Fxx slows the song, Axy fades the last chord, 2xx sinks the bass and the master volume is automated.\n\
    • The matrix holds back the hats and arp in the first verse, and the sections name the parts.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_demo_uses_every_module_and_nearly_every_effect() {
        let p = Project::demo();
        // All but the Input: opening the demo shouldn't open the microphone.
        for kind in ModuleKind::ADDABLE.into_iter().filter(|&k| k != ModuleKind::Input) {
            assert!(p.modules.iter().any(|m| m.kind == kind), "{}", kind.name());
        }
        // And everything that makes sound is heard: links lead from it to
        // the output.
        // Over the whole path, copies for track effects and all.
        let graph = p.signal_graph().1;
        let links: Vec<(u8, u8)> = graph.iter().map(|(a, b)| (a.0, b.0)).collect();
        // The hats' copy of the drum kit, past its own effects, reaches the
        // hats' echo.
        let hat_echo = p.tracks[HATS].effects[0];
        assert!(graph.iter().any(|(a, b)| a.1 == Some(HATS) && *b == (hat_echo, None)), "hats through their echo");
        let heard = |id: u8| {
            let mut seen = vec![id];
            while let Some(at) = seen.pop() {
                if at == OUTPUT_ID {
                    return true;
                }
                seen.extend(links.iter().filter(|l| l.0 == at).map(|l| l.1));
            }
            false
        };
        for m in p.modules.iter().filter(|m| m.kind.makes_sound() && m.kind.has_output()) {
            assert!(heard(m.id), "{} reaches the output", m.name);
        }
        let mut commands: Vec<u8> = Vec::new();
        let cells = p.patterns.iter().flat_map(|pat| (0..pat.num_lanes()).flat_map(move |l| pat.lane(l).iter()));
        let phrase_cells = p.modules.iter().flat_map(|m| m.phrases.iter().flat_map(|ph| ph.cells.iter()));
        let (mut pan, mut delay) = (false, false);
        for c in cells.chain(phrase_cells) {
            commands.extend(c.fx.map(|f| f.0));
            pan |= c.pan.is_some();
            delay |= c.delay.is_some();
        }
        // Bxx, the jump, is the one the song does without.
        for cmd in [0x0, 0x1, 0x2, 0x3, 0x4, 0x7, 0x8, 0x9, 0xA, 0xC, 0xD, 0xE, 0xF, FX_AUTOPAN, FX_MAYBE, FX_PHRASE] {
            assert!(commands.contains(&cmd), "effect {:?}", char::from_digit(cmd as u32, 36));
        }
        assert!(pan && delay, "the panning and delay columns");
        assert!(p.patterns.iter().filter(|pat| !pat.automation.is_empty()).count() >= 4);
        assert!(p.order.iter().any(|s| s.muted != 0) && p.sections.len() >= 6);
        assert!(p.modules.iter().any(|m| m.phrases.len() >= 2));
        assert!(
            p.modules
                .iter()
                .any(|m| m.kind == ModuleKind::Sampler && m.samples.iter().any(|s| s.velocities != [0, 127]))
        );
    }

    #[test]
    fn harmony_is_a_third_below_in_d_minor() {
        // D → Bb, F → D, A → F, E → C.
        assert_eq!([third_below(74), third_below(77), third_below(69), third_below(76)], [70, 74, 65, 72]);
    }
}
