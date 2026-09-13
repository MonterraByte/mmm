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

#![expect(unused)]

use std::borrow::Cow;
use std::fmt;
use std::mem;
use std::path::{Path, PathBuf};

use eframe::egui;
use egui::{Button, Pos2, Rect, TextEdit, Ui, Vec2};
use rfd::AsyncFileDialog;

use crate::utils::{FilePicker, PickerResult};

pub struct PathInput {
    inner: PathInputRepr,
    picker: Option<FilePicker>,
    picker_button_width: Option<f32>,
}

// egui doesn't support `PathBuf` input fields.
// We default to a String representation, and fall back to a non-editable
// `PathBuf` representation if a non-Unicode path is selected with the file picker.
enum PathInputRepr {
    String(String),
    PathBuf(PathBuf),
}

impl PathInput {
    pub const fn new() -> Self {
        Self {
            inner: PathInputRepr::String(String::new()),
            picker: None,
            picker_button_width: None,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, frame: &eframe::Frame) -> PathChanged {
        const PICKER_BUTTON_LABEL: &str = "…";
        let mut changed = PathChanged::No;

        if let Some(p) = &mut self.picker {
            match p.poll() {
                PickerResult::Pending => {}
                PickerResult::Ready(picked) => {
                    self.assign(picked);
                    self.picker = None;
                    changed = PathChanged::Yes;
                }
                PickerResult::Closed => self.picker = None,
            }
        }

        match &mut self.inner {
            PathInputRepr::String(path) => {
                let button_width = *self.picker_button_width.get_or_insert_with(|| {
                    let rect = Rect::from_min_size(Pos2::new(-1000.0, -1000.0), Vec2::ONE);
                    ui.place(rect, Button::new(PICKER_BUTTON_LABEL)).rect.width()
                });
                let input_width = ui
                    .available_width()
                    .algebraic_sub(button_width)
                    .algebraic_sub(ui.spacing().item_spacing.x);

                let response = ui.add(TextEdit::singleline(path).desired_width(input_width));
                if response.changed() {
                    changed = PathChanged::Yes;
                }
            }
            PathInputRepr::PathBuf(_) => {
                ui.label(self.as_str());
            }
        }

        if ui.button(PICKER_BUTTON_LABEL).clicked() {
            let dialog = AsyncFileDialog::new().set_parent(frame);
            self.picker = Some(FilePicker::new_directory(dialog));
        }

        changed
    }

    pub fn value(&self) -> &Path {
        match &self.inner {
            PathInputRepr::String(string) => Path::new(string),
            PathInputRepr::PathBuf(path) => path,
        }
    }

    pub fn as_str(&self) -> Cow<'_, str> {
        match &self.inner {
            PathInputRepr::String(string) => Cow::Borrowed(string),
            PathInputRepr::PathBuf(path) => Cow::Owned(path.display().to_string()),
        }
    }

    pub fn into_path(self) -> PathBuf {
        match self.inner {
            PathInputRepr::String(string) => PathBuf::from(string),
            PathInputRepr::PathBuf(path) => path,
        }
    }

    pub fn assign(&mut self, path: PathBuf) {
        match path.into_string() {
            Ok(string) => self.inner = PathInputRepr::String(string),
            Err(path) => self.inner = PathInputRepr::PathBuf(path),
        }
    }

    pub fn clear_and_insert(&mut self, f: impl FnOnce(&mut PathBuf)) {
        let mut path = match &mut self.inner {
            PathInputRepr::String(string) => {
                string.clear();
                PathBuf::from(mem::take(string))
            }
            PathInputRepr::PathBuf(path) => {
                path.clear();
                mem::take(path)
            }
        };

        f(&mut path);

        match path.into_string() {
            Ok(string) => self.inner = PathInputRepr::String(string),
            Err(path) => self.inner = PathInputRepr::PathBuf(path),
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum PathChanged {
    Yes,
    No,
}

impl fmt::Display for PathInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.inner {
            PathInputRepr::String(str) => f.write_str(str),
            PathInputRepr::PathBuf(path) => write!(f, "{}", path.display()),
        }
    }
}

pub struct AutoPathInput {
    path: PathInput,
    base: Option<PathBuf>,
}

impl AutoPathInput {
    pub fn new(base: PathBuf, name: &str) -> Self {
        let mut input = Self { path: PathInput::new(), base: Some(base) };
        input.update(name);
        input
    }

    pub fn ui(&mut self, ui: &mut Ui, frame: &eframe::Frame) -> PathChanged {
        let changed = self.path.ui(ui, frame);
        if changed == PathChanged::Yes {
            self.base = None;
        }
        changed
    }

    pub fn update(&mut self, name: &str) {
        let Some(base) = &self.base else {
            return;
        };

        self.path.clear_and_insert(|path| {
            path.push(base);

            if name.contains('/') {
                // prevent accidental nested directories
                path.push(name.replace('/', "⧸"));
            } else {
                path.push(name);
            }
        });
    }

    pub fn value(&self) -> &Path {
        self.path.value()
    }
}
