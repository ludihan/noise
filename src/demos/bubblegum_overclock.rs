//! "Bubblegum Overclock", the second demo song: hyperpop in E major, a
//! sugary pop song pushed until it breaks, at 155 BPM and then 180.
//!
//! It spins up from a crawl to tempo under bell arpeggios, coin blips and
//! stuttering voice chops. A strummed acoustic guitar carries the verses
//! over half-time trap drums with rattling hat rolls and a gliding,
//! distorted 808, a pitched-up voice snapping from note to note above. The
//! build cuts to silence but for a stuttered "ah", and the drop is a wall
//! of clipped supersaws pumping under double-time drums, the voice singing
//! the hook with bells sparkling over it and the mix stuttering at the
//! ends of phrases. Then it melts, slowed down into the room, spins back
//! up to 180 a semitone higher for the last drops, and crashes: the mix
//! freezes, crushes to nothing and stops like a tape.
//!
//! The voice chops are rendered here (the syllables in `sounds`), so its
//! Sampler has slices without audio files; they are written next to the
//! song when it is saved.

use super::*;
use crate::project::*;

const LPB: usize = 4;
/// Lines a bar, and a pattern of four bars.
const BAR: usize = 4 * LPB;
const LINES: usize = 4 * BAR;

// ---------------------------------------------------------------- harmony

/// A chord: the 808's root, four notes for the guitar and the wall, and
/// the arpeggio (0xy) the bells make of their note.
#[derive(Clone, Copy)]
struct Chord {
    root: u8,
    tones: [u8; 4],
    arp: u8,
}

// MIDI numbers: 48 is the tracker's C-4. E, B, C♯ minor, A.
const LOOP: [Chord; 4] = [
    Chord { root: 28, tones: [52, 56, 59, 64], arp: 0x47 },
    Chord { root: 35, tones: [54, 59, 63, 66], arp: 0x47 },
    Chord { root: 37, tones: [56, 61, 64, 68], arp: 0x37 },
    Chord { root: 33, tones: [57, 61, 64, 69], arp: 0x47 },
];

/// The hook, an eighth note each over four bars; 0 holds the note before.
const HOOK: [u8; 32] = [
    71, 71, 73, 76, 76, 75, 73, 0, //
    75, 73, 71, 71, 68, 71, 0, 0, //
    76, 76, 78, 80, 80, 78, 76, 73, //
    76, 73, 71, 69, 68, 0, 0, 0,
];

/// The verse's line: (line, note, lines).
const VERSE: [(usize, u8, usize); 15] = [
    (0, 64, 4),
    (4, 66, 2),
    (6, 68, 6),
    (12, 66, 4),
    (16, 63, 4),
    (20, 64, 2),
    (22, 66, 6),
    (28, 63, 4),
    (32, 61, 4),
    (36, 64, 2),
    (38, 68, 6),
    (44, 66, 4),
    (48, 64, 6),
    (54, 61, 2),
    (56, 64, 8),
];

/// The note a third below `n` in E major, for the harmonies.
fn third_below(n: u8) -> u8 {
    const SCALE: [u8; 7] = [4, 6, 8, 9, 11, 1, 3];
    let mut m = n;
    for _ in 0..2 {
        m -= 1;
        while !SCALE.contains(&(m % 12)) {
            m -= 1;
        }
    }
    m
}

// ---------------------------------------------------------------- tracks

const DRUMS: usize = 0;
const HATS: usize = 1;
const BASS: usize = 2;
const GUITAR: usize = 3;
const WALL: usize = 4;
const VOX: usize = 5;
const BELLS: usize = 6;
const CHOPS: usize = 7;
const FX: usize = 8;
const TRACKS: usize = 9;

/// Note columns per track, where more than one.
const COLUMNS: [(usize, usize); 5] = [(DRUMS, 2), (GUITAR, 4), (WALL, 5), (VOX, 2), (FX, 2)];

/// The instruments and effects patterns refer to.
struct Ids {
    drums: u8,
    impact: u8,
    bass: u8,
    guitar: u8,
    wall: u8,
    wall_filter: u8,
    vox: u8,
    mouth: u8,
    bells: u8,
    chops: u8,
    blip: u8,
    riser: u8,
    sweep: u8,
    stutter: u8,
    crush: u8,
}

/// Makes every module and wires them up.
fn instruments(p: &mut Project) -> Ids {
    use ModuleKind as K;
    let add = |p: &mut Project, kind: ModuleKind, name: &str, params: &[(usize, f32)]| {
        let id = super::add(p, kind, name, params);
        p.connect(id, OUTPUT_ID);
        id
    };

    // A big bright room the voice, guitar, bells and wall share.
    let room = add(p, K::Reverb, "Bubble Room", &[(0, 0.88), (1, 0.35), (2, 0.28)]);

    // The master: a Repeater for the stutter edits, a clip that can crush
    // to a few bits at the end, glue, sheen and a limiter, loud.
    let [stutter, crush, ..] = effects(
        p,
        OUTPUT_ID,
        [
            (K::Repeater, "Stutter", &[]),
            (K::Distortion, "Overclock", &[(0, 1.8), (1, 0.75), (2, 0.5), (3, 1.0)]),
            (K::Compressor, "Squeeze", &[(0, 0.4), (1, 3.0), (2, 0.004), (3, 0.12), (4, 1.3)]),
            (K::Exciter, "Glitter", &[(0, 6500.0), (1, 2.5), (2, 0.15)]),
            (K::Maximizer, "Max Out", &[(0, 1.7), (1, 0.95), (2, 0.06)]),
        ],
    );

    // Drums: a tight synth kit, crunched and smacked.
    let drums = add(p, K::Drums, "Sugar Kit", &[(0, 0.75), (1, 58.0), (2, 0.2), (3, 0.8), (4, 0.03)]);
    effects(
        p,
        drums,
        [
            (K::Distortion, "Kit Crunch", &[(0, 3.0), (1, 0.8), (2, 0.4), (3, 1.0)]),
            (K::Compressor, "Kit Smack", &[(0, 0.3), (1, 4.0), (2, 0.003), (3, 0.07), (4, 1.5)]),
        ],
    );

    // A hit to land the drops on: a Kicker dropping far into a plate.
    let impact = add(p, K::Kicker, "Drop Hit", &[(0, 0.45), (2, 3.0), (3, 0.25), (5, 2.0), (6, 0.8)]);
    effects(p, impact, [(K::PlateReverb, "Hit Plate", &[(0, 0.85), (1, 0.01), (2, 0.5), (4, 0.4)])]);

    // The 808: a long sine Kicker that glides, rounded, then driven hard.
    let bass = add(p, K::Kicker, "808", &[(0, 0.1), (2, 0.6), (3, 0.04), (4, 0.002), (5, 1.1), (6, 0.5)]);
    let [.., low_808] = effects(
        p,
        bass,
        [
            (K::WaveShaper, "808 Round", &[(0, 1.6), (9, 0.4), (10, 0.68), (11, 0.86), (12, 0.95)]),
            (K::Distortion, "808 Grit", &[(0, 4.0), (1, 0.5), (2, 0.7)]),
            (K::DcBlocker, "808 Centre", &[]),
            (K::FilterPro, "808 Low", &[(1, 3000.0), (4, 1.0)]),
        ],
    );
    // Its level after the grit, which levels whatever goes in.
    p.module_mut(low_808).unwrap().gain = 0.55;

    // The guitar: three plucked strings a note, bright, strummed with the
    // delay column, chorused into the room.
    let guitar = add(
        p,
        K::PluckedString,
        "Emo Guitar",
        &[(0, 0.6), (1, 0.18), (2, 0.75), (3, 2.5), (4, 0.35), (5, 0.25), (6, 2.0), (7, 7.0)],
    );
    effects(p, guitar, [(K::Chorus, "Guitar Shimmer", &[(0, 2.0), (1, 0.6), (2, 0.4), (6, 0.35)])]);
    send(p, guitar, room);

    // The wall: four detuned saws a voice, clipped, chorused and spread
    // wide, pumping under the kit.
    let wall = add(
        p,
        K::Generator,
        "Supersaw Wall",
        &[(0, 0.2), (1, 0.0), (2, 0.005), (3, 0.3), (4, 0.85), (5, 0.3), (6, 30.0), (7, 4.0)],
    );
    let [wall_filter, ..] = effects(
        p,
        wall,
        [
            (K::Filter, "Wall Filter", &[(0, 0.0), (1, 18000.0), (2, 0.2)]),
            (K::Distortion, "Wall Clip", &[(0, 6.0), (1, 0.7), (2, 0.8), (3, 1.0)]),
            (K::Chorus, "Wall Chorus", &[(0, 3.0), (1, 0.5), (2, 0.6), (4, 0.45), (6, 0.5)]),
            (K::StereoExpander, "Wall Wide", &[(0, 1.7), (1, 180.0)]),
            (K::Amplifier, "Wall Level", &[(0, 0.35)]),
        ],
    );
    send(p, wall, room);

    // The voice: a MultiSynth singing with a saw through a Vocal Filter and
    // an octave-up pitch shifter, snapping between notes, doubled and
    // echoed, with a breathy SpectraVoice under it.
    let vox = add(p, K::MultiSynth, "Pop Vox", &[(3, 3.0), (5, 0.06)]);
    let vox_saw = add(
        p,
        K::Generator,
        "Vox Tone",
        &[(0, 0.9), (1, 0.0), (2, 0.008), (3, 0.3), (4, 0.85), (5, 0.12), (6, 6.0), (7, 2.0)],
    );
    let [mouth, ..] = effects(
        p,
        vox_saw,
        [
            (K::VocalFilter, "Vox Mouth", &[(1, 3.0), (4, 2.2)]),
            (K::PitchShifter, "Vox Up", &[(0, 12.0), (2, 0.03), (4, 0.85)]),
            (K::Chorus, "Vox Double", &[(0, 2.0), (1, 1.2), (2, 0.3), (6, 0.3)]),
            (K::Echo, "Vox Echo", &[(0, 0.29), (1, 0.3), (2, 0.4), (3, 0.6), (4, 0.2)]),
        ],
    );
    send(p, vox_saw, room);
    let air = add(p, K::SpectraVoice, "Vox Breath", &[(0, 0.05), (1, 24.0), (2, 1.6), (3, 0.5), (5, 0.4), (9, 0.3)]);
    send(p, air, room);
    p.connect(vox, vox_saw);
    p.connect(vox, air);

    // Bells: bright FM, its notes made chords with 0xy, sparkling about.
    let bells =
        add(p, K::Fm, "Sparkle Bells", &[(0, 0.3), (1, 3.5), (2, 3.5), (3, 0.3), (5, 0.001), (6, 0.5), (8, 0.3)]);
    effects(p, bells, [(K::Delay, "Bell Echo", &[(0, 3.0), (1, 0.35), (2, 0.25), (3, 1.0)])]);
    send(p, bells, room);

    // Vox Chops: the rendered syllables as slices, tuned up an octave onto
    // the chord, gated tight.
    let chops = add(p, K::Sampler, "Vox Chops", &[(0, 1.0), (4, 0.3), (6, 0.04)]);
    let (vox_sample, starts) = syllables(44100.0);
    let mut slot = rendered(vox_sample);
    slot.base_note = 64;
    slot.slices = starts[1..].to_vec();
    slot.slice_settings = [12, 16, 19, 24]
        .iter()
        .map(|&t| SliceSettings { transpose: t, oneshot: true, ..SliceSettings::default() })
        .collect();
    p.module_mut(chops).unwrap().samples = vec![slot];
    effects(p, chops, [(K::Gate, "Chop Gate", &[(0, 0.04), (2, 0.02), (3, 0.04)])]);
    send(p, chops, room);

    // Blips: short, bright FM, like a game's coins.
    let blip = add(p, K::Fm, "Coin Blip", &[(0, 0.3), (1, 3.0), (2, 3.0), (3, 0.05), (5, 0.001), (6, 0.09)]);
    send(p, blip, room);

    // The riser: noise swept up through a band.
    let riser = add(p, K::Generator, "Sugar Riser", &[(0, 0.45), (1, 4.0), (2, 2.0), (4, 1.0), (5, 0.4)]);
    let [sweep] = effects(p, riser, [(K::FilterPro, "Riser Sweep", &[(0, 2.0), (1, 400.0), (2, 4.0), (4, 1.0)])]);
    send(p, riser, room);

    // The pump: the wall and the guitar duck under the kit.
    let pump = super::add(p, K::Modulator, "Sugar Pump", &[(0, 1.0), (5, -0.7), (6, 0.003), (7, 0.16)]);
    let kit_end = p.chain(drums).effects.last().copied().unwrap_or(drums);
    p.connect(kit_end, pump);
    for target in [wall, guitar] {
        let fader = p.module(target).unwrap().kind.params().len();
        p.connect(pump, target);
        p.set_control_param(pump, target, fader);
    }

    Ids { drums, impact, bass, guitar, wall, wall_filter, vox, mouth, bells, chops, blip, riser, sweep, stutter, crush }
}

// ---------------------------------------------------------------- parts

fn pattern() -> Pattern {
    let mut pat = Pattern::new("", TRACKS, LINES);
    for (t, cols) in COLUMNS {
        pat.set_columns(t, cols);
    }
    pat
}

/// The guitar strummed through each bar, down on the beat and up off it,
/// a string a few ticks after the last (the delay column); with `eighths`
/// on every eighth, else a pop-punk pattern.
fn strums(pat: &mut Pattern, ids: &Ids, key: u8, vol: u8, eighths: bool) {
    let hits: &[usize] = if eighths { &[0, 2, 4, 6, 8, 10, 12, 14] } else { &[0, 3, 6, 8, 10, 12, 14] };
    for (bar, ch) in LOOP.iter().enumerate() {
        for &h in hits {
            let up = h % 4 != 0;
            for c in 0..4 {
                let string = if up { 3 - c } else { c };
                let v = if up { vol - 0x18 } else { vol };
                let cell = n(ch.tones[string] + key, ids.guitar, v);
                *pat.cell_mut(GUITAR, c, bar * BAR + h) = Cell { delay: Some(0x14 * c as u8), ..cell };
            }
        }
    }
}

/// Half-time trap drums: the kick, a snare on the third beat with a flam
/// (Dxx) before the next, and hats on the eighths that rattle into rolls
/// (Exx) faster and faster at the end of every other bar.
fn trap(pat: &mut Pattern, ids: &Ids) {
    for bar in 0..4 {
        let at = bar * BAR;
        pat.tracks[DRUMS][at] = n(48, ids.drums, 0x78);
        pat.tracks[DRUMS][at + 10] = n(48, ids.drums, 0x60);
        if bar % 2 == 1 {
            pat.tracks[DRUMS][at + 6] = n(48, ids.drums, 0x50);
        }
        pat.tracks[DRUMS][at + 8] = n(50, ids.drums, 0x78);
        pat.tracks[DRUMS][at + 15] = fx(n(50, ids.drums, 0x28), 0xD, 0x03);
        for l in (0..BAR).step_by(2) {
            pat.tracks[HATS][at + l] = n(54, ids.drums, if l % 4 == 0 { 0x48 } else { 0x34 });
        }
        if bar % 2 == 1 {
            for (l, rate) in [(12, 0x03), (13, 0x03), (14, 0x02), (15, 0x01)] {
                pat.tracks[HATS][at + l] = fx(n(54, ids.drums, 0x30 + (l as u8 - 12) * 8), 0xE, rate);
            }
        }
    }
}

/// Double-time drums for the drops: kicks pushing, snares on two and
/// four, sixteenth hats with rolls, and an open hat cut short (Cxx).
fn rave(pat: &mut Pattern, ids: &Ids) {
    for bar in 0..4 {
        let at = bar * BAR;
        for l in [0, 6, 8] {
            pat.tracks[DRUMS][at + l] = n(48, ids.drums, 0x7C);
        }
        if bar % 2 == 1 {
            pat.tracks[DRUMS][at + 11] = n(48, ids.drums, 0x60);
        }
        pat.tracks[DRUMS][at + 4] = n(50, ids.drums, 0x7C);
        pat.tracks[DRUMS][at + 12] = n(50, ids.drums, 0x7C);
        for l in 0..BAR {
            pat.tracks[HATS][at + l] = n(54, ids.drums, if l % 2 == 0 { 0x40 } else { 0x2C });
        }
        pat.tracks[HATS][at + 10] = fx(n(58, ids.drums, 0x50), 0xC, 0x03);
        if bar == 1 || bar == 3 {
            pat.tracks[HATS][at + 14] = fx(n(54, ids.drums, 0x48), 0xE, 0x02);
            pat.tracks[HATS][at + 15] = fx(n(54, ids.drums, 0x50), 0xE, 0x01);
        }
    }
    *pat.cell_mut(DRUMS, 1, 0) = n(30, ids.impact, 0x78);
}

/// The 808 on each bar's root, `hits` as (line, semitones up, glide into
/// it with 3xx).
fn bass(pat: &mut Pattern, ids: &Ids, key: u8, hits: &[(usize, u8, bool)]) {
    for (bar, ch) in LOOP.iter().enumerate() {
        for &(l, up, glide) in hits {
            let cell = n(ch.root + key + up, ids.bass, 0x70);
            pat.tracks[BASS][bar * BAR + l] = if glide { fx(cell, 0x3, 0x40) } else { cell };
        }
    }
}

/// The wall's chords, a bar each: the root an octave up and the four
/// tones.
fn wall(pat: &mut Pattern, ids: &Ids, key: u8, vol: u8) {
    for (bar, ch) in LOOP.iter().enumerate() {
        *pat.cell_mut(WALL, 0, bar * BAR) = n(ch.root + 24 + key, ids.wall, vol);
        for (c, &tone) in ch.tones.iter().enumerate() {
            *pat.cell_mut(WALL, c + 1, bar * BAR) = n(tone + 12 + key, ids.wall, vol);
        }
    }
}

/// The bells in sixteenths over the chord's top notes, their first of each
/// beat made a chord (0xy), panned from side to side (8xx) and cut short.
fn bells(pat: &mut Pattern, ids: &Ids, key: u8, vol: u8) {
    for (bar, ch) in LOOP.iter().enumerate() {
        let notes = [ch.tones[3] + 12, ch.tones[2] + 12, ch.tones[1] + 12, ch.tones[2] + 12];
        for l in 0..BAR {
            let cell = n(notes[l % 4] + key, ids.bells, if l % 4 == 0 { vol } else { vol - 0x18 });
            pat.tracks[BELLS][bar * BAR + l] = match l % 4 {
                0 => fx(cell, 0x0, ch.arp),
                2 => fx(cell, 0x8, if l % 8 == 2 { 0x30 } else { 0xD0 }),
                _ => fx(cell, 0xC, 0x02),
            };
        }
    }
}

/// The voice singing `line` (line, note, lines) an octave below where it
/// sounds, snapping into leaps with a fast glide (3xx); with `harmony` a
/// third below in its second column.
fn sing(pat: &mut Pattern, ids: &Ids, key: u8, line: &[(usize, u8, usize)], vol: u8, harmony: bool) {
    let mut last = 0;
    for &(l, note, _) in line {
        let mut cell = n(note - 12 + key, ids.vox, vol);
        if note.abs_diff(last) >= 3 && last != 0 {
            cell = fx(cell, 0x3, 0xC0);
        }
        pat.tracks[VOX][l] = cell;
        if harmony {
            *pat.cell_mut(VOX, 1, l) = n(third_below(note) - 12 + key, ids.vox, vol - 0x20);
        }
        last = note;
    }
    let (end, _, len) = line[line.len() - 1];
    if end + len < LINES {
        pat.tracks[VOX][end + len] = off();
        *pat.cell_mut(VOX, 1, end + len) = off();
    }
}

/// The hook as (line, note, lines), from its eighth notes.
fn hook() -> Vec<(usize, u8, usize)> {
    let mut line: Vec<(usize, u8, usize)> = Vec::new();
    for (k, &note) in HOOK.iter().enumerate() {
        match (note, line.last_mut()) {
            (0, Some(last)) => last.2 += 2,
            (0, None) => {}
            (note, _) => line.push((k * 2, note, 2)),
        }
    }
    line
}

/// A voice chop, slice `slice` (0 to 3) as a note above the sample's base.
fn chop(ids: &Ids, slice: u8, vol: u8) -> Cell {
    n(65 + slice, ids.chops, vol)
}

/// The riser over the last `bars`, its band sweeping up, let go at the end.
fn riser(pat: &mut Pattern, ids: &Ids, bars: usize) {
    let from = LINES - bars * BAR;
    pat.tracks[FX][from] = n(60, ids.riser, 0x70);
    pat.tracks[FX][LINES - 1] = off();
    let f = ModuleKind::FilterPro;
    let sweep = [(0.0, 300.0), (from as f32, 300.0), (LINES as f32, 10000.0)];
    pat.automation.push(envelope(ids.sweep, f, 1, &sweep, false, true));
}

/// The tempo at lines, as (line, BPM), in the second FX column.
fn tempo(pat: &mut Pattern, bpms: &[(usize, u8)]) {
    for &(l, bpm) in bpms {
        pat.cell_mut(FX, 1, l).fx = Some((0xF, bpm));
    }
}

/// The Stutter on the master holding during `holds` (from, to), at the
/// lengths `lengths` say: (line, index into `REPEATER_LENGTHS`).
fn stutter(pat: &mut Pattern, ids: &Ids, holds: &[(f32, f32)], lengths: &[(f32, f32)]) {
    hold(pat, ids.stutter, holds, lengths);
}

// ---------------------------------------------------------------- sections

/// The spin-up: from 80 BPM to the song's 155 a bar at a time, bells
/// swelling in (Lxx), coins maybe there (Yxx), chops calling from side to
/// side, and the riser.
fn boot(ids: &Ids) -> Pattern {
    let mut pat = pattern();
    tempo(&mut pat, &[(0, 0x50), (16, 0x6E), (32, 0x8C), (48, 0x9B)]);
    bells(&mut pat, ids, 0, 0x60);
    for (bar, l) in (0..LINES).step_by(BAR).enumerate() {
        pat.tracks[BELLS][l + 1].fx = Some((FX_TRACK_VOLUME, 0x20 + bar as u8 * 0x20));
    }
    let coins = [88, 95, 100, 92, 104, 97];
    for (k, l) in (2..48).step_by(5).enumerate() {
        pat.tracks[FX][l] = fx(n(coins[k % coins.len()], ids.blip, 0x40), FX_MAYBE, 0xA0);
    }
    for (l, slice, pan) in [(8, 0, 0x20), (24, 1, 0xE0), (40, 2, 0x40), (52, 3, 0xC0)] {
        pat.tracks[CHOPS][l] = fx(chop(ids, slice, 0x60), 0x8, pan);
    }
    // "ah-ah-ah-ah": the last chop played again every three ticks.
    pat.tracks[CHOPS][60] = fx(chop(ids, 0, 0x60), 0xE, 0x03);
    riser(&mut pat, ids, 1);
    pat
}

/// A verse: the guitar and trap drums, the 808, and the voice; the second
/// time with a harmony, and chops answering, one of them backwards (Rxx).
fn verse(ids: &Ids, second: bool) -> Pattern {
    let mut pat = pattern();
    strums(&mut pat, ids, 0, 0x60, false);
    trap(&mut pat, ids);
    bass(&mut pat, ids, 0, &[(0, 0, false), (10, 0, false), (14, 12, true)]);
    sing(&mut pat, ids, 0, &VERSE, 0x78, second);
    // Stutters on the long notes, a gate in ticks (Txy).
    for l in [9, 25, 41] {
        pat.tracks[VOX][l].fx = Some((FX_TREMOR, 0x21));
    }
    if second {
        for (k, l) in [14, 30, 46].into_iter().enumerate() {
            pat.tracks[CHOPS][l] = fx(chop(ids, k as u8, 0x58), 0x8, 0x40 + k as u8 * 0x40);
        }
        pat.tracks[CHOPS][62] = fx(chop(ids, 3, 0x60), FX_REVERSE, 0x01);
    }
    pat
}

/// The build: eighth strums, the wall rising out of muffle, kicks on every
/// beat and a snare tightening into a roll, the 808 sliding up (1xx) and
/// the voice rising on "ah", then a last bar cut to silence but for a
/// stuttered chop.
fn pre(ids: &Ids) -> Pattern {
    let mut pat = pattern();
    strums(&mut pat, ids, 0, 0x58, true);
    wall(&mut pat, ids, 0, 0x50);
    for l in (0..48).step_by(LPB) {
        pat.tracks[DRUMS][l] = n(48, ids.drums, 0x68);
    }
    for (k, l) in (16..48).step_by(2).enumerate() {
        let rate = [0x06, 0x03, 0x02][k / 6];
        pat.tracks[DRUMS][l + 1] = fx(n(50, ids.drums, 0x30 + k as u8 * 3), 0xE, rate);
    }
    pat.tracks[BASS][0] = n(28, ids.bass, 0x68);
    pat.tracks[BASS][32] = fx(n(28, ids.bass, 0x68), 0x1, 0x03);
    for l in 33..48 {
        pat.tracks[BASS][l].fx = Some((0x1, 0x03));
    }
    pat.tracks[VOX][32] = fx(n(59, ids.vox, 0x58), 0x1, 0x01);
    for l in 33..48 {
        pat.tracks[VOX][l].fx = Some((0x1, 0x01));
    }
    // The cut: everything let go on the last bar.
    for (t, cols) in [(GUITAR, 4), (WALL, 5), (BASS, 1), (VOX, 1)] {
        for c in 0..cols {
            *pat.cell_mut(t, c, 48) = off();
        }
    }
    for l in 48..LINES {
        for c in 0..4 {
            *pat.cell_mut(GUITAR, c, l) = Cell::default();
        }
    }
    for l in [52, 56] {
        pat.tracks[CHOPS][l] = fx(chop(ids, 0, 0x68), 0xE, 0x02);
    }
    pat.tracks[CHOPS][60] = fx(chop(ids, 3, 0x70), 0xE, 0x01);
    riser(&mut pat, ids, 3);
    let flt = ModuleKind::Filter;
    pat.automation.push(envelope(ids.wall_filter, flt, 1, &[(0.0, 300.0), (48.0, 9000.0)], false, true));
    pat
}

/// A drop: the wall pumping, the 808 bouncing in octaves, double-time
/// drums, the voice on the hook and the bells over it. The second time the
/// harmony comes in, chops answer the hook, and the mix stutters at the
/// end of each phrase.
fn drop(ids: &Ids, key: u8, second: bool) -> Pattern {
    let mut pat = pattern();
    wall(&mut pat, ids, key, 0x70);
    rave(&mut pat, ids);
    bass(
        &mut pat,
        ids,
        key,
        &[(0, 0, false), (3, 0, false), (6, 12, true), (8, 0, true), (11, 0, false), (14, 7, true)],
    );
    sing(&mut pat, ids, key, &hook(), 0x70, second);
    bells(&mut pat, ids, key, 0x58);
    let v = ModuleKind::VocalFilter;
    let vowels = [(0.0, 0.0), (12.0, 2.0), (24.0, 1.0), (40.0, 3.0), (56.0, 0.0), (64.0, 2.0)];
    pat.automation.push(envelope(ids.mouth, v, 0, &vowels, false, true));
    if second {
        for (k, l) in [15, 31, 47].into_iter().enumerate() {
            pat.tracks[CHOPS][l] = fx(chop(ids, k as u8 + 1, 0x60), 0x9, 0x40);
        }
        stutter(&mut pat, ids, &[(28.0, 32.0), (60.0, 64.0)], &[(0.0, 4.0), (30.0, 6.0), (60.0, 5.0), (62.0, 8.0)]);
    }
    pat
}

/// The melt: slowed to 104 BPM, the guitar and the voice alone in the
/// room, the guitar sinking (2xx); in the last bar the tempo spins up beat
/// by beat to 180 under a snare roll and the riser.
fn melt(ids: &Ids) -> Pattern {
    let mut pat = pattern();
    for c in 0..5 {
        *pat.cell_mut(WALL, c, 0) = off();
    }
    pat.tracks[BASS][0] = off();
    tempo(&mut pat, &[(0, 0x68), (48, 0x80), (52, 0x98), (56, 0xA8), (60, 0xB4)]);
    strums(&mut pat, ids, 0, 0x50, false);
    sing(&mut pat, ids, 0, &VERSE[..12], 0x60, true);
    for l in 40..48 {
        for c in 0..4 {
            pat.cell_mut(GUITAR, c, l).fx = Some((0x2, 0x01));
        }
    }
    for l in 48..LINES {
        for c in 0..4 {
            *pat.cell_mut(GUITAR, c, l) = Cell::default();
        }
        pat.tracks[DRUMS][l] = fx(n(50, ids.drums, 0x40 + (l as u8 - 48) * 3), 0xE, if l < 56 { 0x03 } else { 0x02 });
    }
    *pat.cell_mut(GUITAR, 0, 48) = off();
    riser(&mut pat, ids, 1);
    pat
}

/// The crash: two bars of the drop, then the mix freezes in ever shorter
/// loops, crushes down to two bits, slows like a tape (Fxx) and fades
/// (Axy), and F00 ends it.
fn crash(ids: &Ids) -> Pattern {
    let mut pat = drop(ids, 1, true);
    pat.automation.retain(|e| e.module != ids.stutter);
    for l in 32..LINES {
        for t in [DRUMS, HATS, BASS, BELLS, CHOPS] {
            pat.tracks[t][l] = Cell::default();
        }
        *pat.cell_mut(VOX, 1, l) = Cell::default();
        pat.tracks[VOX][l] = Cell::default();
    }
    pat.tracks[VOX][32] = off();
    *pat.cell_mut(VOX, 1, 32) = off();
    stutter(&mut pat, ids, &[(32.0, 64.0)], &[(32.0, 2.0), (40.0, 4.0), (48.0, 6.0), (56.0, 8.0)]);
    let d = ModuleKind::Distortion;
    pat.automation.push(envelope(ids.crush, d, 4, &[(0.0, 16.0), (40.0, 16.0), (56.0, 3.0), (64.0, 2.0)], true, false));
    tempo(&mut pat, &[(48, 0x90), (52, 0x70), (56, 0x50), (60, 0x34)]);
    for l in 48..LINES {
        for c in 0..5 {
            pat.cell_mut(WALL, c, l).fx = Some((0xA, 0x02));
        }
    }
    pat.cell_mut(FX, 1, LINES - 1).fx = Some((0xF, 0x00));
    pat
}

impl Project {
    /// "Bubblegum Overclock", the second demo song.
    pub fn bubblegum_overclock() -> Self {
        let mut p = Self::empty();
        p.title = "Bubblegum Overclock".into();
        p.artist = "noise".into();
        p.comments = COMMENTS.into();
        p.bpm = 155.0;
        p.lpb = LPB as u32;
        p.modules[0].params[0] = 0.8;
        let names = ["Drums", "Hats", "808", "Guitar", "Wall", "Vox", "Bells", "Chops", "FX"];
        let colors = [
            [255, 90, 170],
            [255, 170, 220],
            [170, 80, 255],
            [255, 200, 120],
            [100, 200, 255],
            [255, 130, 230],
            [140, 255, 230],
            [255, 230, 90],
            [210, 210, 210],
        ];
        for (t, (name, color)) in names.iter().zip(colors).enumerate() {
            p.tracks[t].name = name.to_string();
            p.tracks[t].color = Some(color);
        }
        p.tracks[GUITAR].show_delay = true;
        let ids = instruments(&mut p);
        let mut patterns = vec![
            boot(&ids),
            verse(&ids, false),
            verse(&ids, true),
            pre(&ids),
            drop(&ids, 0, false),
            drop(&ids, 0, true),
            melt(&ids),
            drop(&ids, 1, false),
            drop(&ids, 1, true),
            crash(&ids),
        ];
        let names = ["Boot", "Verse", "Verse 2", "Pre", "Drop", "Drop 2", "Melt", "Up", "Up 2", "Crash"];
        for (pat, name) in patterns.iter_mut().zip(names) {
            pat.name = name.into();
        }
        p.patterns = patterns;
        for (t, cols) in COLUMNS {
            p.set_columns(t, cols);
        }
        p.order = (0..10).map(Slot::new).collect();
        p.sections = [("Boot", 0), ("Verse", 1), ("Pre", 3), ("Drop", 4), ("Melt", 6), ("Up", 7), ("Crash", 9)]
            .into_iter()
            .map(|(name, start)| Section { start, name: name.into() })
            .collect();
        // Its shared room becomes a send track.
        p.untangle();
        p
    }
}

const COMMENTS: &str = "Bubblegum Overclock — the second demo song: hyperpop in E major, 155 BPM and then 180.\n\n\
    It spins up to tempo, strums an acoustic guitar over trap drums with a pitched-up voice, cuts to silence, \
    drops into clipped supersaws, melts down slow, spins back up a semitone higher, and crashes.\n\n\
    Where to look:\n\
    • Pop Vox: a MultiSynth singing through a Vocal Filter (its vowel automated in the drops) and a Pitch Shifter \
    an octave up, snapping into leaps with a fast 3xx; Txy stutters its long notes, and the second verse and \
    drops add a harmony in its second column.\n\
    • Emo Guitar: a Plucked String, three strings a note, strummed down and up with the delay column.\n\
    • 808: a Kicker gliding with 3xx and sliding up with 1xx, rounded by a WaveShaper and driven.\n\
    • Supersaw Wall: four detuned saws clipped, chorused and spread; Sugar Pump ducks it and the guitar under the \
    kit.\n\
    • Sparkle Bells: FM in sixteenths, chords made with 0xy, panned with 8xx and cut with Cxx.\n\
    • Vox Chops: the syllables as slices tuned up an octave onto the chord; Exx stutters them, 9xx starts one \
    partway in, 8xx pans them and R01 plays one backwards.\n\
    • Hats roll with Exx, faster at the end of every other bar; the snare flams with Dxx.\n\
    • Boot and Melt change tempo with Fxx; the bells swell in with Lxx and the coins are left to chance with Yxx.\n\
    • Crash: Stutter, a Repeater on the master, freezes the mix in shorter and shorter loops, Overclock crushes it \
    to two bits, Fxx slows it like a tape, Axy fades the wall and F00 ends it.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bubblegum_overclock_uses_its_effects_and_reaches_the_output() {
        let p = Project::bubblegum_overclock();
        let mut commands: Vec<u8> = Vec::new();
        for pat in &p.patterns {
            for l in 0..pat.num_lanes() {
                commands.extend(pat.lane(l).iter().filter_map(|c| c.fx.map(|f| f.0)));
            }
        }
        let wanted = [0x0, 0x1, 0x2, 0x3, 0x8, 0x9, 0xA, 0xC, 0xD, 0xE, 0xF, FX_MAYBE, FX_REVERSE, FX_TREMOR];
        for cmd in wanted.into_iter().chain([FX_TRACK_VOLUME]) {
            assert!(commands.contains(&cmd), "effect {:?}", char::from_digit(cmd as u32, 36));
        }
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
        let chops = p.modules.iter().find(|m| m.kind == ModuleKind::Sampler).unwrap();
        assert!(chops.samples[0].data.is_some() && chops.samples[0].slice_ranges().len() >= 4);
        assert_eq!(third_below(71), 68, "B to G#");
        assert_eq!(third_below(76), 73, "E to C#");
    }

    #[test]
    fn bubblegum_overclock_plays_loud_enough_without_clipping_and_ends() {
        let frames = crate::audio::render(std::sync::Arc::new(Project::bubblegum_overclock()), 8000, 1.0, false);
        let peak = frames.iter().fold(0f32, |a, f| a.max(f[0].abs()).max(f[1].abs()));
        assert!(peak > 0.5 && peak <= 1.0, "{peak}");
        let secs = frames.len() as f32 / 8000.0;
        assert!((50.0..110.0).contains(&secs), "{secs} s");
    }
}
