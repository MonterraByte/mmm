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
use std::io::{self, BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use camino::Utf8PathBuf;
use tracing::warn;
use zip::ZipArchive;

use mmm_core::file_tree::util::NodePathBuilder;
use mmm_core::file_tree::{FileTree, FileTreeBuilderWithCounter, new_tree};

use super::{ArchiveFormat, ArchiveFormatDef, ExtractSelection};

pub struct Zip(ZipArchive<BufReader<File>>);

impl ArchiveFormatDef for Zip {
    fn file_is_archive(_: &mut File, first_eight_bytes: [u8; 8]) -> Result<bool, io::Error> {
        Ok(matches!(
            first_eight_bytes,
            [b'P', b'K', 3, 4, _, _, _, _] | [b'P', b'K', 5, 6, _, _, _, _] | [b'P', b'K', 7, 8, _, _, _, _]
        ))
    }

    fn ext_is_archive(ext: &str) -> bool {
        ext == "zip"
    }

    fn open(file: File, _: Arc<Path>) -> Result<Box<dyn ArchiveFormat>, anyhow::Error> {
        let reader = BufReader::new(file);
        let archive = ZipArchive::new(reader).context("failed to open ZIP archive")?;
        Ok(Box::new(Self(archive)))
    }
}

impl ArchiveFormat for Zip {
    fn file_tree(&mut self, tree_builder: &FileTreeBuilderWithCounter) -> Result<FileTree, anyhow::Error> {
        let mut tree = new_tree();

        for idx in 0..self.0.len() {
            let file = self
                .0
                .by_index_raw(idx)
                .with_context(|| format!("failed to read entry {idx}"))?;
            if file.is_dir() {
                continue;
            }

            if let Some(name) = file.enclosed_name() {
                let name = Utf8PathBuf::try_from(name).context("entry path is not valid UTF-8")?;
                tree_builder
                    .create_file_node_with_parents(&mut tree, &name)
                    .context("failed to create file node")?;
            } else {
                warn!("ignoring invalid path '{}'", file.name());
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

        for idx in 0..self.0.len() {
            let mut entry = self
                .0
                .by_index(idx)
                .with_context(|| format!("failed to read entry {idx}"))?;
            if entry.is_dir() {
                continue;
            }

            let path_in_archive = if let Some(path) = entry.enclosed_name() {
                Utf8PathBuf::try_from(path).context("entry path is not valid UTF-8")?
            } else {
                warn!("ignoring invalid path '{}'", entry.name());
                continue;
            };

            if let Some(target) = selection.get_target_node(file_tree, &path_in_archive) {
                let parent_dir = path_builder.reset_and_push_ancestors(&target);
                fs::create_dir_all(parent_dir)
                    .with_context(|| format!("failed to create directory '{}'", parent_dir.display()))?;

                let target_path = path_builder.push_node(&target);
                let mut file = BufWriter::new(
                    File::create(target_path)
                        .with_context(|| format!("failed to create '{}'", target_path.display()))?,
                );
                io::copy(&mut entry, &mut file)
                    .with_context(|| format!("failed to write '{path_in_archive}' into '{}'", target_path.display()))?;
                let _ = file
                    .into_inner()
                    .map_err(io::IntoInnerError::into_error)
                    .with_context(|| format!("failed to finish writing into '{}'", target_path.display()))?;
            }
        }

        Ok(())
    }
}
