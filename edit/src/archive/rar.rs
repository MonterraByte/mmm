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
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use camino::{Utf8Path, Utf8PathBuf};
use unrar::Archive;

use mmm_core::file_tree::util::NodePathBuilder;
use mmm_core::file_tree::{FileTree, FileTreeBuilderWithCounter, new_tree, node_path};

use super::{ArchiveFormat, ArchiveFormatDef, ExtractSelection};

pub struct Rar(Arc<Path>);

impl ArchiveFormatDef for Rar {
    fn file_is_archive(_: &mut File, first_eight_bytes: [u8; 8]) -> Result<bool, std::io::Error> {
        Ok(matches!(
            first_eight_bytes,
            [b'R', b'a', b'r', b'!', 0x1A, 7, 1, 0] | [b'R', b'a', b'r', b'!', 0x1A, 7, 0, _]
        ))
    }

    fn ext_is_archive(ext: &str) -> bool {
        ext == "rar"
    }

    fn open(_: File, path: Arc<Path>) -> Result<Box<dyn ArchiveFormat>, anyhow::Error> {
        Ok(Box::new(Self(path)))
    }
}

impl ArchiveFormat for Rar {
    fn file_tree(&mut self, tree_builder: &FileTreeBuilderWithCounter) -> Result<FileTree, anyhow::Error> {
        let mut tree = new_tree();

        let archive = Archive::new(&self.0);
        for entry in archive.open_for_listing().context("failed to open RAR archive")? {
            let entry = entry.context("failed to read entry")?;
            if entry.is_directory() {
                continue;
            }
            let name = Utf8PathBuf::try_from(entry.filename).context("entry path is not valid UTF-8")?;
            tree_builder
                .create_file_node_with_parents(&mut tree, &name)
                .context("failed to create file node")?;
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

        let mut archive = Archive::new(&self.0)
            .open_for_processing()
            .context("failed to open RAR archive")?;
        while let Some(header) = archive.read_header().context("failed to read entry header")? {
            archive = if header.entry().is_file() {
                let path_in_archive: &Utf8Path = header
                    .entry()
                    .filename
                    .as_path()
                    .try_into()
                    .context("entry path is not valid UTF-8")?;
                if let Some(target) = selection.get_target_node(file_tree, path_in_archive) {
                    let parent_dir = path_builder.reset_and_push_ancestors(&target);
                    fs::create_dir_all(parent_dir)
                        .with_context(|| format!("failed to create directory '{}'", parent_dir.display()))?;

                    let target_path = path_builder.push_node(&target);
                    header.extract_to(target_path).with_context(|| {
                        format!(
                            "failed to extract '{}' to '{}'",
                            node_path(&target),
                            target_path.display()
                        )
                    })?
                } else {
                    header.skip().context("failed to skip entry header")?
                }
            } else {
                header.skip().context("failed to skip entry header")?
            }
        }

        Ok(())
    }
}
