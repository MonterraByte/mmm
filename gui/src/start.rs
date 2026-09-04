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

use std::mem;
use std::path::Path;

use eframe::{App, egui};
use egui::{CentralPanel, Context, Ui, Vec2};
use rfd::AsyncFileDialog;
use tracing::error;

use mmm_edit::EditableInstance;
use mmm_edit::util::ErrorChainDisplay;

use crate::utils::{FilePicker, PickerResult};
use crate::{AppUi, ModManagerUi};

pub struct StartUi {
    state: State,
    picker: Option<FilePicker>,
}

enum State {
    Main,
    LoadInstance(EditableInstance),
}

impl StartUi {
    pub fn new() -> Self {
        Self { state: State::Main, picker: None }
    }

    pub fn enter_main_ui_if_instance_loaded(app_ui: &mut AppUi) -> bool {
        let AppUi::Start(start_ui) = app_ui else { unreachable!() };
        let load_instance = matches!(start_ui.state, State::LoadInstance(_));
        if load_instance {
            let State::LoadInstance(instance) = mem::replace(&mut start_ui.state, State::Main) else {
                unreachable!();
            };
            *app_ui = AppUi::ModManager(ModManagerUi::new(instance));
        }
        load_instance
    }
}

impl App for StartUi {
    fn logic(&mut self, _: &Context, _: &mut eframe::Frame) {
        if let Some(picker) = &mut self.picker {
            match picker.poll() {
                PickerResult::Pending => {}
                PickerResult::Ready(path) => {
                    self.picker = None;

                    if let Some(parent) = path.parent() {
                        self.load_instance(parent);
                    } else {
                        error!("selected path has no parent");
                    }
                }
                PickerResult::Closed => self.picker = None,
            }
        }
    }

    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        CentralPanel::default().show(ui, |ui| match self.state {
            State::Main => self.main_screen(ui, frame),
            State::LoadInstance(_) => {}
        });
    }
}

impl StartUi {
    pub const INITIAL_SIZE: Vec2 = Vec2::new(600.0, 450.0);

    fn main_screen(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        ui.horizontal(|ui| {
            if ui.button("Open instance").clicked() {
                let picker = AsyncFileDialog::new()
                    .set_parent(frame)
                    .add_filter("mmm.cbor file", &["cbor"]);
                self.picker = Some(FilePicker::new(picker));
            }
        });
    }

    fn load_instance(&mut self, path: &Path) {
        let instance = match EditableInstance::open(path) {
            Ok(instance) => instance,
            Err(err) => {
                error!("failed to load instance: {}", ErrorChainDisplay(&err));
                return;
            }
        };

        self.state = State::LoadInstance(instance);
    }
}
