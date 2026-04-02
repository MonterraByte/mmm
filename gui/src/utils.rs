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
use egui::{Vec2, ViewportBuilder, ViewportId};

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

pub(crate) use show_immediate;

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
