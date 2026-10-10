//! The waveform tab: the selected sample drawn with its loop and slice
//! markers, its edit toolbar and keys, and its settings below.

use super::*;

pub(super) fn waveform_tab(app: &mut App, ui: &mut egui::Ui, id: u8) {
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
    if let (Some(pos), Some(Preview { source: Some((m, s, offset)), .. })) = (app.preview_pos(), &app.previewing)
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
pub(super) fn slice_properties(app: &mut App, ui: &mut egui::Ui, id: u8, i: usize, k: usize, slot: &SampleSlot) {
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
pub(super) fn edit_toolbar(
    app: &mut App,
    ui: &mut egui::Ui,
    id: u8,
    i: usize,
    len: usize,
    v0: &mut f64,
    vlen: &mut f64,
) {
    let sel = app.sampler.selection;
    ui.horizontal_wrapped(|ui| {
        if icons::button(ui, icons::Icon::Play).on_hover_text("Play the selection, or the whole sample").clicked() {
            play(app, id, i);
        }
        if icons::button(ui, icons::Icon::Stop).on_hover_text("Stop").clicked() {
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
            ui.separator();
            stretch_menu(app, ui, id, i, sel.map_or(len, |(a, b)| b - a));
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
pub(super) fn keyboard_shortcuts(app: &mut App, ui: &mut egui::Ui, id: u8, i: usize) {
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

pub(super) fn play(app: &mut App, id: u8, i: usize) {
    let Some(data) = slots(app, id).get(i).and_then(|s| s.data.clone()) else { return };
    let (sample, start) = match app.sampler.selection {
        Some((a, b)) => (Arc::new(data.with_frames(data.frames[a..b].to_vec())), a),
        None => (data, 0),
    };
    app.preview(sample, Some((id, i, start)));
}

/// The sample's settings, below the waveform.
pub(super) fn properties(app: &mut App, ui: &mut egui::Ui, id: u8, i: usize, slot: &SampleSlot) {
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
pub(super) fn sample_general(ui: &mut egui::Ui, s: &mut SampleSlot) -> bool {
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
pub(super) fn sample_pitch(ui: &mut egui::Ui, s: &mut SampleSlot) -> bool {
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
pub(super) fn sample_loop(ui: &mut egui::Ui, s: &mut SampleSlot) -> bool {
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
pub(super) fn sample_zone(ui: &mut egui::Ui, s: &mut SampleSlot) -> bool {
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
