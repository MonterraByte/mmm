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

mod rar;
mod seven_zip;
mod tar;
mod zip;

use camino::Utf8Path;
use compact_str::ToCompactString;
use foldhash::HashMap;
use nary_tree::NodeId;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

use mmm_core::file_tree::{
    Counters, FileTree, FileTreeBuilder, FileTreeBuilderWithCounter, TreeNode, TreeNodeKind, TreeNodeRef,
    find_node_by_path, new_tree,
};

use self::rar::Rar;
use self::seven_zip::SevenZip;
use self::tar::Tar;
use self::zip::Zip;
use crate::file_tree::node_ord;

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

    pub fn extract(&mut self, path: PathBuf, selection: &ExtractSelection) -> Result<(), anyhow::Error> {
        fs::create_dir_all(&path)?;
        self.handle.extract(path, &self.tree, selection)
    }

    #[must_use]
    pub fn tree(&self) -> &FileTree {
        &self.tree
    }
}

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

pub struct ExtractSelection {
    file_map: HashMap<NodeId, NodeId>,
    tree: FileTree<bool>,
}

impl ExtractSelection {
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

    #[must_use]
    pub fn tree(&mut self) -> &mut FileTree<bool> {
        &mut self.tree
    }

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

/*#[cfg(test)]
#[test]
fn eq() {
    use mmm_core::file_tree::FileTree;
    use std::path::Path;

    let assert_tree_eq = |a: &FileTree, b: &FileTree| {
        itertools::assert_equal(
            a.root().unwrap().traverse_pre_order().map(|n| n.data()),
            b.root().unwrap().traverse_pre_order().map(|n| n.data()),
        );
    };

    let sort_tree = |mut tree: FileTree| {
        println!("Before:");
        //ptree::print_tree(&SingleFileTreeDisplay::new(&tree)).unwrap();
        tree.root_mut().unwrap().sort_recursive_by(crate::file_tree::node_ord);
        println!("After:");
        //ptree::print_tree(&SingleFileTreeDisplay::new(&tree)).unwrap();
        tree
    };

    let tar_path = "/home/joaquim/Projects/mmm/Skyrim's Got Talent-50357-1-70-1706046902.tar";
    let tar_archive = Archive::open(Path::new(tar_path).into()).unwrap();
    let tar_tree = sort_tree(tar_archive.tree);
    //ptree::print_tree(&SingleFileTreeDisplay::new(&tar_tree)).unwrap();

    let zip_path = "/home/joaquim/Projects/mmm/Skyrim's Got Talent-50357-1-70-1706046902.zip";
    let zip_archive = Archive::open(Path::new(zip_path).into()).unwrap();
    let zip_tree = sort_tree(zip_archive.tree);
    //ptree::print_tree(&SingleFileTreeDisplay::new(&zip_tree)).unwrap();
    assert_tree_eq(&tar_tree, &zip_tree);

    let sz_path = "/home/joaquim/Projects/mmm/Skyrim's Got Talent-50357-1-70-1706046902.7z";
    let sz_archive = Archive::open(Path::new(sz_path).into()).unwrap();
    let sz_tree = sort_tree(sz_archive.tree);
    //ptree::print_tree(&SingleFileTreeDisplay::new(&sz_tree)).unwrap();
    assert_tree_eq(&tar_tree, &sz_tree);

    let rar_path = "/home/joaquim/Projects/mmm/Skyrim's Got Talent-50357-1-70-1706046902.rar";
    let rar_archive = Archive::open(Path::new(rar_path).into()).unwrap();
    let rar_tree = sort_tree(rar_archive.tree);
    //ptree::print_tree(&SingleFileTreeDisplay::new(&rar_tree)).unwrap();
    assert_tree_eq(&tar_tree, &rar_tree);
}*/

/*fn sort_node(tree: &mut FileTree, parent: NodeId) {
    let collator = LazyLock::force(&COLLATOR);
    let get_first = |tree: &FileTree| tree.get(parent).unwrap().first_child().map(|c| c.node_id());
    let Some(mut first) = get_first(tree) else {
        return;
    };
    println!(
        "sorting {} and its {} siblings",
        &tree.get(first).unwrap().data().name,
        &tree.get(first).unwrap().parent().map_or(1, |p| p.children().count()) - 1
    );

    let mut outer_loop_count = 0u32;
    loop {
        outer_loop_count += 1;
        let mut swapped = false;
        first = get_first(tree).unwrap();
        let mut a = first;
        println!("\t{}- a: {}", outer_loop_count, &tree.get(a).unwrap().data().name);
        let mut b = if let Some(next) = tree.get(first).unwrap().next_sibling() {
            next.node_id()
        } else {
            println!("\t{}- no sibling, breaking", outer_loop_count);
            break;
        };
        println!("\t{}- b: {}", outer_loop_count, &tree.get(b).unwrap().data().name);

        let mut inner_loop_count = 0u32;
        loop {
            inner_loop_count += 1;
            println!(
                "\t\t{}- comparing {} with {}",
                inner_loop_count,
                &tree.get(a).unwrap().data().name,
                &tree.get(b).unwrap().data().name
            );

            if node_ord(tree.get(a).unwrap().data(), tree.get(b).unwrap().data()) == Ordering::Greater {
                println!(
                    "\t\t{}- {} is greater, swapping",
                    inner_loop_count,
                    &tree.get(a).unwrap().data().name
                );
                if tree.get_mut(a).unwrap().swap_next_sibling() {
                    swapped = true;
                    println!(
                        "\t\t{}- swapped {} with {}",
                        inner_loop_count,
                        &tree.get(a).unwrap().data().name,
                        &tree.get(b).unwrap().data().name
                    );
                    b = if let Some(next) = tree.get(a).unwrap().next_sibling() {
                        next.node_id()
                    } else {
                        println!("\t\t{}- no sibling, breaking", inner_loop_count);
                        break;
                    };
                    println!("\t\t{}- b: {}", inner_loop_count, &tree.get(b).unwrap().data().name);
                } else {
                    unreachable!()
                }
            } else {
                println!(
                    "\t\t{}- {} is greater or equal, skipping",
                    inner_loop_count,
                    &tree.get(b).unwrap().data().name
                );
                a = b;
                println!("\t\t{}- a: {}", inner_loop_count, &tree.get(a).unwrap().data().name);
                b = if let Some(next) = tree.get(a).unwrap().next_sibling() {
                    next.node_id()
                } else {
                    println!("\t\t{}- no sibling, breaking", inner_loop_count);
                    break;
                };
                println!("\t\t{}- b: {}", inner_loop_count, &tree.get(b).unwrap().data().name);
            }
        }

        if !swapped {
            println!("\t{}- no swaps, breaking", outer_loop_count);
            break;
        }
        println!("\t{}- swapped, continuing", outer_loop_count);
    }

    let mut next = get_first(tree).unwrap();
    loop {
        if let Some(child) = tree.get(next).unwrap().first_child().map(|n| n.node_id()) {
            sort_node(tree, child);
        }

        if let Some(next_sibling) = tree.get(next).unwrap().next_sibling() {
            next = next_sibling.node_id();
        } else {
            break;
        }
    }
}
*/
