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

//! [`FileTree`] integration for [`egui_ltreeview`]

use std::time::Instant;

use eframe::egui;
use egui::{Id, ScrollArea, Ui};
use egui_ltreeview::{Action, DragAndDrop, TreeView, TreeViewState};
use nary_tree::NodeId;

use mmm_core::file_tree::{FileTree, TreeNodeKind, TreeNodeRef};

/// [`egui_ltreeview`] wrapper for displaying [`FileTree`]s.
pub struct TreeDisplay {
    state: TreeViewState<NodeId>,
    queue: Vec<NodeId>,
    parent_stack: Vec<NodeId>,
    id: Id,
}

impl TreeDisplay {
    pub fn new() -> Self {
        Self {
            state: TreeViewState::default(),
            queue: Vec::new(),
            parent_stack: Vec::new(),
            id: Id::new(("tree", Instant::now())),
        }
    }

    pub fn display<T>(
        &mut self,
        ui: &mut Ui,
        tree: &mut FileTree<T>,
        mut label_fn: impl FnMut(&mut Ui, &mut FileTree<T>, &NodeId),
        mut handle_actions_fn: impl FnMut(&mut Ui, &mut FileTree<T>, Vec<Action<NodeId>>),
        max_height: f32,
    ) {
        ScrollArea::both().max_height(max_height).show(ui, |ui| {
            let (_, actions) =
                TreeView::new(self.id)
                    .override_striped(Some(true))
                    .show_state(ui, &mut self.state, |builder| {
                        let root_id = tree.root_id().expect("has root node");
                        builder.node(RootConfig(&root_id));

                        self.parent_stack.clear();
                        self.parent_stack.push(root_id);

                        self.queue.clear();
                        if let Some(first) = tree.root().expect("has root node").first_child() {
                            self.queue.push(first.node_id());
                        }

                        while let Some(node_id) = self.queue.pop() {
                            let parent_id = tree
                                .get(node_id)
                                .expect("node exists")
                                .parent()
                                .expect("has parent")
                                .node_id();
                            while parent_id
                                != *self
                                    .parent_stack
                                    .last()
                                    .expect("parent stack always has at least one element")
                            {
                                self.parent_stack.pop();
                                builder.close_dir();
                            }
                            if let TreeNodeKind::Dir = tree.get(node_id).expect("node exists").data().kind {
                                self.parent_stack.push(node_id);
                            }

                            let open = builder.node(NodeConfig::new(tree, &node_id, &mut label_fn));

                            let node = tree.get(node_id).expect("node exists");
                            if let Some(next_sibling) = node.next_sibling() {
                                self.queue.push(next_sibling.node_id());
                            }

                            if open && let Some(first_child) = node.first_child() {
                                self.queue.push(first_child.node_id());
                            }
                        }

                        builder.close_dir();
                    });

            handle_actions_fn(ui, tree, actions);
        });
    }
}

pub fn dnd_handle_actions_fn<T>(
    mut on_move: impl FnMut(&mut FileTree<T>, DragAndDrop<NodeId>),
) -> impl FnMut(&mut Ui, &mut FileTree<T>, Vec<Action<NodeId>>) {
    move |ui: &mut Ui, tree: &mut FileTree<T>, actions| {
        for action in actions {
            match action {
                Action::Move(dnd) => on_move(tree, dnd),
                Action::Drag(dnd) => {
                    if dnd.source.iter().all(|source| {
                        tree.get(*source)
                            .expect("node exists")
                            .parent()
                            .expect("has parent")
                            .node_id()
                            == dnd.target
                    }) {
                        dnd.remove_drop_marker(ui);
                    }
                }
                Action::Activate(_) | Action::SetSelected(_) | Action::DragExternal(_) | Action::MoveExternal(_) => {}
            }
        }
    }
}

struct NodeConfig<'a, T, LabelFn>
where
    LabelFn: FnMut(&mut Ui, &mut FileTree<T>, &NodeId),
{
    tree: &'a mut FileTree<T>,
    id: &'a NodeId,
    label_fn: LabelFn,
}

impl<'a, T, LabelFn> NodeConfig<'a, T, LabelFn>
where
    LabelFn: FnMut(&mut Ui, &mut FileTree<T>, &NodeId),
{
    const fn new(tree: &'a mut FileTree<T>, id: &'a NodeId, label_fn: LabelFn) -> Self {
        Self { tree, id, label_fn }
    }

    fn node(&self) -> TreeNodeRef<'_, T> {
        self.tree.get(*self.id).expect("node exists")
    }
}

impl<T, LabelFn> egui_ltreeview::NodeConfig<NodeId> for NodeConfig<'_, T, LabelFn>
where
    LabelFn: FnMut(&mut Ui, &mut FileTree<T>, &NodeId),
{
    fn id(&self) -> &NodeId {
        self.id
    }

    fn is_dir(&self) -> bool {
        matches!(self.node().data().kind, TreeNodeKind::Dir)
    }

    fn label(&mut self, ui: &mut Ui) {
        (self.label_fn)(ui, self.tree, self.id);
    }

    fn default_open(&self) -> bool {
        let get_id = |node: TreeNodeRef<T>| node.node_id();
        let one_or_no_children =
            |node: &TreeNodeRef<T>| node.first_child().map(get_id) == node.last_child().map(get_id);
        self.node().parent().is_some_and(|parent| one_or_no_children(&parent))
    }
}

struct RootConfig<'a>(&'a NodeId);

impl egui_ltreeview::NodeConfig<NodeId> for RootConfig<'_> {
    fn id(&self) -> &NodeId {
        self.0
    }

    fn is_dir(&self) -> bool {
        true
    }

    fn label(&mut self, _: &mut Ui) {}

    fn flatten(&self) -> bool {
        true
    }
}
