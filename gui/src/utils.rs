// Copyright © 2026 Joaquim Monteiro
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

use std::cell::Cell;

use eframe::egui;
use egui::{Align, CornerRadius, Frame, Label, ScrollArea, Sides, Ui, Vec2, ViewportBuilder, ViewportId};

pub struct Viewport {
    pub id: ViewportId,
    pub builder: Cell<ViewportBuilder>,
}

impl Viewport {
    #[must_use]
    pub fn new(id: ViewportId, title: String, size: Option<Vec2>) -> Box<Viewport> {
        let mut builder = ViewportBuilder::default()
            .with_app_id(crate::APP_NAME)
            .with_title(title);
        builder.inner_size = size;

        Box::new(Viewport { id, builder: Cell::new(builder) })
    }
}

macro_rules! show_immediate {
    ($viewport:expr, $ui:expr, $callback:expr) => {{
        $ui.show_viewport_immediate($viewport.id, $viewport.builder.take(), |ui, viewport| {
            if ui.input(|i| i.viewport().close_requested()) {
                return ViewportResult::Drop;
            }
            ($callback)(ui, viewport);
            ViewportResult::Keep
        })
    }};
}

macro_rules! show_immediate_panel {
    ($viewport:expr, $ui:expr, $callback:expr) => {{
        $ui.show_viewport_immediate($viewport.id, $viewport.builder.take(), |ui, _| {
            if ui.input(|i| i.viewport().close_requested()) {
                return ViewportResult::Drop;
            }
            ::eframe::egui::CentralPanel::default().show(ui, $callback);
            ViewportResult::Keep
        })
    }};
}

pub(crate) use show_immediate;
pub(crate) use show_immediate_panel;

pub enum ViewportResult {
    Drop,
    Keep,
}

impl From<ViewportResult> for bool {
    #[inline]
    fn from(value: ViewportResult) -> Self {
        matches!(value, ViewportResult::Keep)
    }
}

pub fn show_frame_with_buttons(
    ui: &mut Ui,
    add_frame_contents: impl FnOnce(&mut Ui),
    add_left_buttons: impl FnOnce(&mut Ui),
    add_right_buttons: impl FnOnce(&mut Ui),
) {
    Frame::new()
        .stroke(ui.style().visuals.window_stroke)
        .corner_radius(CornerRadius::same(4))
        .show(ui, |ui| {
            ui.set_max_height(ui.available_height() - ui.style().spacing.interact_size.y);
            add_frame_contents(ui);
        });

    Sides::new().show(ui, add_left_buttons, add_right_buttons);
}

pub fn show_error_message(ui: &mut Ui, err: &str) {
    ScrollArea::both().show(ui, |ui| {
        ui.centered_and_justified(|ui| {
            ui.add(Label::new(err).extend().halign(Align::Min));
        })
    });
}
