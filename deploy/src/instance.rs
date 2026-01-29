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

use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;
use typed_index_collections::{TiSlice, TiVec};

use mmm_core::instance::data::{INSTANCE_DATA_FILE, InstanceData, InstanceDataOpenError};
use mmm_core::instance::{
    DEFAULT_PROFILE_NAME, Instance, ModDeclaration, ModIndex, ModOrderEntry, ModOrderIndex, Profile,
};

#[derive(Debug)]
pub struct DeployInstance {
    dir: PathBuf,
    mods: TiVec<ModIndex, ModDeclaration>,
    profile: Profile,
}

impl DeployInstance {
    pub fn open(dir: &Path, profile_name: Option<&str>) -> Result<Self, DeployInstanceOpenError> {
        let dir = dir
            .canonicalize()
            .map_err(|source| DeployInstanceOpenError::DirCanonicalize { source, dir: dir.to_owned() })?;
        if !dir
            .metadata()
            .map_err(|source| DeployInstanceOpenError::DirMetadata { source, dir: dir.clone() })?
            .is_dir()
        {
            return Err(DeployInstanceOpenError::NotADirectory(dir));
        }

        let data_file = dir.join(INSTANCE_DATA_FILE);
        let mut data = InstanceData::from_file(&data_file)?;

        let profile = if let Some(profile_name) = profile_name {
            data.profiles
                .remove(profile_name)
                .ok_or_else(|| DeployInstanceOpenError::ProfileNotFound(profile_name.to_owned()))?
        } else if let Some(profile) = data.profiles.remove(&DEFAULT_PROFILE_NAME) {
            profile
        } else if let Some((_, profile)) = data.profiles.pop_first() {
            profile
        } else {
            return Err(DeployInstanceOpenError::NoProfiles);
        };

        Ok(Self { dir, mods: data.mods, profile })
    }
}

impl Instance for DeployInstance {
    fn dir(&self) -> &Path {
        &self.dir
    }

    fn mods(&self) -> &TiSlice<ModIndex, ModDeclaration> {
        &self.mods
    }

    fn mod_order(&self) -> &TiSlice<ModOrderIndex, ModOrderEntry> {
        &self.profile.mod_order
    }
}

#[derive(Debug, Error)]
pub enum DeployInstanceOpenError {
    #[error("failed to canonicalize path '{dir}'")]
    DirCanonicalize { source: io::Error, dir: PathBuf },
    #[error("failed to get metadata of '{dir}'")]
    DirMetadata { source: io::Error, dir: PathBuf },
    #[error("instance has no profiles")]
    NoProfiles,
    #[error("'{0}' is not a directory")]
    NotADirectory(PathBuf),
    #[error("profile '{0}' does not exist")]
    ProfileNotFound(String),
    #[error("failed to open instance data file")]
    DataOpen(#[from] InstanceDataOpenError),
}
