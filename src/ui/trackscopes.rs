//! Track scopes, in the upper frame: a small scope
//! for each track of the pattern, drawn in the track's color. They show
//! what each track's notes play, before the effects they go through.

use super::{App, pattern, theme};
use crate::engine::TRACK_SCOPE_LEN;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2};

pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    let scopes = app.track_scopes();
    let tracks = app.pattern().num_tracks();
    let area = ui.available_rect_before_wrap();
    ui.allocate_rect(area, Sense::hover());
    // One row up to eight tracks, then more rows.
    let rows = tracks.div_ceil(8).max(1);
    let cols = tracks.div_ceil(rows).max(1);
    let size = Vec2::new(area.width() / cols as f32, area.height() / rows as f32);
    let slot = app.project.order[app.slot];
    let mut toggle = None;
    for t in 0..tracks {
        let cell = Rect::from_min_size(
            area.min + Vec2::new((t % cols) as f32 * size.x, (t / cols) as f32 * size.y),
            size - Vec2::splat(3.0),
        );
        let audible = app.project.track_audible(t) && !slot.is_muted(t);
        let color = pattern::track_color(&app.project, t);
        let color = if audible { color } else { color.gamma_multiply(0.3) };
        let painter = ui.painter_at(cell);
        painter.rect_filled(cell, 2.0, theme::INSET);
        painter.rect_filled(Rect::from_min_size(cell.min, Vec2::new(cell.width(), 2.0)), 0.0, color);
        let mid = cell.center().y + 5.0;
        painter
            .line_segment([Pos2::new(cell.left(), mid), Pos2::new(cell.right(), mid)], (1.0, Color32::from_gray(30)));
        if let Some(wave) = scopes.get(t * TRACK_SCOPE_LEN..(t + 1) * TRACK_SCOPE_LEN) {
            let half = (cell.height() - 14.0) * 0.5;
            let points: Vec<Pos2> = wave
                .iter()
                .enumerate()
                .step_by(2)
                .map(|(i, x)| {
                    let px = cell.left() + i as f32 / (TRACK_SCOPE_LEN - 1) as f32 * cell.width();
                    Pos2::new(px, mid - (x * 1.5).clamp(-1.0, 1.0) * half)
                })
                .collect();
            painter.add(Shape::line(points, Stroke::new(1.0, color)));
        }
        painter.text(
            cell.left_top() + Vec2::new(4.0, 4.0),
            Align2::LEFT_TOP,
            app.project.track_name(t),
            FontId::proportional(10.5),
            if audible { theme::TEXT } else { theme::TEXT_WEAK },
        );
        if t == app.cursor.track {
            painter.rect_stroke(cell, 2.0, Stroke::new(1.0, theme::SELECTED), StrokeKind::Inside);
        }
        if app.project.tracks[t].solo {
            painter.text(
                cell.right_top() + Vec2::new(-4.0, 4.0),
                Align2::RIGHT_TOP,
                "S",
                FontId::proportional(10.5),
                theme::SELECTED,
            );
        }
        let resp = ui.interact(cell, ui.id().with(("track_scope", t)), Sense::click());
        let resp = resp.on_hover_text("Click to mute this track, right-click to solo it");
        if resp.clicked() {
            toggle = Some((t, false));
        } else if resp.secondary_clicked() {
            toggle = Some((t, true));
        }
    }
    // Muting and soloing, as the track headers do.
    if let Some((t, solo)) = toggle {
        if solo {
            app.project.solo_track(t);
        } else {
            app.project.tracks[t].mute ^= true;
        }
        app.mark();
    }
}
