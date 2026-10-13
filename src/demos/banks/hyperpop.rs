//! The hyperpop bank: a trap kit, a gliding and driven 808, a strummed
//! plucked guitar, a supersaw wall, a pitched-up voice and its chops,
//! bells, coin blips and a riser, with the room they share and a master
//! that stutters and crushes.

use super::super::*;

/// Makes every module and wires them up.
fn instruments(p: &mut Project) {
    use ModuleKind as K;
    let add = |p: &mut Project, kind: ModuleKind, name: &str, params: &[(usize, f32)]| {
        let id = super::super::add(p, kind, name, params);
        p.connect(id, OUTPUT_ID);
        id
    };

    // A big bright room the voice, guitar, bells and wall share.
    let room = add(p, K::Reverb, "Bubble Room", &[(0, 0.88), (1, 0.35), (2, 0.28)]);

    // The master: a Repeater for the stutter edits, a clip that can crush
    // to a few bits at the end, glue, sheen and a limiter, loud.
    effects(
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
    effects(
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
    effects(
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
    effects(p, riser, [(K::FilterPro, "Riser Sweep", &[(0, 2.0), (1, 400.0), (2, 4.0), (4, 1.0)])]);
    send(p, riser, room);

    // The pump: the wall and the guitar duck under the kit.
    let pump = super::super::add(p, K::Modulator, "Sugar Pump", &[(0, 1.0), (5, -0.7), (6, 0.003), (7, 0.16)]);
    let kit_end = p.chain(drums).effects.last().copied().unwrap_or(drums);
    p.connect(kit_end, pump);
    for target in [wall, guitar] {
        let fader = p.module(target).unwrap().kind.params().len();
        p.connect(pump, target);
        p.set_control_param(pump, target, fader);
    }
}

impl Project {
    /// The hyperpop bank's instruments and effects, for the factory presets.
    pub fn hyperpop_bank() -> Self {
        let mut p = Self::empty();
        p.title = "Hyperpop".into();
        instruments(&mut p);
        // Effects several chains share become send tracks.
        p.untangle();
        p
    }
}
