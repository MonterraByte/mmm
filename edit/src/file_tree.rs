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

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use camino::{Utf8Component, Utf8Path};
use icu_collator::options::{CaseLevel, CollatorOptions};
use icu_collator::preferences::CollationNumericOrdering;
use icu_collator::{Collator, CollatorBorrowed, CollatorPreferences};
use recycle_vec::VecExt;
use thiserror::Error;

use crate::util::ResettablePathBuf;
use mmm_core::file_tree::TreeNodeRef;
use mmm_core::file_tree::{FileTree, TreeNode, TreeNodeKind, create_dir_node, find_child_with_name};

/*pub(crate) fn create_file_node_with_parents<T>(
    tree: &mut FileTree<T>,
    path: &Utf8Path,
    info_builder: impl info::BuilderWithValue<T>,
) -> Result<(), CreateNodeError> {
    assert!(!path.components().any(|c| c == Utf8Component::ParentDir));

    let mut components = path.components();
    let Some(Utf8Component::Normal(file_name)) = components.next_back() else {
        return Err(CreateNodeError::NonNormalFilename);
    };

    let mut parent = tree.root_id().expect("has root node");
    for component in components {
        if let Utf8Component::Normal(name) = component {
            parent = if let Some(next_node) = find_child_with_name(tree, parent, name) {
                if !matches!(tree.get(next_node).expect("node exists").data().kind, TreeNodeKind::Dir) {
                    return Err(CreateNodeError::DirectoryExists);
                }
                next_node
            } else {
                create_dir_node(tree.get_mut(parent).expect("node exists"), name)
            };
        } else {
            // TODO: warn about broken archive
        }
    }

    if let Some(id) = find_child_with_name(tree, parent, file_name) {
        let node = tree.get(id).expect("node exists");
        if matches!(node.data().kind, TreeNodeKind::Dir) {
            return Err(CreateNodeError::DirectoryExists);
        }
    } else {
        create_file_node(tree.get_mut(parent).expect("node exists"), file_name, info_builder);
    }
    Ok(())
}

pub(crate) fn create_dir_node_with_parents<T>(tree: &mut FileTree<T>, path: &Utf8Path) -> Result<(), CreateNodeError> {
    assert!(!path.components().any(|c| c == Utf8Component::ParentDir)); // TODO:return err instead

    let mut components = path.components();
    let Some(Utf8Component::Normal(file_name)) = components.next_back() else {
        return Err(CreateNodeError::NonNormalFilename);
    };

    let mut parent = tree.root_id().expect("has root node");
    for component in components {
        if let Utf8Component::Normal(name) = component {
            parent = if let Some(next_node) = find_child_with_name(tree, parent, name) {
                if !matches!(tree.get(next_node).expect("node exists").data().kind, TreeNodeKind::Dir) {
                    return Err(CreateNodeError::FileExists);
                }
                next_node
            } else {
                create_dir_node(tree.get_mut(parent).expect("node exists"), name)
            };
        }
    }

    if let Some(id) = find_child_with_name(tree, parent, file_name) {
        let node = tree.get(id).expect("node exists");
        if matches!(node.data().kind, TreeNodeKind::File(_)) {
            return Err(CreateNodeError::FileExists);
        }
    } else {
        create_dir_node(tree.get_mut(parent).expect("node exists"), file_name);
    }
    Ok(())
}*/

pub struct NodePathBuilder<'unused> {
    ancestors: Vec<TreeNodeRef<'unused>>,
    path: ResettablePathBuf,
}

impl NodePathBuilder<'_> {
    #[must_use]
    pub fn new(base: PathBuf) -> Self {
        Self {
            ancestors: Vec::new(),
            path: ResettablePathBuf::new(base),
        }
    }

    pub fn reset_and_push<T>(&mut self, node: &TreeNodeRef<T>) -> &Path {
        self.reset_and_push_ancestors(node);
        self.push_node(node)
    }

    pub fn reset_and_push_ancestors<T>(&mut self, node: &TreeNodeRef<T>) -> &Path {
        self.path.reset_to_base();

        self.ancestors.clear();
        replace_with::replace_with_or_abort(&mut self.ancestors, |buf| {
            let mut ancestors = buf.recycle();

            ancestors.extend(node.ancestors());
            for node in ancestors.iter().rev().skip(1) {
                self.path.push(&node.data().name);
            }

            ancestors.recycle()
        });

        self.path.as_ref()
    }

    pub fn push_node<T>(&mut self, node: &TreeNodeRef<T>) -> &Path {
        self.path.push(&node.data().name);
        self.path.as_ref()
    }

    #[must_use]
    pub fn into_inner(self) -> ResettablePathBuf {
        self.path
    }
}

#[derive(Debug, Error)]
pub enum CreateNodeError {
    #[error("Aaaa")]
    NonNormalFilename,
    #[error("Bbbb")]
    FileExists,
    #[error("Bbbb")]
    DirectoryExists,
}

thread_local! {
    static COLLATOR: CollatorBorrowed<'static> = {
        let mut prefs = CollatorPreferences::default();
        prefs.numeric_ordering = Some(CollationNumericOrdering::True);
        let mut options = CollatorOptions::default();
        options.case_level = Some(CaseLevel::Off);
        Collator::try_new(prefs, options).unwrap()
    };
}

pub fn node_ord<F: PartialEq + Eq>(left: &TreeNode<F>, right: &TreeNode<F>) -> Ordering {
    match (&left.kind, &right.kind) {
        (TreeNodeKind::Dir, TreeNodeKind::File(_)) => return Ordering::Less,
        (TreeNodeKind::File(_), TreeNodeKind::Dir) => return Ordering::Greater,
        _ => {}
    }

    COLLATOR.with(|collator| match collator.compare(&left.name, &right.name) {
        Ordering::Equal => left.name.cmp(&right.name),
        other => other,
    })
}
