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

//! Utilities for displaying the contents of a [`FileTree`].

use std::io;

use nary_tree::NodeId;

use super::{FileTree, TreeNodeKind};
use crate::instance::Instance;

/// Structure to display [`FileTree`]s using [`ptree`].
#[derive(Clone)]
pub struct FileTreeDisplay<'a> {
    tree: &'a FileTree,
    instance: &'a dyn Instance,
    current_node: NodeId,
    kind: FileTreeDisplayKind,
}

/// Specifies what files are displayed by [`FileTreeDisplay`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FileTreeDisplayKind {
    /// Show all files.
    All,
    /// Only show files provided by multiple mods.
    Conflicts,
}

impl<'a> FileTreeDisplay<'a> {
    #[must_use]
    pub fn new(tree: &'a FileTree, instance: &'a dyn Instance, kind: FileTreeDisplayKind) -> Self {
        Self {
            tree,
            instance,
            current_node: tree.root_id().expect("has root node"),
            kind,
        }
    }
}

impl ptree::TreeItem for FileTreeDisplay<'_> {
    type Child = Self;

    fn write_self<W: io::Write>(&self, f: &mut W, style: &ptree::Style) -> io::Result<()> {
        let node = self.tree.get(self.current_node).expect("node exists");
        match &node.data().kind {
            TreeNodeKind::Dir => write!(f, "📁 {}", style.paint(&node.data().name)),
            TreeNodeKind::File { providing_mods } => {
                write!(
                    f,
                    "📄 {} ('{}')",
                    style.paint(&node.data().name),
                    itertools::join(
                        providing_mods.iter().map(|idx| self.instance.mods()[*idx].name()),
                        "', '"
                    )
                )
            }
        }
    }

    fn children(&self) -> std::borrow::Cow<'_, [Self::Child]> {
        let node = self.tree.get(self.current_node).expect("node exists");
        let children: Vec<_> = node
            .children()
            .filter(|node| {
                if self.kind != FileTreeDisplayKind::Conflicts {
                    return true;
                }
                match &node.data().kind {
                    TreeNodeKind::Dir => node.traverse_pre_order().any(|node| match node.data().kind {
                        TreeNodeKind::Dir => false,
                        TreeNodeKind::File { ref providing_mods } => providing_mods.len() > 1,
                    }),
                    TreeNodeKind::File { providing_mods } => providing_mods.len() > 1,
                }
            })
            .map(|node| FileTreeDisplay {
                tree: self.tree,
                instance: self.instance,
                current_node: node.node_id(),
                kind: self.kind,
            })
            .collect();
        std::borrow::Cow::Owned(children)
    }
}
