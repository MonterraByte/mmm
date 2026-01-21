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

//! Representation and (de)serialization of instance data.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::File;
use std::io::{self, BufReader};
use std::marker::PhantomData;
use std::path::Path;

use cbor4ii::serde::DecodeError;
use compact_str::CompactString;
use const_format::formatcp;
use serde::de::{Error, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;
use typed_index_collections::TiVec;

use super::{ModDeclaration, ModIndex, Profile};

/// File name of the instance data file in the instance's root directory.
pub const INSTANCE_DATA_FILE: &str = "mmm.cbor";
const INSTANCE_DATA_VERSION: u32 = 0;

/// Data contained in the instance data file.
///
/// When modifying the data within, the programmer is responsible for its integrity.
/// Indices must be kept in sync, mods must have exactly one entry in the mod order, etc.
///
/// Useful for implementing [`Instance`](super::Instance).
#[derive(Debug, Serialize)]
pub struct InstanceData {
    #[serde(serialize_with = "serialize_version")]
    version: PhantomData<u32>, // Keep this at the top of the struct, so it gets (de)serialized first.
    pub mods: TiVec<ModIndex, ModDeclaration>,
    pub profiles: BTreeMap<CompactString, Profile>,
}

#[allow(clippy::trivially_copy_pass_by_ref, reason = "required by serde")]
fn serialize_version<S: Serializer>(_: &PhantomData<u32>, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_u32(INSTANCE_DATA_VERSION)
}

impl InstanceData {
    /// Deserializes `InstanceData` from the file at the provided path.
    pub fn from_file(path: &Path) -> Result<Self, InstanceDataOpenError> {
        UnverifiedInstanceData::from_file(path)?.verify().map_err(Into::into)
    }
}

#[derive(Debug, Deserialize)]
struct UnverifiedInstanceData {
    #[serde(deserialize_with = "deserialize_version")]
    version: PhantomData<u32>,
    mods: TiVec<ModIndex, ModDeclaration>,
    profiles: BTreeMap<CompactString, Profile>,
}

#[allow(clippy::unnecessary_wraps, clippy::needless_pass_by_value, reason = "required by serde")]
fn deserialize_version<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PhantomData<u32>, D::Error> {
    deserializer.deserialize_u32(VersionVisitor).and(Ok(PhantomData))
}

/// A `serde` visitor that returns an error if the integer it visits does not equal [`INSTANCE_DATA_VERSION`].
struct VersionVisitor;

macro_rules! version_impl {
    ($fn_name:ident, $ty:ty) => {
        #[allow(irrefutable_let_patterns)]
        fn $fn_name<E: Error>(self, v: $ty) -> Result<Self::Value, E> {
            let expected: Result<$ty, _> = INSTANCE_DATA_VERSION.try_into();
            if let Ok(e) = expected
                && v == e
            {
                Ok(())
            } else {
                Err(E::custom(format_args!(
                    "expected data version {INSTANCE_DATA_VERSION}, found version {v}"
                )))
            }
        }
    };
}

impl Visitor<'_> for VersionVisitor {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("an unsigned integer")
    }

    version_impl!(visit_i8, i8);
    version_impl!(visit_i16, i16);
    version_impl!(visit_i32, i32);
    version_impl!(visit_i64, i64);
    version_impl!(visit_u8, u8);
    version_impl!(visit_u16, u16);
    version_impl!(visit_u32, u32);
    version_impl!(visit_u64, u64);
}

const VERSION_MISMATCH_ERROR_PREFIX: &str = formatcp!("expected data version {INSTANCE_DATA_VERSION}, found version ");

impl UnverifiedInstanceData {
    pub fn from_file(path: &Path) -> Result<Self, InstanceDataOpenError> {
        let file = File::open(path).map_err(InstanceDataOpenError::Open)?;
        let reader = BufReader::new(file);

        cbor4ii::serde::from_reader(reader).map_err(|err| match err {
            DecodeError::Custom(msg) if msg.starts_with(VERSION_MISMATCH_ERROR_PREFIX) => {
                let (_, version_str) = msg.split_at(VERSION_MISMATCH_ERROR_PREFIX.len());
                let version = version_str.parse().expect("error contains version number");
                InstanceDataOpenError::UnsupportedVersion(version)
            }
            _ => InstanceDataOpenError::Deserialize(err),
        })
    }

    pub fn verify(self) -> Result<InstanceData, InstanceDataVerificationError> {
        let mods_len = self.mods.len();
        for profile in self.profiles.values() {
            Self::verify_profile(profile, mods_len)?;
        }

        Ok(InstanceData {
            version: PhantomData,
            mods: self.mods,
            profiles: self.profiles,
        })
    }

    fn verify_profile(profile: &Profile, mods_len: usize) -> Result<(), InstanceDataVerificationError> {
        let mut mods_present = vec![false; mods_len];
        for order_entry in &profile.mod_order {
            let idx: usize = order_entry.mod_index().into();
            match mods_present.get(idx).copied() {
                Some(false) => mods_present[idx] = true,
                Some(true) => return Err(InstanceDataVerificationError::DuplicateModIndex),
                None => return Err(InstanceDataVerificationError::ModIndexOutOfRange),
            }
        }
        Ok(())
    }
}

/// Error type returned when verifying invalid instance data.
#[derive(Debug, Error)]
pub enum InstanceDataVerificationError {
    #[error("mod order contains duplicate mod indices")]
    DuplicateModIndex,
    #[error("mod order contains out of range mod index")]
    ModIndexOutOfRange,
}

/// Error type returned by [`InstanceData::from_file`].
#[derive(Debug, Error)]
pub enum InstanceDataOpenError {
    #[error("failed to deserialize instance data: {0}")]
    Deserialize(#[from] DecodeError<io::Error>),
    #[error("instance data file contains invalid data: {0}")]
    InvalidData(#[from] InstanceDataVerificationError),
    #[error("failed to open instance data file: {0}")]
    Open(#[source] io::Error),
    #[error("instance data file contains version {0} data, but version {INSTANCE_DATA_VERSION} is expected")]
    UnsupportedVersion(u32),
}
