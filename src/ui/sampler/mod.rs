//! The sampler: the sample list, a waveform editor and a keyzone
//! map for the selected Sampler instrument.

use super::{App, Preview, icons, instruments, theme};
use crate::dsp::Frame;
use crate::engine::{Cmd, LIVE_KEY};
use crate::project::{LOOP_MODES, ModuleKind, Note, SampleSlot};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::sync::Arc;

mod edit;
mod keyzones;
mod waveform;

pub use edit::*;
use keyzones::*;
pub use waveform::*;

#[derive(Clone, Copy, PartialEq, Default)]
pub enum Tab {
    #[default]
    Waveform,
    Keyzones,
    Modulation,
    /// The instrument and its effects, as a chain of devices.
    Effects,
    Phrase,
}

/// The pages of a synth (anything but a Sampler) in the instrument editor.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum SynthTab {
    #[default]
    Synth,
    Modulation,
    Phrase,
}

#[derive(Clone, Copy, PartialEq)]
enum WaveDrag {
    /// Selecting from this frame.
    Select(usize),
    LoopStart,
    LoopEnd,
    /// Dragging the scrollbar thumb.
    Scroll,
    /// Moving slice marker `n`.
    Slice(usize),
}

#[derive(Clone, Copy, PartialEq)]
enum ZoneDrag {
    Move { grab: i32 },
    Low,
    High,
    VelLow,
    VelHigh,
}

#[derive(Default)]
pub struct SamplerView {
    pub tab: Tab,
    pub synth_tab: SynthTab,
    /// Selected sample slot.
    pub sample: usize,
    /// Selected slice of that sample, whose settings are shown.
    pub slice: Option<usize>,
    /// Visible range of the waveform in frames: first frame and length.
    view: Option<(f64, f64)>,
    /// What `view` and `selection` belong to: module, slot and audio.
    shown: Option<(u8, usize, usize)>,
    /// Selected frames, end exclusive.
    selection: Option<(usize, usize)>,
    wave_drag: Option<WaveDrag>,
    zone_drag: Option<(usize, ZoneDrag)>,
    /// Note held down on the keyzone keyboard.
    held: Option<(u8, u8)>,
    clipboard: Option<Vec<Frame>>,
    /// Overview of the shown sample, keyed by the audio's address.
    peaks: Option<(usize, Vec<Peak>)>,
}

/// Frames per entry in the waveform overview.
const PEAK_BLOCK: usize = 256;

/// Lowest and highest value of each channel over `PEAK_BLOCK` frames.
pub type Peak = [[f32; 2]; 2];

pub fn overview(frames: &[Frame]) -> Vec<Peak> {
    frames
        .chunks(PEAK_BLOCK)
        .map(|c| {
            let mut p = [[f32::MAX, f32::MIN]; 2];
            for f in c {
                for ch in 0..2 {
                    p[ch] = [p[ch][0].min(f[ch]), p[ch][1].max(f[ch])];
                }
            }
            p
        })
        .collect()
}

fn slots(app: &App, id: u8) -> &[SampleSlot] {
    app.project.module(id).map_or(&[], |m| &m.samples)
}

fn slot_mut(app: &mut App, id: u8, i: usize) -> Option<&mut SampleSlot> {
    app.project.module_mut(id).and_then(|m| m.samples.get_mut(i))
}

/// The instrument editor: the selected instrument's pages.
pub fn editor(app: &mut App, ui: &mut egui::Ui) {
    let Some(id) = app.instrument() else {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("Select an instrument in the list on the right, or add one.").color(theme::TEXT_WEAK),
            );
            ui.horizontal(|ui| {
                for kind in ModuleKind::ADDABLE.into_iter().filter(|k| k.is_instrument()) {
                    if ui.button(format!("+ {}", kind.name())).clicked() {
                        instruments::add(app, kind);
                    }
                }
            });
        });
        return;
    };
    let Some(m) = app.project.module(id).cloned() else { return };
    if !m.kind.holds_samples() {
        // A synth: its device chain, and its modulation.
        ui.horizontal(|ui| {
            theme::caption(ui, &format!("INSTRUMENT {id:02X} · {}", m.name));
            let tip = "The synth and the effects it goes through, each with its parameters";
            if theme::toggle(ui, app.sampler.synth_tab == SynthTab::Synth, "Synth").on_hover_text(tip).clicked() {
                app.sampler.synth_tab = SynthTab::Synth;
            }
            if m.kind.has_modulation() {
                let tip = "Pitch and filter envelopes, vibrato and tremolo for every voice";
                if theme::toggle(ui, app.sampler.synth_tab == SynthTab::Modulation, "Modulation")
                    .on_hover_text(tip)
                    .clicked()
                {
                    app.sampler.synth_tab = SynthTab::Modulation;
                }
            }
            if m.kind.makes_sound() {
                let tip = "A short pattern the instrument plays for each note";
                let label = if m.plays_phrases() { "Phrase •" } else { "Phrase" };
                if theme::toggle(ui, app.sampler.synth_tab == SynthTab::Phrase, label).on_hover_text(tip).clicked() {
                    app.sampler.synth_tab = SynthTab::Phrase;
                }
            }
        });
        match app.sampler.synth_tab {
            SynthTab::Modulation if m.kind.has_modulation() => super::modulation::page(app, ui, id),
            SynthTab::Phrase if m.kind.makes_sound() => super::phrase::page(app, ui, id),
            _ => super::chain::page(app, ui, id),
        }
        return;
    }
    let tabs = |app: &mut App, ui: &mut egui::Ui| {
        ui.horizontal(|ui| {
            if theme::toggle(ui, app.sampler.tab == Tab::Waveform, "Waveform").clicked() {
                app.sampler.tab = Tab::Waveform;
            }
            if theme::toggle(ui, app.sampler.tab == Tab::Keyzones, "Keyzones").clicked() {
                app.sampler.tab = Tab::Keyzones;
            }
            let tip = "Pitch and filter envelopes, vibrato and tremolo for every voice";
            if m.kind.has_modulation()
                && theme::toggle(ui, app.sampler.tab == Tab::Modulation, "Modulation").on_hover_text(tip).clicked()
            {
                app.sampler.tab = Tab::Modulation;
            }
            let tip = "The Sampler's own settings and the effects it goes through";
            if theme::toggle(ui, app.sampler.tab == Tab::Effects, "Effects").on_hover_text(tip).clicked() {
                app.sampler.tab = Tab::Effects;
            }
            let tip = "A short pattern the Sampler plays for each note";
            let on = app.instrument().and_then(|id| app.project.module(id)).is_some_and(|m| m.plays_phrases());
            if theme::toggle(ui, app.sampler.tab == Tab::Phrase, if on { "Phrase •" } else { "Phrase" })
                .on_hover_text(tip)
                .clicked()
            {
                app.sampler.tab = Tab::Phrase;
            }
        });
    };
    if app.sampler.tab == Tab::Effects {
        // The chain needs the width; the sample list stays on the others.
        ui.horizontal(|ui| {
            theme::caption(ui, &format!("INSTRUMENT {id:02X} · {}", m.name));
            tabs(app, ui);
        });
        super::chain::page(app, ui, id);
        return;
    }
    let count = slots(app, id).len();
    app.sampler.sample = app.sampler.sample.min(count.saturating_sub(1));

    let h = ui.available_height();
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(210.0, h), egui::Layout::top_down(egui::Align::Min), |ui| {
            sample_list(app, ui, id);
        });
        ui.separator();
        ui.vertical(|ui| {
            tabs(app, ui);
            match app.sampler.tab {
                Tab::Waveform => waveform_tab(app, ui, id),
                Tab::Keyzones => keyzones_tab(app, ui, id),
                Tab::Modulation if m.kind.has_modulation() => super::modulation::page(app, ui, id),
                Tab::Modulation => waveform_tab(app, ui, id),
                Tab::Phrase => super::phrase::page(app, ui, id),
                Tab::Effects => {}
            }
        });
    });
}

// ---------------------------------------------------------------- sample list

fn sample_list(app: &mut App, ui: &mut egui::Ui, id: u8) {
    theme::caption(ui, "SAMPLES");
    let mut select = None;
    egui::Frame::new().fill(theme::INSET).inner_margin(2).show(ui, |ui| {
        let list = egui::ScrollArea::vertical().id_salt("samples").max_height(ui.available_height() - 60.0);
        list.auto_shrink(false).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                for (i, s) in slots(app, id).iter().enumerate() {
                    let mut text = RichText::new(format!("{i:02}  {}", s.name));
                    if s.data.is_none() {
                        text = text.color(theme::RECORD);
                    }
                    let resp = ui.selectable_label(i == app.sampler.sample, text);
                    let resp = match &s.path {
                        Some(p) if s.data.is_none() => resp.on_hover_text(format!("Missing: {p}")),
                        _ => resp,
                    };
                    if resp.clicked() {
                        select = Some((i, None));
                    }
                    // A sliced sample lists its slices.
                    for (k, (a, b)) in s.slice_ranges().into_iter().enumerate() {
                        let secs = s.data.as_ref().map_or(0.0, |d| (b - a) as f32 / d.sample_rate);
                        let text = format!("    {:02} {}  {secs:.2} s", k + 1, Note::On(s.slice_note(k)).label());
                        let on = i == app.sampler.sample && app.sampler.slice == Some(k);
                        let color = if on { theme::SELECTED_TEXT } else { theme::PAT_EFFECT };
                        if ui.selectable_label(on, RichText::new(text).color(color).small()).clicked() {
                            select = Some((i, Some(k)));
                        }
                    }
                }
                if slots(app, id).is_empty() {
                    ui.label(
                        RichText::new("No samples. Load one or a soundfont, or drop audio files onto the window.")
                            .color(theme::TEXT_WEAK),
                    );
                }
            });
        });
    });
    if let Some((i, slice)) = select {
        app.sampler.sample = i;
        app.sampler.slice = slice;
        let range = slice.and_then(|k| slots(app, id).get(i)?.slice_ranges().get(k).copied());
        if let Some(range) = range {
            app.sampler.selection = Some(range);
        }
        // With Autoplay, as in the disk browser, a click plays the sample,
        // or the slice clicked.
        if app.browser.autoplay
            && let Some(data) = slots(app, id).get(i).and_then(|s| s.data.clone())
        {
            let (a, b) = range.unwrap_or((0, data.len()));
            let sample = if range.is_some() { Arc::new(data.with_frames(data.frames[a..b].to_vec())) } else { data };
            app.preview(sample, Some((id, i, a)));
        }
    }
    let cur = app.sampler.sample;
    let has = cur < slots(app, id).len();
    ui.horizontal_wrapped(|ui| {
        if ui.button("Load…").on_hover_text("Add a sample from a WAV, FLAC or Ogg file, or load an SF2, SF3 or SFZ soundfont in place of the samples").clicked() {
            app.pick_file(super::files::Purpose::LoadSample(id));
        }
        let tip = "Record the sound device's input, or the song as it plays, into a new sample";
        if ui.button("Record…").on_hover_text(tip).clicked() {
            app.recorder.open = true;
        }
        if ui.add_enabled(has, egui::Button::new("Dup")).on_hover_text("Duplicate the sample").clicked() {
            let m = app.project.module_mut(id).unwrap();
            let mut copy = m.samples[cur].clone();
            copy.name.push_str(" copy");
            m.samples.insert(cur + 1, copy);
            app.sampler.sample = cur + 1;
            app.sampler.slice = None;
            app.mark();
        }
        if ui.add_enabled(has, egui::Button::new("−")).on_hover_text("Delete the sample").clicked() {
            app.project.module_mut(id).unwrap().samples.remove(cur);
            app.mark();
        }
        if ui.add_enabled_ui(has && cur > 0, |ui| super::icons::button(ui, super::icons::Icon::Up)).inner.on_hover_text("Move up").clicked() {
            app.project.module_mut(id).unwrap().samples.swap(cur, cur - 1);
            app.sampler.sample = cur - 1;
            app.sampler.slice = None;
            app.mark();
        }
        if ui.add_enabled_ui(cur + 1 < slots(app, id).len(), |ui| super::icons::button(ui, super::icons::Icon::Down)).inner.on_hover_text("Move down").clicked()
        {
            app.project.module_mut(id).unwrap().samples.swap(cur, cur + 1);
            app.sampler.sample = cur + 1;
            app.sampler.slice = None;
            app.mark();
        }
    });
}

/// Lays samples out one key each from C-4 up, each at its original pitch.
pub fn drum_kit(samples: &mut [SampleSlot]) {
    for (k, s) in samples.iter_mut().enumerate() {
        let note = (48 + k).min(119) as u8;
        s.keys = [note, note];
        s.base_note = note;
    }
}

#[cfg(test)]
mod tests;
