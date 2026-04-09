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

use std::fs::{self, File};
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use tar::{Archive, EntryType};

use mmm_core::file_tree::util::NodePathBuilder;
use mmm_core::file_tree::{FileTree, FileTreeBuilderWithCounter, new_tree, node_path};

use crate::archive::{ArchiveFormat, ArchiveFormatDef, ExtractSelection};

pub struct Tar(BufReader<File>);

impl ArchiveFormatDef for Tar {
    fn file_is_archive(file: &mut File, _: [u8; 8]) -> Result<bool, io::Error> {
        const POSIX_TAR: [u8; 8] = [b'u', b's', b't', b'a', b'r', 0, b'0', b'0'];
        const GNU_TAR: [u8; 8] = [b'u', b's', b't', b'a', b'r', 0x20, 0x20, 0];

        file.seek(SeekFrom::Start(257))?;
        let mut buf = [0; 8];
        match file.read_exact(&mut buf) {
            Ok(()) => Ok(buf == POSIX_TAR || buf == GNU_TAR),
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
            Err(err) => Err(err),
        }
    }

    fn ext_is_archive(ext: &str) -> bool {
        ext == "tar"
    }

    fn open(file: File, _: Arc<Path>) -> Result<Box<dyn ArchiveFormat>, anyhow::Error> {
        let reader = BufReader::new(file);
        Ok(Box::new(Self(reader)))
    }
}

impl ArchiveFormat for Tar {
    fn file_tree(&mut self, tree_builder: &FileTreeBuilderWithCounter) -> Result<FileTree, anyhow::Error> {
        let mut tree = new_tree();

        self.0.seek(SeekFrom::Start(0)).context("failed to seek to start")?;
        let mut archive = Archive::new(&mut self.0);

        for entry in archive.entries_with_seek().context("failed to open tar archive")? {
            let entry = entry.context("failed to read entry")?;
            let path = entry.path().context("failed to access entry path")?;
            if matches!(
                entry.header().entry_type(),
                EntryType::Regular | EntryType::Continuous | EntryType::Link | EntryType::Symlink
            ) {
                tree_builder
                    .create_file_node_with_parents(
                        &mut tree,
                        path.as_ref().try_into().context("entry path is not valid UTF-8")?,
                    )
                    .context("failed to create file node")?;
            }
        }

        Ok(tree)
    }

    fn extract(
        &mut self,
        dir: PathBuf,
        file_tree: &FileTree,
        selection: &ExtractSelection,
    ) -> Result<(), anyhow::Error> {
        let mut path_builder = NodePathBuilder::new(dir);

        self.0.seek(SeekFrom::Start(0)).context("failed to seek to start")?;
        let mut archive = Archive::new(&mut self.0);

        for entry in archive.entries_with_seek().context("failed to open tar archive")? {
            let mut entry = entry.context("failed to read entry")?;
            if !matches!(
                entry.header().entry_type(),
                EntryType::Regular | EntryType::Continuous | EntryType::Link | EntryType::Symlink
            ) {
                continue;
            }

            if let Some(target) = selection.get_target_node(
                file_tree,
                entry
                    .path()
                    .context("failed to access entry path")?
                    .as_ref()
                    .try_into()
                    .context("entry path is not valid UTF-8")?,
            ) {
                let parent_dir = path_builder.reset_and_push_ancestors(&target);
                fs::create_dir_all(parent_dir)
                    .with_context(|| format!("failed to create directory '{}'", parent_dir.display()))?;

                let target_path = path_builder.push_node(&target);
                entry.unpack(target_path).with_context(|| {
                    format!(
                        "failed to extract '{}' to '{}'",
                        node_path(&target),
                        target_path.display()
                    )
                })?;
            }
        }

        Ok(())
    }
}
