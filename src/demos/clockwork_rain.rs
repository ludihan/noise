//! "Clockwork Rain", the third demo song: a minute and a half of IDM in E
//! minor at 172 BPM, a music box over a breakbeat cut up every way a
//! tracker can cut one.
//!
//! A key winds a music box, which plays alone over a record's crackle; a
//! break comes in through a filter and is chopped apart under electric
//! piano chords, a sub and an acid line; a fretless bass runs through jazz
//! changes; the drill rolls, stumbles in triplets and fives, buzzes and
//! dives; it unwinds into fog and rain, limps along in 7/8, then storms
//! with gabber kicks, gated strings and a crying lead, faster and faster,
//! until the whole mix sticks in a loop and leaves the music box to run
//! down.
//!
//! It runs at eight lines a beat, so a line is a 32nd note. The break and
//! the crackle are sampled here, so its Samplers have audio without files;
//! they are written next to the song when it is saved.

use super::Hit::{self, *};
use super::{Rng, effects, envelope, fx, hit, hold, let_go, n, normalize, off, rendered, roll, set, tune};
use crate::dsp::Frame;
use crate::project::*;
use crate::sample::Sample;

const BPM: f32 = 172.0;
/// Lines a beat: a line is a 32nd note.
const LPB: usize = 8;
/// Lines a bar of 4/4, a pattern of four of them, and a bar of 7/8.
const BAR: usize = 4 * LPB;
const LINES: usize = 4 * BAR;
const BAR7: usize = 7 * LPB / 2;

// ---------------------------------------------------------------- harmony

/// A chord: the sub's root and the electric piano's four notes.
#[derive(Clone, Copy)]
struct Chord {
    root: u8,
    keys: [u8; 4],
}

const fn chord(root: u8, keys: [u8; 4]) -> Chord {
    Chord { root, keys }
}

// MIDI numbers: 48 is the tracker's C-4.
const EM9: Chord = chord(28, [55, 59, 62, 66]);
const CMAJ9: Chord = chord(36, [55, 59, 62, 64]);
const AM9: Chord = chord(33, [55, 59, 60, 64]);
const B7: Chord = chord(35, [57, 60, 63, 66]);
const AM11: Chord = chord(33, [55, 60, 62, 64]);
const D13: Chord = chord(38, [54, 59, 60, 64]);
const GMAJ9: Chord = chord(31, [54, 57, 59, 62]);
const FS7ALT: Chord = chord(30, [52, 55, 58, 62]);
const CMAJ7S11: Chord = chord(36, [54, 59, 64, 67]);
const BM11: Chord = chord(35, [54, 57, 62, 64]);

/// The song's four chords, a bar each; the jazz changes; and the 7/8's.
const LOOP: [Chord; 4] = [EM9, CMAJ9, AM9, B7];
const JAZZ: [Chord; 4] = [AM11, D13, GMAJ9, FS7ALT];
const SEVEN: [Chord; 4] = [CMAJ7S11, BM11, AM9, BM11];

/// The strings' chords over `LOOP`.
const STRINGS_LOOP: [[u8; 3]; 4] = [[64, 67, 71], [64, 67, 72], [64, 69, 72], [63, 66, 71]];

/// The acid line's notes over each chord of `LOOP`: root, third, fifth,
/// seventh and octave.
const ACID_TONES: [[u8; 5]; 4] =
    [[52, 55, 59, 62, 64], [48, 52, 55, 59, 60], [45, 48, 52, 55, 57], [47, 51, 54, 57, 59]];

/// A bar of the acid line, a step a sixteenth: which of the chord's notes
/// it plays (see `ACID_TONES`), or a rest; the steps it accents, and those
/// it slides into from the step before, as a 303 would.
const ACID_BAR: [Option<usize>; 16] = [
    Some(0),
    Some(0),
    Some(4),
    Some(0),
    None,
    Some(1),
    Some(0),
    Some(3),
    None,
    Some(2),
    Some(0),
    Some(4),
    None,
    Some(3),
    Some(2),
    Some(1),
];
const ACCENTS: [usize; 5] = [0, 3, 6, 9, 13];
const SLIDES: [usize; 4] = [2, 7, 11, 15];

/// The music box's tune over `LOOP`: (line, note).
const THEME: [(usize, u8); 20] = [
    (0, 76),
    (4, 83),
    (8, 88),
    (16, 86),
    (20, 83),
    (28, 78),
    (32, 79),
    (40, 76),
    (48, 83),
    (60, 78),
    (64, 84),
    (72, 83),
    (80, 81),
    (84, 88),
    (92, 86),
    (96, 84),
    (100, 83),
    (104, 81),
    (112, 78),
    (120, 75),
];

/// Its answer, higher and busier.
const ANSWER: [(usize, u8); 26] = [
    (0, 79),
    (4, 83),
    (8, 86),
    (12, 90),
    (16, 88),
    (24, 83),
    (28, 86),
    (32, 88),
    (40, 84),
    (44, 83),
    (48, 79),
    (56, 83),
    (64, 84),
    (68, 88),
    (72, 91),
    (80, 90),
    (84, 88),
    (88, 86),
    (92, 84),
    (96, 83),
    (104, 87),
    (108, 84),
    (112, 83),
    (116, 81),
    (120, 78),
    (124, 75),
];

/// The music box's clockwork in 7/8: seven eighths, round and round.
const CLOCKWORK: [u8; 7] = [88, 83, 79, 83, 86, 83, 78];

/// The fretless's run through `JAZZ`: (line, note, glide into it).
const RUN: &[(usize, u8, bool)] = &[
    // A minor 11.
    (0, 33, false),
    (6, 40, false),
    (8, 43, true),
    (10, 45, false),
    (12, 48, false),
    (14, 50, true),
    (16, 52, false),
    (20, 50, false),
    (22, 48, false),
    (24, 45, false),
    (26, 43, false),
    (28, 40, false),
    (30, 36, false),
    // D13.
    (32, 38, true),
    (38, 45, false),
    (40, 48, false),
    (42, 47, false),
    (44, 45, false),
    (46, 42, false),
    (48, 40, false),
    (50, 42, true),
    (52, 45, false),
    (54, 47, false),
    (56, 48, false),
    (58, 50, false),
    (60, 54, true),
    (62, 52, false),
    // G major 9.
    (64, 43, false),
    (68, 50, false),
    (70, 54, false),
    (72, 57, true),
    (74, 59, false),
    (76, 57, false),
    (78, 54, false),
    (80, 50, false),
    (82, 47, false),
    (84, 45, false),
    (86, 43, false),
    (88, 42, false),
    (90, 38, false),
    (92, 35, false),
    (94, 31, true),
    // F#7, altered.
    (96, 30, false),
    (102, 34, false),
    (104, 36, false),
    (106, 38, false),
    (108, 40, false),
    (110, 43, false),
    (112, 46, true),
    (114, 48, false),
    (116, 50, false),
    (118, 52, false),
    (120, 50, false),
    (122, 46, false),
    (124, 43, false),
    (126, 39, true),
];

/// The fretless's riff in each bar of 7/8, an eighth a note.
const SEVEN_RIFF: [[u8; 7]; 4] = [
    [36, 48, 43, 47, 36, 40, 43],
    [35, 47, 42, 45, 35, 38, 42],
    [33, 45, 40, 43, 33, 36, 40],
    [35, 47, 42, 45, 47, 50, 54],
];

// ---------------------------------------------------------------- the break

/// The break: two bars of sixteenths, as (step, hit, how hard).
const BREAK: &[(usize, Hit, f32)] = &[
    (0, Kick, 1.0),
    (0, Hat, 0.5),
    (2, Hat, 0.4),
    (4, Snare, 1.0),
    (4, Hat, 0.35),
    (6, Hat, 0.45),
    (7, Ghost, 0.45),
    (9, Ghost, 0.35),
    (10, Kick, 0.85),
    (10, Hat, 0.45),
    (12, Snare, 0.95),
    (12, Hat, 0.35),
    (14, Open, 0.5),
    (15, Kick, 0.55),
    (16, Kick, 1.0),
    (16, Hat, 0.5),
    (18, Kick, 0.8),
    (20, Snare, 1.0),
    (20, Hat, 0.35),
    (22, Hat, 0.45),
    (23, Ghost, 0.4),
    (25, Kick, 0.75),
    (26, Snare, 0.9),
    (28, Rim, 0.8),
    (30, Snare, 0.85),
    (30, Hat, 0.35),
    (31, Ghost, 0.35),
];

/// The sixteenths where the break's slices start: at each of its hits, then
/// at the reversed snare, the big snare and the crash after its two bars.
const STARTS: [usize; 23] = [0, 2, 4, 6, 7, 9, 10, 12, 14, 15, 16, 18, 20, 22, 23, 25, 26, 28, 30, 31, 32, 36, 40];
/// The whole sample's length in sixteenths.
const STEPS: usize = 48;

/// The note that plays the slice starting at sixteenth `step`.
const fn slice(step: usize) -> u8 {
    let mut i = 0;
    while STARTS[i] != step {
        i += 1;
    }
    49 + i as u8
}

const KICK: u8 = slice(0);
const HAT: u8 = slice(2);
const SNARE: u8 = slice(4);
const GHOST: u8 = slice(7);
const SNARE2: u8 = slice(12);
const OPEN: u8 = slice(14);
const PICKUP: u8 = slice(15);
const DRY_KICK: u8 = slice(18);
const SNARE3: u8 = slice(20);
const GHOST2: u8 = slice(23);
const DRY_SNARE: u8 = slice(26);
const RIM: u8 = slice(28);
const REVERSE: u8 = slice(32);
const BIG: u8 = slice(36);
const CRASH: u8 = slice(40);

/// The break's slice each letter of a grid plays: kicks (K with a hat, k
/// dry, q soft), snares (S, s and T with a hat, t dry, r a rim), ghost
/// notes (g, G), hats (h, o open), and after the break the reversed snare
/// (R), the big snare (B) and the crash (C).
fn slice_of(c: char) -> Option<u8> {
    Some(match c {
        'K' => KICK,
        'k' => DRY_KICK,
        'q' => PICKUP,
        'S' => SNARE,
        's' => SNARE2,
        'T' => SNARE3,
        't' => DRY_SNARE,
        'r' => RIM,
        'g' => GHOST,
        'G' => GHOST2,
        'h' => HAT,
        'o' => OPEN,
        'R' => REVERSE,
        'B' => BIG,
        'C' => CRASH,
        _ => return None,
    })
}

// The grids the break is chopped by: a letter a sixteenth, or a 32nd.
const RAIN: &str = "K.h.S.hg.gK.s.oq K.k.T.hG.kt.r.Sg K.h.S.hgK.K.s.hq K.k.T.hGk.t.SSSS";
const RAIN_2: &str = "K...h.g. S..gS.g. K.g.k... S...h.gg  K.k.h..g T..g.gT. k.k.K... t.r.tSSS  \
                      K...h.g. S..gS.g. K.g.k.g. SgSgSgSg  K..gk..g T....... ........ R.......";
const SWING: &str = "K.hgS.hg.gK.s.hg K.hgS.hgk.k.T.hG K.hgS.hg.gK.s.og K.hgS.hgk.T.r.SG";
const SWING_2: &str = "K..gh.gg S..gh.g. ..gK..g. s..gh.gg  K..gh.gg S..gh.g. k.k.T..g ..G.S.SS  \
                       K..gh.gg S..gh.g. ..gK..g. s..gh.gg  K..gh.gg S..gk.k. T.T.T.TT TTTTTTTT";
const LIMP: &str = "K.hgS.h.K.k.Sg K.hgS.hgK.K.SS K.hgS.h.k.k.Sr K.hgS.hgK.SSSS";
const STORM: &str = "K.g.S.gS .gK.S..g K.g.S.gS kgK.SSSS  K.h.T.gT ..k.TgTg K.k.T.r. tSStSSrS  \
                     K.g.S.gS .gK.S..g K.g.S.gS SSSSSSSS  K.k.T.gT C....... k.k.k.k. TTTTTTTT";
const STORM_2: &str = "K.gSS.gS .gK.SgSg K.gSS.gS kgKgSSSS  K.h.T.gT ..k.TgTg K.k.T.r. tSStSSrS  \
                       K.gSS.gS .gK.SgSg K.gSS.gS SSSSSSSS  K.k.T.gT K.k.T.gT TTTTTTTT TTTTTTTT";

// ---------------------------------------------------------------- samples

/// Two bars of breakbeat at 172 BPM and, after them, a reversed snare, a
/// snare in a big room and a crash, a little saturated and crunched as if
/// sampled at twelve bits. Returns it and how long a sixteenth is.
fn break_sample(sr: f32) -> (Sample, usize) {
    let step = (sr * 60.0 / BPM / 4.0) as usize;
    let mut frames = vec![[0.0f32; 2]; step * STEPS];
    // Adds `sound` from sixteenth `at`, up to sixteenth `end`.
    let put = |frames: &mut [Frame], at: usize, end: usize, sound: &[f32], gain: f32, pan: f32| {
        for (f, &x) in frames[at * step..end * step].iter_mut().zip(sound) {
            f[0] += x * gain * (1.0 - pan);
            f[1] += x * gain * (1.0 + pan);
        }
    };
    for (k, &(at, kind, vel)) in BREAK.iter().enumerate() {
        let pan = match kind {
            Hat => 0.15,
            Open => -0.15,
            Rim => 0.2,
            _ => 0.0,
        };
        put(&mut frames, at, 32, &hit(sr, kind, 0x5EED_0000 + k as u32, 0.2), vel, pan);
    }
    // The reversed snare swells into the sixteenth after its four, out of
    // nothing.
    let mut swell = hit(sr, Snare, 0xBAC4, 0.4);
    swell.truncate(4 * step);
    swell.reverse();
    let len = swell.len() as f32;
    swell.iter_mut().enumerate().for_each(|(k, x)| *x *= (k as f32 / len * 3.0).min(1.0));
    put(&mut frames, 32, 36, &swell, 0.9, 0.0);
    put(&mut frames, 36, 40, &hit(sr, Snare, 0xB16, 0.45), 1.0, 0.0);
    put(&mut frames, 40, 48, &hit(sr, Crash, 0xC2A5, 0.0), 0.8, 0.1);
    // The big snare and the crash fade out by the ends of their slices.
    for end in [40, 48] {
        let fade = step * 4 / 3;
        for (k, f) in frames[end * step - fade..end * step].iter_mut().enumerate() {
            let g = 1.0 - k as f32 / fade as f32;
            *f = [f[0] * g, f[1] * g];
        }
    }
    // Saturated, darkened and crunched to twelve bits.
    let mut lp = [0.0f32; 2];
    for f in &mut frames {
        for ch in 0..2 {
            lp[ch] += 0.7 * ((f[ch] * 1.5).tanh() - lp[ch]);
            f[ch] = (lp[ch] * 2048.0).round() / 2048.0;
        }
    }
    normalize(&mut frames, 0.9);
    (Sample { name: "Rain Break (172)".into(), sample_rate: sr, channels: 2, frames }, step)
}

/// Two seconds of a worn record, hiss and crackle, to loop under the song.
fn dust(sr: f32) -> Sample {
    let len = (sr * 2.0) as usize;
    let mut rng = Rng(0xD057_D057);
    let mut lp = [0.0f32; 2];
    let mut frames: Vec<Frame> = (0..len)
        .map(|_| {
            let mut f = [0.0; 2];
            for ch in 0..2 {
                lp[ch] += 0.2 * (rng.next() - lp[ch]);
                f[ch] = lp[ch] * 0.05;
            }
            f
        })
        .collect();
    // Pops: mostly small, now and then a loud one, each a millisecond or so.
    for _ in 0..60 {
        let at = (rng.next().abs() * (len - 500) as f32) as usize;
        let size = 0.05 + 0.9 * rng.next().abs().powi(4);
        let decay = sr * (0.0002 + 0.001 * rng.next().abs());
        let pan = rng.next() * 0.7;
        for (k, f) in frames[at..at + (decay * 5.0) as usize].iter_mut().enumerate() {
            let v = rng.next() * size * (-(k as f32) / decay).exp();
            f[0] += v * (1.0 - pan);
            f[1] += v * (1.0 + pan);
        }
    }
    normalize(&mut frames, 0.5);
    Sample { name: "Dust".into(), sample_rate: sr, channels: 2, frames }
}

// ---------------------------------------------------------------- tracks

const BEAT: usize = 0;
const CHOPS: usize = 1;
const HAMMER: usize = 2;
const SUB: usize = 3;
const ACID: usize = 4;
const BASS: usize = 5;
const KEYS: usize = 6;
const BOX: usize = 7;
const STRINGS: usize = 8;
const FOG: usize = 9;
const LEAD: usize = 10;
const FX: usize = 11;
const TRACKS: usize = 12;

/// The tracks with more than one note column.
const COLUMNS: [(usize, usize); 6] = [(BEAT, 2), (KEYS, 4), (BOX, 2), (STRINGS, 3), (FOG, 3), (FX, 2)];
/// The Beat track's effect column, after its two note columns: slides
/// under its rolls.
const BEAT_FX: usize = 2;

/// The instruments and effects patterns refer to.
struct Ids {
    brk: u8,
    break_filter: u8,
    stutter: u8,
    dust: u8,
    hammer: u8,
    sub: u8,
    acid: u8,
    ladder: u8,
    bass: u8,
    keys: u8,
    music_box: u8,
    strings: u8,
    gate: u8,
    fog: u8,
    lead: u8,
    data: u8,
    room: u8,
    freeze: u8,
}

/// A drawn shape for a gate: open (half way up, where it leaves what it
/// moves alone) on each x of `steps` and shut on each dot.
fn gate_shape(steps: &str) -> Vec<(f32, f32)> {
    let n = steps.len() as f32;
    let mut points: Vec<(f32, f32)> = steps
        .chars()
        .enumerate()
        .flat_map(|(k, c)| {
            let v = if c == 'x' { 0.5 } else { 0.0 };
            [(k as f32 / n, v), ((k as f32 + 0.9) / n, v)]
        })
        .collect();
    points.push((1.0, points[0].1));
    points
}

/// A phrase of the break's slices, spelled as `chops` spells them, at
/// `lpb` lines a beat, louder towards its end.
fn break_phrase(lpb: u32, grid: &str) -> Phrase {
    let steps: Vec<char> = grid.chars().filter(|c| *c != ' ').collect();
    let mut ph = Phrase { lines: steps.len(), lpb, ..Phrase::default() };
    for (l, &c) in steps.iter().enumerate() {
        if let Some(note) = slice_of(c) {
            let vol = 0x50 + (0x30 * l / steps.len()) as u8;
            ph.cells[l] = Cell { note: Some(Note::On(note)), vol: Some(vol), ..Cell::default() };
        }
    }
    ph
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

    // The room most things go through, its lows and fizz taken out.
    let room = add(p, K::Reverb, "Room", &[(0, 0.8), (1, 0.5), (2, 0.28)]);
    p.connect(room, OUTPUT_ID);
    effects(p, room, [(K::Eq10, "Room Tone", &[(0, 0.25), (1, 0.35), (2, 0.6), (8, 0.8), (9, 0.6)])]);

    // The master: Freeze, a Repeater that sticks the mix in a loop at the
    // end, then glue, a tilt and a limiter. The mix comes in a little down,
    // so the limiter has less to do.
    let [freeze, ..] = effects(
        p,
        OUTPUT_ID,
        [
            (K::Repeater, "Freeze", &[]),
            (K::Compressor, "Bus Glue", &[(0, 0.4), (1, 2.5), (2, 0.008), (3, 0.15), (4, 1.2)]),
            (K::Eq5, "Tilt", &[(0, 70.0), (1, 1.25), (9, 3000.0), (10, 1.1), (12, 10000.0), (13, 1.2)]),
            (K::Maximizer, "Limit", &[(0, 1.4), (1, 0.95), (2, 0.06)]),
        ],
    );
    p.module_mut(freeze).unwrap().gain = 0.6;

    // The break: sliced at every hit, and beat-synced, so a faster song
    // plays it faster and higher, as old samplers did; it seeks when the
    // song starts partway. Its phrases stumble in triplets, fire 64ths and
    // limp in fives, finer than the pattern's grid.
    let brk = add(p, K::Sampler, "Rain Break", &[(0, 1.0), (6, 0.02)]);
    let (beat, step) = break_sample(sr);
    let mut slot = rendered(beat);
    slot.beat_sync = (STEPS * 2) as u16;
    slot.autoseek = true;
    slot.slices = STARTS[1..].iter().map(|&s| s * step).collect();
    // The hits after the break ring out whatever note-offs say, and the
    // big snare is tuned down a little.
    slot.slice_settings = STARTS
        .iter()
        .map(|&s| SliceSettings {
            oneshot: s >= 32,
            transpose: if s == 36 { -3 } else { 0 },
            ..SliceSettings::default()
        })
        .collect();
    let m = p.module_mut(brk).unwrap();
    m.samples = vec![slot];
    m.phrases =
        vec![break_phrase(6, "K.gS.g K.gSgS"), break_phrase(16, "SrSrSSrS KSKSSSSS"), break_phrase(5, "KgSgg KSgSS")];
    let [_, _, break_filter, stutter] = effects(
        p,
        brk,
        [
            (K::Distortion, "Break Grit", &[(0, 1.8), (1, 0.8), (2, 0.6), (4, 12.0)]),
            (K::Compressor, "Break Bus", &[(0, 0.3), (1, 3.0), (2, 0.004), (3, 0.09), (4, 1.5)]),
            (K::FilterPro, "Break Filter", &[(1, 20000.0), (4, 1.0)]),
            (K::Repeater, "Stutter", &[]),
        ],
    );
    // The Chops track plays the same break into an echo of its own: a
    // highpass and a multitap delay, as track effects.
    let throw = [
        (0, 3.0),
        (1, 0.5),
        (2, -0.8),
        (3, 6.0),
        (4, 0.4),
        (5, 0.8),
        (6, 9.0),
        (7, 0.3),
        (8, -0.4),
        (9, 12.0),
        (10, 0.25),
        (11, 0.4),
        (12, 0.25),
        (13, 0.5),
    ];
    effects(
        p,
        Owner::Track(CHOPS),
        [(K::FilterPro, "Throw Cut", &[(0, 1.0), (1, 500.0)]), (K::Multitap, "Throw", &throw)],
    );

    // A worn record's crackle, looped, seeking too.
    let dust_id = add(p, K::Sampler, "Dust", &[(0, 1.1), (6, 0.5)]);
    let mut slot = rendered(dust(sr));
    slot.loop_mode = 1;
    slot.autoseek = true;
    p.module_mut(dust_id).unwrap().samples = vec![slot];
    effects(p, dust_id, [(K::FilterPro, "Dust Cut", &[(0, 1.0), (1, 300.0)])]);

    // The storm's gabber kick: a Kicker driven hard and clipped, turned
    // down after.
    let hammer = add(p, K::Kicker, "Hammer", &[(0, 0.7), (2, 4.0), (3, 0.025), (5, 0.42), (6, 0.85)]);
    let [_, hammer_tone] = effects(
        p,
        hammer,
        [
            (K::Distortion, "Hammer Clip", &[(0, 5.0), (1, 0.55), (3, 1.0)]),
            (K::Eq, "Hammer Tone", &[(0, 1.4), (1, 0.6), (2, 1.3), (3, 90.0), (4, 500.0), (5, 4000.0)]),
        ],
    );
    p.module_mut(hammer_tone).unwrap().gain = 0.6;

    // The sub: a sine, warmed by a WaveShaper so small speakers hear it,
    // and turned down after.
    let sub = add(p, K::Generator, "Deep Sub", &[(0, 0.55), (1, 3.0), (2, 0.004), (3, 0.4), (4, 0.8), (5, 0.06)]);
    let [_, _, sub_low] = effects(
        p,
        sub,
        [
            (K::WaveShaper, "Sub Warmth", &[(0, 1.6), (2, 0.9), (9, 0.4), (10, 0.7), (11, 0.88), (12, 0.95)]),
            (K::DcBlocker, "Sub Centre", &[]),
            (K::FilterPro, "Sub Low", &[(1, 220.0), (4, 1.0)]),
        ],
    );
    p.module_mut(sub_low).unwrap().gain = 0.4;

    // The acid line: a saw through a resonant filter that snaps shut after
    // each note, as a 303's, then a ladder, fuzz and an echo.
    let acid = add(p, K::Generator, "Acid Line", &[(0, 0.3), (2, 0.001), (3, 0.18), (4, 0.5), (5, 0.03)]);
    let m = p.module_mut(acid).unwrap();
    m.modulation.filter = true;
    m.modulation.cutoff = 300.0;
    m.modulation.resonance = 0.8;
    m.modulation.filter_env =
        VoiceEnvelope { on: true, points: vec![(0.0, 1.0), (0.2, 0.0)], sustain: None, curve: true, amount: 4.0 };
    let [ladder, _, acid_echo] = effects(
        p,
        acid,
        [
            (K::AnalogFilter, "Acid Ladder", &[(1, 700.0), (2, 0.5), (3, 2.5)]),
            (K::Distortion, "Acid Fuzz", &[(0, 4.0), (1, 0.6), (2, 0.7)]),
            (K::Delay, "Acid Echo", &[(0, 6.0), (1, 0.4), (2, 0.22), (3, 1.0)]),
        ],
    );
    p.module_mut(acid_echo).unwrap().gain = 0.6;

    // The fretless: a triangle whose filter closes as it rings, with a late
    // vibrato, and a mouth that Mwah opens on each note.
    let bass = add(p, K::Generator, "Fretless", &[(0, 0.7), (1, 2.0), (2, 0.003), (3, 0.5), (4, 0.5), (5, 0.1)]);
    let m = p.module_mut(bass).unwrap();
    m.modulation.vibrato = VoiceLfo { on: true, shape: 0, rate: 5.0, depth: 0.12, delay: 0.2 };
    m.modulation.filter = true;
    m.modulation.cutoff = 500.0;
    m.modulation.resonance = 0.3;
    m.modulation.filter_env =
        VoiceEnvelope { on: true, points: vec![(0.0, 1.0), (0.3, 0.25)], sustain: None, curve: true, amount: 2.5 };
    let [mouth, _, fretless_chorus] = effects(
        p,
        bass,
        [
            (K::VocalFilter, "Fretless Mouth", &[(1, -4.0), (2, 1.6), (3, 3.0), (4, 2.2), (5, 0.55)]),
            (K::Compressor, "Bass Squeeze", &[(0, 0.25), (1, 4.0), (2, 0.003), (3, 0.12), (4, 1.6)]),
            (K::Chorus, "Fretless Chorus", &[(0, 2.0), (1, 0.3), (2, 0.25), (6, 0.25)]),
        ],
    );
    p.module_mut(fretless_chorus).unwrap().gain = 1.1;

    // Tines: an FMX electric piano, two stacks, one with a bright tine,
    // swinging left and right.
    let body = [(3, 1.0), (4, 1.0), (5, 0.002), (6, 2.5), (7, 0.2), (8, 0.4), (9, 0.35), (12, 0.8), (13, 0.15)];
    let tine = [(15, 0.45), (16, 1.0), (18, 1.2), (19, 0.0), (21, 0.25), (22, 14.0), (24, 0.06), (25, 0.0)];
    let keys = add(p, K::Fmx, "Tines", &[&[(0, 0.35), (1, 4.0)], &body[..], &tine[..]].concat());
    effects(
        p,
        keys,
        [
            (K::Lfo, "Tine Pan", &[(0, 1.0), (2, 0.6), (4, 1.0), (5, 6.0)]),
            (K::Chorus, "Tine Chorus", &[(0, 2.0), (1, 0.5), (2, 0.4), (3, 0.012), (6, 0.35)]),
        ],
    );
    send(p, keys, room);

    // The music box: FM tines, wobbling as on an old tape, crunched to a
    // few bits and echoing.
    let box_params = [(0, 0.5), (1, 7.0), (2, 1.6), (3, 0.08), (4, 0.05), (5, 0.001), (6, 1.3), (7, 0.0), (8, 0.7)];
    let music_box = add(p, K::Fm, "Music Box", &box_params);
    effects(
        p,
        music_box,
        [
            (K::Vibrato, "Wow", &[(0, 0.7), (1, 0.4), (2, 0.15)]),
            (K::Distortion, "Lo-Fi", &[(0, 1.3), (1, 0.6), (4, 10.0), (5, 3.0)]),
            (K::Echo, "Box Echo", &[(0, 0.523), (1, 0.35), (2, 0.5), (3, 0.6), (4, 0.25)]),
        ],
    );
    send(p, music_box, room);

    // Strings: a wide saw ensemble, through an Amplifier the Gate chops.
    let strings_params = [(0, 0.2), (2, 0.25), (3, 1.0), (4, 0.85), (5, 0.8), (6, 16.0), (7, 4.0)];
    let strings = add(p, K::Generator, "Rain Strings", &strings_params);
    let [_, _, string_gate] = effects(
        p,
        strings,
        [
            (K::FilterPro, "String Tone", &[(1, 4500.0), (2, 0.7)]),
            (K::Chorus, "String Chorus", &[(0, 3.0), (1, 0.6), (2, 0.5), (6, 0.4)]),
            (K::Amplifier, "String Gate", &[]),
        ],
    );
    send(p, strings, room);

    // Fog: a slow SpectraVoice pad, its harmonics shimmering, phased.
    let fog_params = [(0, 0.35), (1, 20.0), (2, 1.3), (3, 0.5), (4, 0.0015), (5, 0.8), (6, 1.2), (7, 2.0), (9, 2.5)];
    let fog = add(p, K::SpectraVoice, "Fog", &fog_params);
    effects(
        p,
        fog,
        [
            (K::Phaser, "Fog Phase", &[(0, 0.08), (1, 0.7), (2, 300.0), (3, 3000.0), (4, 6.0), (5, 0.3), (6, 0.4)]),
            (K::FilterPro, "Fog Cut", &[(0, 1.0), (1, 180.0)]),
        ],
    );
    send(p, fog, room);

    // The lead cries: detuned saws screaming through a band, an octave up
    // behind them, and an echo.
    let lead =
        add(p, K::Generator, "Cry Lead", &[(0, 0.25), (2, 0.008), (3, 0.4), (4, 0.75), (5, 0.25), (6, 9.0), (7, 2.0)]);
    p.module_mut(lead).unwrap().modulation.vibrato =
        VoiceLfo { on: true, shape: 0, rate: 6.0, depth: 0.2, delay: 0.35 };
    effects(
        p,
        lead,
        [
            (K::ScreamFilter, "Cry", &[(0, 2.0), (1, 1500.0), (2, 0.55), (3, 3.0), (4, 0.6)]),
            (K::PitchShifter, "Cry Octave", &[(2, 0.05), (4, 0.22)]),
            (K::Echo, "Cry Echo", &[(0, 0.349), (1, 0.3), (2, 0.4), (3, 0.5), (4, 0.25)]),
        ],
    );
    send(p, lead, room);

    // Data: short, bright FM bleeps, ring-modulated and scattered in taps.
    let data_params = [(0, 0.42), (1, 5.5), (2, 4.0), (3, 0.03), (4, 0.3), (5, 0.001), (6, 0.08), (7, 0.0), (8, 0.03)];
    let data = add(p, K::Fm, "Data", &data_params);
    let taps =
        [(0, 2.0), (2, -0.9), (3, 5.0), (5, 0.9), (6, 7.0), (8, -0.4), (9, 11.0), (11, 0.6), (12, 0.2), (13, 0.5)];
    effects(
        p,
        data,
        [(K::RingMod, "Data Ring", &[(0, 1800.0), (1, 2.0), (2, 0.25), (3, 0.35)]), (K::Multitap, "Data Taps", &taps)],
    );
    send(p, data, room);

    // Modulators. Duck: the fog and the strings pump under the break.
    // Accent: the acid's ladder opens on its loud notes. Mwah: the
    // fretless's mouth opens on each note. Gate: a drawn rhythm synced to
    // the beat chops the strings, once its Amount is turned up.
    let duck = add(p, K::Modulator, "Duck", &[(0, 1.0), (5, -0.35), (6, 0.002), (7, 0.16)]);
    p.connect(stutter, duck);
    for (target, kind) in [(fog, K::SpectraVoice), (strings, K::Generator)] {
        p.connect(duck, target);
        p.set_control_param(duck, target, kind.params().len());
    }
    let accent = add(p, K::Modulator, "Accent", &[(0, 3.0), (5, 0.3), (6, 0.003), (7, 0.1)]);
    p.connect(acid, accent);
    p.connect(accent, ladder);
    p.set_control_param(accent, ladder, 1);
    let mwah = add(p, K::Modulator, "Mwah", &[(0, 4.0), (5, 0.7), (6, 0.005), (7, 0.3)]);
    p.connect(bass, mwah);
    p.connect(mwah, mouth);
    p.set_control_param(mwah, mouth, 0);
    let gate = add(p, K::Modulator, "Gate", &[(1, DRAWN_SHAPE as f32), (3, 2.0), (5, 0.0), (8, 4.0)]);
    p.module_mut(gate).unwrap().shape = gate_shape("x.xx.x.x");
    p.connect(gate, string_gate);
    p.set_control_param(gate, string_gate, 0);

    Ids {
        brk,
        break_filter,
        stutter,
        dust: dust_id,
        hammer,
        sub,
        acid,
        ladder,
        bass,
        keys,
        music_box,
        strings,
        gate,
        fog,
        lead,
        data,
        room,
        freeze,
    }
}

// ---------------------------------------------------------------- parts

/// A new pattern of `lines` with the song's columns.
fn pattern(lines: usize) -> Pattern {
    let mut pat = Pattern::new("", TRACKS, lines);
    for (t, cols) in COLUMNS {
        pat.set_columns(t, cols);
    }
    pat.set_fx_columns(BEAT, 1);
    pat
}

/// Plays the break's slices on `track` from `line` as `grid` spells them
/// (see `slice_of`), a letter every `every` lines: `.` plays nothing, and
/// spaces are only for reading. Hits on the beat are a little louder.
fn chops(pat: &mut Pattern, brk: u8, track: usize, line: usize, every: usize, grid: &str) {
    for (k, c) in grid.chars().filter(|c| *c != ' ').enumerate() {
        let at = line + k * every;
        if let Some(note) = slice_of(c) {
            pat.tracks[track][at] = n(note, brk, if at.is_multiple_of(LPB) { 0x78 } else { 0x68 });
        }
    }
}

/// A roll that walks through the break: `cell` at `line`, played again
/// every `rate` ticks (Exx) for `lines` lines while the effect column
/// slides its pitch with `slide` (1xx or 2xx), so each retrigger lands on
/// another slice.
fn walk(pat: &mut Pattern, line: usize, lines: usize, cell: Cell, rate: u8, slide: (u8, u8)) {
    for l in line..line + lines {
        pat.tracks[BEAT][l] = fx(if l == line { cell } else { Cell::default() }, 0xE, rate);
        pat.cell_mut(BEAT, BEAT_FX, l).fx = Some(slide);
    }
}

/// Rain: the break's ghost notes, hats and rims on the Chops track every
/// `every` lines from `from`, each maybe there (Yxx) and thrown about by
/// the panning column.
fn drops(pat: &mut Pattern, brk: u8, from: usize, every: usize, chance: u8) {
    const DROPS: [u8; 4] = [GHOST, HAT, GHOST2, RIM];
    const PANS: [u8; 7] = [0x10, 0x60, 0x28, 0x78, 0x40, 0x18, 0x68];
    for (k, l) in (from..pat.lines).step_by(every).enumerate() {
        let cell = n(DROPS[k % DROPS.len()], brk, 0x30 + (k % 3) as u8 * 0x0C);
        pat.tracks[CHOPS][l] = Cell { pan: Some(PANS[k % PANS.len()]), ..fx(cell, FX_MAYBE, chance) };
    }
}

/// Bleeps every `every` lines from `from`, thrown about by the panning
/// column: each maybe there (Yxx), but every third certain, and spinning a
/// chord (0xy).
fn bleeps(pat: &mut Pattern, id: u8, from: usize, every: usize, chance: u8) {
    const NOTES: [u8; 8] = [88, 95, 83, 100, 91, 86, 98, 79];
    const PANS: [u8; 5] = [0x08, 0x70, 0x30, 0x78, 0x50];
    for (k, l) in (from..pat.lines).step_by(every).enumerate() {
        let (cmd, arg) = if k % 3 == 2 { (0x0, 0x37) } else { (FX_MAYBE, chance) };
        let cell = Cell { pan: Some(PANS[k % PANS.len()]), ..n(NOTES[k % NOTES.len()], id, 0x58) };
        pat.tracks[FX][l] = fx(cell, cmd, arg);
    }
}

/// The fog's chords, the electric piano's top three notes, one every
/// `every` lines.
fn pads(pat: &mut Pattern, fog: u8, chords: &[Chord], every: usize, vol: u8) {
    for (k, ch) in chords.iter().enumerate() {
        for (c, &note) in ch.keys[1..].iter().enumerate() {
            *pat.cell_mut(FOG, c, k * every) = n(note, fog, vol);
        }
    }
}

/// The electric piano's chords, one to each bar of `bar` lines: each of
/// `hits` in it as (line, lines held, volume).
fn comp(pat: &mut Pattern, keys: u8, chords: &[Chord], bar: usize, hits: &[(usize, usize, u8)]) {
    for (b, ch) in chords.iter().enumerate() {
        for &(l, held, vol) in hits {
            let at = b * bar + l;
            for (c, &note) in ch.keys.iter().enumerate() {
                *pat.cell_mut(KEYS, c, at) = n(note, keys, vol);
                let end = pat.cell_mut(KEYS, c, at + held);
                if end.note.is_none() {
                    *end = off();
                }
            }
        }
    }
}

/// The sub in each bar: each of `hits` as (line, an octave up, gliding
/// into it with 3xx).
fn sub(pat: &mut Pattern, id: u8, chords: &[Chord], hits: &[(usize, bool, bool)], vol: u8) {
    for (b, ch) in chords.iter().enumerate() {
        for &(l, up, glide) in hits {
            let cell = n(ch.root + if up { 12 } else { 0 }, id, vol);
            pat.tracks[SUB][b * BAR + l] = if glide { fx(cell, 0x3, 0x18) } else { cell };
        }
    }
}

/// The acid line, a bar over each of `tones` (see `ACID_TONES`): accents
/// louder, slides glided into (3xx), and the other notes cut short (Cxx),
/// as a 303's gate lets go.
fn acid(pat: &mut Pattern, id: u8, tones: &[[u8; 5]; 4]) {
    for (b, t) in tones.iter().enumerate() {
        for (s, step) in ACID_BAR.iter().enumerate() {
            let Some(k) = *step else { continue };
            let at = b * BAR + s * 2;
            let cell = n(t[k], id, if ACCENTS.contains(&s) { 0x80 } else { 0x50 });
            pat.tracks[ACID][at] = if SLIDES.contains(&s) { fx(cell, 0x3, 0x20) } else { cell };
            if !SLIDES.contains(&(s + 1)) {
                pat.tracks[ACID][at + 1].fx = Some((0xC, 0x03));
            }
        }
    }
}

/// The fretless plays `notes` (line, note, glide): glides with 3xx, and
/// notes held longer waver (4xy).
fn run(pat: &mut Pattern, id: u8, notes: &[(usize, u8, bool)]) {
    for (k, &(l, note, glide)) in notes.iter().enumerate() {
        let cell = n(note, id, if l.is_multiple_of(LPB) { 0x78 } else { 0x60 });
        pat.tracks[BASS][l] = if glide { fx(cell, 0x3, 0x30) } else { cell };
        let next = notes.get(k + 1).map_or(pat.lines, |nt| nt.0);
        if next - l >= 6 {
            for v in l + 3..next {
                pat.tracks[BASS][v].fx = Some((0x4, 0x12));
            }
        }
    }
}

/// The lead cries `notes` an octave down: it glides (3xx) into leaps of a
/// fourth or more, and its long notes waver (4xy).
fn cry(pat: &mut Pattern, id: u8, notes: &[(usize, u8)], vol: u8) {
    let mut last = None;
    for (k, &(l, note)) in notes.iter().enumerate() {
        let note = note - 12;
        let cell = n(note, id, vol);
        pat.tracks[LEAD][l] = match last {
            Some(p) if note.abs_diff(p) >= 5 => fx(cell, 0x3, 0x40),
            _ => cell,
        };
        let next = notes.get(k + 1).map_or(pat.lines, |nt| nt.0);
        if next - l >= LPB {
            for v in l + 3..next {
                pat.tracks[LEAD][v].fx = Some((0x4, 0x13));
            }
        }
        last = Some(note);
    }
}

/// The strings hold `STRINGS_LOOP`, a chord a bar.
fn strings(pat: &mut Pattern, id: u8, vol: u8) {
    for (b, chord) in STRINGS_LOOP.iter().enumerate() {
        for (c, &note) in chord.iter().enumerate() {
            *pat.cell_mut(STRINGS, c, b * BAR) = n(note, id, vol);
        }
    }
}

/// The tune's first two bars, `stretch` times as long, from `from`.
fn first_bars(stretch: usize, from: usize) -> Vec<(usize, u8)> {
    THEME.iter().filter(|t| t.0 < 2 * BAR).map(|&(l, note)| (from + l * stretch, note)).collect()
}

fn key(ids: &Ids) -> Pattern {
    let mut pat = pattern(BAR);
    // The crackle starts, and runs to the end.
    *pat.cell_mut(FX, 1, 0) = n(48, ids.dust, 0x80);
    // A key winds the music box: rim clicks, quick and uneven and each cut
    // short (Cxx), twice.
    for turn in [0, 16] {
        for (k, l) in [0, 1, 2, 3, 5, 6, 7, 9, 10].into_iter().enumerate() {
            pat.tracks[CHOPS][turn + l] = fx(n(RIM, ids.brk, 0x24 + 5 * k as u8), 0xC, 0x01);
        }
    }
    pat
}

fn music_box(ids: &Ids) -> Pattern {
    let mut pat = pattern(LINES);
    tune(&mut pat, BOX, 0, ids.music_box, &THEME, 0x78);
    pads(&mut pat, ids.fog, &LOOP, BAR, 0x50);
    // The first drops of rain.
    drops(&mut pat, ids.brk, 2 * BAR, 4, 0x40);
    pat
}

fn wind_up(ids: &Ids) -> Pattern {
    let mut pat = pattern(LINES);
    // The break plays whole, twice, through a resonant filter opening up.
    for l in [0, 2 * BAR] {
        pat.tracks[BEAT][l] = n(48, ids.brk, 0x78);
    }
    let f = ModuleKind::FilterPro;
    let opening = [(0.0, 150.0), (96.0, 3000.0), (124.0, 20000.0)];
    pat.automation.push(envelope(ids.break_filter, f, 1, &opening, false, true));
    pat.automation.push(envelope(ids.break_filter, f, 2, &[(0.0, 5.0), (124.0, 0.7)], false, true));
    tune(&mut pat, BOX, 0, ids.music_box, &THEME, 0x70);
    pads(&mut pat, ids.fog, &LOOP, BAR, 0x48);
    sub(&mut pat, ids.sub, &LOOP, &[(0, false, false)], 0x68);
    // A reversed snare sucks the song into the rain.
    pat.tracks[CHOPS][LINES - 8] = n(REVERSE, ids.brk, 0x78);
    pat
}

fn rain(ids: &Ids, second: bool) -> Pattern {
    let mut pat = pattern(LINES);
    let_go(&mut pat, &[(FOG, 3)]);
    let brk = ids.brk;
    if second {
        // In 32nds: a ghost note left to chance (Yxx), a flam before a
        // snare (the delay column), a ghost a half line late (Dxx), a
        // machine gun (Exx), a run of snares and ghosts getting louder, a
        // beat stuck in the Stutter, a beat of nothing, and the reversed
        // snare into the next.
        chops(&mut pat, brk, BEAT, 0, 1, RAIN_2);
        pat.tracks[BEAT][31].fx = Some((FX_MAYBE, 0x80));
        *pat.cell_mut(BEAT, 1, 39) = Cell { delay: Some(0xA0), ..n(GHOST, brk, 0x50) };
        pat.tracks[BEAT][70].fx = Some((0xD, 0x03));
        roll(&mut pat, BEAT, 61, 3, n(SNARE, brk, 0x60), 0x02, 0x7F);
        for (k, l) in (88..96).enumerate() {
            pat.tracks[BEAT][l].vol = Some(0x40 + k as u8 * 8);
        }
        hold(&mut pat, ids.stutter, &[(48.0, 56.0)], &[(48.0, 4.0)]);
        // The acid line comes in, its ladder opening and closing.
        acid(&mut pat, ids.acid, &ACID_TONES);
        let a = ModuleKind::AnalogFilter;
        let sweep = [(0.0, 300.0), (64.0, 2500.0), (128.0, 600.0)];
        pat.automation.push(envelope(ids.ladder, a, 1, &sweep, false, true));
        pat.automation.push(envelope(ids.ladder, a, 2, &[(0.0, 0.45), (128.0, 0.75)], false, true));
        tune(&mut pat, BOX, 0, ids.music_box, &THEME, 0x68);
    } else {
        chops(&mut pat, brk, BEAT, 0, 2, RAIN);
        roll(&mut pat, BEAT, 120, 8, n(SNARE, brk, 0x48), 0x03, 0x7F);
        tune(&mut pat, BOX, 0, ids.music_box, &ANSWER, 0x68);
    }
    // Chops thrown into the echo, left and right.
    for (l, slice, pan) in [(30, SNARE, 0x08), (62, RIM, 0x78), (94, SNARE3, 0x10), (110, GHOST, 0x70)] {
        pat.tracks[CHOPS][l] = Cell { pan: Some(pan), ..n(slice, brk, 0x60) };
    }
    comp(&mut pat, ids.keys, &LOOP, BAR, &[(0, 14, 0x50), (22, 4, 0x44), (28, 3, 0x3C)]);
    sub(&mut pat, ids.sub, &LOOP, &[(0, false, false), (20, false, false), (28, true, true)], 0x70);
    pat
}

fn fretless(ids: &Ids, second: bool) -> Pattern {
    let mut pat = pattern(LINES);
    let_go(&mut pat, &[(SUB, 1)]);
    let brk = ids.brk;
    let mut notes = RUN.to_vec();
    if second {
        // Ghost notes in 32nds, the Stutter catching two beats, and the
        // snare marching into the drill.
        chops(&mut pat, brk, BEAT, 0, 1, SWING_2);
        roll(&mut pat, BEAT, 120, 8, n(SNARE3, brk, 0x50), 0x03, 0x7F);
        hold(&mut pat, ids.stutter, &[(48.0, 56.0), (88.0, 92.0)], &[(48.0, 4.0), (88.0, 6.0)]);
        // The run climbs an octave in the third bar and ends in a
        // chromatic fall of 32nds.
        for note in notes.iter_mut().filter(|nt| (2 * BAR..3 * BAR).contains(&nt.0)) {
            note.1 += 12;
        }
        notes.retain(|nt| nt.0 < 120);
        notes.extend((0..8).map(|k| (120 + k, 52 - k as u8, false)));
    } else {
        chops(&mut pat, brk, BEAT, 0, 2, SWING);
    }
    run(&mut pat, ids.bass, &notes);
    // Ghost notes left to chance.
    for cell in pat.tracks[BEAT][..LINES].iter_mut().filter(|c| c.note == Some(Note::On(GHOST)) && c.fx.is_none()) {
        cell.fx = Some((FX_MAYBE, 0xA0));
    }
    comp(&mut pat, ids.keys, &JAZZ, BAR, &[(0, 5, 0x60), (12, 4, 0x50)]);
    if second {
        comp(&mut pat, ids.keys, &[FS7ALT], BAR, &[(3 * BAR + 20, 3, 0x48), (3 * BAR + 26, 4, 0x58)]);
    }
    pat
}

fn drill(ids: &Ids) -> Pattern {
    let mut pat = pattern(LINES);
    let_go(&mut pat, &[(BASS, 1)]);
    let brk = ids.brk;
    // A crash on the one, chops, and a last beat of 96ths (E02).
    chops(&mut pat, brk, BEAT, 0, 1, "K..gS.g. .gK.S.gS K.g.S..g");
    *pat.cell_mut(BEAT, 1, 0) = n(CRASH, brk, 0x60);
    roll(&mut pat, BEAT, 24, 8, n(SNARE, brk, 0x30), 0x02, 0x7F);
    // The break's triplet phrase (Z01) for two beats, then chops, and rims
    // thrown hard left and right (8xx).
    pat.tracks[BEAT][32] = fx(n(48, brk, 0x78), FX_PHRASE, 1);
    chops(&mut pat, brk, BEAT, 48, 1, "K...S... KK..S.SS");
    // Cut short by the volume column (C2).
    let clipped = vol_command_value('C', 2);
    for (k, l) in [50, 54, 58, 62].into_iter().enumerate() {
        let rim = Cell { vol: clipped, ..n(RIM, brk, 0) };
        pat.tracks[CHOPS][l] = fx(rim, 0x8, if k % 2 == 0 { 0x00 } else { 0xFF });
    }
    // Twelve ticks a line (F0C): a snare buzzing on every tick (E01) and
    // diving as the break's Transpose falls, a rim walking back through
    // the slices (E02 under 2xx), and kicks and snares into 64ths (E06).
    pat.cell_mut(FX, 1, 64).fx = Some((0xF, 0x0C));
    pat.tracks[BEAT][64] = n(KICK, brk, 0x78);
    *pat.cell_mut(BEAT, 1, 64) = fx(n(BIG, brk, 0x58), 0x9, 0x30);
    roll(&mut pat, BEAT, 66, 8, n(SNARE, brk, 0x60), 0x01, 0x78);
    let s = ModuleKind::Sampler;
    let dive = [(0.0, 0.0), (66.0, 0.0), (68.0, -3.0), (70.0, -6.0), (72.0, -10.0), (74.0, 0.0)];
    pat.automation.push(envelope(brk, s, 2, &dive, true, false));
    walk(&mut pat, 76, 4, n(RIM, brk, 0x60), 0x02, (0x2, 0x04));
    // A bar played from the kick's key alone, each line picking its
    // slice (Sxx), the snare at the end backwards (Rxx).
    const RESEQUENCED: [Option<u8>; 16] = [
        Some(KICK),
        None,
        Some(SNARE),
        Some(GHOST),
        Some(KICK),
        None,
        Some(SNARE3),
        Some(HAT),
        Some(KICK),
        Some(DRY_KICK),
        Some(SNARE),
        Some(SNARE2),
        Some(KICK),
        Some(GHOST2),
        Some(SNARE),
        Some(SNARE),
    ];
    for (k, hit) in RESEQUENCED.into_iter().enumerate() {
        if let Some(note) = hit {
            pat.tracks[BEAT][80 + k] = fx(n(KICK, brk, if k % 4 == 0 { 0x78 } else { 0x64 }), FX_SLICE, note - KICK);
        }
    }
    pat.cell_mut(BEAT, BEAT_FX, 95).fx = Some((FX_REVERSE, 0x01));
    roll(&mut pat, BEAT, 92, 4, n(SNARE, brk, 0x68), 0x06, 0x7F);
    // Six ticks again (F06): the phrase in fives (Z03), caught in the
    // Stutter; the big snare from partway in (9xx), a crash diving (2xx)
    // and the reversed snare.
    pat.cell_mut(FX, 1, 96).fx = Some((0xF, 0x06));
    pat.tracks[BEAT][96] = fx(n(48, brk, 0x78), FX_PHRASE, 3);
    hold(&mut pat, ids.stutter, &[(106.0, 112.0)], &[(106.0, 6.0)]);
    pat.tracks[BEAT][112] = fx(n(BIG, brk, 0x70), 0x9, 0x20);
    pat.tracks[CHOPS][114] = fx(n(CRASH, brk, 0x58), 0x8, 0x80);
    for l in 115..120 {
        pat.tracks[CHOPS][l].fx = Some((0x2, 0x08));
    }
    pat.tracks[BEAT][120] = n(REVERSE, brk, 0x78);
    // The acid's ladder jumps about as if a hand were on it.
    acid(&mut pat, ids.acid, &ACID_TONES);
    let a = ModuleKind::AnalogFilter;
    let jumps: Vec<(f32, f32)> = [
        400.0, 3500.0, 700.0, 5000.0, 300.0, 2000.0, 900.0, 6000.0, 500.0, 4000.0, 1200.0, 7000.0, 400.0, 2500.0,
        800.0, 9000.0,
    ]
    .iter()
    .enumerate()
    .map(|(k, &v)| (k as f32 * 8.0, v))
    .collect();
    pat.automation.push(envelope(ids.ladder, a, 1, &jumps, true, false));
    pat.automation.push(envelope(ids.ladder, a, 2, &[(0.0, 0.8)], true, false));
    sub(&mut pat, ids.sub, &LOOP, &[(0, false, false), (20, false, false), (28, true, true)], 0x70);
    bleeps(&mut pat, ids.data, 4, 8, 0x50);
    pat
}

fn unwound(ids: &Ids) -> Pattern {
    let mut pat = pattern(LINES);
    // The drums stop; rain is left, thrown into the Chops track's echo.
    drops(&mut pat, ids.brk, 0, 2, 0x48);
    // It starts far off and comes closer: the track's volume (Lxx).
    for (l, vol) in [(1, 0x28), (33, 0x40), (65, 0x58), (97, 0x70), (121, 0x80)] {
        pat.tracks[CHOPS][l].fx = Some((FX_TRACK_VOLUME, vol));
    }
    // The fog holds E minor, then C, trembling (7xy) as it turns; the room
    // opens; the music box plays the tune at half speed and now and then
    // spins a chord (0xy); bleeps come and go.
    pads(&mut pat, ids.fog, &[EM9, CMAJ9], 2 * BAR, 0x68);
    for c in 0..3 {
        for l in 48..64 {
            pat.cell_mut(FOG, c, l).fx = Some((0x7, 0x24));
        }
    }
    let r = ModuleKind::Reverb;
    let wash = [(0.0, 0.28), (32.0, 0.5), (112.0, 0.5), (128.0, 0.3)];
    pat.automation.push(envelope(ids.room, r, 2, &wash, false, true));
    tune(&mut pat, BOX, 0, ids.music_box, &first_bars(2, 0), 0x78);
    for l in [16, 96] {
        pat.tracks[BOX][l].fx = Some((0x0, 0x37));
    }
    pat.tracks[SUB][0] = n(EM9.root, ids.sub, 0x60);
    pat.tracks[SUB][2 * BAR] = fx(n(CMAJ9.root, ids.sub, 0x60), 0x3, 0x08);
    bleeps(&mut pat, ids.data, 2, 6, 0x60);
    pat
}

fn seven(ids: &Ids) -> Pattern {
    let mut pat = pattern(4 * BAR7);
    let_go(&mut pat, &[(SUB, 1), (FOG, 3)]);
    let brk = ids.brk;
    // Seven eighths a bar, grouped two, two and three.
    chops(&mut pat, brk, BEAT, 0, 2, LIMP);
    roll(&mut pat, BEAT, 3 * BAR7 + 20, 8, n(SNARE, brk, 0x50), 0x03, 0x7F);
    for (b, riff) in SEVEN_RIFF.iter().enumerate() {
        for (k, &note) in riff.iter().enumerate() {
            pat.tracks[BASS][b * BAR7 + k * 4] = n(note, ids.bass, if k == 0 { 0x78 } else { 0x60 });
        }
        // The music box's clockwork, swinging across (Nxy) at each turn.
        for (k, &note) in CLOCKWORK.iter().enumerate() {
            let cell = n(note, ids.music_box, 0x60);
            pat.tracks[BOX][b * BAR7 + k * 4] = if k == 0 { fx(cell, FX_AUTOPAN, 0x1C) } else { cell };
        }
    }
    comp(&mut pat, ids.keys, &SEVEN, BAR7, &[(0, 10, 0x58), (16, 4, 0x48)]);
    pat
}

fn build(ids: &Ids) -> Pattern {
    let mut pat = pattern(LINES);
    let_go(&mut pat, &[(BASS, 1)]);
    let brk = ids.brk;
    // The snare comes faster each bar, eighths, sixteenths and 32nds, then
    // rolls of 64ths, 96ths and 192nds, louder all the way, and pitched up
    // in the last bar by the break's Transpose. Kicks keep the beat until
    // then, and the Stutter catches the end.
    for (bar, every) in [(0, 4), (1, 2), (2, 1)] {
        for l in (bar * BAR..(bar + 1) * BAR).step_by(every) {
            pat.tracks[BEAT][l] = n(SNARE, brk, 0x30 + (l / 2) as u8);
        }
    }
    for (l, lines, rate, from, to) in
        [(96, 16, 0x03, 0x60, 0x70), (112, 8, 0x02, 0x70, 0x78), (120, 8, 0x01, 0x78, 0x80)]
    {
        roll(&mut pat, BEAT, l, lines, n(SNARE, brk, from), rate, to);
    }
    let s = ModuleKind::Sampler;
    let rise = [(0.0, 0.0), (96.0, 0.0), (104.0, 2.0), (112.0, 5.0), (120.0, 9.0), (124.0, 12.0)];
    pat.automation.push(envelope(brk, s, 2, &rise, true, false));
    for l in (0..3 * BAR).step_by(LPB) {
        *pat.cell_mut(BEAT, 1, l) = n(KICK, brk, 0x70);
    }
    hold(&mut pat, ids.stutter, &[(124.0, 128.0)], &[(124.0, 8.0)]);
    // The last line holds two more (Wxx), stuttering, before the storm.
    pat.cell_mut(FX, 1, LINES - 1).fx = Some((FX_WAIT, 0x02));
    // The sub holds B and climbs an octave in the last bar (1xx); the
    // strings swell in on B7; the music box spins faster and faster; and
    // the acid holds B, its ladder opening.
    for b in 0..4 {
        pat.tracks[SUB][b * BAR] = n(B7.root, ids.sub, 0x68);
    }
    for l in 3 * BAR..LINES {
        pat.tracks[SUB][l].fx = Some((0x1, 0x01));
    }
    for (c, &note) in STRINGS_LOOP[3].iter().enumerate() {
        *pat.cell_mut(STRINGS, c, 0) = n(note, ids.strings, 0x68);
    }
    let g = ModuleKind::Generator;
    pat.automation.push(envelope(ids.strings, g, g.params().len(), &[(0.0, 0.0), (120.0, 1.0)], false, true));
    const SPIN: [u8; 4] = [75, 78, 81, 83];
    for (bar, every) in [(0, 4), (1, 2), (2, 1)] {
        for (k, l) in (bar * BAR..(bar + 1) * BAR).step_by(every).enumerate() {
            pat.tracks[BOX][l] = n(SPIN[k % SPIN.len()], ids.music_box, 0x50 + (l / 4) as u8);
        }
    }
    acid(&mut pat, ids.acid, &[ACID_TONES[3]; 4]);
    let a = ModuleKind::AnalogFilter;
    pat.automation.push(envelope(ids.ladder, a, 1, &[(0.0, 250.0), (128.0, 6000.0)], false, true));
    pat
}

fn storm(ids: &Ids, second: bool) -> Pattern {
    let mut pat = pattern(LINES);
    let brk = ids.brk;
    chops(&mut pat, brk, BEAT, 0, 1, if second { STORM_2 } else { STORM });
    *pat.cell_mut(BEAT, 1, 0) = n(CRASH, brk, 0x70);
    if second {
        // Faster every bar (Fxx), the beat-synced break pitched up with the
        // tempo; a roll walks up through the slices (E03 under 1xx), and
        // the last bar's rolls climb with the break's Transpose.
        for (l, bpm) in [(0, 180), (BAR, 186), (2 * BAR, 192), (3 * BAR, 200)] {
            pat.cell_mut(FX, 1, l).fx = Some((0xF, bpm));
        }
        walk(&mut pat, 88, 8, n(SNARE, brk, 0x70), 0x03, (0x1, 0x02));
        roll(&mut pat, BEAT, 112, 8, n(SNARE3, brk, 0x60), 0x02, 0x78);
        roll(&mut pat, BEAT, 120, 8, n(SNARE3, brk, 0x78), 0x01, 0x80);
        let s = ModuleKind::Sampler;
        let rise = [(0.0, 0.0), (112.0, 0.0), (116.0, 3.0), (120.0, 7.0), (124.0, 12.0)];
        pat.automation.push(envelope(brk, s, 2, &rise, true, false));
    } else {
        // Faster (Fxx); and the 64ths phrase (Z02) under a crash.
        pat.cell_mut(FX, 1, 0).fx = Some((0xF, 180));
        roll(&mut pat, BEAT, 88, 8, n(SNARE, brk, 0x50), 0x02, 0x7F);
        pat.tracks[BEAT][104] = fx(n(48, brk, 0x78), FX_PHRASE, 2);
        *pat.cell_mut(BEAT, 1, 104) = n(CRASH, brk, 0x60);
        roll(&mut pat, BEAT, 120, 8, n(SNARE3, brk, 0x60), 0x03, 0x7F);
    }
    // Chops thrown into the echo.
    for (l, slice, pan) in [(30, SNARE, 0x08), (62, RIM, 0x78), (94, SNARE3, 0x10), (126, GHOST, 0x70)] {
        pat.tracks[CHOPS][l] = Cell { pan: Some(pan), ..n(slice, brk, 0x60) };
    }
    // Gabber kicks on every beat, tuned to each bar's root, doubled at the
    // end of every other bar; the last one dives (2xx).
    for (b, ch) in LOOP.iter().enumerate() {
        for l in [0, 8, 16, 24] {
            pat.tracks[HAMMER][b * BAR + l] = n(ch.root, ids.hammer, 0x78);
        }
        if b % 2 == 1 {
            pat.tracks[HAMMER][b * BAR + 28] = n(ch.root, ids.hammer, 0x60);
        }
    }
    if second {
        for l in 125..LINES {
            pat.tracks[HAMMER][l].fx = Some((0x2, 0x10));
        }
    }
    // The strings, chopped by the Gate from the third bar, and from the
    // start the second time, when they stutter (Txy) at the end; the lead
    // cries the tune an octave under the music box.
    strings(&mut pat, ids.strings, 0x68);
    let m = ModuleKind::Modulator;
    let gate: &[(f32, f32)] = if second { &[(0.0, 1.0)] } else { &[(0.0, 0.0), (64.0, 1.0)] };
    pat.automation.push(envelope(ids.gate, m, 5, gate, true, false));
    if !second {
        // The first storm breaks off (Jxx) two lines early, into the next.
        pat.cell_mut(FX, 1, LINES - 3).fx = Some((FX_BREAK, 0x00));
    }
    let tune_of: &[(usize, u8)] = if second { &ANSWER } else { &THEME };
    cry(&mut pat, ids.lead, tune_of, 0x70);
    tune(&mut pat, BOX, 0, ids.music_box, tune_of, 0x60);
    sub(&mut pat, ids.sub, &LOOP, &[(0, false, false), (16, false, false), (28, true, true)], 0x78);
    if second {
        // The strings stutter on and off (Txy), faster in the last beat.
        for c in 0..3 {
            for l in 3 * BAR..LINES {
                let rate = if l >= LINES - LPB { 0x11 } else { 0x21 };
                pat.cell_mut(STRINGS, c, l).fx = Some((FX_TREMOR, rate));
            }
        }
        acid(&mut pat, ids.acid, &ACID_TONES);
        let a = ModuleKind::AnalogFilter;
        pat.automation.push(envelope(ids.ladder, a, 1, &[(0.0, 1500.0), (128.0, 9000.0)], false, true));
    }
    pat
}

fn crash(ids: &Ids) -> Pattern {
    let mut pat = pattern(LINES);
    // Back to 172 BPM, and everything lets go, while Freeze, on the
    // master, holds the storm's last two lines round and round, shorter
    // and shorter until they buzz, then lets go too.
    pat.cell_mut(FX, 1, 0).fx = Some((0xF, 172));
    let playing =
        [(BEAT, 2), (CHOPS, 1), (HAMMER, 1), (SUB, 1), (ACID, 1), (BASS, 1), (KEYS, 4), (BOX, 2), (STRINGS, 3)];
    let_go(&mut pat, &playing);
    let_go(&mut pat, &[(LEAD, 1)]);
    let lengths =
        [(0.0, 2.0), (6.0, 3.0), (12.0, 4.0), (17.0, 5.0), (21.0, 6.0), (25.0, 7.0), (28.0, 8.0), (30.0, 9.0)];
    hold(&mut pat, ids.freeze, &[(0.0, 32.0)], &lengths);
    // The music box, alone in a bigger room, runs down: the tune's first two
    // bars, and an E that sinks (2xx) as it rings.
    let r = ModuleKind::Reverb;
    pat.automation.push(envelope(ids.room, r, 2, &[(32.0, 0.28), (48.0, 0.5)], false, true));
    tune(&mut pat, BOX, 0, ids.music_box, &first_bars(1, 40), 0x70);
    pat.tracks[BOX][108] = n(76, ids.music_box, 0x68);
    for l in 112..120 {
        pat.tracks[BOX][l].fx = Some((0x2, 0x01));
    }
    for (c, &note) in EM9.keys[1..].iter().enumerate() {
        *pat.cell_mut(FOG, c, 36) = n(note, ids.fog, 0x50);
    }
    // The fog and the crackle fade (Axy), and F00 ends the song.
    for l in 96..LINES {
        for c in 0..3 {
            pat.cell_mut(FOG, c, l).fx = Some((0xA, 0x01));
        }
        pat.cell_mut(FX, 1, l).fx = Some((0xA, 0x01));
    }
    pat.tracks[FX][LINES - 1].fx = Some((0xF, 0x00));
    pat
}

impl Project {
    /// "Clockwork Rain", the third demo song.
    pub fn clockwork_rain() -> Self {
        let mut p = Self::empty();
        p.title = "Clockwork Rain".into();
        p.artist = "noise".into();
        p.comments = COMMENTS.into();
        p.bpm = BPM;
        p.lpb = LPB as u32;
        p.modules[0].params[0] = 0.8;
        let names = [
            "Beat",
            "Chops",
            "Hammer",
            "Sub",
            "Acid",
            "Fretless",
            "Tines",
            "Music Box",
            "Strings",
            "Fog",
            "Lead",
            "FX",
        ];
        let colors = [
            [235, 95, 70],
            [245, 155, 90],
            [190, 45, 65],
            [115, 85, 205],
            [160, 235, 60],
            [205, 165, 95],
            [235, 205, 125],
            [250, 215, 235],
            [125, 170, 240],
            [150, 160, 195],
            [240, 110, 200],
            [190, 190, 190],
        ];
        for (t, (name, color)) in names.iter().zip(colors).enumerate() {
            p.tracks[t].name = name.to_string();
            p.tracks[t].color = Some(color);
        }
        p.tracks[BEAT].show_delay = true;
        p.tracks[CHOPS].show_pan = true;
        p.tracks[FX].show_pan = true;
        let ids = instruments(&mut p);
        let mut patterns = vec![
            key(&ids),
            music_box(&ids),
            wind_up(&ids),
            rain(&ids, false),
            rain(&ids, true),
            fretless(&ids, false),
            fretless(&ids, true),
            drill(&ids),
            unwound(&ids),
            seven(&ids),
            build(&ids),
            storm(&ids, false),
            storm(&ids, true),
            crash(&ids),
        ];
        let names = [
            "Key",
            "Music Box",
            "Wind-Up",
            "Rain",
            "Rain 2",
            "Fretless",
            "Fretless 2",
            "Drill",
            "Unwound",
            "Seven",
            "Build",
            "Storm",
            "Storm 2",
            "Crash",
        ];
        for (pat, name) in patterns.iter_mut().zip(names) {
            pat.name = name.into();
        }
        p.patterns = patterns;
        for (t, cols) in COLUMNS {
            p.set_columns(t, cols);
        }
        p.set_fx_columns(BEAT, 1);
        // The matrix holds back the music box the first time the rain falls
        // and the first time round the 7/8, and the lead in the first storm.
        let muted = |pattern: usize, tracks: &[usize]| {
            let mut s = Slot::new(pattern);
            tracks.iter().for_each(|&t| s.toggle_mute(t));
            s
        };
        p.order = vec![
            Slot::new(0),
            Slot::new(1),
            Slot::new(2),
            muted(3, &[BOX]),
            Slot::new(4),
            Slot::new(3),
            Slot::new(4),
            Slot::new(5),
            Slot::new(6),
            Slot::new(7),
            Slot::new(8),
            muted(9, &[BOX]),
            Slot::new(9),
            Slot::new(10),
            muted(11, &[LEAD]),
            Slot::new(11),
            Slot::new(12),
            Slot::new(13),
        ];
        p.sections = [
            ("Music Box", 0),
            ("Rain", 3),
            ("Fretless", 7),
            ("Drill", 9),
            ("Unwound", 10),
            ("Seven", 11),
            ("Storm", 13),
            ("Crash", 17),
        ]
        .into_iter()
        .map(|(name, start)| Section { start, name: name.into() })
        .collect();
        p
    }
}

const COMMENTS: &str = "Clockwork Rain — the third demo song: IDM in E minor at 172 BPM, at eight lines a beat, so \
    each line is a 32nd note.\n\n\
    A key winds a music box, which plays alone over a record's crackle; a break comes in through a filter and is \
    chopped apart under electric piano chords, a sub and an acid line; a fretless bass runs through jazz changes; \
    the drill rolls, stumbles in triplets and fives, buzzes and dives; it unwinds into fog and rain, limps in 7/8, \
    then storms with gabber kicks, gated strings and a crying lead, faster and faster, until the mix sticks in a \
    loop and leaves the music box to run down.\n\n\
    Where to look:\n\
    • Rain Break: a Sampler with a break the song renders itself, sliced at every hit and beat-synced, so the \
    faster storm plays it faster and higher, as old samplers did. The Beat track chops it: Exx rolls (a note on \
    every line keeps a roll even), Cxx cuts, 9xx starts into a slice, Dxx and the delay column put hits late, Yxx \
    leaves ghost notes to chance. A roll under a slide (its effect column) lands each retrigger on another slice; \
    automating its Transpose pitches a roll instead.\n\
    • Its phrases play finer than the grid: Z01 stumbles in triplets, Z02 fires 64ths, Z03 limps in fives.\n\
    • Chops: the same break on its own track, through the track's highpass and multitap (track effects); in \
    Unwound it is the rain, maybe there and thrown about by the panning column.\n\
    • Drill: F0C gives lines twelve ticks, for buzzing E01 rolls; F06 puts six back. One bar is played from the \
    kick's key alone, Sxx picking each slice, its last snare reversed with R01; rims are cut short with C2 in the \
    volume column.\n\
    • Unwound: the rain comes closer as L28 to L80 raise the Chops track's volume. Build holds its last line two \
    more with W02; the first Storm breaks off early into the next with J00, and Storm 2's strings stutter with Txy.\n\
    • Stutter, the break's Repeater, and Freeze, on the master, hold what they heard and loop it ever shorter; \
    Freeze ends the song.\n\
    • Acid Line: a saw with a filter envelope (its Modulation page), gliding with 3xx and cut with Cxx like a \
    303; Accent, a Modulator following its velocity, opens the Acid Ladder on loud notes.\n\
    • Fretless: 3xx glides and 4xy vibrato; Mwah, a Modulator running an envelope on each note, opens its Vocal \
    Filter from U to A.\n\
    • Rain Strings: Gate, a Modulator with a drawn rhythm synced to the beat, chops them in the storm, where its \
    Amount is automated.\n\
    • Seven is 112 lines: four bars of 7/8. Storm 2 speeds up with Fxx.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clockwork_rain_chops_its_break_every_way() {
        let p = Project::clockwork_rain();
        let brk = p.modules.iter().find(|m| m.name == "Rain Break").unwrap();
        let slot = &brk.samples[0];
        assert_eq!(slot.slice_ranges().len(), STARTS.len(), "a slice at every hit");
        assert!(BREAK.iter().all(|h| STARTS.contains(&h.0)));
        assert!(slot.data.is_some() && slot.autoseek && slot.beat_sync as usize == 2 * STEPS);
        assert_eq!(brk.phrases.iter().map(|ph| ph.lpb).collect::<Vec<_>>(), [6, 16, 5]);
        let mut commands: Vec<(u8, u8)> = Vec::new();
        for pat in &p.patterns {
            for lane in 0..pat.num_lanes() {
                commands.extend(pat.lane(lane)[..pat.lines].iter().filter_map(|c| c.fx));
            }
        }
        for cmd in [
            0x0,
            0x1,
            0x2,
            0x3,
            0x4,
            0x7,
            0x8,
            0x9,
            0xA,
            0xC,
            0xD,
            0xE,
            0xF,
            FX_AUTOPAN,
            FX_MAYBE,
            FX_PHRASE,
            FX_BREAK,
            FX_TRACK_VOLUME,
            FX_REVERSE,
            FX_SLICE,
            FX_TREMOR,
            FX_WAIT,
        ] {
            assert!(commands.iter().any(|c| c.0 == cmd), "effect {:?}", char::from_digit(cmd as u32, 36));
        }
        // Lines of twelve ticks, a storm at 200 BPM, and a part in 7/8.
        assert!(commands.contains(&(0xF, 0x0C)) && commands.contains(&(0xF, 200)));
        assert!(p.patterns.iter().any(|pat| pat.lines == 4 * BAR7));
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
        let last = &p.patterns[p.order.last().unwrap().pattern];
        assert_eq!(last.cell(FX, 0, last.lines - 1).fx, Some((0xF, 0)));
    }

    #[test]
    fn clockwork_rain_plays_loud_enough_without_clipping_and_ends() {
        let frames = crate::audio::render(std::sync::Arc::new(Project::clockwork_rain()), 8000, 1.0, false);
        let peak = frames.iter().fold(0f32, |a, f| a.max(f[0].abs()).max(f[1].abs()));
        assert!(peak > 0.5 && peak <= 1.0, "{peak}");
        let secs = frames.len() as f32 / 8000.0;
        assert!((70.0..130.0).contains(&secs), "{secs} s");
    }
}
