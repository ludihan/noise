use super::*;

#[test]
fn processes_change_frames() {
    let mut f = vec![[0.5, -0.25], [0.7, 0.1]];
    process(&mut f, Op::SwapChannels);
    assert_eq!(f[0], [-0.25, 0.5]);
    process(&mut f, Op::Invert);
    assert_eq!(f[0], [0.25, -0.5]);
    process(&mut f, Op::MixToMono);
    assert_eq!(f[0], [-0.125, -0.125]);
    let mut dc = vec![[1.0, 0.5], [3.0, 0.5]];
    process(&mut dc, Op::RemoveDc);
    assert_eq!(dc, [[-1.0, 0.0], [1.0, 0.0]]);
    let mut g = vec![[1.0, 1.0]];
    process(&mut g, Op::Gain(-6.0206));
    assert!((g[0][0] - 0.5).abs() < 1e-4);
}

#[test]
fn crossfaded_loop_ends_where_it_starts() {
    // A ramp: the loop 40..80 jumps from 79 back to 40 without a fade.
    let mut f: Vec<Frame> = (0..100).map(|i| [i as f32; 2]).collect();
    assert!(crossfade_loop(&mut f, 40, 80));
    // Its last frame now matches the one just before the start, so
    // wrapping to the start continues smoothly.
    assert_eq!(f[79][0], 39.0);
    assert_eq!(f[69][0], 69.0, "before the fade");
    assert!(!crossfade_loop(&mut f, 0, 80), "nothing before the loop");
}

#[test]
fn beats_are_found_where_hits_start() {
    // Four clicks that ring out, 0.25 s apart, in a second at 8 kHz.
    let sr = 8000.0;
    let mut frames = vec![[0.0f32; 2]; 8000];
    for hit in [0, 2000, 4000, 6000] {
        for k in 0..1500 {
            let x = (k as f32 * 0.3).sin() * (-(k as f32) / 300.0).exp();
            frames[hit + k] = [x, x];
        }
    }
    let beats = detect_beats(&frames, sr, 0.5);
    assert_eq!(beats.len(), 3, "the first hit is the start: {beats:?}");
    for (b, want) in beats.iter().zip([2000, 4000, 6000]) {
        assert!(b.abs_diff(want) <= 40, "{beats:?}");
    }
    assert!(detect_beats(&[[0.0; 2]; 4000], sr, 1.0).is_empty(), "nothing in silence");
    // A quiet hit is let in only by a higher sensitivity.
    for k in 0..400 {
        let x = 0.05 * (k as f32 * 0.3).sin() * (-(k as f32) / 300.0).exp();
        frames[7600 + k] = [x, x];
    }
    assert_eq!(detect_beats(&frames, sr, 0.0).len(), 3);
    assert_eq!(detect_beats(&frames, sr, 1.0).len(), 4);
}
