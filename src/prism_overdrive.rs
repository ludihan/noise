//! "Prism Overdrive", the fourth demo song: a minute and a half of
//! rhythm-game hardcore in D major at 185 BPM, the kind of boss song a
//! chart is written for.
//!
//! A piano plays the hook's chords alone; the kick comes in through a
//! filter, a snare rolls up its keys, and the drop hits: a hardcore kick
//! with a bass on the off-beats, supersaws pumping under it and a lead
//! singing the hook. A psytrance part rolls the bass between the kicks
//! under gated stabs, chopped vocals, and the pluck's stairs, trills and
//! jacks; the hook comes back bigger; the boss part goes half time with a
//! growling bass, speeds up to 255 BPM for a bar of kick rolls and glitches
//! out; the piano breaks it down; a kick roll lifts it a whole tone, and the
//! last chorus ends on one big hit stuttering away.
//!
//! The kit, the orchestra hit, the piano and the voice are sampled here, so
//! its Samplers have audio without files; they are written next to the song
//! when it is saved.

use crate::demo::{
    Hit, effects, envelope, felt_piano, fx, hit, hold, let_go, n, normalize, off, rendered, roll, set, tune,
};
use crate::dsp::Frame;
use crate::project::*;
use crate::sample::Sample;
use crate::static_heart::syllables;
use std::f32::consts::TAU;

const BPM: f32 = 185.0;
const LPB: usize = 4;
/// Lines a bar, and a phrase of eight bars: most patterns are one.
const BAR: usize = 4 * LPB;
const PHRASE: usize = 8 * BAR;
/// The kick's note: it is tuned to the key, D.
const KICK_NOTE: u8 = 38;

// ---------------------------------------------------------------- harmony

/// A chord: the bass's note, the supersaws' four, and whether the pluck
/// arpeggiates it major or minor.
#[derive(Clone, Copy)]
struct Chord {
    bass: u8,
    stack: [u8; 4],
    major: bool,
}

const fn major(bass: u8, stack: [u8; 4]) -> Chord {
    Chord { bass, stack, major: true }
}

const fn minor(bass: u8, stack: [u8; 4]) -> Chord {
    Chord { bass, stack, major: false }
}

// MIDI numbers: 48 is the tracker's C-4.
const G: Chord = major(43, [59, 62, 66, 69]);
const A: Chord = major(45, [57, 61, 64, 71]);
const FSM: Chord = minor(42, [57, 61, 64, 66]);
const BM: Chord = minor(47, [57, 61, 62, 66]);
const BB: Chord = major(46, [58, 62, 65, 70]);
const C: Chord = major(48, [60, 64, 67, 72]);
const D: Chord = major(50, [57, 62, 66, 69]);
const EM: Chord = minor(40, [59, 62, 64, 67]);
const BSUS: Chord = major(47, [59, 64, 66, 71]);
const B: Chord = major(47, [59, 63, 66, 71]);

/// The hook's chords, a bar each: IV, V, iii and vi, then IV and V and the
/// borrowed bVI and bVII climbing back.
const HOOK_CHORDS: [Chord; 8] = [G, A, FSM, BM, G, A, BB, C];
/// The rolling part's: vi, IV, I and V, twice.
const ROLL_CHORDS: [Chord; 8] = [BM, G, D, A, BM, G, D, A];
/// The first rise's, a bar each, and the breakdown's, two bars each.
const RISE_CHORDS: [Chord; 4] = [EM, FSM, G, A];
const CALM_CHORDS: [Chord; 4] = [G, A, FSM, BM];
/// The second rise's: up to B, the V of the last chorus's key, E.
const LIFT_CHORDS: [Chord; 4] = [G, A, BSUS, B];

/// The hook, over `HOOK_CHORDS`: (line, note). Each bar sets off in threes,
/// three, three and two sixteenths, and the last climbs.
const HOOK: &[(usize, u8)] = &[
    // G
    (0, 86),
    (3, 83),
    (6, 86),
    (8, 88),
    (10, 90),
    (12, 88),
    (14, 86),
    // A
    (16, 85),
    (19, 81),
    (22, 85),
    (24, 88),
    (28, 86),
    (30, 85),
    // F#m
    (32, 85),
    (35, 81),
    (38, 85),
    (40, 86),
    (42, 88),
    (44, 86),
    (46, 85),
    // Bm
    (48, 86),
    (54, 83),
    (56, 85),
    (58, 86),
    (60, 90),
    // G
    (64, 86),
    (67, 83),
    (70, 86),
    (72, 88),
    (74, 90),
    (76, 91),
    (78, 90),
    // A
    (80, 88),
    (83, 85),
    (86, 88),
    (88, 93),
    (92, 90),
    (94, 88),
    // Bb
    (96, 89),
    (99, 86),
    (102, 89),
    (104, 91),
    (106, 89),
    (108, 86),
    (110, 89),
    // C
    (112, 91),
    (115, 88),
    (118, 91),
    (120, 93),
    (124, 95),
    (126, 96),
];

/// The growl's riff, two bars of it: (line, note, glide into it).
const GROWL_RIFF: [(usize, u8, bool); 15] = [
    (0, 35, false),
    (3, 35, false),
    (6, 38, false),
    (8, 35, false),
    (10, 42, true),
    (11, 40, false),
    (12, 35, false),
    (14, 47, false),
    (16, 35, false),
    (19, 35, false),
    (22, 45, false),
    (24, 42, false),
    (26, 40, true),
    (28, 38, false),
    (30, 33, false),
];

// ---------------------------------------------------------------- samples

// The kit's keys.
const CLAP: u8 = 48;
/// The snare, and the three keys above it pitch it up, for fills.
const SNARE: u8 = 50;
const CLOSED: u8 = 54;
const OPEN: u8 = 58;
const RIDE: u8 = 59;
const CRASH: u8 = 61;
const REVERSE: u8 = 63;

/// The kit, each sound a sample on keys of its own: the clap, the snare
/// (pitched up on the keys above it), the closed and open hats in one mute
/// group, so a closed one cuts an open one short, the ride, the crash, and
/// the crash reversed, two beats long whatever the tempo.
fn rave_kit(sr: f32) -> Vec<SampleSlot> {
    let mut swell = hit(sr, Hit::Crash, 0x5E7E, 0.0);
    swell.truncate((sr * 120.0 / BPM) as usize);
    swell.reverse();
    let rise = swell.len() as f32 / 4.0;
    swell.iter_mut().enumerate().for_each(|(k, x)| *x *= (k as f32 / rise).min(1.0));
    let sounds = [
        ("Clap", hit(sr, Hit::Clap, 0xC1A9, 0.0), [CLAP, CLAP], 0, 0.0),
        ("Snare", hit(sr, Hit::Snare, 0x5A4E, 0.25), [SNARE, SNARE + 3], 0, 0.0),
        ("Closed Hat", hit(sr, Hit::Hat, 0x4A70, 0.0), [CLOSED, CLOSED], 1, 0.2),
        ("Open Hat", hit(sr, Hit::Open, 0x0BE4, 0.0), [OPEN, OPEN], 1, 0.2),
        ("Ride", hit(sr, Hit::Ride, 0x41DE, 0.0), [RIDE, RIDE], 0, -0.25),
        ("Crash", hit(sr, Hit::Crash, 0xC4A5, 0.0), [CRASH, CRASH], 0, 0.1),
        ("Reverse Crash", swell, [REVERSE, REVERSE], 0, 0.0),
    ];
    sounds
        .into_iter()
        .map(|(name, sound, keys, group, pan)| {
            let mut frames: Vec<Frame> = sound.iter().map(|&x| [x, x]).collect();
            normalize(&mut frames, 0.9);
            let mut slot = rendered(Sample { name: name.into(), sample_rate: sr, channels: 2, frames });
            slot.base_note = keys[0];
            slot.keys = keys;
            slot.mute_group = group;
            slot.panning = pan;
            slot.oneshot = true;
            if keys[0] == REVERSE {
                slot.beat_sync = 2 * LPB as u16;
            }
            slot
        })
        .collect()
}

/// An orchestra hit on C-4: strings and brass stabbing a C major chord over
/// three octaves, each harmonic dying sooner the higher it is, over a
/// timpani's thump and the bows' scrape.
fn orchestra_hit(sr: f32) -> Sample {
    const NOTES: [f32; 7] = [36.0, 48.0, 52.0, 55.0, 60.0, 64.0, 67.0];
    let len = (sr * 0.8) as usize;
    let mut frames = vec![[0.0f32; 2]; len];
    let attack = sr * 0.004;
    for (k, &note) in NOTES.iter().enumerate() {
        // Two sections a few cents apart, one on each side.
        for (side, cents) in [(0, -6.0f32), (1, 6.0)] {
            let f0 = 440.0 * 2f32.powf((note + cents / 100.0 - 69.0) / 12.0);
            for h in 1..=16 {
                let f = f0 * h as f32;
                if f > sr * 0.45 {
                    break;
                }
                // A phasor turned a step each frame, its level falling.
                let (s, c) = (TAU * f / sr).sin_cos();
                let start = (k + h + side) as f32 * 0.7;
                let mut z = [start.cos(), start.sin()];
                let (mut level, fall) = (1.0 / h as f32, (-(1.5 + 0.9 * h as f32) / sr).exp());
                for (i, fr) in frames.iter_mut().enumerate() {
                    z = [z[0] * c - z[1] * s, z[0] * s + z[1] * c];
                    fr[side] += z[1] * level * (i as f32 / attack).min(1.0);
                    level *= fall;
                    if level < 1e-4 {
                        break;
                    }
                }
            }
        }
    }
    let mut noise = crate::demo::Rng(0x0C4E_57A5);
    let (mut phase, mut lp) = (0.0f32, 0.0f32);
    for (i, fr) in frames.iter_mut().enumerate() {
        let t = i as f32 / sr;
        phase += (65.0 + 70.0 * (-t / 0.04).exp()) / sr;
        let w = noise.next();
        lp += 0.3 * (w - lp);
        let x = (TAU * phase).sin() * (-t / 0.25).exp() * 1.2 + (w - lp) * (-t / 0.03).exp() * 0.5;
        // Faded out by its end.
        let g = ((len - i) as f32 / (sr * 0.15)).min(1.0);
        *fr = [(fr[0] + x) * g, (fr[1] + x) * g];
    }
    normalize(&mut frames, 0.85);
    Sample { name: "Orchestra Hit (C)".into(), sample_rate: sr, channels: 2, frames }
}

/// The pluck's phrases: an arpeggio up and down two octaves of a major
/// chord (Z01) and of a minor one (Z02), a bar of sixteenths each, and a
/// trill in 32nds (Z03) for two beats.
fn pluck_phrases() -> Vec<Phrase> {
    let arp = |third: u8| {
        let steps = [0, 7, 12, 12 + third, 19, 24, 19, 12 + third, 12, 7, 12, 12 + third, 19, 24, 24 + third, 31];
        let mut ph = Phrase { lines: steps.len(), lpb: LPB as u32, ..Phrase::default() };
        for (l, &s) in steps.iter().enumerate() {
            let vol = if l % 4 == 0 { 0x70 } else { 0x50 };
            ph.cells[l] = Cell { note: Some(Note::On(48 + s)), vol: Some(vol), ..Cell::default() };
        }
        ph
    };
    let mut trill = Phrase { lines: 16, lpb: 2 * LPB as u32, ..Phrase::default() };
    for l in 0..trill.lines {
        let note = if l % 2 == 0 { 48 } else { 50 };
        trill.cells[l] = Cell { note: Some(Note::On(note)), vol: Some(0x40 + 3 * l as u8), ..Cell::default() };
    }
    vec![arp(4), arp(3), trill]
}

// ---------------------------------------------------------------- tracks

const KICK: usize = 0;
const KIT: usize = 1;
const BASS: usize = 2;
const GROWL: usize = 3;
const STACK: usize = 4;
const LEAD: usize = 5;
const PLUCK: usize = 6;
const PIANO: usize = 7;
const PAD: usize = 8;
const HOOVER: usize = 9;
const VOX: usize = 10;
const ORCH: usize = 11;
const FX: usize = 12;
const TRACKS: usize = 13;

/// The tracks with more than one note column. The kit's are the snare,
/// the clap, the hats and the cymbals.
const COLUMNS: [(usize, usize); 6] = [(KIT, 4), (STACK, 4), (LEAD, 2), (PIANO, 4), (PAD, 3), (FX, 2)];

/// The instruments and effects patterns refer to.
struct Ids {
    kick: u8,
    kick_filter: u8,
    boom: u8,
    kit: u8,
    bass: u8,
    bass_filter: u8,
    growl: u8,
    stack: u8,
    stack_filter: u8,
    lead: u8,
    pluck: u8,
    piano: u8,
    pad: u8,
    hoover: u8,
    vox: u8,
    orch: u8,
    uplifter: u8,
    sweep: u8,
    glitch: u8,
}

/// Makes every module and wires them up.
fn instruments(p: &mut Project) -> Ids {
    use ModuleKind as K;
    let add = |p: &mut Project, kind: ModuleKind, name: &str, params: &[(usize, f32)]| {
        let id = p.add_module(kind, [0.0, 0.0]).unwrap();
        set(p, id, name, params);
        id
    };
    // Sends the end of `id`'s chain to `to` rather than the output.
    let send = |p: &mut Project, id: u8, to: u8| {
        let last = p.chain(id).effects.last().copied().unwrap_or(id);
        p.disconnect(last, OUTPUT_ID);
        p.connect(last, to);
    };
    let sr = 44100.0;

    // The arena the melodic parts share, its mud taken out.
    let room = add(p, K::Reverb, "Arena", &[(0, 0.82), (1, 0.4), (2, 0.25)]);
    p.connect(room, OUTPUT_ID);
    effects(p, room, [(K::Eq10, "Arena Tone", &[(0, 0.25), (1, 0.3), (2, 0.55), (9, 0.7)])]);

    // The master: Glitch, a Repeater for the stutters, then glue, sparkle
    // and a limiter. The mix comes in a little down, so the limiter has
    // less to do.
    let [glitch, ..] = effects(
        p,
        OUTPUT_ID,
        [
            (K::Repeater, "Glitch", &[]),
            (K::Compressor, "Glue", &[(0, 0.45), (1, 2.5), (2, 0.005), (3, 0.12), (4, 1.2)]),
            (K::Eq5, "Sparkle", &[(0, 60.0), (1, 1.2), (12, 11000.0), (13, 1.25)]),
            (K::Maximizer, "Limiter", &[(0, 1.3), (1, 0.95), (2, 0.05)]),
        ],
    );
    p.module_mut(glitch).unwrap().gain = 0.6;

    // The kick: a Kicker falling five octaves onto D, its tail squared off
    // as a hardcore kick's, driven and filtered.
    let kick = add(p, K::Kicker, "Hard Kick", &[(0, 0.8), (2, 5.0), (3, 0.012), (5, 0.32), (6, 0.75)]);
    let [_, kick_filter, kick_weight] = effects(
        p,
        kick,
        [
            (K::Distortion, "Kick Drive", &[(0, 3.0), (1, 0.75), (2, 0.7)]),
            (K::FilterPro, "Kick Filter", &[(1, 20000.0), (4, 1.0)]),
            (K::Eq, "Kick Weight", &[(0, 1.3), (1, 0.7), (2, 1.2), (3, 80.0), (4, 350.0), (5, 4000.0)]),
        ],
    );
    // The drops' boom: a Kicker sinking slowly, in a plate.
    let boom = add(p, K::Kicker, "Drop Boom", &[(0, 0.6), (2, 3.0), (3, 0.25), (5, 2.5), (6, 0.4)]);
    effects(p, boom, [(K::PlateReverb, "Boom Plate", &[(0, 0.85), (1, 0.01), (2, 0.5), (4, 0.35)])]);

    // The kit: a Sampler with a sample for each sound on keys of its own.
    let kit = add(p, K::Sampler, "Rave Kit", &[(0, 1.0), (6, 0.08)]);
    p.module_mut(kit).unwrap().samples = rave_kit(sr);
    effects(
        p,
        kit,
        [
            (K::Compressor, "Kit Punch", &[(0, 0.35), (1, 3.0), (2, 0.003), (3, 0.1), (4, 1.3)]),
            (K::PlateReverb, "Kit Plate", &[(0, 0.5), (1, 0.01), (4, 0.12)]),
        ],
    );

    // The bass: detuned saws snapping through a filter, on the off-beats
    // or rolling between the kicks.
    let bass_params = [(0, 0.4), (2, 0.002), (3, 0.14), (4, 0.35), (5, 0.04), (6, 6.0), (7, 2.0)];
    let bass = add(p, K::Generator, "Off Bass", &bass_params);
    let m = p.module_mut(bass).unwrap();
    m.modulation.filter = true;
    m.modulation.cutoff = 320.0;
    m.modulation.resonance = 0.35;
    m.modulation.filter_env =
        VoiceEnvelope { on: true, points: vec![(0.0, 1.0), (0.12, 0.0)], sustain: None, curve: true, amount: 3.0 };
    let [_, bass_filter, bass_end] = effects(
        p,
        bass,
        [
            (K::WaveShaper, "Bass Bite", &[(0, 1.5), (9, 0.4), (10, 0.7), (11, 0.88), (12, 0.95)]),
            (K::FilterPro, "Bass Filter", &[(1, 20000.0), (4, 1.0)]),
            (K::Compressor, "Bass Squash", &[(0, 0.3), (1, 4.0), (2, 0.002), (3, 0.06), (4, 1.4)]),
        ],
    );

    // The growl: saws through a ladder that Wub opens and a mouth that Talk
    // moves on every note, folded over and squashed.
    let growl =
        add(p, K::Generator, "Growl", &[(0, 0.5), (2, 0.002), (3, 0.3), (4, 0.8), (5, 0.08), (6, 12.0), (7, 2.0)]);
    let [growl_filter, growl_mouth, _, _] = effects(
        p,
        growl,
        [
            (K::AnalogFilter, "Growl Filter", &[(1, 300.0), (2, 0.55), (3, 3.5)]),
            (K::VocalFilter, "Growl Mouth", &[(0, 3.0), (2, 1.2), (3, 4.0), (4, 2.5), (5, 0.7)]),
            (K::Distortion, "Growl Fold", &[(0, 3.0), (1, 0.7), (2, 0.6), (3, 2.0)]),
            (K::Compressor, "Growl Squash", &[(0, 0.25), (1, 6.0), (2, 0.002), (3, 0.08), (4, 1.3)]),
        ],
    );

    // The supersaw stack, wide.
    let stack_params = [(0, 0.3), (2, 0.006), (3, 0.6), (4, 0.75), (5, 0.25), (6, 20.0), (7, 4.0)];
    let stack = add(p, K::Generator, "Saw Stack", &stack_params);
    let [stack_filter, _, stack_end] = effects(
        p,
        stack,
        [
            (K::FilterPro, "Stack Filter", &[(1, 20000.0), (4, 1.0)]),
            (K::Chorus, "Stack Chorus", &[(0, 3.0), (1, 0.4), (2, 0.4), (6, 0.35)]),
            (K::StereoExpander, "Stack Width", &[(0, 1.5), (1, 200.0)]),
        ],
    );
    send(p, stack, room);

    // The lead: a MultiSynth playing a supersaw with a late vibrato and an
    // FM bell for its edge, into one chorus, an echo and a wide image.
    let lead = add(p, K::MultiSynth, "Prism Lead", &[]);
    let saws_params = [(0, 0.3), (2, 0.004), (3, 0.4), (4, 0.75), (5, 0.18), (6, 25.0), (7, 4.0)];
    let saws = add(p, K::Generator, "Lead Saws", &saws_params);
    p.module_mut(saws).unwrap().modulation.vibrato =
        VoiceLfo { on: true, shape: 0, rate: 5.5, depth: 0.15, delay: 0.3 };
    let bell_params = [(0, 0.18), (1, 2.0), (2, 2.5), (3, 0.15), (5, 0.001), (6, 0.5), (7, 0.3), (8, 0.2)];
    let bell = add(p, K::Fm, "Lead Bell", &bell_params);
    let lead_bus = add(p, K::Chorus, "Lead Chorus", &[(0, 2.0), (1, 0.6), (2, 0.35), (6, 0.3)]);
    for (from, to) in [(lead, saws), (lead, bell), (saws, lead_bus), (bell, lead_bus)] {
        p.connect(from, to);
    }
    let [_, lead_end] = effects(
        p,
        lead_bus,
        [
            (K::Delay, "Lead Echo", &[(0, 3.0), (1, 0.3), (2, 0.2), (3, 1.0)]),
            (K::StereoExpander, "Lead Width", &[(0, 1.3)]),
        ],
    );
    send(p, lead_bus, room);

    // The pluck: a saw snapping through a filter, with its phrases.
    let pluck = add(p, K::Generator, "Prism Pluck", &[(0, 0.55), (2, 0.001), (3, 0.16), (4, 0.0), (5, 0.08)]);
    let m = p.module_mut(pluck).unwrap();
    m.modulation.filter = true;
    m.modulation.cutoff = 900.0;
    m.modulation.resonance = 0.3;
    m.modulation.filter_env =
        VoiceEnvelope { on: true, points: vec![(0.0, 1.0), (0.1, 0.0)], sustain: None, curve: true, amount: 3.0 };
    m.phrases = pluck_phrases();
    let [pluck_end] = effects(p, pluck, [(K::Delay, "Pluck Echo", &[(0, 3.0), (1, 0.35), (2, 0.25), (3, 1.0)])]);
    send(p, pluck, room);

    // The piano: Last Light's felt piano, both layers, brightened.
    let piano = add(p, K::Sampler, "Bright Piano", &[(0, 0.6), (6, 0.4)]);
    let mut soft = rendered(felt_piano(sr, false));
    soft.velocities = [0, 84];
    let mut hard = rendered(felt_piano(sr, true));
    hard.velocities = [85, 127];
    p.module_mut(piano).unwrap().samples = vec![soft, hard];
    effects(
        p,
        piano,
        [
            (K::Eq, "Piano Shine", &[(0, 0.8), (2, 1.6), (5, 6000.0)]),
            (K::Exciter, "Piano Air", &[(0, 5000.0), (1, 2.5), (2, 0.2)]),
        ],
    );
    send(p, piano, room);

    // The pad: a shimmering SpectraVoice.
    let pad_params = [(0, 0.3), (1, 16.0), (2, 1.2), (3, 0.7), (5, 0.6), (6, 0.6), (7, 1.5), (8, 0.8), (9, 1.5)];
    let pad = add(p, K::SpectraVoice, "Starlight Pad", &pad_params);
    let [_, pad_end] = effects(
        p,
        pad,
        [
            (K::Chorus, "Pad Chorus", &[(0, 3.0), (1, 0.3), (2, 0.5), (6, 0.4)]),
            (K::FilterPro, "Pad Cut", &[(0, 1.0), (1, 200.0)]),
        ],
    );
    send(p, pad, room);

    // The hoover: a detuned pulse scooping up into each note from six
    // semitones under (its pitch envelope), chorused deep and gritty.
    let hoover_params = [(0, 0.12), (1, 1.0), (2, 0.002), (3, 0.3), (4, 0.7), (5, 0.2), (6, 35.0), (7, 4.0), (8, 0.3)];
    let hoover = add(p, K::Generator, "Hoover", &hoover_params);
    p.module_mut(hoover).unwrap().modulation.pitch =
        VoiceEnvelope { on: true, points: vec![(0.0, 0.25), (0.09, 0.5)], sustain: None, curve: true, amount: 12.0 };
    let [_, _, hoover_end] = effects(
        p,
        hoover,
        [
            (K::Chorus, "Hoover Chorus", &[(0, 4.0), (1, 0.5), (2, 0.8), (6, 0.5)]),
            (K::Distortion, "Hoover Grit", &[(0, 2.5), (1, 0.7), (2, 0.5)]),
            (K::FilterPro, "Hoover Top", &[(1, 7000.0)]),
        ],
    );
    send(p, hoover, room);

    // The voice: Static Heart's four sung syllables, one sample, a quarter
    // each; 9xx picks one, and the notes pitch them up high.
    let vox = add(p, K::Sampler, "Sky Vox", &[(0, 0.5), (6, 0.03)]);
    let mut slot = rendered(syllables(sr).0);
    slot.base_note = 64;
    p.module_mut(vox).unwrap().samples = vec![slot];
    effects(
        p,
        vox,
        [
            (K::FilterPro, "Vox Cut", &[(0, 1.0), (1, 250.0)]),
            (K::Echo, "Vox Echo", &[(0, 3.0 * 60.0 / (BPM * LPB as f32)), (1, 0.3), (2, 0.4), (3, 0.6), (4, 0.25)]),
        ],
    );
    send(p, vox, room);

    // The orchestra hit: a C major chord, played on other notes for other
    // chords, in a hall.
    let orch = add(p, K::Sampler, "Orchestra Hit", &[(0, 0.8), (6, 0.3)]);
    p.module_mut(orch).unwrap().samples = vec![rendered(orchestra_hit(sr))];
    effects(
        p,
        orch,
        [
            (K::Exciter, "Orch Bite", &[(0, 3000.0), (1, 2.0), (2, 0.2)]),
            (K::PlateReverb, "Orch Hall", &[(0, 0.75), (1, 0.02), (4, 0.3)]),
        ],
    );

    // The uplifter: noise swept up through a band, jetting.
    let uplifter = add(p, K::Generator, "Uplifter", &[(0, 0.4), (1, 4.0), (2, 2.0), (3, 0.0), (4, 1.0), (5, 0.3)]);
    let [sweep, _] = effects(
        p,
        uplifter,
        [
            (K::FilterPro, "Uplift Sweep", &[(0, 2.0), (1, 400.0), (2, 3.0), (4, 1.0)]),
            (K::Flanger, "Uplift Jet", &[(2, 0.8), (3, 0.25), (4, 0.6), (5, 0.5)]),
        ],
    );
    send(p, uplifter, room);

    // Modulators. Pump, following the kick: everything with a tone ducks
    // under it. Wub and Talk: each growl note opens its ladder and moves its
    // mouth, with an envelope of its own.
    let pump = add(p, K::Modulator, "Pump", &[(0, 1.0), (5, -0.45), (6, 0.002), (7, 0.17)]);
    p.connect(kick_weight, pump);
    for target in [stack_end, lead_end, pluck_end, pad_end, bass_end, hoover_end] {
        let fader = p.module(target).unwrap().kind.params().len();
        p.connect(pump, target);
        p.set_control_param(pump, target, fader);
    }
    for (name, target, param, amount, attack, release) in
        [("Wub", growl_filter, 1, 0.45, 0.04, 0.14), ("Talk", growl_mouth, 0, -0.6, 0.07, 0.2)]
    {
        let m = add(p, K::Modulator, name, &[(0, 4.0), (5, amount), (6, attack), (7, release)]);
        p.connect(growl, m);
        p.connect(m, target);
        p.set_control_param(m, target, param);
    }

    Ids {
        kick,
        kick_filter,
        boom,
        kit,
        bass,
        bass_filter,
        growl,
        stack,
        stack_filter,
        lead,
        pluck,
        piano,
        pad,
        hoover,
        vox,
        orch,
        uplifter,
        sweep,
        glitch,
    }
}

// ---------------------------------------------------------------- parts

/// A new pattern of `lines` with the song's columns.
fn pattern(lines: usize) -> Pattern {
    let mut pat = Pattern::new("", TRACKS, lines);
    for (t, cols) in COLUMNS {
        pat.set_columns(t, cols);
    }
    pat
}

/// `chords` `key` semitones up, for the last chorus.
fn up(chords: &[Chord], key: u8) -> Vec<Chord> {
    chords.iter().map(|c| Chord { bass: c.bass + key, stack: c.stack.map(|n| n + key), ..*c }).collect()
}

/// Four on the floor through `bars`: the kick on every beat, a snare and a
/// clap on two and four, an open hat on each off-beat with a closed one
/// before it, and now and then (Yxx) one after it that cuts it short.
fn four_floor(pat: &mut Pattern, ids: &Ids, bars: std::ops::Range<usize>, kick: u8) {
    for b in bars {
        for beat in 0..4 {
            let at = b * BAR + beat * LPB;
            pat.tracks[KICK][at] = n(kick, ids.kick, 0x78);
            *pat.cell_mut(KIT, 2, at + 1) = n(CLOSED, ids.kit, 0x30);
            *pat.cell_mut(KIT, 2, at + 2) = n(OPEN, ids.kit, 0x48);
            *pat.cell_mut(KIT, 2, at + 3) = fx(n(CLOSED, ids.kit, 0x28), FX_MAYBE, 0x50);
        }
        for at in [b * BAR + 4, b * BAR + 12] {
            pat.tracks[KIT][at] = n(SNARE, ids.kit, 0x68);
            *pat.cell_mut(KIT, 1, at) = n(CLAP, ids.kit, 0x70);
        }
    }
}

/// The bass on every off-beat, cut short (Cxx).
fn off_bass(pat: &mut Pattern, id: u8, chords: &[Chord], vol: u8) {
    for (b, ch) in chords.iter().enumerate() {
        for beat in 0..4 {
            let at = b * BAR + beat * LPB + 2;
            pat.tracks[BASS][at] = n(ch.bass, id, vol);
            pat.tracks[BASS][at + 1].fx = Some((0xC, 0x04));
        }
    }
}

/// The bass rolling on the three sixteenths after every kick, each cut
/// short (Cxx).
fn rolling_bass(pat: &mut Pattern, id: u8, chords: &[Chord], vol: u8) {
    for (b, ch) in chords.iter().enumerate() {
        for beat in 0..4 {
            for k in 1..LPB {
                let at = b * BAR + beat * LPB + k;
                pat.tracks[BASS][at] = fx(n(ch.bass, id, if k == 1 { vol } else { vol - 0x10 }), 0xC, 0x04);
            }
        }
    }
}

/// The supersaws hold each bar's chord.
fn stack(pat: &mut Pattern, id: u8, chords: &[Chord], vol: u8) {
    for (b, ch) in chords.iter().enumerate() {
        for (c, &note) in ch.stack.iter().enumerate() {
            *pat.cell_mut(STACK, c, b * BAR) = n(note, id, vol);
        }
    }
}

/// The supersaws in stabs, each bar as `gate` spells it, a sixteenth a
/// letter: x a stab cut short (Cxx), . a rest.
fn stabs(pat: &mut Pattern, id: u8, chords: &[Chord], gate: &str, vol: u8) {
    for (b, ch) in chords.iter().enumerate() {
        for (s, _) in gate.chars().enumerate().filter(|(_, c)| *c == 'x') {
            for (c, &note) in ch.stack.iter().enumerate() {
                *pat.cell_mut(STACK, c, b * BAR + s) = fx(n(note, id, vol), 0xC, 0x03);
            }
        }
    }
}

/// The pluck's arpeggio from each bar's root, a bar a chord from `line`:
/// Z01 on major chords, Z02 on minor ones.
fn arps(pat: &mut Pattern, id: u8, chords: &[Chord], line: usize, vol: u8) {
    for (b, ch) in chords.iter().enumerate() {
        pat.tracks[PLUCK][line + b * BAR] = fx(n(ch.bass, id, vol), FX_PHRASE, if ch.major { 1 } else { 2 });
    }
}

/// The lead plays `notes` `key` semitones up on column `col`: it glides
/// (3xx) into leaps of a fifth or more, and notes held a beat waver (4xy).
fn lead(pat: &mut Pattern, id: u8, notes: &[(usize, u8)], key: u8, col: usize, vol: u8) {
    let mut last: Option<u8> = None;
    for (k, &(l, note)) in notes.iter().enumerate() {
        let note = note + key;
        let cell = n(note, id, vol);
        *pat.cell_mut(LEAD, col, l) = match last {
            Some(p) if note.abs_diff(p) >= 7 => fx(cell, 0x3, 0x60),
            _ => cell,
        };
        let next = notes.get(k + 1).map_or(pat.lines, |nt| nt.0);
        if next - l >= LPB {
            for v in l + 2..next {
                pat.cell_mut(LEAD, col, v).fx = Some((0x4, 0x23));
            }
        }
        last = Some(note);
    }
}

/// The piano, a chord every `every` lines: its root in octaves in the left
/// hand, and the right hand running up and down it in sixteenths over two
/// octaves, softer between the beats.
fn piano(pat: &mut Pattern, id: u8, chords: &[Chord], every: usize, vol: u8) {
    for (k, ch) in chords.iter().enumerate() {
        let at = k * every;
        *pat.cell_mut(PIANO, 0, at) = n(ch.bass - 12, id, vol);
        *pat.cell_mut(PIANO, 1, at) = n(ch.bass, id, vol);
        let run: Vec<u8> = ch.stack.iter().chain(ch.stack.map(|n| n + 12).iter()).copied().collect();
        for l in 0..every {
            let i = l % 16;
            let note = run[if i < 8 { i } else { 15 - i }];
            *pat.cell_mut(PIANO, 2, at + l) = n(note, id, if l % LPB == 0 { vol } else { vol - 0x18 });
        }
    }
}

/// The pad holds `chords`, one every `every` lines.
fn pads(pat: &mut Pattern, id: u8, chords: &[Chord], every: usize, vol: u8) {
    for (k, ch) in chords.iter().enumerate() {
        for (c, &note) in ch.stack[1..].iter().enumerate() {
            *pat.cell_mut(PAD, c, k * every) = n(note, id, vol);
        }
    }
}

/// A chop of the voice on `note`: syllable `s` (ah, oh, ee or ay, a
/// quarter of the sample each, picked by 9xx), thrown by the panning
/// column, and cut short (Cxx) on the next line unless another starts
/// there.
fn chop(pat: &mut Pattern, id: u8, line: usize, note: u8, s: u8, pan: u8) {
    pat.tracks[VOX][line] = Cell { pan: Some(pan), ..fx(n(note, id, 0x70), 0x9, s * 0x40) };
    let next = &mut pat.tracks[VOX][line + 1];
    if next.note.is_none() {
        next.fx = Some((0xC, 0x02));
    }
}

/// A rhythm game's figure on the pluck from `line`: `notes` in
/// sixteenths, up a scale (stairs) or the same over and over (jacks).
fn figure(pat: &mut Pattern, id: u8, line: usize, notes: &[u8], vol: u8) {
    for (k, &note) in notes.iter().enumerate() {
        pat.tracks[PLUCK][line + k] = n(note, id, if k % 4 == 0 { vol } else { vol - 0x10 });
    }
}

/// The uplifter rising from `line` to `to`, its band sweeping up and its
/// fader coming up.
fn uplift(pat: &mut Pattern, ids: &Ids, line: usize, to: usize) {
    pat.tracks[FX][line] = n(60, ids.uplifter, 0x70);
    let (from, to) = (line as f32, to as f32);
    let f = ModuleKind::FilterPro;
    pat.automation.push(envelope(ids.sweep, f, 1, &[(from, 400.0), (to, 14000.0)], false, true));
    let g = ModuleKind::Generator;
    pat.automation.push(envelope(ids.uplifter, g, g.params().len(), &[(from, 0.2), (to, 1.0)], false, true));
}

fn intro(ids: &Ids) -> Pattern {
    let mut pat = pattern(PHRASE);
    // The piano alone through the hook's chords, the hook's first four bars
    // on top; the pad joins halfway.
    piano(&mut pat, ids.piano, &HOOK_CHORDS, BAR, 0x60);
    let first: Vec<(usize, u8)> = HOOK.iter().copied().filter(|h| h.0 < 4 * BAR).collect();
    tune(&mut pat, PIANO, 3, ids.piano, &first, 0x68);
    pads(&mut pat, ids.pad, &HOOK_CHORDS[4..], BAR, 0x48);
    // Then the kick, through a filter opening up, the pluck's arpeggios,
    // open hats on the off-beats, and the uplifter.
    for l in (4 * BAR..8 * BAR - LPB).step_by(LPB) {
        pat.tracks[KICK][l] = n(KICK_NOTE, ids.kick, 0x78);
    }
    let f = ModuleKind::FilterPro;
    let opening = [(64.0, 150.0), (112.0, 2500.0), (124.0, 20000.0)];
    pat.automation.push(envelope(ids.kick_filter, f, 1, &opening, false, true));
    arps(&mut pat, ids.pluck, &HOOK_CHORDS[4..], 4 * BAR, 0x40);
    for l in (6 * BAR + 2..8 * BAR).step_by(LPB) {
        *pat.cell_mut(KIT, 2, l) = n(OPEN, ids.kit, 0x40);
    }
    uplift(&mut pat, ids, 6 * BAR, PHRASE);
    // The voice calls out at the end of the first half.
    for (l, note, s) in [(60, 81, 0), (62, 86, 1)] {
        chop(&mut pat, ids.vox, l, note, s, 0x40);
    }
    // The snare rolls up its keys into the drop (Exx), and the reversed
    // crash sucks into it.
    for (l, key) in [(112, 0), (116, 0), (120, 1), (122, 1), (124, 2), (125, 2)] {
        pat.tracks[KIT][l] = n(SNARE + key, ids.kit, 0x50 + l as u8 - 112);
    }
    roll(&mut pat, KIT, 126, 2, n(SNARE + 3, ids.kit, 0x70), 0x03, 0x7F);
    *pat.cell_mut(KIT, 3, PHRASE - 2 * LPB) = n(REVERSE, ids.kit, 0x70);
    pat
}

/// The hook's drop, `key` semitones up; `layer` 1 doubles the lead and
/// brings in the voice, 2 doubles it throughout and adds orchestra hits.
fn drop(ids: &Ids, key: u8, layer: u8) -> Pattern {
    let mut pat = pattern(PHRASE);
    let_go(&mut pat, &[(PIANO, 4), (PAD, 3), (GROWL, 1)]);
    let chords = up(&HOOK_CHORDS, key);
    four_floor(&mut pat, ids, 0..8, KICK_NOTE + key);
    // A boom and a crash on the one, a crash halfway; the clap flams (Dxx)
    // at the end of every other bar; the last beat the kick rests and the
    // snare rolls.
    pat.tracks[FX][0] = n(26 + key, ids.boom, 0x70);
    for l in [0, 4 * BAR] {
        *pat.cell_mut(KIT, 3, l) = n(CRASH, ids.kit, 0x70);
    }
    for b in [1, 3, 5] {
        pat.cell_mut(KIT, 1, b * BAR + 12).fx = Some((0xD, 0x02));
    }
    pat.tracks[KICK][PHRASE - LPB] = Cell::default();
    roll(&mut pat, KIT, PHRASE - 2 * LPB, 2 * LPB, n(SNARE, ids.kit, 0x40), 0x03, 0x78);
    off_bass(&mut pat, ids.bass, &chords, 0x70);
    stack(&mut pat, ids.stack, &chords, 0x60);
    arps(&mut pat, ids.pluck, &chords, 0, 0x48);
    lead(&mut pat, ids.lead, HOOK, key, 0, 0x70);
    // Orchestra hits open each half, the sample's C major played on each
    // chord's root.
    for b in [0, 4] {
        pat.tracks[ORCH][b * BAR] = n(chords[b].bass + 12, ids.orch, 0x70);
    }
    // A hoover dives (2xx) under the lead's long note in the fourth bar.
    pat.tracks[HOOVER][60] = n(chords[3].bass + 12, ids.hoover, 0x60);
    for l in 61..64 {
        pat.tracks[HOOVER][l].fx = Some((0x2, 0x18));
    }
    pat.tracks[HOOVER][64] = off();
    if layer > 0 {
        // The lead doubled an octave down from halfway, or throughout; the
        // voice answers it at the end of each half.
        let from = if layer > 1 { 0 } else { 4 * BAR };
        let low: Vec<(usize, u8)> = HOOK.iter().filter(|h| h.0 >= from).map(|&(l, note)| (l, note - 12)).collect();
        lead(&mut pat, ids.lead, &low, key, 1, 0x58);
        for b in [3, 7] {
            let top = chords[b].stack.map(|n| n + 12);
            chop(&mut pat, ids.vox, b * BAR + 12, top[3], 0, 0x30);
            chop(&mut pat, ids.vox, b * BAR + 14, top[2], 1, 0x50);
        }
    }
    if layer > 1 {
        // The last bar hit in threes, three, three and two.
        for l in [112, 115, 118] {
            pat.tracks[ORCH][l] = n(chords[7].bass + 12, ids.orch, 0x68);
        }
    }
    pat
}

fn rolling(ids: &Ids) -> Pattern {
    let mut pat = pattern(PHRASE);
    let_go(&mut pat, &[(STACK, 4), (LEAD, 2)]);
    let chords = ROLL_CHORDS;
    // Psytrance: the kick on the beats and the bass rolling between them,
    // its filter opening; claps on two and four, the ride on the off-beats,
    // closed hats between, maybe.
    for b in 0..8 {
        for beat in 0..4 {
            let at = b * BAR + beat * LPB;
            pat.tracks[KICK][at] = n(KICK_NOTE, ids.kick, 0x78);
            *pat.cell_mut(KIT, 3, at + 2) = n(RIDE, ids.kit, 0x48);
            for k in [1, 3] {
                *pat.cell_mut(KIT, 2, at + k) = fx(n(CLOSED, ids.kit, 0x30), FX_MAYBE, 0xC0);
            }
        }
        for at in [b * BAR + 4, b * BAR + 12] {
            *pat.cell_mut(KIT, 1, at) = n(CLAP, ids.kit, 0x70);
        }
    }
    *pat.cell_mut(KIT, 3, 0) = n(CRASH, ids.kit, 0x70);
    rolling_bass(&mut pat, ids.bass, &chords, 0x70);
    let f = ModuleKind::FilterPro;
    pat.automation.push(envelope(ids.bass_filter, f, 1, &[(0.0, 400.0), (120.0, 6000.0)], false, true));
    // The supersaws gated into a rhythm.
    stabs(&mut pat, ids.stack, &chords, "x.xx.x.xx.x.xx.x", 0x50);
    // The voice chops a hook of its own, ah, oh, ee in threes, thrown about.
    for (b, ch) in chords.iter().enumerate().filter(|(b, _)| b % 4 != 3) {
        let top = ch.stack.map(|n| n + 12);
        for (l, note, s, pan) in [
            (0, top[3], 0, 0x40),
            (3, top[2], 1, 0x28),
            (6, top[3], 2, 0x58),
            (10, top[1], 0, 0x30),
            (12, top[2], 3, 0x50),
        ] {
            chop(&mut pat, ids.vox, b * BAR + l, note, s, pan);
        }
    }
    // The pluck in a rhythm game's figures: stairs up and down, a trill
    // (Z03), stairs down and up, and jacks spinning chords (0xy).
    figure(&mut pat, ids.pluck, BAR, &[67, 69, 71, 73, 74, 76, 78, 79, 79, 78, 76, 74, 73, 71, 69, 67], 0x60);
    pat.tracks[PLUCK][3 * BAR] = fx(n(76, ids.pluck, 0x60), FX_PHRASE, 3);
    pat.tracks[PLUCK][3 * BAR + 8] = off();
    figure(&mut pat, ids.pluck, 5 * BAR, &[83, 81, 79, 78, 76, 74, 73, 71, 71, 73, 74, 76, 78, 79, 81, 83], 0x60);
    figure(&mut pat, ids.pluck, 7 * BAR, &[69; 8], 0x60);
    figure(&mut pat, ids.pluck, 7 * BAR + 8, &[73, 73, 73, 73, 76, 76, 76, 76], 0x60);
    for l in 7 * BAR..PHRASE {
        let arp = if pat.tracks[PLUCK][l].note == Some(Note::On(73)) { 0x37 } else { 0x47 };
        pat.tracks[PLUCK][l].fx = Some((0x0, arp));
    }
    // Hoovers stab at each half's end, thrown left then right (8xx), and
    // dive (2xx).
    for (l, note, pan) in [(60, 57, 0x30), (124, 57, 0xD0)] {
        pat.tracks[HOOVER][l] = fx(n(note, ids.hoover, 0x68), 0x8, pan);
        for v in l + 1..l + 4 {
            pat.tracks[HOOVER][v].fx = Some((0x2, 0x18));
        }
    }
    pat.tracks[HOOVER][64] = off();
    pat
}

fn rise(ids: &Ids) -> Pattern {
    let mut pat = pattern(4 * BAR);
    let_go(&mut pat, &[(HOOVER, 1)]);
    // The snare comes faster each bar and climbs its keys: quarters,
    // eighths, sixteenths, then 32nds (Exx); the kick on the beats until the
    // last bar; the reversed crash into the drop.
    for (b, every) in [(0, 4), (1, 2), (2, 1)] {
        for l in (b * BAR..(b + 1) * BAR).step_by(every) {
            pat.tracks[KIT][l] = n(SNARE + b as u8, ids.kit, 0x40 + l as u8);
        }
    }
    roll(&mut pat, KIT, 3 * BAR, 3 * LPB, n(SNARE + 3, ids.kit, 0x70), 0x03, 0x7F);
    for l in (0..3 * BAR).step_by(LPB) {
        pat.tracks[KICK][l] = n(KICK_NOTE, ids.kick, 0x78);
    }
    *pat.cell_mut(KIT, 3, 4 * BAR - 2 * LPB) = n(REVERSE, ids.kit, 0x70);
    // The supersaws pulse in eighths, louder each bar, through a filter
    // opening up; the pluck's arpeggios; the uplifter.
    for (b, ch) in RISE_CHORDS.iter().enumerate() {
        for l in (b * BAR..(b + 1) * BAR).step_by(2).filter(|&l| l < 4 * BAR - LPB) {
            for (c, &note) in ch.stack.iter().enumerate() {
                *pat.cell_mut(STACK, c, l) = fx(n(note, ids.stack, 0x30 + l as u8), 0xC, 0x04);
            }
        }
    }
    let f = ModuleKind::FilterPro;
    pat.automation.push(envelope(ids.stack_filter, f, 1, &[(0.0, 300.0), (60.0, 12000.0)], false, true));
    arps(&mut pat, ids.pluck, &RISE_CHORDS, 0, 0x48);
    uplift(&mut pat, ids, 0, 4 * BAR);
    // The voice in eighths, then sixteenths, climbing.
    for (k, l) in (2 * BAR..4 * BAR - LPB).step_by(2).chain((3 * BAR + 1..4 * BAR - LPB).step_by(2)).enumerate() {
        chop(&mut pat, ids.vox, l, 76 + (k % 8) as u8, (k % 2) as u8 * 2, if k % 2 == 0 { 0x28 } else { 0x58 });
    }
    // The lead climbs the last bar and slides up (1xx) into the drop.
    let climb = [(48, 81), (52, 83), (56, 85), (58, 86)];
    tune(&mut pat, LEAD, 0, ids.lead, &climb, 0x68);
    for l in 59..4 * BAR {
        pat.tracks[LEAD][l].fx = Some((0x1, 0x10));
    }
    pat
}

fn boss(ids: &Ids) -> Pattern {
    let mut pat = pattern(PHRASE);
    let_go(&mut pat, &[(STACK, 4), (LEAD, 2)]);
    // Half time: the kick on the one (and before the three now and then),
    // the snare and clap on the three, hats in eighths, a crash on each
    // half.
    for b in 0..6 {
        let at = b * BAR;
        pat.tracks[KICK][at] = n(KICK_NOTE, ids.kick, 0x78);
        if b % 2 == 1 {
            pat.tracks[KICK][at + 6] = n(KICK_NOTE, ids.kick, 0x60);
        }
        pat.tracks[KIT][at + 8] = n(SNARE, ids.kit, 0x78);
        *pat.cell_mut(KIT, 1, at + 8) = n(CLAP, ids.kit, 0x70);
        for l in (at..at + BAR).step_by(2) {
            *pat.cell_mut(KIT, 2, l) = n(CLOSED, ids.kit, if l % LPB == 0 { 0x40 } else { 0x30 });
        }
    }
    for l in [0, 4 * BAR] {
        *pat.cell_mut(KIT, 3, l) = n(CRASH, ids.kit, 0x70);
    }
    // The growl's riff, three times, every note opening its ladder and its
    // mouth; it dives (2xx) at the end of each.
    for rep in 0..3 {
        for &(l, note, glide) in &GROWL_RIFF {
            let cell = n(note, ids.growl, 0x78);
            pat.tracks[GROWL][rep * 2 * BAR + l] = if glide { fx(cell, 0x3, 0x60) } else { cell };
        }
        pat.tracks[GROWL][rep * 2 * BAR + 31].fx = Some((0x2, 0x30));
    }
    // The voice squeaks "ee" at the end of every other bar, and a hoover
    // wails over the second half.
    for b in [1, 3, 5] {
        chop(&mut pat, ids.vox, b * BAR + 14, 86, 2, if b == 3 { 0x10 } else { 0x70 });
    }
    pat.tracks[HOOVER][4 * BAR] = n(59, ids.hoover, 0x60);
    for l in 4 * BAR + 24..6 * BAR {
        pat.tracks[HOOVER][l].fx = Some((0x2, 0x04));
    }
    pat.tracks[HOOVER][6 * BAR] = off();
    // A bar at 255 BPM (Fxx): the kick on every line, again in between
    // (Exx), each a semitone higher; the snare rolling; the growl diving.
    pat.cell_mut(FX, 1, 6 * BAR).fx = Some((0xF, 0xFF));
    for k in 0..BAR {
        let l = 6 * BAR + k;
        pat.tracks[KICK][l] = fx(n(KICK_NOTE + k as u8, ids.kick, 0x70), 0xE, 0x03);
    }
    roll(&mut pat, KIT, 6 * BAR + 8, 8, n(SNARE + 1, ids.kit, 0x50), 0x02, 0x7F);
    pat.tracks[GROWL][6 * BAR] = n(47, ids.growl, 0x78);
    for l in 6 * BAR + 1..7 * BAR {
        pat.tracks[GROWL][l].fx = Some((0x2, 0x04));
    }
    // Back to 185, and everything lets go while Glitch, on the master,
    // holds what it heard, shorter and shorter, then lets go to silence.
    pat.cell_mut(FX, 1, 7 * BAR).fx = Some((0xF, BPM as u8));
    pat.tracks[GROWL][7 * BAR] = off();
    let lengths = [(112.0, 3.0), (115.0, 4.0), (118.0, 6.0), (120.0, 8.0), (122.0, 9.0)];
    hold(&mut pat, ids.glitch, &[(112.0, 124.0)], &lengths);
    pat
}

fn calm(ids: &Ids) -> Pattern {
    let mut pat = pattern(PHRASE);
    // The piano alone again, two bars a chord, with the hook slowed down on
    // top, and the pad swinging slowly across (Nxy).
    piano(&mut pat, ids.piano, &CALM_CHORDS, 2 * BAR, 0x58);
    let slow: Vec<(usize, u8)> = HOOK.iter().filter(|h| h.0 < 4 * BAR).map(|&(l, note)| (2 * l, note)).collect();
    tune(&mut pat, PIANO, 3, ids.piano, &slow, 0x68);
    pads(&mut pat, ids.pad, &CALM_CHORDS, 2 * BAR, 0x50);
    for k in 0..4 {
        pat.cell_mut(PAD, 0, k * 2 * BAR).fx = Some((FX_AUTOPAN, 0x16));
    }
    // In the last two bars the kick comes back through its filter, the
    // supersaws swell in, the uplifter rises, and the reversed crash.
    for l in (6 * BAR..PHRASE).step_by(LPB) {
        pat.tracks[KICK][l] = n(KICK_NOTE, ids.kick, 0x70);
    }
    let f = ModuleKind::FilterPro;
    pat.automation.push(envelope(ids.kick_filter, f, 1, &[(96.0, 150.0), (128.0, 2500.0)], false, true));
    pat.automation.push(envelope(ids.stack_filter, f, 1, &[(96.0, 300.0), (128.0, 9000.0)], false, true));
    for (c, &note) in A.stack.iter().enumerate() {
        *pat.cell_mut(STACK, c, 6 * BAR) = n(note, ids.stack, 0x50);
    }
    uplift(&mut pat, ids, 6 * BAR, PHRASE);
    *pat.cell_mut(KIT, 3, PHRASE - 2 * LPB) = n(REVERSE, ids.kit, 0x70);
    pat
}

fn lift(ids: &Ids) -> Pattern {
    let mut pat = pattern(4 * BAR);
    let_go(&mut pat, &[(PIANO, 4), (PAD, 3)]);
    // The kick rolls in: quarters, eighths, sixteenths, then 32nds (Exx),
    // under a snare climbing its keys; then a beat of nothing but the
    // reversed crash.
    for (b, every) in [(0, 4), (1, 2), (2, 1)] {
        for l in (b * BAR..(b + 1) * BAR).step_by(every) {
            pat.tracks[KICK][l] = n(KICK_NOTE, ids.kick, 0x70 + b as u8 * 4);
        }
    }
    roll(&mut pat, KICK, 3 * BAR, 3 * LPB, n(KICK_NOTE, ids.kick, 0x70), 0x03, 0x7F);
    for (k, l) in (2 * BAR..4 * BAR - LPB).enumerate() {
        pat.tracks[KIT][l] = fx(n(SNARE + (k / 7) as u8, ids.kit, 0x40 + 2 * k as u8), 0xE, 0x03);
    }
    *pat.cell_mut(KIT, 3, 4 * BAR - 2 * LPB) = n(REVERSE, ids.kit, 0x70);
    // The supersaws hold each chord, trembling faster (7xy); an orchestra
    // hit on each; the uplifter.
    for (b, ch) in LIFT_CHORDS.iter().enumerate() {
        let at = b * BAR;
        for (c, &note) in ch.stack.iter().enumerate() {
            *pat.cell_mut(STACK, c, at) = n(note, ids.stack, 0x60);
            for l in at..(at + BAR).min(4 * BAR - LPB) {
                pat.cell_mut(STACK, c, l).fx = Some((0x7, 0x28 + 0x10 * b as u8));
            }
        }
        pat.tracks[ORCH][at] = n(ch.bass + 12, ids.orch, 0x70);
    }
    let_go_at(&mut pat, 4 * BAR - LPB, &[(STACK, 4)]);
    uplift(&mut pat, ids, 0, 4 * BAR);
    // The lead climbs the new key's scale to its B and slides up (1xx).
    let climb = [(32, 83), (36, 85), (40, 87), (44, 88), (48, 90), (52, 92), (56, 93), (58, 95)];
    tune(&mut pat, LEAD, 0, ids.lead, &climb, 0x70);
    for l in 59..4 * BAR {
        pat.tracks[LEAD][l].fx = Some((0x1, 0x10));
    }
    pat
}

/// Lets go of the notes on the columns of `tracks` (as `let_go` takes
/// them) at `line`.
fn let_go_at(pat: &mut Pattern, line: usize, tracks: &[(usize, usize)]) {
    for &(t, cols) in tracks {
        for c in 0..cols {
            *pat.cell_mut(t, c, line) = off();
        }
    }
}

fn outro(ids: &Ids) -> Pattern {
    let mut pat = pattern(2 * BAR);
    let e = up(&[D], 2)[0];
    // One last hit on E, everything at once...
    pat.tracks[KICK][0] = n(KICK_NOTE + 2, ids.kick, 0x78);
    pat.tracks[FX][0] = n(28, ids.boom, 0x78);
    *pat.cell_mut(KIT, 3, 0) = n(CRASH, ids.kit, 0x78);
    pat.tracks[ORCH][0] = n(52, ids.orch, 0x78);
    pat.tracks[BASS][0] = n(40, ids.bass, 0x78);
    for (c, &note) in e.stack.iter().enumerate() {
        *pat.cell_mut(STACK, c, 0) = n(note, ids.stack, 0x70);
    }
    *pat.cell_mut(LEAD, 0, 0) = n(88, ids.lead, 0x78);
    *pat.cell_mut(LEAD, 1, 0) = n(76, ids.lead, 0x60);
    // ...that Glitch stutters away to nothing...
    let lengths = [(2.0, 4.0), (5.0, 6.0), (8.0, 8.0), (10.0, 9.0)];
    hold(&mut pat, ids.glitch, &[(2.0, 12.0)], &lengths);
    let_go_at(&mut pat, 12, &[(BASS, 1), (STACK, 4), (LEAD, 2)]);
    // ...and the piano runs up E major alone, over the pad fading (Axy),
    // and F00 ends the song.
    let run = [64, 68, 71, 76, 80, 83, 88, 92];
    for (k, &note) in run.iter().enumerate() {
        *pat.cell_mut(PIANO, 2, 14 + k) = n(note, ids.piano, 0x50 + 4 * k as u8);
    }
    *pat.cell_mut(PIANO, 3, 22) = n(95, ids.piano, 0x70);
    *pat.cell_mut(PIANO, 0, 22) = n(40, ids.piano, 0x60);
    for (c, &note) in e.stack[1..].iter().enumerate() {
        *pat.cell_mut(PAD, c, 12) = n(note, ids.pad, 0x50);
        for l in 24..2 * BAR {
            pat.cell_mut(PAD, c, l).fx = Some((0xA, 0x02));
        }
    }
    pat.tracks[FX][2 * BAR - 1].fx = Some((0xF, 0x00));
    pat
}

impl Project {
    /// "Prism Overdrive", the fourth demo song.
    pub fn prism_overdrive() -> Self {
        let mut p = Self::empty();
        p.title = "Prism Overdrive".into();
        p.artist = "noise".into();
        p.comments = COMMENTS.into();
        p.bpm = BPM;
        p.modules[0].params[0] = 0.8;
        let names =
            ["Kick", "Kit", "Bass", "Growl", "Saws", "Lead", "Pluck", "Piano", "Pad", "Hoover", "Vox", "Orch", "FX"];
        let colors = [
            [255, 80, 120],
            [255, 150, 170],
            [140, 90, 255],
            [120, 40, 200],
            [70, 160, 255],
            [80, 240, 255],
            [120, 255, 200],
            [255, 245, 220],
            [170, 200, 255],
            [255, 190, 60],
            [255, 130, 230],
            [255, 220, 120],
            [200, 200, 200],
        ];
        for (t, (name, color)) in names.iter().zip(colors).enumerate() {
            p.tracks[t].name = name.to_string();
            p.tracks[t].color = Some(color);
        }
        p.tracks[VOX].show_pan = true;
        p.tracks[KIT].show_delay = true;
        let ids = instruments(&mut p);
        let mut patterns = vec![
            intro(&ids),
            drop(&ids, 0, 0),
            rolling(&ids),
            rise(&ids),
            drop(&ids, 0, 1),
            boss(&ids),
            calm(&ids),
            lift(&ids),
            drop(&ids, 2, 2),
            outro(&ids),
        ];
        let names = ["Intro", "Drop", "Roll", "Rise", "Drop 2", "Boss", "Calm", "Lift", "Final", "Outro"];
        for (pat, name) in patterns.iter_mut().zip(names) {
            pat.name = name.into();
        }
        p.patterns = patterns;
        for (t, cols) in COLUMNS {
            p.set_columns(t, cols);
        }
        // The matrix holds back the hoover and the voice the first time
        // round the last chorus.
        let mut first_final = Slot::new(8);
        for t in [HOOVER, VOX] {
            first_final.toggle_mute(t);
        }
        p.order = [0, 1, 2, 3, 4, 5, 6, 7]
            .map(Slot::new)
            .into_iter()
            .chain([first_final, Slot::new(8), Slot::new(9)])
            .collect();
        p.sections = [
            ("Intro", 0),
            ("Drop", 1),
            ("Roll", 2),
            ("Drop 2", 3),
            ("Boss", 5),
            ("Calm", 6),
            ("Final", 7),
            ("Outro", 10),
        ]
        .into_iter()
        .map(|(name, start)| Section { start, name: name.into() })
        .collect();
        p
    }
}

const COMMENTS: &str = "Prism Overdrive — the fourth demo song: rhythm-game hardcore in D major at 185 BPM, its \
    last chorus a whole tone up.\n\n\
    A piano plays the hook's chords alone; the kick comes in through a filter and the drop hits: a hardcore kick \
    with the bass on the off-beats, supersaws pumping under it and a lead singing the hook. A psytrance part rolls \
    the bass between the kicks; the hook comes back bigger; the boss part goes half time with a growling bass, \
    speeds up to 255 BPM for a bar and glitches out; the piano breaks it down; a kick roll lifts it to E, and it \
    ends on one big hit stuttering away.\n\n\
    Where to look:\n\
    • Rave Kit: a Sampler with a sample for each sound on keys of its own (its Keyzones page). The snare's keys \
    above it pitch it up for fills; the closed and open hats share a mute group, so a closed one cuts an open one. \
    The reversed crash is beat-synced to two beats, to land on the one.\n\
    • Pump: a Modulator following the kick, pulling the faders of the saws, the lead, the pluck, the pad, the \
    bass and the hoover down under every hit.\n\
    • Prism Lead: a MultiSynth playing a supersaw and an FM bell through one chorus; it glides with 3xx and wavers \
    with 4xy. The last chorus doubles it an octave down in its second column.\n\
    • Prism Pluck: its phrases arpeggiate major (Z01) and minor (Z02) chords and trill in 32nds (Z03); in Roll it \
    plays a rhythm game's stairs and jacks, the jacks spinning chords with 0xy.\n\
    • Sky Vox: Static Heart's four syllables in one sample; 9xx picks one (00, 40, 80 or C0) and Cxx cuts it.\n\
    • Growl: Wub and Talk, Modulators running an envelope on each note, open its ladder and move its Vocal \
    Filter, so every note growls.\n\
    • Hoover: a pitch envelope scoops every note up from six semitones under.\n\
    • Orchestra Hit: one C major chord, played on other notes for other chords.\n\
    • Boss: Fxx jumps to 255 BPM for a bar of kick rolls (Exx on every line, a semitone higher each), and \
    Glitch, a Repeater on the master, stutters it to silence. The last chorus is the drop a whole tone up, and \
    the matrix holds back its hoover and voice the first time.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prism_overdrive_uses_its_kit_and_changes_key_and_tempo() {
        let p = Project::prism_overdrive();
        // The kit: a sample on keys of its own for each sound, the hats in
        // one mute group.
        let kit = p.modules.iter().find(|m| m.name == "Rave Kit").unwrap();
        assert_eq!(kit.samples.len(), 7);
        assert!(kit.samples.iter().all(|s| s.data.is_some() && s.keys[0] == s.base_note && s.oneshot));
        assert_eq!(kit.samples.iter().filter(|s| s.mute_group == 1).count(), 2);
        let mut commands: Vec<(u8, u8)> = Vec::new();
        for pat in &p.patterns {
            for lane in 0..pat.num_lanes() {
                commands.extend(pat.lane(lane)[..pat.lines].iter().filter_map(|c| c.fx));
            }
        }
        let phrase_cells = p.modules.iter().flat_map(|m| m.phrases.iter().flat_map(|ph| ph.cells.iter()));
        commands.extend(phrase_cells.filter_map(|c| c.fx));
        for cmd in [0x0, 0x1, 0x2, 0x3, 0x4, 0x7, 0x8, 0x9, 0xA, 0xC, 0xD, 0xE, 0xF, FX_AUTOPAN, FX_MAYBE, FX_PHRASE] {
            assert!(commands.iter().any(|c| c.0 == cmd), "effect {:?}", char::from_digit(cmd as u32, 36));
        }
        // A bar at 255 BPM, and the last chorus a whole tone up, its kick on E.
        assert!(commands.contains(&(0xF, 0xFF)) && commands.contains(&(0xF, BPM as u8)));
        let last = p.patterns.iter().find(|pat| pat.name == "Final").unwrap();
        assert_eq!(last.tracks[KICK][0].note, Some(Note::On(KICK_NOTE + 2)));
        // Everything that makes a sound reaches the output.
        let links: Vec<(u8, u8)> = p.signal_graph().1.iter().map(|(a, b)| (a.0, b.0)).collect();
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
        // It ends on its F00.
        let end = &p.patterns[p.order.last().unwrap().pattern];
        assert_eq!(end.tracks[FX][end.lines - 1].fx, Some((0xF, 0)));
    }

    #[test]
    fn prism_overdrive_plays_loud_enough_without_clipping_and_ends() {
        let frames = crate::audio::render(std::sync::Arc::new(Project::prism_overdrive()), 8000, 1.0, false);
        let peak = frames.iter().fold(0f32, |a, f| a.max(f[0].abs()).max(f[1].abs()));
        assert!(peak > 0.5 && peak <= 1.0, "{peak}");
        let secs = frames.len() as f32 / 8000.0;
        assert!((70.0..130.0).contains(&secs), "{secs} s");
    }
}
