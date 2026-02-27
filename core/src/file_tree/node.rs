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

use std::mem;

use compact_str::CompactString;
use smallvec::SmallVec;

use crate::instance::ModIndex;

pub type ModVec = SmallVec<[ModIndex; 4]>;
const _: () = assert!(mem::size_of::<ModVec>() == 24);
const _: () = assert!(mem::size_of::<SmallVec<[ModIndex; 5]>>() == 32);

/// A node of a [`FileTree`].
#[derive(Debug)]
pub struct TreeNode {
    pub name: CompactString,
    pub kind: TreeNodeKind,
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
