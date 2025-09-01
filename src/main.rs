// Copyright © 2025 Joaquim Monteiro
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

use std::fs;
use std::io;
use std::mem;
use std::path::{Path, PathBuf};

use clap::Parser;
use compact_str::CompactString;
use nary_tree::{NodeId, NodeMut, Tree, TreeBuilder};
use smallvec::{SmallVec, smallvec};
use thiserror::Error;

#[derive(Parser)]
struct Args {
    path: PathBuf,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct ModIndex(u32);

impl From<usize> for ModIndex {
    fn from(value: usize) -> Self {
        Self(u32::try_from(value).expect("mod count does not exceed 32 bits"))
    }
}

type ModVec = SmallVec<[ModIndex; 4]>;
const _: () = assert!(mem::size_of::<ModVec>() == 24);
const _: () = assert!(mem::size_of::<SmallVec<[ModIndex; 5]>>() == 32);

#[derive(Debug)]
struct TreeNode {
    name: CompactString,
    kind: TreeNodeKind,
}

#[derive(Debug)]
enum TreeNodeKind {
    Dir,
    File { providing_mods: ModVec },
}

type FileTree = Tree<TreeNode>;
type FileNodeMut<'a> = NodeMut<'a, TreeNode>;

fn build_path_tree(mods_dir: &Path, mods: &[String]) -> Result<FileTree, TreeBuildError> {
    let mut tree = TreeBuilder::new()
        .with_root(TreeNode {
            name: CompactString::const_new("."),
            kind: TreeNodeKind::Dir,
        })
        .build();
    let root = tree.root_id().expect("has root node");

    for (mod_index, mod_name) in mods.iter().enumerate() {
        let mod_index = ModIndex::from(mod_index);
        let mod_dir = mods_dir.join(mod_name);
        iter_dir(&mut tree, mod_index, &mod_dir, root)
            .map_err(|err| err.with_context(&tree, mod_name, mods_dir, mods))?;
    }

    Ok(tree)
}

fn iter_dir(
    tree: &mut FileTree,
    mod_index: ModIndex,
    dir: &Path,
    node: NodeId,
) -> Result<(), UnresolvedTreeBuildError> {
    for entry in fs::read_dir(dir)? {
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
            iter_dir(tree, mod_index, &dir.join(entry_name), entry_node)?; // TODO: use stack
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
        .append(TreeNode {
            name: name.into(),
            kind: TreeNodeKind::Dir,
        })
        .node_id()
}

fn create_file_node(mut parent: FileNodeMut, mod_index: ModIndex, name: &str) -> NodeId {
    parent
        .append(TreeNode {
            name: name.into(),
            kind: TreeNodeKind::File {
                providing_mods: smallvec![mod_index],
            },
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
    fn with_context(self, tree: &FileTree, mod_name: &str, mods_dir: &Path, mods: &[String]) -> TreeBuildError {
        match self {
            Self::Io(err) => TreeBuildError::Io(err),
            Self::TypeMismatch(node_id) => {
                let conflict_node = tree.get(node_id).expect("node exists");
                let name = &conflict_node.data().name;

                // TODO: use the dir logic for both branches?
                match &conflict_node.data().kind {
                    TreeNodeKind::Dir => {
                        let ancestors: Vec<_> = conflict_node.ancestors().collect();
                        let node_path: PathBuf = ancestors.iter().rev().map(|node| &node.data().name).collect();

                        let mut conflicting_mod_names = Vec::new();
                        for mod_name in mods {
                            let path_to_check = {
                                let mut p = mods_dir.to_owned();
                                p.push(mod_name);
                                p.join(&node_path)
                            };
                            match fs::symlink_metadata(&path_to_check) {
                                Ok(m) => {
                                    if m.is_dir() {
                                        conflicting_mod_names.push(mod_name);
                                    }
                                }
                                Err(err) => return TreeBuildError::Io(err), // TODO: log initial error
                            }
                        }

                        let joined_conflicting_mod_names = itertools::join(conflicting_mod_names, "', '");
                        TreeBuildError::TypeMismatch(format!(
                            "'{name}' is used as both a directory and a file by different mods: it's a file in '{mod_name}', but a directory in '{joined_conflicting_mod_names}'"
                        ))
                    }
                    TreeNodeKind::File { providing_mods } => {
                        let conflicting_mod_names = providing_mods.iter().map(|idx| &mods[idx.0 as usize]);
                        let joined_conflicting_mod_names = itertools::join(conflicting_mod_names, "', '");
                        TreeBuildError::TypeMismatch(format!(
                            "'{name}' is used as both a directory and a file by different mods: it's a directory in '{mod_name}', but a file in '{joined_conflicting_mod_names}'"
                        ))
                    }
                }
            }
        }
    }
}

#[derive(Debug, Error)]
enum TreeBuildError {
    #[error("failed to read directory: {0}")]
    Io(#[from] io::Error),
    #[error("{0}")]
    TypeMismatch(String),
}

fn main() {
    let args = Args::parse();

    let base_dir = args.path;
    let mods_dir = base_dir.join("mods");
    let mods: Vec<String> = {
        let mut file = fs::File::open(base_dir.join("mods.json")).expect("open mods json");
        serde_json::from_reader(&mut file).expect("deserialize mods json")
    };

    let tree = build_path_tree(&mods_dir, &mods).unwrap();
    ptree::print_tree(&FileTreeDisplay {
        tree: &tree,
        mods: &mods,
        current_node: tree.root_id().expect("has root node"),
    })
    .unwrap();
}

#[derive(Clone)]
struct FileTreeDisplay<'a> {
    tree: &'a FileTree,
    mods: &'a [String],
    current_node: NodeId,
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
                    "📄 {} ({})",
                    style.paint(&node.data().name),
                    itertools::join(providing_mods.iter().map(|idx| &self.mods[idx.0 as usize]), "', '")
                )
            }
        }
    }

    fn children(&self) -> std::borrow::Cow<'_, [Self::Child]> {
        let node = self.tree.get(self.current_node).expect("node exists");
        let children: Vec<_> = node
            .children()
            .map(|node| FileTreeDisplay {
                tree: self.tree,
                mods: self.mods,
                current_node: node.node_id(),
            })
            .collect();
        std::borrow::Cow::Owned(children)
    }
}
