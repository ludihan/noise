//! The sampler: the sample list, a waveform editor and a keyzone
//! map for the selected Sampler instrument.

use super::{App, instruments, theme};
use crate::dsp::Frame;
use crate::engine::{Cmd, LIVE_KEY};
use crate::project::{LOOP_MODES, ModuleKind, Note, SampleSlot};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::sync::Arc;

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

// ---------------------------------------------------------------- waveform

fn waveform_tab(app: &mut App, ui: &mut egui::Ui, id: u8) {
    let i = app.sampler.sample;
    let Some(slot) = slots(app, id).get(i).cloned() else {
        ui.label(RichText::new("No sample selected.").color(theme::TEXT_WEAK));
        return;
    };
    let Some(data) = slot.data.clone() else {
        ui.label(RichText::new(format!("The file for this sample is missing: {}", slot.path.unwrap_or_default())))
            .on_hover_text("Load the sample again, or fix the path in the song file");
        return;
    };
    let len = data.len();
    let key = (id, i, Arc::as_ptr(&data) as usize);
    let sv = &mut app.sampler;
    if sv.shown != Some(key) {
        // Another sample, or the audio changed: keep what still fits.
        if sv.shown.is_none_or(|(m, s, _)| (m, s) != (id, i)) {
            sv.view = None;
            sv.selection = None;
        }
        sv.shown = Some(key);
    }
    sv.selection = sv.selection.and_then(|(a, b)| {
        let (a, b) = (a.min(len), b.min(len));
        (b > a).then_some((a, b))
    });
    let (mut v0, mut vlen) = sv.view.unwrap_or((0.0, len as f64));
    edit_toolbar(app, ui, id, i, len, &mut v0, &mut vlen);
    keyboard_shortcuts(app, ui, id, i);
    vlen = vlen.clamp(16.0_f64.min(len as f64), len as f64);
    v0 = v0.clamp(0.0, len as f64 - vlen);

    // The selected slice's settings go above the sample's.
    let slice = app.sampler.slice.filter(|&k| k < slot.slice_ranges().len());
    // Their height last frame, which grows when they wrap in a narrow window.
    let props_id = ui.id().with(("sample_props_h", slice.is_some()));
    let props_h = ui.data(|d| d.get_temp::<f32>(props_id)).unwrap_or(if slice.is_some() { 140.0 } else { 112.0 });
    // The waveform keeps a useful height; what doesn't fit below it scrolls.
    let avail = ui.available_height();
    let size = Vec2::new(ui.available_width(), (avail - props_h).max((avail * 0.45).min(160.0)).max(80.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, theme::INSET);

    let ruler = Rect::from_min_size(rect.min, Vec2::new(rect.width(), 16.0));
    let loop_bar = Rect::from_min_size(Pos2::new(rect.left(), ruler.bottom()), Vec2::new(rect.width(), 12.0));
    // Slice markers, above the waveform.
    let slice_bar = Rect::from_min_size(Pos2::new(rect.left(), loop_bar.bottom()), Vec2::new(rect.width(), 14.0));
    let scrollbar = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 10.0), rect.max);
    let wave = Rect::from_min_max(Pos2::new(rect.left(), slice_bar.bottom()), Pos2::new(rect.right(), scrollbar.top()));

    // Conversions between screen x and frames for the view `(v0, vlen)`.
    let to_x = |f: f64, (v0, vlen): (f64, f64)| wave.left() + ((f - v0) / vlen) as f32 * wave.width();
    let to_frame =
        |x: f32, (v0, vlen): (f64, f64)| (v0 + ((x - wave.left()) / wave.width()) as f64 * vlen).clamp(0.0, len as f64);

    // Mouse wheel zooms around the pointer; horizontal scrolling pans.
    if resp.hovered() {
        let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
        if let Some(p) = resp.hover_pos() {
            let factor = if zoom != 1.0 { 1.0 / zoom as f64 } else { (-scroll.y as f64 / 200.0).exp() };
            if factor != 1.0 {
                let at = to_frame(p.x, (v0, vlen));
                let new_len = (vlen * factor).clamp(16.0_f64.min(len as f64), len as f64);
                v0 = at - (at - v0) * new_len / vlen;
                vlen = new_len;
            }
        }
        v0 -= scroll.x as f64 / wave.width() as f64 * vlen;
        v0 = v0.clamp(0.0, len as f64 - vlen);
    }

    let loop_on = slot.loop_mode != 0;
    // The slice marker under a point, if any.
    let marker_at = |p: Pos2, view: (f64, f64)| {
        slot.slices
            .iter()
            .enumerate()
            .filter(|(_, f)| (to_x(**f as f64, view) - p.x).abs() < 5.0)
            .map(|(n, _)| n)
            .next()
    };
    if resp.drag_started()
        && let Some(p) = resp.interact_pointer_pos()
    {
        let sv = &mut app.sampler;
        sv.wave_drag = if scrollbar.contains(p) {
            Some(WaveDrag::Scroll)
        } else if let Some(n) = marker_at(p, (v0, vlen)).filter(|_| slice_bar.contains(p)) {
            Some(WaveDrag::Slice(n))
        } else if loop_bar.contains(p) && loop_on {
            let ds = (p.x - to_x(slot.loop_start as f64, (v0, vlen))).abs();
            let de = (p.x - to_x(slot.loop_end as f64, (v0, vlen))).abs();
            Some(if ds <= de { WaveDrag::LoopStart } else { WaveDrag::LoopEnd })
        } else {
            Some(WaveDrag::Select(to_frame(p.x, (v0, vlen)).round() as usize))
        };
    }
    if let (Some(drag), Some(p)) = (app.sampler.wave_drag, resp.interact_pointer_pos())
        && resp.dragged()
    {
        let f = to_frame(p.x, (v0, vlen)).round() as usize;
        match drag {
            WaveDrag::Select(a) => {
                app.sampler.selection = (f != a).then(|| (a.min(f), a.max(f)));
            }
            WaveDrag::Scroll => {
                v0 += resp.drag_delta().x as f64 / scrollbar.width() as f64 * len as f64;
                v0 = v0.clamp(0.0, len as f64 - vlen);
            }
            WaveDrag::Slice(n) => {
                if let Some(s) = slot_mut(app, id, i)
                    && n < s.slices.len()
                {
                    // A marker stays between its neighbours.
                    let lo = if n > 0 { s.slices[n - 1] + 1 } else { 1 };
                    let hi = s.slices.get(n + 1).map_or(len.saturating_sub(1), |m| m - 1);
                    s.slices[n] = f.clamp(lo, hi.max(lo));
                    app.mark();
                }
            }
            WaveDrag::LoopStart | WaveDrag::LoopEnd => {
                if let Some(s) = slot_mut(app, id, i) {
                    if drag == WaveDrag::LoopStart {
                        s.loop_start = f.min(s.loop_end.saturating_sub(1));
                    } else {
                        s.loop_end = f.clamp(s.loop_start + 1, len);
                    }
                    app.mark();
                }
            }
        }
    }
    if resp.drag_stopped() {
        app.sampler.wave_drag = None;
    }
    // In the slice bar: double-click adds a marker, right-click deletes
    // one, and a click selects that slice and plays it.
    let in_slices = resp.interact_pointer_pos().filter(|p| slice_bar.contains(*p));
    if let Some(p) = in_slices {
        let f = to_frame(p.x, (v0, vlen)).round() as usize;
        let marker = marker_at(p, (v0, vlen));
        if resp.double_clicked() && marker.is_none() && f > 0 && f < len {
            if let Some(s) = slot_mut(app, id, i) {
                s.add_slice(f);
            }
            app.mark();
        } else if resp.secondary_clicked()
            && let Some(n) = marker
        {
            if let Some(s) = slot_mut(app, id, i) {
                s.remove_slice(n);
            }
            app.sampler.slice = None;
            app.mark();
        } else if resp.clicked()
            && marker.is_none()
            && let Some((k, &range)) = slot.slice_ranges().iter().enumerate().find(|(_, r)| (r.0..r.1).contains(&f))
        {
            app.sampler.selection = Some(range);
            app.sampler.slice = Some(k);
            play(app, id, i);
        }
    } else if resp.clicked() {
        app.sampler.selection = None;
    }
    app.sampler.view = Some((v0, vlen));

    // Ruler: ticks at a round number of seconds.
    let secs = vlen / data.sample_rate as f64;
    let step = [0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0]
        .into_iter()
        .find(|s| secs / s <= (wave.width() / 70.0) as f64)
        .unwrap_or(60.0);
    let mut t = (v0 / data.sample_rate as f64 / step).ceil() * step;
    while t * (data.sample_rate as f64) < v0 + vlen {
        let x = to_x(t * data.sample_rate as f64, (v0, vlen));
        let tick = [Pos2::new(x, ruler.bottom() - 4.0), Pos2::new(x, ruler.bottom())];
        painter.line_segment(tick, (1.0, theme::TEXT_WEAK));
        let decimals = if step < 0.01 {
            3
        } else if step < 1.0 {
            2
        } else {
            0
        };
        let label = format!("{t:.decimals$}");
        let font = FontId::monospace(10.0);
        painter.text(Pos2::new(x + 2.0, ruler.top()), Align2::LEFT_TOP, label, font, theme::TEXT_WEAK);
        t += step;
    }

    // Selection and loop range behind the waveform.
    if let Some((a, b)) = app.sampler.selection {
        let r = Rect::from_x_y_ranges(to_x(a as f64, (v0, vlen))..=to_x(b as f64, (v0, vlen)), wave.y_range());
        painter.rect_filled(r, 0.0, theme::SELECTED.gamma_multiply(0.25));
    }
    if loop_on {
        let (ls, le) = (to_x(slot.loop_start as f64, (v0, vlen)), to_x(slot.loop_end as f64, (v0, vlen)));
        let r = Rect::from_x_y_ranges(ls..=le, loop_bar.y_range());
        painter.rect_filled(r, 0.0, theme::SCOPE.gamma_multiply(0.35));
        for x in [ls, le] {
            painter.line_segment([Pos2::new(x, loop_bar.top()), Pos2::new(x, wave.bottom())], (1.0, theme::SCOPE));
        }
        let tri = |x: f32, dir: f32| {
            let y = loop_bar.top();
            vec![Pos2::new(x, y), Pos2::new(x + 8.0 * dir, y), Pos2::new(x, loop_bar.bottom())]
        };
        painter.add(egui::Shape::convex_polygon(tri(ls, 1.0), theme::SCOPE, Stroke::NONE));
        painter.add(egui::Shape::convex_polygon(tri(le, -1.0), theme::SCOPE, Stroke::NONE));
        painter.text(
            Pos2::new((ls + le) / 2.0, loop_bar.center().y),
            Align2::CENTER_CENTER,
            LOOP_MODES[slot.loop_mode as usize % LOOP_MODES.len()],
            FontId::proportional(9.5),
            theme::SELECTED_TEXT,
        );
    }

    // Slices: shaded in turn along the bar, each with the note it plays
    // on, and a line down the waveform at each marker.
    painter.rect_filled(slice_bar, 0.0, Color32::from_gray(22));
    if slot.slices.is_empty() {
        let hint = "double-click here to add a slice marker";
        painter.text(
            slice_bar.left_center() + Vec2::new(4.0, 0.0),
            Align2::LEFT_CENTER,
            hint,
            FontId::proportional(9.5),
            Color32::from_gray(80),
        );
    }
    for (n, (a, b)) in slot.slice_ranges().into_iter().enumerate() {
        let (xa, xb) = (to_x(a as f64, (v0, vlen)), to_x(b as f64, (v0, vlen)));
        if xb < slice_bar.left() || xa > slice_bar.right() {
            continue;
        }
        let r = Rect::from_x_y_ranges(xa..=xb, slice_bar.y_range());
        let shade =
            if n % 2 == 0 { theme::PAT_EFFECT.gamma_multiply(0.22) } else { theme::PAT_EFFECT.gamma_multiply(0.12) };
        painter.rect_filled(r, 0.0, shade);
        // The number and note where they fit, the number alone, or nothing.
        let label = match xb - xa {
            w if w > 52.0 => format!("{:02} {}", n + 1, Note::On(slot.slice_note(n)).label()),
            w if w > 20.0 => format!("{:02}", n + 1),
            _ => continue,
        };
        painter.with_clip_rect(r.intersect(slice_bar)).text(
            Pos2::new(xa.max(slice_bar.left()) + 3.0, slice_bar.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::monospace(9.5),
            theme::PAT_EFFECT,
        );
    }
    for &m in &slot.slices {
        let x = to_x(m as f64, (v0, vlen));
        painter.line_segment(
            [Pos2::new(x, slice_bar.top()), Pos2::new(x, wave.bottom())],
            (1.0, theme::PAT_EFFECT.gamma_multiply(0.8)),
        );
        let flag = vec![
            Pos2::new(x, slice_bar.top()),
            Pos2::new(x + 6.0, slice_bar.top()),
            Pos2::new(x, slice_bar.top() + 7.0),
        ];
        painter.add(egui::Shape::convex_polygon(flag, theme::PAT_EFFECT, Stroke::NONE));
    }

    if app.sampler.peaks.as_ref().is_none_or(|p| p.0 != key.2) {
        app.sampler.peaks = Some((key.2, overview(&data.frames)));
    }
    let peaks = &app.sampler.peaks.as_ref().unwrap().1;

    // One lane per channel.
    let lanes = if data.channels >= 2 { 2 } else { 1 };
    let lane_h = wave.height() / lanes as f32;
    for ch in 0..lanes {
        let top = wave.top() + lane_h * ch as f32;
        let mid = top + lane_h / 2.0;
        let half = lane_h * 0.45;
        let across = |y: f32| [Pos2::new(wave.left(), y), Pos2::new(wave.right(), y)];
        painter.line_segment(across(mid), (1.0, Color32::from_gray(40)));
        if ch == 1 {
            painter.line_segment(across(top), (1.0, Color32::from_gray(50)));
        }
        draw_channel(&painter, &data.frames, peaks, ch, wave, v0, vlen, mid, half);
    }

    // Where the sample is playing.
    let mut heads: Vec<(f64, f32)> =
        app.playheads().iter().filter(|p| p.module == id && p.slot == i).map(|p| (p.pos, p.level)).collect();
    if let (Some(pos), Some(super::Preview { source: Some((m, s, offset)), .. })) = (app.preview_pos(), &app.previewing)
        && (*m, *s) == (id, i)
    {
        heads.push((*offset as f64 + pos, 1.0));
    }
    for (pos, level) in heads {
        let x = to_x(pos, (v0, vlen));
        if wave.x_range().contains(x) {
            let color = theme::SELECTED.gamma_multiply(0.4 + 0.6 * level.min(1.0));
            painter.line_segment([Pos2::new(x, ruler.top()), Pos2::new(x, wave.bottom())], (1.5, color));
        }
    }

    // Scrollbar.
    painter.rect_filled(scrollbar, 0.0, Color32::from_gray(24));
    let thumb = Rect::from_x_y_ranges(
        scrollbar.left() + (v0 / len as f64) as f32 * scrollbar.width()
            ..=scrollbar.left() + ((v0 + vlen) / len as f64) as f32 * scrollbar.width(),
        scrollbar.y_range(),
    );
    painter.rect_filled(thumb.shrink2(Vec2::new(0.0, 2.0)), 2.0, Color32::from_gray(90));

    // When even the smallest waveform leaves too little room, they scroll.
    let height = egui::ScrollArea::vertical()
        .id_salt("sample_props")
        .auto_shrink([false, true])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
        .show(ui, |ui| {
            let top = ui.cursor().min.y;
            if let Some(k) = slice {
                slice_properties(app, ui, id, i, k, &slot);
            }
            properties(app, ui, id, i, &slot);
            ui.cursor().min.y - top
        })
        .inner;
    ui.data_mut(|d| d.insert_temp(props_id, height + ui.spacing().item_spacing.y));
}

/// The settings of slice `k` of slot `i`, in a row.
fn slice_properties(app: &mut App, ui: &mut egui::Ui, id: u8, i: usize, k: usize, slot: &SampleSlot) {
    let mut st = slot.slice(k);
    let before = st.clone();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let title = format!("SLICE {:02} · {}", k + 1, Note::On(slot.slice_note(k)).label());
        ui.label(RichText::new(title).small().color(theme::PAT_EFFECT));
        ui.label("Volume");
        let mut db = 20.0 * st.volume.max(1e-4).log10();
        if ui.add(egui::DragValue::new(&mut db).range(-60.0..=12.0).speed(0.2).suffix(" dB").max_decimals(1)).changed()
        {
            st.volume = if db <= -60.0 { 0.0 } else { 10f32.powf(db / 20.0) };
        }
        ui.label("Pan");
        ui.add(egui::DragValue::new(&mut st.panning).range(-1.0..=1.0).speed(0.01).max_decimals(2));
        ui.label("Transpose");
        ui.add(egui::DragValue::new(&mut st.transpose).range(-48..=48).suffix(" st"));
        ui.label("Finetune");
        ui.add(egui::DragValue::new(&mut st.finetune).range(-100..=100).suffix(" ct"));
        ui.label("Loop");
        egui::ComboBox::from_id_salt("slice_loop")
            .selected_text(LOOP_MODES[st.loop_mode as usize % LOOP_MODES.len()])
            .width(80.0)
            .show_ui(ui, |ui| {
                for (m, label) in LOOP_MODES.iter().enumerate() {
                    ui.selectable_value(&mut st.loop_mode, m as u8, *label);
                }
            });
        ui.checkbox(&mut st.oneshot, "One-shot");
    });
    if st != before
        && let Some(s) = slot_mut(app, id, i)
    {
        *s.slice_mut(k) = st;
        app.mark();
    }
}

/// Draws channel `ch` of the frames `v0..v0 + vlen` across `wave`, centered
/// on `mid` and scaled to `half` pixels per unit.
#[allow(clippy::too_many_arguments)]
pub fn draw_channel(
    painter: &egui::Painter,
    frames: &[Frame],
    peaks: &[Peak],
    ch: usize,
    wave: Rect,
    v0: f64,
    vlen: f64,
    mid: f32,
    half: f32,
) {
    let cols = wave.width().max(1.0) as usize;
    let per_px = vlen / cols as f64;
    let color = Color32::from_rgb(150, 220, 120);
    if per_px < 1.0 {
        // Zoomed in far enough to draw the samples as a line.
        let a = v0.floor() as usize;
        let b = ((v0 + vlen).ceil() as usize + 1).min(frames.len());
        let pts: Vec<Pos2> = (a..b)
            .map(|f| {
                let x = wave.left() + ((f as f64 - v0) / vlen) as f32 * wave.width();
                Pos2::new(x, mid - frames[f][ch].clamp(-1.0, 1.0) * half)
            })
            .collect();
        painter.line(pts, Stroke::new(1.0, color));
        return;
    }
    // Zoomed far out, read the overview instead of every frame.
    let coarse = per_px >= PEAK_BLOCK as f64 * 2.0;
    for x in 0..cols {
        let a = (v0 + x as f64 * per_px) as usize;
        let b = ((v0 + (x + 1) as f64 * per_px) as usize).max(a + 1).min(frames.len());
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        if coarse {
            let (pa, pb) = (a / PEAK_BLOCK, b.div_ceil(PEAK_BLOCK).min(peaks.len()));
            for p in &peaks[pa.min(pb)..pb] {
                lo = lo.min(p[ch][0]);
                hi = hi.max(p[ch][1]);
            }
        } else {
            for f in &frames[a.min(b)..b] {
                lo = lo.min(f[ch]);
                hi = hi.max(f[ch]);
            }
        }
        if lo > hi {
            continue;
        }
        let px = wave.left() + x as f32 + 0.5;
        painter.line_segment(
            [Pos2::new(px, mid - hi.clamp(-1.0, 1.0) * half), Pos2::new(px, mid - lo.clamp(-1.0, 1.0) * half + 0.5)],
            Stroke::new(1.0, color),
        );
    }
}

/// Buttons above the waveform: playback, edits and zoom.
fn edit_toolbar(app: &mut App, ui: &mut egui::Ui, id: u8, i: usize, len: usize, v0: &mut f64, vlen: &mut f64) {
    let sel = app.sampler.selection;
    ui.horizontal_wrapped(|ui| {
        if super::icons::button(ui, super::icons::Icon::Play)
            .on_hover_text("Play the selection, or the whole sample")
            .clicked()
        {
            play(app, id, i);
        }
        if super::icons::button(ui, super::icons::Icon::Stop).on_hover_text("Stop").clicked() {
            app.stop_preview();
        }
        ui.separator();
        let ops: [(&str, &str, Op); 5] = [
            ("Reverse", "Play backwards", Op::Reverse),
            ("Normalize", "Raise the level so the loudest peak is at 0 dB", Op::Normalize),
            ("Fade in", "Fade in from silence", Op::FadeIn),
            ("Fade out", "Fade out to silence", Op::FadeOut),
            ("Silence", "Replace with silence", Op::Silence),
        ];
        for (label, tip, op) in ops {
            if ui.button(label).on_hover_text(format!("{tip} (the selection, or the whole sample)")).clicked() {
                edit(app, id, i, op);
            }
        }
        ui.menu_button("Slices", |ui| slices_menu(app, ui, id, i));
        ui.menu_button("Process", |ui| {
            for (label, tip, op) in PROCESSES {
                let tip = if op == Op::CrossfadeLoop {
                    tip.to_string()
                } else {
                    format!("{tip} (the selection, or the whole sample)")
                };
                if ui.button(label).on_hover_text(tip).clicked() {
                    edit(app, id, i, op);
                    ui.close();
                }
            }
        });
        ui.separator();
        if ui.add_enabled(sel.is_some(), egui::Button::new("Crop")).on_hover_text("Keep only the selection").clicked() {
            edit(app, id, i, Op::Crop);
        }
        if ui.add_enabled(sel.is_some(), egui::Button::new("Delete")).on_hover_text("Del").clicked() {
            edit(app, id, i, Op::Delete);
        }
        if ui.add_enabled(sel.is_some(), egui::Button::new("Cut")).on_hover_text("Ctrl+X").clicked() {
            edit(app, id, i, Op::Cut);
        }
        if ui.add_enabled(sel.is_some(), egui::Button::new("Copy")).on_hover_text("Ctrl+C").clicked() {
            edit(app, id, i, Op::Copy);
        }
        let paste_tip = "Ctrl+V: replace the selection, or insert at the start";
        let paste = ui.add_enabled(app.sampler.clipboard.is_some(), egui::Button::new("Paste"));
        if paste.on_hover_text(paste_tip).clicked() {
            edit(app, id, i, Op::Paste);
        }
        ui.separator();
        if ui
            .add_enabled(sel.is_some(), egui::Button::new("Loop selection"))
            .on_hover_text("Set the loop to the selection")
            .clicked()
        {
            let (a, b) = sel.unwrap();
            if let Some(s) = slot_mut(app, id, i) {
                s.loop_start = a;
                s.loop_end = b;
                if s.loop_mode == 0 {
                    s.loop_mode = 1;
                }
                app.mark();
            }
        }
        ui.separator();
        if ui.button("Show all").clicked() {
            *v0 = 0.0;
            *vlen = len as f64;
        }
        if ui.add_enabled(sel.is_some(), egui::Button::new("Zoom to selection")).clicked() {
            let (a, b) = sel.unwrap();
            *v0 = a as f64;
            *vlen = ((b - a) as f64).max(16.0);
        }
    });
}

/// Delete, cut, copy and paste from the keyboard while the waveform is shown.
fn keyboard_shortcuts(app: &mut App, ui: &mut egui::Ui, id: u8, i: usize) {
    if ui.ctx().text_edit_focused() {
        return;
    }
    let events = ui.input(|inp| inp.events.clone());
    for e in events {
        let op = match e {
            egui::Event::Copy => Op::Copy,
            egui::Event::Cut => Op::Cut,
            egui::Event::Paste(_) => Op::Paste,
            egui::Event::Key { key: egui::Key::Delete, pressed: true, .. } if app.sampler.selection.is_some() => {
                Op::Delete
            }
            _ => continue,
        };
        edit(app, id, i, op);
    }
}

fn play(app: &mut App, id: u8, i: usize) {
    let Some(data) = slots(app, id).get(i).and_then(|s| s.data.clone()) else { return };
    let (sample, start) = match app.sampler.selection {
        Some((a, b)) => (Arc::new(data.with_frames(data.frames[a..b].to_vec())), a),
        None => (data, 0),
    };
    app.preview(sample, Some((id, i, start)));
}

#[derive(Clone, Copy, PartialEq)]
enum Op {
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
}

/// The processes in the Process menu.
const PROCESSES: [(&str, &str, Op); 7] = [
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

/// The Slices menu: markers made evenly or at the beats, cleared, or
/// turned into samples of their own.
fn slices_menu(app: &mut App, ui: &mut egui::Ui, id: u8, i: usize) {
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
fn render_slices(app: &mut App, id: u8, i: usize) {
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
fn process(frames: &mut [Frame], op: Op) {
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
fn crossfade_loop(frames: &mut [Frame], start: usize, end: usize) -> bool {
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
fn edit(app: &mut App, id: u8, i: usize, op: Op) {
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

/// The sample's settings, below the waveform.
fn properties(app: &mut App, ui: &mut egui::Ui, id: u8, i: usize, slot: &SampleSlot) {
    let mut s = slot.clone();
    let mut changed = false;
    ui.add_space(4.0);
    // The four groups fill as many rows as the width needs, each placed by
    // its width as last drawn.
    const GROUPS: [fn(&mut egui::Ui, &mut SampleSlot) -> bool; 4] =
        [sample_general, sample_pitch, sample_loop, sample_zone];
    let width_id = |k: usize| egui::Id::new(("sample_props_w", k));
    let widths: Vec<f32> = (0..GROUPS.len()).map(|k| ui.data(|d| d.get_temp(width_id(k))).unwrap_or(0.0)).collect();
    let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
    let mut x = 0.0;
    for (k, w) in widths.iter().enumerate() {
        let row = rows.last_mut().unwrap();
        if !row.is_empty() && x + w + 12.0 > ui.available_width() {
            rows.push(Vec::new());
            x = 0.0;
        }
        rows.last_mut().unwrap().push(k);
        x += w + 12.0;
    }
    for row in rows {
        // Separators are drawn once the row's height is known: a vertical
        // separator widget would take all the height there is.
        let mut gaps = Vec::new();
        let r = ui.horizontal_top(|ui| {
            for (j, &k) in row.iter().enumerate() {
                if j > 0 {
                    gaps.push(ui.cursor().min.x + 6.0);
                    ui.add_space(12.0);
                }
                let r = ui.scope(|ui| GROUPS[k](ui, &mut s));
                changed |= r.inner;
                ui.data_mut(|d| d.insert_temp(width_id(k), r.response.rect.width()));
            }
        });
        let y = r.response.rect.y_range();
        for x in gaps {
            ui.painter().vline(x, y, ui.visuals().widgets.noninteractive.bg_stroke);
        }
    }
    if changed {
        let name_only = s.name != slot.name;
        *slot_mut(app, id, i).unwrap() = s;
        if name_only { app.mark_layout() } else { app.mark() }
    }
}

/// The sample's name, length, volume and panning.
fn sample_general(ui: &mut egui::Ui, s: &mut SampleSlot) -> bool {
    let len = s.len();
    let rate = s.data.as_ref().map_or(44100.0, |d| d.sample_rate);
    let channels = s.data.as_ref().map_or(1, |d| d.channels);
    let mut changed = false;
    egui::Grid::new("sample_props_1").num_columns(2).spacing([6.0, 3.0]).show(ui, |ui| {
        ui.label("Name");
        changed |= ui.add(egui::TextEdit::singleline(&mut s.name).desired_width(150.0)).changed();
        ui.end_row();
        ui.label("");
        let ch = if channels >= 2 { "stereo" } else { "mono" };
        ui.label(
            RichText::new(format!("{:.3} s · {} Hz · {ch}", len as f32 / rate, rate as u32))
                .small()
                .color(theme::TEXT_WEAK),
        );
        ui.end_row();
        ui.label("Volume");
        let mut db = 20.0 * s.volume.max(1e-4).log10();
        let r = ui.add(egui::DragValue::new(&mut db).range(-60.0..=12.0).speed(0.2).suffix(" dB").max_decimals(1));
        if r.changed() {
            s.volume = if db <= -60.0 { 0.0 } else { 10f32.powf(db / 20.0) };
            changed = true;
        }
        ui.end_row();
        ui.label("Panning");
        changed |= ui.add(egui::Slider::new(&mut s.panning, -1.0..=1.0).show_value(true)).changed();
        ui.end_row();
    });
    changed
}

/// Base note, transpose, finetune and beat sync.
fn sample_pitch(ui: &mut egui::Ui, s: &mut SampleSlot) -> bool {
    let mut changed = false;
    egui::Grid::new("sample_props_2").num_columns(2).spacing([6.0, 3.0]).show(ui, |ui| {
        ui.label("Base note");
        let mut base = s.base_note as i32;
        let r =
            ui.add(egui::DragValue::new(&mut base).range(0..=119).custom_formatter(|v, _| Note::On(v as u8).label()));
        if r.changed() {
            s.base_note = base as u8;
            changed = true;
        }
        ui.end_row();
        ui.label("Transpose");
        changed |= ui.add(egui::DragValue::new(&mut s.transpose).range(-120..=120).suffix(" st")).changed();
        ui.end_row();
        ui.label("Finetune");
        changed |= ui.add(egui::DragValue::new(&mut s.finetune).range(-100..=100).suffix(" ct")).changed();
        ui.end_row();
        ui.label("Beat sync");
        let lines = |v: f64, _| if v == 0.0 { "Off".to_string() } else { format!("{v} lines") };
        let sync = egui::DragValue::new(&mut s.beat_sync).range(0..=512).custom_formatter(lines);
        let tip = "Play the whole sample in this many lines at the song's tempo";
        changed |= ui.add(sync).on_hover_text(tip).changed();
        ui.end_row();
    });
    changed
}

/// The loop and one-shot.
fn sample_loop(ui: &mut egui::Ui, s: &mut SampleSlot) -> bool {
    let len = s.len();
    let mut changed = false;
    egui::Grid::new("sample_props_3").num_columns(2).spacing([6.0, 3.0]).show(ui, |ui| {
        ui.label("Loop");
        ui.horizontal(|ui| {
            for (m, label) in LOOP_MODES.iter().enumerate() {
                if theme::toggle(ui, s.loop_mode as usize == m, *label).clicked() {
                    s.loop_mode = m as u8;
                    changed = true;
                }
            }
        });
        ui.end_row();
        ui.label("Loop start");
        let end = s.loop_end;
        changed |= ui.add(egui::DragValue::new(&mut s.loop_start).range(0..=end.saturating_sub(1))).changed();
        ui.end_row();
        ui.label("Loop end");
        let start = s.loop_start;
        changed |= ui.add(egui::DragValue::new(&mut s.loop_end).range(start + 1..=len)).changed();
        ui.end_row();
        ui.label("");
        let oneshot = ui.checkbox(&mut s.oneshot, "One-shot");
        changed |= oneshot.on_hover_text("Note-offs don't stop the sample").changed();
        ui.end_row();
        ui.label("");
        let tip = "Starting the song partway through plays the sample from where it would be by then";
        changed |= ui.checkbox(&mut s.autoseek, "Autoseek").on_hover_text(tip).changed();
        ui.end_row();
    });
    changed
}

/// The keys and velocities it plays on, and its mute group.
fn sample_zone(ui: &mut egui::Ui, s: &mut SampleSlot) -> bool {
    let mut changed = false;
    egui::Grid::new("sample_props_4").num_columns(2).spacing([6.0, 3.0]).show(ui, |ui| {
        ui.label("Keys");
        ui.horizontal(|ui| {
            let note = |v: f64, _| Note::On(v as u8).label();
            let hi = s.keys[1];
            changed |= ui.add(egui::DragValue::new(&mut s.keys[0]).range(0..=hi).custom_formatter(note)).changed();
            let lo = s.keys[0];
            changed |= ui.add(egui::DragValue::new(&mut s.keys[1]).range(lo..=119).custom_formatter(note)).changed();
        });
        ui.end_row();
        ui.label("Velocity");
        ui.horizontal(|ui| {
            let hex = |v: f64, _| format!("{:02X}", v as u8);
            let hi = s.velocities[1];
            changed |= ui.add(egui::DragValue::new(&mut s.velocities[0]).range(0..=hi).custom_formatter(hex)).changed();
            let lo = s.velocities[0];
            changed |=
                ui.add(egui::DragValue::new(&mut s.velocities[1]).range(lo..=127).custom_formatter(hex)).changed();
        });
        ui.end_row();
        ui.label("Mute group");
        let group = |v: f64, _| if v == 0.0 { "None".to_string() } else { format!("{v}") };
        let field = egui::DragValue::new(&mut s.mute_group).range(0..=15).custom_formatter(group);
        let tip = "A sample stops the others in its group, as a closed hi-hat cuts an open one";
        changed |= ui.add(field).on_hover_text(tip).changed();
        ui.end_row();
    });
    changed
}

// ---------------------------------------------------------------- keyzones

/// The notes a sample plays on: its keyzone, or for a sliced sample its
/// base note and the slices' notes after it.
fn shown_keys(s: &SampleSlot) -> [u8; 2] {
    match s.slice_ranges().len() {
        0 => s.keys,
        n => [s.base_note, s.slice_note(n - 1)],
    }
}

fn keyzones_tab(app: &mut App, ui: &mut egui::Ui, id: u8) {
    ui.horizontal(|ui| {
        let n = slots(app, id).len();
        if ui.button("Layer all").on_hover_text("Every sample on every key").clicked() {
            for s in &mut app.project.module_mut(id).unwrap().samples {
                s.keys = [0, 119];
            }
            app.mark();
        }
        if ui.button("Drum kit").on_hover_text("One key each, from C-4 up, at original pitch").clicked() {
            drum_kit(&mut app.project.module_mut(id).unwrap().samples);
            app.mark();
        }
        if ui.button("Spread").on_hover_text("Split the keyboard evenly between the samples").clicked() && n > 0 {
            for (k, s) in app.project.module_mut(id).unwrap().samples.iter_mut().enumerate() {
                s.keys = [(k * 120 / n) as u8, ((k + 1) * 120 / n - 1) as u8];
            }
            app.mark();
        }
        ui.label(
            RichText::new("Drag a zone to move it, or its edges to resize it. Click a key to play it.")
                .small()
                .color(theme::TEXT_WEAK),
        );
    });

    let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, theme::INSET);
    let piano_h = 46.0;
    let grid = Rect::from_min_max(rect.min, Pos2::new(rect.right(), rect.bottom() - piano_h));
    let piano = Rect::from_min_max(Pos2::new(rect.left(), grid.bottom()), rect.max);
    let key_w = rect.width() / 120.0;
    let key_x = |k: f32| rect.left() + k * key_w;
    let vel_y = |v: f32| grid.bottom() - v / 128.0 * grid.height();

    // Octave lines.
    for o in 0..=10 {
        let x = key_x(o as f32 * 12.0);
        painter.line_segment([Pos2::new(x, grid.top()), Pos2::new(x, piano.bottom())], (1.0, Color32::from_gray(38)));
    }

    let zone_rect = |s: &SampleSlot| {
        let keys = shown_keys(s);
        Rect::from_min_max(
            Pos2::new(key_x(keys[0] as f32), vel_y(s.velocities[1] as f32 + 1.0)),
            Pos2::new(key_x(keys[1] as f32 + 1.0), vel_y(s.velocities[0] as f32)),
        )
    };
    let samples = slots(app, id).to_vec();
    let sel = app.sampler.sample;
    // Draw the selected zone last so it is on top.
    let mut order: Vec<usize> = (0..samples.len()).filter(|&k| k != sel).collect();
    if sel < samples.len() {
        order.push(sel);
    }
    for &k in &order {
        let s = &samples[k];
        let r = zone_rect(s);
        let color = if k == sel { theme::SELECTED } else { theme::track_color(k + 3) };
        painter.rect_filled(r, 2.0, color.gamma_multiply(if k == sel { 0.55 } else { 0.3 }));
        painter.rect_stroke(r, 2.0, Stroke::new(1.0, color), egui::StrokeKind::Inside);
        // Narrow zones only have room for the number, or nothing.
        let label = match r.width() {
            w if w > 50.0 => format!("{k:02} {}", s.name),
            w if w > 18.0 => format!("{k:02}"),
            _ => String::new(),
        };
        painter.with_clip_rect(r.intersect(grid)).text(
            r.left_top() + Vec2::new(2.0, 2.0),
            Align2::LEFT_TOP,
            label,
            FontId::monospace(10.0),
            Color32::WHITE,
        );
        // A sliced sample: the whole on its base note, then a key a slice.
        for n in 0..s.slice_ranges().len() {
            let x = key_x(s.slice_note(n) as f32);
            painter.line_segment([Pos2::new(x, r.top()), Pos2::new(x, r.bottom())], (1.0, theme::PAT_EFFECT));
            if key_w > 9.0 {
                let at = Pos2::new(x + key_w / 2.0, r.bottom() - 3.0);
                painter.text(
                    at,
                    Align2::CENTER_BOTTOM,
                    format!("{}", n + 1),
                    FontId::monospace(8.5),
                    theme::PAT_EFFECT,
                );
            }
        }
    }

    // Piano: one column per note, with black keys over the top part.
    painter.rect_filled(piano, 0.0, Color32::from_gray(200));
    let black_bottom = piano.top() + piano.height() * 0.6;
    let line = Stroke::new(1.0, Color32::from_gray(110));
    for k in 0..120u8 {
        let black = matches!(k % 12, 1 | 3 | 6 | 8 | 10);
        let (x0, x1) = (key_x(k as f32), key_x(k as f32 + 1.0));
        let held = app.sampler.held.is_some_and(|(h, _)| h == k);
        if black {
            let r = Rect::from_min_max(Pos2::new(x0, piano.top()), Pos2::new(x1, black_bottom));
            painter.rect_filled(r, 0.0, if held { theme::SELECTED } else { Color32::from_gray(28) });
            // The edge between the white keys on either side.
            let mid = (x0 + x1) / 2.0;
            painter.line_segment([Pos2::new(mid, black_bottom), Pos2::new(mid, piano.bottom())], line);
        } else {
            if held {
                let r = Rect::from_min_max(Pos2::new(x0, piano.top()), Pos2::new(x1, piano.bottom()));
                painter.rect_filled(r, 0.0, theme::SELECTED);
            }
            if matches!(k % 12, 0 | 5) {
                painter.line_segment([Pos2::new(x0, piano.top()), Pos2::new(x0, piano.bottom())], line);
            }
        }
        if k % 12 == 0 {
            painter.text(
                Pos2::new(x0 + 1.0, piano.bottom() - 2.0),
                Align2::LEFT_BOTTOM,
                format!("C{}", k / 12),
                FontId::proportional(9.0),
                Color32::from_gray(40),
            );
        }
    }
    if let Some(s) = samples.get(sel) {
        // Mark the selected sample's base note.
        let x = key_x(s.base_note as f32 + 0.5);
        let y = piano.top();
        painter.add(egui::Shape::convex_polygon(
            vec![Pos2::new(x - 4.0, y), Pos2::new(x + 4.0, y), Pos2::new(x, y + 6.0)],
            theme::RECORD,
            Stroke::NONE,
        ));
    }

    // Interaction.
    let note_at = |x: f32| ((x - rect.left()) / key_w).floor().clamp(0.0, 119.0) as i32;
    let vel_at = |y: f32| ((grid.bottom() - y) / grid.height() * 128.0).floor().clamp(0.0, 127.0) as i32;
    if (resp.drag_started() || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        if piano.contains(p) {
            let note = note_at(p.x) as u8;
            key_down(app, id, note);
        } else {
            let hit = order.iter().rev().copied().find(|&k| zone_rect(&samples[k]).expand(3.0).contains(p));
            if let Some(k) = hit {
                app.sampler.sample = k;
                app.sampler.slice = None;
                let r = zone_rect(&samples[k]);
                let edge = 5.0;
                let mode = if (p.x - r.left()).abs() < edge {
                    ZoneDrag::Low
                } else if (p.x - r.right()).abs() < edge {
                    ZoneDrag::High
                } else if (p.y - r.top()).abs() < edge {
                    ZoneDrag::VelHigh
                } else if (p.y - r.bottom()).abs() < edge {
                    ZoneDrag::VelLow
                } else {
                    ZoneDrag::Move { grab: note_at(p.x) - shown_keys(&samples[k])[0] as i32 }
                };
                app.sampler.zone_drag = Some((k, mode));
            }
        }
    }
    if let (Some((k, mode)), Some(p), true) = (app.sampler.zone_drag, resp.interact_pointer_pos(), resp.dragged()) {
        let (note, vel) = (note_at(p.x), vel_at(p.y));
        if let Some(s) = slot_mut(app, id, k) {
            let [lo, hi] = shown_keys(s).map(i32::from);
            let [vlo, vhi] = s.velocities.map(i32::from);
            let base = s.base_note;
            match mode {
                // A sliced sample moves with its slices, by its base note.
                ZoneDrag::Move { grab } if !s.slices.is_empty() => {
                    s.base_note = (note - grab).clamp(0, 119 - (hi - lo)) as u8;
                }
                ZoneDrag::Low | ZoneDrag::High if !s.slices.is_empty() => {}
                ZoneDrag::Move { grab } => {
                    let w = hi - lo;
                    let new_lo = (note - grab).clamp(0, 119 - w);
                    s.keys = [new_lo as u8, (new_lo + w) as u8];
                }
                ZoneDrag::Low => s.keys[0] = note.min(hi) as u8,
                ZoneDrag::High => s.keys[1] = note.max(lo) as u8,
                ZoneDrag::VelLow => s.velocities[0] = vel.min(vhi) as u8,
                ZoneDrag::VelHigh => s.velocities[1] = vel.max(vlo) as u8,
            }
            if shown_keys(s).map(i32::from) != [lo, hi]
                || s.velocities.map(i32::from) != [vlo, vhi]
                || s.base_note != base
            {
                app.mark();
            }
        }
    }
    if let Some(p) = resp.hover_pos() {
        let icon = if piano.contains(p) { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Default };
        ui.ctx().set_cursor_icon(icon);
        let mut text = format!("{}  vel {:02X}", Note::On(note_at(p.x) as u8).label(), vel_at(p.y));
        for &k in order.iter().rev().filter(|&&k| grid.contains(p) && zone_rect(&samples[k]).contains(p)) {
            text.push_str(&format!("\n{k:02} {}", samples[k].name));
        }
        resp.clone().on_hover_text_at_pointer(text);
    }
    if !ui.input(|i| i.pointer.any_down()) {
        app.sampler.zone_drag = None;
        if let Some((note, module)) = app.sampler.held.take() {
            app.send(Cmd::NoteOff { module, key: LIVE_KEY + 500 + note as u32 });
        }
    }
}

fn key_down(app: &mut App, id: u8, note: u8) {
    if let Some((old, module)) = app.sampler.held.take() {
        app.send(Cmd::NoteOff { module, key: LIVE_KEY + 500 + old as u32 });
    }
    app.sampler.held = Some((note, id));
    app.send(Cmd::NoteOn { module: id, key: LIVE_KEY + 500 + note as u32, note, vel: 1.0 });
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
mod tests {
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
}
