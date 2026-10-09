//! The master spectrum analyzer of the upper frame: the
//! output's frequencies on a log scale from 20 Hz, in decibels.

use super::theme;
use crate::dsp::Frame;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke};
use std::f32::consts::TAU;

/// The loudest and quietest levels shown, in dB.
const TOP: f32 = 0.0;
const BOTTOM: f32 = -84.0;
/// How fast the display falls after a peak, in dB per second.
const FALL: f32 = 48.0;

#[derive(Default)]
pub struct SpectrumView {
    /// What each column of the display shows, in dB.
    columns: Vec<f32>,
}

/// The amplitude of each frequency bin of the mono sum of `frames`, which
/// must be a power of two long, scaled so a full-scale sine reads 1.
fn magnitudes(frames: &[Frame]) -> Vec<f32> {
    let n = frames.len();
    let window: Vec<f32> = (0..n).map(|i| 0.5 - 0.5 * (TAU * i as f32 / n as f32).cos()).collect();
    let gain = 2.0 / window.iter().sum::<f32>();
    let mut re: Vec<f32> = frames.iter().zip(&window).map(|(f, w)| (f[0] + f[1]) * 0.5 * w).collect();
    let mut im = vec![0.0; n];
    crate::dsp::fft::Fft::new(n).forward(&mut re, &mut im);
    (0..n / 2).map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt() * gain).collect()
}

fn db(amp: f32) -> f32 {
    20.0 * amp.max(1e-9).log10()
}

pub fn panel(view: &mut SpectrumView, ui: &mut egui::Ui, frames: &[Frame], sr: f32) {
    let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, theme::INSET);
    let nyquist = sr / 2.0;
    let x_of = |f: f32| rect.left() + (f / 20.0).ln() / (nyquist / 20.0).ln() * rect.width();
    let y_of = |d: f32| rect.top() + (TOP - d.clamp(BOTTOM, TOP)) / (TOP - BOTTOM) * rect.height();

    // Grid lines at round frequencies and every 12 dB.
    let grid = Color32::from_gray(34);
    let marks: Vec<f32> = [50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0, 20000.0]
        .into_iter()
        .filter(|&f| f < nyquist)
        .collect();
    for &f in &marks {
        let x = x_of(f);
        painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], (1.0, grid));
    }
    for d in (1..7).map(|i| -12.0 * i as f32) {
        let y = y_of(d);
        painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], (1.0, grid));
    }

    let bins = magnitudes(frames);
    let bin_hz = sr / frames.len() as f32;
    let cols = rect.width().max(1.0) as usize;
    view.columns.resize(cols, BOTTOM);
    let fall = FALL * ui.input(|i| i.stable_dt).min(0.1);
    let freq_at = |x: f32| 20.0 * (nyquist / 20.0).powf(x / cols as f32);
    for (x, shown) in view.columns.iter_mut().enumerate() {
        let (k0, k1) = (freq_at(x as f32) / bin_hz, freq_at(x as f32 + 1.0) / bin_hz);
        // Low columns fall between bins; high ones span several.
        let amp = if k1 - k0 < 1.0 {
            let k = (k0 + k1) / 2.0;
            let (i, t) = (k.floor() as usize, k.fract());
            let at = |i: usize| bins.get(i).copied().unwrap_or(0.0);
            at(i) * (1.0 - t) + at(i + 1) * t
        } else {
            bins[(k0.ceil() as usize).min(bins.len() - 1)..(k1.floor() as usize + 1).min(bins.len())]
                .iter()
                .fold(0.0, |m: f32, &a| m.max(a))
        };
        *shown = db(amp).max(*shown - fall);
    }

    let points: Vec<Pos2> =
        view.columns.iter().enumerate().map(|(x, &d)| Pos2::new(rect.left() + x as f32 + 0.5, y_of(d))).collect();
    for (x, p) in points.iter().enumerate() {
        let col = Rect::from_x_y_ranges(rect.left() + x as f32..=rect.left() + x as f32 + 1.0, p.y..=rect.bottom());
        painter.rect_filled(col, 0.0, theme::SCOPE.gamma_multiply(0.35));
    }
    painter.add(Shape::line(points, Stroke::new(1.0, theme::SCOPE)));
    for f in marks {
        let label = if f >= 1000.0 { format!("{}k", f / 1000.0) } else { format!("{f}") };
        let at = Pos2::new(x_of(f) + 2.0, rect.bottom() - 1.0);
        painter.text(at, Align2::LEFT_BOTTOM, label, FontId::proportional(9.0), theme::TEXT_WEAK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sine_shows_up_in_its_bin() {
        let n = 1024;
        // Bin 64 of 1024 at 44.1 kHz.
        let frames: Vec<Frame> = (0..n).map(|i| [(TAU * 64.0 * i as f32 / n as f32).sin() * 0.5; 2]).collect();
        let m = magnitudes(&frames);
        let peak = (0..m.len()).max_by(|&a, &b| m[a].total_cmp(&m[b])).unwrap();
        assert_eq!(peak, 64);
        assert!((m[64] - 0.5).abs() < 0.01, "{}", m[64]);
        assert!(m[200] < 1e-3);
    }
}
