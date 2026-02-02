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

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::Context as _;
use clap::Parser;
use eframe::{App, Frame, NativeOptions, egui};
use egui::{Align, CentralPanel, Color32, Context, Layout, ScrollArea, Sense, Stroke, Ui};
use egui_extras::{Column, TableBuilder};
use tracing::{Level, error};
use tracing_subscriber::EnvFilter;

use mmm_core::instance::{Instance, ModOrderIndex};
use mmm_edit::EditableInstance;

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
    selection: HashSet<ModOrderIndex>,
    last_selected: Option<ModOrderIndex>,
}

impl ModManagerUi {
    fn new(instance: EditableInstance) -> Box<Self> {
        Box::new(Self {
            instance,
            selection: HashSet::default(),
            last_selected: None,
        })
    }
}

impl App for ModManagerUi {
    fn update(&mut self, ctx: &Context, _frame: &mut Frame) {
        CentralPanel::default().show(ctx, |ui| {
            self.center_panel(ui);
        });
    }
}

impl ModManagerUi {
    fn center_panel(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
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

        self.instance.save();
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
