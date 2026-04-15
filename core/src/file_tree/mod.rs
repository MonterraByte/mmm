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

pub mod display;
mod node;
pub mod util;

use std::fs;
use std::io;
use std::marker::PhantomData;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use compact_str::CompactString;
use nary_tree::{NodeId, NodeMut, NodeRef, Tree, TreeBuilder};
use smallvec::{SmallVec, smallvec};
use thiserror::Error;

pub use self::node::{ModVec, TreeNode, TreeNodeKind};
use crate::instance::{Instance, ModDeclaration, ModIndex};

/// A tree of files.
pub type FileTree<F = ()> = Tree<TreeNode<F>>;

pub type TreeNodeRef<'a, F = ()> = NodeRef<'a, TreeNode<F>>;
pub type TreeNodeMut<'a, F = ()> = NodeMut<'a, TreeNode<F>>;

/// Creates a new empty [`FileTree`].
#[must_use]
pub fn new_tree<F>() -> FileTree<F> {
    TreeBuilder::new()
        .with_root(TreeNode {
            name: CompactString::const_new("."),
            kind: TreeNodeKind::Dir,
        })
        .build()
}

/// Struct for building out a [`FileTree`] in a configurable way.
pub struct FileTreeBuilder<F = (), Value: ProvideValue<F> = Unit, Counter: Count = NoCounter> {
    value: Value,
    counter: Counter,
    _file_type: PhantomData<F>,
}

pub type FileTreeBuilderWithCounter<F = (), Value = Unit> = FileTreeBuilder<F, Value, Arc<Counters>>;

impl FileTreeBuilder {
    /// Creates a new `FileTreeBuilder`.
    #[must_use]
    pub const fn new() -> FileTreeBuilder<()> {
        FileTreeBuilder {
            value: Unit,
            counter: NoCounter,
            _file_type: PhantomData,
        }
    }
}

impl Default for FileTreeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl<F, Value: ProvideValue<F>, Counter: Count> FileTreeBuilder<F, Value, Counter> {
    /// Returns a new `FileTreeBuilder` with the specified counter.
    pub fn with_counter<C: Count>(self, counter: C) -> FileTreeBuilder<F, Value, C> {
        FileTreeBuilder {
            value: self.value,
            counter,
            _file_type: PhantomData,
        }
    }

    /// Returns a new `FileTreeBuilder` that will create file nodes with the specified value in their `Vec`s.
    #[inline]
    pub fn with_item_value<T, Arr>(self, value: T) -> FileTreeBuilder<SmallVec<Arr>, VariableVec<T>, Counter>
    where
        T: Clone,
        Arr: smallvec::Array<Item = T>,
    {
        FileTreeBuilder {
            value: VariableVec(value),
            counter: self.counter,
            _file_type: PhantomData,
        }
    }

    /// Iterates over the specified directory, creating node that correspond to each entry in the provided tree.
    pub fn iter_dir(&self, tree: &mut FileTree<F>, dir: PathBuf) -> Result<(), IterDirError> {
        self.iter_dir_inner(tree, dir).map_err(|err| err.without_context(tree))
    }

    fn iter_dir_inner(&self, tree: &mut FileTree<F>, dir: PathBuf) -> Result<(), UnresolvedIterDirError> {
        let mut dirs_to_visit = vec![(dir, tree.root_id().expect("has root node"))];
        let mut root = true;

        while let Some((dir, node)) = dirs_to_visit.pop() {
            for entry in fs::read_dir(&dir)? {
                let entry = entry?;
                let entry_name = entry.file_name().into_string().unwrap();
                let entry_type = entry.file_type()?;
                drop(entry);

                if root && entry_name == ".git" {
                    continue;
                }

                let entry_node = if let Some(child_node) = find_child_with_name(tree, node, &entry_name) {
                    self.value
                        .add_to_existing_node(tree.get_mut(child_node).expect("node exists"), entry_type.is_dir())
                        .map_err(UnresolvedIterDirError::TypeMismatch)?;
                    self.counter.file_appended();
                    child_node
                } else {
                    let parent = tree.get_mut(node).expect("node exists");
                    if entry_type.is_dir() {
                        self.counter.dir_added();
                        create_dir_node(parent, &entry_name)
                    } else {
                        self.counter.file_added();
                        self.counter.file_appended();
                        self.value.create_file_node(parent, &entry_name)
                    }
                };

                if entry_type.is_dir() {
                    dirs_to_visit.push((dir.join(entry_name), entry_node));
                }
            }

            if root {
                root = false;
            }
        }

        Ok(())
    }

    /// Creates nodes in the tree representing the combination of files from all the enabled mods
    /// in the specified instance.
    ///
    /// Each node in the tree that represents a file contains the list of mods that provide that file,
    /// sorted from higher priority to lower.
    pub fn iter_mods(self, tree: &mut FileTree<ModVec>, instance: &impl Instance) -> Result<(), IterDirError> {
        let mut iter = self.with_item_value(ModIndex::ZERO);
        for entry in instance.mod_order().iter().rev() {
            if !entry.enabled {
                continue;
            }

            let mod_index = entry.mod_index();
            let mod_decl = &instance.mods()[mod_index];
            let Some(mod_dir) = instance.mod_dir(mod_decl) else {
                // skip separators
                continue;
            };

            iter = iter.with_item_value(mod_index);
            iter.iter_dir_inner(tree, mod_dir)
                .map_err(|err| err.with_modvec_context(tree, mod_decl, instance))?;
        }

        Ok(())
    }

    /// Creates a file node given the specified path from the root, creating any missing parent directory nodes.
    pub fn create_file_node_with_parents(
        &self,
        tree: &mut FileTree<F>,
        path: &Utf8Path,
    ) -> Result<NodeId, CreateFileNodeError> {
        let mut components = path.components();
        let Some(Utf8Component::Normal(file_name)) = components.next_back() else {
            return Err(CreateFileNodeError::NonNormalFilename);
        };

        let mut parent = tree.root_id().expect("has root node");
        for component in components {
            match component {
                Utf8Component::Normal(name) => {
                    parent = if let Some(next_node_id) = find_child_with_name(tree, parent, name) {
                        let next_node = tree.get(next_node_id).expect("node exists");
                        if !matches!(next_node.data().kind, TreeNodeKind::Dir) {
                            return Err(CreateFileNodeError::FileExists(node_path(&next_node).into_boxed_path()));
                        }
                        next_node_id
                    } else {
                        self.counter.dir_added();
                        create_dir_node(tree.get_mut(parent).expect("node exists"), name)
                    };
                }
                Utf8Component::CurDir => {}
                other => {
                    return Err(CreateFileNodeError::InvalidPathComponent(
                        other.to_string().into_boxed_str(),
                    ));
                }
            }
        }

        if let Some(id) = find_child_with_name(tree, parent, file_name) {
            let node = tree.get_mut(id).expect("node exists");
            self.value
                .add_to_existing_node(node, false)
                .map_err(|_| CreateFileNodeError::DirectoryExists)?;
            self.counter.file_appended();
            Ok(id)
        } else {
            self.counter.file_added();
            self.counter.file_appended();
            Ok(self
                .value
                .create_file_node(tree.get_mut(parent).expect("node exists"), file_name))
        }
    }
}

/// Error type returned by [`create_file_node_with_parents`](FileTreeBuilder::create_file_node_with_parents).
#[derive(Debug, Error)]
pub enum CreateFileNodeError {
    #[error("specified path does not end in a normal component")]
    NonNormalFilename,
    #[error("specified path contains invalid component {0}")]
    InvalidPathComponent(Box<str>),
    #[error("cannot create parent node '{0}' for the specified path, as there's a file node there already")]
    FileExists(Box<Utf8Path>),
    #[error("cannot create a file node at the specified path, as there's a directory node there already")]
    DirectoryExists,
}

/// A value provider for use with [`FileTreeBuilder`].
pub trait ProvideValue<F> {
    fn create_file_node(&self, parent: TreeNodeMut<F>, name: &str) -> NodeId;
    fn add_to_existing_node(&self, node: TreeNodeMut<F>, expect_dir: bool) -> Result<(), NodeId>;
}

/// A [value provider](ProvideValue) for the [`()`] type.
pub struct Unit;

impl ProvideValue<()> for Unit {
    fn create_file_node(&self, mut parent: TreeNodeMut<()>, name: &str) -> NodeId {
        parent
            .append(TreeNode { name: name.into(), kind: TreeNodeKind::File(()) })
            .node_id()
    }

    fn add_to_existing_node(&self, mut node: TreeNodeMut<()>, expect_dir: bool) -> Result<(), NodeId> {
        let kind = &node.data().kind;
        match (kind, expect_dir) {
            (TreeNodeKind::Dir, true) | (TreeNodeKind::File(()), false) => Ok(()),
            (TreeNodeKind::Dir, false) | (TreeNodeKind::File(()), true) => Err(node.node_id()),
        }
    }
}

/// A [value provider](ProvideValue) for the [`ModVec`] type.
pub struct VariableVec<T: Clone>(T);

impl<T, Arr> ProvideValue<SmallVec<Arr>> for VariableVec<T>
where
    T: Clone,
    Arr: smallvec::Array<Item = T>,
{
    fn create_file_node(&self, mut parent: TreeNodeMut<SmallVec<Arr>>, name: &str) -> NodeId {
        parent
            .append(TreeNode {
                name: name.into(),
                kind: TreeNodeKind::File(smallvec![self.0.clone()]),
            })
            .node_id()
    }

    fn add_to_existing_node(&self, mut node: TreeNodeMut<SmallVec<Arr>>, expect_dir: bool) -> Result<(), NodeId> {
        let kind = &mut node.data().kind;
        match (kind, expect_dir) {
            (TreeNodeKind::Dir, true) => Ok(()),
            (TreeNodeKind::File(info), false) => {
                info.push(self.0.clone());
                Ok(())
            }
            (TreeNodeKind::Dir, false) | (TreeNodeKind::File(_), true) => Err(node.node_id()),
        }
    }
}

#[allow(clippy::must_use_candidate)]
fn create_dir_node<F>(mut parent: TreeNodeMut<F>, name: &str) -> NodeId {
    parent
        .append(TreeNode { name: name.into(), kind: TreeNodeKind::Dir })
        .node_id()
}

/// A counter for use with [`FileTreeBuilder`].
pub trait Count {
    fn file_added(&self);
    fn file_appended(&self);
    fn dir_added(&self);
}

/// A [counter](Count) that does not count.
pub struct NoCounter;

impl Count for NoCounter {
    fn file_added(&self) {}
    fn file_appended(&self) {}
    fn dir_added(&self) {}
}

/// A [counter](Count) that does count.
#[derive(Default)]
pub struct Counters {
    files: AtomicUsize,
    unique_files: AtomicUsize,
    directories: AtomicUsize,
}

impl Counters {
    /// Creates a new counter.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Returns the number of times file nodes were created or updated.
    #[must_use]
    pub fn files(&self) -> usize {
        self.files.load(Ordering::Relaxed)
    }

    /// Returns the number of file nodes created.
    #[must_use]
    pub fn unique_files(&self) -> usize {
        self.unique_files.load(Ordering::Relaxed)
    }

    /// Returns the number of directory nodes created.
    #[must_use]
    pub fn directories(&self) -> usize {
        self.directories.load(Ordering::Relaxed)
    }
}

impl Count for Counters {
    fn file_added(&self) {
        self.files.fetch_add(1, Ordering::Relaxed);
    }

    fn file_appended(&self) {
        self.unique_files.fetch_add(1, Ordering::Relaxed);
    }

    fn dir_added(&self) {
        self.directories.fetch_add(1, Ordering::Relaxed);
    }
}

impl<C: Count> Count for Arc<C> {
    #[inline]
    fn file_added(&self) {
        self.deref().file_added();
    }

    #[inline]
    fn file_appended(&self) {
        self.deref().file_appended();
    }

    #[inline]
    fn dir_added(&self) {
        self.deref().dir_added();
    }
}

#[derive(Debug)]
enum UnresolvedIterDirError {
    Io(io::Error),
    TypeMismatch(NodeId),
}

impl From<io::Error> for UnresolvedIterDirError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl UnresolvedIterDirError {
    fn with_modvec_context(
        self,
        tree: &FileTree<ModVec>,
        mod_decl: &ModDeclaration,
        instance: &impl Instance,
    ) -> IterDirError {
        match self {
            Self::Io(err) => IterDirError::Io(err),
            Self::TypeMismatch(node_id) => {
                let conflict_node = tree.get(node_id).expect("node exists");
                let expected_dir = !matches!(&conflict_node.data().kind, TreeNodeKind::Dir);
                let node_path = node_path(&conflict_node);

                let mut conflicting_mod_names = Vec::new();
                for other_mod in instance.mods() {
                    if other_mod == mod_decl {
                        continue;
                    }

                    let path_to_check = instance
                        .mod_dir(other_mod)
                        .expect("separators don't have files")
                        .join(&node_path);
                    match fs::symlink_metadata(&path_to_check) {
                        Ok(m) => {
                            if m.is_dir() != expected_dir {
                                conflicting_mod_names.push(other_mod.name());
                            }
                        }
                        Err(err) => return IterDirError::Io(err), // TODO: log initial error
                    }
                }

                let mod_name = mod_decl.name();
                let joined_conflicting_mod_names = itertools::join(conflicting_mod_names, "', '");
                IterDirError::TypeMismatch(match &conflict_node.data().kind {
                    TreeNodeKind::Dir => format!(
                        "'{node_path}' is used as both a directory and a file by different mods: it's a file in '{mod_name}', but a directory in '{joined_conflicting_mod_names}'",
                    ),
                    TreeNodeKind::File(_) => format!(
                        "'{node_path}' is used as both a directory and a file by different mods: it's a directory in '{mod_name}', but a file in '{joined_conflicting_mod_names}'",
                    ),
                }.into_boxed_str())
            }
        }
    }

    fn without_context<F>(self, tree: &FileTree<F>) -> IterDirError {
        match self {
            Self::Io(err) => IterDirError::Io(err),
            Self::TypeMismatch(node_id) => {
                let conflict_node = tree.get(node_id).expect("node exists");
                let path = node_path(&conflict_node);
                IterDirError::TypeMismatch(format!("'{path}' is used as both a directory and a file").into_boxed_str())
            }
        }
    }
}

/// Error type returned by [`iter_dir`](FileTreeBuilder::iter_dir) and [`iter_mods`](FileTreeBuilder::iter_mods).
#[derive(Debug, Error)]
pub enum IterDirError {
    #[error("failed to read directory")]
    Io(#[from] io::Error),
    #[error("{0}")]
    TypeMismatch(Box<str>),
}

#[must_use]
fn find_child_with_name<F>(tree: &FileTree<F>, parent: NodeId, name: &str) -> Option<NodeId> {
    tree.get(parent)
        .expect("node exists")
        .children()
        .find(|child| child.data().name == name)
        .as_ref()
        .map(NodeRef::node_id)
}

/// Finds a node in the tree by its path relative to the root.
#[must_use]
pub fn find_node_by_path<'tree, F>(tree: &'tree FileTree<F>, path: &Utf8Path) -> Option<TreeNodeRef<'tree, F>> {
    let mut node = tree.root().expect("has root node");

    for component in path.components() {
        match component {
            Utf8Component::Normal(name) => node = node.children().find(|child| child.data().name == name)?,
            Utf8Component::ParentDir => {
                let parent = node.parent()?.node_id();
                node = tree.get(parent).expect("node exists");
            }
            Utf8Component::CurDir => {
                if matches!(node.data().kind, TreeNodeKind::File(_)) {
                    return None;
                }
            }
            Utf8Component::Prefix(_) | Utf8Component::RootDir => return None,
        }
    }

    Some(node)
}

/// Returns the path from the root to the specified node.
#[must_use]
pub fn node_path<F>(node: &TreeNodeRef<F>) -> Utf8PathBuf {
    let ancestors: Vec<_> = node.ancestors().collect();
    ancestors
        .iter()
        .rev()
        .skip(1) // don't add "./" to the start of every path
        .chain(std::iter::once(node))
        .map(|node| node.data().name.as_str())
        .collect()
}
