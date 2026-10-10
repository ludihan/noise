//! The transport bar: playback, song and edit settings, the position, the
//! master volume, the CPU meter and the scopes.

use super::*;

impl App {
    /// The transport bar: playback, song and edit settings, the position and
    /// the CPU load, in framed groups.
    pub(super) fn transport(&mut self, ui: &mut egui::Ui) {
        // One row when it fits, as in a wide window; else the song and
        // entry settings go below.
        let two_rows = self.transport_width > ui.available_width();
        let mut width = 0.0;
        ui.horizontal(|ui| {
            let x0 = ui.cursor().min.x;
            let playing = self.is_playing();
            let big = Vec2::new(30.0, 22.0);
            group(ui, |ui| {
                let play = icons::sized_button(ui, icons::Icon::Play, big, playing);
                if play.on_hover_text("Play (Space)").clicked() {
                    self.toggle_play();
                }
                if icons::sized_button(ui, icons::Icon::PlayFrom, big, false)
                    .on_hover_text("Play from the cursor's line (Shift+Space); Play starts the pattern from its top")
                    .clicked()
                {
                    self.play_from_cursor();
                }
                if icons::sized_button(ui, icons::Icon::Loop, big, self.loop_pattern)
                    .on_hover_text("Loop the current pattern")
                    .clicked()
                {
                    self.loop_pattern = !self.loop_pattern;
                }
                if icons::sized_button(ui, icons::Icon::Stop, big, false).on_hover_text("Stop").clicked() {
                    self.send(Cmd::Stop);
                }
                let rec = egui::Button::new("").min_size(big);
                let rec = if self.edit_mode { rec.fill(theme::RECORD) } else { rec };
                let resp = ui.add(rec).on_hover_text("Edit mode (Esc)");
                let dot = if self.edit_mode { Color32::WHITE } else { theme::RECORD };
                ui.painter().circle_filled(resp.rect.center(), 5.0, dot);
                if resp.clicked() {
                    self.edit_mode = !self.edit_mode;
                }
            });
            group(ui, |ui| {
                let h = Vec2::new(0.0, 22.0);
                if ui
                    .add(egui::Button::selectable(self.follow, "Follow").min_size(h))
                    .on_hover_text("Follow the play position")
                    .clicked()
                {
                    self.follow = !self.follow;
                }
                if ui
                    .add(egui::Button::selectable(self.metronome, "Metronome").min_size(h))
                    .on_hover_text("Click on every beat while playing")
                    .clicked()
                {
                    self.metronome = !self.metronome;
                    self.send(Cmd::Metronome(self.metronome));
                }
                let panic = ui.add(egui::Button::new("Panic").min_size(h));
                if panic.on_hover_text("Silence everything").clicked() {
                    self.send(Cmd::Panic);
                }
            });
            let settings_x = ui.cursor().min.x;
            if !two_rows {
                self.settings_groups(ui);
            }
            let settings_width = ui.cursor().min.x - settings_x;
            if settings_width > 0.0 {
                self.settings_width = settings_width;
            }
            group(ui, |ui| self.position(ui));
            let left = ui.cursor().min.x - x0;

            // On the right: the CPU meter and, before it, the master volume.
            let right = ui
                .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let x1 = ui.cursor().max.x;
                    if self.show_cpu {
                        group(ui, |ui| self.cpu_meter(ui));
                    }
                    group(ui, |ui| self.master_volume(ui));
                    x1 - ui.cursor().max.x
                })
                .inner;
            width = left + right + if two_rows { self.settings_width } else { 0.0 };
        });
        if two_rows && (self.show_song_settings || self.show_entry_settings) {
            ui.horizontal(|ui| {
                let x0 = ui.cursor().min.x;
                self.settings_groups(ui);
                self.settings_width = ui.cursor().min.x - x0;
            });
        }
        if !self.show_song_settings && !self.show_entry_settings {
            self.settings_width = 0.0;
        }
        self.transport_width = width;
    }

    /// The song settings (tempo, lines per beat, ticks, swing, song loop)
    /// and the entry settings (octave, step, volume) of the transport.
    pub(super) fn settings_groups(&mut self, ui: &mut egui::Ui) {
        let playing = self.is_playing();
        if self.show_song_settings {
            group(ui, |ui| {
                transport_label(ui, "BPM");
                // While an Fxx has changed the tempo, the field shows the
                // tempo playing, in the play colour; editing it sets the
                // song's.
                let live = f32::from_bits(self.shared.bpm.load(Ordering::Relaxed));
                let changed = playing && (live - self.project.bpm).abs() > 0.01;
                let mut bpm = if changed { live } else { self.project.bpm };
                let drag = egui::DragValue::new(&mut bpm).range(20.0..=999.0).speed(0.5).max_decimals(1);
                let tip = if changed {
                    format!("Beats per minute: {live:.0} set by an Fxx command, {:.1} for the song", self.project.bpm)
                } else {
                    "Beats per minute".into()
                };
                let resp = ui
                    .scope(|ui| {
                        if changed {
                            ui.visuals_mut().override_text_color = Some(theme::SCOPE);
                        }
                        field(ui, 46.0, drag)
                    })
                    .inner;
                if resp.on_hover_text(tip).changed() {
                    self.project.bpm = bpm;
                    self.mark();
                }
                transport_label(ui, "LPB");
                let mut lpb = self.project.lpb;
                if field(ui, 30.0, egui::DragValue::new(&mut lpb).range(1..=32))
                    .on_hover_text("Lines per beat")
                    .changed()
                {
                    self.project.lpb = lpb;
                    self.mark();
                }
                transport_label(ui, "TPL");
                let mut tpl = self.project.tpl;
                let tip = "Ticks per line: the steps effects take within a line";
                if field(ui, 30.0, egui::DragValue::new(&mut tpl).range(1..=16)).on_hover_text(tip).changed() {
                    self.project.tpl = tpl;
                    self.mark();
                }
                transport_label(ui, "SWING");
                let mut swing = self.project.groove * 100.0;
                let tip = "Groove: every odd line starts up to half a line late; double-click for none";
                let swing_field =
                    egui::DragValue::new(&mut swing).range(0.0..=100.0).speed(0.5).suffix("%").max_decimals(0);
                let resp = field(ui, 40.0, swing_field).on_hover_text(tip);
                if resp.double_clicked() {
                    swing = 0.0;
                }
                if swing / 100.0 != self.project.groove {
                    self.project.groove = swing / 100.0;
                    self.mark();
                }
            });
        }
        if self.show_entry_settings {
            group(ui, |ui| {
                transport_label(ui, "OCT");
                field(ui, 30.0, egui::DragValue::new(&mut self.octave).range(0..=9)).on_hover_text("Octave (- / =)");
                transport_label(ui, "STEP");
                field(ui, 30.0, egui::DragValue::new(&mut self.step).range(0..=16))
                    .on_hover_text("Lines the cursor moves after entering a note");
                transport_label(ui, "VOL");
                let vol = egui::DragValue::new(&mut self.entry_vol)
                    .range(0..=0x80)
                    .speed(1.0)
                    .custom_formatter(|v, _| if v >= 128.0 { "--".into() } else { format!("{:02X}", v as u8) })
                    .custom_parser(|t| {
                        if t.trim() == "--" {
                            Some(128.0)
                        } else {
                            u8::from_str_radix(t.trim(), 16).ok().map(f64::from)
                        }
                    });
                field(ui, 30.0, vol)
                    .on_hover_text("Volume written with the notes you enter, in hex; -- writes none (full volume)");
            });
        }
    }

    /// The play position, or the cursor while stopped, and the time played.
    pub(super) fn position(&self, ui: &mut egui::Ui) {
        let playing = self.is_playing();
        let (slot, line) = if playing { self.play_position() } else { (self.slot, self.cursor.line) };
        let pattern = self.project.order.get(slot).map_or(0, |s| s.pattern);
        let secs = f32::from_bits(self.shared.time.load(Ordering::Relaxed));
        let text =
            format!("SEQ {slot:02} PAT {pattern:02} LINE {line:03} {}:{:04.1}", (secs / 60.0) as u32, secs % 60.0);
        egui::Frame::new().fill(theme::INSET).corner_radius(2).inner_margin(egui::Margin::symmetric(6, 2)).show(
            ui,
            |ui| {
                let color = if playing { theme::SCOPE } else { theme::TEXT_WEAK };
                ui.label(RichText::new(text).monospace().color(color))
                    .on_hover_text("Song position, pattern, line and time played");
            },
        );
    }

    /// The master volume: the Output module's, as a bar to drag (Shift
    /// for fine steps), double-click for its default.
    pub(super) fn master_volume(&mut self, ui: &mut egui::Ui) {
        let Some(out) = self.project.module(OUTPUT_ID) else { return };
        let spec = crate::project::ParamSpec { name: "Master", ..out.kind.params()[0] };
        // While an envelope moves it, the bar follows, as parameter bars do.
        let shown = self.automated(OUTPUT_ID, 0).unwrap_or(out.params[0]);
        let mut volume = shown;
        let resp = ui
            .allocate_ui(Vec2::new(104.0, ui.spacing().interact_size.y), |ui| {
                widgets::param_bar(ui, &spec, &mut volume, |_| {})
            })
            .inner;
        if resp.on_hover_text("The volume of everything, after the mix").changed() && volume != shown {
            self.project.module_mut(OUTPUT_ID).unwrap().params[0] = volume;
            self.mark();
        }
    }

    /// How much of the real time rendering audio takes.
    pub(super) fn cpu_meter(&self, ui: &mut egui::Ui) {
        let cpu = f32::from_bits(self.shared.cpu.load(Ordering::Relaxed));
        // The load as a bar with its figure over it.
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(62.0, 16.0), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 2.0, theme::INSET);
        let color = match cpu {
            c if c > 0.8 => theme::RECORD,
            c if c > 0.5 => theme::SELECTED,
            _ => theme::SCOPE,
        };
        let bar = Rect::from_min_size(rect.min, Vec2::new(rect.width() * cpu.clamp(0.0, 1.0), rect.height()));
        painter.rect_filled(bar, 2.0, color);
        let rate = self.audio.as_ref().map_or("no audio device".into(), |a| format!("{} Hz", a.sample_rate));
        let text = format!("CPU {:.0}%", cpu * 100.0);
        let font = egui::FontId::monospace(10.0);
        painter.text(
            rect.center() + Vec2::new(1.0, 1.0),
            egui::Align2::CENTER_CENTER,
            &text,
            font.clone(),
            Color32::BLACK,
        );
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, font, theme::TEXT);
        resp.on_hover_text(format!("Time spent rendering audio ({rate})"));
    }

    /// Master scope (left and right channel) and peak meters.
    pub(super) fn scopes(&self, ui: &mut egui::Ui) {
        let size = ui.available_size();
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 2.0, theme::INSET);
        let meters_w = 34.0;
        let scope = Rect::from_min_max(rect.min, Pos2::new(rect.right() - meters_w - 4.0, rect.bottom()));
        let lane_h = scope.height() / 2.0;
        if let Ok(data) = self.shared.scope.lock() {
            let data = &data[data.len().saturating_sub(1024)..];
            for ch in [0, 1] {
                let mid = scope.top() + lane_h * (ch as f32 + 0.5);
                painter.line_segment(
                    [Pos2::new(scope.left(), mid), Pos2::new(scope.right(), mid)],
                    Stroke::new(1.0, Color32::from_gray(36)),
                );
                let w = scope.width().max(1.0) as usize;
                let pts: Vec<Pos2> = (0..w)
                    .map(|x| {
                        let v = data[x * data.len() / w][ch];
                        Pos2::new(scope.left() + x as f32, mid - v.clamp(-1.0, 1.0) * lane_h * 0.45)
                    })
                    .collect();
                painter.line(pts, Stroke::new(1.0, theme::SCOPE));
            }
        }
        for ch in 0..2 {
            let peak = f32::from_bits(self.shared.peak[ch].load(Ordering::Relaxed));
            let db = 20.0 * peak.max(1e-6).log10();
            // Show -48 dB .. +6 dB.
            let frac = ((db + 48.0) / 54.0).clamp(0.0, 1.0);
            let x = rect.right() - meters_w + ch as f32 * 17.0;
            let full = Rect::from_min_max(Pos2::new(x, rect.top() + 2.0), Pos2::new(x + 14.0, rect.bottom() - 2.0));
            painter.rect_filled(full, 1.0, Color32::from_gray(30));
            let top = full.bottom() - full.height() * frac;
            let bar = Rect::from_min_max(Pos2::new(full.left(), top), full.max);
            let color = if peak >= 1.0 {
                theme::RECORD
            } else if db > -6.0 {
                theme::SELECTED
            } else {
                theme::SCOPE
            };
            painter.rect_filled(bar, 1.0, color);
            let zero = full.bottom() - full.height() * (48.0 / 54.0);
            painter.line_segment(
                [Pos2::new(full.left(), zero), Pos2::new(full.right(), zero)],
                Stroke::new(1.0, Color32::from_gray(120)),
            );
        }
    }
}

fn transport_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).small().color(theme::TEXT_WEAK));
}

/// Adds a transport field at a fixed width, so a value growing a digit
/// doesn't push what comes after it.
fn field(ui: &mut egui::Ui, width: f32, widget: impl egui::Widget) -> egui::Response {
    ui.add_sized(Vec2::new(width, ui.spacing().interact_size.y), widget)
}

/// A framed group of controls in the transport bar.
fn group<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(theme::FRAME_BG)
        .stroke(Stroke::new(1.0, theme::FRAME_LINE))
        .corner_radius(3)
        .inner_margin(egui::Margin::symmetric(4, 3))
        .show(ui, |ui| ui.horizontal(add).inner)
        .inner
}
