use super::*;

#[test]
fn volume_column_commands_read_back_as_written() {
    for v in 0..=0x80 {
        assert_eq!(parse_vol(&vol_text(v)), Some(v));
    }
    let fade = vol_command_value('O', 0xA).unwrap();
    assert_eq!((vol_text(fade), vol_command(fade)), ("OA".into(), Some(('O', 0xA))));
    assert_eq!(parse_vol("OA"), Some(fade));
    assert_eq!(vol_effect(fade), Some((0xA, 0x0A)));
    assert_eq!(parse_vol("FF"), Some(0x80), "hex past 80 is full volume, not a command");
    assert_eq!(parse_vol("XA"), None);
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("noise-test-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_wav(path: &Path, frames: usize, channels: u16) {
    let frames = (0..frames).map(|i| [i as f32 / frames as f32, -(i as f32) / frames as f32]).collect();
    Sample { name: "t".into(), sample_rate: 22050.0, channels, frames }.save(path).unwrap();
}

#[test]
fn params_map_to_sliders_and_text() {
    let specs = ModuleKind::Filter.params();
    let cutoff = &specs[1];
    // Frequencies move in octaves: the middle of 20 Hz..20 kHz is 632 Hz.
    assert!((cutoff.value_at(0.5) - 632.5).abs() < 1.0);
    let attack = &ModuleKind::Generator.params()[2];
    // Envelope times give the short end more room.
    assert!((attack.value_at(0.5) - 0.5).abs() < 1e-6);
    for spec in ModuleKind::ADDABLE.iter().flat_map(|k| k.params()) {
        for v in [spec.min, spec.default, spec.max] {
            let back = spec.value_at(spec.position(v));
            assert!((back - v).abs() <= (spec.max - spec.min) * 1e-4, "{}: {v} -> {back}", spec.name);
        }
    }
    let unison = &ModuleKind::Generator.params()[7];
    assert_eq!(unison.value_at(0.4), 2.0);
    assert_eq!(cutoff.format(2000.0), "2.00 kHz");
    assert_eq!(attack.format(0.005), "5 ms");
    assert_eq!(ModuleKind::Generator.params()[0].format(0.5), "-6.0 dB");
    assert_eq!(ModuleKind::Generator.params()[9].format(-0.25), "25L");
    assert_eq!(ModuleKind::Generator.params()[1].format(3.0), "Sine");
}

#[test]
fn tracks_move_with_their_settings() {
    let mut p = Project::simple();
    p.tracks[1].name = "Hats".into();
    p.tracks[2].mute = true;
    let hats = p.patterns[1].tracks[1].clone();
    assert!(p.insert_track(1, 0));
    assert_eq!(p.tracks[2].name, "Hats");
    assert!(p.tracks[3].mute);
    assert_eq!(p.patterns[1].tracks[2], hats);
    assert_eq!(p.patterns[1].num_tracks(), 5);
    // After the last track, only the pattern being edited grows.
    assert!(p.insert_track(5, 0));
    assert_eq!((p.patterns[0].num_tracks(), p.patterns[1].num_tracks()), (6, 5));
    p.remove_track(5);
    p.remove_track(1);
    assert_eq!(p.track_name(1), "Hats");
    assert_eq!(p.track_name(MAX_TRACKS - 1), format!("Track {MAX_TRACKS}"), "a fresh track at the end");
    assert_eq!(p.patterns[1].tracks[1], hats);
    assert_eq!(p.tracks.len(), MAX_TRACKS);
    p.patterns[0].tracks.resize(MAX_TRACKS, vec![Cell::default(); MAX_LINES]);
    assert!(!p.insert_track(0, 0), "a full pattern refuses");
}

#[test]
fn solo_and_mute_decide_which_tracks_play() {
    let mut p = Project::empty();
    assert!(p.track_audible(0) && p.track_audible(1));
    p.tracks[1].solo = true;
    assert!(!p.track_audible(0) && p.track_audible(1));
    p.tracks[1].mute = true;
    assert!(!p.track_audible(1), "mute wins over solo");
}

#[test]
fn old_track_mutes_are_kept() {
    let dir = temp_dir("mutes");
    let mut json = serde_json::to_value(Project::empty()).unwrap();
    json.as_object_mut().unwrap().remove("tracks");
    json["track_mute"] = serde_json::json!([false, true]);
    let song = dir.join("old.json");
    std::fs::write(&song, json.to_string()).unwrap();
    let (p, _) = Project::load(song.to_str().unwrap()).unwrap();
    assert!(!p.tracks[0].mute && p.tracks[1].mute);
    assert_eq!(p.tracks.len(), MAX_TRACKS);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn note_columns_make_lanes() {
    let mut p = Project::empty();
    p.patterns.push(Pattern::new("b", 4, 64));
    p.set_columns(1, 3);
    let pat = &mut p.patterns[0];
    assert_eq!(pat.num_lanes(), 6);
    assert_eq!(pat.lane_pos(3), (1, 2));
    assert_eq!(pat.lane_of(2, 0), 4);
    pat.cell_mut(1, 2, 5).note = Some(Note::On(60));
    assert_eq!(pat.lane(3)[5].note, Some(Note::On(60)));
    assert!(pat.track_used(1));
    assert_eq!(p.patterns[1].columns(1), 3, "every pattern gets the columns");

    // Tracks move with their columns.
    p.insert_track(0, 0);
    assert_eq!(p.patterns[0].columns(2), 3);
    assert_eq!(p.patterns[0].cell(2, 2, 5).note, Some(Note::On(60)));
    p.remove_track(0);
    assert_eq!(p.patterns[0].columns(1), 3);

    // Hiding a column keeps its notes.
    p.set_columns(1, 1);
    assert!(!p.patterns[0].track_used(1));
    p.set_columns(1, 3);
    assert!(p.patterns[0].track_used(1));

    // A new pattern takes the song's columns.
    p.patterns.push(Pattern::new("c", 4, 64));
    p.sync_columns(2);
    assert_eq!(p.patterns[2].columns(1), 3);
}

#[test]
fn note_columns_are_saved() {
    let dir = temp_dir("columns");
    let mut p = Project::empty();
    p.set_columns(0, 2);
    p.patterns[0].cell_mut(0, 1, 3).note = Some(Note::On(50));
    let song = dir.join("song.json");
    std::fs::write(&song, serde_json::to_string(&p).unwrap()).unwrap();
    let (p, _) = Project::load(song.to_str().unwrap()).unwrap();
    assert_eq!(p.patterns[0].columns(0), 2);
    assert_eq!(p.patterns[0].cell(0, 1, 3).note, Some(Note::On(50)));
    // Songs without extra columns or comments save as before.
    let plain = serde_json::to_string(&Project::empty()).unwrap();
    assert!(!plain.contains("extra") && !plain.contains("columns") && !plain.contains("title"));
    let mut p = Project::empty();
    p.title = "Song".into();
    p.comments = "line one\nline two".into();
    std::fs::write(&song, serde_json::to_string(&p).unwrap()).unwrap();
    let (p, _) = Project::load(song.to_str().unwrap()).unwrap();
    assert_eq!((p.title.as_str(), p.comments.as_str()), ("Song", "line one\nline two"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn curves_pass_through_their_points() {
    let mut e = Envelope { curve: true, ..Envelope::new(1, 0, vec![(0.0, 0.0), (4.0, 1.0), (8.0, 0.0)]) };
    for (pos, v) in [(0.0, 0.0), (4.0, 1.0), (8.0, 0.0)] {
        assert_eq!(e.value_at(pos), Some(v));
    }
    let (curve, line) = (e.value_at(2.0).unwrap(), 0.5);
    assert!(curve > line, "it bows up towards the peak: {curve}");
    assert!(e.value_at(5.0).unwrap() <= 1.0, "and stays in range");
    e.steps = true;
    assert_eq!(e.value_at(2.0), Some(0.0), "points win");
    assert_eq!(MIXER_GAIN.name, ModuleKind::Eq.automatable(6).unwrap().name);
    assert_eq!(ModuleKind::Eq.automatable(7).unwrap().name, "Pan");
    assert!(ModuleKind::Eq.automatable(8).is_none());
}

#[test]
fn modulation_is_saved_only_when_used() {
    let mut p = Project::empty();
    let id = p.add_module(ModuleKind::Sampler, [0.0, 0.0]).unwrap();
    assert!(!serde_json::to_string(&p).unwrap().contains("modulation"));
    let m = &mut p.module_mut(id).unwrap().modulation;
    m.pitch.on = true;
    m.pitch.sustain = Some(7);
    let dir = temp_dir("modulation");
    let song = dir.join("song.json");
    std::fs::write(&song, serde_json::to_string(&p).unwrap()).unwrap();
    let (p, _) = Project::load(song.to_str().unwrap()).unwrap();
    let m = &p.module(id).unwrap().modulation;
    assert!(m.pitch.on);
    assert_eq!(m.pitch.sustain, None, "a sustain point that isn't there goes");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn excerpts_keep_only_the_selection() {
    let mut p = Project::empty();
    p.set_columns(0, 2);
    let pat = &mut p.patterns[0];
    pat.tracks[0][4].note = Some(Note::On(60));
    pat.cell_mut(0, 1, 5).note = Some(Note::On(62));
    pat.tracks[1][5].note = Some(Note::On(64));
    pat.tracks[1][20].note = Some(Note::On(65));
    pat.automation.push(Envelope::new(0, 0, vec![(0.0, 0.0), (8.0, 1.0), (40.0, 0.0)]));
    // Lines 4 to 11 of lanes 1 and 2: track 0's second column and track 1.
    let song = p.excerpt(0, (4, 11), (1, 2));
    let pat = &song.patterns[0];
    assert_eq!((song.patterns.len(), song.order.len(), pat.lines), (1, 1, 8));
    assert_eq!(pat.tracks[0][0].note, None, "the first column wasn't selected");
    assert_eq!(pat.cell(0, 1, 1).note, Some(Note::On(62)));
    assert_eq!(pat.tracks[1][1].note, Some(Note::On(64)));
    assert!(pat.tracks[1].iter().filter(|c| c.note.is_some()).count() == 1, "line 20 is outside");
    assert_eq!(pat.automation[0].points, [(0.0, 0.5), (4.0, 1.0)]);
}

#[test]
fn slice_settings_follow_markers() {
    let sample = Sample { name: "s".into(), sample_rate: 100.0, channels: 1, frames: vec![[0.0; 2]; 100] };
    let mut slot = SampleSlot::new(sample, None);
    slot.slices = vec![50];
    slot.slice_mut(1).transpose = 7;
    // Splitting the first slice: the second keeps its settings.
    slot.add_slice(20);
    assert_eq!(slot.slices, [20, 50]);
    assert_eq!(slot.slice(2).transpose, 7);
    // Splitting the last: both halves get them.
    slot.add_slice(80);
    assert_eq!((slot.slice(2).transpose, slot.slice(3).transpose), (7, 7));
    slot.remove_slice(0);
    assert_eq!(slot.slices, [50, 80]);
    assert_eq!((slot.slice(0).transpose, slot.slice(1).transpose), (0, 7));
    // Only settings that differ are saved.
    let plain = SampleSlot { slice_settings: vec![SliceSettings::default(); 3], ..slot.clone() };
    assert!(
        !serde_json::to_string(&SampleSlot { slice_settings: vec![], ..plain }).unwrap().contains("slice_settings")
    );
    assert!(serde_json::to_string(&slot).unwrap().contains("slice_settings"));
}

#[test]
fn instrument_chains() {
    let mut p = Project::simple();
    // The demo: the bass through its filter, the bell through delay
    // and reverb, the drums straight out.
    let (drums, bass, bell, filter, delay, verb) = (1, 2, 3, 4, 5, 6);
    assert_eq!(p.chain(bass), Chain { effects: vec![filter], outputs: vec![OUTPUT_ID] });
    assert_eq!(p.chain(bell), Chain { effects: vec![delay, verb], outputs: vec![OUTPUT_ID] });
    assert_eq!(p.chain(drums), Chain { effects: vec![], outputs: vec![OUTPUT_ID] });

    let eq = p.chain_insert(bass, 1, ModuleKind::Eq).unwrap();
    assert_eq!(p.chain(bass).effects, [filter, eq]);
    p.chain_move(bass, eq, -1);
    assert_eq!(p.chain(bass).effects, [eq, filter]);
    let amp = p.chain_insert(bass, 2, ModuleKind::Amplifier).unwrap();
    p.chain_place(bass, amp, 0);
    assert_eq!(p.chain(bass).effects, [amp, eq, filter]);
    p.chain_place(bass, amp, 9);
    assert_eq!(p.chain(bass).effects, [eq, filter, amp]);
    p.chain_remove(bass, amp);
    p.chain_remove(bass, eq);
    assert_eq!(p.chain(bass), Chain { effects: vec![filter], outputs: vec![OUTPUT_ID] });
    assert!(p.module(eq).is_none());

    // An effect two instruments feed is shared, not part of either chain.
    p.disconnect(drums, OUTPUT_ID);
    p.connect(drums, verb);
    assert_eq!(p.chain(bell), Chain { effects: vec![delay], outputs: vec![verb] });
    assert_eq!(p.chain(drums).outputs, [verb]);
    p.disconnect(drums, verb);
    p.connect(drums, OUTPUT_ID);
    assert_eq!(p.chain(bell).effects, [delay, verb]);
    // Sending the bell's chain into its own delay would loop: refused.
    assert!(!p.connect(verb, delay));

    // A Modulator moving the filter doesn't break the bass's chain.
    let lfo = p.add_module(ModuleKind::Modulator, [0.0, 0.0]).unwrap();
    p.connect(lfo, filter);
    assert_eq!(p.chain(bass).effects, [filter]);

    // A chain going nowhere gets the output when an effect is added.
    p.links.retain(|l| l.0 != drums);
    let comp = p.chain_insert(drums, 0, ModuleKind::Compressor).unwrap();
    assert_eq!(p.chain(drums), Chain { effects: vec![comp], outputs: vec![OUTPUT_ID] });
}

#[test]
fn phrases_are_saved_only_when_used() {
    let mut p = Project::empty();
    let id = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
    assert!(!serde_json::to_string(&p).unwrap().contains("phrase"));
    let mut phrase = Phrase { lines: 500, ..Phrase::default() };
    phrase.cells[1].note = Some(Note::On(50));
    let m = p.module_mut(id).unwrap();
    m.phrases = vec![Phrase::default(), phrase];
    m.selected_phrase = 1;
    m.phrase_mode = PhraseMode::Keymap;
    let dir = temp_dir("phrase");
    let song = dir.join("song.json");
    std::fs::write(&song, serde_json::to_string(&p).unwrap()).unwrap();
    let (p, _) = Project::load(song.to_str().unwrap()).unwrap();
    let m = p.module(id).unwrap();
    assert_eq!((m.phrases.len(), m.selected_phrase, m.phrase_mode), (2, 1, PhraseMode::Keymap));
    assert_eq!(m.phrases[1].cells[1].note, Some(Note::On(50)));
    assert_eq!(m.phrases[1].lines, MAX_PHRASE_LINES, "kept in range");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_single_phrase_of_older_songs_becomes_the_first() {
    let mut p = Project::empty();
    let id = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
    let mut json: serde_json::Value = serde_json::to_value(&p).unwrap();
    let m = json["modules"].as_array_mut().unwrap().iter_mut().find(|m| m["id"] == id).unwrap();
    m["phrase"] = serde_json::json!({ "on": true, "lines": 4, "lpb": 4, "looping": true, "cells": [] });
    let dir = temp_dir("old_phrase");
    let song = dir.join("song.json");
    std::fs::write(&song, json.to_string()).unwrap();
    let (p, _) = Project::load(song.to_str().unwrap()).unwrap();
    let m = p.module(id).unwrap();
    assert_eq!(m.phrase_mode, PhraseMode::Program);
    assert_eq!((m.phrases.len(), m.phrases[0].lines, m.phrases[0].looping), (1, 4, true));
    assert_eq!(m.phrases[0].keys, [0, 119]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn columns_a_track_does_not_show_read_as_empty() {
    let mut p = Pattern::new("t", 4, 64);
    let note = Cell { note: Some(Note::On(60)), ..Cell::default() };
    p.set_columns(0, 2);
    *p.cell_mut(0, 1, 3) = note;
    // The − under the name: the second note column goes, and a track
    // with no effect columns has nothing past its note columns.
    p.set_columns(0, 1);
    assert_eq!(p.column(0, 1)[3], Cell::default(), "no panic, and empty");
    assert_eq!(p.column(2, 5)[0], Cell::default(), "a track that never had columns");
    assert_eq!(p.column(9, 0)[0], Cell::default(), "a track the pattern lacks");
    // With an effect column, the same index is that column.
    p.set_fx_columns(0, 1);
    p.cell_mut(0, 1, 3).fx = Some((0x4, 0x22));
    assert_eq!(p.column(0, 1)[3].fx, Some((0x4, 0x22)));
    assert_eq!(p.column(0, 2)[3], Cell::default(), "past the effect columns");
}

#[test]
fn note_columns_come_back_with_their_notes() {
    let mut p = Pattern::new("t", 2, 64);
    let note = Cell { note: Some(Note::On(60)), ..Cell::default() };
    for fx in [0, 2] {
        p.set_fx_columns(0, fx);
        p.set_columns(0, 3);
        *p.cell_mut(0, 2, 7) = note;
        p.set_columns(0, 1);
        p.set_columns(0, 3);
        assert_eq!(p.cell(0, 2, 7), note, "with {fx} effect columns");
        // The effect columns weren't touched by any of it.
        for c in 0..fx {
            assert_eq!(p.column(0, 3 + c)[7], Cell::default());
        }
        p.clear_track(0);
    }
}

#[test]
fn lanes_follow_the_columns_shown() {
    let mut p = Pattern::new("t", 3, 64);
    // Any order of + and − leaves a lane for each column shown, and
    // every lane reads without panicking.
    let steps: [(usize, usize, usize); 6] = [(0, 3, 0), (0, 3, 2), (0, 1, 2), (1, 2, 1), (0, 1, 0), (1, 1, 0)];
    for (t, notes, fx) in steps {
        p.set_columns(t, notes);
        p.set_fx_columns(t, fx);
        let widths: usize = (0..3).map(|t| p.width(t)).sum();
        assert_eq!(p.num_lanes(), widths);
        for lane in 0..p.num_lanes() {
            let (track, col) = p.lane_pos(lane);
            assert_eq!(p.lane_of(track, col), lane);
            assert_eq!(p.lane(lane).len(), MAX_LINES);
        }
    }
}

#[test]
fn effect_columns_are_kept_by_the_matrix_and_song_files() {
    let mut p = Project::empty();
    p.set_fx_columns(1, 2);
    p.patterns[0].cell_mut(1, 2, 5).fx = Some((0xA, 0x0F));
    p.order.push(Slot::new(0));
    p.patterns.push(Pattern::new("b", 4, 64));
    p.order[1].pattern = 1;
    let clip = p.copy_matrix((0, 0), (1, 1));
    assert_eq!((clip[0][0].notes.len(), clip[0][0].effects.len()), (1, 2));
    p.paste_matrix(&clip, 1, 1);
    assert_eq!(p.patterns[1].fx_columns(1), 2, "the effect columns come along");
    assert_eq!(p.patterns[1].cell(1, 2, 5).fx, Some((0xA, 0x0F)));
    let dir = temp_dir("fx_columns");
    let song = dir.join("song.json");
    std::fs::write(&song, serde_json::to_string(&p).unwrap()).unwrap();
    let (back, _) = Project::load(song.to_str().unwrap()).unwrap();
    assert_eq!(back.patterns[0].fx_columns(1), 2);
    assert_eq!(back.patterns[0].track_effects(1, 5).collect::<Vec<_>>(), [(0xA, 0x0F)]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn soloing_a_track_silences_the_others_until_it_is_soloed_again() {
    let mut p = Project::empty();
    p.solo_track(2);
    assert!(p.track_audible(2) && !p.track_audible(0));
    p.solo_track(1);
    assert!(p.track_audible(1) && !p.track_audible(2), "the solo moves");
    p.solo_track(1);
    assert!((0..4).all(|t| p.track_audible(t)), "and comes off");
}

#[test]
fn the_demo_saves_and_opens_the_same() {
    let p = Project::demo();
    let dir = temp_dir("demo");
    let song = dir.join("demo.json");
    std::fs::write(&song, serde_json::to_string(&p).unwrap()).unwrap();
    let (back, warnings) = Project::load(song.to_str().unwrap()).unwrap();
    assert!(warnings.is_empty());
    assert_eq!(serde_json::to_string(&back).unwrap(), serde_json::to_string(&p).unwrap());
    // Its strings play chords in three note columns, and the arp phrases.
    assert_eq!(back.patterns[1].columns(3), 3);
    assert!(back.modules.iter().any(|m| m.phrase_mode == PhraseMode::Program && !m.phrases.is_empty()));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_mix_goes_through_the_master_chain() {
    let mut p = Project::empty();
    let synth = p.add_module(ModuleKind::Generator, [0.0; 2]).unwrap();
    p.connect(synth, OUTPUT_ID);
    let eq = p.chain_insert(OUTPUT_ID, 0, ModuleKind::Eq).unwrap();
    let limit = p.chain_insert(OUTPUT_ID, 1, ModuleKind::Maximizer).unwrap();
    assert_eq!(p.master, [eq, limit]);
    assert!(!p.links.iter().any(|l| l.0 == eq || l.1 == eq), "no links of their own");
    let path = p.audio_links();
    assert!(path.contains(&(synth, eq)) && path.contains(&(eq, limit)) && path.contains(&(limit, OUTPUT_ID)));
    assert!(!path.contains(&(synth, OUTPUT_ID)), "the synth reaches the output through them");
    assert!(!p.connect(synth, eq), "they aren't wired by hand");
    let lfo = p.add_module(ModuleKind::Modulator, [0.0; 2]).unwrap();
    assert!(p.connect(lfo, eq), "but Modulators move them");
    p.chain_move(OUTPUT_ID, limit, -1);
    assert_eq!(p.master, [limit, eq]);
    p.chain_remove(OUTPUT_ID, limit);
    assert_eq!(p.master, [eq]);
    assert!(p.module(limit).is_none());
    // Saved and loaded, it stays; a stray link to it or a wrong id goes.
    p.links.push((synth, eq));
    p.master.extend([eq, 200, synth]);
    let path = std::env::temp_dir().join(format!("noise-master-{}.json", std::process::id()));
    std::fs::write(&path, serde_json::to_string(&p).unwrap()).unwrap();
    let (back, _) = Project::load(path.to_str().unwrap()).unwrap();
    std::fs::remove_file(path).ok();
    assert_eq!(back.master, [eq]);
    assert!(!back.links.contains(&(synth, eq)));
}

#[test]
fn songs_saved_not_to_loop_end_on_an_f00() {
    let mut p = Project::empty();
    p.patterns[0].lines = 8;
    p.patterns[0].tracks[0][7].fx = Some((0x1, 0x10));
    let mut json: serde_json::Value = serde_json::to_value(&p).unwrap();
    assert!(json.get("loop_song").is_none(), "no longer saved");
    let path = std::env::temp_dir().join(format!("noise-loop-song-{}.json", std::process::id()));
    for looping in [true, false] {
        json["loop_song"] = serde_json::Value::Bool(looping);
        std::fs::write(&path, json.to_string()).unwrap();
        let (back, _) = Project::load(path.to_str().unwrap()).unwrap();
        let last = |t: usize| back.patterns[0].tracks[t][7].fx;
        assert_eq!(last(0), Some((0x1, 0x10)), "an effect there stays");
        assert_eq!(last(1), (!looping).then_some((0xF, 0)), "the next track's cell gets the F00");
    }
    std::fs::remove_file(path).ok();
}

#[test]
fn tracks_keep_their_own_effects() {
    let mut p = Project::empty();
    let synth = p.add_module(ModuleKind::Generator, [0.0; 2]).unwrap();
    p.connect(synth, OUTPUT_ID);
    p.patterns[0].tracks[2][0] = Cell { note: Some(Note::On(60)), module: Some(synth), ..Cell::default() };
    let echo = p.chain_insert(Owner::Track(2), 0, ModuleKind::Echo).unwrap();
    assert_eq!(p.tracks[2].effects, [echo]);
    assert!(p.placed(echo) && !p.connect(synth, echo), "wired by its place, not by hand");
    assert_eq!(p.track_instruments(2), [synth]);
    // A Modulator listening halfway along the synth's own chain doesn't
    // cut it short: the chain goes on to the output past it.
    let amp = p.chain_insert(synth, 0, ModuleKind::Amplifier).unwrap();
    let lim = p.chain_insert(synth, 1, ModuleKind::Maximizer).unwrap();
    let duck = p.add_module(ModuleKind::Modulator, [0.0; 2]).unwrap();
    assert!(p.connect(amp, duck));
    assert_eq!(p.chain(synth).effects, [amp, lim]);
    // The path: the synth's copy for track 2, with its chain, goes
    // through the echo.
    let (nodes, links) = p.signal_graph();
    assert!(nodes.contains(&(synth, Some(2))) && nodes.contains(&(lim, Some(2))));
    assert!(links.contains(&((lim, Some(2)), (echo, None))) && links.contains(&((echo, None), (OUTPUT_ID, None))));
    // Saved and loaded, it stays; a stray link to it goes.
    p.links.push((synth, echo));
    let path = std::env::temp_dir().join(format!("noise-track-fx-{}.json", std::process::id()));
    std::fs::write(&path, serde_json::to_string(&p).unwrap()).unwrap();
    let (mut back, _) = Project::load(path.to_str().unwrap()).unwrap();
    std::fs::remove_file(path).ok();
    assert_eq!(back.tracks[2].effects, [echo]);
    assert!(!back.links.contains(&(synth, echo)));
    // A track that goes takes its effects with it.
    back.remove_track(2);
    assert!(back.module(echo).is_none());
}

#[test]
fn grouped_tracks_go_on_through_the_group_s_effects() {
    let mut p = Project::empty();
    for _ in 0..3 {
        p.insert_track(0, 0);
    }
    let synth = p.add_module(ModuleKind::Generator, [0.0; 2]).unwrap();
    p.connect(synth, OUTPUT_ID);
    for t in [0, 1] {
        p.patterns[0].tracks[t][0] = Cell { note: Some(Note::On(60)), module: Some(synth), ..Cell::default() };
    }
    let echo = p.chain_insert(Owner::Track(0), 0, ModuleKind::Echo).unwrap();
    let bus = p.chain_insert(Owner::Track(2), 0, ModuleKind::Compressor).unwrap();
    p.tracks[0].group = Some(2);
    p.tracks[1].group = Some(2);
    let (nodes, links) = p.signal_graph();
    // Track 0's echo goes into the bus; track 1, without effects of its
    // own, goes straight in; the bus goes to the output.
    assert!(links.contains(&((echo, None), (bus, None))));
    assert!(nodes.contains(&(synth, Some(1))) && links.contains(&((synth, Some(1)), (bus, None))));
    assert!(links.contains(&((bus, None), (OUTPUT_ID, None))));
    // No loops: track 2 can't go through track 0, which goes through it.
    assert!(!p.can_group(2, 0) && !p.can_group(2, 2) && p.can_group(2, 3));
    // Tracks moving keep their groups; the group's going ends them.
    p.insert_track(0, 0);
    assert_eq!((p.tracks[1].group, p.tracks[2].group), (Some(3), Some(3)));
    p.remove_track(3);
    assert_eq!((p.tracks[1].group, p.tracks[2].group), (None, None));
}

#[test]
fn envelopes_run_from_line_to_line_or_repeat_every_so_many_beats() {
    // A ramp over 16 lines from line 8 of a 64-line pattern, 4 lines a beat.
    let mut env = Envelope::new(1, 0, vec![(0.0, 0.0), (16.0, 1.0)]);
    (env.start, env.lines) = (8.0, 16.0);
    assert_eq!(env.span(64, 4), 16.0);
    assert_eq!(env.value_in(4.0, 64, 4), None, "not before its first line");
    assert_eq!(env.value_in(16.0, 64, 4), Some(0.5));
    assert_eq!(env.value_in(30.0, 64, 4), None, "nor after its last");
    // Every 2 beats, 8 lines, from the top of the pattern to its end.
    let mut every = Envelope::new(1, 0, vec![(0.0, 0.0), (8.0, 1.0)]);
    every.every = 2.0;
    assert_eq!(every.span(64, 4), 8.0);
    assert_eq!(every.value_in(4.0, 64, 4), Some(0.5));
    assert_eq!(every.value_in(52.0, 64, 4), Some(0.5), "over and over");
    assert_eq!(every.span(64, 8), 16.0, "it keeps to beats when the lines per beat change");
    // No lines given, it runs to the end of the pattern and saves as before.
    let whole = Envelope::new(1, 0, vec![(0.0, 0.0), (64.0, 1.0)]);
    assert_eq!(whole.value_in(32.0, 64, 4), Some(0.5));
    let json = serde_json::to_string(&whole).unwrap();
    assert!(!["length", "lines", "start", "repeat", "every"].iter().any(|k| json.contains(k)), "{json}");
}

#[test]
fn envelopes_saved_by_earlier_versions_open_as_they_sounded() {
    let dir = temp_dir("old-envelopes");
    let mut p = Project::empty();
    let id = p.add_module(ModuleKind::Generator, [0.0; 2]).unwrap();
    p.patterns[0].automation = vec![Envelope::new(id, 0, vec![(0.0, 0.0), (16.0, 1.0)])];
    let mut json = serde_json::to_value(&p).unwrap();
    // A quarter of the pattern repeating, one holding its last value, and
    // 8 lines repeating as the version before this one saved them.
    let env = json["patterns"][0]["automation"][0].clone();
    let old = |fields: serde_json::Value| {
        let mut e = env.clone();
        e.as_object_mut().unwrap().extend(fields.as_object().unwrap().clone());
        e
    };
    json["patterns"][0]["automation"] = serde_json::json!([
        old(serde_json::json!({"length": 0.25, "repeat": true})),
        old(serde_json::json!({"length": 0.25})),
        old(serde_json::json!({"lines": 8.0, "repeat": true})),
    ]);
    let path = dir.join("old.json");
    std::fs::write(&path, json.to_string()).unwrap();
    let (back, _) = Project::load(path.to_str().unwrap()).unwrap();
    let a = &back.patterns[0].automation;
    let lpb = back.lpb;
    assert_eq!(a[0].every, 16.0 / lpb as f32, "a repeating quarter repeats every 16 lines");
    assert_eq!(a[0].value_in(40.0, 64, lpb), Some(0.5));
    assert_eq!(a[1].value_in(40.0, 64, lpb), Some(1.0), "one that held still holds its last value");
    assert_eq!(a[2].every, 8.0 / lpb as f32);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn slots_move_to_where_they_are_dropped() {
    let mut p = Project::empty();
    p.order = (0..4).map(Slot::new).collect();
    let patterns = |p: &Project| p.order.iter().map(|s| s.pattern).collect::<Vec<_>>();
    p.move_slot(0, 2);
    assert_eq!(patterns(&p), [1, 2, 0, 3]);
    p.move_slot(3, 0);
    assert_eq!(patterns(&p), [3, 1, 2, 0]);
    p.move_slot(1, 9);
    assert_eq!(patterns(&p), [3, 2, 0, 1], "past the end is the end");
}

#[test]
fn sections_follow_their_slots() {
    let mut p = Project::empty();
    for _ in 0..4 {
        p.insert_slot(1, Slot::new(0));
    }
    p.set_section(0, "Intro".into());
    p.set_section(2, "Verse".into());
    assert_eq!(p.section_slots(0), (0, 1));
    assert_eq!(p.section_slots(2), (2, 4));
    p.insert_slot(1, Slot::new(0));
    assert_eq!(p.sections[1].start, 3, "pushed down");
    p.insert_slot(0, Slot::new(0));
    assert_eq!(p.sections[0].start, 0, "the first section stays at the top");
    p.remove_slot(4);
    assert_eq!(p.sections[1].start, 4, "the section's next slot starts it");
    for _ in 0..3 {
        p.remove_slot(0);
    }
    assert_eq!(p.sections.iter().map(|s| s.start).collect::<Vec<_>>(), [0, 1]);
    p.remove_slot(0);
    assert_eq!(p.sections.len(), 1, "sections that meet merge");
    assert_eq!(p.sections[0].name, "Intro");
}

#[test]
fn matrix_blocks_copy_tracks_between_slots() {
    let mut p = Project::empty();
    p.patterns.push(Pattern::new("b", 4, 64));
    p.order.push(Slot::new(1));
    p.set_columns(1, 2);
    p.patterns[0].cell_mut(1, 1, 3).note = Some(Note::On(60));
    p.patterns[0].tracks[2][0].note = Some(Note::On(50));
    let clip = p.copy_matrix((0, 0), (1, 2));
    p.paste_matrix(&clip, 1, 0);
    assert_eq!(p.patterns[1].cell(0, 1, 3).note, Some(Note::On(60)), "columns come along");
    assert_eq!(p.patterns[1].columns(0), 2);
    assert_eq!(p.patterns[1].tracks[1][0].note, Some(Note::On(50)));
    p.clear_matrix((0, 1), (0, 1));
    assert!(!p.patterns[1].track_used(0) && !p.patterns[0].track_used(1));
    assert!(p.patterns[0].track_used(2), "outside the block");
}

#[test]
fn new_params_start_at_their_defaults() {
    let dir = temp_dir("params");
    let mut p = Project::empty();
    let id = p.add_module(ModuleKind::Distortion, [0.0, 0.0]).unwrap();
    p.module_mut(id).unwrap().params.truncate(3);
    let song = dir.join("old.json");
    std::fs::write(&song, serde_json::to_string(&p).unwrap()).unwrap();
    let (p, _) = Project::load(song.to_str().unwrap()).unwrap();
    let defaults: Vec<f32> = ModuleKind::Distortion.params().iter().map(|s| s.default).collect();
    assert_eq!(p.module(id).unwrap().params, defaults);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn slots_without_mutes_save_as_numbers() {
    let mut slot = Slot::new(3);
    assert_eq!(serde_json::to_string(&slot).unwrap(), "3");
    slot.toggle_mute(2);
    let json = serde_json::to_string(&slot).unwrap();
    assert_eq!(json, r#"{"pattern":3,"muted":4}"#);
    assert_eq!(serde_json::from_str::<Slot>(&json).unwrap(), slot);
    assert!(slot.is_muted(2) && !slot.is_muted(1));
    let old: Vec<Slot> = serde_json::from_str("[0, 1, 1]").unwrap();
    assert_eq!(old, [Slot::new(0), Slot::new(1), Slot::new(1)]);
}

#[test]
fn envelopes_interpolate_between_points() {
    let mut e = Envelope::new(1, 0, vec![]);
    assert_eq!(e.value_at(3.0), None);
    e.set(4.0, 1.0);
    e.set(0.0, 0.0);
    e.set(8.0, 0.5);
    assert_eq!(e.points, [(0.0, 0.0), (4.0, 1.0), (8.0, 0.5)]);
    assert_eq!(e.value_at(2.0), Some(0.5));
    assert_eq!(e.value_at(6.0), Some(0.75));
    assert_eq!(e.value_at(20.0), Some(0.5), "the last point holds");
    e.set(4.0, 0.25);
    assert_eq!(e.points.len(), 3, "setting a point that is there moves it");
    e.steps = true;
    assert_eq!(e.value_at(7.9), Some(0.25));
}

#[test]
fn sample_files_round_trip() {
    let dir = temp_dir("roundtrip");
    for channels in [1, 2] {
        let path = dir.join(format!("{channels}.wav"));
        write_wav(&path, 1000, channels);
        let s = Sample::load(&path).unwrap();
        assert_eq!((s.len(), s.channels, s.sample_rate), (1000, channels, 22050.0));
        assert_eq!(s.frames[500][0], 0.5);
        // Mono files hold the left channel in both.
        let right = if channels == 1 { 0.5 } else { -0.5 };
        assert_eq!(s.frames[500][1], right);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn old_samplers_become_sample_slots() {
    let dir = temp_dir("migrate");
    write_wav(&dir.join("pad.wav"), 1000, 2);
    let mut p = Project::empty();
    let id = p.add_module(ModuleKind::Sampler, [0.0, 0.0]).unwrap();
    let mut json = serde_json::to_value(&p).unwrap();
    // Volume, root note, fine tune, attack, release, loop (ping-pong),
    // start, loop start, loop end, pan.
    let m = json["modules"].as_array_mut().unwrap().iter_mut().find(|m| m["id"] == id).unwrap();
    m["params"] = serde_json::json!([0.9, 50.0, 12.0, 0.01, 0.2, 2.0, 0.0, 0.25, 0.75, -0.5]);
    m["sample_path"] = "pad.wav".into();
    let song = dir.join("old.json");
    std::fs::write(&song, json.to_string()).unwrap();

    let (p, warnings) = Project::load(song.to_str().unwrap()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let m = p.module(id).unwrap();
    assert_eq!(m.params, vec![0.9, -0.5, 0.0, 0.01, 0.5, 1.0, 0.2]);
    let s = &m.samples[0];
    assert_eq!((s.base_note, s.finetune, s.loop_mode), (50, 12, 3));
    assert_eq!((s.loop_start, s.loop_end), (250, 750));
    assert_eq!(s.len(), 1000);

    // Saved again, the old field is gone and the slot comes back.
    let again = serde_json::to_string(&p).unwrap();
    assert!(!again.contains("sample_path"));
    std::fs::write(&song, again).unwrap();
    let (p2, _) = Project::load(song.to_str().unwrap()).unwrap();
    assert_eq!(p2.module(id).unwrap().samples[0].loop_end, 750);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_samples_are_warnings() {
    let dir = temp_dir("missing");
    let mut p = Project::empty();
    let id = p.add_module(ModuleKind::Sampler, [0.0, 0.0]).unwrap();
    p.module_mut(id).unwrap().samples.push(SampleSlot { path: Some("gone.wav".into()), ..Default::default() });
    let song = dir.join("song.json");
    std::fs::write(&song, serde_json::to_string(&p).unwrap()).unwrap();
    let (p, warnings) = Project::load(song.to_str().unwrap()).unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(p.module(id).unwrap().samples[0].data.is_none());
    std::fs::remove_dir_all(dir).unwrap();
}
