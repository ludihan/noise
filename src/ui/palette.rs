//! The command palette (Ctrl+K): every action of the program, found by
//! typing a few letters of its name, and run with Enter.

use super::*;
use crate::project::{ModuleKind, Owner};
use eframe::egui::{Align2, Sense};

/// The palette's state between frames.
#[derive(Default)]
pub struct Palette {
    pub open: bool,
    #[cfg(test)]
    pub preset: Option<String>,
    query: String,
    /// The command highlighted, among those found.
    selected: usize,
}

/// A command: what it is called, where it belongs, its keys, and what it
/// does.
struct Command {
    label: String,
    group: &'static str,
    keys: &'static str,
    run: Box<dyn FnOnce(&mut App)>,
}

fn command(
    group: &'static str,
    label: impl Into<String>,
    keys: &'static str,
    run: impl FnOnce(&mut App) + 'static,
) -> Command {
    Command { label: label.into(), group, keys, run: Box::new(run) }
}

/// Every command there is right now.
fn commands(app: &App) -> Vec<Command> {
    use files::Purpose;
    let mut list = vec![
        command("Transport", "Play / Stop", "Space", |a: &mut App| a.toggle_play()),
        command("Transport", "Play from the Cursor", "Shift+Space", |a: &mut App| a.play_from_cursor()),
        command("Transport", "Loop the Pattern", "", |a: &mut App| a.loop_pattern = !a.loop_pattern),
        command("Transport", "Follow the Play Position", "", |a: &mut App| a.follow = !a.follow),
        command("Transport", "Metronome", "", |a: &mut App| {
            a.metronome = !a.metronome;
            a.send(Cmd::Metronome(a.metronome));
        }),
        command("Transport", "Panic: Silence Everything", "", |a: &mut App| a.send(Cmd::Panic)),
        command("File", "New Song", "Ctrl+N", |a: &mut App| a.request(Pending::New)),
        command("File", "Open Song…", "Ctrl+O", |a: &mut App| a.pick_file(Purpose::OpenSong)),
        command("File", "Save", "Ctrl+S", |a: &mut App| a.save_or_ask()),
        command("File", "Save As…", "Ctrl+Shift+S", |a: &mut App| a.pick_file(Purpose::SaveSong)),
        command("File", "Open Backup…", "", |a: &mut App| a.pick_backup()),
        command("File", "Import Project…", "", |a: &mut App| a.pick_file(Purpose::ImportProject)),
        command("File", "Export Project…", "", |a: &mut App| a.pick_file(Purpose::ExportProject)),
        command("File", "Render to WAV…", "", |a: &mut App| a.render_dialog = Some(Purpose::Render)),
        command("File", "Render Stems…", "", |a: &mut App| a.render_dialog = Some(Purpose::RenderStems)),
        command("Edit", "Undo", "Ctrl+Z", |a: &mut App| a.undo()),
        command("Edit", "Redo", "Ctrl+Y", |a: &mut App| a.redo()),
        command("View", "Pattern Editor", "F2", |a: &mut App| a.view = View::Pattern),
        command("View", "Mixer", "F3", |a: &mut App| a.view = View::Mixer),
        command("View", "Instrument Editor", "F4", |a: &mut App| a.view = View::Sampler),
        command("View", "Automation", "", |a: &mut App| (a.lower, a.show_lower) = (Lower::Automation, true)),
        command("View", "Track Effects", "", |a: &mut App| {
            let t = a.cursor.track;
            a.show_track_fx(t);
        }),
        command("View", "Master Chain", "", |a: &mut App| a.show_chain(Owner::Module(OUTPUT_ID))),
        command("View", "Song Comments", "", |a: &mut App| a.show_comments = !a.show_comments),
        command("View", "Show or Hide the Upper Frame", "Ctrl+1", |a: &mut App| a.panels.upper = !a.panels.upper),
        command("View", "Show or Hide the Sequencer", "Ctrl+2", |a: &mut App| a.panels.sequencer = !a.panels.sequencer),
        command("View", "Show or Hide the Lower Frame", "Ctrl+3", |a: &mut App| a.show_lower = !a.show_lower),
        command("View", "Show or Hide the Instrument List", "Ctrl+4", |a: &mut App| {
            a.panels.instruments = !a.panels.instruments
        }),
        command("View", "Show or Hide the Disk Browser", "Ctrl+5", |a: &mut App| a.panels.browser = !a.panels.browser),
        command("Edit", "MIDI Input…", "", |a: &mut App| {
            a.show_midi = true;
            a.midi_ports = crate::midi::Midi::ports();
        }),
        command("Tracks", "Add a Send Track", "", |a: &mut App| {
            let name = a.project.next_send_name();
            if let Some(id) = a.project.add_send_track(&name) {
                a.show_send(id);
                a.mark();
            }
        }),
    ];
    for (i, (name, _)) in DEMOS.iter().enumerate() {
        list.push(command("Demo songs", format!("Open the Demo {name}"), "", move |a: &mut App| {
            a.request(Pending::Demo(i))
        }));
    }
    for title in help::titles() {
        list.push(command("Manual", format!("Manual: {title}"), "F1", move |a: &mut App| {
            a.help.open(title);
            a.show_help = true;
        }));
    }
    // The instruments, to pick, and kinds to add.
    let p = &app.project;
    let mut ids: Vec<u8> = p.modules.iter().filter(|m| m.kind.in_instrument_list()).map(|m| m.id).collect();
    ids.sort();
    for id in ids {
        let name = p.module(id).map(|m| m.name.clone()).unwrap_or_default();
        list.push(command("Instruments", format!("Edit {id:02X} {name}"), "", move |a: &mut App| {
            a.select(Some(id));
            a.view = View::Sampler;
        }));
    }
    for kind in ModuleKind::ADDABLE.into_iter().filter(|k| k.in_instrument_list()) {
        list.push(command("Instruments", format!("Add a {}", kind.name()), "", move |a: &mut App| {
            instruments::add(a, kind);
            a.view = View::Sampler;
        }));
    }
    // Effects for the instrument being edited, at the end of its chain.
    if let Some(inst) = app.instrument().filter(|&i| p.module(i).is_some_and(|m| m.kind.starts_chain())) {
        let name = p.module(inst).map(|m| m.name.clone()).unwrap_or_default();
        for kind in ModuleKind::ADDABLE.into_iter().filter(|k| k.is_effect() && *k != ModuleKind::Send) {
            let label = format!("Add {} to {name}", kind.name());
            list.push(command("Effects", label, "", move |a: &mut App| {
                let at = a.project.chain(inst).effects.len();
                if let Some(new) = a.project.chain_insert(inst, at, kind) {
                    a.show_chain(Owner::Module(inst));
                    a.select(Some(new));
                    a.chain_view.reveal(new);
                    a.mark();
                }
            }));
        }
    }
    for t in 0..app.pattern().num_tracks() {
        let label = format!("Track Effects: {:02} {}", t + 1, p.track_name(t));
        list.push(command("Tracks", label, "", move |a: &mut App| a.show_track_fx(t)));
    }
    for send in &p.sends {
        let (id, name) = (send.module, p.module(send.module).map(|m| m.name.clone()).unwrap_or_default());
        list.push(command("Tracks", format!("Send Track: {name}"), "", move |a: &mut App| a.show_send(id)));
    }
    list
}

/// How well `label` matches `query`: `None` when some word of the query is
/// missing from it, higher for a match at its start or a word's.
fn score(label: &str, query: &str) -> Option<i32> {
    let label = label.to_lowercase();
    let mut total = 0;
    for word in query.split_whitespace() {
        let at = label.find(word)? as i32;
        let starts_word = at == 0 || label.as_bytes()[at as usize - 1] == b' ';
        total += if at == 0 {
            30
        } else if starts_word {
            20
        } else {
            5
        } - at.min(20) / 4;
    }
    Some(total)
}

/// The palette, while it is open: a field to type in over the commands
/// found, the best first.
pub fn window(app: &mut App, ctx: &egui::Context) {
    if !app.palette.open {
        return;
    }
    #[cfg(test)]
    if let Some(q) = app.palette.preset.take() {
        app.palette.query = q;
    }
    let mut found: Vec<(i32, Command)> = commands(app)
        .into_iter()
        .filter_map(|c| {
            let text = format!("{} {}", c.label, c.group);
            score(&text, &app.palette.query.to_lowercase()).map(|s| (s, c))
        })
        .collect();
    found.sort_by_key(|(s, _)| -s);
    found.truncate(14);
    let (up, down, enter, escape) = ctx.input(|i| {
        (
            i.key_pressed(Key::ArrowUp),
            i.key_pressed(Key::ArrowDown),
            i.key_pressed(Key::Enter),
            i.key_pressed(Key::Escape),
        )
    });
    let palette = &mut app.palette;
    if down {
        palette.selected = (palette.selected + 1).min(found.len().saturating_sub(1));
    }
    if up {
        palette.selected = palette.selected.saturating_sub(1);
    }
    palette.selected = palette.selected.min(found.len().saturating_sub(1));
    let mut run = None;
    let screen = ctx.content_rect();
    let shown = egui::Area::new(egui::Id::new("palette"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(screen.center().x - 280.0, screen.top() + 80.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).inner_margin(8.0).show(ui, |ui| {
                ui.set_width(560.0);
                let field = egui::TextEdit::singleline(&mut palette.query)
                    .hint_text("Type a command: play, add reverb, mixer, demo…")
                    .desired_width(f32::INFINITY)
                    .font(egui::FontId::proportional(15.0));
                let resp = ui.add(field);
                resp.request_focus();
                if resp.changed() {
                    palette.selected = 0;
                }
                ui.add_space(4.0);
                if found.is_empty() {
                    ui.label(RichText::new("Nothing by that name").color(theme::TEXT_WEAK));
                }
                for (k, (_, c)) in found.iter().enumerate() {
                    let on = k == palette.selected;
                    let (rect, row) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::click());
                    if on || row.hovered() {
                        ui.painter().rect_filled(rect, 3.0, if on { theme::SELECTED } else { theme::BUTTON_HOVER });
                    }
                    let color = if on { theme::SELECTED_TEXT } else { theme::TEXT };
                    let weak = if on { theme::SELECTED_TEXT } else { theme::TEXT_WEAK };
                    let font = egui::FontId::proportional(13.0);
                    let painter = ui.painter();
                    painter.text(
                        rect.left_center() + Vec2::new(8.0, 0.0),
                        Align2::LEFT_CENTER,
                        &c.label,
                        font.clone(),
                        color,
                    );
                    let right =
                        if c.keys.is_empty() { c.group.to_string() } else { format!("{}   {}", c.keys, c.group) };
                    painter.text(
                        rect.right_center() - Vec2::new(8.0, 0.0),
                        Align2::RIGHT_CENTER,
                        right,
                        egui::FontId::proportional(11.0),
                        weak,
                    );
                    if row.clicked() {
                        run = Some(k);
                    }
                }
                ui.add_space(2.0);
                let hint = "Up and Down to pick, Enter to run, Esc to close";
                ui.label(RichText::new(hint).small().color(theme::TEXT_WEAK));
            });
        });
    // A click anywhere else closes it.
    let elsewhere = shown.response.clicked_elsewhere();
    if enter && !found.is_empty() {
        run = Some(palette.selected);
    }
    if escape || elsewhere || run.is_some() {
        palette.open = false;
        palette.query.clear();
        palette.selected = 0;
    }
    if let Some(k) = run {
        let (_, c) = found.swap_remove(k);
        (c.run)(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_find_commands_best_at_their_start() {
        assert!(score("Add Reverb to Pad", "rev").is_some());
        assert!(score("Add Reverb to Pad", "add pad").is_some());
        assert!(score("Add Reverb to Pad", "delay").is_none());
        assert!(score("Mixer", "mix") > score("Show the Mixer", "mix"));
        assert!(score("Plate Reverb", "rev") > score("Unreverbed", "rev"));
    }
}
