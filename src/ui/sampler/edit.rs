//! Editing samples: the processes, time stretch, slicing (by hand, evenly
//! or by beats) and rendering slices to samples of their own.

use super::*;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Op {
    Reverse,
    Normalize,
    FadeIn,
    FadeOut,
    Silence,
    Crop,
    Delete,
    Cut,
    Copy,
    Paste,
    Invert,
    RemoveDc,
    MixToMono,
    SwapChannels,
    /// Change the level by this many decibels.
    Gain(f32),
    CrossfadeLoop,
    /// Make it this many times as long, at the same pitch.
    Stretch(f64),
}

/// The processes in the Process menu.
pub(super) const PROCESSES: [(&str, &str, Op); 7] = [
    ("Invert", "Turn the waveform upside down", Op::Invert),
    ("Remove DC Offset", "Center the waveform on zero", Op::RemoveDc),
    ("Mix to Mono", "Put the average of both channels in each", Op::MixToMono),
    ("Swap Channels", "Swap left and right", Op::SwapChannels),
    ("+3 dB", "Make it louder", Op::Gain(3.0)),
    ("−3 dB", "Make it quieter", Op::Gain(-3.0)),
    (
        "Crossfade Loop",
        "Blend the end of the loop into what comes before its start, so it wraps without a click",
        Op::CrossfadeLoop,
    ),
];

/// Time Stretch: the selection, or the whole sample, made longer or
/// shorter at the same pitch, by a ratio or to fill lines of the song.
pub(super) fn stretch_menu(app: &mut App, ui: &mut egui::Ui, id: u8, i: usize, frames: usize) {
    let tip = "Longer or shorter without changing the pitch (the selection, or the whole sample)";
    ui.menu_button("Time Stretch", |ui| {
        let mut ratio = None;
        for (label, r) in [("Half as Long", 0.5), ("75%", 0.75), ("90%", 0.9), ("110%", 1.1), ("125%", 1.25)] {
            if ui.button(label).clicked() {
                ratio = Some(r);
            }
        }
        for (label, r) in [("150%", 1.5), ("Twice as Long", 2.0)] {
            if ui.button(label).clicked() {
                ratio = Some(r);
            }
        }
        ui.separator();
        // A line at the song's tempo, in the sample's frames.
        let rate = slots(app, id).get(i).and_then(|s| s.data.as_ref()).map_or(44100.0, |d| d.sample_rate);
        let line = 60.0 / (app.project.bpm.max(1.0) as f64 * app.project.lpb.max(1) as f64) * rate as f64;
        for lines in [1, 2, 4, 8, 16, 32, 64] {
            let r = lines as f64 * line / frames.max(1) as f64;
            let label = format!("Fit to {lines} Line{} ({:.0}%)", if lines == 1 { "" } else { "s" }, r * 100.0);
            if ui.add_enabled((0.25..=4.0).contains(&r), egui::Button::new(label)).clicked() {
                ratio = Some(r);
            }
        }
        if let Some(r) = ratio {
            edit(app, id, i, Op::Stretch(r));
            app.set_status(format!("Stretched to {:.0}%", r * 100.0));
            ui.close();
        }
    })
    .response
    .on_hover_text(tip);
}

/// The Slices menu: markers made evenly or at the beats, cleared, or
/// turned into samples of their own.
pub(super) fn slices_menu(app: &mut App, ui: &mut egui::Ui, id: u8, i: usize) {
    let Some(slot) = slots(app, id).get(i).cloned() else { return };
    let Some(data) = slot.data.clone() else { return };
    let len = data.len();
    let mut slices = None;
    if let Some((a, b)) = app.sampler.selection
        && ui.button("Add Markers at Selection").on_hover_text("At its start and its end").clicked()
    {
        let s = slot_mut(app, id, i).unwrap();
        s.add_slice(a);
        s.add_slice(b);
        app.mark();
        ui.close();
        return;
    }
    ui.menu_button("Divide Evenly", |ui| {
        for n in [2, 4, 8, 16, 32, 64] {
            if ui.button(format!("{n} slices")).clicked() {
                slices = Some((1..n).map(|k| len * k / n).collect());
            }
        }
    });
    ui.menu_button("Detect Beats", |ui| {
        let tip = "A marker where each hit starts, for drum loops";
        for (label, sensitivity) in [("Only Strong Hits", 0.25), ("Most Hits", 0.5), ("Every Hit", 1.0)] {
            if ui.button(label).on_hover_text(tip).clicked() {
                slices = Some(detect_beats(&data.frames, data.sample_rate, sensitivity));
            }
        }
    });
    if ui.add_enabled(!slot.slices.is_empty(), egui::Button::new("Clear Markers")).clicked() {
        slices = Some(Vec::new());
    }
    ui.separator();
    let tip = "Make each slice a sample of its own on its note, and clear the markers";
    if ui
        .add_enabled(!slot.slices.is_empty(), egui::Button::new("Render Slices to Samples"))
        .on_hover_text(tip)
        .clicked()
    {
        render_slices(app, id, i);
        ui.close();
        return;
    }
    if let Some(markers) = slices {
        // New markers, new slices: their settings start over.
        let s = slot_mut(app, id, i).unwrap();
        s.slices = markers;
        s.slice_settings.clear();
        s.clamp_slices();
        let n = s.slices.len();
        app.sampler.slice = None;
        app.mark();
        app.set_status(if n == 0 { "No slice markers".to_string() } else { format!("{} slices", n + 1) });
        ui.close();
    }
}

/// Where hits start in `frames`: points where the level jumps well above
/// what came just before, at least 80 ms apart. `sensitivity` (0..1) lets
/// in smaller jumps and quieter hits.
pub fn detect_beats(frames: &[Frame], sample_rate: f32, sensitivity: f32) -> Vec<usize> {
    let jump = 2.0 + 14.0 * (1.0 - sensitivity.clamp(0.0, 1.0));
    let loudest = frames.iter().fold(0f32, |m, f| m.max(f[0] * f[0] + f[1] * f[1]));
    let floor = loudest * 0.02 * (1.0 - sensitivity.clamp(0.0, 1.0)) + 1e-5;
    let win = ((sample_rate * 0.005) as usize).max(16);
    let energy: Vec<f32> =
        frames.chunks(win).map(|c| c.iter().map(|f| f[0] * f[0] + f[1] * f[1]).sum::<f32>() / c.len() as f32).collect();
    let gap = ((sample_rate * 0.08) as usize / win).max(1);
    let mut beats = Vec::new();
    let mut last: Option<usize> = None;
    for w in 1..energy.len() {
        let before = energy[w.saturating_sub(8)..w].iter().sum::<f32>() / (w - w.saturating_sub(8)) as f32;
        let onset = energy[w] > floor && energy[w] > before * jump + 1e-6;
        if onset && last.is_none_or(|l| w - l >= gap) {
            last = Some(w);
            if w * win > 0 {
                beats.push(w * win);
            }
        }
    }
    beats
}

/// Copies each slice of slot `i` into a sample of its own after it, on the
/// note the slice played on, and clears the markers.
pub(super) fn render_slices(app: &mut App, id: u8, i: usize) {
    let Some(slot) = slots(app, id).get(i).cloned() else { return };
    let Some(data) = slot.data.clone() else { return };
    let ranges = slot.slice_ranges();
    let m = app.project.module_mut(id).unwrap();
    for (k, &(a, b)) in ranges.iter().enumerate() {
        let note = slot.slice_note(k);
        let mut piece = data.with_frames(data.frames[a..b].to_vec());
        piece.name = format!("{} {:02}", slot.name, k + 1);
        let mut s = SampleSlot::new(piece, None);
        (s.base_note, s.keys, s.velocities) = (note, [note, note], slot.velocities);
        (s.volume, s.panning, s.transpose, s.finetune) = (slot.volume, slot.panning, slot.transpose, slot.finetune);
        s.mute_group = slot.mute_group;
        s.unsaved = true;
        m.samples.insert(i + 1 + k, s);
    }
    // The whole sample stays where it played, on its base note.
    let whole = &mut m.samples[i];
    whole.slices.clear();
    whole.keys = [whole.base_note, whole.base_note];
    app.mark();
    app.set_status(format!("{} slices made into samples", ranges.len()));
}

/// Applies the processes that change frames in place, other than the
/// loop crossfade, to `frames`.
pub(super) fn process(frames: &mut [Frame], op: Op) {
    match op {
        Op::Invert => {
            for f in frames.iter_mut() {
                *f = [-f[0], -f[1]];
            }
        }
        Op::RemoveDc => {
            let n = frames.len().max(1) as f32;
            let mean = frames.iter().fold([0.0; 2], |m, f| [m[0] + f[0] / n, m[1] + f[1] / n]);
            for f in frames.iter_mut() {
                *f = [f[0] - mean[0], f[1] - mean[1]];
            }
        }
        Op::MixToMono => {
            for f in frames.iter_mut() {
                let m = (f[0] + f[1]) * 0.5;
                *f = [m, m];
            }
        }
        Op::SwapChannels => {
            for f in frames.iter_mut() {
                *f = [f[1], f[0]];
            }
        }
        Op::Gain(db) => {
            let g = 10f32.powf(db / 20.0);
            for f in frames.iter_mut() {
                *f = [f[0] * g, f[1] * g];
            }
        }
        _ => {}
    }
}

/// Blends the last frames of the loop `start..end` into the ones just
/// before `start`, so playback runs on from the end into the start
/// smoothly. Returns false when there is nothing before the loop.
pub(super) fn crossfade_loop(frames: &mut [Frame], start: usize, end: usize) -> bool {
    let n = start.min((end - start) / 4);
    if n == 0 {
        return false;
    }
    for k in 0..n {
        let t = (k + 1) as f32 / n as f32;
        let (to, from) = (end - n + k, start - n + k);
        let (a, b) = (frames[to], frames[from]);
        frames[to] = [a[0] * (1.0 - t) + b[0] * t, a[1] * (1.0 - t) + b[1] * t];
    }
    true
}

/// Applies `op` to the selection (or the whole sample) of slot `i`.
pub(super) fn edit(app: &mut App, id: u8, i: usize, op: Op) {
    let Some(slot) = slots(app, id).get(i) else { return };
    let Some(data) = slot.data.clone() else { return };
    let (loop_mode, loop_start, loop_end) = (slot.loop_mode, slot.loop_start, slot.loop_end);
    let len = data.len();
    let sel = app.sampler.selection;
    let (a, b) = sel.unwrap_or((0, len));
    if op == Op::Copy || op == Op::Cut {
        if sel.is_none() {
            return;
        }
        app.sampler.clipboard = Some(data.frames[a..b].to_vec());
        // Ctrl+V only reaches the editor while the system clipboard has text.
        app.ctx.copy_text(format!("{} frames of {}", b - a, data.name));
        if op == Op::Copy {
            app.set_status(format!("Copied {} frames", b - a));
            return;
        }
    }
    let mut frames = data.frames.clone();
    // Where an old frame position ends up, for keeping the loop in place.
    let mut map: Box<dyn Fn(usize) -> usize> = Box::new(|x| x);
    let mut new_sel = sel;
    match op {
        Op::Reverse => frames[a..b].reverse(),
        Op::Normalize => {
            let peak = frames[a..b].iter().flat_map(|f| [f[0].abs(), f[1].abs()]).fold(0.0, f32::max);
            if peak > 0.0 {
                for f in &mut frames[a..b] {
                    *f = [f[0] / peak, f[1] / peak];
                }
            }
        }
        Op::FadeIn | Op::FadeOut => {
            let n = (b - a).max(1) as f32;
            for (k, f) in frames[a..b].iter_mut().enumerate() {
                let t = k as f32 / n;
                let g = if op == Op::FadeIn { t } else { 1.0 - t };
                *f = [f[0] * g, f[1] * g];
            }
        }
        Op::Silence => frames[a..b].fill([0.0; 2]),
        Op::Crop => {
            frames = frames[a..b].to_vec();
            map = Box::new(move |x| x.clamp(a, b) - a);
            new_sel = None;
        }
        Op::Delete | Op::Cut => {
            if b - a >= len {
                app.set_status("Can't delete the whole sample; delete the sample slot instead");
                return;
            }
            frames.drain(a..b);
            map = Box::new(move |x| {
                if x < a {
                    x
                } else if x >= b {
                    x - (b - a)
                } else {
                    a
                }
            });
            new_sel = None;
        }
        Op::Paste => {
            let Some(clip) = app.sampler.clipboard.clone() else { return };
            let (a, b) = sel.unwrap_or((0, 0));
            let n = clip.len();
            frames.splice(a..b, clip);
            map = Box::new(move |x| {
                if x < a {
                    x
                } else if x >= b {
                    x - (b - a) + n
                } else {
                    a
                }
            });
            new_sel = Some((a, a + n));
        }
        Op::Invert | Op::RemoveDc | Op::MixToMono | Op::SwapChannels | Op::Gain(_) => process(&mut frames[a..b], op),
        Op::CrossfadeLoop => {
            if loop_mode == 0 || !crossfade_loop(&mut frames, loop_start, loop_end.min(len)) {
                app.set_status("Crossfade Loop needs a loop with audio before its start");
                return;
            }
        }
        Op::Stretch(ratio) => {
            let stretched = crate::sample::time_stretch(&frames[a..b], ratio, data.sample_rate);
            let n = stretched.len();
            frames.splice(a..b, stretched);
            map = Box::new(move |x| {
                if x < a {
                    x
                } else if x >= b {
                    x - (b - a) + n
                } else {
                    a + ((x - a) as f64 * ratio) as usize
                }
            });
            new_sel = sel.map(|_| (a, a + n));
        }
        Op::Copy => unreachable!(),
    }
    let sample = Arc::new(data.with_frames(frames));
    let s = slot_mut(app, id, i).unwrap();
    s.loop_start = map(s.loop_start);
    s.loop_end = map(s.loop_end);
    s.slices = s.slices.iter().map(|&x| map(x)).collect();
    s.data = Some(sample);
    s.unsaved = true;
    s.clamp_loop();
    s.clamp_slices();
    app.sampler.selection = new_sel;
    app.mark();
}
