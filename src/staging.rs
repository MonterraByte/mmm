// Copyright © 2025 Joaquim Monteiro
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

use std::fs;
use std::io;
use std::iter;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

use thiserror::Error;

use crate::mods::Mods;
use crate::mods::{FileTree, TreeNodeKind};
use crate::mount::{TempMount, TempMountCreationError};

pub fn build_staging_tree(tree: &FileTree, mods: &Mods) -> Result<TempMount, StagingTreeBuildError> {
    let staging_dir = TempMount::new()?;

    let mut ancestors = Vec::new();
    for node in tree.root().expect("has root node").traverse_pre_order().skip(1) {
        ancestors.extend(node.ancestors());
        let relative_path: PathBuf = ancestors
            .iter()
            .rev()
            .skip(1)
            .chain(iter::once(&node))
            .map(|node| node.data().name())
            .collect();
        ancestors.clear();
        let staging_path = staging_dir.path().join(&relative_path);

        match node.data().kind() {
            TreeNodeKind::Dir => {
                fs::create_dir(&staging_path).map_err(|source| StagingTreeBuildError::Mkdir {
                    path: staging_path,
                    source,
                })?;
            }
            TreeNodeKind::File { providing_mods } => {
                let mod_index = *providing_mods
                    .first()
                    .expect("files are always provided by at least one mod");
                let source_path = mods.path(mod_index).expect("mod exists").join(&relative_path);

                symlink(&source_path, &staging_path).map_err(|source| StagingTreeBuildError::Symlink {
                    source_path,
                    link_path: staging_path,
                    source,
                })?;
            }
        }
    }

    Ok(staging_dir)
}

#[derive(Debug, Error)]
pub enum StagingTreeBuildError {
    #[error("failed to create directory '{path}': {source}")]
    Mkdir { path: PathBuf, source: io::Error },
    #[error("failed to create symlink '{link_path}' that points to '{source_path}': {source}")]
    Symlink {
        source_path: PathBuf,
        link_path: PathBuf,
        source: io::Error,
    },
    #[error(transparent)]
    TempDir(#[from] TempMountCreationError),
}
