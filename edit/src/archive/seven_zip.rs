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

use std::fs::{self, File, FileTimes};
use std::io::{self, BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use camino::Utf8Path;
use mmm_core::file_tree::{FileTree, FileTreeBuilderWithCounter, new_tree};
use sevenz_rust2::{ArchiveReader, Password};
use thiserror::Error;

use crate::archive::{ArchiveFormat, ArchiveFormatDef, ExtractSelection};
use crate::file_tree::{CreateNodeError, NodePathBuilder};

pub struct SevenZip(ArchiveReader<BufReader<File>>);

impl ArchiveFormatDef for SevenZip {
    fn file_is_archive(_: &mut File, first_eight_bytes: [u8; 8]) -> Result<bool, io::Error> {
        Ok(matches!(first_eight_bytes, [b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C, _, _]))
    }

    fn ext_is_archive(ext: &str) -> bool {
        ext == "7z"
    }

    fn open(file: File, _: Arc<Path>) -> Result<Box<dyn ArchiveFormat>, anyhow::Error> {
        let reader = BufReader::new(file);
        let archive = ArchiveReader::new(reader, Password::empty())?;
        Ok(Box::new(Self(archive)))
    }
}

impl ArchiveFormat for SevenZip {
    fn file_tree(&mut self, tree_builder: &FileTreeBuilderWithCounter) -> Result<FileTree, anyhow::Error> {
        let mut tree = new_tree();

        self.0.for_each_entries(|entry, _| {
            if entry.is_directory {
                return Ok(true); // continue iterating
            }

            let path = Utf8Path::new(&entry.name);
            tree_builder
                .create_file_node_with_parents(&mut tree, path)
                .map_err(|err| sevenz_rust2::Error::Other(err.to_string().into()))?;

            Ok(true) // continue iterating
        })?;

        Ok(tree)
    }

    fn extract(
        &mut self,
        dir: PathBuf,
        file_tree: &FileTree,
        selection: &ExtractSelection,
    ) -> Result<(), anyhow::Error> {
        let mut path_builder = NodePathBuilder::new(dir);

        self.0.for_each_entries(|entry, reader| {
            if !entry.is_directory
                && let Some(target) = selection.get_target_node(file_tree, Utf8Path::new(&entry.name))
            {
                let parent_dir = path_builder.reset_and_push_ancestors(&target);
                fs::create_dir_all(parent_dir)?;

                let target_path = path_builder.push_node(&target);
                let mut file = BufWriter::new(File::create(target_path)?);
                io::copy(reader, &mut file)?;
                let file = file.into_inner().map_err(io::IntoInnerError::into_error)?;

                let file_times = FileTimes::new().set_modified(entry.last_modified_date().into());
                let _ = file.set_times(file_times);
            }

            Ok(true) // continue iterating
        })?;

        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to open file")]
    Io(#[from] io::Error),
    #[error("failed")]
    SevenZip(#[from] sevenz_rust2::Error),
    #[error("TODO")]
    Tree(#[from] CreateNodeError),
}
