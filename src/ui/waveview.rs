//! The wave view: the samples that are sounding right now, each with a
//! playhead per voice. It replaces the master scope when switched on.

use super::sampler::{Peak, draw_channel, overview};
use super::{App, theme};
use crate::sample::Sample;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use std::sync::Arc;

/// Lanes shown at most; more sounding samples are counted in the last one.
const MAX_LANES: usize = 4;

#[derive(Default)]
pub struct WaveView {
    /// Overviews of recently shown samples, by the audio's address.
    peaks: Vec<(usize, Vec<Peak>)>,
}

struct Lane {
    /// Sampler and sample slot; `None` for the preview.
    source: Option<(u8, usize)>,
    label: String,
    data: Arc<Sample>,
    /// Loop range in frames, if the sample loops.
    looped: Option<(usize, usize)>,
    /// Position in frames and loudness of each voice.
    heads: Vec<(f64, f32)>,
}

pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    let mut lanes: Vec<Lane> = Vec::new();
    let mut heads = app.playheads();
    heads.sort_by_key(|p| (p.module, p.slot));
    for p in heads {
        if let Some(l) = lanes.last_mut()
            && l.source == Some((p.module, p.slot))
        {
            l.heads.push((p.pos, p.level));
            continue;
        }
        let Some(m) = app.project.module(p.module) else { continue };
        let Some(slot) = m.samples.get(p.slot) else { continue };
        let Some(data) = slot.data.clone() else { continue };
        lanes.push(Lane {
            source: Some((p.module, p.slot)),
            label: format!("{:02X} {} · {:02} {}", p.module, m.name, p.slot, slot.name),
            data,
            looped: (slot.loop_mode != 0).then_some((slot.loop_start, slot.loop_end)),
            heads: vec![(p.pos, p.level)],
        });
    }
    if let (Some(pos), Some(p)) = (app.preview_pos(), &app.previewing) {
        lanes.push(Lane {
            source: None,
            label: format!("Preview · {}", p.sample.name),
            data: p.sample.clone(),
            looped: None,
            heads: vec![(pos, 1.0)],
        });
    }

    let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, theme::INSET);
    if lanes.is_empty() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Samples show here while they play.",
            FontId::proportional(12.0),
            theme::TEXT_WEAK,
        );
        return;
    }
    let hidden = lanes.len().saturating_sub(MAX_LANES);
    lanes.truncate(MAX_LANES);

    let view = &mut app.waves;
    // Forget overviews of samples that are no longer shown.
    view.peaks.retain(|(ptr, _)| lanes.iter().any(|l| Arc::as_ptr(&l.data) as usize == *ptr));
    let lane_h = rect.height() / lanes.len() as f32;
    for (n, lane) in lanes.iter().enumerate() {
        let top = rect.top() + lane_h * n as f32;
        let r = Rect::from_min_size(Pos2::new(rect.left(), top), Vec2::new(rect.width(), lane_h));
        let ptr = Arc::as_ptr(&lane.data) as usize;
        if !view.peaks.iter().any(|p| p.0 == ptr) {
            view.peaks.push((ptr, overview(&lane.data.frames)));
        }
        let peaks = &view.peaks.iter().find(|p| p.0 == ptr).unwrap().1;
        draw_lane(&painter, r, lane, peaks);
        if n > 0 {
            painter.line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, theme::FRAME_LINE));
        }
    }
    if hidden > 0 {
        painter.text(
            rect.right_bottom() - Vec2::new(4.0, 2.0),
            Align2::RIGHT_BOTTOM,
            format!("+{hidden} more"),
            FontId::proportional(11.0),
            theme::TEXT_WEAK,
        );
    }
    ui.ctx().request_repaint();
}

fn draw_lane(painter: &egui::Painter, r: Rect, lane: &Lane, peaks: &[Peak]) {
    let len = lane.data.len() as f64;
    let to_x = |f: f64| r.left() + (f / len) as f32 * r.width();
    if let Some((a, b)) = lane.looped {
        let lr = Rect::from_x_y_ranges(to_x(a as f64)..=to_x(b as f64), r.y_range());
        painter.rect_filled(lr, 0.0, theme::SCOPE.gamma_multiply(0.12));
    }
    let wave = r.shrink2(Vec2::new(0.0, 2.0));
    let half = wave.height() * 0.45;
    for ch in 0..lane.data.channels.clamp(1, 2) as usize {
        draw_channel(painter, &lane.data.frames, peaks, ch, wave, 0.0, len, wave.center().y, half);
    }
    for &(pos, level) in &lane.heads {
        let x = to_x(pos);
        let color = theme::SELECTED.gamma_multiply(0.4 + 0.6 * level.min(1.0));
        painter.line_segment([Pos2::new(x, r.top()), Pos2::new(x, r.bottom())], Stroke::new(2.0, color));
    }
    let galley = painter.layout_no_wrap(lane.label.clone(), FontId::proportional(11.0), theme::TEXT);
    let at = r.left_top() + Vec2::new(4.0, 2.0);
    painter.rect_filled(Rect::from_min_size(at, galley.size()).expand(2.0), 2.0, Color32::from_black_alpha(170));
    painter.galley(at, galley, theme::TEXT);
}
