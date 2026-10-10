//! Work that takes a while (rendering, exporting, importing, opening and
//! saving songs, backing them up) on threads of their own, so the window
//! keeps answering and the desktop doesn't take it for frozen. The status
//! bar shows each job and how far it has got, with a button to cancel those
//! that can stop; what a job made is put in place when it is done, back on
//! the window's thread.

use super::{App, theme};
use eframe::egui::{self, RichText};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// What a job hands back: the change to make to the app.
pub type Finish = Box<dyn FnOnce(&mut App) + Send>;

/// How far a job has got, and whether it was asked to stop, shared
/// between it and the window.
pub struct Progress {
    /// 0..1, or below 0 until the job says.
    fraction: AtomicU32,
    cancel: AtomicBool,
}

impl Default for Progress {
    fn default() -> Self {
        Progress { fraction: AtomicU32::new((-1f32).to_bits()), cancel: AtomicBool::new(false) }
    }
}

impl Progress {
    /// Says it is `f` (0..1) of the way through; false once asked to stop.
    pub fn set(&self, f: f32) -> bool {
        self.fraction.store(f.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        !self.cancel.load(Ordering::Relaxed)
    }

    /// How far through, if the job has said.
    fn get(&self) -> Option<f32> {
        Some(f32::from_bits(self.fraction.load(Ordering::Relaxed))).filter(|f| *f >= 0.0)
    }
}

pub struct Job {
    what: String,
    progress: Arc<Progress>,
    done: Receiver<Finish>,
}

/// Starts `work` on a thread, as `what` ("Rendering song.wav"), unless the
/// same job is still going.
pub fn spawn(app: &mut App, what: impl Into<String>, work: impl FnOnce(&Progress) -> Finish + Send + 'static) {
    let what = what.into();
    if app.jobs.iter().any(|j| j.what == what) {
        app.set_status(format!("Still busy: {what}"));
        return;
    }
    let progress = Arc::new(Progress::default());
    let (tx, done) = channel();
    let shared = progress.clone();
    std::thread::spawn(move || {
        let _ = tx.send(work(&shared));
    });
    app.jobs.push(Job { what, progress, done });
}

/// Puts finished jobs' work in place.
pub fn poll(app: &mut App) {
    let mut k = 0;
    while k < app.jobs.len() {
        match app.jobs[k].done.try_recv() {
            Ok(finish) => {
                app.jobs.remove(k);
                finish(app);
            }
            Err(TryRecvError::Empty) => k += 1,
            Err(TryRecvError::Disconnected) => {
                let job = app.jobs.remove(k);
                app.set_status(format!("{} failed", job.what));
            }
        }
    }
    if !app.jobs.is_empty() {
        // Keep the progress moving while nothing else asks to draw.
        app.ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// The jobs going on, in the status bar: what each is, how far it has got,
/// and Cancel for those that can stop. True while there are any.
pub fn status(app: &App, ui: &mut egui::Ui) -> bool {
    if app.close_when_done {
        ui.label(RichText::new("Closing when done:").color(theme::TEXT_WEAK));
    }
    for job in &app.jobs {
        ui.label(RichText::new(format!("{}…", job.what)).color(theme::SELECTED));
        match job.progress.get() {
            Some(f) => {
                ui.add(egui::ProgressBar::new(f).desired_width(160.0).show_percentage());
                if ui.small_button("Cancel").clicked() {
                    job.progress.cancel.store(true, Ordering::Relaxed);
                }
            }
            None => {
                ui.spinner();
            }
        }
    }
    !app.jobs.is_empty()
}
