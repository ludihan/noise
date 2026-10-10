//! Tests of the modules together: every one at the ends of its
//! parameters, and those that share helpers.

use super::*;

const SR: f32 = 1000.0;

/// A slot whose audio is a ramp from 0 to 1, so the output level tells
/// where in the sample playback is.
fn ramp_slot(len: usize) -> SampleSlot {
    let frames = (0..len).map(|i| [i as f32 / len as f32; 2]).collect();
    let sample = Sample { name: "ramp".into(), sample_rate: SR, channels: 1, frames };
    let mut slot = SampleSlot::new(sample, None);
    slot.base_note = 60;
    slot
}

fn render(s: &mut Sampler, frames: usize) -> Vec<f32> {
    let ctx = Ctx { sr: SR, samples_per_line: 100.0, song_line: None };
    // Volume, pan, transpose, attack, decay, sustain, release.
    let params = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let mut out = vec![[0.0; 2]; frames];
    s.process(&ctx, &params, &[], &mut out);
    // Undo the center pan gain.
    out.iter().map(|f| f[0] / pan_gains(0.0).0).collect()
}

#[test]
fn numbers_too_small_to_matter_are_zero() {
    flush_denormals();
    let tiny = std::hint::black_box(f32::MIN_POSITIVE);
    assert_eq!(std::hint::black_box(tiny * 0.5), 0.0);
}

#[test]
fn an_lfo_synced_to_lines_follows_the_song() {
    // Tremolo, a sine over 4 lines at full depth: silent a quarter of the
    // way round the other way, three lines into the song.
    let mut lfo = Lfo::default();
    let p = [0.0, 0.0, 1.0, 2.0, 1.0, 4.0];
    let input = [[1.0; 2]; 4];
    let mut out = [[0.0; 2]; 4];
    for (line, gain) in [(1.0, 1.0), (3.0, 0.0)] {
        let ctx = Ctx { sr: 1000.0, samples_per_line: 100.0, song_line: Some(line) };
        lfo.process(&ctx, &p, &input, &mut out);
        assert!((out[0][0] - gain).abs() < 1e-3, "at line {line}: {}", out[0][0]);
    }
}

#[test]
fn keyzones_pick_samples() {
    let mut s = Sampler::new();
    let mut a = ramp_slot(100);
    let mut b = ramp_slot(100);
    a.keys = [48, 48];
    b.keys = [49, 49];
    b.volume = 0.5;
    s.set_samples(&[a, b]);
    s.note_on(0, 49.0, 1.0);
    assert_eq!(s.voices.iter().filter(|v| v.env.active()).count(), 1);
    assert_eq!(s.voices.iter().find(|v| v.env.active()).unwrap().zone, 1);
    s.note_on(1, 60.0, 1.0);
    assert_eq!(s.voices.iter().filter(|v| v.env.active()).count(), 1, "no zone holds C-5");
}

#[test]
fn overlapping_zones_layer() {
    let mut s = Sampler::new();
    s.set_samples(&[ramp_slot(100), ramp_slot(100)]);
    s.note_on(0, 60.0, 1.0);
    assert_eq!(s.voices.iter().filter(|v| v.env.active()).count(), 2);
}

#[test]
fn velocity_ranges() {
    let mut s = Sampler::new();
    let mut soft = ramp_slot(100);
    soft.velocities = [0, 63];
    s.set_samples(&[soft]);
    s.note_on(0, 60.0, 1.0);
    assert!(s.voices.iter().all(|v| !v.env.active()));
    s.note_on(0, 60.0, 0.25);
    assert!(s.voices.iter().any(|v| v.env.active()));
}

#[test]
fn no_loop_stops_at_the_end() {
    let mut s = Sampler::new();
    s.set_samples(&[ramp_slot(100)]);
    s.note_on(0, 60.0, 1.0);
    let out = render(&mut s, 200);
    assert!(out[99] > 0.9);
    assert!(out[150..].iter().all(|&x| x == 0.0));
}

#[test]
fn forward_loop_repeats() {
    let mut s = Sampler::new();
    let mut slot = ramp_slot(100);
    slot.loop_mode = 1;
    slot.loop_start = 50;
    slot.loop_end = 100;
    s.set_samples(&[slot]);
    s.note_on(0, 60.0, 1.0);
    let out = render(&mut s, 300);
    // After the first pass the ramp restarts from the loop start.
    assert!((out[100] - 0.5).abs() < 0.02, "{}", out[100]);
    assert!((out[249] - 0.99).abs() < 0.02, "{}", out[249]);
    assert!((out[250] - 0.5).abs() < 0.02, "{}", out[250]);
}

#[test]
fn backward_loop_plays_the_loop_in_reverse() {
    let mut s = Sampler::new();
    let mut slot = ramp_slot(100);
    slot.loop_mode = 2;
    slot.loop_start = 50;
    slot.loop_end = 100;
    s.set_samples(&[slot]);
    s.note_on(0, 60.0, 1.0);
    let out = render(&mut s, 400);
    // Forward to the loop end, then falling repeatedly from the end.
    assert!(out[120] < out[110] && out[110] < out[101]);
    assert!(out[200..400].iter().all(|&x| x >= 0.48));
    let rising = out[200..400].windows(2).filter(|w| w[1] > w[0] + 0.1).count();
    assert!(rising >= 3, "jumps back to the loop end: {rising}");
}

#[test]
fn ping_pong_loop_bounces() {
    let mut s = Sampler::new();
    let mut slot = ramp_slot(100);
    slot.loop_mode = 3;
    slot.loop_start = 50;
    slot.loop_end = 100;
    s.set_samples(&[slot]);
    s.note_on(0, 60.0, 1.0);
    let out = render(&mut s, 400);
    assert!(out[120] < out[110], "falling after the loop end");
    assert!(out[170] > out[160], "rising again after the loop start");
    assert!(out.iter().skip(50).all(|&x| x >= 0.48));
}

#[test]
fn pitch_follows_base_note_and_transpose() {
    let mut s = Sampler::new();
    let mut slot = ramp_slot(200);
    slot.transpose = -12;
    s.set_samples(&[slot]);
    // C-6 with a -12 transpose plays at the original speed.
    s.note_on(0, 72.0, 1.0);
    let out = render(&mut s, 100);
    assert!((out[50] - 0.25).abs() < 0.02, "{}", out[50]);
}

#[test]
fn replacing_the_audio_stops_its_voices() {
    let mut s = Sampler::new();
    let slot = ramp_slot(100);
    s.set_samples(std::slice::from_ref(&slot));
    s.note_on(0, 60.0, 1.0);
    // A settings change keeps the voice.
    let mut louder = slot.clone();
    louder.volume = 2.0;
    s.set_samples(&[louder]);
    assert!(s.voices.iter().any(|v| v.env.active()));
    // New audio stops it.
    s.set_samples(&[ramp_slot(100)]);
    assert!(s.voices.iter().all(|v| !v.env.active()));
}

#[test]
fn mute_groups_cut_each_other() {
    let mut s = Sampler::new();
    let (mut open, mut closed, other) = (ramp_slot(1000), ramp_slot(1000), ramp_slot(1000));
    (open.keys, closed.keys) = ([48, 48], [49, 49]);
    (open.mute_group, closed.mute_group) = (1, 1);
    s.set_samples(&[open, closed, other]);
    s.note_on(0, 48.0, 1.0);
    s.note_on(1, 60.0, 1.0);
    render(&mut s, 10);
    s.note_on(2, 49.0, 1.0);
    render(&mut s, 10);
    let zones: Vec<usize> = s.voices.iter().filter(|v| v.env.active()).map(|v| v.zone).collect();
    assert!(!zones.contains(&0), "the open hat is cut: {zones:?}");
    assert!(zones.contains(&1) && zones.contains(&2), "others play on: {zones:?}");
}

#[test]
fn oneshots_ignore_note_offs() {
    let mut s = Sampler::new();
    let mut slot = ramp_slot(100);
    slot.oneshot = true;
    s.set_samples(&[slot]);
    s.note_on(0, 60.0, 1.0);
    s.note_off(0);
    assert!(s.voices.iter().any(|v| v.env.active() && !v.slot.released));
}

#[test]
fn beat_sync_fits_the_sample_to_lines() {
    let mut s = Sampler::new();
    let mut slot = ramp_slot(100);
    // Two lines of 100 frames: half speed, whatever the sample rate.
    slot.beat_sync = 2;
    s.set_samples(&[slot]);
    s.note_on(0, 60.0, 1.0);
    let out = render(&mut s, 150);
    assert!((out[100] - 0.5).abs() < 0.02, "{}", out[100]);
}

#[test]
fn the_sine_table_is_a_sine() {
    for i in -2000..2000 {
        let x = i as f32 * 0.00377;
        assert!((sine(x) as f64 - (x as f64 * std::f64::consts::TAU).sin()).abs() < 2e-6, "{x}");
    }
}

#[test]
fn reverse_plays_from_the_end_or_from_where_it_is() {
    let mut s = Sampler::new();
    s.set_samples(&[ramp_slot(100)]);
    s.note_on(0, 60.0, 1.0);
    s.reverse(0, true);
    let out = render(&mut s, 10);
    assert!((out[0] - 0.99).abs() < 0.02 && out[9] < out[0], "{out:?}");
    // Forwards again from there.
    s.reverse(0, false);
    let out = render(&mut s, 10);
    assert!(out[9] > out[0], "{out:?}");
    // Backwards past the start, it stops.
    s.reverse(0, true);
    let out = render(&mut s, 200);
    assert_eq!(out[199], 0.0);
}

#[test]
fn a_slice_plays_at_the_pitch_of_the_note() {
    let mut s = Sampler::new();
    let mut slot = ramp_slot(100);
    slot.slices = vec![25, 50, 75];
    s.set_samples(&[slot]);
    // The whole sample on its base note, an octave up, switched to the
    // third slice.
    s.note_on(0, 72.0, 1.0);
    s.play_slice(0, 2);
    let out = render(&mut s, 10);
    assert!((out[0] - 0.5).abs() < 0.02, "{out:?}");
    assert!((out[1] - out[0] - 0.02).abs() < 0.002, "still an octave up: {out:?}");
    // On the sample's base note it plays at the sample's pitch.
    s.note_on(1, 60.0, 1.0);
    s.play_slice(1, 1);
    s.note_off(0);
    let low = render(&mut s, 3);
    assert!((low[0] - 0.25).abs() < 0.02 && (low[2] - low[1] - 0.01).abs() < 0.002, "{low:?}");
    // A slice that isn't there leaves the note alone.
    s.note_on(2, 60.0, 1.0);
    s.play_slice(2, 9);
    assert!(s.voices.iter().any(|v| v.slot.key == 2 && v.zone == 0));
}

#[test]
fn sample_offset_starts_later() {
    let mut s = Sampler::new();
    s.set_samples(&[ramp_slot(100)]);
    s.note_on(0, 60.0, 1.0);
    s.sample_offset(0, 0.5);
    let out = render(&mut s, 10);
    assert!((out[0] - 0.5).abs() < 0.02, "{}", out[0]);
}

#[test]
fn seeking_moves_through_the_loop_or_ends_the_sample() {
    let mut s = Sampler::new();
    let mut slot = ramp_slot(100);
    slot.autoseek = true;
    s.set_samples(&[slot.clone()]);
    s.note_on(0, 60.0, 1.0);
    s.seek(0, 40.0);
    assert!((render(&mut s, 1)[0] - 0.4).abs() < 0.02, "40 frames in");
    let mut s = Sampler::new();
    s.set_samples(&[slot.clone()]);
    s.note_on(0, 60.0, 1.0);
    s.seek(0, 150.0);
    assert!(render(&mut s, 4).iter().all(|&x| x == 0.0), "played out by then");
    // Looped over its second half, it is still going round.
    (slot.loop_mode, slot.loop_start, slot.loop_end) = (1, 50, 100);
    let mut s = Sampler::new();
    s.set_samples(&[slot.clone()]);
    s.note_on(0, 60.0, 1.0);
    s.seek(0, 130.0);
    assert!((render(&mut s, 1)[0] - 0.8).abs() < 0.02, "30 frames past the loop's start");
    // Without autoseek, the note stays silent.
    slot.autoseek = false;
    let mut s = Sampler::new();
    s.set_samples(&[slot]);
    s.note_on(0, 60.0, 1.0);
    s.seek(0, 10.0);
    assert!(render(&mut s, 4).iter().all(|&x| x == 0.0));
}

/// Runs `kind` with its default parameters, changed by `set`, on a sine
/// of `freq` Hz at 48 kHz, and returns the peak of the second half.
fn effect_peak(kind: ModuleKind, set: &[(usize, f32)], freq: f32, amp: f32) -> f32 {
    let sr = 48000.0;
    let ctx = Ctx { sr, samples_per_line: 6000.0, song_line: None };
    let mut params: Vec<f32> = kind.params().iter().map(|p| p.default).collect();
    for &(i, v) in set {
        params[i] = v;
    }
    let input: Vec<Frame> = (0..48000).map(|i| [amp * (TAU * freq * i as f32 / sr).sin(); 2]).collect();
    let mut out = vec![[0.0; 2]; input.len()];
    create(kind, sr).process(&ctx, &params, &input, &mut out);
    out[24000..].iter().fold(0.0f32, |m, f| m.max(f[0].abs()))
}

#[test]
fn phaser_cuts_a_notch_where_its_stages_turn_the_phase_over() {
    // Two stages held at 1 kHz shift 1 kHz by half a cycle, so mixed
    // half and half with the input it cancels out; far below it doesn't.
    let still = [(1, 0.0), (2, 1000.0), (3, 1000.0), (4, 2.0), (5, 0.0), (6, 0.5)];
    let notch = effect_peak(ModuleKind::Phaser, &still, 1000.0, 0.5);
    assert!(notch < 0.02, "notch at 1 kHz: {notch}");
    let low = effect_peak(ModuleKind::Phaser, &still, 60.0, 0.5);
    assert!(low > 0.45, "60 Hz passes: {low}");
    let dry = effect_peak(ModuleKind::Phaser, &[(6, 0.0)], 1000.0, 0.5);
    assert!((dry - 0.5).abs() < 1e-4, "no mix, no change: {dry}");
    let swept = effect_peak(ModuleKind::Phaser, &[(5, 0.9)], 1000.0, 0.5);
    assert!(swept.is_finite() && swept < 2.0, "stays stable with feedback: {swept}");
}

#[test]
fn vocal_filter_passes_its_vowels_formants() {
    let unity = [(4, 1.0)];
    // A's first formant is at 650 Hz, I's at 290 Hz and 1870 Hz.
    let a = effect_peak(ModuleKind::VocalFilter, &unity, 650.0, 0.5);
    assert!((a - 0.5).abs() < 0.05, "A passes 650 Hz: {a}");
    let i = effect_peak(ModuleKind::VocalFilter, &[(0, 2.0), (4, 1.0)], 650.0, 0.5);
    assert!(i < a * 0.3, "I doesn't: {i}");
    let high = effect_peak(ModuleKind::VocalFilter, &unity, 8000.0, 0.5);
    assert!(high < 0.05, "nor 8 kHz: {high}");
    // An octave up, A's first formant is at 1300 Hz.
    let up = effect_peak(ModuleKind::VocalFilter, &[(1, 12.0), (4, 1.0)], 1300.0, 0.5);
    assert!((up - 0.5).abs() < 0.05, "shifted: {up}");
}

#[test]
fn repeater_loops_the_input_before_hold() {
    let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
    let mut r = create(ModuleKind::Repeater, ctx.sr);
    // 1/16 line is 375 frames; the input counts up.
    let input: Vec<Frame> = (0..4000).map(|i| [i as f32; 2]).collect();
    let mut out = vec![[0.0; 2]; input.len()];
    r.process(&ctx, &[0.0, 9.0, 1.0], &input, &mut out);
    assert_eq!(out, input, "off, it passes the input");
    let silence = vec![[0.0; 2]; 2000];
    let mut held = vec![[0.0; 2]; 2000];
    r.process(&ctx, &[1.0, 9.0, 1.0], &silence, &mut held);
    // Once faded in, the loop's middle repeats every 375 frames.
    for k in [400 + 100, 400 + 375 + 100, 400 + 750 + 100] {
        assert_eq!(held[k][0], (4000 - 375 + (k % 375)) as f32, "frame {k}");
    }
    let mut after = vec![[0.0; 2]; 2000];
    r.process(&ctx, &[0.0, 9.0, 1.0], &silence, &mut after);
    assert!(after[500..].iter().all(|f| f[0] == 0.0), "let go, it passes the input again");
}

#[test]
fn ring_mod_moves_a_tone_to_the_sum_and_difference() {
    let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
    let mut r = create(ModuleKind::RingMod, ctx.sr);
    let n = 48000;
    let input: Vec<Frame> = (0..n).map(|i| [(TAU * 1000.0 * i as f32 / ctx.sr).sin(); 2]).collect();
    let mut out = vec![[0.0; 2]; n];
    r.process(&ctx, &[300.0, 0.0, 0.0, 1.0], &input, &mut out);
    let level = |f: f32| level_at(&out, f, ctx.sr);
    assert!(
        (level(700.0) - 0.5).abs() < 0.02 && (level(1300.0) - 0.5).abs() < 0.02,
        "{} {}",
        level(700.0),
        level(1300.0)
    );
    assert!(level(1000.0) < 0.01, "the tone itself is gone: {}", level(1000.0));
}

#[test]
fn gate_passes_loud_sounds_and_silences_quiet_ones() {
    let loud = effect_peak(ModuleKind::Gate, &[], 100.0, 0.5);
    assert!((loud - 0.5).abs() < 0.01, "above the threshold it is open: {loud}");
    let quiet = effect_peak(ModuleKind::Gate, &[], 100.0, 0.02);
    assert!(quiet < 1e-4, "below it, closed: {quiet}");
    let floor = effect_peak(ModuleKind::Gate, &[(4, 0.5)], 100.0, 0.02);
    assert!((floor - 0.01).abs() < 1e-3, "closed, it lets the floor through: {floor}");
}

#[test]
fn kicker_falls_to_the_note_and_dies_away() {
    let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
    let mut k = create(ModuleKind::Kicker, ctx.sr);
    let params: Vec<f32> = ModuleKind::Kicker.params().iter().map(|p| p.default).collect();
    k.note_on(0, 33.0, 1.0);
    let mut out = vec![[0.0; 2]; 48000];
    for chunk in out.chunks_mut(MAX_BLOCK) {
        k.process(&ctx, &params, &[[0.0; 2]; MAX_BLOCK][..chunk.len()], chunk);
    }
    // Rising zero crossings in a stretch, to count cycles.
    let cycles = |from: usize, to: usize| out[from..to].windows(2).filter(|w| w[0][0] < 0.0 && w[1][0] >= 0.0).count();
    assert!(cycles(0, 480) >= 3, "it starts high: {} cycles in 10 ms", cycles(0, 480));
    let low = cycles(9600, 14400);
    assert!((5..=6).contains(&low), "then plays A1, 55 Hz: {low} cycles in 100 ms");
    assert!(out[40000..].iter().all(|f| f[0].abs() < 1e-3), "and is gone after the decay");
}

/// How much of channel 0 of `out` is at `f`, by correlating with it.
fn level_at(out: &[Frame], f: f32, sr: f32) -> f32 {
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (k, o) in out.iter().enumerate() {
        let ph = (TAU * f * k as f32 / sr) as f64;
        re += o[0] as f64 * ph.cos();
        im += o[0] as f64 * ph.sin();
    }
    ((re * re + im * im).sqrt() * 2.0 / out.len() as f64) as f32
}

#[test]
fn spectravoice_stacks_harmonics_at_their_slope() {
    let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
    let play = |set: &[(usize, f32)]| {
        let mut params: Vec<f32> = ModuleKind::SpectraVoice.params().iter().map(|p| p.default).collect();
        params[1] = 4.0;
        params[8] = 1.0;
        for &(i, v) in set {
            params[i] = v;
        }
        let mut s = create(ModuleKind::SpectraVoice, ctx.sr);
        s.note_on(0, 69.0, 1.0);
        let mut out = vec![[0.0; 2]; 24000];
        for chunk in out.chunks_mut(MAX_BLOCK) {
            s.process(&ctx, &params, &[[0.0; 2]; MAX_BLOCK][..chunk.len()], chunk);
        }
        out.split_off(12000)
    };
    let full = play(&[]);
    let (first, second) = (level_at(&full, 440.0, ctx.sr), level_at(&full, 880.0, ctx.sr));
    assert!(first > 0.05 && (second / first - 0.5).abs() < 0.02, "the second harmonic at half: {first} {second}");
    let hollow = play(&[(3, 0.0)]);
    let (first, second) = (level_at(&hollow, 440.0, ctx.sr), level_at(&hollow, 880.0, ctx.sr));
    assert!(second < first * 0.01, "no even harmonics: {first} {second}");
    assert!((level_at(&hollow, 1320.0, ctx.sr) / first - 1.0 / 3.0).abs() < 0.02, "the third at a third");
}

#[test]
fn pitch_shifter_moves_a_tone_up_an_octave() {
    let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
    let mut s = create(ModuleKind::PitchShifter, ctx.sr);
    let params: Vec<f32> = ModuleKind::PitchShifter.params().iter().map(|p| p.default).collect();
    let input: Vec<Frame> = (0..48000).map(|i| [0.5 * (TAU * 500.0 * i as f32 / ctx.sr).sin(); 2]).collect();
    let mut out = vec![[0.0; 2]; input.len()];
    s.process(&ctx, &params, &input, &mut out);
    let out = &out[12000..];
    let (up, at) = (level_at(out, 1000.0, ctx.sr), level_at(out, 500.0, ctx.sr));
    assert!(up > 0.3 && at < 0.05, "an octave up: {up} at 1 kHz, {at} left at 500 Hz");
}

#[test]
fn stereo_expander_scales_the_side_and_keeps_the_middle() {
    let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
    let run = |width: f32, mono_bass: f32, freq: f32| {
        let input: Vec<Frame> = (0..48000)
            .map(|i| {
                let x = (TAU * freq * i as f32 / ctx.sr).sin();
                [0.5 + 0.25 * x, 0.5 - 0.25 * x]
            })
            .collect();
        let mut out = vec![[0.0; 2]; input.len()];
        create(ModuleKind::StereoExpander, ctx.sr).process(&ctx, &[width, mono_bass], &input, &mut out);
        out[24000..].iter().fold((0.0f32, 0.0f32), |(m, s), f| (m.max((f[0] + f[1]) * 0.5), s.max((f[0] - f[1]) * 0.5)))
    };
    let (mid, side) = run(1.0, 0.0, 1000.0);
    assert!((mid - 0.5).abs() < 1e-4 && (side - 0.25).abs() < 1e-3, "unchanged at 100%: {mid} {side}");
    let (mid, side) = run(0.0, 0.0, 1000.0);
    assert!((mid - 0.5).abs() < 1e-4 && side < 1e-4, "mono at 0: {mid} {side}");
    let (_, side) = run(2.0, 0.0, 1000.0);
    assert!((side - 0.5).abs() < 1e-3, "twice as wide: {side}");
    let (_, side) = run(2.0, 400.0, 40.0);
    assert!(side < 0.1, "lows kept in the middle: {side}");
}

#[test]
fn comb_filter_rings_at_its_note_and_harmonics() {
    let wet = [(3, 0.0), (4, 1.0)];
    let at = |f: f32| effect_peak(ModuleKind::CombFilter, &wet, f, 0.1);
    let (tooth, harmonic, between) = (at(220.0), at(440.0), at(330.0));
    assert!(tooth > 0.09 && harmonic > 0.09, "A-3 and its octave come through: {tooth} {harmonic}");
    assert!(between < tooth * 0.2, "between them is cut: {between}");
}

#[test]
fn maximizer_boosts_up_to_the_ceiling_and_no_further() {
    let quiet = effect_peak(ModuleKind::Maximizer, &[], 100.0, 0.2);
    assert!((quiet - 0.4).abs() < 1e-3, "quiet sounds get the boost: {quiet}");
    let loud = effect_peak(ModuleKind::Maximizer, &[(0, 4.0), (1, 0.8)], 100.0, 0.5);
    assert!(loud <= 0.8 && loud > 0.75, "loud ones stop at the ceiling: {loud}");
}

#[test]
fn exciter_adds_to_the_highs_and_leaves_the_lows() {
    let low = effect_peak(ModuleKind::Exciter, &[], 100.0, 0.3);
    assert!((low - 0.3).abs() < 0.01, "lows pass: {low}");
    let high = effect_peak(ModuleKind::Exciter, &[], 10000.0, 0.3);
    assert!(high > 0.45, "highs get louder: {high}");
}

#[test]
fn dc_blocker_takes_away_an_offset() {
    let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
    let input: Vec<Frame> = (0..48000).map(|i| [0.5 + 0.3 * (TAU * 440.0 * i as f32 / ctx.sr).sin(); 2]).collect();
    let mut out = vec![[0.0; 2]; input.len()];
    create(ModuleKind::DcBlocker, ctx.sr).process(&ctx, &[10.0], &input, &mut out);
    let tail = &out[24000..];
    let mean = tail.iter().map(|f| f[0]).sum::<f32>() / tail.len() as f32;
    assert!(mean.abs() < 1e-3, "no offset left: {mean}");
    assert!((level_at(tail, 440.0, ctx.sr) - 0.3).abs() < 0.01, "the tone stays");
}

#[test]
fn scream_filter_filters_and_stays_bounded_at_full_resonance() {
    let gentle = [(2, 0.0), (3, 1.0)];
    let low = effect_peak(ModuleKind::ScreamFilter, &gentle, 100.0, 0.3);
    let high = effect_peak(ModuleKind::ScreamFilter, &gentle, 12000.0, 0.3);
    assert!(low > 0.2 && high < low * 0.1, "a lowpass: {low} {high}");
    let wild = effect_peak(ModuleKind::ScreamFilter, &[(2, 1.0), (3, 20.0)], 1500.0, 1.0);
    assert!(wild.is_finite() && wild < 1.5, "screams but stays bounded: {wild}");
}

#[test]
fn multitap_echoes_at_each_taps_time_level_and_pan() {
    let ctx = Ctx { sr: 48000.0, samples_per_line: 1000.0, song_line: None };
    let mut params: Vec<f32> = ModuleKind::Multitap.params().iter().map(|p| p.default).collect();
    params[12] = 0.0;
    params[13] = 1.0;
    let mut input = vec![[0.0; 2]; 6000];
    input[0] = [1.0; 2];
    let mut out = vec![[0.0; 2]; input.len()];
    create(ModuleKind::Multitap, ctx.sr).process(&ctx, &params, &input, &mut out);
    for (t, at) in [1000, 2000, 3000, 4000].into_iter().enumerate() {
        let (l, r) = pan_gains(params[t * 3 + 2]);
        let level = params[t * 3 + 1];
        assert!((out[at][0] - level * l).abs() < 1e-5 && (out[at][1] - level * r).abs() < 1e-5, "tap {t}");
    }
    assert!(out[4500..].iter().all(|f| f[0] == 0.0), "and no more without feedback");
}

#[test]
fn eq10_bands_move_their_octave() {
    let flat = effect_peak(ModuleKind::Eq10, &[], 1000.0, 0.25);
    assert!((flat - 0.25).abs() < 0.01, "flat at the defaults: {flat}");
    let boosted = effect_peak(ModuleKind::Eq10, &[(5, 4.0)], 1000.0, 0.25);
    assert!(boosted > 0.8, "the 1 kHz band boosts 1 kHz by 12 dB: {boosted}");
    let far = effect_peak(ModuleKind::Eq10, &[(5, 4.0)], 62.5, 0.25);
    assert!(far < 0.3, "and leaves 63 Hz about alone: {far}");
}

#[test]
fn cabinets_keep_the_middle_and_roll_off_the_ends() {
    for cabinet in 0..CABINET_BANDS.len() as u32 {
        let at = |f: f32| effect_peak(ModuleKind::Cabinet, &[(0, cabinet as f32), (1, 1.0)], f, 0.1);
        let (low, mid, high) = (at(30.0), at(1500.0), at(12000.0));
        assert!(mid > 0.05 && low < mid && high < mid * 0.3, "cabinet {cabinet}: {low} {mid} {high}");
    }
}

#[test]
fn vibrato_bends_a_tone_up_and_down_around_it() {
    let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
    let input: Vec<Frame> = (0..48000).map(|i| [(TAU * 1000.0 * i as f32 / ctx.sr).sin(); 2]).collect();
    let run = |depth: f32| {
        let mut out = vec![[0.0; 2]; input.len()];
        create(ModuleKind::Vibrato, ctx.sr).process(&ctx, &[5.0, depth, 0.0, 1.0], &input, &mut out);
        level_at(&out[4800..], 1000.0, ctx.sr)
    };
    assert!(run(0.0) > 0.95, "without depth the tone stays");
    assert!(run(1.0) < 0.8, "with it, part of the tone moves off 1 kHz: {}", run(1.0));
}

#[test]
fn every_module_stays_finite_at_the_ends_of_its_parameters() {
    let ctx = Ctx { sr: 44100.0, samples_per_line: 5512.0, song_line: None };
    let mut rng = Rng(0x2468_ace1);
    let noise: Vec<Frame> = (0..MAX_BLOCK).map(|_| [rng.next(), rng.next()]).collect();
    for kind in ModuleKind::ADDABLE {
        let specs = kind.params();
        let settings: [Vec<f32>; 3] = [
            specs.iter().map(|p| p.min).collect(),
            specs.iter().map(|p| p.max).collect(),
            specs.iter().map(|p| p.default).collect(),
        ];
        for params in &settings {
            let mut m = create(kind, ctx.sr);
            m.note_on(0, 60.0, 1.0);
            m.note_on(1, 24.0, 1.0);
            let mut peak = 0f32;
            for block in 0..700 {
                if block == 350 {
                    m.note_off(0);
                    m.note_off(1);
                }
                let mut out = [[0.0; 2]; MAX_BLOCK];
                m.process(&ctx, params, &noise, &mut out);
                for f in out {
                    assert!(f[0].is_finite() && f[1].is_finite(), "{} at {params:?}", kind.name());
                    peak = peak.max(f[0].abs()).max(f[1].abs());
                }
            }
            assert!(peak < 50.0, "{} blows up to {peak} at {params:?}", kind.name());
        }
    }
}

#[test]
fn fmx_operators_modulate_by_their_algorithm() {
    let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
    let play = |set: &[(usize, f32)]| {
        let mut params: Vec<f32> = ModuleKind::Fmx.params().iter().map(|p| p.default).collect();
        // Sustained operators, so the tone holds still.
        for k in 0..OPS {
            params[3 + k * 6 + 4] = 1.0;
        }
        for &(i, v) in set {
            params[i] = v;
        }
        let mut s = create(ModuleKind::Fmx, ctx.sr);
        s.note_on(0, 69.0, 1.0);
        let mut out = vec![[0.0; 2]; 24000];
        for chunk in out.chunks_mut(MAX_BLOCK) {
            s.process(&ctx, &params, &[[0.0; 2]; MAX_BLOCK][..chunk.len()], chunk);
        }
        out.split_off(12000)
    };
    // Only operator 1 at its full level: a sine at the note.
    let only_one = [(9, 0.0), (15, 0.0), (21, 0.0)];
    let pure = play(&only_one);
    let (f0, f2) = (level_at(&pure, 440.0, ctx.sr), level_at(&pure, 880.0, ctx.sr));
    assert!(f0 > 0.3 && f2 < f0 * 0.01, "a pure tone: {f0} {f2}");
    // Operator 2 modulating it in algorithm 1 adds harmonics.
    let modulated = play(&[(15, 0.0), (21, 0.0)]);
    assert!(level_at(&modulated, 880.0, ctx.sr) > 0.05, "operator 2 brings harmonics");
    // In algorithm 8 operator 2 is heard beside 1 rather than bending it.
    let apart = play(&[(1, 7.0), (10, 2.0), (15, 0.0), (21, 0.0)]);
    let (one, two) = (level_at(&apart, 440.0, ctx.sr), level_at(&apart, 880.0, ctx.sr));
    assert!(one > 0.2 && two > 0.1 && level_at(&apart, 1320.0, ctx.sr) < 0.01, "two sines: {one} {two}");
}

#[test]
fn input_plays_what_arrives_on_its_tape() {
    let ctx = Ctx { sr: 1000.0, samples_per_line: 100.0, song_line: None };
    let tape = Arc::new(Tape::default());
    tape.start(4096);
    let mut input = LiveInput::new(tape.clone());
    let frames: Vec<Frame> = (0..100).map(|i| [i as f32 / 100.0, -(i as f32) / 100.0]).collect();
    tape.push(&frames);
    let mut out = [[0.0; 2]; 50];
    input.process(&ctx, &[2.0, 0.0], &[], &mut out);
    assert_eq!(out[10], [0.2, -0.2], "at its volume");
    input.process(&ctx, &[1.0, 1.0], &[], &mut out);
    assert_eq!(out[0], [0.5, 0.5], "the left side on both, picking up where it was");
    // Twice the rate: every other frame.
    tape.set_rate(2000);
    tape.push(&frames);
    input.process(&ctx, &[1.0, 0.0], &[], &mut out);
    // The last frame of before is held over, so it joins on smoothly.
    assert!((out[2][0] - out[1][0] - 0.02).abs() < 1e-5, "{:?}", &out[..3]);
}

#[test]
fn waveshaper_bends_the_sound_through_its_curve() {
    let defaults: Vec<f32> = ModuleKind::WaveShaper.params().iter().map(|p| p.default).collect();
    let points = &defaults[4..];
    for x in [-0.9, -0.3, 0.0, 0.4, 1.0] {
        assert!((shape(points, true, x) - x).abs() < 1e-6, "the default curve is a straight line at {x}");
    }
    // A curve flat from +50% clips there, on both sides when symmetric.
    let mut clip = points.to_vec();
    clip[7] = 0.5;
    clip[8] = 0.5;
    assert_eq!(shape(&clip, true, 0.9), 0.5);
    assert_eq!(shape(&clip, true, -0.9), -0.5);
    assert_eq!(shape(&clip, false, -0.9), -0.9, "not symmetric: the left half is its own");
    let peak = effect_peak(ModuleKind::WaveShaper, &[(11, 0.5), (12, 0.5)], 100.0, 0.9);
    assert!((peak - 0.5).abs() < 1e-3, "{peak}");
}

#[test]
fn filter_pro_types_and_slopes() {
    let at = |set: &[(usize, f32)], f: f32| effect_peak(ModuleKind::FilterPro, set, f, 0.5);
    let (pass, cut) = (at(&[], 100.0), at(&[], 8000.0));
    assert!(pass > 0.45 && cut < 0.05, "a lowpass at 1 kHz: {pass} {cut}");
    let steep = at(&[(4, 3.0)], 8000.0);
    assert!(steep < cut * 0.1, "48 dB an octave cuts much more: {steep} against {cut}");
    let high = at(&[(0, 1.0)], 100.0);
    assert!(high < 0.05, "a highpass cuts the lows: {high}");
    let notch = at(&[(0, 3.0), (2, 2.0)], 1000.0);
    assert!(notch < 0.01, "a notch takes out its frequency: {notch}");
    let peak = at(&[(0, 5.0), (3, 4.0)], 1000.0);
    assert!((peak - 2.0).abs() < 0.05, "a peak of 12 dB: {peak}");
}

#[test]
fn chorus_voices_spread_a_tone_around_itself() {
    let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
    let input: Vec<Frame> = (0..48000).map(|i| [(TAU * 1000.0 * i as f32 / ctx.sr).sin(); 2]).collect();
    let run = |set: &[(usize, f32)]| {
        let mut params: Vec<f32> = ModuleKind::Chorus.params().iter().map(|p| p.default).collect();
        params[6] = 1.0;
        for &(i, v) in set {
            params[i] = v;
        }
        let mut out = vec![[0.0; 2]; input.len()];
        create(ModuleKind::Chorus, ctx.sr).process(&ctx, &params, &input, &mut out);
        out.split_off(4800)
    };
    let still = run(&[(2, 0.0), (0, 1.0)]);
    assert!(level_at(&still, 1000.0, ctx.sr) > 0.95, "one unswept voice is the tone, delayed");
    let moving = run(&[]);
    let left_right = moving.iter().map(|f| (f[0] - f[1]).abs()).fold(0f32, f32::max);
    assert!(level_at(&moving, 1000.0, ctx.sr) < 0.9 && left_right > 0.1, "swept voices smear it, wider than mono");
}

#[test]
fn echo_repeats_after_its_time_and_fades() {
    let ctx = Ctx { sr: 1000.0, samples_per_line: 100.0, song_line: None };
    let mut params: Vec<f32> = ModuleKind::Echo.params().iter().map(|p| p.default).collect();
    params[2] = 0.0;
    params[3] = 0.0;
    params[4] = 1.0;
    let mut input = vec![[0.0; 2]; 1000];
    input[0] = [1.0; 2];
    let mut out = vec![[0.0; 2]; 1000];
    create(ModuleKind::Echo, ctx.sr).process(&ctx, &params, &input, &mut out);
    assert!((out[300][0] - 1.0).abs() < 1e-4 && (out[600][0] - 0.5).abs() < 1e-4, "{} {}", out[300][0], out[600][0]);
    assert!(out[150][0].abs() < 1e-6, "nothing between");
}

#[test]
fn analog_filter_cuts_and_sings() {
    let at = |set: &[(usize, f32)], f: f32| effect_peak(ModuleKind::AnalogFilter, set, f, 0.3);
    let dry = [(2, 0.0)];
    assert!(at(&dry, 100.0) > 0.25 && at(&dry, 8000.0) < 0.01, "a 24 dB lowpass");
    assert!(at(&[(0, 3.0), (2, 0.0)], 100.0) < 0.05, "the highpass cuts the lows");
    // At full resonance an impulse leaves it ringing at the cutoff.
    let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
    let mut input = vec![[0.0; 2]; 24000];
    input[0] = [0.5; 2];
    let mut out = vec![[0.0; 2]; input.len()];
    create(ModuleKind::AnalogFilter, ctx.sr).process(&ctx, &[0.0, 1000.0, 1.0, 1.0, 1.0], &input, &mut out);
    let late = out[20000..].iter().fold(0f32, |m, f| m.max(f[0].abs()));
    assert!(late > 0.01 && late < 2.0, "it sings by itself, and stays bounded: {late}");
}

#[test]
fn plate_reverb_rings_on_longer_with_more_decay() {
    let ctx = Ctx { sr: 44100.0, samples_per_line: 5512.0, song_line: None };
    let tail = |decay: f32| {
        let mut input = vec![[0.0; 2]; 88200];
        input[0] = [1.0; 2];
        let mut out = vec![[0.0; 2]; input.len()];
        create(ModuleKind::PlateReverb, ctx.sr).process(&ctx, &[decay, 0.0, 0.3, 1.0, 1.0], &input, &mut out);
        let energy = |r: std::ops::Range<usize>| out[r].iter().map(|f| f[0] * f[0] + f[1] * f[1]).sum::<f32>();
        (energy(4410..22050), energy(44100..88200), out[1..].iter().any(|f| (f[0] - f[1]).abs() > 1e-4))
    };
    let (short_early, short_late, _) = tail(0.2);
    let (long_early, long_late, wide) = tail(0.9);
    assert!(short_early > 0.0 && long_early > 0.0, "it rings");
    assert!(long_late > short_late * 100.0, "more decay rings on: {long_late} {short_late}");
    assert!(wide, "the sides differ");
}

#[test]
fn eq5_bands_move_their_frequencies() {
    let flat = effect_peak(ModuleKind::Eq5, &[], 1200.0, 0.25);
    assert!((flat - 0.25).abs() < 0.01, "flat at the defaults: {flat}");
    let mid = effect_peak(ModuleKind::Eq5, &[(7, 4.0)], 1200.0, 0.25);
    assert!((mid - 1.0).abs() < 0.05, "the mid band boosts 1.2 kHz by 12 dB: {mid}");
    let narrow = effect_peak(ModuleKind::Eq5, &[(7, 4.0), (8, 10.0)], 600.0, 0.25);
    assert!(narrow < 0.3, "a narrow one leaves an octave below alone: {narrow}");
}

#[test]
fn eq_bands_boost_their_frequencies() {
    let flat = effect_peak(ModuleKind::Eq, &[], 100.0, 0.5);
    assert!((flat - 0.5).abs() < 0.01, "flat EQ changes nothing: {flat}");
    let low = effect_peak(ModuleKind::Eq, &[(0, 4.0)], 50.0, 0.5);
    assert!((low - 2.0).abs() < 0.1, "+12 dB low shelf: {low}");
    let high_at_low = effect_peak(ModuleKind::Eq, &[(2, 4.0)], 50.0, 0.5);
    assert!((high_at_low - 0.5).abs() < 0.05, "high shelf leaves 50 Hz: {high_at_low}");
    let mid = effect_peak(ModuleKind::Eq, &[(1, 0.25)], 1000.0, 0.5);
    assert!((mid - 0.125).abs() < 0.02, "-12 dB mid: {mid}");
}

#[test]
fn compressor_turns_down_loud_input() {
    // Threshold 0.25, ratio 4: a peak of 1 (12 dB over) comes out 3 dB over.
    let loud = effect_peak(ModuleKind::Compressor, &[], 100.0, 1.0);
    assert!(loud < 0.5 && loud > 0.3, "{loud}");
    let quiet = effect_peak(ModuleKind::Compressor, &[], 100.0, 0.1);
    assert!((quiet - 0.1).abs() < 0.01, "{quiet}");
}

#[test]
fn distortion_crushes_bits() {
    let sr = 48000.0;
    let ctx = Ctx { sr, samples_per_line: 6000.0, song_line: None };
    // Drive 1 and full tone keep the shape; 2 bits leave 2 levels a side.
    let params = [1.0, 1.0, 1.0, 1.0, 2.0, 1.0];
    let input: Vec<Frame> = (0..1000).map(|i| [(i as f32 / 1000.0) * 2.0 - 1.0; 2]).collect();
    let mut out = vec![[0.0; 2]; input.len()];
    let mut d = Distortion { lp: [0.0; 2], ..Default::default() };
    // Tone at 1 makes the smoothing filter pass everything at once.
    d.process(&ctx, &params, &input, &mut out);
    let mut levels: Vec<i32> = out.iter().map(|f| (f[0] / 0.7 * 100.0).round() as i32).collect();
    levels.dedup();
    assert_eq!(levels, vec![-100, -50, 0, 50, 100]);
}

#[test]
fn lfo_tremolo_and_sync() {
    // Full depth: the volume dips to 0 and back once per period.
    let ctx = Ctx { sr: 1000.0, samples_per_line: 10.0, song_line: None };
    // Tremolo, sine, depth 1, rate ignored, synced to 10 lines.
    let params = [0.0, 0.0, 1.0, 2.0, 1.0, 10.0];
    let input = vec![[1.0; 2]; 100];
    let mut out = vec![[0.0; 2]; 100];
    Lfo::default().process(&ctx, &params, &input, &mut out);
    assert!((out[25][0] - 1.0).abs() < 1e-3, "top at a quarter: {}", out[25][0]);
    assert!(out[75][0].abs() < 1e-3, "silent at three quarters: {}", out[75][0]);
}

#[test]
fn flanger_passes_signal() {
    let peak = effect_peak(ModuleKind::Flanger, &[(5, 0.0)], 100.0, 0.5);
    assert!((peak - 0.5).abs() < 1e-3, "dry: {peak}");
    let chorus = effect_peak(ModuleKind::Flanger, &[(0, 1.0)], 100.0, 0.5);
    assert!(chorus > 0.3 && chorus < 1.0, "{chorus}");
}

#[test]
fn pitch_envelope_bends_and_holds_at_sustain() {
    let mut s = Sampler::new();
    s.set_samples(&[ramp_slot(100_000)]);
    // Up an octave at once, back to the note by 0.2 s, holding at the
    // octave (point 1) while the key is down.
    let pitch = VoiceEnvelope {
        on: true,
        points: vec![(0.0, 1.0), (0.1, 1.0), (0.2, 0.5)],
        sustain: Some(1),
        curve: false,
        amount: 12.0,
    };
    let m = Modulation { pitch, ..Modulation::default() };
    s.set_modulation(&m);
    s.note_on(0, 60.0, 1.0);
    render(&mut s, 1000);
    let v = s.voices.iter().find(|v| v.env.active()).unwrap();
    assert!((v.pos - 2000.0).abs() < 70.0, "twice as fast for a second: {}", v.pos);
    assert!((v.md.pitch_t - 0.1).abs() < 1e-6, "held at the sustain point");
    s.note_off(0);
    // A second's release keeps the voice going.
    let ctx = Ctx { sr: SR, samples_per_line: 100.0, song_line: None };
    let mut out = vec![[0.0; 2]; 300];
    s.process(&ctx, &[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0], &[], &mut out);
    let v = s.voices.iter().find(|v| v.env.active()).unwrap();
    assert!(v.md.pitch_t > 0.2, "on past it once released: {}", v.md.pitch_t);
}

#[test]
fn voice_filter_and_tremolo() {
    // A sample at the highest frequency there is.
    let frames: Vec<Frame> = (0..4000).map(|i| [if i % 2 == 0 { 0.5 } else { -0.5 }; 2]).collect();
    let sample = Sample { name: "nyquist".into(), sample_rate: SR, channels: 1, frames };
    let mut slot = SampleSlot::new(sample, None);
    slot.base_note = 60;
    let peak = |m: &Modulation| {
        let mut s = Sampler::new();
        s.set_samples(std::slice::from_ref(&slot));
        s.set_modulation(m);
        s.note_on(0, 60.0, 1.0);
        render(&mut s, 2000)[1000..].iter().fold(0f32, |a, x| a.max(x.abs()))
    };
    let open = peak(&Modulation::default());
    assert!(open > 0.4, "{open}");
    let low = Modulation { filter: true, cutoff: 20.0, ..Modulation::default() };
    assert!(peak(&low) < open * 0.05, "a 20 Hz lowpass takes it away: {}", peak(&low));

    // Full tremolo dips to silence once a cycle.
    let trem = Modulation {
        tremolo: VoiceLfo { on: true, shape: 0, rate: 4.0, depth: 1.0, delay: 0.0 },
        ..Modulation::default()
    };
    let mut s = Sampler::new();
    s.set_samples(std::slice::from_ref(&slot));
    s.set_modulation(&trem);
    s.note_on(0, 60.0, 1.0);
    let mut out = Vec::new();
    for _ in 0..16 {
        out.extend(render(&mut s, 32));
    }
    let quietest = out[100..].chunks(8).map(|c| c.iter().fold(0f32, |a, x| a.max(x.abs()))).fold(1f32, f32::min);
    assert!(quietest < 0.1, "{quietest}");
}

/// Plays A-4 on a fresh `kind` with `m` and counts the zero crossings
/// and the peak of a second of the left channel, past the attack.
fn synth_with(kind: ModuleKind, m: &Modulation) -> (usize, f32) {
    let sr = 48000.0;
    let ctx = Ctx { sr, samples_per_line: 6000.0, song_line: None };
    let mut params: Vec<f32> = kind.params().iter().map(|p| p.default).collect();
    if kind == ModuleKind::Generator {
        params[1] = 3.0; // sine
    }
    let mut dsp = create(kind, sr);
    dsp.set_modulation(m);
    dsp.note_on(0, 69.0, 1.0);
    let mut out = vec![[0.0; 2]; 64];
    let mut left = Vec::new();
    for _ in 0..(sr as usize / 64) {
        dsp.process(&ctx, &params, &[], &mut out);
        left.extend(out.iter().map(|f| f[0]));
    }
    let tail = &left[4800..];
    let crossings = tail.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
    (crossings, tail.iter().fold(0f32, |a, x| a.max(x.abs())))
}

#[test]
fn synths_take_modulation() {
    let plain = synth_with(ModuleKind::Generator, &Modulation::default());
    let pitch = VoiceEnvelope { on: true, points: vec![(0.0, 1.0)], sustain: None, curve: false, amount: 12.0 };
    let up = Modulation { pitch, ..Modulation::default() };
    let octave = synth_with(ModuleKind::Generator, &up);
    let ratio = octave.0 as f32 / plain.0 as f32;
    assert!((ratio - 2.0).abs() < 0.05, "an octave up: {ratio}");
    let shut = Modulation { filter: true, cutoff: 20.0, resonance: 0.0, ..Modulation::default() };
    let filtered = synth_with(ModuleKind::Generator, &shut);
    assert!(filtered.1 < plain.1 * 0.05, "{} {}", filtered.1, plain.1);
    let fm = synth_with(ModuleKind::Fm, &Modulation::default());
    let fm_up = synth_with(ModuleKind::Fm, &up);
    assert!(fm_up.0 as f32 > fm.0 as f32 * 1.5, "{} {}", fm_up.0, fm.0);
}

#[test]
fn slices_play_on_the_notes_after_the_base_note() {
    let mut s = Sampler::new();
    let mut slot = ramp_slot(100);
    slot.slices = vec![25, 50];
    s.set_samples(&[slot]);
    // Slice 2 (frames 25..50) is on D-5, two notes above C-5.
    s.note_on(0, 62.0, 1.0);
    let out = render(&mut s, 40);
    assert!((out[0] - 0.25).abs() < 0.02, "starts at its marker: {}", out[0]);
    assert!(out[30].abs() < 1e-6, "stops at the next one: {}", out[30]);
    assert_eq!(s.voices.iter().filter(|v| v.env.active()).count(), 0);
    // The whole sample is on the base note only.
    s.note_on(1, 60.0, 1.0);
    let out = render(&mut s, 80);
    assert!((out[70] - 0.7).abs() < 0.02, "{}", out[70]);
    s.note_on(2, 70.0, 1.0);
    assert!(s.voices.iter().filter(|v| v.env.active()).count() <= 1, "no zone above the slices");
    let mut heads = Vec::new();
    s.playheads(&mut heads);
    assert!(heads.iter().all(|p| p.slot == 0), "playheads name the slot");
}

#[test]
fn slices_have_settings_of_their_own() {
    let mut s = Sampler::new();
    let mut slot = ramp_slot(100);
    slot.slices = vec![50];
    // The second slice is at half volume and loops.
    *slot.slice_mut(1) = crate::project::SliceSettings { volume: 0.5, loop_mode: 1, ..Default::default() };
    s.set_samples(&[slot]);
    s.note_on(0, 62.0, 1.0);
    let out = render(&mut s, 120);
    assert!((out[0] - 0.25).abs() < 0.02, "half of 0.5: {}", out[0]);
    assert!((out[60] - 0.3).abs() < 0.02, "looped back to its start: {}", out[60]);
}

#[test]
fn the_voice_filter_follows_the_key_and_velocity() {
    let sr = 48000.0;
    let ctx = Ctx { sr, samples_per_line: 6000.0, song_line: None };
    let params: Vec<f32> = ModuleKind::Generator.params().iter().map(|p| p.default).collect();
    // How loud a saw on `note` comes through the voice filter, per velocity.
    let level = |m: &Modulation, note: f32, vel: f32| {
        let mut dsp = create(ModuleKind::Generator, sr);
        dsp.set_modulation(m);
        dsp.note_on(0, note, vel);
        let mut out = vec![[0.0; 2]; 64];
        let mut sum = 0.0;
        for k in 0..400 {
            dsp.process(&ctx, &params, &[], &mut out);
            if k > 100 {
                sum += out.iter().map(|f| f[0] * f[0]).sum::<f32>();
            }
        }
        (sum / vel / vel).sqrt()
    };
    let low = Modulation { filter: true, cutoff: 150.0, resonance: 0.0, ..Modulation::default() };
    let tracked = Modulation { key_track: 1.0, ..low.clone() };
    // Three octaves above C-4 the plain filter cuts most of a saw; the
    // tracked one moves up with the note.
    assert!(level(&tracked, 84.0, 1.0) > 2.0 * level(&low, 84.0, 1.0));
    // At C-4 tracking changes nothing.
    assert!((level(&tracked, 48.0, 1.0) / level(&low, 48.0, 1.0) - 1.0).abs() < 0.01);
    // A soft note closes a filter that follows velocity; a hard one doesn't.
    let soft = Modulation { cutoff: 2000.0, velocity: 3.0, ..low.clone() };
    let open = Modulation { cutoff: 2000.0, ..low };
    assert!(level(&soft, 60.0, 0.25) < 0.7 * level(&open, 60.0, 0.25));
    assert!((level(&soft, 60.0, 1.0) / level(&open, 60.0, 1.0) - 1.0).abs() < 0.01);
}

/// The Analog Synth's defaults with `set` changed.
fn analog_params(set: &[(usize, f32)]) -> Vec<f32> {
    let mut p: Vec<f32> = ModuleKind::Analog.params().iter().map(|p| p.default).collect();
    for &(i, v) in set {
        p[i] = v;
    }
    p
}

/// Renders `frames` of the left channel of `dsp` in blocks.
fn left(dsp: &mut dyn Dsp, params: &[f32], frames: usize) -> Vec<f32> {
    let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
    let mut out = vec![[0.0; 2]; frames];
    for chunk in out.chunks_mut(MAX_BLOCK) {
        dsp.process(&ctx, params, &[[0.0; 2]; MAX_BLOCK][..chunk.len()], chunk);
    }
    out.iter().map(|f| f[0]).collect()
}

/// Rising zero crossings a second in `x`, at 48 kHz.
fn pitch_of(x: &[f32]) -> f32 {
    x.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count() as f32 * 48000.0 / x.len() as f32
}

#[test]
fn analog_synth_plays_its_note_through_a_closing_filter() {
    // Sines, wide open, so the pitch is plain to count.
    let sine = analog_params(&[(1, 3.0), (2, 3.0), (4, 0.0), (10, 5000.0), (11, 0.0), (12, 0.0)]);
    let mut s = create(ModuleKind::Analog, 48000.0);
    s.note_on(0, 69.0, 1.0);
    let x = left(s.as_mut(), &sine, 24000);
    let hz = pitch_of(&x[4800..]);
    assert!((hz - 440.0).abs() < 5.0, "A-4: {hz}");
    // A saw through the filter envelope: bright at first, darker once it
    // has fallen to its sustain.
    let saw = analog_params(&[(5, 0.0), (10, 300.0), (12, 5.0), (16, 1.0), (17, 0.0)]);
    let mut s = create(ModuleKind::Analog, 48000.0);
    s.note_on(0, 57.0, 1.0);
    let x = left(s.as_mut(), &saw, 48000);
    // The fifth harmonic of A-3 against its fundamental.
    let bright = |from: usize| {
        let f: Vec<Frame> = x[from..from + 2400].iter().map(|&v| [v, v]).collect();
        level_at(&f, 1100.0, 48000.0) / level_at(&f, 220.0, 48000.0)
    };
    assert!(bright(0) > 4.0 * bright(40000), "{} {}", bright(0), bright(40000));
}

#[test]
fn analog_synth_mono_voices_glide_and_legato_keeps_the_envelope() {
    let sine = [(1, 3.0), (2, 3.0), (4, 0.0), (10, 5000.0), (11, 0.0), (12, 0.0)];
    let mono = analog_params(&[sine.as_slice(), &[(29, 1.0), (30, 0.05)]].concat());
    let mut s = create(ModuleKind::Analog, 48000.0);
    s.note_on(0, 57.0, 1.0);
    left(s.as_mut(), &mono, 9600);
    s.note_on(1, 69.0, 1.0);
    let x = left(s.as_mut(), &mono, 48000);
    let early = pitch_of(&x[..1200]);
    let late = pitch_of(&x[24000..]);
    assert!(early < 400.0, "still on its way up: {early}");
    assert!((late - 440.0).abs() < 5.0, "then there: {late}");
    // One voice: the first note gave way.
    let mut heads = 0;
    let mut s2 = create(ModuleKind::Analog, 48000.0);
    s2.note_on(0, 57.0, 1.0);
    s2.note_on(1, 60.0, 1.0);
    left(s2.as_mut(), &mono, 64);
    s2.note_off(1);
    let tail = left(s2.as_mut(), &mono, 48000);
    heads += tail[40000..].iter().filter(|v| v.abs() > 1e-3).count();
    assert_eq!(heads, 0, "letting go of the note playing ends it");

    // Legato: a second note while the first is held doesn't start the
    // filter envelope again, so it stays dark where Mono opens it.
    let sweep = [(5, 0.0), (10, 300.0), (12, 5.0), (16, 1.0), (17, 0.0)];
    let legato = analog_params(&[sweep.as_slice(), &[(29, 2.0)]].concat());
    let retrig = analog_params(&[sweep.as_slice(), &[(29, 1.0)]].concat());
    let second = |p: &[f32]| {
        let mut s = create(ModuleKind::Analog, 48000.0);
        s.note_on(0, 57.0, 1.0);
        left(s.as_mut(), p, 48000);
        s.note_on(1, 57.0, 1.0);
        let f: Vec<Frame> = left(s.as_mut(), p, 2400).iter().map(|&v| [v, v]).collect();
        level_at(&f, 1100.0, 48000.0) / level_at(&f, 220.0, 48000.0)
    };
    assert!(second(&retrig) > 4.0 * second(&legato), "{} {}", second(&legato), second(&retrig));
}
