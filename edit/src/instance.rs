// Copyright © 2025-2026 Joaquim Monteiro
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
use std::sync::mpsc::Sender;

use compact_str::CompactString;
use thiserror::Error;
use tracing::{error, trace};
use typed_index_collections::{TiSlice, TiVec};

use mmm_core::instance::data::{INSTANCE_DATA_FILE, InstanceData, InstanceDataOpenError};
use mmm_core::instance::{
    DEFAULT_PROFILE, DEFAULT_PROFILE_NAME, Instance, ModDeclaration, ModIndex, ModOrderEntry, ModOrderIndex,
};

use crate::writer::{WriteRequest, WriteTarget, spawn_writer_thread};

/// Implementation of [`Instance`] with editing support (for interactive applications).
pub struct EditableInstance {
    dir: PathBuf,
    data: InstanceData,
    state: EditorState,
    write_queue: Sender<WriteRequest>,
    changed: bool,
}

impl EditableInstance {
    /// Opens the instance at the specified path.
    #[allow(clippy::assigning_clones, reason = "compact_str clones don't share resources")]
    pub fn open(dir: &Path) -> Result<Self, InstanceOpenError> {
        let dir = dir
            .canonicalize()
            .map_err(|source| InstanceOpenError::DirCanonicalize { source, dir: dir.to_owned() })?;
        if !dir
            .metadata()
            .map_err(|source| InstanceOpenError::DirMetadata { source, dir: dir.clone() })?
            .is_dir()
        {
            return Err(InstanceOpenError::NotADirectory(dir));
        }

        let data_file = dir.join(INSTANCE_DATA_FILE);
        let mut data = InstanceData::from_file(&data_file)?;

        let mut state = EditorState::default();
        if !data.profiles.contains_key(state.current_profile()) {
            let default = DEFAULT_PROFILE_NAME;
            if data.profiles.contains_key(&default) {
                state.current_profile = default;
            } else if let Some((name, _)) = data.profiles.first_key_value() {
                state.current_profile = name.to_owned();
            } else {
                let _ = data.profiles.insert(default.clone(), DEFAULT_PROFILE);
                state.current_profile = default;
            }
        }

        let write_queue = spawn_writer_thread(&dir).map_err(InstanceOpenError::SpawnWriterThread)?;

        let mut instance = Self { dir, data, state, write_queue, changed: false };
        instance.add_missing_mods_to_mod_order();

        Ok(instance)
    }

    /// Saves the state of the instance and queues writing it to disk.
    ///
    /// Does nothing if the state hasn't changed since the last call to this method.
    pub fn save(&mut self) {
        if !self.changed {
            return;
        }
        self.changed = false;
        trace!("saving instance data");

        let content = match cbor4ii::serde::to_vec(Vec::new(), &self.data) {
            Ok(value) => value,
            Err(err) => {
                error!("failed to serialize instance data: {}", err);
                return;
            }
        };

        let req = WriteRequest { content, target: WriteTarget::InstanceData };
        if self.write_queue.send(req).is_err() {
            error!("write thread crashed");
        }
    }
}

/// Error type returned by [`EditableInstance::open`].
#[derive(Debug, Error)]
pub enum InstanceOpenError {
    #[error("failed to canonicalize path '{dir}': {source}")]
    DirCanonicalize { source: io::Error, dir: PathBuf },
    #[error("failed to get metadata of '{dir}': {source}")]
    DirMetadata { source: io::Error, dir: PathBuf },
    #[error("'{0}' is not a directory")]
    NotADirectory(PathBuf),
    #[error("err: {0}")]
    DataOpen(#[from] InstanceDataOpenError),
    #[error("failed to spawn writer thread: {0}")]
    SpawnWriterThread(#[source] io::Error),
}

impl Instance for EditableInstance {
    fn dir(&self) -> &Path {
        &self.dir
    }

    fn mods(&self) -> &TiSlice<ModIndex, ModDeclaration> {
        &self.data.mods
    }

    fn mod_order(&self) -> &TiSlice<ModOrderIndex, ModOrderEntry> {
        &self
            .data
            .profiles
            .get(&self.state.current_profile)
            .expect("profile exists")
            .mod_order
    }
}

impl EditableInstance {
    fn mod_order_mut(&mut self) -> &mut TiVec<ModOrderIndex, ModOrderEntry> {
        &mut self
            .data
            .profiles
            .get_mut(&self.state.current_profile)
            .expect("profile exists")
            .mod_order
    }

    /// Adds missing [`entries`](ModOrderEntry) to the current profile's mod order.
    ///
    /// This should be called when switching profiles, as we only add entries to the current profile
    /// (and we don't know if the deserialized mod order is missing any entries).
    fn add_missing_mods_to_mod_order(&mut self) {
        let mods = self.mods().len();
        let Some(mods_to_add) = mods.checked_sub(self.mod_order().len()) else {
            // nothing to add
            return;
        };

        let mod_order = self.mod_order_mut();
        mod_order.reserve(mods_to_add);

        let mut mods_present = vec![false; mods];
        for order_entry in mod_order.iter() {
            mods_present[Into::<usize>::into(order_entry.mod_index())] = true;
        }

        for (idx, present) in mods_present.iter().enumerate() {
            if !present {
                mod_order.push(ModOrderEntry::new(ModIndex::from(idx)));
            }
        }
    }

    /// Switches the current profile to the specified one.
    ///
    /// Does nothing if the profile doesn't exist.
    pub fn switch_to_profile(&mut self, profile_name: CompactString) {
        if !self.data.profiles.contains_key(&profile_name) {
            error!("tried to switch to non-existent profile '{}'", profile_name);
            return;
        }
        self.state.current_profile = profile_name;
        self.add_missing_mods_to_mod_order();
    }
}

struct EditorState {
    current_profile: CompactString,
}

impl Default for EditorState {
    fn default() -> Self {
        Self { current_profile: DEFAULT_PROFILE_NAME }
    }
}

impl EditorState {
    #[must_use]
    pub const fn current_profile(&self) -> &CompactString {
        &self.current_profile
    }
}
