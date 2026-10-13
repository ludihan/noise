//! The orchestral bank: a piano and a swell rendered here, a wavetable
//! string pad, a sub bass, a rippling analog arp with its phrases, a
//! choir-like lead, voices, bells, a granular haze and the drums, with
//! the room and hall they share.

use super::super::*;
use crate::dsp::Frame;
use crate::sample::Sample;

/// The track the hats' own effects are on.
const HATS: usize = 1;

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

/// Makes every module and wires them up.
fn instruments(p: &mut Project) {
    use ModuleKind as K;

    // The shared hall everything but the drums and bass goes through.
    let hall = add(p, K::Reverb, "Hall", &[(0, 0.9), (1, 0.45), (2, 0.38)]);
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
    let drums = add(p, K::Drums, "Drums", &[(0, 0.8), (1, 48.0), (2, 0.6), (3, 0.35), (4, 0.05)]);
    let tight = p.chain_insert(drums, 0, K::Gate).unwrap();
    set(p, tight, "Tight", &[(0, 0.02), (2, 0.03), (3, 0.08), (4, 0.15)]);
    let bus = p.chain_insert(drums, 1, K::Compressor).unwrap();
    set(p, bus, "Drum Bus", &[(0, 0.35), (1, 3.0), (2, 0.008), (3, 0.12), (4, 1.3)]);
    let stutter = p.chain_insert(drums, 2, K::Repeater).unwrap();
    set(p, stutter, "Stutter", &[]);
    let punch = p.chain_insert(drums, 3, K::Maximizer).unwrap();
    set(p, punch, "Punch", &[(0, 1.25), (1, 0.9), (2, 0.06)]);
    // A Kicker tuned to D under the chorus's kicks.
    let sub_kick = add(p, K::Kicker, "Sub Kick", &[(0, 0.45), (2, 2.5), (5, 0.7), (6, 0.35)]);
    p.connect(sub_kick, OUTPUT_ID);
    // Rounded off by a WaveShaper bent into a soft clip.
    let round = p.chain_insert(sub_kick, 0, K::WaveShaper).unwrap();
    set(p, round, "Round", &[(0, 1.3), (3, 0.6), (9, 0.3), (10, 0.55), (11, 0.72), (12, 0.8)]);

    // A warm sub bass: filtered and gently driven, with the drive's
    // offset taken away, and a little of a bass cabinet.
    let bass = add(
        p,
        K::Generator,
        "Sub Bass",
        &[(0, 0.5), (1, 0.0), (2, 0.01), (3, 0.4), (4, 0.7), (5, 0.25), (6, 6.0), (7, 2.0)],
    );
    let bass_filter = p.chain_insert(bass, 0, K::Filter).unwrap();
    set(p, bass_filter, "Bass Filter", &[(1, 500.0), (2, 0.35)]);
    let drive = p.chain_insert(bass, 1, K::Distortion).unwrap();
    set(p, drive, "Warmth", &[(0, 1.6), (1, 0.5), (2, 0.35)]);
    let centre = p.chain_insert(bass, 2, K::DcBlocker).unwrap();
    set(p, centre, "Centre", &[]);
    let speaker = p.chain_insert(bass, 3, K::Cabinet).unwrap();
    set(p, speaker, "Speaker", &[(0, 2.0), (1, 1.2), (2, 0.35)]);

    // Strings: a wide wavetable pad between triangle and saw that grows
    // brighter as a chord holds, chorused, phased, darkened and widened.
    let pad = add(
        p,
        K::Wavetable,
        "Strings",
        &[(0, 0.13), (2, 0.55), (3, 0.03), (4, 5.0), (5, 16.0), (6, 0.6), (7, 0.9), (8, 1.0), (9, 0.85), (10, 2.2)],
    );
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
    let voices = add(
        p,
        K::Generator,
        "Voices",
        &[(0, 0.22), (1, 0.0), (2, 0.5), (3, 1.0), (4, 0.9), (5, 1.5), (6, 10.0), (7, 3.0)],
    );
    let voice = p.chain_insert(voices, 0, K::VocalFilter).unwrap();
    set(p, voice, "Voice", &[(2, 1.3), (4, 2.5)]);
    let ring_d = p.chain_insert(voices, 1, K::CombFilter).unwrap();
    set(p, ring_d, "Ring in D", &[(0, 50.0), (2, 0.7), (3, 0.4), (4, 0.3)]);
    let far = p.chain_insert(voices, 2, K::Echo).unwrap();
    set(p, far, "Far Echo", &[(0, 0.43), (1, 0.4), (2, 0.5), (3, 0.5), (4, 0.2)]);
    // The drums speak through them, half way, from a Vocoder keyed by
    // the kit.
    let speak = p.chain_insert(voices, 0, K::Vocoder).unwrap();
    set(p, speak, "Drum Talk", &[(0, 12.0), (5, 0.08), (7, 3.0), (8, 0.5)]);
    p.module_mut(speak).unwrap().key = Some(drums);
    send(p, voices, hall);

    // The piano: plainly struck, its tone left as it is, in a small room;
    // velocity alone shapes it.
    let piano = add(p, K::Sampler, "Piano", &[(0, 0.3), (3, 0.0), (4, 4.0), (5, 1.0), (6, 0.3)]);
    let sr = 44100.0;
    let mut struck = rendered(felt_piano(sr, true));
    struck.name = "Piano".into();
    p.module_mut(piano).unwrap().samples = vec![struck];
    let room = p.chain_insert(piano, 0, K::Convolver).unwrap();
    set(p, room, "Piano Room", &[(0, 0.0), (1, 0.12), (2, 1.0)]);
    // The haze: the felt piano's soft layer as a cloud of grains.
    let haze_sample = rendered(felt_piano(sr, false));
    let haze = add(
        p,
        K::Granular,
        "Haze",
        &[
            (0, 0.35),
            (1, 0.15),
            (2, 0.05),
            (3, 0.15),
            (4, 25.0),
            (5, 0.08),
            (6, 8.0),
            (7, 0.8),
            (8, 0.3),
            (9, 1.0),
            (10, 2.5),
        ],
    );
    p.module_mut(haze).unwrap().samples = vec![haze_sample];
    send(p, haze, hall);

    // The swell: one shot, stretched over four beats whatever the tempo.
    let swell_id = add(p, K::Sampler, "Swell", &[(0, 0.7), (6, 0.05)]);
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

    // The arp: an Analog Synth's soft pulse, a filter envelope plucking
    // each note, playing its phrases, with a little bite from a scream
    // filter before its echo.
    #[rustfmt::skip]
    let arp = add(p, K::Analog, "Ripple", &[
        (0, 0.18), (1, 1.0), (5, 0.0), (8, 0.3),
        (9, 1.0), (10, 900.0), (11, 0.35), (12, 2.0), (13, 0.3), (14, 1.0),
        (16, 0.25), (17, 0.25), (19, 0.002), (20, 0.15), (21, 0.15), (22, 0.2),
    ]);
    let m = p.module_mut(arp).unwrap();
    m.phrases = vec![ripple(), sparkle()];
    m.phrase_mode = PhraseMode::Program;
    let arp_echo = p.chain_insert(arp, 0, K::Delay).unwrap();
    set(p, arp_echo, "Arp Echo", &[(0, 3.0), (1, 0.4), (2, 0.3), (3, 0.8)]);
    let bite = p.chain_insert(arp, 0, K::ScreamFilter).unwrap();
    set(p, bite, "Bite", &[(1, 2500.0), (2, 0.6), (3, 2.0), (4, 0.35)]);
    send(p, arp, hall);

    // The lead: a MultiSynth playing an FM voice with a late vibrato, and
    // a saw the Vocal Filter turns into a choir, into one echo.
    let lead = add(p, K::MultiSynth, "Lead", &[(3, 5.0), (5, 0.1)]);
    let lead_fm = add(
        p,
        K::Fm,
        "Lead FM",
        &[(0, 0.2), (1, 1.0), (2, 1.6), (3, 0.8), (4, 0.15), (5, 0.03), (6, 1.0), (7, 0.6), (8, 0.6)],
    );
    let m = p.module_mut(lead_fm).unwrap();
    m.modulation.vibrato = VoiceLfo { on: true, shape: 0, rate: 5.2, depth: 0.18, delay: 0.35 };
    let lead_saw = add(
        p,
        K::Generator,
        "Lead Choir",
        &[(0, 0.12), (1, 0.0), (2, 0.12), (3, 0.6), (4, 0.8), (5, 0.7), (6, 12.0), (7, 3.0)],
    );
    let choir = p.chain_insert(lead_saw, 0, K::VocalFilter).unwrap();
    set(p, choir, "Choir", &[(4, 2.2), (5, 0.85)]);
    let waver = p.chain_insert(lead_saw, 1, K::Vibrato).unwrap();
    set(p, waver, "Waver", &[(0, 5.2), (1, 0.12), (2, 0.25)]);
    let lead_echo = add(p, K::Delay, "Lead Echo", &[(0, 6.0), (1, 0.35), (2, 0.25), (3, 0.6)]);
    // The choir slides between notes played over each other.
    let slide = add(p, K::Glide, "Choir Slide", &[(0, 1.0), (1, 0.07)]);
    p.connect(lead, lead_fm);
    p.connect(lead, slide);
    p.connect(slide, lead_saw);
    p.connect(lead_fm, lead_echo);
    // And a twelve-string's pluck under each note.
    let lead_pluck = add(
        p,
        K::PluckedString,
        "Lead Pluck",
        &[(0, 0.1), (1, 0.18), (2, 0.6), (3, 2.5), (4, 0.35), (5, 0.4), (6, 2.0), (7, 7.0)],
    );
    p.connect(lead, lead_pluck);
    p.connect(lead_pluck, lead_echo);
    let warm = p.chain_insert(lead_fm, 0, K::AnalogFilter).unwrap();
    set(p, warm, "Warm", &[(1, 3800.0), (2, 0.25), (3, 1.4)]);
    send(p, lead_saw, lead_echo);
    p.connect(lead_echo, hall);

    // Bells: FM, a touch of ring modulation, panned to and fro, with a
    // shimmer an octave up.
    let bells =
        add(p, K::Fm, "Bells", &[(0, 0.12), (1, 3.5), (2, 3.0), (3, 1.2), (5, 0.001), (6, 2.0), (7, 0.0), (8, 1.5)]);
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
    let glass = add(
        p,
        K::SpectraVoice,
        "Glass",
        &[(0, 0.1), (1, 16.0), (2, 1.3), (3, 0.15), (4, 0.002), (5, 0.6), (6, 1.5), (9, 3.0)],
    );
    let scatter = p.chain_insert(glass, 0, K::Multitap).unwrap();
    set(p, scatter, "Scatter", &[(0, 3.0), (3, 5.0), (6, 7.0), (9, 11.0), (12, 0.2), (13, 0.35)]);
    send(p, glass, hall);

    // Keys: an FMX electric piano, two stacks with a bright tine, for the
    // outro's chords.
    let tine = [(3, 1.0), (4, 1.0), (6, 2.5), (7, 0.2), (9, 0.35), (10, 14.0), (13, 0.0), (15, 0.8)];
    let keys = add(
        p,
        K::Fmx,
        "Keys",
        &[&[(0, 0.35), (1, 4.0)], &tine[..], &[(16, 1.0), (21, 0.4), (22, 1.0), (24, 1.5)]].concat(),
    );
    let shimmer = p.chain_insert(keys, 0, K::Chorus).unwrap();
    set(p, shimmer, "Keys Chorus", &[(1, 0.6), (2, 0.4), (6, 0.4)]);
    send(p, keys, hall);

    // Modulators: the strings breathe brighter and darker over four bars,
    // and the strings and arp duck under the drums.
    let breath = add(p, K::Modulator, "Breath", &[(3, 1.0), (4, 64.0), (5, 0.35)]);
    p.connect(breath, pad_eq);
    p.set_control_param(breath, pad_eq, 2);
    let duck = add(p, K::Modulator, "Duck", &[(0, 1.0), (5, -0.55), (6, 0.004), (7, 0.25)]);
    p.connect(stutter, duck);
    for target in [pad, arp] {
        let fader = p.module(target).unwrap().kind.params().len();
        p.connect(duck, target);
        p.set_control_param(duck, target, fader);
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

impl Project {
    /// The orchestral bank's instruments and effects, for the factory presets.
    pub fn orchestral_bank() -> Self {
        let mut p = Self::empty();
        p.title = "Orchestral".into();
        instruments(&mut p);
        // Effects several chains share become send tracks.
        p.untangle();
        p
    }
}
