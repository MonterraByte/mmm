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

//! Archive reading and extraction interface

mod rar;
mod seven_zip;
mod tar;
mod zip;

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use camino::Utf8Path;
use compact_str::ToCompactString;
use foldhash::HashMap;
use nary_tree::NodeId;
use thiserror::Error;

use mmm_core::file_tree::{
    Counters, FileTree, FileTreeBuilder, FileTreeBuilderWithCounter, TreeNode, TreeNodeKind, TreeNodeRef,
    find_node_by_path, new_tree,
};

use self::rar::Rar;
use self::seven_zip::SevenZip;
use self::tar::Tar;
use self::zip::Zip;
use crate::util::node_ord;

/// An open archive file.
pub struct Archive {
    handle: Box<dyn ArchiveFormat>,
    tree: FileTree,
}

trait ArchiveFormat: Send {
    fn file_tree(&mut self, tree_builder: &FileTreeBuilderWithCounter) -> anyhow::Result<FileTree>;
    fn extract(&mut self, dir: PathBuf, file_tree: &FileTree, selection: &ExtractSelection) -> anyhow::Result<()>;
}

trait ArchiveFormatDef {
    fn file_is_archive(file: &mut File, first_eight_bytes: [u8; 8]) -> Result<bool, io::Error>;
    fn ext_is_archive(ext: &str) -> bool;
    fn open(file: File, path: Arc<Path>) -> anyhow::Result<Box<dyn ArchiveFormat>>;
}

macro_rules! ar {
    ($name:ident) => {
        ArchiveFormatDefFns::from::<$name>()
    };
}

static FORMATS: &[ArchiveFormatDefFns] = &[ar!(Rar), ar!(SevenZip), ar!(Tar), ar!(Zip)];

impl Archive {
    /// Opens the archive at the specified path.
    pub fn open(path: Arc<Path>, counters: Arc<Counters>) -> Result<Self, OpenError> {
        let mut handle = Self::open_inner(path)?;
        let mut tree = handle
            .file_tree(&FileTreeBuilder::new().with_counter(counters))
            .map_err(OpenError::List)?;
        tree.root_mut().expect("has root node").sort_recursive_by(node_ord);
        Ok(Self { handle, tree })
    }

    fn open_inner(path: Arc<Path>) -> Result<Box<dyn ArchiveFormat>, OpenError> {
        let mut file = File::open(&path)?;

        let mut buf = [0u8; 8];
        file.read_exact(&mut buf)?;
        file.seek(SeekFrom::Start(0))?;

        for format in FORMATS {
            let matched = (format.file_is_archive)(&mut file, buf)?;
            file.seek(SeekFrom::Start(0))?;
            if matched {
                return (format.open)(file, path).map_err(OpenError::Archive);
            }
        }

        let ext = {
            let mut ext = path
                .extension()
                .ok_or(OpenError::NoExtension)?
                .to_str()
                .ok_or(OpenError::NonUnicodeExtension)?
                .to_compact_string();
            ext.make_ascii_lowercase();
            ext
        };
        for format in FORMATS {
            if (format.ext_is_archive)(ext.as_str()) {
                return (format.open)(file, path).map_err(OpenError::Archive);
            }
        }

        Err(OpenError::Unsupported)
    }

    /// Extract the contents of the archive to the specified directory, according to `selection`.
    pub fn extract(&mut self, path: PathBuf, selection: &ExtractSelection) -> Result<(), anyhow::Error> {
        fs::create_dir_all(&path)?;
        self.handle.extract(path, &self.tree, selection)
    }

    /// Returns the file tree that represents the contents of the archive.
    #[must_use]
    pub fn tree(&self) -> &FileTree {
        &self.tree
    }
}

/// Error type returned by [`Archive::open`].
#[derive(Debug, Error)]
pub enum OpenError {
    #[error("failed to open archive")]
    Archive(#[source] anyhow::Error),
    #[error("failed to list files in archive")]
    List(#[source] anyhow::Error),
    #[error("failed to access file")]
    Io(#[from] io::Error),
    #[error("file name has no extension")]
    NoExtension,
    #[error("file extension is not valid Unicode")]
    NonUnicodeExtension,
    #[error("archive format is not supported")]
    Unsupported,
}

struct ArchiveFormatDefFns {
    file_is_archive: fn(file: &mut File, first_eight_bytes: [u8; 8]) -> Result<bool, io::Error>,
    ext_is_archive: fn(ext: &str) -> bool,
    open: fn(file: File, path: Arc<Path>) -> anyhow::Result<Box<dyn ArchiveFormat>>,
}

impl ArchiveFormatDefFns {
    const fn from<T: ArchiveFormatDef>() -> Self {
        Self {
            file_is_archive: T::file_is_archive,
            ext_is_archive: T::ext_is_archive,
            open: T::open,
        }
    }
}

/// Controls what files are extracted where when using [`Archive::extract`].
pub struct ExtractSelection {
    file_map: HashMap<NodeId, NodeId>,
    tree: FileTree<bool>,
}

impl ExtractSelection {
    /// Creates a new `ExtractSelection` for the given archive.
    #[must_use]
    pub fn new(archive: &Archive) -> Self {
        let mut tree = new_tree();
        let mut file_map = HashMap::default();

        let mut parent_stack = vec![(
            archive.tree().root_id().expect("has root node"),
            tree.root_id().expect("has root node"),
        )];
        for archive_node in archive
            .tree()
            .root()
            .expect("has root node")
            .traverse_pre_order()
            .skip(1)
        {
            let archive_parent_id = archive_node.parent().expect("has parent").node_id();
            while archive_parent_id
                != *parent_stack
                    .last()
                    .map(|(id, _)| id)
                    .expect("parent stack always has at least one element")
            {
                parent_stack.pop();
            }

            let parent_id = parent_stack
                .last()
                .map(|(_, id)| id)
                .expect("parent stack always has at least one element");
            let mut parent = tree.get_mut(*parent_id).expect("node exists");

            let name = archive_node.data().name.clone();
            match archive_node.data().kind {
                TreeNodeKind::Dir => {
                    let node = parent.append(TreeNode { name, kind: TreeNodeKind::Dir });
                    parent_stack.push((archive_node.node_id(), node.node_id()));
                }
                TreeNodeKind::File(()) => {
                    let node = parent.append(TreeNode { name, kind: TreeNodeKind::File(true) });
                    file_map.insert(archive_node.node_id(), node.node_id());
                }
            }
        }

        Self { file_map, tree }
    }

    /// Returns a mutable reference to the file tree that represents the end result of calling [`Archive::extract`].
    ///
    /// Nodes can be moved around and renamed to alter what gets extracted where.
    /// The boolean value associated with each file node determines if that file is extracted or not.
    #[must_use]
    pub fn tree(&mut self) -> &mut FileTree<bool> {
        &mut self.tree
    }

    /// Looks up the node in the selection tree that corresponds to the node with the specified path
    /// in the original archive tree.
    #[must_use]
    pub fn get_target_node(
        &self,
        archive_tree: &FileTree,
        path_in_archive: &Utf8Path,
    ) -> Option<TreeNodeRef<'_, bool>> {
        find_node_by_path(archive_tree, path_in_archive)
            .and_then(|archive_node| self.file_map.get(&archive_node.node_id()))
            .and_then(|target_node| self.tree.get(*target_node))
            .take_if(|target_node| matches!(target_node.data().kind, TreeNodeKind::File(true)))
    }
}
