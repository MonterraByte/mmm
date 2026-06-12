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

//! Installation staging with temporary directories.

use std::fs;
use std::io;
use std::path::Path;

use tempfile::TempDir;
use thiserror::Error;

use crate::archive::{Archive, ExtractSelection};

/// Utility for atomic archive extraction through a temporary directory.
///
/// The directory will be deleted when this struct is dropped, unless [`place`](Self::place) is called successfully.
pub struct StagedInstall(TempDir);

impl StagedInstall {
    /// Extracts the specified archive to a temporary directory in the mods directory.
    pub fn stage_archive(
        mods_dir: &Path,
        archive: &mut Archive,
        selection: &ExtractSelection,
    ) -> Result<Self, StageError> {
        let temp_dir = TempDir::with_prefix_in(".staging-", mods_dir).map_err(StageError::CreateStagingDir)?;

        archive
            .extract(temp_dir.path().to_owned(), selection)
            .map_err(StageError::Extract)?;

        Ok(Self(temp_dir))
    }

    /// Moves the directory containing the extracted files to the specified location.
    pub(crate) fn place(mut self, new_path: &Path) -> Result<(), PlaceError> {
        fs::rename(self.0.path(), new_path)?;
        self.0.disable_cleanup(true);
        Ok(())
    }
}

/// Error type returned by [`StagedInstall::stage_archive`].
#[derive(Debug, Error)]
pub enum StageError {
    #[error("failed to create staging directory")]
    CreateStagingDir(#[source] io::Error),
    #[error("failed to extract archive")]
    Extract(#[source] anyhow::Error),
}

/// Error type returned by [`StagedInstall::place`].
#[derive(Debug, Error)]
#[error("failed to move staged mod to its final location")]
pub struct PlaceError(#[from] io::Error);
