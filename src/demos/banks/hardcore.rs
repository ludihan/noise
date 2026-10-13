//! The hardcore bank: a tight kick and a screaming gabber kick, a
//! rendered kit, a swept metal hammer, a hoover, a chip lead and bass and
//! a riser, with a pump and a master driven flat out.

use super::super::*;
use crate::dsp::Frame;
use crate::sample::Sample;

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

fn instruments(p: &mut Project) {
    use ModuleKind as K;
    // Everything here goes to the output unless sent elsewhere.
    let add = |p: &mut Project, kind: ModuleKind, name: &str, params: &[(usize, f32)]| {
        let id = super::super::add(p, kind, name, params);
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

    // The tight kick: a short sine thump, lightly clipped, for the loop
    // and the break.
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
    effects(
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
    let sweep =
        super::super::add(p, K::Modulator, "Hammer Sweep", &[(0, 0.0), (1, 0.0), (3, 1.0), (4, 64.0), (5, 0.35)]);
    p.connect(sweep, hammer_filter);
    p.set_control_param(sweep, hammer_filter, 1);
    p.module_mut(sweep).unwrap().home = Some(Owner::Module(hammer));

    // The hoover: four detuned saws a Glide slides between, chorused wide,
    // driven, stuttered by a Repeater and thrown into a big room.
    let glide = p.add_module(K::Glide, [0.0, 0.0]).unwrap();
    set(p, glide, "Hoover Glide", &[(0, 1.0), (1, 0.12)]);
    let hoover =
        add(p, K::Generator, "Hoover", &[(0, 0.24), (1, 0.0), (2, 0.01), (4, 0.9), (5, 0.25), (6, 35.0), (7, 4.0)]);
    p.connect(glide, hoover);
    effects(
        p,
        hoover,
        [
            (K::Chorus, "Hoover Width", &[(0, 3.0), (1, 0.4), (2, 0.6), (4, 0.4), (6, 0.6)]),
            (K::Distortion, "Hoover Drive", &[(0, 3.0), (1, 0.6), (2, 0.5)]),
            (K::Repeater, "Hoover Stutter", &[(0, 0.0), (1, 4.0)]),
            (K::Reverb, "Warehouse", &[(0, 0.85), (1, 0.5), (2, 0.25)]),
        ],
    );

    // The chip lead: a narrow pulse, crushed to a few bits at a lower rate
    // like an old console's, and echoed a dotted eighth behind.
    let chip = add(
        p,
        K::Generator,
        "Chip Lead",
        &[(0, 0.26), (1, 1.0), (2, 0.001), (3, 0.12), (4, 0.75), (5, 0.06), (8, 0.25)],
    );
    effects(
        p,
        chip,
        [
            (K::Distortion, "Chip Crush", &[(0, 1.2), (1, 0.8), (2, 1.0), (4, 5.0), (5, 3.0)]),
            (K::Delay, "Chip Echo", &[(0, 3.0), (1, 0.3), (2, 0.22), (3, 0.8)]),
        ],
    );

    // The chip bass: a square bouncing in octaves, crushed and kept low.
    let bass = add(p, K::Generator, "Chip Bass", &[(0, 0.15), (1, 1.0), (2, 0.001), (3, 0.08), (4, 0.8), (5, 0.03)]);
    effects(
        p,
        bass,
        [
            (K::Distortion, "Bass Crush", &[(0, 2.0), (1, 0.6), (2, 1.0), (4, 4.0)]),
            (K::FilterPro, "Bass Low", &[(1, 1100.0), (2, 0.9), (4, 1.0)]),
        ],
    );

    // The riser: noise through a band-pass that sweeps up.
    let riser = add(p, K::Generator, "Storm Riser", &[(0, 0.35), (1, 4.0), (2, 2.0), (4, 1.0), (5, 0.4)]);
    effects(p, riser, [(K::Filter, "Storm Sweep", &[(0, 2.0), (1, 400.0), (2, 0.6)])]);

    // The pump: a Modulator following both kicks pulls the hammer, the
    // hoover and the bass down under every hit.
    let pump = p.add_module(K::Modulator, [0.0, 0.0]).unwrap();
    set(p, pump, "Storm Pump", &[(0, 1.0), (5, -0.65), (6, 0.002), (7, 0.15)]);
    for kick in [tight, gabber] {
        let end = p.chain(kick).effects.last().copied().unwrap_or(kick);
        p.connect(end, pump);
    }
    for target in [hammer, hoover, bass] {
        let fader = p.module(target).unwrap().kind.params().len();
        p.connect(pump, target);
        p.set_control_param(pump, target, fader);
    }
}

impl Project {
    /// The hardcore bank's instruments and effects, for the factory presets.
    pub fn hardcore_bank() -> Self {
        let mut p = Self::empty();
        p.title = "Hardcore".into();
        instruments(&mut p);
        // Effects several chains share become send tracks.
        p.untangle();
        p
    }
}
