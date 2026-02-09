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

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use compact_str::{CompactString, format_compact};
use foldhash::HashSet;
use thiserror::Error;
use tracing::{error, trace};
use typed_index_collections::{TiSlice, TiVec};
use unicode_segmentation::UnicodeSegmentation;

use mmm_core::instance::data::{INSTANCE_DATA_FILE, InstanceData, InstanceDataOpenError};
use mmm_core::instance::{
    DEFAULT_PROFILE, DEFAULT_PROFILE_NAME, Instance, InvalidModNameError, ModDeclaration, ModEntryKind, ModIndex,
    ModOrderEntry, ModOrderIndex, Profile,
};

use crate::util::move_multiple;
use crate::writer::{WriteRequest, WriteTarget, spawn_writer_thread};
use crate::{Mod, ModInitError};

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
    #[error("failed to canonicalize path '{dir}'")]
    DirCanonicalize { source: io::Error, dir: PathBuf },
    #[error("failed to get metadata of '{dir}'")]
    DirMetadata { source: io::Error, dir: PathBuf },
    #[error("'{0}' is not a directory")]
    NotADirectory(PathBuf),
    #[error("failed to open instance data file")]
    DataOpen(#[from] InstanceDataOpenError),
    #[error("failed to spawn writer thread")]
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

    /// Creates a [`Profile`] with the specified name.
    ///
    /// If the name is too long, or if it's the same as another profile in the instance,
    /// a new name is selected. This method returns the name that ends up being used.
    ///
    /// The profile's display name will always be set to the originally specified name,
    /// even if this method picks a new name.
    #[must_use]
    pub fn add_profile(&mut self, name: &str) -> CompactString {
        let name = name.trim();
        let profile = Profile::new(CompactString::new(name));

        // Limit names to 24 bytes to always fit in compact_str's small string optimization
        const LIMIT: usize = 24;
        let truncated_name = truncate_str(name, LIMIT);
        let mut actual_name = truncated_name.clone();

        let mut n: u32 = 0;
        while self.data.profiles.contains_key(&actual_name) {
            n = n.strict_add(1);
            let n_str = format_compact!("{}", n);

            actual_name = truncate_str(&truncated_name, LIMIT.strict_sub(n_str.len()));
            actual_name.push_str(&n_str);
        }
        assert!(!actual_name.is_heap_allocated());

        self.changed = true;
        let _ = self.data.profiles.insert(actual_name.clone(), profile);
        actual_name
    }

    /// Creates a new mod with the specified name.
    pub fn create_mod(&mut self, name: &str, kind: ModEntryKind) -> Result<(), CreateModError> {
        if self.mods().iter().any(|m| m.name() == name) {
            return Err(CreateModError::AlreadyExists);
        }

        let mod_decl = ModDeclaration::new(name.into(), kind)?;

        self.changed = true;
        let idx = self.data.mods.push_and_get_key(mod_decl);
        self.mod_order_mut().push(ModOrderEntry::new(idx));

        Mod::init(self, idx).map_err(Into::into)
    }

    /// Removes the specified mod.
    ///
    /// The mod's files are not deleted. This function returns the path to the mod directory,
    /// if applicable, so that the caller can delete the files.
    ///
    /// `ModIndex`s greater or equal to `idx` are invalidated when this method is called,
    /// as well as `ModOrderIndex`s greater or equal to the `ModOrderIndex` corresponding to
    /// the removed mod in each profile's mod order.
    pub fn remove_mod(&mut self, idx: ModIndex) -> Option<PathBuf> {
        self.changed = true;

        self.data.profiles.values_mut().for_each(|p| {
            p.mod_order.retain_mut(|entry| {
                let retain = entry.mod_index() != idx;
                if entry.mod_index() > idx {
                    entry.decrement_index();
                }
                retain
            });
        });

        let mod_decl = self.data.mods.remove(idx);
        self.mod_dir(&mod_decl)
    }

    /// Renames the specified mod.
    pub fn rename_mod(&mut self, idx: ModIndex, new_name: &str) -> Result<(), RenameModError> {
        if self.data.mods.iter().any(|m| m.name() == new_name) {
            return Err(RenameModError::AlreadyExists);
        }

        let mod_decl = &self.data.mods[idx];
        if let Some(from) = self.mod_dir(mod_decl) {
            let to = from.with_file_name(new_name);
            fs::rename(from, to)?;
        }

        self.data.mods[idx] = ModDeclaration::new(new_name.into(), mod_decl.kind())?;
        Ok(())
    }

    /// Toggles the enabled state of a mod in the mod order.
    pub fn toggle_mod_enabled(&mut self, index: ModOrderIndex) {
        self.changed = true;
        let entry = &mut self.mod_order_mut()[index];
        entry.enabled = !entry.enabled;
    }

    /// Moves a set of mods to a specific index in the mod order.
    pub fn move_mods(&mut self, mods_to_move: &HashSet<ModOrderIndex>, to: ModOrderIndex) -> ModOrderIndex {
        self.changed = true;
        move_multiple(
            self.mod_order_mut().as_mut(),
            mods_to_move.iter().map(|idx| (*idx).into()),
            to.into(),
        )
        .into()
    }
}

#[derive(Debug, Error)]
pub enum CreateModError {
    #[error("there already exists a mod with the specified name")]
    AlreadyExists,
    #[error(transparent)]
    InvalidName(#[from] InvalidModNameError),
    #[error("failed to initialize mod directory")]
    Init(#[from] ModInitError),
}

#[derive(Debug, Error)]
pub enum RenameModError {
    #[error("there already exists a mod with the specified name")]
    AlreadyExists,
    #[error(transparent)]
    InvalidName(#[from] InvalidModNameError),
    #[error("failed to rename mod directory")]
    Io(#[from] io::Error),
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

fn truncate_str(s: &str, len: usize) -> CompactString {
    let mut truncated = CompactString::default();
    for cluster in s.graphemes(true) {
        if truncated.len() + cluster.len() > len {
            break;
        }
        truncated.push_str(cluster);
    }
    truncated
}
