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

use std::time::Instant;

use eframe::egui;
use egui::{Button, CornerRadius, Frame, Id, Label, Panel, Popup, ScrollArea, Sides, TextStyle, Ui, ViewportCommand};
use nary_tree::NodeId;

use mmm_edit::EditableInstance;
use mmm_edit::archive::{Archive, ExtractSelection};
use mmm_edit::install::fomod::module_config::{FileGroup, FileGroupType, PluginName};
use mmm_edit::install::fomod::{FomodInstaller, InstallStepState};
use mmm_edit::util::{EMPTY_STR, SharedStr};

use crate::install::Images;

pub struct FomodDialog {
    can_go_back: Option<bool>,
    can_go_forward: Option<bool>,
    description: Option<SharedStr>,
    image: Option<NodeId>,
    left_panel_id: Id,
    top_panel_id: Id,
}

impl FomodDialog {
    pub fn new() -> Self {
        Self {
            can_go_back: None,
            can_go_forward: None,
            description: None,
            image: None,
            left_panel_id: Id::new(("fomod_left", Instant::now())),
            top_panel_id: Id::new(("fomod_top", Instant::now())),
        }
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        archive: &Archive,
        fomod: &mut FomodInstaller,
        instance: &EditableInstance,
        images: &Images,
    ) -> Option<ExtractSelection> {
        let (step, step_state) = fomod
            .current_step()
            .expect("installer has been initialized and hasn't ended");
        let images = images.lock().expect("lock is not poisoned");
        let mut toggled = None;

        ui.heading(step.name.as_ref());

        Frame::new()
            .stroke(ui.style().visuals.window_stroke)
            .corner_radius(CornerRadius::same(4))
            .show(ui, |ui| {
                ui.set_max_height(ui.available_height() - ui.style().spacing.interact_size.y);

                Panel::left(self.left_panel_id)
                    .resizable(false)
                    .exact_size(ui.available_width() - 200.0)
                    .show(ui, |ui| {
                        let text_height = TextStyle::Body.resolve(ui.style()).size;

                        Panel::top(self.top_panel_id)
                            .min_size(text_height)
                            .max_size(ui.available_height() - 200.0)
                            .default_size(text_height * 4.0)
                            .resizable(true)
                            .show(ui, |ui| {
                                ScrollArea::vertical().show(ui, |ui| {
                                    ui.take_available_space();
                                    let description = self.description.as_ref().unwrap_or(&EMPTY_STR).as_ref();
                                    ui.add(Label::new(description).wrap());
                                });
                            });

                        if let Some(image_node) = &self.image
                            && let Some(image) = images.get(image_node)
                        {
                            ui.centered_and_justified(|ui| image.show(ui));
                        }
                    });

                ui.take_available_width();

                ScrollArea::vertical().show(ui, |ui| {
                    let mut first = true;
                    for group in step.file_groups.as_ref() {
                        if first {
                            first = false;
                        } else {
                            ui.separator();
                        }

                        if let Some(plugin_name) =
                            Self::show_file_group(ui, fomod, step_state, group, &mut self.description, &mut self.image)
                        {
                            toggled = Some((group.name.clone(), plugin_name));
                        }
                    }
                });
            });

        if let Some((group_name, plugin_name)) = toggled {
            fomod.toggle(group_name, plugin_name);
            self.can_go_forward = None;
        }

        self.show_buttons(ui, archive, fomod, instance)
    }

    fn show_file_group(
        ui: &mut Ui,
        fomod: &FomodInstaller,
        state: &InstallStepState,
        group: &FileGroup,
        description: &mut Option<SharedStr>,
        image: &mut Option<NodeId>,
    ) -> Option<PluginName> {
        let selection = state.selection(&group.name).expect("group exists");
        let mut toggled = None;

        ui.label(group.name.0.as_ref());

        if group.ty == FileGroupType::SelectAtLeastOne {
            ui.small("Select at least one");
        }

        for plugin in group.plugins.as_ref() {
            let mut selected = selection.contains(&plugin.name);
            let can_never_be_toggled = fomod.can_never_be_toggled(group, plugin, selected);
            let radio = group.ty == FileGroupType::SelectExactlyOne
                || (group.ty == FileGroupType::SelectAtMostOne && !selected);

            let response = ui
                .scope(|ui| {
                    if can_never_be_toggled {
                        // We just want to gray out the option, calling ui.disable() would also prevent hover interaction.
                        ui.multiply_opacity(ui.visuals().disabled_alpha());
                    }

                    if radio {
                        ui.radio(selected, plugin.name.0.as_ref())
                    } else {
                        ui.checkbox(&mut selected, plugin.name.0.as_ref())
                    }
                })
                .inner;

            if response.clicked() {
                toggled = Some(plugin.name.clone());
            }

            if response.hovered() {
                if let Some(current) = description
                    && std::ptr::addr_eq(current.as_ptr(), plugin.description.as_ptr())
                {
                    // already showing this description
                } else {
                    *description = Some(plugin.description.clone());
                }

                *image = plugin.image.iter().find_map(|img| img.node);
            }
        }

        toggled
    }

    fn show_buttons(
        &mut self,
        ui: &mut Ui,
        archive: &Archive,
        fomod: &mut FomodInstaller,
        instance: &EditableInstance,
    ) -> Option<ExtractSelection> {
        let mut selection = None;
        let mut allow_unusable_plugins = fomod.allow_unusable_plugins;
        let mut allow_disabling_required_plugins = fomod.allow_disabling_required_plugins;

        Sides::new().show(
            ui,
            |ui| {
                let response = ui.button("Options");
                Popup::menu(&response).show(|ui| {
                    ui.label("Hacks:");
                    ui.checkbox(&mut allow_unusable_plugins, "Allow selecting unusable plugins");
                    ui.checkbox(
                        &mut allow_disabling_required_plugins,
                        "Allow disabling required plugins",
                    );
                });
            },
            |ui| {
                if ui.button("Cancel").clicked() {
                    ui.send_viewport_cmd(ViewportCommand::Close);
                }

                let can_go_forward = *self.can_go_forward.get_or_insert_with(|| fomod.can_go_forward());
                if ui.add_enabled(can_go_forward, Button::new("Next")).clicked() {
                    selection = fomod
                        .next(archive, instance)
                        .expect("FomodInstaller::next can only fail on the first call");
                    self.can_go_back = None;
                    self.can_go_forward = None;
                    self.description = None;
                    self.image = None;
                }

                let can_go_back = *self.can_go_back.get_or_insert_with(|| fomod.can_go_back(instance));
                if ui.add_enabled(can_go_back, Button::new("Back")).clicked() {
                    fomod.back(instance);
                    self.can_go_back = None;
                    self.can_go_forward = None;
                    self.description = None;
                    self.image = None;
                }
            },
        );

        fomod.allow_unusable_plugins = allow_unusable_plugins;
        fomod.allow_disabling_required_plugins = allow_disabling_required_plugins;

        selection
    }
}
