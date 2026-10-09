//! "Static Heart", the second demo song: a minute and a half of glitchy,
//! bitcrushed hyperpop in F# minor at 160 BPM: 8-bit noise, chopped-up
//! pop and walls of fuzz.
//!
//! It boots up from bleeps and a crushed arp, drops a breakbeat under a
//! square-wave lead, builds into a wall of fuzzed supersaws with a
//! pitched-up voice gliding over it, breaks down to vocal chops and
//! static, comes back harder with a gated wall and a counter-melody, and
//! dies in a tape stop.
//!
//! The vocal chops and the breakbeat are sampled here, so its Samplers
//! have slices without audio files; they are written next to the song
//! when it is saved.

use crate::demo::{Rng, envelope, fx, n, normalize, off, rendered};
use crate::dsp::Frame;
use crate::project::*;
use crate::sample::Sample;

/// A chord: the 808's root, three notes for the wall, and the chip arp's
/// note and the arpeggio (0xy) that makes the chord of it.
#[derive(Clone, Copy)]
struct Chord {
    root: u8,
    tones: [u8; 3],
    arp: u8,
    arpeggio: u8,
}

const fn minor(root: u8, tones: [u8; 3], arp: u8) -> Chord {
    Chord { root, tones, arp, arpeggio: 0x37 }
}

const fn major(root: u8, tones: [u8; 3], arp: u8) -> Chord {
    Chord { root, tones, arp, arpeggio: 0x47 }
}

// MIDI numbers: 48 is the tracker's C-4.
const FSM: Chord = minor(30, [54, 57, 61], 66);
const D: Chord = major(26, [54, 57, 62], 62);
const A: Chord = major(33, [52, 57, 61], 69);
const E: Chord = major(28, [52, 56, 59], 64);
const BM: Chord = minor(35, [54, 59, 62], 71);
const CS: Chord = major(37, [53, 56, 61], 61);

/// The song's four chords, a bar each, and the break's darker turn.
const LOOP: [Chord; 4] = [FSM, D, A, E];
const BREAK: [Chord; 4] = [BM, D, A, CS];

/// The verse's square lead, staccato in sixteenths: (line, note).
const VERSE_LEAD: [(usize, u8); 32] = [
    (0, 73),
    (2, 69),
    (4, 66),
    (6, 69),
    (8, 73),
    (10, 76),
    (12, 73),
    (14, 69),
    (16, 74),
    (18, 69),
    (20, 66),
    (22, 69),
    (24, 74),
    (26, 78),
    (28, 74),
    (30, 69),
    (32, 73),
    (34, 69),
    (36, 64),
    (38, 69),
    (40, 73),
    (42, 76),
    (44, 73),
    (46, 69),
    (48, 71),
    (50, 68),
    (52, 64),
    (54, 68),
    (56, 71),
    (58, 76),
    (60, 73),
    (62, 71),
];

/// The hook the voice sings in the drops: (line, note, lines).
const HOOK: [(usize, u8, usize); 20] = [
    (0, 73, 3),
    (3, 73, 3),
    (6, 76, 4),
    (10, 73, 2),
    (12, 71, 4),
    (16, 69, 3),
    (19, 69, 3),
    (22, 66, 2),
    (24, 69, 4),
    (28, 71, 4),
    (32, 73, 3),
    (35, 73, 3),
    (38, 76, 4),
    (42, 78, 2),
    (44, 76, 4),
    (48, 68, 4),
    (52, 71, 4),
    (56, 68, 2),
    (58, 64, 2),
    (60, 66, 4),
];

/// The second verse's low line: (line, note, lines).
const LOW_LINE: [(usize, u8, usize); 12] = [
    (0, 61, 6),
    (6, 64, 2),
    (8, 66, 8),
    (16, 66, 4),
    (20, 64, 4),
    (24, 62, 8),
    (32, 61, 6),
    (38, 64, 2),
    (40, 69, 8),
    (48, 68, 6),
    (54, 66, 2),
    (56, 64, 8),
];

/// The note a third below `n` in F# minor, for the last drop's harmony.
fn third_below(n: u8) -> u8 {
    const SCALE: [u8; 7] = [6, 8, 9, 11, 1, 2, 4];
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

// ---------------------------------------------------------------- samples

/// Four sung syllables, "ah", "oh", "ee" and "ay", on E (MIDI 64): a
/// buzzing source whose harmonics are shaped by each vowel's formants,
/// with a little vibrato and breath. Returns the sample and where each
/// syllable starts.
pub(crate) fn syllables(sr: f32) -> (Sample, Vec<usize>) {
    const VOWELS: [[(f32, f32, f32); 3]; 4] = [
        [(800.0, 80.0, 1.0), (1150.0, 90.0, 0.5), (2900.0, 120.0, 0.25)],
        [(450.0, 70.0, 1.0), (800.0, 80.0, 0.45), (2830.0, 120.0, 0.2)],
        [(270.0, 60.0, 1.0), (2300.0, 100.0, 0.4), (3000.0, 120.0, 0.3)],
        [(530.0, 70.0, 1.0), (1840.0, 90.0, 0.5), (2480.0, 120.0, 0.3)],
    ];
    let f0 = 440.0 * 2f32.powf((64.0 - 69.0) / 12.0);
    let syllable = (sr * 0.26) as usize;
    let mut frames = vec![[0.0f32; 2]; syllable * VOWELS.len()];
    let mut starts = Vec::new();
    let mut rng = Rng(0x1234_5678);
    for (v, formants) in VOWELS.iter().enumerate() {
        let at = v * syllable;
        starts.push(at);
        let weight = |f: f32| formants.iter().map(|&(c, b, g)| g / (1.0 + ((f - c) / b).powi(2))).sum::<f32>();
        let mut phases = [0.0f32; 24];
        let mut breath = 0.0f32;
        for k in 0..syllable - (sr * 0.03) as usize {
            let t = k as f32 / sr;
            let wobble = 1.0 + 0.004 * (std::f32::consts::TAU * 5.5 * t).sin();
            let env = (t / 0.015).min(1.0) * ((0.23 - t) / 0.05).clamp(0.0, 1.0);
            let mut x = 0.0;
            for (h, ph) in phases.iter_mut().enumerate() {
                let f = f0 * (h + 1) as f32 * wobble;
                if f > sr * 0.45 {
                    break;
                }
                *ph = (*ph + f / sr).fract();
                x += (std::f32::consts::TAU * *ph).sin() * weight(f) / (h + 1) as f32;
            }
            breath += 0.3 * (rng.next() - breath);
            let y = (x + breath * 0.04 * weight(2500.0)) * env;
            frames[at + k] = [y * 0.95, y];
        }
    }
    normalize(&mut frames, 0.8);
    (Sample { name: "Vox (ah oh ee ay)".into(), sample_rate: sr, channels: 2, frames }, starts)
}

/// A bar of breakbeat at 160 BPM, kick, snare, ghost notes and hats, a
/// little saturated. Returns it and its length of a sixteenth.
fn breakbeat(sr: f32) -> (Sample, usize) {
    let step = (sr * 60.0 / 160.0 / 4.0) as usize;
    let mut frames = vec![[0.0f32; 2]; step * 16];
    let rng = Rng(0xBEEF_CAFE);
    let add = |frames: &mut Vec<Frame>, at: usize, f: &mut dyn FnMut(f32) -> f32, len: f32| {
        for k in 0..(sr * len) as usize {
            if let Some(fr) = frames.get_mut(at * step + k) {
                let v = f(k as f32 / sr);
                fr[0] += v;
                fr[1] += v;
            }
        }
    };
    for (at, vel) in [(0, 1.0), (7, 0.8), (9, 0.9)] {
        let mut phase = 0.0f32;
        add(
            &mut frames,
            at,
            &mut |t| {
                phase += (50.0 + 110.0 * (-t / 0.04).exp()) / sr;
                (std::f32::consts::TAU * phase).sin() * (-t / 0.22).exp() * vel
            },
            0.6,
        );
    }
    for (at, vel) in [(4, 0.9), (12, 1.0), (10, 0.3), (15, 0.4)] {
        let mut lp = 0.0f32;
        let mut noise = Rng(0x5EED + at as u32);
        add(
            &mut frames,
            at,
            &mut |t| {
                let w = noise.next();
                lp += 0.5 * (w - lp);
                let tone = (std::f32::consts::TAU * 190.0 * t).sin() * (-t / 0.05).exp();
                (0.5 * tone + (w - lp) * (-t / 0.11).exp()) * vel
            },
            0.4,
        );
    }
    for at in 0..16 {
        let vel = if at == 14 {
            0.35
        } else if at % 2 == 0 {
            0.22
        } else {
            0.13
        };
        let decay = if at == 14 { 0.12 } else { 0.025 };
        let mut prev = 0.0f32;
        let mut seed = Rng(rng.0 ^ (at as u32 * 7919));
        add(
            &mut frames,
            at,
            &mut |t| {
                let w = seed.next();
                let hp = w - prev;
                prev = w;
                hp * (-t / decay).exp() * vel
            },
            0.3,
        );
    }
    for f in &mut frames {
        *f = [(f[0] * 1.6).tanh(), (f[1] * 1.6).tanh()];
    }
    normalize(&mut frames, 0.85);
    (Sample { name: "Break (160)".into(), sample_rate: sr, channels: 2, frames }, step)
}

// ---------------------------------------------------------------- tracks

const DRUMS: usize = 0;
const HATS: usize = 1;
const BASS: usize = 2;
const WALL: usize = 3;
const ARP: usize = 4;
const VOX: usize = 5;
const LEAD: usize = 6;
const HAZE: usize = 7;
const CHOPS: usize = 8;
const FX: usize = 9;

/// The instruments and effects patterns refer to.
struct Ids {
    drums: u8,
    impact: u8,
    bass: u8,
    wall: u8,
    wall_filter: u8,
    chop: u8,
    arp: u8,
    crush: u8,
    vox: u8,
    mouth: u8,
    lead: u8,
    pluck: u8,
    haze: u8,
    haze_filter: u8,
    haze_ring: u8,
    chops: u8,
    break_loop: u8,
    riser: u8,
    sweep: u8,
    blip: u8,
    glitch: u8,
    space: u8,
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
    let sr = 44100.0;

    // A big shared space, its boom and fizz taken out.
    let space = add(p, K::Reverb);
    set(p, space, "Space", &[(0, 0.93), (1, 0.55), (2, 0.3)]);
    p.connect(space, OUTPUT_ID);
    let space_tone = p.chain_insert(space, 0, K::Eq10).unwrap();
    set(p, space_tone, "Space Tone", &[(0, 0.3), (1, 0.45), (2, 0.7), (8, 0.8), (9, 0.5)]);

    // The master: glue, a smile of an EQ, a little sheen and a limiter.
    let glue = p.chain_insert(OUTPUT_ID, 0, K::Compressor).unwrap();
    set(p, glue, "Glue", &[(0, 0.4), (1, 2.5), (2, 0.01), (3, 0.15), (4, 1.2)]);
    let smile = p.chain_insert(OUTPUT_ID, 1, K::Eq5).unwrap();
    set(
        p,
        smile,
        "Smile",
        &[(0, 60.0), (1, 1.0), (6, 700.0), (7, 0.9), (9, 2500.0), (10, 1.5), (12, 11000.0), (13, 1.3)],
    );
    let sheen = p.chain_insert(OUTPUT_ID, 2, K::Exciter).unwrap();
    set(p, sheen, "Sheen", &[(0, 6000.0), (1, 2.5), (2, 0.12)]);
    let loud = p.chain_insert(OUTPUT_ID, 3, K::Maximizer).unwrap();
    set(p, loud, "Loud", &[(0, 1.5), (1, 0.95), (2, 0.08)]);

    // Drums: crunched and crushed a little, squashed, and a Repeater to
    // glitch them.
    let drums = add(p, K::Drums);
    set(p, drums, "Crush Kit", &[(0, 0.7), (1, 60.0), (2, 0.25), (3, 0.75), (4, 0.04)]);
    let crunch = p.chain_insert(drums, 0, K::Distortion).unwrap();
    set(p, crunch, "Crunch", &[(0, 3.0), (1, 0.8), (2, 0.35), (3, 1.0), (4, 12.0), (5, 2.0)]);
    let smack = p.chain_insert(drums, 1, K::Compressor).unwrap();
    set(p, smack, "Smack", &[(0, 0.3), (1, 4.0), (2, 0.003), (3, 0.08), (4, 1.5)]);
    let glitch = p.chain_insert(drums, 2, K::Repeater).unwrap();
    set(p, glitch, "Glitch", &[]);
    // The hats swing around the room, as their own track's effect.
    let swing = p.chain_insert(Owner::Track(HATS), 0, K::Lfo).unwrap();
    set(p, swing, "Hat Swing", &[(0, 1.0), (1, 1.0), (2, 0.7), (4, 1.0), (5, 8.0)]);
    let hat_air = p.chain_insert(Owner::Track(HATS), 1, K::FilterPro).unwrap();
    set(p, hat_air, "Hat Air", &[(0, 1.0), (1, 3000.0), (4, 1.0)]);

    // A hit to land the drops on: a Kicker dropping far, in a plate.
    let impact = add(p, K::Kicker);
    set(p, impact, "Impact", &[(0, 0.45), (1, 1.0), (2, 3.0), (3, 0.25), (5, 2.2), (6, 0.8)]);
    let boom = p.chain_insert(impact, 0, K::PlateReverb).unwrap();
    set(p, boom, "Boom Plate", &[(0, 0.85), (1, 0.01), (2, 0.5), (4, 0.4)]);
    p.connect(impact, OUTPUT_ID);

    // The 808: a long Kicker, rounded off by a WaveShaper, its offset
    // taken out and its top cut.
    let bass = add(p, K::Kicker);
    set(p, bass, "808", &[(0, 0.14), (1, 0.0), (2, 0.6), (3, 0.04), (4, 0.002), (5, 1.0), (6, 0.5)]);
    let shape = p.chain_insert(bass, 0, K::WaveShaper).unwrap();
    set(p, shape, "808 Shape", &[(0, 1.6), (9, 0.4), (10, 0.68), (11, 0.86), (12, 0.95)]);
    let centre = p.chain_insert(bass, 1, K::DcBlocker).unwrap();
    set(p, centre, "808 Centre", &[]);
    let low = p.chain_insert(bass, 2, K::FilterPro).unwrap();
    set(p, low, "808 Low", &[(0, 0.0), (1, 2500.0), (4, 1.0)]);

    // The wall: a MultiSynth playing a detuned supersaw and a narrow
    // square, both into one fuzz, a guitar cabinet, a chorus and a wide
    // stereo image, as a shoegaze guitar.
    let wall = add(p, K::MultiSynth);
    set(p, wall, "Wall", &[(3, 6.0), (5, 0.1)]);
    let saw = add(p, K::Generator);
    set(p, saw, "Wall Saw", &[(0, 0.2), (1, 0.0), (2, 0.01), (3, 0.4), (4, 0.85), (5, 0.35), (6, 28.0), (7, 4.0)]);
    let square = add(p, K::Generator);
    let square_params = [(0, 0.05), (1, 1.0), (2, 0.01), (3, 0.3), (4, 0.8), (5, 0.3), (6, 12.0), (7, 2.0), (8, 0.3)];
    set(p, square, "Wall Square", &square_params);
    let fuzz = add(p, K::Distortion);
    set(p, fuzz, "Fuzz", &[(0, 14.0), (1, 0.55), (2, 1.0), (3, 0.0)]);
    p.connect(wall, saw);
    p.connect(wall, square);
    p.connect(saw, fuzz);
    p.connect(square, fuzz);
    let wall_filter = p.chain_insert(fuzz, 0, K::Filter).unwrap();
    set(p, wall_filter, "Wall Filter", &[(0, 0.0), (1, 18000.0), (2, 0.25)]);
    let cab = p.chain_insert(fuzz, 1, K::Cabinet).unwrap();
    set(p, cab, "4x12", &[(0, 1.0), (1, 2.5), (2, 0.8)]);
    let chorus = p.chain_insert(fuzz, 2, K::Chorus).unwrap();
    set(p, chorus, "Wall Chorus", &[(0, 3.0), (1, 0.6), (2, 0.6), (4, 0.4), (6, 0.5)]);
    let wide = p.chain_insert(fuzz, 3, K::StereoExpander).unwrap();
    set(p, wide, "Wall Wide", &[(0, 1.6), (1, 200.0)]);
    let wall_level = p.chain_insert(fuzz, 4, K::Amplifier).unwrap();
    set(p, wall_level, "Wall Level", &[(0, 0.12)]);
    send(p, fuzz, space);

    // The chip arp: a thin square crushed to a few bits and a low rate,
    // its pulse width swept by a Modulator, into an echo.
    let arp = add(p, K::Generator);
    set(p, arp, "Chip Arp", &[(0, 0.12), (1, 1.0), (2, 0.001), (3, 0.12), (4, 0.4), (5, 0.05), (8, 0.25)]);
    let crush = p.chain_insert(arp, 0, K::Distortion).unwrap();
    set(p, crush, "8-bit", &[(0, 1.5), (1, 0.9), (2, 1.0), (3, 1.0), (4, 4.0), (5, 8.0)]);
    let arp_echo = p.chain_insert(arp, 1, K::Delay).unwrap();
    set(p, arp_echo, "Arp Echo", &[(0, 3.0), (1, 0.35), (2, 0.25), (3, 1.0)]);
    // Its level after the crush, which needs a strong signal to keep its bits.
    p.module_mut(arp_echo).unwrap().gain = 0.5;
    p.module_mut(arp).unwrap().phrases = vec![climb(), shards()];
    p.module_mut(arp).unwrap().phrase_mode = PhraseMode::Off;
    send(p, arp, space);

    // The voice: a MultiSynth singing with a saw through a Vocal Filter, an
    // octave-up pitch shifter and a waver, and a breathy SpectraVoice.
    let vox = add(p, K::MultiSynth);
    set(p, vox, "Vox", &[(3, 4.0), (5, 0.08)]);
    let vox_saw = add(p, K::Generator);
    set(p, vox_saw, "Vox Saw", &[(0, 0.5), (1, 0.0), (2, 0.01), (3, 0.3), (4, 0.8), (5, 0.15), (6, 8.0), (7, 2.0)]);
    let m = p.module_mut(vox_saw).unwrap();
    m.modulation.vibrato = VoiceLfo { on: true, shape: 0, rate: 5.5, depth: 0.15, delay: 0.25 };
    let mouth = p.chain_insert(vox_saw, 0, K::VocalFilter).unwrap();
    set(p, mouth, "Mouth", &[(1, 3.0), (4, 2.2)]);
    let chipmunk = p.chain_insert(vox_saw, 1, K::PitchShifter).unwrap();
    set(p, chipmunk, "Chipmunk", &[(0, 12.0), (2, 0.04), (4, 0.4)]);
    let waver = p.chain_insert(vox_saw, 2, K::Vibrato).unwrap();
    set(p, waver, "Waver", &[(0, 5.5), (1, 0.08), (2, 0.2)]);
    let vox_echo = p.chain_insert(vox_saw, 3, K::Echo).unwrap();
    set(p, vox_echo, "Vox Echo", &[(0, 0.28), (1, 0.35), (2, 0.4), (3, 0.6), (4, 0.25)]);
    send(p, vox_saw, space);
    let breath = add(p, K::SpectraVoice);
    set(p, breath, "Vox Air", &[(0, 0.05), (1, 24.0), (2, 1.6), (3, 0.5), (5, 0.4), (6, 0.02), (9, 0.3)]);
    send(p, breath, space);
    p.connect(vox, vox_saw);
    p.connect(vox, breath);

    // The verse's lead: a square with a filter envelope, crushed, phased
    // and scattered in taps.
    let lead = add(p, K::Generator);
    set(p, lead, "Pulse Lead", &[(0, 0.2), (1, 1.0), (2, 0.002), (3, 0.15), (4, 0.6), (5, 0.08)]);
    let m = p.module_mut(lead).unwrap();
    m.modulation.filter = true;
    m.modulation.cutoff = 1800.0;
    m.modulation.resonance = 0.45;
    m.modulation.filter_env =
        VoiceEnvelope { on: true, points: vec![(0.0, 1.0), (0.12, 0.3)], sustain: None, curve: true, amount: 2.0 };
    let lead_crush = p.chain_insert(lead, 0, K::Distortion).unwrap();
    set(p, lead_crush, "Lead Crush", &[(0, 2.0), (1, 0.7), (2, 1.0), (3, 1.0), (4, 6.0), (5, 3.0)]);
    let lead_phase = p.chain_insert(lead, 1, K::Phaser).unwrap();
    set(p, lead_phase, "Lead Phase", &[(0, 0.3), (1, 0.6), (6, 0.3)]);
    let taps = p.chain_insert(lead, 2, K::Multitap).unwrap();
    set(p, taps, "Lead Taps", &[(0, 3.0), (3, 6.0), (6, 9.0), (9, 12.0), (12, 0.25), (13, 0.25)]);
    send(p, lead, space);

    // A glassy FMX pluck for the last drop's counter-melody, ringing in F#.
    let pluck = add(p, K::Fmx);
    let ops = [(3, 1.0), (4, 1.0), (6, 0.4), (7, 0.0), (9, 0.6), (10, 7.0), (12, 0.12), (13, 0.0)];
    set(
        p,
        pluck,
        "Glass Pluck",
        &[&[(0, 0.6), (1, 4.0)], &ops[..], &[(15, 1.0), (16, 2.0), (18, 0.3), (21, 0.3)]].concat(),
    );
    let ring_fs = p.chain_insert(pluck, 0, K::CombFilter).unwrap();
    set(p, ring_fs, "Ring in F#", &[(0, 66.0), (2, 0.6), (3, 0.3), (4, 0.25)]);
    let pluck_plate = p.chain_insert(pluck, 1, K::PlateReverb).unwrap();
    set(p, pluck_plate, "Pluck Plate", &[(0, 0.7), (1, 0.02), (4, 0.3)]);
    send(p, pluck, space);

    // Haze: a shimmering SpectraVoice pad, phased, ringing and filtered.
    let haze = add(p, K::SpectraVoice);
    set(p, haze, "Haze", &[(0, 0.3), (1, 20.0), (2, 1.2), (3, 0.6), (5, 0.5), (6, 1.2), (7, 1.0), (8, 0.8), (9, 2.5)]);
    let haze_phase = p.chain_insert(haze, 0, K::Phaser).unwrap();
    set(p, haze_phase, "Haze Phase", &[(0, 0.1), (1, 0.8), (4, 6.0), (6, 0.4)]);
    let haze_ring = p.chain_insert(haze, 1, K::RingMod).unwrap();
    set(p, haze_ring, "Haze Ring", &[(0, 330.0), (2, 0.3), (3, 0.1)]);
    let haze_filter = p.chain_insert(haze, 2, K::AnalogFilter).unwrap();
    set(p, haze_filter, "Haze Filter", &[(1, 6000.0), (2, 0.35), (3, 1.5)]);
    send(p, haze, space);

    // Vocal chops: the four syllables as slices, each tuned to F# minor,
    // gated tight and screamed through.
    let chops = add(p, K::Sampler);
    set(p, chops, "Vox Chops", &[(0, 1.0), (4, 0.3), (5, 0.0), (6, 0.04)]);
    let (vox_sample, starts) = syllables(sr);
    let mut slot = rendered(vox_sample);
    slot.base_note = 64;
    slot.slices = starts[1..].to_vec();
    slot.slice_settings = [2, -3, 0, 5]
        .iter()
        .map(|&t| SliceSettings { transpose: t, oneshot: true, ..SliceSettings::default() })
        .collect();
    p.module_mut(chops).unwrap().samples = vec![slot];
    let chop_gate = p.chain_insert(chops, 0, K::Gate).unwrap();
    set(p, chop_gate, "Chop Gate", &[(0, 0.04), (2, 0.02), (3, 0.04)]);
    let scream = p.chain_insert(chops, 1, K::ScreamFilter).unwrap();
    set(p, scream, "Chop Scream", &[(0, 2.0), (1, 1800.0), (2, 0.5), (3, 3.0), (4, 0.45)]);
    send(p, chops, space);

    // The break: a bar of breakbeat, synced to four beats, sliced in
    // eight, and seeking when the song starts partway through.
    let break_loop = add(p, K::Sampler);
    set(p, break_loop, "Break", &[(0, 0.3), (4, 0.5), (5, 1.0), (6, 0.03)]);
    let (beat, step) = breakbeat(sr);
    let mut slot = rendered(beat);
    slot.base_note = 48;
    slot.beat_sync = 16;
    slot.autoseek = true;
    slot.loop_mode = 1;
    slot.slices = (1..8).map(|k| k * 2 * step).collect();
    p.module_mut(break_loop).unwrap().samples = vec![slot];
    let break_crush = p.chain_insert(break_loop, 0, K::Distortion).unwrap();
    set(p, break_crush, "Break Crush", &[(0, 2.0), (1, 0.75), (2, 0.6), (3, 1.0), (4, 8.0), (5, 2.0)]);
    let break_bus = p.chain_insert(break_loop, 1, K::Compressor).unwrap();
    set(p, break_bus, "Break Bus", &[(0, 0.3), (1, 3.0), (2, 0.005), (3, 0.1), (4, 1.4)]);
    let break_tone = p.chain_insert(break_loop, 2, K::Eq).unwrap();
    set(p, break_tone, "Break Tone", &[(0, 0.6), (1, 1.1), (2, 1.4), (3, 150.0)]);

    // A noise riser: swept through a band, jetting through a flanger.
    let riser = add(p, K::Generator);
    set(p, riser, "Riser", &[(0, 0.5), (1, 4.0), (2, 2.0), (3, 0.0), (4, 1.0), (5, 0.4)]);
    let sweep = p.chain_insert(riser, 0, K::FilterPro).unwrap();
    set(p, sweep, "Sweep", &[(0, 2.0), (1, 400.0), (2, 4.0), (4, 1.0)]);
    let jet = p.chain_insert(riser, 1, K::Flanger).unwrap();
    set(p, jet, "Jet", &[(2, 0.8), (3, 0.2), (4, 0.7), (5, 0.5)]);
    let riser_level = p.chain_insert(riser, 2, K::Amplifier).unwrap();
    set(p, riser_level, "Riser Level", &[(0, 1.2)]);
    send(p, riser, space);

    // Bleeps for the boot: short, bright FM.
    let blip = add(p, K::Fm);
    set(p, blip, "Blip", &[(0, 0.4), (1, 3.5), (2, 4.0), (3, 0.05), (5, 0.001), (6, 0.08), (8, 0.05)]);
    send(p, blip, space);

    // Modulators. Duck: the wall and the haze pump under the drums. PWM:
    // the arp's pulse width swims. Chop: a drawn gate on the wall's
    // square, its amount automated in the last drop. Bright: the lead's
    // crush opens up on high notes.
    let duck = add(p, K::Modulator);
    set(p, duck, "Duck", &[(0, 1.0), (5, -0.6), (6, 0.003), (7, 0.18)]);
    p.connect(glitch, duck);
    for (target, fader) in [(fuzz, K::Distortion.params().len()), (haze, K::SpectraVoice.params().len())] {
        p.connect(duck, target);
        p.set_control_param(duck, target, fader);
    }
    let pwm = add(p, K::Modulator);
    set(p, pwm, "PWM", &[(0, 0.0), (1, 0.0), (2, 0.3), (5, 0.3)]);
    p.connect(pwm, arp);
    p.set_control_param(pwm, arp, 8);
    let chop = add(p, K::Modulator);
    set(p, chop, "Chop", &[(0, 0.0), (1, DRAWN_SHAPE as f32), (3, 2.0), (5, 0.0), (8, 4.0)]);
    p.module_mut(chop).unwrap().shape =
        vec![(0.0, 0.0), (0.4, 0.0), (0.45, 1.0), (0.6, 1.0), (0.65, 0.0), (0.8, 0.0), (0.85, 1.0), (1.0, 1.0)];
    p.connect(chop, square);
    p.set_control_param(chop, square, 0);
    let bright = add(p, K::Modulator);
    set(p, bright, "Bright", &[(0, 2.0), (5, 0.4)]);
    p.connect(lead, bright);
    p.connect(bright, lead_crush);
    p.set_control_param(bright, lead_crush, 1);

    Ids {
        drums,
        impact,
        bass,
        wall,
        wall_filter,
        chop,
        arp,
        crush,
        vox,
        mouth,
        lead,
        pluck,
        haze,
        haze_filter,
        haze_ring,
        chops,
        break_loop,
        riser,
        sweep,
        blip,
        glitch,
        space,
    }
}

/// The arp's climbing phrase: root, fifth and octaves in thirty-seconds,
/// cut short.
fn climb() -> Phrase {
    let mut ph = Phrase { lines: 16, lpb: 8, looping: true, ..Phrase::default() };
    for (l, note) in [48, 55, 60, 67, 72, 79, 72, 67, 48, 55, 60, 67, 72, 67, 60, 55].into_iter().enumerate() {
        ph.cells[l] = Cell {
            note: Some(Note::On(note)),
            vol: Some(if l % 4 == 0 { 0x80 } else { 0x50 }),
            fx: Some((0xC, 0x02)),
            ..Cell::default()
        };
    }
    ph
}

/// The arp's broken phrase: octaves jumping about, some left to chance.
fn shards() -> Phrase {
    let mut ph = Phrase { lines: 16, lpb: 8, looping: true, ..Phrase::default() };
    for (l, note) in [60, 48, 67, 55, 72, 60, 79, 67, 72, 84, 67, 79, 60, 72, 55, 67].into_iter().enumerate() {
        let cmd = if l % 5 == 3 { (FX_MAYBE, 0x50) } else { (0xC, 0x02) };
        ph.cells[l] = Cell {
            note: Some(Note::On(note)),
            vol: Some(if l % 2 == 0 { 0x70 } else { 0x40 }),
            fx: Some(cmd),
            ..Cell::default()
        };
    }
    ph
}

// ---------------------------------------------------------------- parts

/// A new pattern with the song's columns.
fn pattern() -> Pattern {
    let mut pat = Pattern::new("", 10, 64);
    pat.set_columns(DRUMS, 2);
    pat.set_columns(WALL, 4);
    pat.set_columns(VOX, 2);
    pat.set_columns(HAZE, 3);
    pat
}

/// The wall's chords, strummed down a little with the delay column, held
/// a bar each.
fn wall_chords(pat: &mut Pattern, ids: &Ids, chords: &[Chord; 4], vol: u8) {
    for (bar, ch) in chords.iter().enumerate() {
        let at = bar * 16;
        *pat.cell_mut(WALL, 0, at) = n(ch.root + 12, ids.wall, vol);
        for (c, &tone) in ch.tones.iter().enumerate() {
            *pat.cell_mut(WALL, c + 1, at) = Cell { delay: Some(0x18 * (c as u8 + 1)), ..n(tone, ids.wall, vol) };
        }
    }
}

fn haze_chords(pat: &mut Pattern, ids: &Ids, chords: &[Chord; 4], vol: u8) {
    for (bar, ch) in chords.iter().enumerate() {
        for (c, &tone) in ch.tones.iter().enumerate() {
            *pat.cell_mut(HAZE, c, bar * 16) = n(tone + 12, ids.haze, vol);
        }
    }
}

/// The chip arp on the off-beats: the chord's note, made a chord with
/// 0xy and cut short with Cxx.
fn chip_stabs(pat: &mut Pattern, ids: &Ids, chords: &[Chord; 4], vol: u8) {
    for (bar, ch) in chords.iter().enumerate() {
        for k in [2, 6, 10, 14] {
            pat.tracks[ARP][bar * 16 + k] = fx(n(ch.arp, ids.arp, vol), 0x0, ch.arpeggio);
            pat.tracks[ARP][bar * 16 + k + 1].fx = Some((0xC, 0x03));
        }
    }
}

/// The arp playing phrase `phrase` (Zxx) from each bar's chord.
fn arp_phrase(pat: &mut Pattern, ids: &Ids, chords: &[Chord; 4], phrase: u8) {
    for (bar, ch) in chords.iter().enumerate() {
        pat.tracks[ARP][bar * 16] = fx(n(ch.root + 24, ids.arp, 0x60), FX_PHRASE, phrase);
        pat.tracks[ARP][bar * 16 + 15] = off();
    }
}

/// Sixteenth hats, panned with the panning column, with rolls (Exx) and
/// an open hat cut short (Cxx).
fn hats(pat: &mut Pattern, ids: &Ids, every: usize, vol: u8, rolls: &[usize]) {
    for l in (0..64).step_by(every) {
        let pan = if (l / every).is_multiple_of(2) { 0x30 } else { 0x50 };
        let v = if l % 4 == 0 { vol } else { vol.saturating_sub(0x14) };
        pat.tracks[HATS][l] = Cell { pan: Some(pan), ..n(54, ids.drums, v) };
    }
    for &l in rolls {
        pat.tracks[HATS][l] = fx(n(54, ids.drums, vol), 0xE, 0x02);
        pat.tracks[HATS][l + 1] = fx(Cell::default(), 0xE, 0x02);
    }
    for bar in 0..4 {
        pat.tracks[HATS][bar * 16 + 14] = fx(n(58, ids.drums, vol + 0x10), 0xC, 0x04);
    }
}

/// The 808, `hits` in each bar as (line, octave up, glide), gliding
/// with 3xx.
fn bass_line(pat: &mut Pattern, ids: &Ids, chords: &[Chord; 4], hits: &[(usize, u8, bool)], vol: u8) {
    for (bar, ch) in chords.iter().enumerate() {
        for &(l, up, glide) in hits {
            let cell = n(ch.root + up, ids.bass, vol);
            pat.tracks[BASS][bar * 16 + l] = if glide { fx(cell, 0x3, 0x30) } else { cell };
        }
    }
}

/// A sung line, gliding into notes with 3xx and with vibrato (4xy) on
/// its long ones; `harmony` adds a third below in the second column.
fn sing(pat: &mut Pattern, ids: &Ids, line: &[(usize, u8, usize)], vol: u8, harmony: bool) {
    let mut last = 0;
    for &(l, note, len) in line {
        let mut cell = n(note, ids.vox, vol);
        if note.abs_diff(last) >= 3 && last != 0 {
            cell = fx(cell, 0x3, 0x40);
        }
        pat.tracks[VOX][l] = cell;
        if len >= 4 {
            for k in l + 2..l + len {
                pat.tracks[VOX][k].fx = Some((0x4, 0x63));
            }
        }
        if harmony {
            *pat.cell_mut(VOX, 1, l) = n(third_below(note), ids.vox, vol - 0x20);
        }
        last = note;
    }
    let (end, _, len) = line[line.len() - 1];
    if end + len < 64 {
        pat.tracks[VOX][end + len] = off();
    }
}

/// Bleeps of the boot, each one maybe there (Yxx).
fn blips(pat: &mut Pattern, ids: &Ids, from: usize) {
    let notes = [90, 85, 97, 78, 93, 88, 102, 81];
    for (k, l) in (from..64).step_by(3).enumerate() {
        pat.tracks[FX][l] = fx(n(notes[k % notes.len()], ids.blip, 0x40), FX_MAYBE, 0x90);
    }
}

fn boot(ids: &Ids) -> Pattern {
    let mut pat = pattern();
    blips(&mut pat, ids, 0);
    haze_chords(&mut pat, ids, &LOOP, 0x50);
    chip_stabs(&mut pat, ids, &LOOP, 0x50);
    // Chops answer the bleeps from the left and the right (8xx).
    for (l, slice, pan) in [(8, 65, 0x20), (24, 67, 0xE0), (40, 66, 0x40), (56, 68, 0xC0)] {
        pat.tracks[CHOPS][l] = fx(n(slice, ids.chops, 0x60), 0x8, pan);
    }
    // The haze opens up, the arp comes out of its crush, and the riser
    // takes the last bar.
    let a = ModuleKind::AnalogFilter;
    pat.automation.push(envelope(ids.haze_filter, a, 1, &[(0.0, 300.0), (64.0, 6000.0)], false, true));
    let d = ModuleKind::Distortion;
    let rates = [(0.0, 24.0), (16.0, 16.0), (32.0, 12.0), (48.0, 8.0)];
    pat.automation.push(envelope(ids.crush, d, 5, &rates, true, false));
    pat.tracks[FX][48] = n(60, ids.riser, 0x60);
    pat.tracks[FX][63] = off();
    let f = ModuleKind::FilterPro;
    pat.automation.push(envelope(ids.sweep, f, 1, &[(48.0, 300.0), (64.0, 9000.0)], false, true));
    pat
}

fn intro(ids: &Ids) -> Pattern {
    let mut pat = pattern();
    *pat.cell_mut(DRUMS, 1, 0) = n(30, ids.impact, 0x70);
    // The break plays whole, the bar long whatever the tempo.
    for bar in 0..4 {
        pat.tracks[CHOPS][bar * 16] = n(48, ids.break_loop, 0x70);
    }
    pat.tracks[CHOPS][63] = off();
    chip_stabs(&mut pat, ids, &LOOP, 0x60);
    haze_chords(&mut pat, ids, &LOOP, 0x40);
    bass_line(&mut pat, ids, &LOOP, &[(0, 0, false), (10, 0, false)], 0x60);
    for l in [0, 16, 32, 48] {
        pat.tracks[DRUMS][l] = n(48, ids.drums, 0x70);
    }
    // The drums glitch at the end: the Repeater holds shorter and shorter.
    glitch_at(&mut pat, ids, 56.0);
    pat
}

/// The Repeater holding the drums from `from` to the end, faster as it
/// goes.
fn glitch_at(pat: &mut Pattern, ids: &Ids, from: f32) {
    let r = ModuleKind::Repeater;
    pat.automation.push(envelope(ids.glitch, r, 0, &[(0.0, 0.0), (from, 1.0)], true, false));
    let lengths = [(from, 4.0), (from + 4.0, 6.0), (from + 6.0, 8.0)];
    pat.automation.push(envelope(ids.glitch, r, 1, &lengths, true, false));
}

/// Half-time drums: kicks, a snare on the third beat with a flam (Dxx)
/// before the next, and ghost notes.
fn half_time(pat: &mut Pattern, ids: &Ids) {
    for bar in 0..4 {
        let at = bar * 16;
        pat.tracks[DRUMS][at] = n(48, ids.drums, 0x78);
        pat.tracks[DRUMS][at + 10] = n(48, ids.drums, 0x60);
        if bar % 2 == 1 {
            pat.tracks[DRUMS][at + 6] = n(48, ids.drums, 0x50);
        }
        pat.tracks[DRUMS][at + 8] = n(50, ids.drums, 0x78);
        pat.tracks[DRUMS][at + 14] = fx(n(50, ids.drums, 0x24), 0xD, 0x03);
    }
}

fn verse(ids: &Ids, second: bool) -> Pattern {
    let mut pat = pattern();
    half_time(&mut pat, ids);
    hats(&mut pat, ids, 2, 0x40, &[28, 60]);
    bass_line(&mut pat, ids, &LOOP, &[(0, 0, false), (7, 0, false), (10, 12, true)], 0x68);
    chip_stabs(&mut pat, ids, &LOOP, 0x60);
    for bar in 0..4 {
        pat.tracks[CHOPS][bar * 16] = n(48, ids.break_loop, 0x50);
    }
    if second {
        // The voice, low and close, and chops on the off-beats.
        sing(&mut pat, ids, &LOW_LINE, 0x60, false);
        for (k, l) in [6, 22, 38, 54].into_iter().enumerate() {
            pat.tracks[FX][l] = fx(n(65 + k as u8, ids.chops, 0x50), FX_MAYBE, 0xC0);
        }
    } else {
        // The square lead, staccato.
        for (l, note) in VERSE_LEAD {
            pat.tracks[LEAD][l] = fx(n(note, ids.lead, if l % 4 == 0 { 0x70 } else { 0x58 }), 0xC, 0x04);
        }
    }
    pat
}

fn build(ids: &Ids) -> Pattern {
    let mut pat = pattern();
    wall_chords(&mut pat, ids, &LOOP, 0x58);
    arp_phrase(&mut pat, ids, &LOOP, 1);
    // Kicks on every beat, the snare tightening into a roll (Exx).
    for l in (0..48).step_by(4) {
        pat.tracks[DRUMS][l] = n(48, ids.drums, 0x68);
    }
    for l in (32..48).step_by(2) {
        pat.tracks[DRUMS][l + 1] = n(50, ids.drums, 0x30 + l as u8);
    }
    for (l, rate) in [(48, 0x03), (52, 0x02), (56, 0x02), (60, 0x01)] {
        pat.tracks[DRUMS][l] = fx(n(50, ids.drums, 0x50 + (l as u8 - 48) * 2), 0xE, rate);
        for k in 1..4 {
            pat.tracks[DRUMS][l + k].fx = Some((0xE, rate));
        }
    }
    hats(&mut pat, ids, 2, 0x38, &[]);
    // The 808 holds the root and slides up an octave (1xx) in the last bar.
    pat.tracks[BASS][0] = n(30, ids.bass, 0x60);
    pat.tracks[BASS][32] = n(30, ids.bass, 0x60);
    pat.tracks[BASS][48] = fx(n(30, ids.bass, 0x60), 0x1, 0x02);
    for l in 49..64 {
        pat.tracks[BASS][l].fx = Some((0x1, 0x02));
    }
    // The voice rises on "ah" with the riser.
    pat.tracks[VOX][32] = fx(n(66, ids.vox, 0x50), 0x1, 0x01);
    for l in 33..64 {
        pat.tracks[VOX][l].fx = Some((0x1, 0x01));
    }
    pat.tracks[FX][0] = n(60, ids.riser, 0x70);
    pat.tracks[FX][63] = off();
    // The wall opens from muffled to bright, the riser sweeps up, and the
    // drums glitch in the last bar.
    let flt = ModuleKind::Filter;
    pat.automation.push(envelope(ids.wall_filter, flt, 1, &[(0.0, 250.0), (60.0, 18000.0)], false, true));
    let f = ModuleKind::FilterPro;
    pat.automation.push(envelope(ids.sweep, f, 1, &[(0.0, 300.0), (64.0, 10000.0)], false, true));
    glitch_at(&mut pat, ids, 60.0);
    pat
}

fn drop(ids: &Ids, last: bool) -> Pattern {
    let mut pat = pattern();
    *pat.cell_mut(DRUMS, 1, 0) = n(30, ids.impact, 0x78);
    wall_chords(&mut pat, ids, &LOOP, 0x70);
    arp_phrase(&mut pat, ids, &LOOP, if last { 2 } else { 1 });
    // Two-step drums, doubled by the break.
    for bar in 0..4 {
        let at = bar * 16;
        for l in [0, 6, 10] {
            pat.tracks[DRUMS][at + l] = n(48, ids.drums, 0x78);
        }
        if bar % 2 == 1 {
            pat.tracks[DRUMS][at + 13] = n(48, ids.drums, 0x58);
        }
        pat.tracks[DRUMS][at + 4] = n(50, ids.drums, 0x78);
        pat.tracks[DRUMS][at + 12] = n(50, ids.drums, 0x78);
        pat.tracks[CHOPS][at] = n(48, ids.break_loop, 0x60);
    }
    // The last two lines fill with a snare retriggered fast.
    pat.tracks[DRUMS][62] = fx(n(50, ids.drums, 0x60), 0xE, 0x01);
    pat.tracks[DRUMS][63].fx = Some((0xE, 0x01));
    hats(&mut pat, ids, 1, 0x40, &[30, 46]);
    bass_line(
        &mut pat,
        ids,
        &LOOP,
        &[(0, 0, false), (3, 0, false), (6, 12, true), (8, 0, true), (11, 0, false), (14, 7, true)],
        0x70,
    );
    sing(&mut pat, ids, &HOOK, 0x70, last);
    let v = ModuleKind::VocalFilter;
    let vowels = [(0.0, 0.0), (12.0, 2.0), (24.0, 1.0), (40.0, 3.0), (56.0, 0.0), (64.0, 2.0)];
    pat.automation.push(envelope(ids.mouth, v, 0, &vowels, false, true));
    if last {
        // A glassy counter-melody, swinging across (Nxy), and the wall's
        // square chopped by the drawn gate.
        for (bar, ch) in LOOP.iter().enumerate() {
            for (k, l) in [2, 5, 8, 11, 14].into_iter().enumerate() {
                let note = ch.tones[k % 3] + 24 + if k == 4 { 12 } else { 0 };
                let cell = n(note, ids.pluck, 0x48 - k as u8 * 4);
                pat.tracks[LEAD][bar * 16 + l] = if k == 0 { fx(cell, FX_AUTOPAN, 0x48) } else { cell };
            }
        }
        let m = ModuleKind::Modulator;
        pat.automation.push(envelope(ids.chop, m, 5, &[(0.0, -0.9)], true, false));
        // Glitches halfway and at the end.
        let r = ModuleKind::Repeater;
        let hold = [(0.0, 0.0), (28.0, 1.0), (32.0, 0.0), (60.0, 1.0)];
        pat.automation.push(envelope(ids.glitch, r, 0, &hold, true, false));
        pat.automation.push(envelope(ids.glitch, r, 1, &[(28.0, 5.0), (60.0, 6.0), (62.0, 8.0)], true, false));
    }
    pat
}

fn breakdown(ids: &Ids) -> Pattern {
    let mut pat = pattern();
    // The voice, the wall and the 808 let go; the haze trembles (7xy)
    // over the darker chords.
    pat.tracks[VOX][0] = off();
    *pat.cell_mut(VOX, 1, 0) = off();
    for c in 0..4 {
        *pat.cell_mut(WALL, c, 0) = off();
    }
    pat.tracks[BASS][0] = off();
    haze_chords(&mut pat, ids, &BREAK, 0x60);
    for bar in 0..4 {
        for c in 0..3 {
            for l in 8..16 {
                pat.cell_mut(HAZE, c, bar * 16 + l).fx = Some((0x7, 0x46));
            }
        }
    }
    chip_stabs(&mut pat, ids, &BREAK, 0x50);
    // Break slices chopped out of order, some retriggered (Exx), some
    // maybe there (Yxx); chops start partway in (9xx) and jump about (8xx).
    let order = [1, 4, 1, 2, 7, 6, 3, 8, 1, 1, 5, 2, 8, 7, 4, 4];
    for (k, &slice) in order.iter().cycle().take(32).enumerate() {
        let l = k * 2;
        let cell = n(48 + slice, ids.break_loop, if k % 4 == 0 { 0x70 } else { 0x50 });
        pat.tracks[CHOPS][l] = match k % 7 {
            3 => fx(cell, 0xE, 0x02),
            5 => fx(cell, FX_MAYBE, 0x80),
            _ => cell,
        };
    }
    for (k, l) in (4..64).step_by(6).enumerate() {
        let cell = n(65 + (k % 4) as u8, ids.chops, 0x58);
        pat.tracks[FX][l] =
            fx(cell, if k % 2 == 0 { 0x9 } else { 0x8 }, if k % 2 == 0 { 0x30 } else { 0x20 + k as u8 * 0x18 });
    }
    pat.tracks[DRUMS][0] = n(48, ids.drums, 0x70);
    // The arp sinks into the crush, the haze rings more, and the space
    // opens.
    let d = ModuleKind::Distortion;
    let rates = [(0.0, 2.0), (16.0, 6.0), (32.0, 12.0), (48.0, 20.0), (56.0, 32.0)];
    pat.automation.push(envelope(ids.crush, d, 5, &rates, true, false));
    pat.automation.push(envelope(ids.crush, d, 4, &[(0.0, 6.0), (32.0, 4.0), (48.0, 3.0)], true, false));
    let rm = ModuleKind::RingMod;
    pat.automation.push(envelope(ids.haze_ring, rm, 3, &[(0.0, 0.1), (64.0, 0.6)], false, true));
    let rv = ModuleKind::Reverb;
    pat.automation.push(envelope(ids.space, rv, 2, &[(0.0, 0.3), (32.0, 0.5), (64.0, 0.3)], false, true));
    pat
}

fn tape_stop(ids: &Ids) -> Pattern {
    let mut pat = pattern();
    *pat.cell_mut(DRUMS, 1, 0) = n(30, ids.impact, 0x78);
    pat.tracks[VOX][0] = off();
    *pat.cell_mut(VOX, 1, 0) = off();
    // The wall rings out two chords, then the tape stops: the tempo falls
    // (Fxx), everything slides down (2xx) and fades (Axy), and F00 ends
    // the song.
    for (at, ch) in [(0, FSM), (32, D)] {
        for (c, &tone) in [ch.root + 12].iter().chain(&ch.tones).enumerate() {
            *pat.cell_mut(WALL, c, at) = n(tone, ids.wall, 0x70);
        }
    }
    haze_chords(&mut pat, ids, &[FSM, FSM, D, D], 0x50);
    pat.tracks[BASS][0] = n(30, ids.bass, 0x70);
    pat.tracks[BASS][32] = n(26, ids.bass, 0x70);
    for (l, slice) in [(0, 48), (16, 48)] {
        pat.tracks[CHOPS][l] = n(slice, ids.break_loop, 0x60);
    }
    pat.tracks[CHOPS][32] = off();
    for (l, bpm) in [(40, 0x88), (44, 0x70), (48, 0x58), (52, 0x44), (56, 0x34), (60, 0x26)] {
        pat.tracks[FX][l].fx = Some((0xF, bpm));
    }
    for l in 40..64 {
        for c in 0..4 {
            pat.cell_mut(WALL, c, l).fx = Some(if l < 52 { (0x2, 0x02) } else { (0xA, 0x02) });
        }
        pat.tracks[BASS][l].fx = Some((0x2, 0x02));
        for c in 0..3 {
            pat.cell_mut(HAZE, c, l).fx = Some((0xA, 0x01));
        }
    }
    pat.tracks[DRUMS][63].fx = Some((0xF, 0x00));
    let flt = ModuleKind::Filter;
    pat.automation.push(envelope(ids.wall_filter, flt, 1, &[(32.0, 18000.0), (64.0, 400.0)], false, true));
    let out = ModuleKind::Output;
    pat.automation.push(envelope(OUTPUT_ID, out, 0, &[(40.0, 0.8), (64.0, 0.3)], false, true));
    pat
}

impl Project {
    /// "Static Heart", the second demo song.
    pub fn static_heart() -> Self {
        let mut p = Self::empty();
        p.title = "Static Heart".into();
        p.artist = "noise".into();
        p.comments = COMMENTS.into();
        p.bpm = 160.0;
        p.groove = 0.04;
        p.modules[0].params[0] = 0.8;
        let names = ["Drums", "Hats", "808", "Wall", "Arp", "Vox", "Lead", "Haze", "Chops", "FX"];
        let colors = [
            [230, 70, 110],
            [240, 150, 190],
            [150, 60, 220],
            [90, 120, 255],
            [80, 230, 200],
            [255, 120, 220],
            [120, 255, 140],
            [140, 170, 230],
            [255, 210, 90],
            [200, 200, 200],
        ];
        for (t, (name, color)) in names.iter().zip(colors).enumerate() {
            p.tracks[t].name = name.to_string();
            p.tracks[t].color = Some(color);
        }
        p.tracks[HATS].show_pan = true;
        p.tracks[WALL].show_delay = true;
        let ids = instruments(&mut p);
        let mut patterns = vec![
            boot(&ids),
            intro(&ids),
            verse(&ids, false),
            verse(&ids, true),
            build(&ids),
            drop(&ids, false),
            drop(&ids, true),
            breakdown(&ids),
            tape_stop(&ids),
        ];
        let names = ["Boot", "Glitch", "Verse", "Verse 2", "Build", "Drop", "Last Drop", "Static", "Tape Stop"];
        for (pat, name) in patterns.iter_mut().zip(names) {
            pat.name = name.into();
        }
        p.patterns = patterns;
        for (t, cols) in [(DRUMS, 2), (WALL, 4), (VOX, 2), (HAZE, 3)] {
            p.set_columns(t, cols);
        }
        // The first verse holds back the arp and the break, in the matrix.
        let muted = |pattern: usize, tracks: &[usize]| {
            let mut s = Slot::new(pattern);
            tracks.iter().for_each(|&t| s.toggle_mute(t));
            s
        };
        p.order = vec![
            Slot::new(0),
            Slot::new(1),
            muted(2, &[ARP, CHOPS]),
            Slot::new(2),
            Slot::new(4),
            Slot::new(5),
            Slot::new(5),
            Slot::new(7),
            muted(7, &[HAZE]),
            Slot::new(3),
            Slot::new(4),
            Slot::new(6),
            Slot::new(6),
            Slot::new(8),
        ];
        p.sections = [
            ("Boot", 0),
            ("Verse", 2),
            ("Build", 4),
            ("Drop", 5),
            ("Static", 7),
            ("Verse 2", 9),
            ("Last Drop", 10),
            ("Tape Stop", 13),
        ]
        .into_iter()
        .map(|(name, start)| Section { start, name: name.into() })
        .collect();
        p
    }
}

const COMMENTS: &str = "Static Heart — the second demo song: glitchy, bitcrushed hyperpop in F# minor at 160 BPM.\n\n\
    It boots from bleeps and a crushed chip arp, drops a breakbeat under a square lead, builds into a wall of \
    fuzzed supersaws with a pitched-up voice gliding over it, breaks down to vocal chops and static, comes back \
    with a gated wall and a glass counter-melody, and dies in a tape stop.\n\n\
    Where to look:\n\
    • Wall: a MultiSynth playing a supersaw and a square into one Fuzz (a Distortion), a 4x12 Cabinet, a Chorus \
    and a Stereo Expander. Duck, following the drums, pumps it; Chop, a Modulator with a drawn shape synced to \
    beats, gates the square in the last drop, where its Amount is automated.\n\
    • Chip Arp: a square crushed to 4 bits by its Distortion, whose downsampling is automated; 0xy makes chords of \
    single notes, and its phrases (Z01, Z02) climb and shatter. PWM sweeps its pulse width.\n\
    • Vox: a MultiSynth singing through a Vocal Filter (its vowel automated), a Pitch Shifter an octave up and a \
    Vibrato, gliding with 3xx and wavering with 4xy; the last drop adds a harmony.\n\
    • Vox Chops and Break: Samplers with samples made by the song itself, sliced; each chop is tuned in its slice \
    settings. The break is beat-synced and seeks (Autoseek) when you play from partway through.\n\
    • 808: a Kicker with a long decay, gliding (3xx) and sliding (1xx), shaped by a WaveShaper.\n\
    • Static: break slices chopped with Exx and Yxx, chops with 9xx and 8xx, the haze trembling with 7xy.\n\
    • Tape Stop: Fxx slows the song to a halt, 2xx and Axy pull everything down, and F00 ends it.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_heart_uses_its_samples_slices_and_effects() {
        let p = Project::static_heart();
        let samplers: Vec<&Module> = p.modules.iter().filter(|m| m.kind == ModuleKind::Sampler).collect();
        assert_eq!(samplers.len(), 2);
        assert!(samplers.iter().all(|m| m.samples[0].slice_ranges().len() >= 4 && m.samples[0].data.is_some()));
        assert!(samplers.iter().any(|m| m.samples[0].autoseek && m.samples[0].beat_sync == 16));
        let mut commands: Vec<u8> = Vec::new();
        let cells = p.patterns.iter().flat_map(|pat| (0..pat.num_lanes()).flat_map(move |l| pat.lane(l).iter()));
        let phrase_cells = p.modules.iter().flat_map(|m| m.phrases.iter().flat_map(|ph| ph.cells.iter()));
        for c in cells.chain(phrase_cells) {
            commands.extend(c.fx.map(|f| f.0));
        }
        for cmd in [0x0, 0x1, 0x2, 0x3, 0x4, 0x7, 0x8, 0x9, 0xA, 0xC, 0xD, 0xE, 0xF, FX_AUTOPAN, FX_MAYBE, FX_PHRASE] {
            assert!(commands.contains(&cmd), "effect {:?}", char::from_digit(cmd as u32, 36));
        }
        // It ends on its F00.
        let last = &p.patterns[p.order.last().unwrap().pattern];
        assert_eq!(last.tracks[DRUMS][63].fx, Some((0xF, 0)));
        assert_eq!(third_below(73), 69, "C# to A");
        assert_eq!(third_below(66), 62, "F# to D");
    }

    #[test]
    fn static_heart_plays_loud_enough_without_clipping_and_ends() {
        let frames = crate::audio::render(std::sync::Arc::new(Project::static_heart()), 8000, 1.0, false);
        let peak = frames.iter().fold(0f32, |a, f| a.max(f[0].abs()).max(f[1].abs()));
        assert!(peak > 0.5 && peak <= 1.0, "{peak}");
        let secs = frames.len() as f32 / 8000.0;
        assert!((70.0..120.0).contains(&secs), "{secs} s");
    }
}
