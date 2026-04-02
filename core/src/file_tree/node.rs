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

//! Node of a [`FileTree`](super::FileTree).

use std::cmp::Ordering;
use std::fmt::{self, Debug};
use std::mem;

use compact_str::CompactString;
use smallvec::SmallVec;

use crate::instance::ModIndex;

pub type ModVec = SmallVec<[ModIndex; 4]>;
const _: () = assert!(mem::size_of::<ModVec>() == 24);
const _: () = assert!(mem::size_of::<SmallVec<[ModIndex; 5]>>() == 32);

/// A node of a [`FileTree`].
pub struct TreeNode<F = ()> {
    pub name: CompactString,
    pub kind: TreeNodeKind<F>,
}

impl<F> TreeNode<F> {
    #[must_use]
    pub const fn name(&self) -> &CompactString {
        &self.name
    }

    #[must_use]
    pub const fn kind(&self) -> &TreeNodeKind<F> {
        &self.kind
    }
}

impl<F: Debug> Debug for TreeNode<F> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("TreeNode")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .finish()
    }
}

impl<F: PartialEq> PartialEq for TreeNode<F> {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.kind == other.kind
    }
}

impl<F: PartialEq + Eq> Eq for TreeNode<F> {}

impl<F: PartialEq + Eq> Ord for TreeNode<F> {
    fn cmp(&self, other: &Self) -> Ordering {
        match (&self.kind, &other.kind) {
            (TreeNodeKind::Dir, TreeNodeKind::File(_)) => return Ordering::Less,
            (TreeNodeKind::File(_), TreeNodeKind::Dir) => return Ordering::Greater,
            _ => {}
        }

        self.name.cmp(&other.name)
    }
}

impl<F: PartialEq + Eq> PartialOrd for TreeNode<F> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The type of node in a [`FileTree`].
pub enum TreeNodeKind<F> {
    /// Node representing a directory.
    Dir,
    /// Node representing a file.
    File(F),
}

impl<F: Debug> Debug for TreeNodeKind<F> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            TreeNodeKind::Dir => f.write_str("Dir"),
            TreeNodeKind::File(info) => f.debug_tuple("File").field(info).finish(),
        }
    }
}

impl<F: PartialEq> PartialEq for TreeNodeKind<F> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (TreeNodeKind::Dir, TreeNodeKind::Dir) => true,
            (TreeNodeKind::File(a), TreeNodeKind::File(b)) => a == b,
            _ => false,
        }
    }
}
