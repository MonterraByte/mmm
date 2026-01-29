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

//! Functions for walking through mod files and representing them as a tree.

use std::fs;
use std::io;
use std::mem;
use std::path::PathBuf;

use compact_str::CompactString;
use nary_tree::{NodeId, NodeMut, Tree, TreeBuilder};
use smallvec::{SmallVec, smallvec};
use thiserror::Error;

use crate::instance::{Instance, ModDeclaration, ModEntryKind, ModIndex};

type ModVec = SmallVec<[ModIndex; 4]>;
const _: () = assert!(mem::size_of::<ModVec>() == 24);
const _: () = assert!(mem::size_of::<SmallVec<[ModIndex; 5]>>() == 32);

/// A node of a [`FileTree`].
#[derive(Debug)]
pub struct TreeNode {
    name: CompactString,
    kind: TreeNodeKind,
}

impl TreeNode {
    #[must_use]
    pub const fn name(&self) -> &CompactString {
        &self.name
    }

    #[must_use]
    pub const fn kind(&self) -> &TreeNodeKind {
        &self.kind
    }
}

/// The type of node in a [`FileTree`].
#[derive(Debug)]
pub enum TreeNodeKind {
    /// Node representing a directory.
    Dir,
    /// Node representing a file.
    File {
        /// The [`ModIndex`]s of the mods that provide this file. The mods that appear first have higher priority.
        providing_mods: ModVec,
    },
}

/// A tree representing the combination of files from multiple mods.
///
/// Each node in the tree that represents a file contains the list of mods that provide that file,
/// sorted from higher priority to lower.
pub type FileTree = Tree<TreeNode>;
type FileNodeMut<'a> = NodeMut<'a, TreeNode>;

/// Builds a [`FileTree`] from all the enabled mods in the specified instance.
pub fn build_path_tree(instance: &impl Instance) -> Result<FileTree, TreeBuildError> {
    let mut tree = TreeBuilder::new()
        .with_root(TreeNode {
            name: CompactString::const_new("."),
            kind: TreeNodeKind::Dir,
        })
        .build();
    let root = tree.root_id().expect("has root node");

    for entry in instance.mod_order().iter().rev() {
        if !entry.enabled {
            continue;
        }

        let mod_index = entry.mod_index();
        let mod_decl = &instance.mods()[mod_index];
        if mod_decl.kind() != ModEntryKind::Mod {
            continue;
        }

        let mod_dir = instance.mod_dir(mod_decl);
        iter_dir(&mut tree, mod_index, mod_dir, root).map_err(|err| err.with_context(&tree, mod_decl, instance))?;
    }

    Ok(tree)
}

fn iter_dir(
    tree: &mut FileTree,
    mod_index: ModIndex,
    dir: PathBuf,
    node: NodeId,
) -> Result<(), UnresolvedTreeBuildError> {
    let mut dirs_to_visit = vec![(dir, node)];

    while let Some((dir, node)) = dirs_to_visit.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let entry_name = entry.file_name().into_string().unwrap();
            let entry_type = entry.file_type()?;
            drop(entry);

            let entry_node = if let Some(child_node) = find_child_with_name(tree, node, &entry_name) {
                add_to_existing_node(
                    tree.get_mut(child_node).expect("node exists"),
                    mod_index,
                    entry_type.is_dir(),
                )?;
                child_node
            } else {
                let parent = tree.get_mut(node).expect("node exists");
                if entry_type.is_dir() {
                    create_dir_node(parent, &entry_name)
                } else {
                    create_file_node(parent, mod_index, &entry_name)
                }
            };

            if entry_type.is_dir() {
                dirs_to_visit.push((dir.join(entry_name), entry_node));
            }
        }
    }

    Ok(())
}

fn find_child_with_name(tree: &FileTree, parent: NodeId, name: &str) -> Option<NodeId> {
    let parent = tree.get(parent).expect("node exists");
    for child in parent.children() {
        if child.data().name == name {
            return Some(child.node_id());
        }
    }
    None
}

fn create_dir_node(mut parent: FileNodeMut, name: &str) -> NodeId {
    parent
        .append(TreeNode { name: name.into(), kind: TreeNodeKind::Dir })
        .node_id()
}

fn create_file_node(mut parent: FileNodeMut, mod_index: ModIndex, name: &str) -> NodeId {
    parent
        .append(TreeNode {
            name: name.into(),
            kind: TreeNodeKind::File { providing_mods: smallvec![mod_index] },
        })
        .node_id()
}

fn add_to_existing_node(
    mut node: FileNodeMut,
    mod_index: ModIndex,
    expect_dir: bool,
) -> Result<(), UnresolvedTreeBuildError> {
    let kind = &mut node.data().kind;
    match (kind, expect_dir) {
        (TreeNodeKind::Dir, true) => Ok(()),
        (TreeNodeKind::File { providing_mods }, false) => {
            providing_mods.push(mod_index);
            Ok(())
        }
        (TreeNodeKind::Dir, false) | (TreeNodeKind::File { .. }, true) => {
            Err(UnresolvedTreeBuildError::TypeMismatch(node.node_id()))
        }
    }
}

#[derive(Debug)]
enum UnresolvedTreeBuildError {
    Io(io::Error),
    TypeMismatch(NodeId),
}

impl From<io::Error> for UnresolvedTreeBuildError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl UnresolvedTreeBuildError {
    fn with_context(self, tree: &FileTree, mod_decl: &ModDeclaration, instance: &impl Instance) -> TreeBuildError {
        match self {
            Self::Io(err) => TreeBuildError::Io(err),
            Self::TypeMismatch(node_id) => {
                let conflict_node = tree.get(node_id).expect("node exists");
                let name = &conflict_node.data().name;
                let expected_dir = matches!(&conflict_node.data().kind, TreeNodeKind::File { .. });

                let ancestors: Vec<_> = conflict_node.ancestors().collect();
                let node_path: PathBuf = ancestors.iter().rev().map(|node| &node.data().name).collect();

                let mut conflicting_mod_names = Vec::new();
                for other_mod in instance.mods() {
                    if other_mod == mod_decl {
                        continue;
                    }

                    let path_to_check = instance.mod_dir(other_mod).join(&node_path);
                    match fs::symlink_metadata(&path_to_check) {
                        Ok(m) => {
                            if m.is_dir() != expected_dir {
                                conflicting_mod_names.push(other_mod.name());
                            }
                        }
                        Err(err) => return TreeBuildError::Io(err), // TODO: log initial error
                    }
                }

                let mod_name = mod_decl.name();
                let joined_conflicting_mod_names = itertools::join(conflicting_mod_names, "', '");
                match &conflict_node.data().kind {
                    TreeNodeKind::Dir => TreeBuildError::TypeMismatch(format!(
                        "'{name}' is used as both a directory and a file by different mods: it's a file in '{mod_name}', but a directory in '{joined_conflicting_mod_names}'"
                    )),
                    TreeNodeKind::File { .. } => TreeBuildError::TypeMismatch(format!(
                        "'{name}' is used as both a directory and a file by different mods: it's a directory in '{mod_name}', but a file in '{joined_conflicting_mod_names}'"
                    )),
                }
            }
        }
    }
}

/// Error type returned by [`build_path_tree`].
#[derive(Debug, Error)]
pub enum TreeBuildError {
    #[error("failed to read directory")]
    Io(#[from] io::Error),
    #[error("{0}")]
    TypeMismatch(String),
}

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
                match node.data().kind() {
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
