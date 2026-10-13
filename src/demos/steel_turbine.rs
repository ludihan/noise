//! "Steel Turbine", a demo song: a minute and a half of schranz in A
//! minor at 152 BPM, hard, looped and hypnotic, built from percussion more
//! than notes.
//!
//! A clipped kick and hats come in under a loop of metal that grinds
//! through a swept band-pass; an off-beat rumble, a flanged clap and ride
//! and the rim join, then tribal toms and a distorted stab echoing on the
//! off-beats. The break takes the kick away and closes the master's filter
//! over a snare roll; the peaks come back with everything at once, the
//! sweep faster, the mix stuttering at the ends of phrases and the hats
//! left partly to chance; the outro strips it back to the kick, which
//! stops dead.
//!
//! The kit is rendered here, so its Sampler has audio without files; the
//! samples are written next to the song when it is saved.

use super::*;
use crate::dsp::Frame;
use crate::project::*;
use crate::sample::Sample;

const BPM: f32 = 152.0;
const LPB: usize = 4;
/// Lines a bar, and a pattern of eight bars.
const BAR: usize = 4 * LPB;
const LINES: usize = 8 * BAR;

/// The kick's note, A.
const KICK_NOTE: u8 = 33;
/// The grinding loop's accents over a bar: `X` loud, `x` soft.
const GRIND: &str = "XxxXxxXxXxxXxxXx";
/// The toms over a bar: `H` high, `L` low.
const TOMS: &str = "..H..L.H..L.H.LL";
/// The stab's hits over two bars, on the off-beats and pushing.
const STABS: &str = "..x...x...x..x....x...x...x.x..x";
/// The stab's chord, A minor with the seventh on top.
const STAB_CHORD: [u8; 4] = [57, 60, 64, 67];

// ---------------------------------------------------------------- the kit

const CLAP: u8 = 48;
const SNARE: u8 = 50;
const RIM: u8 = 52;
const HAT: u8 = 54;
const OPEN: u8 = 58;
const RIDE: u8 = 59;
const CRASH: u8 = 61;

/// Each sound of the kit on a key of its own; the hats choke each other.
fn kit(sr: f32) -> Vec<SampleSlot> {
    let sounds = [
        ("Clap", hit(sr, Hit::Clap, 0x3C1A, 0.15), CLAP, 0, 0.0),
        ("Snare", hit(sr, Hit::Snare, 0x3A2E, 0.05), SNARE, 0, 0.0),
        ("Rim", hit(sr, Hit::Rim, 0x3131, 0.0), RIM, 0, 0.25),
        ("Hat", hit(sr, Hit::Hat, 0x3A11, 0.0), HAT, 1, -0.2),
        ("Open Hat", hit(sr, Hit::Open, 0x3B3E, 0.0), OPEN, 1, -0.2),
        ("Ride", hit(sr, Hit::Ride, 0x31DE, 0.0), RIDE, 0, 0.3),
        ("Crash", hit(sr, Hit::Crash, 0x34A5, 0.0), CRASH, 0, 0.0),
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
const RUMBLE_T: usize = 1;
const PERC: usize = 2;
const GRIND_T: usize = 3;
const TOMS_T: usize = 4;
const STAB: usize = 5;
const FX: usize = 6;
const TRACKS: usize = 7;

/// Note columns per track, where more than one.
const COLUMNS: [(usize, usize); 3] = [(PERC, 4), (STAB, 4), (FX, 2)];

struct Ids {
    kick: u8,
    rumble: u8,
    kit: u8,
    grind: u8,
    sweep: u8,
    toms: u8,
    stab: u8,
    master_filter: u8,
    stutter: u8,
    riser: u8,
    riser_filter: u8,
}

fn instruments(p: &mut Project) -> Ids {
    use ModuleKind as K;
    let add = |p: &mut Project, kind: ModuleKind, name: &str, params: &[(usize, f32)]| {
        let id = super::add(p, kind, name, params);
        p.connect(id, OUTPUT_ID);
        id
    };

    // The master: a filter for the break, a stutter, then driven, glued
    // and limited.
    let [master_filter, stutter, ..] = effects(
        p,
        OUTPUT_ID,
        [
            (K::FilterPro, "Turbine Filter", &[(1, 20000.0), (2, 1.2), (4, 1.0)]),
            (K::Repeater, "Turbine Stutter", &[]),
            (K::Distortion, "Turbine Drive", &[(0, 1.6), (1, 0.6), (2, 0.4)]),
            (K::Compressor, "Turbine Glue", &[(0, 0.45), (1, 3.0), (2, 0.003), (3, 0.1), (4, 1.3)]),
            (K::Maximizer, "Turbine Limit", &[(0, 1.7), (1, 0.95), (2, 0.05)]),
        ],
    );

    // The kick: a short sine thump onto A, clipped hard and punched.
    let kick = add(p, K::Kicker, "Steel Kick", &[(0, 0.8), (2, 3.5), (3, 0.03), (5, 0.28), (6, 0.7)]);
    effects(
        p,
        kick,
        [
            (K::Distortion, "Steel Clip", &[(0, 9.0), (1, 0.5), (2, 0.8), (3, 1.0)]),
            (K::Compressor, "Steel Punch", &[(0, 0.4), (1, 4.0), (2, 0.006), (3, 0.06), (4, 1.3)]),
        ],
    );

    // The rumble: a low thump on the off-beats smeared by a dark reverb and
    // cut to the sub, so the low end never stops.
    let rumble = add(p, K::Kicker, "Rumble", &[(0, 0.6), (2, 1.5), (3, 0.05), (5, 0.6), (6, 0.4)]);
    let [.., rumble_sub] = effects(
        p,
        rumble,
        [
            (K::Reverb, "Rumble Smear", &[(0, 0.7), (1, 0.8), (2, 0.6)]),
            (K::Distortion, "Rumble Grit", &[(0, 4.0), (1, 0.3), (2, 0.6)]),
            (K::FilterPro, "Rumble Sub", &[(1, 140.0), (2, 0.9), (4, 2.0)]),
        ],
    );
    // The levels after the distortions, which level whatever goes in.
    p.module_mut(rumble_sub).unwrap().gain = 0.25;

    // The kit: crushed a little, squashed and flanged, as a looped record.
    let kit_id = add(p, K::Sampler, "Turbine Kit", &[(0, 0.9), (6, 0.05)]);
    p.module_mut(kit_id).unwrap().samples = kit(44100.0);
    effects(
        p,
        kit_id,
        [
            (K::Distortion, "Kit Crush", &[(0, 2.5), (1, 0.65), (2, 0.5), (4, 10.0)]),
            (K::Compressor, "Kit Squash", &[(0, 0.3), (1, 5.0), (2, 0.002), (3, 0.08), (4, 1.4)]),
            (K::Flanger, "Kit Jet", &[(1, 0.004), (2, 0.7), (3, 0.12), (4, 0.6), (5, 0.35)]),
        ],
    );

    // The grind: bursts of noise through a resonant band-pass that a
    // Modulator sweeps over four bars, ring-modulated to metal and clipped.
    let grind = add(p, K::Generator, "Grinder", &[(0, 0.9), (1, 4.0), (2, 0.001), (3, 0.06), (4, 0.0), (5, 0.03)]);
    let [band, _, grind_clip] = effects(
        p,
        grind,
        [
            (K::Filter, "Grinder Band", &[(0, 2.0), (1, 1500.0), (2, 0.8)]),
            (K::RingMod, "Grinder Metal", &[(0, 220.0), (3, 0.4)]),
            (K::Distortion, "Grinder Clip", &[(0, 12.0), (1, 0.55), (2, 0.8), (3, 1.0)]),
        ],
    );
    p.module_mut(grind_clip).unwrap().gain = 2.0;
    let sweep = super::add(p, K::Modulator, "Grinder Sweep", &[(0, 0.0), (1, 1.0), (3, 1.0), (4, 64.0), (5, 0.4)]);
    p.connect(sweep, band);
    p.set_control_param(sweep, band, 1);
    p.module_mut(sweep).unwrap().home = Some(Owner::Module(grind));

    // The toms: a Kicker tuned up and short, in a little room.
    let toms = add(p, K::Kicker, "Tribal Toms", &[(0, 0.55), (2, 1.0), (3, 0.04), (5, 0.18), (6, 0.3)]);
    effects(p, toms, [(K::Reverb, "Tom Room", &[(0, 0.4), (1, 0.6), (2, 0.2)])]);

    // The stab: three saws a voice, hard clipped, through a lowpass and a
    // dotted echo.
    let stab = add(
        p,
        K::Generator,
        "Steel Stab",
        &[(0, 0.18), (1, 0.0), (2, 0.001), (3, 0.12), (4, 0.0), (5, 0.08), (6, 18.0), (7, 3.0)],
    );
    effects(
        p,
        stab,
        [
            (K::Distortion, "Stab Clip", &[(0, 10.0), (1, 0.55), (2, 0.9), (3, 1.0)]),
            (K::FilterPro, "Stab Low", &[(1, 2600.0), (2, 1.5), (4, 1.0)]),
            (K::Delay, "Stab Echo", &[(0, 3.0), (1, 0.45), (2, 0.3), (3, 0.8)]),
        ],
    );

    // The riser: noise through a band-pass that sweeps up.
    let riser = add(p, K::Generator, "Turbine Riser", &[(0, 0.35), (1, 4.0), (2, 2.0), (4, 1.0), (5, 0.4)]);
    let [riser_filter] = effects(p, riser, [(K::Filter, "Riser Band", &[(0, 2.0), (1, 400.0), (2, 0.6)])]);

    // The pump: the grind, the stab and the rumble duck under the kick.
    let pump = super::add(p, K::Modulator, "Turbine Pump", &[(0, 1.0), (5, -0.6), (6, 0.002), (7, 0.14)]);
    let kick_end = p.chain(kick).effects.last().copied().unwrap_or(kick);
    p.connect(kick_end, pump);
    for target in [grind, stab] {
        let fader = p.module(target).unwrap().kind.params().len();
        p.connect(pump, target);
        p.set_control_param(pump, target, fader);
    }

    Ids { kick, rumble, kit: kit_id, grind, sweep, toms, stab, master_filter, stutter, riser, riser_filter }
}

// ---------------------------------------------------------------- parts

fn pattern() -> Pattern {
    let mut pat = Pattern::new("", TRACKS, LINES);
    for (t, cols) in COLUMNS {
        pat.set_columns(t, cols);
    }
    pat
}

/// The kick on every beat through `bars`, and with `rumble` the rumble on
/// every off-beat.
fn kicks(pat: &mut Pattern, ids: &Ids, bars: std::ops::Range<usize>, rumble: bool) {
    for l in (bars.start * BAR..bars.end * BAR).step_by(LPB) {
        pat.tracks[KICK][l] = n(KICK_NOTE, ids.kick, 0x78);
        if rumble {
            pat.tracks[RUMBLE_T][l + 2] = n(KICK_NOTE - 12, ids.rumble, 0x60);
        }
    }
}

/// The hats through `bars`: an open one on every off-beat, closed ones in
/// sixteenths between; with `chance`, the closed ones only now and then
/// (Yxx).
fn hats(pat: &mut Pattern, ids: &Ids, bars: std::ops::Range<usize>, chance: bool) {
    for l in bars.start * BAR..bars.end * BAR {
        let cell = if l % LPB == 2 {
            n(OPEN, ids.kit, 0x50)
        } else {
            let hat = n(HAT, ids.kit, if l % 2 == 0 { 0x38 } else { 0x26 });
            if chance { fx(hat, FX_MAYBE, 0xA0) } else { hat }
        };
        *pat.cell_mut(PERC, 1, l) = cell;
    }
}

/// The clap on two and four, the ride on every beat and the rim pushing,
/// through `bars`.
fn groove(pat: &mut Pattern, ids: &Ids, bars: std::ops::Range<usize>) {
    for b in bars {
        let at = b * BAR;
        for l in [4, 12] {
            *pat.cell_mut(PERC, 0, at + l) = n(CLAP, ids.kit, 0x68);
        }
        for l in (0..BAR).step_by(LPB) {
            *pat.cell_mut(PERC, 2, at + l) = n(RIDE, ids.kit, 0x34);
        }
        for l in [3, 7, 10, 14] {
            *pat.cell_mut(PERC, 3, at + l) = n(RIM, ids.kit, 0x48);
        }
    }
}

/// The grinding loop through `bars`, its accents in the volume column.
fn grind(pat: &mut Pattern, ids: &Ids, bars: std::ops::Range<usize>) {
    for b in bars {
        for (k, c) in GRIND.chars().enumerate() {
            let vol = if c == 'X' { 0x78 } else { 0x40 };
            pat.tracks[GRIND_T][b * BAR + k] = n(60, ids.grind, vol);
        }
    }
}

/// The tribal toms through `bars`.
fn toms(pat: &mut Pattern, ids: &Ids, bars: std::ops::Range<usize>) {
    for b in bars {
        for (k, c) in TOMS.chars().enumerate() {
            let note = match c {
                'H' => 50,
                'L' => 43,
                _ => continue,
            };
            pat.tracks[TOMS_T][b * BAR + k] = n(note, ids.toms, if k % 4 == 2 { 0x68 } else { 0x50 });
        }
    }
}

/// The stab's chord on its two-bar rhythm through `bars`, cut short (Cxx).
fn stabs(pat: &mut Pattern, ids: &Ids, bars: std::ops::Range<usize>, vol: u8) {
    for b in bars {
        for (k, c) in STABS.chars().enumerate().filter(|&(k, _)| k / BAR == b % 2) {
            if c != 'x' {
                continue;
            }
            for (col, &note) in STAB_CHORD.iter().enumerate() {
                let l = b * BAR + k % BAR;
                *pat.cell_mut(STAB, col, l) = fx(n(note, ids.stab, vol), 0xC, 0x03);
            }
        }
    }
}

/// A snare roll over the last `bars`, from eighths to every other tick
/// (Exx), rising.
fn snare_roll(pat: &mut Pattern, ids: &Ids, bars: usize) {
    let from = LINES - bars * BAR;
    let half = bars * BAR / 2;
    for l in (from..from + half).step_by(2) {
        *pat.cell_mut(PERC, 0, l) = n(SNARE, ids.kit, 0x30 + ((l - from) * 0x30 / half) as u8);
    }
    roll(pat, PERC, from + half, half, n(SNARE, ids.kit, 0x50), 2, 0x7C);
}

/// The riser over the last `bars`, its band sweeping up.
fn riser(pat: &mut Pattern, ids: &Ids, bars: usize) {
    let from = LINES - bars * BAR;
    pat.tracks[FX][from] = n(60, ids.riser, 0x70);
    pat.tracks[FX][LINES - 1] = off();
    let f = ModuleKind::Filter;
    let sweep = [(0.0, 300.0), (from as f32, 300.0), (LINES as f32, 9000.0)];
    pat.automation.push(envelope(ids.riser_filter, f, 1, &sweep, false, true));
}

/// A crash with the kick on the downbeat.
fn crash(pat: &mut Pattern, ids: &Ids) {
    *pat.cell_mut(PERC, 0, 0) = Cell { vol: vol_command_value('O', 3), ..n(CRASH, ids.kit, 0) };
}

// ---------------------------------------------------------------- sections

/// The kick and the hats, the grind coming up through its filter.
fn intro(ids: &Ids) -> Pattern {
    let mut pat = pattern();
    kicks(&mut pat, ids, 0..8, false);
    hats(&mut pat, ids, 2..8, false);
    grind(&mut pat, ids, 4..8);
    // The sweep opens up as the grind comes in.
    pat.automation.push(envelope(ids.sweep, ModuleKind::Modulator, 5, &[(64.0, 0.0), (128.0, 0.4)], false, true));
    pat
}

/// The rumble, the clap, ride and rim join; with `tribal`, the toms and the
/// stab too.
fn main_part(ids: &Ids, tribal: bool) -> Pattern {
    let mut pat = pattern();
    kicks(&mut pat, ids, 0..8, true);
    hats(&mut pat, ids, 0..8, false);
    groove(&mut pat, ids, 0..8);
    grind(&mut pat, ids, 0..8);
    if tribal {
        toms(&mut pat, ids, 0..8);
        stabs(&mut pat, ids, 4..8, 0x58);
    }
    pat
}

/// The break: the kick and the rumble out, the master's filter closing
/// over the grind, the stab and the toms, then opening again under a
/// snare roll and the riser.
fn break_part(ids: &Ids) -> Pattern {
    let mut pat = pattern();
    hats(&mut pat, ids, 0..6, false);
    grind(&mut pat, ids, 0..8);
    toms(&mut pat, ids, 0..4);
    stabs(&mut pat, ids, 0..8, 0x60);
    snare_roll(&mut pat, ids, 2);
    riser(&mut pat, ids, 2);
    let f = ModuleKind::FilterPro;
    let close = [(0.0, 20000.0), (32.0, 600.0), (96.0, 400.0), (128.0, 20000.0)];
    pat.automation.push(envelope(ids.master_filter, f, 1, &close, false, true));
    pat
}

/// The peak: everything at once, the sweep twice as fast, a crash on the
/// one and the mix stuttering at the end of every fourth bar; the second
/// time the closed hats are left to chance.
fn peak(ids: &Ids, second: bool) -> Pattern {
    let mut pat = pattern();
    crash(&mut pat, ids);
    kicks(&mut pat, ids, 0..8, true);
    hats(&mut pat, ids, 0..8, second);
    groove(&mut pat, ids, 0..8);
    grind(&mut pat, ids, 0..8);
    toms(&mut pat, ids, 0..8);
    stabs(&mut pat, ids, 0..8, 0x60);
    let m = ModuleKind::Modulator;
    pat.automation.push(envelope(ids.sweep, m, 4, &[(0.0, 32.0)], true, false));
    let held = [(60.0, 64.0), (124.0, 128.0)];
    hold(&mut pat, ids.stutter, &held, &[(0.0, 4.0), (62.0, 6.0), (124.0, 5.0), (126.0, 8.0)]);
    pat
}

/// The outro: two bars at a time the stab, the toms, the groove and the
/// grind drop out, leaving the kick, which stops dead (F00).
fn outro(ids: &Ids) -> Pattern {
    let mut pat = pattern();
    kicks(&mut pat, ids, 0..8, true);
    for l in 6 * BAR..8 * BAR {
        pat.tracks[RUMBLE_T][l] = Cell::default();
    }
    hats(&mut pat, ids, 0..6, false);
    groove(&mut pat, ids, 0..4);
    grind(&mut pat, ids, 0..6);
    toms(&mut pat, ids, 0..2);
    stabs(&mut pat, ids, 0..2, 0x50);
    pat.cell_mut(FX, 1, LINES - 1).fx = Some((0xF, 0x00));
    pat
}

const COMMENTS: &str = "\
Steel Turbine: schranz in A minor at 152 BPM.\n\
\n\
    • Grinder: bursts of noise through a resonant Filter that Grinder Sweep, a Modulator synced to 64 lines, \
    moves; a Ring Mod makes it metal. The peaks automate the sweep's period to 32 lines.\n\
    • Steel Kick and Rumble: a clipped kick on the beat and, on the off-beats, a low Kicker smeared by a reverb \
    and cut to the sub by a Filter Pro.\n\
    • Turbine Kit: a Sampler with a sound on each key, crushed and flanged; the second peak leaves its closed \
    hats to chance with Yxx.\n\
    • Steel Stab: three clipped saws a voice, cut short with Cxx, into a dotted echo.\n\
    • Turbine Pump: a Modulator following the kick ducks the grind and the stab.\n\
    • The break closes the master's Turbine Filter and opens it again under a snare roll (Exx) and the riser; \
    Turbine Stutter, a Repeater on the master, holds the ends of the peaks' phrases.\n\
    • The outro takes the parts away two bars at a time and F00 stops the kick dead.";

impl Project {
    pub fn steel_turbine() -> Self {
        let mut p = Self::empty();
        p.title = "Steel Turbine".into();
        p.artist = "noise".into();
        p.comments = COMMENTS.into();
        p.bpm = BPM;
        p.lpb = LPB as u32;
        p.modules[0].params[0] = 0.8;
        let names = ["Kick", "Rumble", "Perc", "Grind", "Toms", "Stab", "FX"];
        let colors = [
            [255, 70, 50],
            [170, 60, 40],
            [255, 180, 100],
            [170, 180, 200],
            [210, 140, 90],
            [120, 220, 255],
            [190, 190, 190],
        ];
        for (t, (name, color)) in names.iter().zip(colors).enumerate() {
            p.tracks[t].name = name.to_string();
            p.tracks[t].color = Some(color);
        }
        let ids = instruments(&mut p);
        let mut patterns = vec![
            intro(&ids),
            main_part(&ids, false),
            main_part(&ids, true),
            break_part(&ids),
            peak(&ids, false),
            peak(&ids, true),
            outro(&ids),
        ];
        let names = ["Intro", "Groove", "Tribal", "Break", "Peak", "Peak 2", "Outro"];
        for (pat, name) in patterns.iter_mut().zip(names) {
            pat.name = name.into();
        }
        p.patterns = patterns;
        for (t, cols) in COLUMNS {
            p.set_columns(t, cols);
        }
        p.order = (0..7).map(Slot::new).collect();
        p.sections = [("Intro", 0), ("Groove", 1), ("Break", 3), ("Peak", 4), ("Outro", 6)]
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
    fn steel_turbine_reaches_the_output_and_ends() {
        let p = Project::steel_turbine();
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
        let last = &p.patterns[p.order.last().unwrap().pattern];
        assert_eq!(last.cell(FX, 1, LINES - 1).fx, Some((0xF, 0)));
    }

    #[test]
    fn steel_turbine_plays_loud_enough_without_clipping_and_ends() {
        let frames = crate::audio::render(std::sync::Arc::new(Project::steel_turbine()), 8000, 1.0, false);
        let peak = frames.iter().fold(0f32, |a, f| a.max(f[0].abs()).max(f[1].abs()));
        assert!(peak > 0.5 && peak <= 1.0, "{peak}");
        let secs = frames.len() as f32 / 8000.0;
        assert!((70.0..120.0).contains(&secs), "{secs} s");
    }
}
