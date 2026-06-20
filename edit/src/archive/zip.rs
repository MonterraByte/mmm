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

use std::borrow::Cow;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, bail};
use camino::{Utf8Path, Utf8PathBuf};
use tracing::warn;
use zip::ZipArchive;
use zip::result::ZipError;

use mmm_core::file_tree::util::NodePathBuilder;
use mmm_core::file_tree::{FileTree, FileTreeBuilderWithCounter, new_tree};

use super::{ArchiveFormat, ArchiveFormatDef, ExtractSelection};

pub struct Zip {
    archive: ZipArchive<BufReader<File>>,

    // Zip files are only allowed to use `/` as the path separator. However, some broken implementations use `\` instead.
    // `ZipFile::enclosed_path` fixes this automatically, so this isn't normally an issue.
    // It becomes an issue when using `ZipArchive::by_name`, as we don't store the original paths,
    // and it can't find files if the path we give it has its separators changed.
    // Therefore, we need to detect this while listing the files in the archive, so we can correct the paths
    // before calling `ZipArchive::by_name`.
    uses_broken_backslash_separators: Option<bool>,
}

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
        Ok(Box::new(Self { archive, uses_broken_backslash_separators: None }))
    }
}

impl ArchiveFormat for Zip {
    fn file_tree(&mut self, tree_builder: &FileTreeBuilderWithCounter) -> Result<FileTree, anyhow::Error> {
        let mut tree = new_tree();

        for idx in 0..self.archive.len() {
            let file = self
                .archive
                .by_index_raw(idx)
                .with_context(|| format!("failed to read entry {idx}"))?;
            if file.is_dir() {
                continue;
            }

            if self.uses_broken_backslash_separators.is_none() && file.name().contains('\\') {
                self.uses_broken_backslash_separators = Some(true);
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

        if self.uses_broken_backslash_separators.is_none() {
            self.uses_broken_backslash_separators = Some(false);
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

        for idx in 0..self.archive.len() {
            let mut entry = self
                .archive
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

    fn read_file(&mut self, path_in_archive: &Utf8Path) -> anyhow::Result<Option<Vec<u8>>> {
        let path = self.fix_path(path_in_archive);

        let mut entry = match self.archive.by_name(&path) {
            Ok(entry) => entry,
            Err(ZipError::FileNotFound) => return Ok(None),
            Err(err) => return Err(err.into()),
        };

        if !entry.is_file() {
            bail!("entry is not a file");
        }

        let size =
            usize::try_from(entry.size()).with_context(|| format!("file is too large ({} bytes)", entry.size()))?;

        let mut contents = Vec::with_capacity(size);
        entry
            .read_to_end(&mut contents)
            .context("failed to read entry contents")?;

        Ok(Some(contents))
    }
}

impl Zip {
    fn fix_path<'p>(&self, path: &'p Utf8Path) -> Cow<'p, str> {
        if self.uses_broken_backslash_separators == Some(true) {
            Cow::Owned(path.as_str().replace('/', "\\"))
        } else {
            Cow::Borrowed(path.as_str())
        }
    }
}
