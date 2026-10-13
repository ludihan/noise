//! "Rivet Storm", the sixth demo song: a minute and a half of hardcore in
//! G minor that starts as a hypnotic, pounding loop at 155 BPM and speeds
//! up until it comes apart at 255.
//!
//! A tight kick and a hammering loop of filtered noise come in first, the
//! filter breathing slowly over it, a metal ride and a clap joining. The
//! tempo climbs a step every bar to 190, and the kick turns into a
//! distorted, screaming one that plays the tune itself. A hoover glides
//! over the top; the kick drops out for it, a snare roll speeds up under a
//! riser, and the kicks come back twice as fast, the hoover stuttering. To
//! end, the tempo climbs to its limit, the kicks roll on every line and
//! the song stops dead.
//!
//! The kit is synthesized here, so its Sampler has audio without files;
//! the samples are written next to the song when it is saved.

use super::*;
use crate::dsp::Frame;
use crate::project::*;
use crate::sample::Sample;

const BPM: f32 = 155.0;
const LPB: usize = 4;
/// Lines a bar, and a pattern of eight bars.
const BAR: usize = 4 * LPB;
const LINES: usize = 8 * BAR;

// ---------------------------------------------------------------- the tune

/// The tight kick's note, G.
const TIGHT_NOTE: u8 = 43;
/// The gabber kick's tune over two bars, a note a beat: it bends between
/// G, B♭, F and C.
const KICK_TUNE: [u8; 8] = [31, 31, 34, 31, 29, 29, 36, 34];
/// The hoover's line, a note every two beats over four bars, gliding from
/// one into the next.
const HOOVER: [u8; 8] = [67, 70, 74, 72, 70, 69, 67, 62];
/// The hammer's accents over a bar: `X` loud, `x` soft, `.` nothing.
const HAMMER: &str = "X.xX.xX.xX.x.Xxx";
/// The rim's off-beat figure over a bar.
const RIM: &str = "..x..x....x..x.x";

// ---------------------------------------------------------------- the kit

/// The kit's keys.
const CLAP: u8 = 48;
const SNARE: u8 = 50;
const RIMSHOT: u8 = 52;
const HAT: u8 = 54;
const OPEN: u8 = 58;
const RIDE: u8 = 59;
const CRASH: u8 = 61;

/// Each sound of the kit on a key of its own; the hats choke each other.
fn kit(sr: f32) -> Vec<SampleSlot> {
    let sounds = [
        ("Clap", hit(sr, Hit::Clap, 0x7C1A, 0.1), CLAP, 0, 0.0),
        ("Snare", hit(sr, Hit::Snare, 0x5A2E, 0.05), SNARE, 0, 0.0),
        ("Rim", hit(sr, Hit::Rim, 0x2131, 0.0), RIMSHOT, 0, 0.2),
        ("Hat", hit(sr, Hit::Hat, 0x4A11, 0.0), HAT, 1, -0.15),
        ("Open Hat", hit(sr, Hit::Open, 0x0B3E, 0.0), OPEN, 1, -0.15),
        ("Ride", hit(sr, Hit::Ride, 0x51DE, 0.0), RIDE, 0, 0.25),
        ("Crash", hit(sr, Hit::Crash, 0xC4A5, 0.0), CRASH, 0, 0.0),
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
const PERC: usize = 1;
const HAMMER_T: usize = 2;
const LEAD: usize = 3;
const FX: usize = 4;
const TRACKS: usize = 5;

/// Note columns per track, where more than one.
const COLUMNS: [(usize, usize); 2] = [(PERC, 4), (FX, 2)];

/// The instruments and effects the patterns refer to.
struct Ids {
    tight: u8,
    gabber: u8,
    gabber_drive: u8,
    kit: u8,
    hammer: u8,
    hammer_filter: u8,
    hoover: u8,
    stutter: u8,
    riser: u8,
    riser_filter: u8,
}

fn instruments(p: &mut Project) -> Ids {
    use ModuleKind as K;
    // Everything here goes to the output unless sent elsewhere.
    let add = |p: &mut Project, kind: ModuleKind, name: &str, params: &[(usize, f32)]| {
        let id = super::add(p, kind, name, params);
        p.connect(id, OUTPUT_ID);
        id
    };

    // The master: driven, squashed and limited flat out.
    effects(
        p,
        OUTPUT_ID,
        [
            (K::Distortion, "Storm Drive", &[(0, 1.8), (1, 0.6), (2, 0.4)]),
            (K::Compressor, "Storm Glue", &[(0, 0.45), (1, 3.0), (2, 0.003), (3, 0.1), (4, 1.3)]),
            (K::Maximizer, "Storm Limiter", &[(0, 1.8), (1, 0.95), (2, 0.05)]),
        ],
    );

    // The tight kick: a short sine thump, lightly clipped, for the loop.
    let tight = add(p, K::Kicker, "Tight Kick", &[(0, 0.8), (2, 3.0), (3, 0.03), (5, 0.22), (6, 0.5)]);
    effects(
        p,
        tight,
        [
            (K::Distortion, "Tight Clip", &[(0, 3.0), (1, 0.55), (2, 0.6), (3, 1.0)]),
            (K::Compressor, "Tight Punch", &[(0, 0.4), (1, 4.0), (2, 0.006), (3, 0.06), (4, 1.3)]),
        ],
    );

    // The gabber kick: a long sine falling five octaves onto the tune's
    // note, clipped to a square and screamed through a resonant lowpass.
    let gabber = add(p, K::Kicker, "Gabber Kick", &[(0, 0.8), (2, 5.0), (3, 0.05), (5, 0.4), (6, 0.9)]);
    let [gabber_drive, ..] = effects(
        p,
        gabber,
        [
            (K::Distortion, "Gabber Drive", &[(0, 30.0), (1, 0.45), (2, 1.0), (3, 1.0)]),
            (K::ScreamFilter, "Gabber Scream", &[(1, 1400.0), (2, 0.55), (3, 5.0), (4, 0.8)]),
            (K::Compressor, "Gabber Squash", &[(0, 0.35), (1, 6.0), (2, 0.002), (3, 0.08), (4, 1.2)]),
        ],
    );

    // The kit, crushed and pushed.
    let kit_id = add(p, K::Sampler, "Rivet Kit", &[(0, 0.9), (6, 0.05)]);
    p.module_mut(kit_id).unwrap().samples = kit(44100.0);
    effects(
        p,
        kit_id,
        [
            (K::Distortion, "Rivet Grit", &[(0, 2.5), (1, 0.65), (2, 0.6), (4, 11.0)]),
            (K::Compressor, "Rivet Squash", &[(0, 0.3), (1, 5.0), (2, 0.002), (3, 0.08), (4, 1.4)]),
        ],
    );

    // The hammer: short bursts of noise through a resonant band-pass that
    // a slow LFO sweeps, ring-modulated to metal.
    let hammer = add(p, K::Generator, "Hammer", &[(0, 0.9), (1, 4.0), (2, 0.001), (3, 0.07), (4, 0.0), (5, 0.04)]);
    let [hammer_filter, ..] = effects(
        p,
        hammer,
        [
            (K::Filter, "Hammer Band", &[(0, 2.0), (1, 1800.0), (2, 0.75)]),
            (K::RingMod, "Hammer Metal", &[(0, 310.0), (3, 0.35)]),
            (K::Distortion, "Hammer Clip", &[(0, 12.0), (1, 0.6), (2, 0.8), (3, 1.0)]),
        ],
    );
    let sweep = super::add(p, K::Modulator, "Hammer Sweep", &[(0, 0.0), (1, 0.0), (3, 1.0), (4, 64.0), (5, 0.35)]);
    p.connect(sweep, hammer_filter);
    p.set_control_param(sweep, hammer_filter, 1);
    p.module_mut(sweep).unwrap().home = Some(Owner::Module(hammer));

    // The hoover: four detuned saws a Glide slides between, chorused wide,
    // driven, stuttered by a Repeater and thrown into a big room.
    let glide = p.add_module(K::Glide, [0.0, 0.0]).unwrap();
    set(p, glide, "Hoover Glide", &[(0, 1.0), (1, 0.12)]);
    let hoover =
        add(p, K::Generator, "Hoover", &[(0, 0.28), (1, 0.0), (2, 0.01), (4, 0.9), (5, 0.25), (6, 35.0), (7, 4.0)]);
    p.connect(glide, hoover);
    let [.., stutter, _] = effects(
        p,
        hoover,
        [
            (K::Chorus, "Hoover Width", &[(0, 3.0), (1, 0.4), (2, 0.6), (4, 0.4), (6, 0.6)]),
            (K::Distortion, "Hoover Drive", &[(0, 3.0), (1, 0.6), (2, 0.5)]),
            (K::Repeater, "Hoover Stutter", &[(0, 0.0), (1, 4.0)]),
            (K::Reverb, "Warehouse", &[(0, 0.85), (1, 0.5), (2, 0.25)]),
        ],
    );

    // The riser: noise through a band-pass that sweeps up.
    let riser = add(p, K::Generator, "Storm Riser", &[(0, 0.35), (1, 4.0), (2, 2.0), (4, 1.0), (5, 0.4)]);
    let [riser_filter] = effects(p, riser, [(K::Filter, "Storm Sweep", &[(0, 2.0), (1, 400.0), (2, 0.6)])]);

    // The pump: a Modulator following both kicks pulls the hammer and the
    // hoover down under every hit.
    let pump = p.add_module(K::Modulator, [0.0, 0.0]).unwrap();
    set(p, pump, "Storm Pump", &[(0, 1.0), (5, -0.65), (6, 0.002), (7, 0.15)]);
    for kick in [tight, gabber] {
        let end = p.chain(kick).effects.last().copied().unwrap_or(kick);
        p.connect(end, pump);
    }
    for target in [hammer, hoover] {
        let fader = p.module(target).unwrap().kind.params().len();
        p.connect(pump, target);
        p.set_control_param(pump, target, fader);
    }

    Ids { tight, gabber, gabber_drive, kit: kit_id, hammer, hammer_filter, hoover: glide, stutter, riser, riser_filter }
}

// ---------------------------------------------------------------- parts

fn pattern(lines: usize) -> Pattern {
    let mut pat = Pattern::new("", TRACKS, lines);
    for (t, cols) in COLUMNS {
        pat.set_columns(t, cols);
    }
    pat
}

/// The tight kick on every beat through `bars`.
fn tight_kicks(pat: &mut Pattern, ids: &Ids, bars: std::ops::Range<usize>) {
    for l in (bars.start * BAR..bars.end * BAR).step_by(LPB) {
        pat.tracks[KICK][l] = n(TIGHT_NOTE, ids.tight, 0x78);
    }
}

/// The gabber kick through `bars`, every `every` lines, on the tune; the
/// last beat of every fourth bar left out but on the last bar.
fn gabber_kicks(pat: &mut Pattern, ids: &Ids, bars: std::ops::Range<usize>, every: usize) {
    let end = bars.end * BAR;
    for l in (bars.start * BAR..end).step_by(every) {
        let gap = l % (4 * BAR) >= 4 * BAR - LPB && l < end - BAR;
        if !gap {
            let note = KICK_TUNE[l / LPB % KICK_TUNE.len()];
            pat.tracks[KICK][l] = n(note, ids.gabber, 0x7C);
        }
    }
}

/// The hammering loop through `bars`, its accents in the volume column.
fn hammer(pat: &mut Pattern, ids: &Ids, bars: std::ops::Range<usize>) {
    for b in bars {
        for (k, c) in HAMMER.chars().enumerate() {
            let vol = match c {
                'X' => 0x78,
                'x' => 0x40,
                _ => continue,
            };
            pat.tracks[HAMMER_T][b * BAR + k] = n(60, ids.hammer, vol);
        }
    }
}

/// The loop's percussion through `bars`: open hats on the off-beats and
/// quiet closed ones between, the rim's figure, and with `full` the ride
/// and a clap on two and four.
fn loop_perc(pat: &mut Pattern, ids: &Ids, bars: std::ops::Range<usize>, full: bool) {
    for b in bars {
        for k in 0..BAR {
            let l = b * BAR + k;
            let hat = if k % LPB == 2 { n(OPEN, ids.kit, 0x50) } else { n(HAT, ids.kit, 0x22) };
            *pat.cell_mut(PERC, 1, l) = hat;
            if RIM.as_bytes()[k] == b'x' {
                *pat.cell_mut(PERC, 2, l) = n(RIMSHOT, ids.kit, 0x48);
            }
            if full && (k == 4 || k == 12) {
                *pat.cell_mut(PERC, 0, l) = n(CLAP, ids.kit, 0x68);
            }
            if full && k % LPB == 0 {
                *pat.cell_mut(PERC, 3, l) = n(RIDE, ids.kit, 0x30);
            }
        }
    }
}

/// The gabber's percussion through `bars`: an open hat on every off-beat
/// and a clap on two and four, a crash on the one.
fn gabber_perc(pat: &mut Pattern, ids: &Ids, bars: std::ops::Range<usize>) {
    for l in bars.start * BAR..bars.end * BAR {
        if l % LPB == 2 {
            *pat.cell_mut(PERC, 1, l) = n(OPEN, ids.kit, 0x58);
        }
        if l % BAR == 4 || l % BAR == 12 {
            *pat.cell_mut(PERC, 0, l) = n(CLAP, ids.kit, 0x60);
        }
    }
    *pat.cell_mut(PERC, 3, bars.start * BAR) = Cell { vol: vol_command_value('O', 3), ..n(CRASH, ids.kit, 0) };
}

/// The hoover's line through `bars`, a note every two beats, gliding.
fn hoover(pat: &mut Pattern, ids: &Ids, bars: std::ops::Range<usize>) {
    for l in (bars.start * BAR..bars.end * BAR).step_by(2 * LPB) {
        pat.tracks[LEAD][l] = n(HOOVER[l / (2 * LPB) % HOOVER.len()], ids.hoover, 0x70);
    }
}

/// The riser over the last `bars` of the pattern: noise swelling as its
/// band-pass sweeps up, let go on the last line.
fn riser(pat: &mut Pattern, ids: &Ids, bars: usize) {
    let lines = pat.lines;
    let from = lines - bars * BAR;
    pat.tracks[FX][from] = n(60, ids.riser, 0x70);
    pat.tracks[FX][lines - 1] = off();
    let f = ModuleKind::Filter;
    let sweep = [(0.0, 300.0), (from as f32, 300.0), (lines as f32, 9000.0)];
    pat.automation.push(envelope(ids.riser_filter, f, 1, &sweep, false, true));
}

/// The tempo set at the start of each bar from `bpms`, in the second FX
/// column.
fn tempo(pat: &mut Pattern, bpms: &[u8]) {
    for (b, &bpm) in bpms.iter().enumerate() {
        pat.cell_mut(FX, 1, b * BAR).fx = Some((0xF, bpm));
    }
}

/// A snare roll over the last `bars`, from every line to every other tick,
/// rising.
fn snare_roll(pat: &mut Pattern, ids: &Ids, bars: usize) {
    let from = pat.lines - bars * BAR;
    let half = bars * BAR / 2;
    for l in (from..from + half).step_by(2) {
        *pat.cell_mut(PERC, 0, l) = n(SNARE, ids.kit, 0x30 + ((l - from) * 0x30 / half) as u8);
    }
    roll(pat, PERC, from + half, half, n(SNARE, ids.kit, 0x50), 2, 0x7C);
}

// ---------------------------------------------------------------- sections

/// The tight kick and the hammer alone, the hats coming in halfway.
fn furnace(ids: &Ids) -> Pattern {
    let mut pat = pattern(LINES);
    tight_kicks(&mut pat, ids, 0..8);
    hammer(&mut pat, ids, 0..8);
    loop_perc(&mut pat, ids, 4..8, false);
    // The hammer comes in through a closed filter and opens.
    let f = ModuleKind::Filter;
    let open = [(0.0, 500.0), (64.0, 1800.0)];
    pat.automation.push(envelope(ids.hammer_filter, f, 1, &open, false, true));
    pat
}

/// The whole loop, clap and ride in, the tempo climbing a step every bar
/// with `ramp`.
fn grind(ids: &Ids, ramp: bool) -> Pattern {
    let mut pat = pattern(LINES);
    tight_kicks(&mut pat, ids, 0..8);
    hammer(&mut pat, ids, 0..8);
    loop_perc(&mut pat, ids, 0..8, true);
    if ramp {
        // 155 to 190 BPM, then the snare and riser into the gabber.
        tempo(&mut pat, &[0x9B, 0xA0, 0xA5, 0xAA, 0xAF, 0xB4, 0xB9, 0xBE]);
        snare_roll(&mut pat, ids, 2);
        riser(&mut pat, ids, 2);
    }
    pat
}

/// The gabber kick playing the tune, the hoover over it with `lead`.
fn gabber(ids: &Ids, lead: bool) -> Pattern {
    let mut pat = pattern(LINES);
    *pat.cell_mut(PERC, 2, 0) = off();
    gabber_kicks(&mut pat, ids, 0..8, LPB);
    gabber_perc(&mut pat, ids, 0..8);
    hammer(&mut pat, ids, 4..8);
    if lead {
        hoover(&mut pat, ids, 0..8);
    } else {
        // The drive turns up into the first gabber, from the loop's crunch.
        let d = ModuleKind::Distortion;
        pat.automation.push(envelope(ids.gabber_drive, d, 0, &[(0.0, 8.0), (32.0, 30.0)], false, true));
    }
    pat
}

/// The kick out: the hoover alone in the room, the snare speeding up under
/// a riser.
fn scream(ids: &Ids) -> Pattern {
    let mut pat = pattern(LINES);
    hoover(&mut pat, ids, 0..8);
    snare_roll(&mut pat, ids, 4);
    riser(&mut pat, ids, 4);
    pat
}

/// The kicks twice as fast, the hoover stuttering in the Repeater.
fn speedcore(ids: &Ids) -> Pattern {
    let mut pat = pattern(LINES);
    gabber_kicks(&mut pat, ids, 0..8, 2);
    gabber_perc(&mut pat, ids, 0..8);
    hoover(&mut pat, ids, 0..8);
    hammer(&mut pat, ids, 0..8);
    // Shorter kicks, so twice as many don't smear into one.
    let k = ModuleKind::Kicker;
    pat.automation.push(envelope(ids.gabber, k, 5, &[(0.0, 0.18)], false, false));
    // The hoover holds on a beat, then half a beat, a few times a phrase.
    let held = [(28.0, 32.0), (60.0, 64.0), (92.0, 96.0), (112.0, 128.0)];
    hold(&mut pat, ids.stutter, &held, &[(0.0, 1.0), (112.0, 2.0), (120.0, 3.0)]);
    pat
}

/// Four bars climbing to 255 BPM, the kicks rolling on every line in the
/// last, and the song stopping dead.
fn collapse(ids: &Ids) -> Pattern {
    let mut pat = pattern(4 * BAR);
    pat.tracks[LEAD][0] = off();
    tempo(&mut pat, &[0xC8, 0xDC, 0xF0, 0xFF]);
    gabber_kicks(&mut pat, ids, 0..3, 2);
    gabber_perc(&mut pat, ids, 0..4);
    for l in 3 * BAR..4 * BAR {
        pat.tracks[KICK][l] = n(KICK_TUNE[0] + (l - 3 * BAR) as u8 / 2, ids.gabber, 0x7C);
    }
    let k = ModuleKind::Kicker;
    pat.automation.push(envelope(ids.gabber, k, 5, &[(0.0, 0.18), (48.0, 0.12)], false, false));
    pat.cell_mut(FX, 1, 4 * BAR - 1).fx = Some((0xF, 0x00));
    pat
}

const COMMENTS: &str = "\
Rivet Storm: hardcore in G minor, from 155 to 255 BPM.\n\
\n\
    • Tight Kick and Hammer: the loop, a short kick under bursts of noise through a resonant Filter that the \
    Hammer Sweep Modulator moves over four bars, ring-modulated to metal.\n\
    • The tempo climbs with an Fxx at the start of every bar of the third pattern, and again at the end.\n\
    • Gabber Kick: a Kicker clipped square by its Distortion and screamed through a Scream Filter; its notes play \
    the tune. Its drive turns up over the first gabber pattern.\n\
    • Hoover: four detuned saws through a Glide, chorused and thrown into the Warehouse reverb; the Hoover Stutter \
    Repeater holds it in the speedcore pattern.\n\
    • Storm Pump: a Modulator following both kicks pulls the hammer and the hoover down under every hit.\n\
    • The snare rolls speed up with Exx, and the last bar rolls the kick on every line before the F00.";

impl Project {
    pub fn rivet_storm() -> Self {
        let mut p = Self::empty();
        p.title = "Rivet Storm".into();
        p.artist = "noise".into();
        p.comments = COMMENTS.into();
        p.bpm = BPM;
        p.lpb = LPB as u32;
        p.modules[0].params[0] = 0.8;
        let names = ["Kick", "Perc", "Hammer", "Hoover", "FX"];
        let colors = [[255, 60, 40], [255, 170, 90], [170, 180, 200], [120, 255, 170], [190, 190, 190]];
        for t in 0..TRACKS.min(p.tracks.len()) {
            p.tracks[t].name = names[t].into();
            p.tracks[t].color = Some(colors[t]);
        }
        let ids = instruments(&mut p);
        let mut patterns = vec![
            furnace(&ids),
            grind(&ids, false),
            grind(&ids, true),
            gabber(&ids, false),
            gabber(&ids, true),
            scream(&ids),
            speedcore(&ids),
            collapse(&ids),
        ];
        let names = ["Furnace", "Grind", "Ramp", "Gabber", "Hoover", "Scream", "Speedcore", "Collapse"];
        for (pat, name) in patterns.iter_mut().zip(names) {
            pat.name = name.into();
        }
        p.patterns = patterns;
        for (t, cols) in COLUMNS {
            p.set_columns(t, cols);
        }
        p.order = (0..8).map(Slot::new).collect();
        p.sections = [("Loop", 0), ("Ramp", 2), ("Gabber", 3), ("Break", 5), ("Speedcore", 6), ("End", 7)]
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
    fn rivet_storm_reaches_the_output_and_speeds_up() {
        let p = Project::rivet_storm();
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
        // The tempo only rises, to 255.
        let bpms: Vec<u8> = (p.patterns.iter())
            .flat_map(|pat| (0..pat.lines).filter_map(move |l| pat.cell(FX, 1, l).fx))
            .filter(|&(c, v)| c == 0xF && v >= 20)
            .map(|(_, v)| v)
            .collect();
        assert!(bpms.windows(2).all(|w| w[0] < w[1]) && bpms.last() == Some(&0xFF), "{bpms:?}");
        let kit = p.modules.iter().find(|m| m.kind == ModuleKind::Sampler).unwrap();
        assert!(kit.samples.iter().all(|s| s.data.is_some() && s.oneshot));
    }

    #[test]
    fn rivet_storm_plays_loud_enough_without_clipping_and_ends() {
        let frames = crate::audio::render(std::sync::Arc::new(Project::rivet_storm()), 8000, 1.0, false);
        let peak = frames.iter().fold(0f32, |a, f| a.max(f[0].abs()).max(f[1].abs()));
        assert!(peak > 0.5 && peak <= 1.0, "{peak}");
        let secs = frames.len() as f32 / 8000.0;
        assert!((70.0..120.0).contains(&secs), "{secs} s");
    }
}
