use super::*;
use crate::project::{MacroTarget, Owner, Phrase};

const SR: f32 = 8000.0;

fn engine(project: Project) -> Engine {
    Engine::new(SR, Arc::new(project), None, None, Arc::new(Shared::default()))
}

#[test]
fn previews_play_at_their_volume_and_loop_until_stopped() {
    let mut p = Project::empty();
    p.modules[0].params[0] = 1.0;
    let mut e = engine(p);
    let sample = Sample { name: "click".into(), sample_rate: SR, channels: 2, frames: vec![[1.0; 2], [0.0; 2]] };
    e.handle(Cmd::PreviewSettings { volume: 0.5, looping: true });
    e.handle(Cmd::Preview(Some(Arc::new(sample))));
    let mut out = vec![[0.0; 2]; 6];
    e.render(&mut out);
    let left: Vec<f32> = out.iter().map(|f| f[0]).collect();
    assert_eq!(left, [0.5, 0.0, 0.5, 0.0, 0.5, 0.0], "half as loud, over and over");
    e.handle(Cmd::PreviewSettings { volume: 0.5, looping: false });
    let mut out = vec![[0.0; 2]; 6];
    e.render(&mut out);
    assert_eq!(out.iter().filter(|f| f[0] != 0.0).count(), 0, "once it reaches the end, it stops");
    // A sample at a much higher rate than the output steps over its
    // whole length at once, and still loops.
    let fast = Sample { name: "fast".into(), sample_rate: SR * 5.0, channels: 2, frames: vec![[1.0; 2], [0.0; 2]] };
    e.handle(Cmd::PreviewSettings { volume: 1.0, looping: true });
    e.handle(Cmd::Preview(Some(Arc::new(fast))));
    e.render(&mut out);
}

#[test]
fn the_output_goes_onto_the_resample_tape_while_it_is_on() {
    let shared = Arc::new(Shared::default());
    let mut e = Engine::new(SR, Arc::new(Project::demo()), None, None, shared.clone());
    e.play_song();
    let mut out = vec![[0.0; 2]; 256];
    e.render(&mut out);
    let mut got = Vec::new();
    shared.resample.take(&mut got);
    assert!(got.is_empty(), "nothing before it starts");
    shared.resample.start(1024);
    for _ in 0..3 {
        e.render(&mut out);
    }
    assert!(!shared.resample.take(&mut got));
    assert_eq!(got.len(), 768);
    assert_eq!(got[512..], out[..], "the frames played, in order");
}

/// Renders `frames` frames, in pieces the size of an audio callback,
/// and returns the left channel.
fn render(e: &mut Engine, frames: usize) -> Vec<f32> {
    let mut out = vec![[0.0; 2]; frames];
    for chunk in out.chunks_mut(256) {
        e.render(chunk);
    }
    out.iter().map(|f| f[0]).collect()
}

fn loudness(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |m, v| m.max(v.abs()))
}

#[test]
fn the_master_chain_processes_the_whole_mix() {
    let mut p = Project::empty();
    p.modules[0].params[0] = 1.0;
    let synth = p.add_module(ModuleKind::Generator, [0.0; 2]).unwrap();
    p.connect(synth, OUTPUT_ID);
    let mut e = engine(p.clone());
    e.handle(Cmd::NoteOn { module: synth, key: LIVE_KEY, note: 60, vel: 1.0 });
    let dry = loudness(&render(&mut e, 4000)[2000..]);
    assert!(dry > 0.05);
    let amp = p.chain_insert(OUTPUT_ID, 0, ModuleKind::Amplifier).unwrap();
    p.module_mut(amp).unwrap().params[0] = 0.5;
    let mut e = engine(p);
    e.handle(Cmd::NoteOn { module: synth, key: LIVE_KEY, note: 60, vel: 1.0 });
    let through = loudness(&render(&mut e, 4000)[2000..]);
    assert!((through / dry - 0.5).abs() < 0.02, "half as loud through the master Amplifier: {dry} {through}");
}

#[test]
fn songs_loop_unless_f00_ends_them() {
    let mut p = Project::empty();
    p.modules[0].params[0] = 1.0;
    let synth = p.add_module(ModuleKind::Generator, [0.0; 2]).unwrap();
    p.connect(synth, OUTPUT_ID);
    p.patterns[0].lines = 4;
    p.patterns[0].tracks[0][0] = Cell { note: Some(Note::On(60)), module: Some(synth), ..Cell::default() };
    let line = 6 * TICK;
    for stop in [false, true] {
        let mut p = p.clone();
        if stop {
            assert!(p.end_with_stop());
            assert_eq!(p.patterns[0].tracks[0][3].fx, Some((0xF, 0)), "on the last line");
        }
        let mut e = engine(p);
        e.play_song();
        render(&mut e, 4 * line + 64);
        assert_eq!(e.playing, !stop, "F00 {stop}");
        assert!(e.song_ended, "a render stops after the song either way");
        if stop {
            assert_eq!((e.pos_order, e.pos_line), (0, 0), "back at the start for the next play");
            assert!(loudness(&render(&mut e, 64)) > 0.0, "the last note rings out as it is let go");
        }
    }
}

#[test]
fn a_tracks_effects_change_only_what_it_plays() {
    let mut p = Project::empty();
    p.modules[0].params[0] = 1.0;
    let synth = p.add_module(ModuleKind::Generator, [0.0; 2]).unwrap();
    p.connect(synth, OUTPUT_ID);
    // The instrument's own effect halves it, wherever it plays.
    let own = p.chain_insert(synth, 0, ModuleKind::Amplifier).unwrap();
    p.module_mut(own).unwrap().params[0] = 0.5;
    // Both tracks play it; track 1 also halves what it plays.
    let on = |m| Cell { note: Some(Note::On(60)), module: Some(m), ..Cell::default() };
    p.patterns[0].tracks[0][0] = on(synth);
    p.patterns[0].tracks[1][0] = on(synth);
    let level = |p: &Project, track: usize| {
        let mut p = p.clone();
        p.patterns[0].tracks[1 - track][0] = Cell::default();
        let mut e = engine(p);
        e.play_song();
        loudness(&render(&mut e, 4000)[2000..])
    };
    let (plain, before) = (level(&p, 0), level(&p, 1));
    assert!((plain / before - 1.0).abs() < 0.01, "the same without track effects");
    let fx = p.chain_insert(Owner::Track(1), 0, ModuleKind::Amplifier).unwrap();
    p.module_mut(fx).unwrap().params[0] = 0.5;
    assert!(!p.links.iter().any(|l| l.0 == fx || l.1 == fx), "placed, not linked");
    let (other, through) = (level(&p, 0), level(&p, 1));
    assert!((other / plain - 1.0).abs() < 0.01, "track 0 is as it was: {other} {plain}");
    assert!((through / plain - 0.5).abs() < 0.01, "track 1 goes through its effect too: {through} {plain}");
}

#[test]
fn metronome_clicks_on_beats() {
    let mut p = Project::empty();
    p.bpm = 120.0;
    p.lpb = 4;
    let mut e = engine(p);
    e.handle(Cmd::Metronome(true));
    e.play_song();
    // At 120 BPM a beat is half a second.
    let out = render(&mut e, SR as usize);
    let beat = SR as usize / 2;
    assert!(loudness(&out[..200]) > 0.2, "click on the first beat");
    assert!(loudness(&out[beat / 2..beat]) < 0.001, "silent between beats");
    assert!(loudness(&out[beat..beat + 200]) > 0.2, "click on the second beat");

    // Off, an empty song is silent.
    let mut e = engine(Project::empty());
    e.play_song();
    assert_eq!(loudness(&render(&mut e, SR as usize)), 0.0);
}

/// Two generators into the output, each holding a note.
fn two_synths() -> (Project, u8, u8) {
    let mut p = Project::empty();
    let a = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
    let b = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
    p.connect(a, OUTPUT_ID);
    p.connect(b, OUTPUT_ID);
    (p, a, b)
}

fn hold_notes(e: &mut Engine, ids: &[u8]) {
    for &id in ids {
        e.handle(Cmd::NoteOn { module: id, key: LIVE_KEY, note: 60, vel: 1.0 });
    }
}

#[test]
fn a_key_input_lets_another_sound_work_the_compressor() {
    let (mut p, a, b) = two_synths();
    p.disconnect(a, OUTPUT_ID);
    // A heavy compressor on a, keyed by b.
    let comp = p.add_module(ModuleKind::Compressor, [0.0, 0.0]).unwrap();
    p.connect(a, comp);
    p.connect(comp, OUTPUT_ID);
    p.module_mut(comp).unwrap().params = vec![0.02, 20.0, 0.0001, 0.05, 1.0, 1.0];
    // b is only heard through the compressor's key input.
    p.disconnect(b, OUTPUT_ID);
    let level = |p: &Project, notes: &[u8]| {
        let mut e = engine(p.clone());
        hold_notes(&mut e, notes);
        render(&mut e, 4000);
        loudness(&render(&mut e, 4000))
    };
    // On its own input it squashes a's note.
    let own = level(&p, &[a]);
    p.module_mut(comp).unwrap().key = Some(b);
    assert!(p.can_key(comp, b));
    // Keyed by a silent b it lets a through untouched; b playing ducks it.
    let quiet_key = level(&p, &[a]);
    let ducked = level(&p, &[a, b]);
    assert!(quiet_key > 3.0 * own, "{quiet_key} {own}");
    assert!(ducked < 0.5 * quiet_key, "{ducked} {quiet_key}");
    // It hears b before b's mixer strip: muted, b still ducks a.
    p.module_mut(b).unwrap().mute = true;
    let ghost = level(&p, &[a, b]);
    assert!(ghost < 0.5 * quiet_key, "a muted key still keys: {ghost} {quiet_key}");
    // What the compressor feeds can't key it: that would loop.
    let after = p.add_module(ModuleKind::Filter, [0.0, 0.0]).unwrap();
    p.disconnect(comp, OUTPUT_ID);
    p.connect(comp, after);
    p.connect(after, OUTPUT_ID);
    assert!(!p.can_key(comp, after));
    assert!(!p.can_key(comp, comp));
}

#[test]
fn macros_move_their_instruments_parameters() {
    let (mut p, a, b) = two_synths();
    p.disconnect(b, OUTPUT_ID);
    let level = |p: &Project| {
        let mut e = engine(p.clone());
        hold_notes(&mut e, &[a]);
        render(&mut e, 2000);
        loudness(&render(&mut e, 2000))
    };
    let full = level(&p);
    // Macro 1 turns the Generator's volume from nothing to a quarter.
    let m = p.module_mut(a).unwrap();
    m.macro_mut(0).targets.push(MacroTarget { module: a, param: 0, from: 0.0, to: 0.25 });
    assert_eq!(level(&p), 0.0, "turned down, it silences the synth");
    p.module_mut(a).unwrap().macro_mut(0).value = 1.0;
    let half = level(&p);
    assert!(half > 0.1 && half < full, "{half} {full}");
    // A Modulator can turn the macro, as it can any parameter.
    p.module_mut(a).unwrap().macro_mut(0).value = 0.0;
    let n = ModuleKind::Generator.params().len();
    assert_eq!(p.module(a).unwrap().automatable_name(n + 2), "Macro 1");
    let knob = p.add_module(ModuleKind::Modulator, [0.0, 0.0]).unwrap();
    p.module_mut(knob).unwrap().params[0] = 5.0;
    p.module_mut(knob).unwrap().params[5] = 1.0;
    p.connect(knob, a);
    p.set_control_param(knob, a, n + 2);
    assert!(level(&p) > 0.1, "the Modulator turns it up");
}

#[test]
fn mixer_gain_pan_and_meters() {
    let (mut p, a, b) = two_synths();
    p.module_mut(b).unwrap().mute = true;
    let mut e = engine(p.clone());
    hold_notes(&mut e, &[a, b]);
    render(&mut e, 2000);
    let full = e.shared.level(a);
    assert!(full[0] > 0.1 && (full[0] - full[1]).abs() < 1e-3, "{full:?}");
    assert_eq!(e.shared.level(b), [0.0, 0.0], "muted modules read silent");

    // Half the fader, panned hard right.
    p.module_mut(a).unwrap().gain = 0.5;
    p.module_mut(a).unwrap().pan = 1.0;
    e.handle(Cmd::Project(Arc::new(p)));
    render(&mut e, (SR * METER_FALL * 10.0) as usize);
    let [l, r] = e.shared.level(a);
    assert!(l < 1e-3, "left is silent: {l}");
    assert!((r - full[1] * 0.5).abs() < 0.05, "{r} vs {}", full[1]);
}

#[test]
fn solo_keeps_the_chain_audible() {
    let (mut p, a, b) = two_synths();
    // a -> filter -> output; soloing the filter keeps a and the output.
    p.disconnect(a, OUTPUT_ID);
    let f = p.add_module(ModuleKind::Filter, [0.0, 0.0]).unwrap();
    p.connect(a, f);
    p.connect(f, OUTPUT_ID);
    p.module_mut(f).unwrap().solo = true;
    let mut e = engine(p);
    hold_notes(&mut e, &[a, b]);
    render(&mut e, 2000);
    assert!(e.shared.level(a)[0] > 0.01);
    assert!(e.shared.level(f)[0] > 0.01);
    assert!(e.shared.level(OUTPUT_ID)[0] > 0.01);
    assert_eq!(e.shared.level(b), [0.0, 0.0]);
}

impl Engine {
    fn node_mut(&mut self, id: u8) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }
}

/// Records the events the sequencer sends a module.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<(&'static str, f32)>>>);

impl Log {
    fn take(&self) -> Vec<(&'static str, f32)> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

struct Recorder(Log);

impl Dsp for Recorder {
    fn note_on(&mut self, _: u32, note: f32, _: f32) {
        self.0.0.lock().unwrap().push(("on", note));
    }
    fn note_off(&mut self, _: u32) {
        self.0.0.lock().unwrap().push(("off", 0.0));
    }
    fn set_pitch(&mut self, _: u32, note: f32) {
        self.0.0.lock().unwrap().push(("pitch", note));
    }
    fn set_velocity(&mut self, _: u32, vel: f32) {
        self.0.0.lock().unwrap().push(("vel", vel));
    }
    fn set_pan(&mut self, _: u32, pan: f32) {
        self.0.0.lock().unwrap().push(("pan", pan));
    }
    fn process(&mut self, _: &Ctx, _: &[f32], _: &[Frame], out: &mut [Frame]) {
        out.fill([0.0; 2]);
    }
}

/// At 125 BPM, 4 lines per beat and 6 ticks per line.
const TICK: usize = 160;

/// Plays `cells` on track 0, sending them to a recorder.
fn sequencer(cells: &[(usize, Cell)]) -> (Engine, Log) {
    let mut p = Project::empty();
    let id = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
    p.connect(id, OUTPUT_ID);
    for &(line, cell) in cells {
        p.patterns[0].tracks[0][line] = Cell { module: Some(id), ..cell };
    }
    let mut e = engine(p);
    let log = Log::default();
    e.node_mut(id).unwrap().dsp = Box::new(Recorder(log.clone()));
    e.play_song();
    (e, log)
}

fn note(n: u8, vol: Option<u8>, fx: Option<(u8, u8)>) -> Cell {
    Cell { note: Some(Note::On(n)), module: None, vol, fx, ..Cell::default() }
}

fn effect(cmd: u8, arg: u8) -> Cell {
    Cell { fx: Some((cmd, arg)), ..Cell::default() }
}

fn values(log: &[(&str, f32)], what: &str) -> Vec<f32> {
    log.iter().filter(|e| e.0 == what).map(|e| e.1).collect()
}

#[test]
fn arpeggio_and_vibrato_end_with_their_line() {
    let (mut e, log) = sequencer(&[(0, note(48, None, Some((0x0, 0x37)))), (2, note(48, None, Some((0x4, 0x48))))]);
    render(&mut e, 7 * TICK);
    let events = log.take();
    assert_eq!(values(&events, "pitch"), [51.0, 55.0, 48.0, 51.0, 55.0, 48.0], "back to the note on the next line");
    render(&mut e, 6 * TICK + 6 * TICK);
    let pitches = values(&log.take(), "pitch");
    let widest = pitches.iter().fold(0f32, |m, p| m.max((p - 48.0).abs()));
    assert!(widest > 0.5 && widest <= 1.0, "a semitone of vibrato: {pitches:?}");
    assert_eq!(*pitches.last().unwrap(), 48.0);
}

#[test]
fn volume_slide_stays_and_tremolo_does_not() {
    let (mut e, log) = sequencer(&[(0, note(48, Some(0x40), Some((0xA, 0x40)))), (2, effect(0x7, 0x4F))]);
    render(&mut e, 12 * TICK);
    let vels = values(&log.take(), "vel");
    assert_eq!(vels.len(), 5, "one change per tick after the first: {vels:?}");
    assert!((vels[4] - (0.5 + 5.0 * 4.0 / 128.0)).abs() < 1e-6);
    // A line of tremolo dips and comes back.
    render(&mut e, 7 * TICK);
    let vels = values(&log.take(), "vel");
    let low = vels.iter().fold(1f32, |m, &v| m.min(v));
    assert!(low < 0.3, "{vels:?}");
    assert!((vels.last().unwrap() - 0.65625).abs() < 1e-6, "{vels:?}");
}

#[test]
fn panning_stays_with_the_track() {
    let (mut e, log) =
        sequencer(&[(0, note(48, None, Some((0x8, 0x00)))), (1, effect(0x8, 0xFF)), (2, note(50, None, None))]);
    render(&mut e, 13 * TICK);
    let events = log.take();
    let pans = values(&events, "pan");
    assert_eq!(pans, [-1.0, -1.0, 1.0, 1.0]);
    let second = events.iter().rposition(|e| *e == ("on", 50.0)).unwrap();
    assert_eq!(events[second + 1], ("pan", 1.0), "the next note starts on the right");

    // And the generator really plays on one side.
    let mut p = Project::empty();
    let id = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
    p.connect(id, OUTPUT_ID);
    p.patterns[0].tracks[0][0] = Cell { module: Some(id), ..note(60, None, Some((0x8, 0x00))) };
    let mut e = engine(p);
    e.play_song();
    let mut out = vec![[0.0; 2]; 2000];
    e.render(&mut out);
    let side = |ch: usize| out.iter().fold(0f32, |m, f| m.max(f[ch].abs()));
    assert!(side(0) > 0.1 && side(1) < 1e-6, "{} {}", side(0), side(1));
}

#[test]
fn retrigger_plays_the_note_again() {
    let (mut e, log) = sequencer(&[(0, note(48, None, Some((0xE, 0x02))))]);
    render(&mut e, 6 * TICK);
    let events = log.take();
    assert_eq!(values(&events, "on"), [48.0, 48.0, 48.0], "on ticks 0, 2 and 4");
    assert_eq!(values(&events, "off").len(), 2);
}

#[test]
fn slots_mute_tracks() {
    let (mut e, log) = sequencer(&[(0, note(48, None, None))]);
    let mut p = (*e.project).clone();
    let mut muted = p.order[0];
    muted.toggle_mute(0);
    p.order = vec![muted, p.order[0]];
    p.patterns[0].lines = 2;
    e.handle(Cmd::Project(Arc::new(p)));
    e.play_song();
    render(&mut e, 12 * TICK);
    assert!(values(&log.take(), "on").is_empty(), "muted in the first slot");
    render(&mut e, TICK);
    assert_eq!(values(&log.take(), "on"), [48.0]);
}

#[test]
fn position_jump_and_ticks_per_line() {
    let (mut e, _) = sequencer(&[(0, effect(0xF, 0x03)), (1, effect(0xB, 0x00))]);
    render(&mut e, TICK);
    assert_eq!(e.tpl, 3);
    // The line still lasts as long, in three longer ticks.
    render(&mut e, 5 * TICK);
    assert_eq!(e.pos_line, 1);
    render(&mut e, 6 * TICK);
    assert_eq!((e.pos_order, e.pos_line), (0, 0));
    assert!(e.song_ended, "jumping back ends the song for rendering");
}

#[test]
fn a_break_goes_on_at_a_line_of_the_next_slot() {
    let mut p = Project::empty();
    let id = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
    p.connect(id, OUTPUT_ID);
    p.order.push(crate::project::Slot::new(0));
    p.patterns[0].tracks[0][0] = effect(FX_BREAK, 0x05);
    let mut e = engine(p);
    e.play_song();
    render(&mut e, 6 * TICK + 10);
    assert_eq!(e.shown, (1, 5));
    assert!(!e.song_ended);
    // The rest of that slot plays, then the song starts over.
    render(&mut e, (64 - 5) * 6 * TICK);
    assert_eq!(e.shown, (0, 0));
    assert!(e.song_ended);
}

#[test]
fn a_wait_holds_the_line_while_its_effects_go_on() {
    let (mut e, log) = sequencer(&[(0, note(48, None, Some((FX_WAIT, 0x02)))), (1, note(50, None, None))]);
    render(&mut e, 6 * TICK + 10);
    assert_eq!(e.shown, (0, 0));
    render(&mut e, 12 * TICK);
    assert_eq!(e.shown, (0, 1), "three lines' time in all");
    render(&mut e, 6 * TICK);
    assert_eq!(e.shown, (0, 2), "the next line is as long as ever");
    assert_eq!(values(&log.take(), "on"), [48.0, 50.0]);
}

#[test]
fn volume_column_commands_act_as_effects() {
    let vc = |c, x| crate::project::vol_command_value(c, x);
    // A fade out starts at full volume, not at the command's value.
    let (mut e, log) = sequencer(&[(0, note(48, vc('O', 4), None)), (1, Cell::default())]);
    render(&mut e, 6 * TICK + 10);
    let vels = values(&log.take(), "vel");
    assert_eq!(vels.len(), 5);
    assert!((vels[0] - (1.0 - 4.0 / 128.0)).abs() < 1e-6 && vels.windows(2).all(|w| w[1] < w[0]), "{vels:?}");
    // G4 glides a semitone a tick.
    let (mut e, log) = sequencer(&[(0, note(48, None, None)), (1, note(52, vc('G', 4), None))]);
    render(&mut e, 12 * TICK + 10);
    assert_eq!(values(&log.take(), "pitch"), [49.0, 50.0, 51.0, 52.0]);
}

#[test]
fn track_volume_scales_the_track_s_notes() {
    let (mut e, log) = sequencer(&[
        (0, note(48, Some(0x40), None)),
        (1, effect(FX_TRACK_VOLUME, 0x40)),
        (2, Cell { vol: Some(0x80), ..Cell::default() }),
    ]);
    render(&mut e, 18 * TICK + 10);
    // Half the track's volume halves the note playing, and a new
    // volume on the line after.
    assert_eq!(values(&log.take(), "vel"), [0.25, 0.5]);
}

#[test]
fn tremor_switches_the_note_on_and_off_by_ticks() {
    let (mut e, log) = sequencer(&[(0, note(48, None, Some((FX_TREMOR, 0x21)))), (1, Cell::default())]);
    render(&mut e, 6 * TICK + 10);
    // On for ticks 0 and 1, off for 2, on for 3 and 4, off for 5, then
    // back on as the line ends.
    assert_eq!(values(&log.take(), "vel"), [0.0, 1.0, 0.0, 1.0]);
}

/// Keeps the last value of parameter 1 that a module processed with.
struct Probe(Arc<Mutex<f32>>);

impl Dsp for Probe {
    fn process(&mut self, _: &Ctx, params: &[f32], _: &[Frame], out: &mut [Frame]) {
        *self.0.lock().unwrap() = params[1];
        out.fill([0.0; 2]);
    }
}

#[test]
fn envelopes_move_parameters_while_playing() {
    let mut p = Project::empty();
    let f = p.add_module(ModuleKind::Filter, [0.0, 0.0]).unwrap();
    p.connect(f, OUTPUT_ID);
    // Cutoff from 20 Hz at line 0 to 20 kHz at line 4.
    let points = vec![(0.0, 0.0), (4.0, 1.0)];
    p.patterns[0].automation.push(crate::project::Envelope::new(f, 1, points));
    let mut e = engine(p);
    let cutoff = Arc::new(Mutex::new(0.0));
    e.node_mut(f).unwrap().dsp = Box::new(Probe(cutoff.clone()));
    let at = |e: &mut Engine, frames: usize| {
        render(e, frames);
        *cutoff.lock().unwrap()
    };
    assert_eq!(at(&mut e, 64), 2000.0, "not playing: the song's value");
    e.play_song();
    assert!(at(&mut e, 64) < 21.0);
    // Two lines in, halfway, is the middle of the slider: 632 Hz.
    render(&mut e, 12 * TICK - 64);
    let mid = at(&mut e, 16);
    assert!((mid - 632.5).abs() < 0.5, "{mid}");
    assert!(at(&mut e, 12 * TICK + 64) > 19900.0);
    e.handle(Cmd::Stop);
    assert_eq!(at(&mut e, 64), 2000.0);
}

#[test]
fn envelopes_move_the_mixer_and_are_published() {
    let mut p = Project::empty();
    let id = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
    p.connect(id, OUTPUT_ID);
    p.patterns[0].tracks[0][0] = Cell { module: Some(id), ..note(60, None, None) };
    // The fader, the first parameter after the module's own, held at 0.
    let fader = ModuleKind::Generator.params().len();
    p.patterns[0].automation.push(crate::project::Envelope::new(id, fader, vec![(0.0, 0.0)]));
    let shared = Arc::new(Shared::default());
    let mut e = Engine::new(SR, Arc::new(p), None, None, shared.clone());
    e.play_song();
    let out = render(&mut e, 2000);
    assert!(loudness(&out) < 1e-6, "the fader is down");
    assert_eq!(*shared.automated.lock().unwrap(), [(id, fader, 0.0)]);
    e.handle(Cmd::Stop);
    render(&mut e, 64);
    assert!(shared.automated.lock().unwrap().is_empty(), "nothing while stopped");
}

/// A MultiSynth feeding two recorders, with `params` set.
fn multisynth(params: &[(usize, f32)]) -> (Engine, u8, [Log; 2]) {
    let mut p = Project::empty();
    let ms = p.add_module(ModuleKind::MultiSynth, [0.0, 0.0]).unwrap();
    let a = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
    let b = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
    assert!(!p.connect(ms, OUTPUT_ID), "notes don't go to the output");
    assert!(p.connect(ms, a) && p.connect(ms, b));
    assert!(!p.connect(a, ms), "nor sound into a MultiSynth");
    for &(i, v) in params {
        p.module_mut(ms).unwrap().params[i] = v;
    }
    let mut e = engine(p);
    let logs = [Log::default(), Log::default()];
    e.node_mut(a).unwrap().dsp = Box::new(Recorder(logs[0].clone()));
    e.node_mut(b).unwrap().dsp = Box::new(Recorder(logs[1].clone()));
    (e, ms, logs)
}

/// A Glide in `mode` taking 0.1 s, feeding a recorder.
fn glide(mode: f32) -> (Engine, u8, Log) {
    let mut p = Project::empty();
    let g = p.add_module(ModuleKind::Glide, [0.0, 0.0]).unwrap();
    let synth = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
    assert!(p.connect(g, synth) && !p.connect(synth, g));
    p.module_mut(g).unwrap().params = vec![mode, 0.1];
    let mut e = engine(p);
    let log = Log::default();
    e.node_mut(synth).unwrap().dsp = Box::new(Recorder(log.clone()));
    (e, g, log)
}

#[test]
fn glide_slides_each_note_from_the_last_in_its_time() {
    let (mut e, g, log) = glide(0.0);
    e.handle(Cmd::NoteOn { module: g, key: 1, note: 48, vel: 1.0 });
    render(&mut e, 1000);
    e.handle(Cmd::NoteOff { module: g, key: 1 });
    e.handle(Cmd::NoteOn { module: g, key: 1, note: 60, vel: 1.0 });
    render(&mut e, (SR * 0.2) as usize);
    let events = log.take();
    assert_eq!(values(&events, "on"), [48.0, 48.0], "the second note starts where the first was");
    let pitches = values(&events, "pitch");
    assert!(pitches.windows(2).all(|w| w[1] > w[0]) && pitches.last() == Some(&60.0), "{pitches:?}");
    // In steps of at most GLIDE_STEP frames over the 0.1 s.
    let steps = (SR * 0.1) as usize / GLIDE_STEP;
    assert!(pitches.len() >= steps && pitches.len() <= steps + 2, "{}", pitches.len());
}

#[test]
fn legato_glides_only_over_a_held_note_without_starting_again() {
    let (mut e, g, log) = glide(1.0);
    e.handle(Cmd::NoteOn { module: g, key: 1, note: 48, vel: 1.0 });
    render(&mut e, 1000);
    // Let go and played again at once, as a track does: one note on.
    e.handle(Cmd::NoteOff { module: g, key: 1 });
    e.handle(Cmd::NoteOn { module: g, key: 1, note: 55, vel: 0.5 });
    render(&mut e, (SR * 0.2) as usize);
    let events = log.take();
    assert_eq!(values(&events, "on"), [48.0]);
    assert!(values(&events, "off").is_empty() && values(&events, "pitch").last() == Some(&55.0));
    // Let go for good, then a note after the gap jumps.
    e.handle(Cmd::NoteOff { module: g, key: 1 });
    render(&mut e, 1000);
    e.handle(Cmd::NoteOn { module: g, key: 1, note: 40, vel: 1.0 });
    render(&mut e, 1000);
    let events = log.take();
    assert_eq!((values(&events, "off").len(), values(&events, "on")), (1, vec![40.0]));
    assert!(values(&events, "pitch").is_empty());
}

#[test]
fn multisynth_passes_notes_to_all_its_instruments() {
    let (mut e, ms, logs) = multisynth(&[(1, 12.0)]);
    e.handle(Cmd::NoteOn { module: ms, key: 5, note: 48, vel: 1.0 });
    e.handle(Cmd::NoteOff { module: ms, key: 5 });
    for log in &logs {
        assert_eq!(log.take(), [("on", 60.0), ("off", 0.0)]);
    }
}

#[test]
fn multisynth_round_robin_and_range() {
    // Round robin, notes C-4 to C-5 only.
    let (mut e, ms, logs) = multisynth(&[(0, 1.0), (6, 48.0), (7, 60.0)]);
    for (key, note) in [(1, 48), (2, 50), (3, 72), (4, 52)] {
        e.handle(Cmd::NoteOn { module: ms, key, note, vel: 1.0 });
    }
    e.handle(Cmd::NoteOff { module: ms, key: 2 });
    assert_eq!(logs[0].take(), [("on", 48.0), ("on", 52.0)]);
    assert_eq!(logs[1].take(), [("on", 50.0), ("off", 0.0)], "the note off goes where its note went");
}

#[test]
fn multisynth_in_the_pattern() {
    let (mut e, ms, logs) = multisynth(&[(1, -12.0)]);
    let mut p = (*e.project).clone();
    p.patterns[0].tracks[0][0] = Cell { module: Some(ms), ..note(60, None, Some((0x1, 0x10))) };
    e.handle(Cmd::Project(Arc::new(p)));
    e.play_song();
    render(&mut e, 3 * TICK);
    let events = logs[1].take();
    assert_eq!(events[0], ("on", 48.0));
    assert_eq!(values(&events, "pitch"), [49.0, 50.0], "slides keep the transpose");
}

#[test]
fn note_columns_play_together() {
    let (mut e, log) = sequencer(&[(0, note(48, None, None))]);
    let mut p = (*e.project).clone();
    let id = p.patterns[0].tracks[0][0].module;
    p.set_columns(0, 2);
    *p.patterns[0].cell_mut(0, 1, 0) = Cell { module: id, ..note(52, None, None) };
    *p.patterns[0].cell_mut(0, 1, 1) = Cell { note: Some(Note::Off), ..Cell::default() };
    e.handle(Cmd::Project(Arc::new(p.clone())));
    e.play_song();
    render(&mut e, 12 * TICK);
    let events = log.take();
    assert_eq!(values(&events, "on"), [48.0, 52.0]);
    assert_eq!(values(&events, "off").len(), 1, "only the second column's note ends");

    // A hidden column doesn't play.
    p.set_columns(0, 1);
    e.handle(Cmd::Project(Arc::new(p)));
    e.play_song();
    render(&mut e, 2 * TICK);
    assert_eq!(values(&log.take(), "on"), [48.0]);
}

#[test]
fn panning_and_delay_columns() {
    // Pan column 00 is hard left; delay 80 is half of a 6-tick line.
    let (mut e, log) = sequencer(&[
        (0, Cell { pan: Some(0x00), ..note(48, None, None) }),
        (1, Cell { delay: Some(0x80), ..note(50, None, None) }),
    ]);
    render(&mut e, TICK);
    let events = log.take();
    assert_eq!(events[..2], [("on", 48.0), ("pan", -1.0)]);
    assert!(values(&events, "pan").iter().all(|&p| p == -1.0));
    render(&mut e, 6 * TICK);
    let events = log.take();
    assert!(values(&events, "on").is_empty(), "not yet: {events:?}");
    render(&mut e, 3 * TICK);
    assert_eq!(values(&log.take(), "on"), [50.0], "three ticks in");
}

/// A Filter whose cutoff a Modulator moves, with `params` set on the
/// Modulator, and a probe on the cutoff.
fn modulated(params: &[(usize, f32)]) -> (Engine, Arc<Mutex<f32>>, u8, Project) {
    let mut p = Project::empty();
    let f = p.add_module(ModuleKind::Filter, [0.0, 0.0]).unwrap();
    let m = p.add_module(ModuleKind::Modulator, [0.0, 0.0]).unwrap();
    p.connect(f, OUTPUT_ID);
    assert!(p.connect(m, f), "a Modulator links to any module");
    assert!(!p.connect(f, m), "but not in a loop");
    p.set_control_param(m, f, 1);
    for &(i, v) in params {
        p.module_mut(m).unwrap().params[i] = v;
    }
    let e = engine(p.clone());
    let cutoff = Arc::new(Mutex::new(0.0));
    (e, cutoff, f, p)
}

#[test]
fn modulators_move_parameters() {
    // A square LFO over 4 lines, at full amount: the cutoff jumps half
    // the slider up and down from 2 kHz.
    let (mut e, cutoff, f, _) = modulated(&[(1, 2.0), (3, 1.0), (4, 4.0), (5, 1.0)]);
    e.node_mut(f).unwrap().dsp = Box::new(Probe(cutoff.clone()));
    let spec = &ModuleKind::Filter.params()[1];
    let (up, down) = (spec.value_at(spec.position(2000.0) + 0.5), spec.value_at(spec.position(2000.0) - 0.5));
    render(&mut e, 64);
    assert_eq!(*cutoff.lock().unwrap(), up);
    render(&mut e, 12 * TICK);
    assert!((*cutoff.lock().unwrap() - down).abs() < 1e-3, "{} {down}", *cutoff.lock().unwrap());
    assert_eq!(e.shared.automated.lock().unwrap().len(), 1, "its value is published");
}

#[test]
fn modulators_follow_their_input() {
    let (mut e, cutoff, f, mut p) = modulated(&[(0, 1.0), (5, 1.0)]);
    let synth = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
    let m = p.modules.iter().find(|m| m.kind == ModuleKind::Modulator).unwrap().id;
    assert!(p.connect(synth, m));
    e.handle(Cmd::Project(Arc::new(p)));
    e.node_mut(f).unwrap().dsp = Box::new(Probe(cutoff.clone()));
    render(&mut e, 256);
    assert!((*cutoff.lock().unwrap() - 2000.0).abs() < 0.01, "silence leaves it alone");
    e.handle(Cmd::NoteOn { module: synth, key: LIVE_KEY, note: 60, vel: 1.0 });
    render(&mut e, 2000);
    assert!(*cutoff.lock().unwrap() > 2500.0, "{}", *cutoff.lock().unwrap());
}

#[test]
fn modulators_track_the_key_and_velocity_played() {
    // Key mode, at full amount: C-8 is four octaves over C-4 and takes
    // the cutoff half the slider up, C-0 half down.
    let spec = &ModuleKind::Filter.params()[1];
    let at = |pos: f32| spec.value_at(spec.position(2000.0) + pos);
    for (mode, note, vel, expect) in [(2.0, 96, 1.0, at(1.0)), (2.0, 0, 1.0, at(-1.0)), (3.0, 60, 0.5, at(0.5))] {
        let (mut e, cutoff, f, mut p) = modulated(&[(0, mode), (5, 1.0), (6, 0.001), (7, 0.001)]);
        let synth = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
        let m = p.modules.iter().find(|m| m.kind == ModuleKind::Modulator).unwrap().id;
        assert!(p.connect(synth, m));
        e.handle(Cmd::Project(Arc::new(p)));
        e.node_mut(f).unwrap().dsp = Box::new(Probe(cutoff.clone()));
        render(&mut e, 256);
        assert!((*cutoff.lock().unwrap() - 2000.0).abs() < 0.01, "before a note it is left alone");
        e.handle(Cmd::NoteOn { module: synth, key: LIVE_KEY, note, vel });
        render(&mut e, 1000);
        let got = *cutoff.lock().unwrap();
        assert!((got - expect).abs() / expect < 0.01, "mode {mode}, note {note}: {got} for {expect}");
    }
}

#[test]
fn modulators_run_an_envelope_per_note_or_follow_a_knob() {
    // Envelope: 0.1 s up, 0.1 s down, at full amount.
    let (mut e, cutoff, f, mut p) = modulated(&[(0, 4.0), (5, 1.0), (6, 0.1), (7, 0.1)]);
    let synth = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
    let m = p.modules.iter().find(|m| m.kind == ModuleKind::Modulator).unwrap().id;
    assert!(p.connect(synth, m));
    e.handle(Cmd::Project(Arc::new(p)));
    e.node_mut(f).unwrap().dsp = Box::new(Probe(cutoff.clone()));
    let spec = &ModuleKind::Filter.params()[1];
    let top = spec.value_at(spec.position(2000.0) + 1.0);
    render(&mut e, 256);
    assert!((*cutoff.lock().unwrap() - 2000.0).abs() < 0.01, "before a note it is left alone");
    e.handle(Cmd::NoteOn { module: synth, key: LIVE_KEY, note: 60, vel: 1.0 });
    render(&mut e, (SR * 0.1) as usize);
    assert!((*cutoff.lock().unwrap() / top - 1.0).abs() < 0.05, "at the top after the attack");
    render(&mut e, (SR * 0.15) as usize);
    assert!((*cutoff.lock().unwrap() - 2000.0).abs() < 1.0, "and back down after the release");
    // Manual: the amount, as a knob.
    let (mut e, cutoff, f, _) = modulated(&[(0, 5.0), (5, -0.5), (6, 0.001)]);
    e.node_mut(f).unwrap().dsp = Box::new(Probe(cutoff.clone()));
    render(&mut e, 512);
    let want = spec.value_at(spec.position(2000.0) - 0.5);
    assert!((*cutoff.lock().unwrap() / want - 1.0).abs() < 0.01, "{} {want}", *cutoff.lock().unwrap());
}

#[test]
fn drawn_lfo_shapes_move_parameters_on_the_beat() {
    // A drawn shape at the top for the first half of each cycle and at
    // the bottom for the second, a beat long, at full amount.
    let (mut e, cutoff, f, mut p) = modulated(&[(1, crate::project::DRAWN_SHAPE as f32), (3, 2.0), (5, 1.0), (8, 4.0)]);
    let m = p.modules.iter().find(|m| m.kind == ModuleKind::Modulator).unwrap().id;
    p.module_mut(m).unwrap().shape = vec![(0.0, 1.0), (0.49, 1.0), (0.51, 0.0), (1.0, 0.0)];
    e.handle(Cmd::Project(Arc::new(p.clone())));
    e.node_mut(f).unwrap().dsp = Box::new(Probe(cutoff.clone()));
    let spec = &ModuleKind::Filter.params()[1];
    let (up, down) = (spec.value_at(spec.position(2000.0) + 0.5), spec.value_at(spec.position(2000.0) - 0.5));
    render(&mut e, 64);
    assert!((*cutoff.lock().unwrap() / up - 1.0).abs() < 0.01, "up at the start of the cycle");
    // A beat is lpb lines; past half of it, the shape is down.
    let beat = p.lpb as usize * 6 * TICK;
    render(&mut e, beat * 3 / 4);
    assert!(
        (*cutoff.lock().unwrap() / down - 1.0).abs() < 0.01,
        "down in its second half: {}",
        *cutoff.lock().unwrap()
    );
}

#[test]
fn synced_modulators_keep_to_the_song_wherever_it_starts() {
    // The drawn gate above, a beat long: up for the first half of each
    // beat, down for the second, at the same lines of the song whether
    // it plays from the top or from its second line, and however long
    // the program ran before.
    let line = 6 * TICK;
    for start in [0, 1] {
        let (mut e, cutoff, f, mut p) =
            modulated(&[(1, crate::project::DRAWN_SHAPE as f32), (3, 2.0), (5, 1.0), (8, 4.0)]);
        let m = p.modules.iter().find(|m| m.kind == ModuleKind::Modulator).unwrap().id;
        p.module_mut(m).unwrap().shape = vec![(0.0, 1.0), (0.49, 1.0), (0.51, 0.0), (1.0, 0.0)];
        e.handle(Cmd::Project(Arc::new(p.clone())));
        e.node_mut(f).unwrap().dsp = Box::new(Probe(cutoff.clone()));
        let spec = &ModuleKind::Filter.params()[1];
        let (up, down) = (spec.value_at(spec.position(2000.0) + 0.5), spec.value_at(spec.position(2000.0) - 0.5));
        render(&mut e, 5 * TICK + 70);
        e.handle(Cmd::Play { order: 0, line: start, loop_pattern: false });
        // To the middle of the song's second line, then of its fourth.
        render(&mut e, (1 - start) * line + line / 2);
        assert!((*cutoff.lock().unwrap() / up - 1.0).abs() < 0.01, "from line {start}: up in line 1");
        render(&mut e, 2 * line);
        assert!((*cutoff.lock().unwrap() / down - 1.0).abs() < 0.01, "from line {start}: down in line 3");
    }
}

#[test]
fn block_loop_repeats_its_lines() {
    let (mut e, log) = sequencer(&[(1, note(48, None, None)), (3, note(50, None, None))]);
    e.handle(Cmd::BlockLoop(Some((0, 1, 2))));
    render(&mut e, 6 * 6 * TICK);
    let events = log.take();
    assert_eq!(values(&events, "on"), [48.0, 48.0, 48.0], "line 3 never comes");
    assert!((1..=2).contains(&e.pos_line), "{}", e.pos_line);
    e.handle(Cmd::BlockLoop(None));
    render(&mut e, 3 * 6 * TICK);
    assert_eq!(values(&log.take(), "on"), [50.0], "off again, it plays on");
}

#[test]
fn track_scopes_show_each_tracks_notes() {
    let mut p = Project::empty();
    let id = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
    p.connect(id, OUTPUT_ID);
    p.patterns[0].tracks[2][0] = Cell { module: Some(id), ..note(60, None, None) };
    let shared = Arc::new(Shared::default());
    let mut e = Engine::new(SR, Arc::new(p), None, None, shared.clone());
    e.play_song();
    render(&mut e, 1000);
    let scopes = shared.track_scopes.lock().unwrap();
    let track = |t: usize| loudness(&scopes[t * TRACK_SCOPE_LEN..(t + 1) * TRACK_SCOPE_LEN]);
    assert!(track(2) > 0.05, "{}", track(2));
    assert_eq!(track(0), 0.0);
    drop(scopes);
    // Live notes belong to no track.
    e.handle(Cmd::Stop);
    render(&mut e, 2000);
    e.handle(Cmd::NoteOn { module: id, key: LIVE_KEY, note: 60, vel: 1.0 });
    render(&mut e, 1000);
    let scopes = shared.track_scopes.lock().unwrap();
    assert!(scopes.iter().all(|x| x.abs() < 1e-3), "only the release of the old note");
}

#[test]
fn groove_delays_odd_lines() {
    let (mut e, log) = sequencer(&[(1, note(48, None, None)), (2, note(50, None, None))]);
    let mut p = (*e.project).clone();
    p.groove = 1.0;
    e.handle(Cmd::Project(Arc::new(p)));
    e.play_song();
    let line = 6 * TICK;
    render(&mut e, line + line / 2 - 32);
    assert!(values(&log.take(), "on").is_empty(), "line 1 waits half a line");
    render(&mut e, 64);
    assert_eq!(values(&log.take(), "on"), [48.0]);
    render(&mut e, line / 2 - 64);
    assert!(values(&log.take(), "on").is_empty());
    render(&mut e, 64);
    assert_eq!(values(&log.take(), "on"), [50.0], "line 2 is on time");
}

#[test]
fn bypassed_effects_let_sound_through() {
    let mut p = Project::empty();
    let synth = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
    let amp = p.add_module(ModuleKind::Amplifier, [0.0, 0.0]).unwrap();
    p.module_mut(amp).unwrap().params[0] = 0.0;
    p.connect(synth, amp);
    p.connect(amp, OUTPUT_ID);
    let mut e = engine(p.clone());
    e.handle(Cmd::NoteOn { module: synth, key: LIVE_KEY, note: 60, vel: 1.0 });
    assert!(loudness(&render(&mut e, 500)) < 1e-6, "the amplifier is down");
    p.module_mut(amp).unwrap().bypass = true;
    e.handle(Cmd::Project(Arc::new(p)));
    assert!(loudness(&render(&mut e, 500)) > 0.1, "bypassed, it lets the synth through");
}

#[test]
fn phrases_play_transposed_by_the_note() {
    let mut p = Project::empty();
    let id = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
    p.connect(id, OUTPUT_ID);
    let mut phrase = Phrase { lines: 4, ..Phrase::default() };
    phrase.cells[0].note = Some(Note::On(48));
    phrase.cells[2] = Cell { note: Some(Note::On(52)), vol: Some(0x40), ..Cell::default() };
    phrase.cells[3].note = Some(Note::Off);
    let m = p.module_mut(id).unwrap();
    m.phrases.push(phrase);
    m.phrase_mode = PhraseMode::Program;
    let mut e = engine(p.clone());
    let log = Log::default();
    e.node_mut(id).unwrap().dsp = Box::new(Recorder(log.clone()));
    // C-5 moves the phrase up an octave.
    e.handle(Cmd::NoteOn { module: id, key: LIVE_KEY, note: 60, vel: 1.0 });
    assert_eq!(log.take(), [("on", 60.0)], "the first line at once");
    let published = |e: &mut Engine| {
        e.render(&mut []);
        e.shared.phrases.lock().unwrap().clone()
    };
    assert_eq!(published(&mut e), [(id, 0, 0)]);
    let line = 6 * TICK;
    render(&mut e, line);
    assert!(log.take().is_empty());
    render(&mut e, line + 64);
    assert_eq!(log.take(), [("off", 0.0), ("on", 64.0)]);
    assert_eq!(published(&mut e), [(id, 0, 2)], "on its third line");
    render(&mut e, line);
    assert_eq!(log.take(), [("off", 0.0)]);
    render(&mut e, 4 * line);
    assert!(log.take().is_empty(), "it doesn't loop");

    // Looping, it starts again; the key's note-off stops it.
    p.module_mut(id).unwrap().phrases[0].looping = true;
    e.handle(Cmd::Project(Arc::new(p)));
    e.handle(Cmd::NoteOn { module: id, key: LIVE_KEY, note: 48, vel: 1.0 });
    render(&mut e, 4 * line + 64);
    assert_eq!(values(&log.take(), "on"), [48.0, 52.0, 48.0]);
    e.handle(Cmd::NoteOff { module: id, key: LIVE_KEY });
    log.take();
    render(&mut e, 8 * line);
    assert!(values(&log.take(), "on").is_empty(), "stopped");
}

/// A synth with two phrases starting on C-4 and E-4, recorded.
fn two_phrases(mode: PhraseMode) -> (Project, u8) {
    let mut p = Project::empty();
    let id = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
    p.connect(id, OUTPUT_ID);
    let m = p.module_mut(id).unwrap();
    for (n, keys) in [(48, [0, 59]), (52, [60, 71])] {
        let mut phrase = Phrase { keys, ..Phrase::default() };
        phrase.cells[0].note = Some(Note::On(n));
        m.phrases.push(phrase);
    }
    m.phrase_mode = mode;
    m.selected_phrase = 1;
    (p, id)
}

#[test]
fn notes_pick_phrases_by_mode_and_zxx() {
    let first_notes = |p: Project, id: u8, notes: &[u8]| {
        let mut e = engine(p);
        let log = Log::default();
        e.node_mut(id).unwrap().dsp = Box::new(Recorder(log.clone()));
        for (k, &n) in notes.iter().enumerate() {
            e.handle(Cmd::NoteOn { module: id, key: k as u32, note: n, vel: 1.0 });
        }
        values(&log.take(), "on")
    };
    // The key map: C-4 plays the first phrase, C-5 the second, moved
    // up an octave, and C-6, outside both, the synth itself.
    let (p, id) = two_phrases(PhraseMode::Keymap);
    assert_eq!(first_notes(p, id, &[48, 60, 72]), [48.0, 64.0, 72.0]);
    let (p, id) = two_phrases(PhraseMode::Program);
    assert_eq!(first_notes(p, id, &[48, 72]), [52.0, 76.0], "the selected phrase");
    let (p, id) = two_phrases(PhraseMode::Off);
    assert_eq!(first_notes(p, id, &[48]), [48.0]);

    // Zxx picks a phrase whatever the mode, Z00 none.
    let (mut p, id) = two_phrases(PhraseMode::Off);
    for (line, z) in [(0, 1), (4, 2), (8, 0)] {
        p.patterns[0].tracks[0][line] =
            Cell { note: Some(Note::On(48)), module: Some(id), fx: Some((FX_PHRASE, z)), ..Cell::default() };
    }
    p.module_mut(id).unwrap().phrase_mode = PhraseMode::Program;
    let mut e = engine(p);
    let log = Log::default();
    e.node_mut(id).unwrap().dsp = Box::new(Recorder(log.clone()));
    e.play_song();
    render(&mut e, 9 * 6 * TICK + 10);
    assert_eq!(values(&log.take(), "on"), [48.0, 52.0, 48.0]);
}

#[test]
fn phrases_play_effect_commands() {
    let mut p = Project::empty();
    let id = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
    p.connect(id, OUTPUT_ID);
    let mut phrase = Phrase { lines: 4, ..Phrase::default() };
    // A slide up a semitone a tick, a cut two ticks into the next
    // line, and a note delayed by three ticks after that.
    phrase.cells[0] = Cell { note: Some(Note::On(48)), fx: Some((0x1, 0x10)), ..Cell::default() };
    phrase.cells[1].fx = Some((0xC, 2));
    phrase.cells[2] = Cell { note: Some(Note::On(55)), fx: Some((0xD, 3)), ..Cell::default() };
    let m = p.module_mut(id).unwrap();
    m.phrases.push(phrase);
    m.phrase_mode = PhraseMode::Program;
    let mut e = engine(p);
    let log = Log::default();
    e.node_mut(id).unwrap().dsp = Box::new(Recorder(log.clone()));
    // Transposed by the note played, C-5.
    e.handle(Cmd::NoteOn { module: id, key: LIVE_KEY, note: 60, vel: 1.0 });
    render(&mut e, 6 * TICK + 2 * TICK + 10);
    let events = log.take();
    assert_eq!(values(&events, "pitch"), [61.0, 62.0, 63.0, 64.0, 65.0]);
    assert_eq!(events.last(), Some(&("off", 0.0)), "cut");
    render(&mut e, 4 * TICK + 2 * TICK);
    assert!(log.take().is_empty(), "delayed");
    render(&mut e, TICK);
    assert_eq!(values(&log.take(), "on"), [67.0]);
}

#[test]
fn yxx_plays_notes_by_chance() {
    let played = |chance: u8| {
        let cells: Vec<(usize, Cell)> = (0..64).map(|l| (l, note(48, None, Some((FX_MAYBE, chance))))).collect();
        let (mut e, log) = sequencer(&cells);
        render(&mut e, 64 * 6 * TICK);
        values(&log.take(), "on").len()
    };
    assert_eq!(played(0xFF), 64);
    assert_eq!(played(0x00), 0);
    let half = played(0x80);
    assert!((16..=48).contains(&half), "about half: {half}");
}

#[test]
fn auto_pan_swings_the_note_for_its_line() {
    let (mut e, log) = sequencer(&[(0, note(48, None, Some((FX_AUTOPAN, 0x8F)))), (1, Cell::default())]);
    render(&mut e, 6 * TICK + TICK / 2);
    let pans = values(&log.take(), "pan");
    // A quarter cycle a tick at full depth: right, centre, left, ...
    assert_eq!(pans.len(), 6, "ticks 1 to 5, then the next line: {pans:?}");
    assert!((pans[0] - 1.0).abs() < 1e-4 && (pans[2] + 1.0).abs() < 1e-4, "{pans:?}");
    assert_eq!(pans.last(), Some(&0.0), "back in the middle on the next line");
}

#[test]
fn autoseek_plays_a_sample_started_before_playback_from_where_it_would_be() {
    let level = |autoseek: bool, line: usize| {
        let mut p = Project::empty();
        p.modules[0].params[0] = 1.0;
        let id = p.add_module(ModuleKind::Sampler, [0.0, 0.0]).unwrap();
        p.connect(id, OUTPUT_ID);
        // A rising ramp four seconds long, so its level tells the position.
        let frames = (0..SR as usize * 4).map(|i| [i as f32 / (SR * 4.0); 2]).collect();
        let sample = Sample { name: "ramp".into(), sample_rate: SR, channels: 1, frames };
        let mut slot = crate::project::SampleSlot::new(sample, None);
        (slot.base_note, slot.autoseek) = (48, autoseek);
        p.module_mut(id).unwrap().samples.push(slot);
        *p.patterns[0].cell_mut(0, 0, 0) = Cell { note: Some(Note::On(48)), module: Some(id), ..Cell::default() };
        let mut e = engine(p);
        e.handle(Cmd::Play { order: 0, line, loop_pattern: false });
        let out = render(&mut e, 600);
        out[500..].iter().sum::<f32>() / 100.0
    };
    assert!(level(false, 8).abs() < 1e-4, "without autoseek it waits for its next note");
    let (early, late) = (level(true, 8), level(true, 16));
    assert!(early > 0.0, "it plays at once");
    // Twice as far in, nearly twice as high up the ramp.
    assert!((1.8..2.1).contains(&(late / early)), "{early} {late}");
    assert_eq!(level(true, 0), level(false, 0), "from the note itself, nothing changes");
}

#[test]
fn effect_columns_act_on_every_note_column() {
    let mut p = Project::empty();
    let id = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
    p.connect(id, OUTPUT_ID);
    let pat = &mut p.patterns[0];
    pat.set_columns(0, 2);
    pat.set_fx_columns(0, 2);
    *pat.cell_mut(0, 0, 0) = Cell { note: Some(Note::On(48)), module: Some(id), ..Cell::default() };
    *pat.cell_mut(0, 1, 0) = Cell { note: Some(Note::On(55)), module: Some(id), ..Cell::default() };
    // The track's first effect column slides both up, its second sets the tempo.
    pat.cell_mut(0, 2, 0).fx = Some((0x1, 0x10));
    pat.cell_mut(0, 3, 0).fx = Some((0xF, 0x60));
    let mut e = engine(p);
    let log = Log::default();
    e.node_mut(id).unwrap().dsp = Box::new(Recorder(log.clone()));
    e.play_song();
    render(&mut e, 2 * TICK + 10);
    let pitches = values(&log.take(), "pitch");
    assert!(pitches.contains(&49.0) && pitches.contains(&56.0), "{pitches:?}");
    assert_eq!(e.bpm, 96.0);
}
