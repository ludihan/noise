//! The song comments window: the song's title, artist and notes.

use super::{App, theme};
use eframe::egui::{self, RichText};

pub fn window(app: &mut App, ctx: &egui::Context) {
    let mut open = app.show_comments;
    let window = egui::Window::new("Song Comments")
        .open(&mut open)
        .default_size([420.0, 360.0])
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center());
    window.show(ctx, |ui| {
        let mut changed = false;
        egui::Grid::new("song_info").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            theme::caption(ui, "TITLE");
            changed |=
                ui.add(egui::TextEdit::singleline(&mut app.project.title).desired_width(f32::INFINITY)).changed();
            ui.end_row();
            theme::caption(ui, "ARTIST");
            changed |=
                ui.add(egui::TextEdit::singleline(&mut app.project.artist).desired_width(f32::INFINITY)).changed();
            ui.end_row();
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            theme::caption(ui, "COMMENTS");
            ui.label(RichText::new("saved with the song").small().color(theme::TEXT_WEAK));
        });
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let edit = egui::TextEdit::multiline(&mut app.project.comments)
                .desired_width(f32::INFINITY)
                .desired_rows(12)
                .hint_text("Notes about the song: credits, instructions, ideas…");
            changed |= ui.add(edit).changed();
        });
        if changed {
            app.mark_layout();
        }
    });
    app.show_comments = open;
}
