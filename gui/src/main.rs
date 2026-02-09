// Copyright © 2025-2026 Joaquim Monteiro
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

#![forbid(unsafe_code)]

mod background_task;

use std::ffi::OsStr;
use std::fmt::Write;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

use anyhow::Context as _;
use clap::Parser;
use eframe::egui::{Id, Modal, Popup, Sides, TextStyle, TextWrapMode, TopBottomPanel};
use eframe::{App, Frame, NativeOptions, egui};
use egui::{Align, CentralPanel, Color32, Context, Layout, ScrollArea, Sense, Stroke, Ui};
use egui_extras::{Column, TableBuilder};
use foldhash::HashSet;
use tracing::{Level, error, info};
use tracing_subscriber::EnvFilter;

use mmm_core::instance::{Instance, ModDeclaration, ModIndex, ModOrderIndex};
use mmm_edit::EditableInstance;

use crate::background_task::{BackgroundTask, StatusString, spawn_background_thread};

const APP_NAME: &str = "zone.monterra.modmanager";

#[derive(Parser)]
struct Args {
    instance_path: PathBuf,
}

fn main() -> anyhow::Result<()> {
    tracing_setup();
    let instance = {
        let args = Args::parse();
        EditableInstance::open(&args.instance_path).context("failed to open instance")?
    };

    let mut options = NativeOptions::default();
    options.viewport.app_id = Some(APP_NAME.into()); // https://github.com/emilk/egui/issues/7872
    options.viewport.title = Some(format!("mmm — {}", instance.dir().display()));

    // https://github.com/emilk/egui/issues/5815
    if let Err(err) = eframe::run_native(APP_NAME, options, Box::new(|_ctx| Ok(ModManagerUi::new(instance)))) {
        error!("failed to create graphics context: {err}");
        std::process::exit(1);
    }

    Ok(())
}

pub struct ModManagerUi {
    instance: EditableInstance,
    background_task_queue: Sender<BackgroundTask>,
    background_task_status: StatusString,
    selection: HashSet<ModOrderIndex>,
    last_selected: Option<ModOrderIndex>,
    create_new_mod_modal: CreateNewModModal,
    rename_mod_modal: RenameModModal,
    remove_selected_mods_modal: RemoveSelectedModsModal,
}

impl ModManagerUi {
    fn new(instance: EditableInstance) -> Box<Self> {
        let (background_task_queue, background_task_status) =
            spawn_background_thread().expect("failed to spawn background task thread");

        Box::new(Self {
            instance,
            background_task_queue,
            background_task_status,
            selection: HashSet::default(),
            last_selected: None,
            create_new_mod_modal: CreateNewModModal::default(),
            rename_mod_modal: RenameModModal::default(),
            remove_selected_mods_modal: RemoveSelectedModsModal::default(),
        })
    }
}

impl App for ModManagerUi {
    fn update(&mut self, ctx: &Context, _frame: &mut Frame) {
        TopBottomPanel::bottom(Id::new("status")).show(ctx, |ui| {
            self.status_bar(ui);
        });

        CentralPanel::default().show(ctx, |ui| {
            self.center_panel(ui);
        });

        self.instance.save();
    }
}

impl ModManagerUi {
    fn center_panel(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let response = ui.button("Add mod");
            Popup::menu(&response).show(|ui| {
                if ui.button("Create empty mod").clicked() {
                    self.create_new_mod_modal.open = true;
                }
            });

            if ui.button("Rename selected").clicked()
                && let Some(selection) = self.get_single_selected_mod()
            {
                self.rename_mod_modal.open(&self.instance, selection);
            }

            if ui.button("Remove selected").clicked() {
                self.remove_selected_mods_modal.open(&self.instance, &self.selection);
            }

            if ui.button("Toggle selected").clicked() {
                for idx in self.selection.iter().copied() {
                    self.instance.toggle_mod_enabled(idx);
                }
            }
        });

        ui.separator();

        ScrollArea::horizontal().show(ui, |ui| {
            self.table_ui(ui);
        });

        self.create_empty_mod_modal(ui);
        self.rename_mod_modal(ui);
        self.remove_selected_mods_modal(ui);
    }

    fn table_ui(&mut self, ui: &mut Ui) {
        let (modifiers, pointer) = ui.input(|input| (input.modifiers, input.pointer.interact_pos()));

        let available_height = ui.available_height();
        let table = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::exact(18.0))
            .column(Column::remainder().at_least(40.0).clip(true).resizable(true))
            .column(Column::auto())
            .min_scrolled_height(0.0)
            .max_scroll_height(available_height)
            .drag_to_scroll(false)
            .sense(Sense::click_and_drag());

        #[derive(Copy, Clone)]
        struct ModDnDPayload;

        let mut dnd_hover_line = None;
        let mut dnd_drop_index = None;
        table
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.strong("Enabled");
                });
                header.col(|ui| {
                    ui.strong("Mod name");
                });
                header.col(|ui| {
                    ui.strong("Priority");
                });
            })
            .body(|body| {
                let mut entry_to_toggle = None;

                let total_rows = self.instance.mod_order().len();
                body.rows(18.0, total_rows, |mut row| {
                    let row_index = ModOrderIndex::from(row.index());
                    let order_entry = self.instance.mod_order()[row_index];
                    let mod_decl = &self.instance.mods()[order_entry.mod_index()];

                    row.set_selected(self.selection.contains(&row_index));

                    let mut enabled = order_entry.enabled;
                    row.col(|ui| {
                        ui.checkbox(&mut enabled, ());
                    });
                    if enabled != order_entry.enabled {
                        entry_to_toggle = Some(row_index);
                    }

                    row.col(|ui| {
                        ui.label(mod_decl.name().as_str());
                    });

                    row.col(|ui| {
                        ui.label(row_index.to_string());
                    });

                    let response = row.response();
                    if response.clicked() {
                        if modifiers.shift {
                            if let Some(last) = self.last_selected {
                                if !modifiers.ctrl {
                                    self.selection.clear();
                                }

                                let range = if row_index < last {
                                    row_index.inclusive_range_to(last)
                                } else {
                                    last.inclusive_range_to(row_index)
                                };
                                self.selection.extend(range);
                                self.last_selected = Some(row_index);
                            } else {
                                self.selection.insert(row_index);
                                self.last_selected = Some(row_index);
                            }
                        } else if modifiers.ctrl {
                            if !self.selection.contains(&row_index) {
                                self.selection.insert(row_index);
                                self.last_selected = Some(row_index);
                            } else {
                                self.selection.remove(&row_index);
                                self.last_selected = None;
                            }
                        } else {
                            self.selection.clear();
                            self.selection.insert(row_index);
                            self.last_selected = Some(row_index);
                        }
                    }

                    if response.drag_started() && !self.selection.contains(&row_index) {
                        self.selection.clear();
                        self.selection.insert(row_index);
                        self.last_selected = Some(row_index);
                    }

                    response.dnd_set_drag_payload(ModDnDPayload);

                    if response.dnd_hover_payload::<ModDnDPayload>().is_some()
                        && let Some(pointer) = pointer
                    {
                        let rect = response.rect;
                        if pointer.y <= rect.center().y {
                            // Above us
                            dnd_hover_line = Some((rect.x_range(), rect.top()));
                        } else {
                            // Below us
                            dnd_hover_line = Some((rect.x_range(), rect.bottom()));
                        }
                    }

                    if response.dnd_release_payload::<ModDnDPayload>().is_some()
                        && let Some(pointer) = pointer
                    {
                        if pointer.y <= response.rect.center().y {
                            dnd_drop_index = Some(row_index);
                        } else {
                            dnd_drop_index = Some(row_index.saturating_add(1u32));
                        }
                    }
                });

                if let Some(index) = entry_to_toggle {
                    self.instance.toggle_mod_enabled(index);
                }
            });

        if let Some((range, y)) = dnd_hover_line {
            const STROKE: Stroke = Stroke { width: 2.0, color: Color32::WHITE };
            ui.painter().hline(range, y, STROKE);
        }

        if let Some(drop_index) = dnd_drop_index {
            let selection_len = self.selection.len();
            let drop_index = self.instance.move_mods(&self.selection, drop_index);

            // indices are no longer valid
            self.selection.clear();
            self.selection
                .extend(drop_index.inclusive_range_to(drop_index.saturating_add(selection_len).saturating_sub(1u32)));
        }
    }

    fn create_empty_mod_modal(&mut self, ui: &mut Ui) {
        if !self.create_new_mod_modal.open {
            return;
        }

        let modal = Modal::new(Id::new("new_mod")).show(ui.ctx(), |ui| {
            ui.set_width(250.0);
            ui.heading("Create empty mod");
            ui.label("Name:");
            ui.text_edit_singleline(&mut self.create_new_mod_modal.input);

            Sides::new().show(
                ui,
                |_| (),
                |ui| {
                    if ui.button("Cancel").clicked() {
                        ui.close();
                    }

                    ui.add_enabled_ui(ModDeclaration::is_name_valid(&self.create_new_mod_modal.input), |ui| {
                        if ui.button("OK").clicked() {
                            if let Err(err) = self.instance.create_mod(&self.create_new_mod_modal.input) {
                                error!("failed to create mod '{}': {}", &self.create_new_mod_modal.input, err);
                            }
                            ui.close();
                        }
                    });
                },
            );
        });

        if modal.should_close() {
            self.create_new_mod_modal.open = false;
            self.create_new_mod_modal.input.clear();
        }
    }

    fn rename_mod_modal(&mut self, ui: &mut Ui) {
        if !self.rename_mod_modal.open {
            return;
        }

        let Some(idx) = self.get_single_selected_mod() else {
            self.rename_mod_modal.open = false;
            return;
        };

        let modal = Modal::new(Id::new("rename_mod")).show(ui.ctx(), |ui| {
            ui.set_width(250.0);
            ui.heading("Rename mod");

            let mod_idx = self.instance.mod_order()[idx].mod_index();
            let mod_decl = &self.instance.mods()[mod_idx];
            ui.horizontal(|ui| {
                ui.label("New name for");
                ui.label(mod_decl.name().as_str());
                ui.label(":");
            });
            ui.text_edit_singleline(&mut self.rename_mod_modal.input);

            Sides::new().show(
                ui,
                |_| (),
                |ui| {
                    if ui.button("Cancel").clicked() {
                        ui.close();
                    }

                    ui.add_enabled_ui(ModDeclaration::is_name_valid(&self.rename_mod_modal.input), |ui| {
                        if ui.button("OK").clicked() {
                            if let Err(err) = self.instance.rename_mod(mod_idx, &self.rename_mod_modal.input) {
                                error!("failed to rename mod to '{}': {}", &self.rename_mod_modal.input, err);
                            }
                            ui.close();
                        }
                    });
                },
            );
        });

        if modal.should_close() {
            self.rename_mod_modal.open = false;
            self.rename_mod_modal.input.clear();
        }
    }

    fn remove_selected_mods_modal(&mut self, ui: &mut Ui) {
        if !self.remove_selected_mods_modal.is_open() {
            return;
        }

        let modal = Modal::new(Id::new("remove_mod")).show(ui.ctx(), |ui| {
            ui.set_width(400.0);
            self.remove_selected_mods_modal.display(&self.instance, ui);

            Sides::new().show(
                ui,
                |_| (),
                |ui| {
                    if ui.button("Cancel").clicked() {
                        ui.close();
                    }

                    if ui.button("Delete").clicked() {
                        let task = self.remove_selected_mods_modal.do_task(&mut self.instance);
                        self.spawn_background_task(task);
                        self.selection.clear();
                        self.last_selected = None;

                        ui.close();
                    }
                },
            );
        });

        if modal.should_close() {
            self.remove_selected_mods_modal.close();
        }
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        let status = self.background_task_status.lock().expect("lock is not poisoned");
        ui.label(status.as_str());
    }

    fn spawn_background_task(&self, task: BackgroundTask) {
        if self.background_task_queue.send(task).is_err() {
            error!("background task panicked");
        }
    }

    fn get_single_selected_mod(&self) -> Option<ModOrderIndex> {
        if self.selection.len() != 1 {
            return None;
        }
        self.selection.iter().next().copied()
    }
}

#[derive(Debug, Default)]
struct CreateNewModModal {
    open: bool,
    input: String,
}

#[derive(Debug, Default)]
struct RenameModModal {
    open: bool,
    input: String,
}

impl RenameModModal {
    fn open(&mut self, instance: &EditableInstance, selected_mod: ModOrderIndex) {
        let mod_decl = instance.mod_by_order_index(selected_mod);
        self.input.clear();
        self.input.push_str(mod_decl.name());
        self.open = true;
    }
}

#[derive(Debug, Default)]
struct RemoveSelectedModsModal(Vec<ModIndex>);

impl RemoveSelectedModsModal {
    fn open(&mut self, instance: &EditableInstance, selection: &HashSet<ModOrderIndex>) {
        self.0.clear();
        self.0
            .extend(selection.iter().map(|idx| instance.mod_order()[*idx].mod_index()));
        self.0.sort_unstable_by_key(|idx| instance.mods()[*idx].name());
    }

    fn is_open(&self) -> bool {
        !self.0.is_empty()
    }

    fn close(&mut self) {
        self.0.clear();
    }

    fn display(&self, instance: &EditableInstance, ui: &mut Ui) {
        match self.0.len() {
            0 => unreachable!(),
            1 => {
                ui.heading("Remove mod");

                let mod_index = *self.0.first().expect("len is 1");
                let mod_decl = &instance.mods()[mod_index];
                ui.horizontal(|ui| {
                    ui.label(mod_decl.name().as_str());
                    ui.label("will be removed.");
                });
            }
            len => {
                ui.heading("Remove mods");
                ui.label("The following mods will be removed:");
                ui.add_space(4.0);

                const TEXT_STYLE: TextStyle = TextStyle::Body;
                let row_height = ui.text_style_height(&TEXT_STYLE);
                ScrollArea::both().show_rows(ui, row_height, len, |ui, rows| {
                    ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);

                    for idx in self.0.get(rows).expect("range is within bounds") {
                        let mod_decl = &instance.mods()[*idx];
                        ui.label(mod_decl.name().as_str());
                    }
                });
            }
        }
    }

    fn do_task(&mut self, instance: &mut EditableInstance) -> BackgroundTask {
        // Sort indices from largest to smallest so that they can be removed in order without being invalidated.
        self.0.sort_unstable_by(|a, b| b.cmp(a));
        let paths: Vec<_> = self.0.iter().filter_map(|idx| instance.remove_mod(*idx)).collect();
        self.0.clear();

        Box::new(move |status| {
            for path in paths {
                {
                    let mut s = status.lock().expect("lock is not poisoned");
                    s.clear();
                    let _ = write!(
                        s,
                        "Deleting mod {}",
                        path.file_name().unwrap_or(OsStr::new("?")).display()
                    );
                }

                info!("removing mod directory '{}'", path.display());
                if let Err(err) = fs::remove_dir_all(&path) {
                    error!("failed to delete '{}': {}", path.display(), err);
                }
            }
        })
    }
}

fn tracing_setup() {
    let filter = EnvFilter::builder()
        .with_default_directive(Level::DEBUG.into())
        .from_env()
        .expect("invalid logging configuration");

    let collector = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .finish();
    tracing::subscriber::set_global_default(collector).expect("failed to set global logger");
}
