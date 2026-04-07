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
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Instant;

use compact_str::CompactString;
use eframe::egui;
use egui::{CentralPanel, CornerRadius, Frame, Ui, ViewportCommand, ViewportId};
use nary_tree::NodeId;
use tracing::error;

use mmm_core::file_tree::util::NodePathBuilder;
use mmm_core::file_tree::{Counters, FileTree, FileTreeBuilder, IterDirError, TreeNodeKind, new_tree};
use mmm_core::instance::{Instance, ModEntryKind, ModIndex};
use mmm_edit::EditableInstance;
use mmm_edit::util::node_ord;

use crate::tree::{TreeDisplay, dnd_handle_actions_fn};
use crate::utils::{Viewport, ViewportResult, show_immediate};

enum Tree {
    Some(FileTree),
    Pending {
        handle: Option<ThreadHandle>,
        counter: Arc<Counters>,
        previous_count: usize,
        message: CompactString,
    },
    Error(Box<str>),
}

type ThreadHandle = JoinHandle<Result<FileTree, IterDirError>>;
const _: () = assert!(size_of::<ThreadHandle>() == size_of::<Option<ThreadHandle>>());

impl Tree {
    fn from_dir(dir: PathBuf) -> Result<Self, io::Error> {
        let counter = Counters::new();
        let tree_builder = FileTreeBuilder::new().with_counter(Arc::clone(&counter));

        let handle = thread::Builder::new().spawn(move || {
            let mut tree = new_tree();
            tree_builder.iter_dir(&mut tree, dir)?;
            tree.root_mut().expect("has root node").sort_recursive_by(node_ord);
            Ok(tree)
        })?;

        Ok(Self::Pending {
            handle: Some(handle),
            counter,
            previous_count: 0,
            message: CompactString::const_new("0 files counted"),
        })
    }

    fn update(&mut self) {
        if let Tree::Pending { handle, .. } = self
            && handle.as_ref().expect("not joined yet").is_finished()
        {
            let handle = handle.take().expect("not joined yet");
            match handle.join() {
                Ok(Ok(tree)) => *self = Tree::Some(tree),
                Ok(Err(err)) => {
                    error!(?err, "failed to build file tree");
                    *self = Tree::Error(format!("Failed to build file tree:\n{}", err).into_boxed_str());
                }
                Err(_) => {
                    error!("file tree thread panicked");
                    *self = Tree::Error(Box::from("Failed to build file tree:\nThread panicked."));
                }
            }
        }
    }
}

pub struct ModDetailsWindow {
    viewport: Box<Viewport>,
    tree: Tree,
    tree_display: TreeDisplay,
    raise: bool,
}

impl ModDetailsWindow {
    pub fn new(instance: &EditableInstance, idx: ModIndex) -> Result<Self, io::Error> {
        let dir = instance.mod_dir(&instance.mods()[idx]).expect("mod is not a separator");
        let tree = Tree::from_dir(dir)?;

        let mod_decl = &instance.mods()[idx];
        assert_eq!(mod_decl.kind(), ModEntryKind::Mod);

        let id = ViewportId::from_hash_of(("details", idx, Instant::now()));
        let viewport = Viewport::new(id, format!("mmm — Details of {}", mod_decl.name()), None);
        let tree_display = TreeDisplay::new();

        Ok(Self { viewport, tree, tree_display, raise: false })
    }

    pub fn raise(&mut self) {
        self.raise = true;
    }

    pub fn update(&mut self, ui: &mut Ui, instance: &EditableInstance, mod_index: ModIndex) -> ViewportResult {
        self.tree.update();

        show_immediate!(self.viewport, ui, |ui: &mut Ui, _viewport| {
            if self.raise {
                self.raise = false;
                ui.send_viewport_cmd(ViewportCommand::Focus);
            }
            CentralPanel::default().show_inside(ui, |ui| self.files(ui, instance, mod_index));
        })
    }

    fn files(&mut self, ui: &mut Ui, instance: &EditableInstance, mod_index: ModIndex) {
        match &mut self.tree {
            Tree::Some(tree) => {
                let dnd = dnd_handle_actions_fn(|tree, dnd| {
                    let target_node = tree.get(dnd.target).expect("node exists");
                    assert!(matches!(target_node.data().kind, TreeNodeKind::Dir));

                    let mod_dir = instance.mod_dir(&instance.mods()[mod_index]).expect("not a separator");

                    let mut target = NodePathBuilder::new(mod_dir.clone());
                    target.reset_and_push(&target_node);
                    let mut target = target.into_inner();
                    target.set_base_to_current();

                    let mut source = NodePathBuilder::new(mod_dir);

                    for node in dnd.source {
                        let mut source_node = tree.get_mut(node).expect("node exists");
                        let from = source.reset_and_push(&source_node.as_ref());

                        target.reset_to_base();
                        let to = target.push(&source_node.data().name);

                        if let Err(err) = fs::rename(from, to) {
                            error!(?err, "failed to move '{}' to '{}'", from.display(), to.display());
                            // TODO: consider refreshing the tree on "not found" errors
                            continue;
                        }

                        source_node.append_to(dnd.target).unwrap();
                    }

                    tree.get_mut(dnd.target).unwrap().sort_children_by(node_ord);
                });

                let tree_height = ui.available_height() - ui.style().spacing.interact_size.y;
                Frame::new()
                    .stroke(ui.style().visuals.window_stroke)
                    .corner_radius(CornerRadius::same(4))
                    .show(ui, |ui| {
                        self.tree_display.display(ui, tree, label_fn, dnd, tree_height);
                    });
            }
            Tree::Pending { counter, previous_count, message, .. } => {
                let count = counter.unique_files();
                if count != *previous_count {
                    *previous_count = count;

                    message.clear();
                    let _ = write!(message, "{} files counted", count);

                    ui.request_repaint();
                }

                ui.centered_and_justified(|ui| {
                    ui.label(message.as_str());
                });
            }
            Tree::Error(err) => {
                ui.centered_and_justified(|ui| {
                    ui.label(err.as_ref());
                });
            }
        }
    }
}

fn label_fn(ui: &mut Ui, tree: &mut FileTree, id: &NodeId) {
    let node = tree.get(*id).expect("node exists");
    ui.label(node.data().name.as_str());
}
