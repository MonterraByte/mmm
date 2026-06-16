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

//! Mod installation dialog.

use std::ffi::OsStr;
use std::fmt::Write;
use std::io;
use std::mem;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::task::{Context, Poll};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use anyhow::Context as _;
use compact_str::CompactString;
use eframe::egui;
use egui::{
    CentralPanel, Checkbox, CornerRadius, Frame, RichText, Sides, TextStyle, Ui, Vec2, ViewportCommand, ViewportId,
};
use foldhash::HashMap;
use futures::task::noop_waker;
use nary_tree::NodeId;
use nary_tree::iter_mut::Lender;
use rfd::AsyncFileDialog;
use tracing::{debug, error};

use mmm_core::file_tree::{Counters, FileTree, TreeNodeKind, TreeNodeRef};
use mmm_core::instance::Instance;
use mmm_edit::EditableInstance;
use mmm_edit::archive::{Archive, ExtractSelection};
use mmm_edit::install::staging::StagedInstall;
use mmm_edit::util::node_ord;

use crate::ModManagerUi;
use crate::background_task::{BackgroundTask, Finalizer, StatusString};
use crate::tree::{TreeDisplay, dnd_handle_actions_fn};
use crate::utils::{Viewport, ViewportResult, show_immediate};

pub struct OngoingModInstallation {
    viewport: Option<Box<Viewport>>,
    state: State,
    background_task_queue: Sender<BackgroundTask>,
}

#[allow(
    clippy::large_enum_variant,
    reason = "each instance will go through every enum variant unless canceled"
)]
enum State {
    FilePicker(Pin<Box<dyn Future<Output = Option<rfd::FileHandle>> + Send>>),
    Opening {
        handle: Option<JoinHandle<anyhow::Result<Archive>>>,
        counter: Arc<Counters>,
        previous_count: usize,
        text: CompactString,
        path: Arc<Path>,
    },
    ExtractDialog {
        mod_name: String,
        mod_already_exists: Option<bool>,
        archive: Archive,
        extract_selection: ExtractSelection,
        tree_display: TreeDisplay,
        dir_checkbox_cache: HashMap<NodeId, CheckboxState>,
    },
    Closing,
    Error(Box<str>),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum CheckboxState {
    Partial,
    Unchecked,
}

impl OngoingModInstallation {
    pub fn new_with_file_picker(frame: &eframe::Frame, background_task_queue: Sender<BackgroundTask>) -> Self {
        let picker = AsyncFileDialog::new()
            .add_filter("Archive file", &["7z", "rar", "tar", "zip"])
            .set_parent(frame)
            .pick_file();
        let picker = Box::pin(picker);

        Self {
            viewport: None,
            state: State::FilePicker(picker),
            background_task_queue,
        }
    }

    fn new_opening_state(path: Arc<Path>) -> Result<State, io::Error> {
        let counter = Counters::new();

        let path_clone = Arc::clone(&path);
        let counter_clone = Arc::clone(&counter);
        let handle = thread::Builder::new().spawn(move || {
            let path = path_clone;
            let counter = counter_clone;

            debug!("opening archive '{}' for installation", path.display());
            let archive = Archive::open(path, counter).context("failed to open archive")?;
            Ok(archive)
        })?;

        Ok(State::Opening {
            handle: Some(handle),
            counter,
            previous_count: 0,
            text: CompactString::const_new("Opening archive..."),
            path,
        })
    }

    pub fn update(&mut self, ui: &mut Ui, instance: &EditableInstance) -> ViewportResult {
        match &mut self.state {
            State::FilePicker(picker) => match picker.as_mut().poll(&mut Context::from_waker(&noop_waker())) {
                Poll::Pending => ViewportResult::Keep,
                Poll::Ready(Some(file)) => {
                    let path: Arc<Path> = PathBuf::from(file).into();
                    match Self::new_opening_state(path) {
                        Ok(new_state) => {
                            self.state = new_state;
                            ViewportResult::Keep
                        }
                        Err(err) => {
                            error!(?err, "failed to spawn thread");
                            ViewportResult::Drop
                        }
                    }
                }
                Poll::Ready(None) => ViewportResult::Drop,
            },
            State::Opening { handle, counter, previous_count, text, path } => {
                let viewport = self.viewport.get_or_insert_with(|| {
                    Viewport::new(
                        ViewportId::from_hash_of(("install", &path, Instant::now())),
                        format!(
                            "mmm — Installing archive {}",
                            path.file_name().and_then(OsStr::to_str).unwrap_or_default(),
                        ),
                        Some(Vec2::new(750.0, 450.0)),
                    )
                });

                if handle.as_ref().expect("not joined yet").is_finished() {
                    let handle = handle.take().expect("not joined yet");
                    self.state = match handle.join() {
                        Ok(Ok(archive)) => {
                            let mod_name = path.file_stem().and_then(OsStr::to_str).unwrap_or_default().to_owned();

                            let extract_selection = ExtractSelection::new(&archive);

                            State::ExtractDialog {
                                mod_name,
                                mod_already_exists: None,
                                archive,
                                extract_selection,
                                tree_display: TreeDisplay::new(),
                                dir_checkbox_cache: HashMap::default(),
                            }
                        }
                        Ok(Err(err)) => {
                            error!(?err, ?path, "failed to open archive");
                            State::Error(format!("Failed to open archive:\n{}", err).into_boxed_str())
                        }
                        Err(_) => {
                            error!("archive read thread panicked");
                            State::Error(Box::from("Failed to open archive:\nThread panicked."))
                        }
                    };
                    return self.update(ui, instance);
                }

                show_immediate!(viewport, ui, |ui, _viewport| {
                    CentralPanel::default().show_inside(ui, |ui| {
                        let count = counter.unique_files();
                        if count != *previous_count {
                            *previous_count = count;
                            text.clear();
                            write!(text, "{} file entries read", count).expect("writing to a string shouldn't fail");
                            ui.request_repaint();
                        }

                        ui.centered_and_justified(|ui| {
                            ui.label(text.as_str());
                        });
                    });
                })
            }
            State::ExtractDialog { .. } => {
                let viewport = self.viewport.as_ref().expect("viewport has been created").as_ref();
                show_immediate!(viewport, ui, |ui, _viewport| {
                    CentralPanel::default().show_inside(ui, |ui| self.extract_dialog(ui, instance));
                })
            }
            State::Closing => ViewportResult::Drop,
            State::Error(err) => {
                let viewport = self.viewport.as_ref().expect("viewport has been created").as_ref();
                show_immediate!(viewport, ui, |ui, _viewport| {
                    CentralPanel::default().show_inside(ui, |ui| {
                        ui.centered_and_justified(|ui| {
                            ui.label(err.as_ref());
                        });
                    });
                })
            }
        }
    }

    fn extract_dialog(&mut self, ui: &mut Ui, instance: &EditableInstance) {
        let State::ExtractDialog {
            mod_name,
            mod_already_exists,
            extract_selection,
            tree_display,
            dir_checkbox_cache,
            ..
        } = &mut self.state
        else {
            unreachable!()
        };

        Self::mod_name(ui, instance, mod_name, mod_already_exists);

        let label_fn = |ui: &mut Ui, tree: &mut FileTree<bool>, id: &NodeId| {
            ui.horizontal(|ui| {
                let is_dir = matches!(tree.get(*id).expect("node exists").data().kind, TreeNodeKind::Dir);

                let changed = if is_dir {
                    let (mut extract, indeterminate) = match dir_checkbox_cache.get(id) {
                        Some(CheckboxState::Partial) => (false, true),
                        Some(CheckboxState::Unchecked) => (false, false),
                        None => (true, false),
                    };
                    let previous = extract;
                    ui.add(Checkbox::without_text(&mut extract).indeterminate(indeterminate));
                    let changed = extract != previous;

                    if changed {
                        let mut iter = tree.get_mut(*id).expect("node exists").traverse_post_order();
                        while let Some(mut node) = iter.next() {
                            match &mut node.data().kind {
                                TreeNodeKind::File(e) => *e = extract,
                                TreeNodeKind::Dir => {
                                    if extract {
                                        dir_checkbox_cache.remove(&node.node_id());
                                    } else {
                                        dir_checkbox_cache.insert(node.node_id(), CheckboxState::Unchecked);
                                    }
                                }
                            }
                        }
                    }

                    changed
                } else {
                    let mut node = tree.get_mut(*id).expect("node exists");
                    if let TreeNodeKind::File(extract) = &mut node.data().kind {
                        let previous = *extract;
                        ui.checkbox(extract, ());
                        *extract != previous
                    } else {
                        unreachable!()
                    }
                };

                let node = tree.get(*id).expect("node exists");
                if changed {
                    let parent = node.parent().expect("has parent");
                    let root_id = tree.root_id().expect("has root node");
                    update_checkbox_cache(dir_checkbox_cache, &parent, root_id);
                }

                ui.label(node.data().name.as_str());
            });
        };

        let handle_actions_fn = dnd_handle_actions_fn(|tree, dnd| {
            assert!(matches!(
                tree.get(dnd.target).expect("node exists").data().kind,
                TreeNodeKind::Dir
            ));

            for node in dnd.source {
                tree.get_mut(node).unwrap().append_to(dnd.target).unwrap();
            }

            tree.get_mut(dnd.target).unwrap().sort_children_by(node_ord);
        });

        let tree_height = ui.available_height() - ui.style().spacing.interact_size.y;
        Frame::new()
            .stroke(ui.style().visuals.window_stroke)
            .corner_radius(CornerRadius::same(4))
            .show(ui, |ui| {
                tree_display.display(ui, extract_selection.tree(), label_fn, handle_actions_fn, tree_height);
            });

        Sides::new().show(
            ui,
            |_| (),
            |ui| {
                if ui.button("Cancel").clicked() {
                    ui.send_viewport_cmd(ViewportCommand::Close);
                }

                if ui.button("Install").clicked() {
                    let State::ExtractDialog { mod_name, mut archive, extract_selection, .. } =
                        mem::replace(&mut self.state, State::Closing)
                    else {
                        unreachable!()
                    };

                    let mods_dir = instance.arc_dir();
                    let task = Box::new(move |status: &StatusString| {
                        {
                            let mut s = status.lock().expect("lock is not poisoned");
                            s.clear();
                            let _ = write!(s, "Installing mod {}", mod_name);
                        }

                        let staged_mod = match StagedInstall::stage_archive(&mods_dir, &mut archive, &extract_selection)
                        {
                            Ok(m) => m,
                            Err(err) => {
                                error!(?err, "failed to extract archive");
                                return None;
                            }
                        };

                        let finalizer: Finalizer = Box::new(move |mm: &mut ModManagerUi| {
                            if let Err(err) = mm.instance.add_staged_mod(&mod_name, staged_mod) {
                                error!("failed to add staged mod: {}", err);
                                return;
                            }

                            debug!("installed mod {}", &mod_name);
                            mm.mod_added();

                            // TODO: highlight newly installed mod
                        });
                        Some(finalizer)
                    });

                    if self.background_task_queue.send(task).is_err() {
                        error!("background task panicked");
                    }
                }
            },
        );
    }

    fn mod_name(
        ui: &mut Ui,
        instance: &EditableInstance,
        mod_name: &mut String,
        mod_already_exists: &mut Option<bool>,
    ) {
        ui.horizontal(|ui| {
            ui.label("Mod name:");
            let response = ui.text_edit_singleline(mod_name);
            if response.changed() || mod_already_exists.is_none() {
                *mod_already_exists = Some(instance.mods().iter().any(|decl| decl.name() == mod_name));
            }

            if *mod_already_exists == Some(true) {
                ui.label(RichText::new("There is already a mod with this name.").text_style(TextStyle::Small));
            }
        });
    }

    pub fn clear_mod_already_exists_state(&mut self) {
        if let State::ExtractDialog { mod_already_exists, .. } = &mut self.state {
            *mod_already_exists = None;
        }
    }
}

fn update_checkbox_cache(cache: &mut HashMap<NodeId, CheckboxState>, parent: &TreeNodeRef<bool>, root_id: NodeId) {
    assert_eq!(parent.data().kind, TreeNodeKind::Dir);
    let parent_id = parent.node_id();
    if parent_id == root_id {
        return;
    }

    let mut checked = false;
    let mut unchecked = false;

    for child in parent.children() {
        match child.data().kind {
            TreeNodeKind::File(true) => checked |= true,
            TreeNodeKind::File(false) => unchecked |= true,
            TreeNodeKind::Dir => match cache.get(&child.node_id()) {
                None => checked |= true,
                Some(CheckboxState::Partial) => {
                    checked |= true;
                    unchecked |= true;
                }
                Some(CheckboxState::Unchecked) => unchecked |= true,
            },
        }
    }

    let changed = match (checked, unchecked) {
        (true, true) => cache.insert(parent_id, CheckboxState::Partial) != Some(CheckboxState::Partial),
        (true, false) => cache.remove(&parent_id).is_some(),
        (false, true) => cache.insert(parent_id, CheckboxState::Unchecked) != Some(CheckboxState::Unchecked),
        (false, false) => false,
    };

    if changed {
        let grandparent = parent.parent().expect("has parent");
        update_checkbox_cache(cache, &grandparent, root_id);
    }
}
