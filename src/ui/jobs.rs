//! Work that takes a while (rendering, exporting, importing and opening
//! songs) on a thread of its own, so the window keeps answering and the
//! desktop doesn't take it for frozen. The status bar shows how far it has
//! got, with a button to cancel; what it made is put in place when it is
//! done, back on the window's thread.

use super::{App, theme};
use eframe::egui::{self, RichText};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// What a job hands back: the change to make to the app.
pub type Finish = Box<dyn FnOnce(&mut App) + Send>;

/// How far a job has got, and whether it was asked to stop, shared
/// between it and the window.
#[derive(Default)]
pub struct Progress {
    fraction: AtomicU32,
    cancel: AtomicBool,
}

impl Progress {
    /// Says it is `f` (0..1) of the way through; false once asked to stop.
    pub fn set(&self, f: f32) -> bool {
        self.fraction.store(f.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        !self.cancel.load(Ordering::Relaxed)
    }

    fn get(&self) -> f32 {
        f32::from_bits(self.fraction.load(Ordering::Relaxed))
    }
}

pub struct Job {
    what: String,
    progress: Arc<Progress>,
    done: Receiver<Finish>,
}

/// Starts `work` on a thread, as `what` ("Rendering song.wav"), unless
/// another job is still going.
pub fn spawn(app: &mut App, what: impl Into<String>, work: impl FnOnce(&Progress) -> Finish + Send + 'static) {
    if let Some(job) = &app.job {
        app.set_status(format!("Still busy: {}", job.what));
        return;
    }
    let progress = Arc::new(Progress::default());
    let (tx, done) = channel();
    let shared = progress.clone();
    std::thread::spawn(move || {
        let _ = tx.send(work(&shared));
    });
    let what = what.into();
    app.set_status(format!("{what}…"));
    app.job = Some(Job { what, progress, done });
}

/// Puts a finished job's work in place.
pub fn poll(app: &mut App) {
    let Some(job) = &app.job else { return };
    match job.done.try_recv() {
        Ok(finish) => {
            app.job = None;
            finish(app);
        }
        Err(TryRecvError::Empty) => {
            // Keep the progress moving while nothing else asks to draw.
            app.ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        Err(TryRecvError::Disconnected) => {
            let what = app.job.take().map(|j| j.what).unwrap_or_default();
            app.set_status(format!("{what} failed"));
        }
    }
}

/// The job going on, in the status bar: what it is, how far it has got,
/// and Cancel. True while there is one.
pub fn status(app: &App, ui: &mut egui::Ui) -> bool {
    let Some(job) = &app.job else { return false };
    let f = job.progress.get();
    ui.label(RichText::new(format!("{}…", job.what)).color(theme::SELECTED));
    ui.add(egui::ProgressBar::new(f).desired_width(160.0).show_percentage());
    if ui.small_button("Cancel").clicked() {
        job.progress.cancel.store(true, Ordering::Relaxed);
    }
    true
}
