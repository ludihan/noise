//! Colors and widget styling: flat dark
//! greys, amber for whatever is selected, and a near-black pattern editor.

use eframe::egui::{self, Color32, CornerRadius, FontId, Stroke, Vec2};

pub const BODY: Color32 = Color32::from_rgb(40, 40, 40);
pub const FRAME_BG: Color32 = Color32::from_rgb(28, 28, 28);
pub const FRAME_LINE: Color32 = Color32::from_rgb(58, 58, 58);
pub const INSET: Color32 = Color32::from_rgb(16, 16, 16);
pub const BUTTON: Color32 = Color32::from_rgb(62, 62, 62);
pub const BUTTON_HOVER: Color32 = Color32::from_rgb(80, 80, 80);
pub const TEXT: Color32 = Color32::from_rgb(205, 205, 205);
pub const TEXT_WEAK: Color32 = Color32::from_rgb(130, 130, 130);
/// The "selected" color, used for active buttons and list selections.
pub const SELECTED: Color32 = Color32::from_rgb(222, 166, 72);
pub const SELECTED_TEXT: Color32 = Color32::from_rgb(24, 20, 14);
pub const RECORD: Color32 = Color32::from_rgb(214, 62, 52);
pub const SCOPE: Color32 = Color32::from_rgb(150, 220, 120);
/// The filled part of parameter bars, and while hovered or dragged.
pub const BAR: Color32 = Color32::from_rgb(98, 80, 50);
pub const BAR_HOT: Color32 = Color32::from_rgb(134, 106, 60);

// Pattern editor.
pub const PAT_BG: Color32 = Color32::from_rgb(14, 14, 14);
pub const PAT_BEAT: Color32 = Color32::from_rgb(30, 30, 30);
pub const PAT_BAR: Color32 = Color32::from_rgb(40, 40, 40);
pub const PAT_CURSOR_ROW: Color32 = Color32::from_rgb(58, 58, 64);
pub const PAT_CURSOR_ROW_EDIT: Color32 = Color32::from_rgb(86, 36, 34);
pub const PAT_PLAY_ROW: Color32 = Color32::from_rgb(52, 70, 40);
pub const PAT_SELECTION: Color32 = Color32::from_rgb(38, 58, 94);
pub const PAT_CURSOR: Color32 = Color32::from_rgb(222, 166, 72);
pub const PAT_EMPTY: Color32 = Color32::from_rgb(70, 70, 70);
pub const PAT_NOTE: Color32 = Color32::from_rgb(232, 232, 232);
pub const PAT_INSTRUMENT: Color32 = Color32::from_rgb(232, 190, 110);
pub const PAT_VOLUME: Color32 = Color32::from_rgb(130, 210, 120);
pub const PAT_EFFECT: Color32 = Color32::from_rgb(120, 180, 250);
pub const PAT_PAN: Color32 = Color32::from_rgb(214, 140, 214);
pub const PAT_DELAY: Color32 = Color32::from_rgb(120, 214, 214);
pub const PAT_LINE_NUMBER: Color32 = Color32::from_rgb(110, 110, 110);

/// Track header colors, cycled through.
pub const TRACK_COLORS: [Color32; 8] = [
    Color32::from_rgb(214, 104, 82),
    Color32::from_rgb(222, 166, 72),
    Color32::from_rgb(170, 196, 82),
    Color32::from_rgb(86, 186, 130),
    Color32::from_rgb(78, 170, 200),
    Color32::from_rgb(110, 128, 214),
    Color32::from_rgb(166, 110, 206),
    Color32::from_rgb(208, 100, 160),
];

pub fn track_color(track: usize) -> Color32 {
    TRACK_COLORS[track % TRACK_COLORS.len()]
}

/// The menus' style: what is on (a view, a part of the window) is shown in
/// the selected color on a dark fill rather than dark on a bright one, so
/// its shortcut stays readable beside it.
pub fn menu_style(style: &mut egui::Style) {
    egui::containers::menu::menu_style(style);
    style.visuals.selection.bg_fill = Color32::from_rgb(70, 54, 28);
    style.visuals.selection.stroke = Stroke::new(1.0, SELECTED);
}

pub fn setup(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BODY;
    v.window_fill = BODY;
    v.window_stroke = Stroke::new(1.0, FRAME_LINE);
    v.extreme_bg_color = INSET;
    v.faint_bg_color = Color32::from_rgb(34, 34, 34);
    v.override_text_color = None;
    v.selection.bg_fill = SELECTED;
    v.selection.stroke = Stroke::new(1.0, SELECTED_TEXT);
    v.hyperlink_color = SELECTED;
    v.slider_trailing_fill = true;
    v.window_corner_radius = CornerRadius::same(3);
    v.menu_corner_radius = CornerRadius::same(3);

    let radius = CornerRadius::same(2);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = FRAME_BG;
    w.noninteractive.weak_bg_fill = FRAME_BG;
    w.noninteractive.bg_stroke = Stroke::new(1.0, FRAME_LINE);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    for (s, fill) in [
        (&mut w.inactive, BUTTON),
        (&mut w.hovered, BUTTON_HOVER),
        (&mut w.active, Color32::from_rgb(100, 100, 100)),
        (&mut w.open, BUTTON_HOVER),
    ] {
        s.bg_fill = fill;
        s.weak_bg_fill = fill;
        s.corner_radius = radius;
        // egui draws a frameless button (a menu entry, an unselected
        // toggle) with a frame only while it is hovered or pressed, and
        // that frame's stroke would grow it by its width. An expansion
        // equal to the stroke width cancels that out, so nothing around a
        // button moves when the mouse goes over it.
        s.expansion = 1.0;
    }
    w.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(30, 30, 30));
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(120, 120, 120));
    w.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    w.active.bg_stroke = Stroke::new(1.0, SELECTED);
    w.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    ctx.set_visuals(v);

    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = Vec2::new(5.0, 4.0);
        s.spacing.button_padding = Vec2::new(6.0, 2.0);
        s.spacing.interact_size.y = 18.0;
    });
}

/// The small caption at the top of a box.
pub fn caption(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(FontId::proportional(11.0)).color(TEXT_WEAK).strong());
}

/// A toggle drawn in the selected color when on.
pub fn toggle(ui: &mut egui::Ui, on: bool, text: impl Into<egui::WidgetText>) -> egui::Response {
    ui.add(egui::Button::selectable(on, text))
}

#[cfg(test)]
mod tests {
    use eframe::egui::{self, Event, Pos2, RawInput, Rect, Vec2};

    /// Runs a frame of a row of buttons with the pointer at `pointer`: an
    /// unselected toggle, a frameless button and a button after them.
    /// Returns where the toggle and the last button were laid out.
    fn row(ctx: &egui::Context, pointer: Option<Pos2>) -> (Rect, Rect) {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0))),
            events: pointer.map(Event::PointerMoved).into_iter().collect(),
            ..Default::default()
        };
        let mut rects = (Rect::NOTHING, Rect::NOTHING);
        let mut output = ctx.run_ui(input, |ui| {
            ui.horizontal(|ui| {
                let toggle = ui.add(egui::Button::selectable(false, "Follow")).rect;
                ui.add(egui::Button::new("Menu").frame(false));
                rects = (toggle, ui.add(egui::Button::new("Next")).rect);
            });
        });
        output.textures_delta.clear();
        rects
    }

    #[test]
    fn hovering_a_button_moves_nothing() {
        let ctx = egui::Context::default();
        super::setup(&ctx);
        let (toggle, idle) = row(&ctx, None);
        for at in [toggle.center(), Pos2::new(toggle.right() + 20.0, toggle.center().y)] {
            // The hover shows from the frame after the pointer gets there.
            row(&ctx, Some(at));
            assert_eq!(row(&ctx, Some(at)).1, idle);
        }
    }
}
