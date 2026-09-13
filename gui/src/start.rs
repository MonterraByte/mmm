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

use std::fmt::Write;
use std::fs;
use std::io;
use std::mem;
use std::path::Path;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use eframe::{App, egui};
use egui::{
    CentralPanel, ComboBox, Context, Frame, Id, Label, Margin, Popup, RichText, ScrollArea, Sense, Stroke, Ui,
    UiBuilder, Vec2,
};
use rfd::AsyncFileDialog;
use tracing::error;

use mmm_edit::EditableInstance;
use mmm_edit::game_providers::{InstalledSteamGame, Steam};
use mmm_edit::instances::{Instances, InstancesShared, Metadata};
use mmm_edit::util::{ErrorChainDisplay, LockExt};

use crate::utils::{FilePicker, FrameWithButtons, Navigate, PathDisplay, PickerResult, show_error_modal};
use crate::widgets::path_input::{PathChanged, PathInput};
use crate::{AppUi, ModManagerUi};

pub struct StartUi {
    state: State,
    instances: InstancesShared,
    picker: Option<FilePicker>,
    text_buffer: String,
    error: String,
    steam: SteamThread,
}

#[allow(clippy::large_enum_variant, reason = "State will eventually be LoadInstance")]
enum State {
    Main,
    PickGame {
        game_path: PathInput,
        source: GameSource,
        can_go_forward: Option<bool>,
    },
    LoadInstance(EditableInstance),
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum GameSource {
    None,
    Steam(Option<usize>), // index of the game in `Steam::games`
}

impl StartUi {
    pub fn new() -> Self {
        Self {
            state: State::Main,
            instances: Instances::get(),
            picker: None,
            steam: SteamThread::None,
            text_buffer: String::new(),
            error: String::new(),
        }
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
            State::PickGame { .. } => self.pick_game(ui, frame),
            State::LoadInstance(_) => {}
        });

        show_error_modal(ui, Id::new("start_error"), &mut self.error);
    }
}

impl StartUi {
    pub const INITIAL_SIZE: Vec2 = Vec2::new(600.0, 450.0);

    fn main_screen(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        ui.horizontal(|ui| {
            if ui.button("New instance").clicked() {
                self.state = State::PickGame {
                    game_path: PathInput::new(),
                    source: GameSource::Steam(None),
                    can_go_forward: Some(false),
                };
            }
            if ui.button("Open instance").clicked() {
                let picker = AsyncFileDialog::new()
                    .set_parent(frame)
                    .add_filter("mmm.cbor file", &["cbor"]);
                self.picker = Some(FilePicker::new(picker));
            }
        });

        ui.add_space(12.0);

        let error_color = ui.visuals().error_fg_color;
        let menu_margin = ui.spacing().menu_margin;
        let mut instances = self.instances.lock_expect();
        let mut instance_to_load = None;
        let mut instance_to_remove = None;

        let mut show_instance = |ui: &mut Ui, location: &Arc<Path>, metadata: Option<&Metadata>| {
            self.text_buffer.clear();
            let _ = write!(&mut self.text_buffer, "{}", PathDisplay(location));

            let response = ui
                .scope_builder(UiBuilder::new().id_salt(location).sense(Sense::click()), |ui| {
                    let response = ui.response();
                    let visuals = ui.style().interact(&response);

                    Frame::canvas(ui.style())
                        .fill(visuals.bg_fill.gamma_multiply(0.3))
                        .stroke(Stroke::new(1.0, visuals.bg_stroke.color))
                        .inner_margin(menu_margin)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());

                            ui.horizontal(|ui| {
                                ui.vertical(|ui| match metadata {
                                    Some(Ok(m)) => {
                                        ui.add(
                                            Label::new(RichText::new(m.name.as_str()).strong())
                                                .wrap()
                                                .selectable(false),
                                        );
                                        ui.add(
                                            Label::new(RichText::new(&self.text_buffer).small())
                                                .wrap()
                                                .selectable(false),
                                        );
                                    }
                                    Some(Err(err)) => {
                                        ui.add(Label::new(&self.text_buffer).wrap().selectable(false));

                                        self.text_buffer.clear();
                                        let _ = write!(&mut self.text_buffer, "{:#}", ErrorChainDisplay(err));
                                        ui.add(Label::new(RichText::new(&self.text_buffer).color(error_color)).wrap());
                                    }
                                    None => {
                                        ui.spinner();
                                        ui.add(
                                            Label::new(RichText::new(&self.text_buffer).small())
                                                .wrap()
                                                .selectable(false),
                                        );
                                    }
                                });

                                ui.add_space(ui.available_width() - 24.0);

                                let response = ui.button("...");
                                Popup::menu(&response).show(|ui| {
                                    if ui.button("Open in file manager").clicked()
                                        && let Err(err) = open::that_detached(location.as_os_str())
                                    {
                                        error!("failed to launch file manager: {}", ErrorChainDisplay(&err));
                                    }

                                    if ui.button("Remove from list").clicked() {
                                        instance_to_remove = Some(Arc::clone(location));
                                    }
                                });
                            });
                        });
                })
                .response;

            if response.clicked() {
                instance_to_load = Some(Arc::clone(location));
            }
        };

        match instances.iter() {
            Ok(iter) => {
                ScrollArea::vertical().show(ui, |ui| {
                    for (location, metadata) in iter {
                        show_instance(ui, location, metadata);
                    }
                });
            }
            Err(err) => {
                self.text_buffer.clear();
                let _ = write!(&mut self.text_buffer, "{}", err);
                ui.add(Label::new(RichText::new(&self.text_buffer).color(error_color)).wrap());
            }
        }

        if let Some(path) = instance_to_remove {
            instances.remove(&path);
        }

        drop(instances);

        if let Some(path) = instance_to_load {
            self.load_instance(path.as_ref());
        }
    }

    fn pick_game(&mut self, ui: &mut Ui, frame: &eframe::Frame) {
        let State::PickGame { game_path, can_go_forward, source } = &mut self.state else {
            unreachable!();
        };

        let steam = self.steam.get();

        let nav = FrameWithButtons::new(ui)
            .with_frame(|frame| frame.inner_margin(Margin::same(8)))
            .show_navigable(ui, |ui| {
                ui.heading("Select game");

                ui.horizontal_wrapped(|ui| {
                    ui.label("Source:");

                    let enable_steam = steam.is_none_or(Steam::found);
                    if !enable_steam && matches!(source, GameSource::Steam(_)) {
                        *source = GameSource::None;
                    }
                    ui.add_enabled_ui(enable_steam, |ui| {
                        ui.radio_value(
                            source,
                            if matches!(source, GameSource::Steam(_)) {
                                *source
                            } else {
                                GameSource::Steam(None)
                            },
                            "Steam",
                        )
                    });

                    ui.radio_value(source, GameSource::None, "Local");
                });

                match source {
                    GameSource::None => {
                        ui.horizontal(|ui| {
                            ui.label("Game directory:");
                            if game_path.ui(ui, frame) == PathChanged::Yes {
                                *can_go_forward = None;
                            }
                        });
                    }
                    GameSource::Steam(game_idx) => {
                        if let Some(steam) = steam {
                            let mut set_game = |game: InstalledSteamGame| {
                                game_path.clear_and_insert(|path| game.push_path(path));
                                *can_go_forward = None;
                            };

                            let game_idx = game_idx.get_or_insert_with(|| {
                                let game = steam.games().next().expect("there is at least one game");
                                set_game(game);
                                0
                            });

                            ui.horizontal_wrapped(|ui| {
                                ui.label("Game:");
                                ComboBox::from_id_salt("steam_game")
                                    .selected_text(steam.game(*game_idx).name.as_str())
                                    .show_ui(ui, |ui| {
                                        for (idx, game) in steam.games().enumerate() {
                                            let response =
                                                ui.selectable_value(game_idx, idx, game.data().name.as_str());
                                            if response.changed() {
                                                set_game(game);
                                            }
                                        }
                                    });
                            });
                            ui.label(game_path.as_str());
                        } else {
                            ui.horizontal_wrapped(|ui| {
                                ui.spinner();
                                ui.label("Finding installed games");
                            });
                        }
                    }
                }

                *can_go_forward.get_or_insert_with(|| {
                    if let GameSource::Steam(idx) = source
                        && idx.is_none()
                    {
                        return false;
                    }

                    let path = game_path.value();
                    if path.is_empty() || path.is_relative() || path.file_name().is_none() {
                        return false;
                    }

                    match fs::metadata(path) {
                        Ok(meta) => meta.is_dir(),
                        Err(err) if err.kind() == io::ErrorKind::NotFound => false,
                        Err(err) => {
                            error!("failed to get metadata of '{}': {}", game_path, err);
                            false
                        }
                    }
                })
            });

        match nav {
            Some(Navigate::Back) => self.state = State::Main,
            Some(Navigate::Forward) => {
                todo!();
            }
            None => {}
        }
    }

    fn load_instance(&mut self, path: &Path) {
        let instance = match EditableInstance::open(path) {
            Ok(instance) => instance,
            Err(err) => {
                error!("failed to load instance: {}", ErrorChainDisplay(&err));
                self.error.clear();
                let _ = write!(
                    &mut self.error,
                    "Failed to load instance '{}':\n\t- {:#}",
                    path.display(),
                    ErrorChainDisplay(&err)
                );
                return;
            }
        };

        self.instances.lock_expect().add_or_touch(&instance);
        self.state = State::LoadInstance(instance);
    }
}

enum SteamThread {
    None,
    Pending(Option<JoinHandle<Steam>>),
    Some(Steam),
    Err,
}

impl SteamThread {
    pub fn get(&mut self) -> Option<&Steam> {
        match self {
            SteamThread::None => {
                *self = Self::Pending(Some(thread::spawn(Steam::get)));
                None
            }
            SteamThread::Pending(handle) => {
                if handle.as_ref().expect("not joined yet").is_finished() {
                    let handle = handle.take().expect("not joined yet");
                    if let Ok(steam) = handle.join() {
                        *self = Self::Some(steam);
                        return self.get();
                    }

                    *self = Self::Err;
                    error!("Steam thread panicked");
                }
                None
            }
            SteamThread::Some(steam) => Some(steam),
            SteamThread::Err => None,
        }
    }
}
