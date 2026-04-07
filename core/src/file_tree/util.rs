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

//! Miscellaneous functions for working with file trees.

use std::path::{Path, PathBuf};

use recycle_vec::VecExt;

use super::TreeNodeRef;
use crate::util::ResettablePathBuf;

/// A [`ResettablePathBuf`] wrapper for [`TreeNode`](super::TreeNode) paths.
pub struct NodePathBuilder<'unused> {
    path: ResettablePathBuf,
    ancestors: Vec<TreeNodeRef<'unused>>,
}

impl NodePathBuilder<'_> {
    /// Creates a new `NodePathBuilder` with the provided path as the base.
    #[must_use]
    pub fn new(base: PathBuf) -> Self {
        Self {
            path: ResettablePathBuf::new(base),
            ancestors: Vec::new(),
        }
    }

    /// Truncates the path to its base, then extends it with the full path to the specified node.
    pub fn reset_and_push<T>(&mut self, node: &TreeNodeRef<T>) -> &Path {
        self.reset_and_push_ancestors(node);
        self.push_node(node)
    }

    /// Truncates the path to its base, then extends it with the full path to the parent of the specified node.
    pub fn reset_and_push_ancestors<T>(&mut self, node: &TreeNodeRef<T>) -> &Path {
        self.path.reset_to_base();

        self.ancestors.clear();
        // When using this method with `NodeMut` references, the lifetime of `NodePathBuilder` being inferred as
        // the same as the `NodeMut`'s causes "cannot borrow as mutable because it is also borrowed as immutable"
        // (E0502) errors.
        // This error isn't correct, as the `Vec` is just kept to reuse its allocation,
        // we don't keep any references there after this method returns. Alas, Rust cannot know that.
        // To fix this, we use a dummy lifetime for the `Vec`, then [recycle it](https://docs.rs/recycle_vec)
        // into a `Vec` with the lifetime of `node`, then recycle it back to the dummy lifetime once we're done with it.
        // `NodeRef`s always have the same size and alignment, independent of lifetime or data type parameters,
        // so recycling `Vec`s of them always works.
        replace_with::replace_with_or_abort(&mut self.ancestors, |buf| {
            let mut ancestors = VecExt::recycle(buf);

            ancestors.extend(node.ancestors());
            for node in ancestors.iter().rev().skip(1) {
                self.path.push(&node.data().name);
            }

            VecExt::recycle(ancestors)
        });

        self.path.as_ref()
    }

    /// Extends the path with the specified node itself.
    pub fn push_node<T>(&mut self, node: &TreeNodeRef<T>) -> &Path {
        self.path.push(&node.data().name);
        self.path.as_ref()
    }

    /// Consumes the `NodePathBuilder`, returning the inner `ResettablePathBuf`.
    #[must_use]
    pub fn into_inner(self) -> ResettablePathBuf {
        self.path
    }
}
