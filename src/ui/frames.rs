//! The frames around the pattern editor: the right column, the upper
//! frame's views and the lower frame with its tabs.

use super::*;

impl App {
    /// The right column: the instrument list, and the disk
    /// browser below it.
    pub(super) fn right_column(&mut self, ui: &mut egui::Ui) {
        let h = ui.available_height();
        let w = ui.available_width();
        if self.panels.instruments {
            let list_h = if self.panels.browser { (h * 0.38).clamp(132.0, 360.0) } else { h };
            widgets::boxed(ui, "instruments", Vec2::new(w, list_h), |ui| instruments::panel(self, ui));
        }
        if self.panels.browser {
            if self.panels.instruments {
                ui.add_space(5.0);
            }
            widgets::boxed(ui, "browser", Vec2::new(w, ui.available_height()), |ui| {
                let now = self.now_playing();
                if now.is_some() {
                    ui.ctx().request_repaint();
                }
                if let Some(action) = self.browser.ui(ui, now) {
                    self.browser_action(action);
                }
            });
        }
    }

    /// The upper frame: the scopes.
    pub(super) fn upper_frame(&mut self, ui: &mut egui::Ui) {
        let h = 132.0;
        ui.horizontal(|ui| {
            widgets::boxed(ui, "scopes", Vec2::new(ui.available_width(), h), |ui| {
                ui.horizontal(|ui| {
                    let caption = match self.scope_view {
                        ScopeView::Scope => "MASTER SCOPE",
                        ScopeView::Tracks => "TRACK SCOPES",
                        ScopeView::Spectrum => "MASTER SPECTRUM",
                        ScopeView::Wave => "PLAYING SAMPLES",
                    };
                    theme::caption(ui, caption);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        for (view, label, tip) in ScopeView::ALL.into_iter().rev() {
                            if theme::toggle(ui, self.scope_view == view, label).on_hover_text(tip).clicked() {
                                self.scope_view = view;
                            }
                        }
                    });
                });
                match self.scope_view {
                    ScopeView::Scope => self.scopes(ui),
                    ScopeView::Tracks => trackscopes::panel(self, ui),
                    ScopeView::Spectrum => {
                        let frames = self.shared.scope.lock().map(|s| s.clone()).unwrap_or_default();
                        let sr = self.audio.as_ref().map_or(44100, |a| a.sample_rate) as f32;
                        spectrum::panel(&mut self.spectrum, ui, &frames, sr);
                    }
                    ScopeView::Wave => waveview::panel(self, ui),
                }
            });
        });
    }

    /// The lower frame's tabs, along the bottom of the window: a tab opens
    /// the frame on its page, the open one closes it.
    pub(super) fn lower_tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (tab, label, tip) in [
                (Lower::Automation, "Automation", "Envelopes that move parameters over the pattern"),
                (
                    Lower::TrackFx,
                    "Track FX",
                    "Each track's effects, the send tracks, and the master chain the whole mix goes through",
                ),
            ] {
                let open = self.show_lower && self.lower == tab;
                let button = egui::Button::selectable(open, label).min_size(Vec2::new(90.0, 20.0));
                let tip = format!("{tip} (Ctrl+3 shows or hides the lower frame)");
                if ui.add(button).on_hover_text(tip).clicked() {
                    self.show_lower = !open;
                    self.lower = tab;
                }
            }
        });
    }

    /// Opens the lower frame on track `t`'s effects.
    pub fn show_track_fx(&mut self, t: usize) {
        self.show_lower = true;
        self.lower = Lower::TrackFx;
        self.track_fx = TrackFx::Track(t);
        self.cursor.track = t;
        self.fx_cursor = t;
        self.clamp_cursor();
    }

    /// Opens the lower frame on send track `id`'s effects.
    pub fn show_send(&mut self, id: u8) {
        self.show_lower = true;
        self.lower = Lower::TrackFx;
        self.track_fx = TrackFx::Send(id);
    }

    /// Shows the chain of `owner`: an instrument's in the instrument
    /// editor, the others in the Track FX tab.
    pub fn show_chain(&mut self, owner: crate::project::Owner) {
        use crate::project::Owner;
        match owner {
            Owner::Track(t) => self.show_track_fx(t),
            Owner::Module(OUTPUT_ID) => {
                (self.show_lower, self.lower, self.track_fx) = (true, Lower::TrackFx, TrackFx::Master);
            }
            Owner::Module(id) if self.project.send_track(id).is_some() => self.show_send(id),
            Owner::Module(id) => {
                self.selected_module = Some(id);
                self.view = View::Sampler;
                self.sampler.synth_tab = super::sampler::SynthTab::Synth;
            }
        }
    }

    /// The Track FX tab: a chip for each track in its color, each send
    /// track and the master, over the chosen one's effects. It follows the
    /// cursor's track.
    pub(super) fn track_fx_page(&mut self, ui: &mut egui::Ui) {
        if let TrackFx::Send(id) = self.track_fx
            && self.project.send_track(id).is_none()
        {
            self.track_fx = TrackFx::Master;
        }
        if self.cursor.track != self.fx_cursor {
            self.fx_cursor = self.cursor.track;
            self.track_fx = TrackFx::Track(self.cursor.track);
        }
        ui.horizontal_wrapped(|ui| {
            for t in 0..self.pattern().num_tracks() {
                let n = self.project.tracks.get(t).map_or(0, |t| t.effects.len());
                let name = self.project.track_name(t);
                let text = if n > 0 { format!("{name}  {n}") } else { name };
                let on = self.track_fx == TrackFx::Track(t);
                let tip = format!("{} effect{} on this track; click to edit them", n, if n == 1 { "" } else { "s" });
                if widgets::chip(ui, pattern::track_color(&self.project, t), &text, on).on_hover_text(tip).clicked() {
                    self.track_fx = TrackFx::Track(t);
                    self.cursor.track = t;
                    self.clamp_cursor();
                    self.fx_cursor = t;
                }
            }
            ui.separator();
            let sends: Vec<u8> = self.project.sends.iter().map(|s| s.module).collect();
            for id in sends {
                let Some(m) = self.project.module(id) else { continue };
                let n = self.project.chain(id).effects.len();
                let text = if n > 0 { format!("{}  {n}", m.name) } else { m.name.clone() };
                let color = modules::module_color(m);
                let tip = "A send track: effects that what Sends send here goes through; click to edit them";
                if widgets::chip(ui, color, &text, self.track_fx == TrackFx::Send(id)).on_hover_text(tip).clicked() {
                    self.track_fx = TrackFx::Send(id);
                }
            }
            let tip = "Add a send track: effects, such as a reverb, that Sends in any chain send to";
            if ui.button("+ Send Track").on_hover_text(tip).clicked() {
                let name = self.project.next_send_name();
                match self.project.add_send_track(&name) {
                    Some(id) => {
                        self.track_fx = TrackFx::Send(id);
                        self.mark();
                    }
                    None => self.set_status("Too many modules"),
                }
            }
            ui.separator();
            let n = self.project.master.len();
            let text = if n > 0 { format!("Master  {n}") } else { "Master".into() };
            let tip = "The master chain: effects the whole mix goes through before the output";
            if widgets::chip(ui, theme::SELECTED, &text, self.track_fx == TrackFx::Master).on_hover_text(tip).clicked()
            {
                self.track_fx = TrackFx::Master;
            }
        });
        ui.add_space(3.0);
        let owner = match self.track_fx {
            TrackFx::Track(t) => crate::project::Owner::Track(t),
            TrackFx::Send(id) => crate::project::Owner::Module(id),
            TrackFx::Master => crate::project::Owner::Module(OUTPUT_ID),
        };
        chain::page(self, ui, owner);
    }

    /// The lower frame: the automation editor, or the tracks', send
    /// tracks' and master's effects.
    pub(super) fn lower_frame(&mut self, ui: &mut egui::Ui) {
        match self.lower {
            Lower::Automation => automation::panel(self, ui),
            Lower::TrackFx => self.track_fx_page(ui),
        }
    }

    /// Tabs that switch the middle of the window.
    /// With arrows at its ends that show and hide the frames around the
    /// middle: the sequencer on the left, the
    /// scopes above and the instrument list and disk browser on the right.
    pub(super) fn view_tabs(&mut self, ui: &mut egui::Ui) {
        use icons::Icon;
        ui.horizontal(|ui| {
            let (icon, tip) = if self.panels.sequencer { (Icon::Left, "Hide") } else { (Icon::Right, "Show") };
            if icons::button(ui, icon).on_hover_text(format!("{tip} the pattern sequencer (Ctrl+2)")).clicked() {
                self.panels.sequencer = !self.panels.sequencer;
            }
            for (view, label, key) in View::ALL {
                let tab = egui::Button::selectable(self.view == view, label).min_size(Vec2::new(110.0, 22.0));
                if ui.add(tab).on_hover_text(key).clicked() {
                    self.view = view;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let right = self.panels.instruments || self.panels.browser;
                let (icon, tip) = if right { (Icon::Right, "Hide") } else { (Icon::Left, "Show") };
                let tip = format!("{tip} the instrument list and disk browser (Ctrl+4, Ctrl+5)");
                if icons::button(ui, icon).on_hover_text(tip).clicked() {
                    (self.panels.instruments, self.panels.browser) = (!right, !right);
                }
                let (icon, tip) = if self.panels.upper { (Icon::Up, "Hide") } else { (Icon::Down, "Show") };
                let tip = format!("{tip} the scopes: track scopes, spectrum and wave view (Ctrl+1)");
                if icons::button(ui, icon).on_hover_text(tip).clicked() {
                    self.panels.upper = !self.panels.upper;
                }
            });
        });
        ui.add_space(3.0);
    }

    pub(super) fn browser_action(&mut self, action: browser::Action) {
        match action {
            browser::Action::Preview(path) => self.preview_file(&path),
            browser::Action::StopPreview => self.stop_preview(),
            browser::Action::PreviewSettings => self.send_preview_settings(),
            browser::Action::LoadSample(path) => {
                self.stop_preview();
                self.load_samples(vec![path]);
            }
            browser::Action::OpenSong(path) => self.request(open_or_import(path.to_string_lossy().into_owned())),
        }
    }
}
