//! The keyzone map: the samples across the keyboard and velocity, to drag,
//! and a keyboard to play them.

use super::*;

/// The notes a sample plays on: its keyzone, or for a sliced sample its
/// base note and the slices' notes after it.
pub(super) fn shown_keys(s: &SampleSlot) -> [u8; 2] {
    match s.slice_ranges().len() {
        0 => s.keys,
        n => [s.base_note, s.slice_note(n - 1)],
    }
}

pub(super) fn keyzones_tab(app: &mut App, ui: &mut egui::Ui, id: u8) {
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

pub(super) fn key_down(app: &mut App, id: u8, note: u8) {
    if let Some((old, module)) = app.sampler.held.take() {
        app.send(Cmd::NoteOff { module, key: LIVE_KEY + 500 + old as u32 });
    }
    app.sampler.held = Some((note, id));
    app.send(Cmd::NoteOn { module: id, key: LIVE_KEY + 500 + note as u32, note, vel: 1.0 });
}
