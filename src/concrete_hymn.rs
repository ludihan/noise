//! "Concrete Hymn", the fifth demo song: a minute and a half of overdriven
//! electro in F♯ minor at 118 BPM, loud, crushed and pumping.
//!
//! An organ sings a hymn alone in a hall, noise rising behind it. A square
//! bass riff comes in, bending and gliding, its filter opening over a
//! crushed kick; then everything drops at once: a gritty kit, wide pulse
//! stabs, the whole mix ducking under every kick, and dead stops where the
//! bar falls silent. The organ comes back with a siren of a lead gliding
//! over it, the riser climbs again, and the second drop is harder: the
//! organ stutters, the stabs swell, the master's drive turns up. The riff
//! is left alone at the end, and the organ's last chord rings out.
//!
//! The kit is synthesized here, so its Sampler has audio without files;
//! the samples are written next to the song when it is saved.

use crate::demo::{Hit, effects, envelope, fx, hit, n, normalize, off, rendered, set};
use crate::dsp::Frame;
use crate::project::*;
use crate::sample::Sample;

const BPM: f32 = 118.0;
const LPB: usize = 4;
/// Lines a bar, and a pattern of eight bars.
const BAR: usize = 4 * LPB;
const LINES: usize = 8 * BAR;
/// The kick's note: tuned to the key, F♯.
const KICK_NOTE: u8 = 30;

// ---------------------------------------------------------------- harmony

/// Each two bars' chord: the bass's root and the organ's four notes.
/// F♯ minor, D, A, E: down to the sixth, up to the third, the seventh.
const ROOTS: [u8; 4] = [42, 38, 45, 40];
const ORGAN: [[u8; 4]; 4] = [[54, 57, 61, 66], [54, 57, 62, 66], [52, 57, 61, 64], [52, 56, 59, 64]];

/// The bass riff over a bar, from the chord's root: (line, semitones up,
/// and whether it bends up into the next note with 1xx).
const RIFF: [(usize, u8, bool); 11] = [
    (0, 0, false),
    (2, 0, false),
    (3, 12, false),
    (5, 0, false),
    (6, 10, false),
    (8, 0, false),
    (10, 12, false),
    (11, 7, false),
    (12, 0, false),
    (14, 3, false),
    (15, 5, true),
];

/// The stabs' rhythm over a bar: `x` a stab, cut short.
const STABS: &str = "x..x..x...x.x...";

/// The siren's line over the break, a note every two beats, gliding into
/// each from the last.
const SIREN: [u8; 16] = [69, 73, 71, 66, 69, 74, 73, 68, 69, 73, 76, 78, 76, 73, 71, 68];

// ---------------------------------------------------------------- the kit

/// The kit's keys.
const CLAP: u8 = 48;
const SNARE: u8 = 50;
const HAT: u8 = 54;
const OPEN: u8 = 58;
const CRASH: u8 = 61;

/// Each sound of the kit on a key of its own; the hats choke each other.
fn kit(sr: f32) -> Vec<SampleSlot> {
    let sounds = [
        ("Clap", hit(sr, Hit::Clap, 0xC0DE, 0.0), CLAP, 0, 0.0),
        ("Snare", hit(sr, Hit::Snare, 0x5EED, 0.15), SNARE, 0, 0.0),
        ("Hat", hit(sr, Hit::Hat, 0x4A75, 0.0), HAT, 1, 0.15),
        ("Open Hat", hit(sr, Hit::Open, 0x0FE2, 0.0), OPEN, 1, 0.15),
        ("Crash", hit(sr, Hit::Crash, 0xC2A5, 0.0), CRASH, 0, -0.1),
    ];
    sounds
        .into_iter()
        .map(|(name, sound, key, group, pan)| {
            let mut frames: Vec<Frame> = sound.iter().map(|&x| [x, x]).collect();
            normalize(&mut frames, 0.9);
            let mut slot = rendered(Sample { name: name.into(), sample_rate: sr, channels: 2, frames });
            (slot.base_note, slot.keys, slot.mute_group, slot.panning, slot.oneshot) =
                (key, [key, key], group, pan, true);
            slot
        })
        .collect()
}

// ---------------------------------------------------------------- instruments

const KICK: usize = 0;
const KIT: usize = 1;
const BASS: usize = 2;
const STAB: usize = 3;
const ORGAN_T: usize = 4;
const LEAD: usize = 5;
const FX: usize = 6;
const TRACKS: usize = 7;

/// Note columns per track, where more than one.
const COLUMNS: [(usize, usize); 4] = [(KIT, 4), (STAB, 4), (ORGAN_T, 4), (FX, 2)];

/// The instruments and effects the patterns refer to.
struct Ids {
    kick: u8,
    kit: u8,
    bass: u8,
    bass_filter: u8,
    bass_end: u8,
    stab: u8,
    organ: u8,
    siren: u8,
    riser: u8,
    riser_filter: u8,
    drive: u8,
}

fn instruments(p: &mut Project) -> Ids {
    use ModuleKind as K;
    let add = |p: &mut Project, kind: ModuleKind, name: &str, params: &[(usize, f32)]| {
        let id = p.add_module(kind, [0.0, 0.0]).unwrap();
        set(p, id, name, params);
        p.connect(id, OUTPUT_ID);
        id
    };

    // The master: everything driven into a soft clip, glued hard and
    // limited, the loudness of the style.
    let [drive, ..] = effects(
        p,
        OUTPUT_ID,
        [
            (K::Distortion, "Master Drive", &[(0, 1.6), (1, 0.65), (2, 0.35)]),
            (K::Compressor, "Glue", &[(0, 0.5), (1, 2.5), (2, 0.004), (3, 0.12), (4, 1.2)]),
            (K::Eq5, "Smile", &[(0, 70.0), (1, 1.3), (12, 9000.0), (13, 1.2)]),
            (K::Maximizer, "Limiter", &[(0, 1.7), (1, 0.95), (2, 0.06)]),
        ],
    );

    // The kick: a sine falling four octaves onto F♯, crushed and punched.
    let kick = add(p, K::Kicker, "Crushed Kick", &[(0, 0.8), (2, 4.0), (3, 0.03), (5, 0.32), (6, 0.6)]);
    effects(
        p,
        kick,
        [
            (K::Distortion, "Kick Crush", &[(0, 5.0), (1, 0.5), (2, 0.8), (4, 12.0)]),
            (K::Compressor, "Kick Punch", &[(0, 0.4), (1, 4.0), (2, 0.008), (3, 0.08), (4, 1.3)]),
        ],
    );

    // The kit, bitcrushed and squashed.
    let kit_id = add(p, K::Sampler, "Grit Kit", &[(0, 1.0), (6, 0.06)]);
    p.module_mut(kit_id).unwrap().samples = kit(44100.0);
    effects(
        p,
        kit_id,
        [
            (K::Distortion, "Kit Grit", &[(0, 3.0), (1, 0.6), (2, 0.7), (4, 10.0), (5, 2.0)]),
            (K::Compressor, "Kit Squash", &[(0, 0.3), (1, 5.0), (2, 0.002), (3, 0.1), (4, 1.4)]),
        ],
    );

    // The bass: three detuned squares, clipped hard, through a lowpass the
    // riff's intro opens.
    let bass = add(
        p,
        K::Generator,
        "Square Bass",
        &[(0, 0.2), (1, 1.0), (2, 0.002), (3, 0.2), (4, 0.7), (5, 0.05), (6, 18.0), (7, 3.0), (8, 0.35)],
    );
    let [_, bass_filter, bass_end] = effects(
        p,
        bass,
        [
            (K::Distortion, "Bass Clip", &[(0, 12.0), (1, 0.55), (2, 1.0), (3, 1.0)]),
            (K::FilterPro, "Bass Filter", &[(1, 2400.0), (2, 1.2), (4, 1.0)]),
            (K::Compressor, "Bass Squash", &[(0, 0.3), (1, 6.0), (2, 0.002), (3, 0.06), (4, 1.3)]),
        ],
    );

    // The stabs: a narrowing pulse table, five voices wide, driven,
    // phased and echoed.
    let stab = add(
        p,
        K::Wavetable,
        "Pulse Stabs",
        &[
            (0, 0.36),
            (1, 1.0),
            (2, 0.3),
            (3, 0.4),
            (4, 5.0),
            (5, 22.0),
            (6, 0.85),
            (7, 0.002),
            (8, 0.18),
            (9, 0.2),
            (10, 0.12),
        ],
    );
    effects(
        p,
        stab,
        [
            (K::Distortion, "Stab Drive", &[(0, 6.0), (1, 0.7), (2, 0.6)]),
            (K::Phaser, "Stab Phase", &[(0, 0.3), (1, 0.7), (6, 0.35)]),
            (K::Delay, "Stab Echo", &[(0, 3.0), (1, 0.3), (2, 0.2)]),
        ],
    );

    // The organ: hollow, odd harmonics, in a convolved hall.
    let organ = add(
        p,
        K::SpectraVoice,
        "Hymn Organ",
        &[(0, 0.3), (1, 16.0), (2, 0.6), (3, 0.35), (5, 0.1), (6, 0.08), (7, 0.5), (8, 0.9), (9, 1.4)],
    );
    effects(
        p,
        organ,
        [
            (K::Distortion, "Organ Warmth", &[(0, 2.0), (1, 0.6), (2, 0.3)]),
            (K::Convolver, "Organ Hall", &[(0, 1.0), (1, 0.4), (2, 1.0)]),
        ],
    );

    // The siren: a saw that a Glide slides between its notes, legato.
    let siren = p.add_module(K::Glide, [0.0, 0.0]).unwrap();
    set(p, siren, "Siren Glide", &[(0, 1.0), (1, 0.14)]);
    let siren_saw = add(p, K::Generator, "Siren", &[(0, 0.22), (2, 0.02), (4, 0.9), (5, 0.3), (6, 10.0), (7, 2.0)]);
    p.connect(siren, siren_saw);
    effects(
        p,
        siren_saw,
        [
            (K::Distortion, "Siren Drive", &[(0, 4.0), (1, 0.7), (2, 0.5)]),
            (K::Delay, "Siren Echo", &[(0, 3.0), (1, 0.35), (2, 0.25)]),
            (K::Convolver, "Siren Plate", &[(0, 2.0), (1, 0.25)]),
        ],
    );

    // The riser: noise through a band-pass that sweeps up.
    let riser = add(p, K::Generator, "Riser", &[(0, 0.16), (1, 4.0), (2, 2.0), (4, 1.0), (5, 0.4)]);
    let [riser_filter] = effects(p, riser, [(K::Filter, "Riser Sweep", &[(0, 2.0), (1, 400.0), (2, 0.6)])]);

    // The pump: a Modulator following the kick pulls the bass, the stabs,
    // the organ and the siren down under every hit.
    let pump = p.add_module(K::Modulator, [0.0, 0.0]).unwrap();
    set(p, pump, "Pump", &[(0, 1.0), (5, -0.6), (6, 0.002), (7, 0.2)]);
    let kick_end = p.chain(kick).effects.last().copied().unwrap_or(kick);
    p.connect(kick_end, pump);
    for target in [bass, stab, organ, siren_saw] {
        let fader = p.module(target).unwrap().kind.params().len();
        p.connect(pump, target);
        p.set_control_param(pump, target, fader);
    }

    Ids { kick, kit: kit_id, bass, bass_filter, bass_end, stab, organ, siren, riser, riser_filter, drive }
}

// ---------------------------------------------------------------- parts

fn pattern() -> Pattern {
    let mut pat = Pattern::new("", TRACKS, LINES);
    for (t, cols) in COLUMNS {
        pat.set_columns(t, cols);
    }
    pat
}

/// Whether line `l` falls in a dead stop: the last beat of bars four and
/// eight, where everything but the organ cuts out.
fn stopped(l: usize) -> bool {
    l % (4 * BAR) >= 4 * BAR - LPB
}

/// The kick on every beat through `bars`, with the stops left silent.
fn kicks(pat: &mut Pattern, ids: &Ids, bars: std::ops::Range<usize>, stops: bool) {
    for l in (bars.start * BAR..bars.end * BAR).step_by(LPB) {
        if !(stops && stopped(l)) {
            pat.tracks[KICK][l] = n(KICK_NOTE, ids.kick, 0x78);
        }
    }
}

/// The kit through `bars`: clap and snare on two and four, sixteenth hats
/// with an open one on each off-beat, and the stops.
fn kit_part(pat: &mut Pattern, ids: &Ids, bars: std::ops::Range<usize>) {
    for l in bars.start * BAR..bars.end * BAR {
        if stopped(l) {
            continue;
        }
        let beat = l % LPB;
        if l % BAR == 4 || l % BAR == 12 {
            pat.tracks[KIT][l] = n(CLAP, ids.kit, 0x70);
            *pat.cell_mut(KIT, 1, l) = n(SNARE, ids.kit, 0x58);
        }
        let hat = if beat == 2 { n(OPEN, ids.kit, 0x48) } else { n(HAT, ids.kit, if beat == 0 { 0x40 } else { 0x28 }) };
        *pat.cell_mut(KIT, 2, l) = hat;
    }
    // A crash on the one, fading out from the volume column (O3).
    *pat.cell_mut(KIT, 3, bars.start * BAR) = Cell { vol: vol_command_value('O', 3), ..n(CRASH, ids.kit, 0) };
}

/// The riff through `bars`, on each two bars' chord; at the stops the
/// bass is let go.
fn riff(pat: &mut Pattern, ids: &Ids, bars: std::ops::Range<usize>, stops: bool) {
    for b in bars {
        let root = ROOTS[b / 2 % ROOTS.len()];
        for &(at, up, bend) in &RIFF {
            let l = b * BAR + at;
            if stops && stopped(l) {
                if at % LPB == 0 {
                    pat.tracks[BASS][l] = off();
                }
                continue;
            }
            let vol = if at % LPB == 0 { 0x78 } else { 0x60 };
            let cell = n(root + up, ids.bass, vol);
            pat.tracks[BASS][l] = if bend { fx(cell, 0x1, 0x20) } else { fx(cell, 0xC, 0x04) };
        }
    }
}

/// The stabs on each two bars' chord, through `bars`.
fn stabs(pat: &mut Pattern, ids: &Ids, bars: std::ops::Range<usize>, vol: u8) {
    for b in bars {
        let chord = ORGAN[b / 2 % ORGAN.len()];
        for (k, c) in STABS.chars().enumerate() {
            let l = b * BAR + k;
            if c != 'x' || stopped(l) {
                continue;
            }
            for (col, &note) in chord.iter().enumerate() {
                *pat.cell_mut(STAB, col, l) = fx(n(note + 12, ids.stab, vol), 0xC, 0x03);
            }
        }
    }
}

/// The organ's chords, one every two bars, from bar `from`.
fn organ(pat: &mut Pattern, ids: &Ids, from: usize, vol: u8) {
    for b in (from..8).step_by(2) {
        let chord = ORGAN[b / 2 % ORGAN.len()];
        for (col, &note) in chord.iter().enumerate() {
            *pat.cell_mut(ORGAN_T, col, b * BAR) = n(note, ids.organ, vol);
        }
    }
}

/// The riser over the last `bars` of the pattern: noise swelling as its
/// band-pass sweeps up, let go on the last line.
fn riser(pat: &mut Pattern, ids: &Ids, bars: usize) {
    let from = LINES - bars * BAR;
    pat.tracks[FX][from] = n(60, ids.riser, 0x70);
    pat.tracks[FX][LINES - 1] = off();
    let f = ModuleKind::Filter;
    let sweep = [(0.0, 300.0), (from as f32, 300.0), (LINES as f32, 9000.0)];
    pat.automation.push(envelope(ids.riser_filter, f, 1, &sweep, false, true));
}

// ---------------------------------------------------------------- sections

/// The organ alone in its hall, the riser coming in behind it.
fn hymn(ids: &Ids) -> Pattern {
    let mut pat = pattern();
    organ(&mut pat, ids, 0, 0x68);
    riser(&mut pat, ids, 2);
    pat
}

/// The riff over the kick, its filter opening; the organ lets go.
fn riff_in(ids: &Ids) -> Pattern {
    let mut pat = pattern();
    for col in 0..4 {
        *pat.cell_mut(ORGAN_T, col, 0) = off();
    }
    kicks(&mut pat, ids, 4..8, false);
    riff(&mut pat, ids, 0..8, false);
    // It comes in held back and rises as its filter opens: its fader
    // after the clipping, which would level a quieter note back up.
    let squash = ModuleKind::Compressor;
    let fader = squash.params().len();
    pat.automation.push(envelope(ids.bass_end, squash, fader, &[(0.0, 0.35), (128.0, 0.7)], false, true));
    let fp = ModuleKind::FilterPro;
    let open = [(0.0, 200.0), (96.0, 2400.0), (128.0, 2400.0)];
    pat.automation.push(envelope(ids.bass_filter, fp, 1, &open, false, true));
    riser(&mut pat, ids, 1);
    pat
}

/// Everything at once, with its dead stops.
fn drop_part(ids: &Ids, second: bool) -> Pattern {
    let mut pat = pattern();
    kicks(&mut pat, ids, 0..8, true);
    kit_part(&mut pat, ids, 0..8);
    riff(&mut pat, ids, 0..8, true);
    stabs(&mut pat, ids, 0..8, if second { 0x40 } else { 0x58 });
    if second {
        // The stabs swell (Lxx), the organ stutters over the top (Txy),
        // and the master's drive turns up.
        for (k, l) in (0..LINES).step_by(2 * BAR).enumerate() {
            pat.tracks[STAB][l + 1].fx = Some((FX_TRACK_VOLUME, 0x50 + k as u8 * 0x10));
        }
        organ(&mut pat, ids, 0, 0x50);
        for col in 0..4 {
            for l in 0..LINES {
                if pat.cell(ORGAN_T, col, l).fx.is_none() {
                    pat.cell_mut(ORGAN_T, col, l).fx = Some((FX_TREMOR, 0x21));
                }
            }
        }
        let d = ModuleKind::Distortion;
        pat.automation.push(envelope(ids.drive, d, 0, &[(0.0, 1.6), (128.0, 3.0)], false, true));
    }
    pat
}

/// The break: the organ back, the siren gliding over it, the kick at half
/// time, and the riser climbing to the second drop.
fn break_part(ids: &Ids) -> Pattern {
    let mut pat = pattern();
    pat.tracks[BASS][0] = off();
    // The stabs swell back from quiet (L30) in the second drop.
    pat.tracks[STAB][0].fx = Some((FX_TRACK_VOLUME, 0x30));
    organ(&mut pat, ids, 0, 0x60);
    for (k, &note) in SIREN.iter().enumerate() {
        pat.tracks[LEAD][k * 2 * LPB] = n(note, ids.siren, 0x60);
    }
    pat.tracks[LEAD][LINES - 1] = off();
    for l in (4 * BAR..LINES).step_by(2 * LPB) {
        pat.tracks[KICK][l] = n(KICK_NOTE, ids.kick, 0x68);
    }
    riser(&mut pat, ids, 2);
    pat
}

/// The riff alone, then the organ's last chord ringing out.
fn outro(ids: &Ids) -> Pattern {
    let mut pat = pattern();
    for col in 0..4 {
        *pat.cell_mut(ORGAN_T, col, 0) = off();
    }
    riff(&mut pat, ids, 0..4, false);
    pat.tracks[BASS][4 * BAR] = off();
    let chord = ORGAN[0];
    for (col, &note) in chord.iter().enumerate() {
        *pat.cell_mut(ORGAN_T, col, 4 * BAR) = n(note, ids.organ, 0x70);
        *pat.cell_mut(ORGAN_T, col, 7 * BAR) = off();
    }
    pat.cell_mut(FX, 1, LINES - 1).fx = Some((0xF, 0x00));
    pat
}

const COMMENTS: &str = "\
Concrete Hymn: overdriven electro in F♯ minor at 118 BPM.\n\
\n\
    • Hymn Organ: a SpectraVoice of hollow, odd harmonics in a Convolver's hall.\n\
    • Square Bass: three detuned squares clipped hard; the riff bends with 1xx and is cut short with Cxx. Its \
    Filter Pro opens over the second pattern.\n\
    • Pump: a Modulator following the kick pulls the bass, stabs, organ and siren down under every hit.\n\
    • Pulse Stabs: a Wavetable on its Pulse table, five voices wide. In the second drop they swell with Lxx \
    while the organ stutters with Txy and the Master Drive turns up.\n\
    • Siren: a saw played through a Glide in Legato mode, so it slides between notes.\n\
    • Grit Kit: a Sampler with a sound on each key, bitcrushed; its crash fades out with O3 in the volume column.\n\
    • The drops stop dead on the last beat of every fourth bar.";

impl Project {
    pub fn concrete_hymn() -> Self {
        let mut p = Self::empty();
        p.title = "Concrete Hymn".into();
        p.artist = "noise".into();
        p.comments = COMMENTS.into();
        p.bpm = BPM;
        p.modules[0].params[0] = 0.8;
        let names = ["Kick", "Kit", "Bass", "Stabs", "Organ", "Siren", "FX"];
        let colors = [
            [255, 90, 60],
            [255, 160, 120],
            [250, 210, 60],
            [120, 230, 255],
            [200, 170, 255],
            [255, 80, 160],
            [190, 190, 190],
        ];
        for t in 0..TRACKS {
            if t >= p.tracks.len() {
                break;
            }
            p.tracks[t].name = names[t].into();
            p.tracks[t].color = Some(colors[t]);
        }
        let ids = instruments(&mut p);
        let mut patterns = vec![
            hymn(&ids),
            riff_in(&ids),
            drop_part(&ids, false),
            break_part(&ids),
            drop_part(&ids, true),
            outro(&ids),
        ];
        for (pat, name) in patterns.iter_mut().zip(["Hymn", "Riff", "Drop", "Break", "Drop 2", "Outro"]) {
            pat.name = name.into();
        }
        p.patterns = patterns;
        for (t, cols) in COLUMNS {
            p.set_columns(t, cols);
        }
        p.order = (0..6).map(Slot::new).collect();
        p.sections = [("Hymn", 0), ("Riff", 1), ("Drop", 2), ("Break", 3), ("Drop 2", 4), ("Outro", 5)]
            .into_iter()
            .map(|(name, start)| Section { name: name.into(), start })
            .collect();
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concrete_hymn_pumps_and_reaches_the_output() {
        let p = Project::concrete_hymn();
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
        let pump = p.modules.iter().find(|m| m.name == "Pump").unwrap();
        assert_eq!(p.links.iter().filter(|l| l.0 == pump.id).count(), 4, "it ducks four instruments");
        let kit = p.modules.iter().find(|m| m.kind == ModuleKind::Sampler).unwrap();
        assert!(kit.samples.iter().all(|s| s.data.is_some() && s.oneshot));
    }

    #[test]
    fn concrete_hymn_plays_loud_enough_without_clipping_and_ends() {
        let frames = crate::audio::render(std::sync::Arc::new(Project::concrete_hymn()), 8000, 1.0, false);
        let peak = frames.iter().fold(0f32, |a, f| a.max(f[0].abs()).max(f[1].abs()));
        assert!(peak > 0.5 && peak <= 1.0, "{peak}");
        let secs = frames.len() as f32 / 8000.0;
        assert!((70.0..130.0).contains(&secs), "{secs} s");
    }
}
