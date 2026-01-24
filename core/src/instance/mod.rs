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

//! Interfaces for the core data needed to work with mods.

pub mod data;

use std::fmt;
use std::path::{Path, PathBuf};

use compact_str::CompactString;
use serde::de::{self, MapAccess, Unexpected, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use typed_index_collections::{TiSlice, TiVec};

/// Trait that represents an open mmm instance.
pub trait Instance {
    /// Returns the absolute path to the instance's base directory.
    fn dir(&self) -> &Path;

    /// Returns the [`ModDeclaration`]s contained in the instance.
    fn mods(&self) -> &TiSlice<ModIndex, ModDeclaration>;

    /// Returns the mod order of the current instance profile.
    ///
    /// Each [`ModDeclaration`] in [`Self::mods`] must not have more than one
    /// corresponding entry in the mod order.
    ///
    /// Entries that appear last have higher priority,
    /// and their files override the files of entries that appear earlier.
    fn mod_order(&self) -> &TiSlice<ModOrderIndex, ModOrderEntry>;

    /// Returns the absolute path to the specified mod's directory.
    fn mod_dir(&self, mod_declaration: &ModDeclaration) -> PathBuf {
        let mut path = self.dir().to_owned();
        path.push("mods");
        path.push(mod_declaration.name());
        path
    }
}

/// An entry in the [mod list](Instance::mods).
#[derive(Debug, Eq, PartialEq)]
pub struct ModDeclaration {
    name: CompactString,
    kind: ModEntryKind,
}

impl ModDeclaration {
    /// Returns the entry's name.
    #[must_use]
    pub const fn name(&self) -> &CompactString {
        &self.name
    }

    /// Returns the entry's type.
    #[must_use]
    pub const fn kind(&self) -> ModEntryKind {
        self.kind
    }
}

impl Serialize for ModDeclaration {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if matches!(self.kind, ModEntryKind::Mod) {
            serializer.serialize_str(&self.name)
        } else {
            let mut entry = serializer.serialize_struct("ModDeclaration", 2)?;
            entry.serialize_field("name", &self.name)?;
            entry.serialize_field("type", &self.kind)?;
            entry.end()
        }
    }
}

impl<'de> Deserialize<'de> for ModDeclaration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum Field {
            Name,
            Type,
        }
        struct ModDeclarationVisitor;

        impl<'de> Visitor<'de> for ModDeclarationVisitor {
            type Value = ModDeclaration;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("string or struct ModDeclaration")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(ModDeclaration {
                    name: CompactString::from(v),
                    kind: ModEntryKind::Mod,
                })
            }

            // Not the same as visit_str, as CompactString's `From<String>` takes ownership of the `String`.
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(ModDeclaration {
                    name: CompactString::from(v),
                    kind: ModEntryKind::Mod,
                })
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut name = None;
                let mut kind = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Name => {
                            if name.is_some() {
                                return Err(de::Error::duplicate_field("index"));
                            }
                            name = Some(map.next_value()?);
                        }
                        Field::Type => {
                            if kind.is_some() {
                                return Err(de::Error::duplicate_field("type"));
                            }
                            kind = Some(map.next_value()?);
                        }
                    }
                }
                let name = name.ok_or_else(|| de::Error::missing_field("name"))?;
                let kind = kind.ok_or_else(|| de::Error::missing_field("type"))?;
                Ok(ModDeclaration { name, kind })
            }
        }

        deserializer.deserialize_any(ModDeclarationVisitor)
    }
}

/// The type of entry in the mod list.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum ModEntryKind {
    /// A mod.
    #[default]
    Mod,
    /// An entry for organizing the mod list. Not a real mod.
    Separator,
}

pub const DEFAULT_PROFILE_NAME: CompactString = CompactString::const_new("default");
pub const DEFAULT_PROFILE: Profile = Profile {
    display_name: CompactString::const_new("Default"),
    mod_order: TiVec::new(),
};

/// Set of configurations that can be swapped within the same instance.
///
/// This includes mod order and activation state.
#[derive(Debug, Serialize, Deserialize)]
pub struct Profile {
    display_name: CompactString,
    pub mod_order: TiVec<ModOrderIndex, ModOrderEntry>,
}

/// Represents a [`ModDeclaration`] in the [mod order](Instance::mod_order).
#[derive(Copy, Clone, Debug)]
pub struct ModOrderEntry {
    index: ModIndex,
    /// The activation state of this mod.
    pub enabled: bool,
}

impl ModOrderEntry {
    /// Creates a new disabled `ModOrderEntry`.
    #[must_use]
    pub const fn new(index: ModIndex) -> Self {
        Self { index, enabled: false }
    }

    /// The index of the [`ModDeclaration`] represented by this entry in the [mod list](Instance::mods).
    #[must_use]
    pub const fn mod_index(&self) -> ModIndex {
        self.index
    }
}

/// A custom de(serializer) is used to save a few bytes in this type's representation.
///
/// Since there can easily be hundreds or thousands of entries in a single mod order,
/// the space savings add up quickly.
impl Serialize for ModOrderEntry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if self.enabled {
            serializer.serialize_u32(self.index.0)
        } else {
            let mut entry = serializer.serialize_struct("ModOrderEntry", 2)?;
            entry.serialize_field("i", &self.index)?;
            entry.serialize_field("e", &self.enabled)?;
            entry.end()
        }
    }
}

impl<'de> Deserialize<'de> for ModOrderEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum Field {
            #[serde(rename = "i")]
            Index,
            #[serde(rename = "e")]
            Enabled,
        }
        struct ModEntryVisitor;

        macro_rules! visitor_impl {
            ($fn_name:ident, $ty:ty, $impl_fn:ident) => {
                fn $fn_name<E: de::Error>(self, v: $ty) -> Result<Self::Value, E> {
                    $impl_fn(v)
                }
            };
        }

        #[allow(clippy::unnecessary_wraps, reason = "required by serde")]
        fn from_idx<E: de::Error, I: Into<u32> + Sized + Copy>(i: I) -> Result<ModOrderEntry, E> {
            Ok(ModOrderEntry { index: ModIndex(i.into()), enabled: true })
        }

        #[allow(clippy::unnecessary_wraps, reason = "required by serde")]
        fn try_from_idx_signed<E: de::Error, I: TryInto<u32> + Into<i64> + Sized + Copy>(
            input: I,
        ) -> Result<ModOrderEntry, E> {
            let i: u32 = input.try_into().ok().ok_or(E::invalid_value(
                Unexpected::Signed(Into::<i64>::into(input)),
                &"an unsigned integer up to 2^32 - 1",
            ))?;
            Ok(ModOrderEntry { index: ModIndex(i), enabled: true })
        }

        impl<'de> Visitor<'de> for ModEntryVisitor {
            type Value = ModOrderEntry;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("unsigned integer or table")
            }

            visitor_impl!(visit_i8, i8, try_from_idx_signed);
            visitor_impl!(visit_i16, i16, try_from_idx_signed);
            visitor_impl!(visit_i32, i32, try_from_idx_signed);
            visitor_impl!(visit_i64, i64, try_from_idx_signed);
            visitor_impl!(visit_u8, u8, from_idx);
            visitor_impl!(visit_u16, u16, from_idx);
            visitor_impl!(visit_u32, u32, from_idx);

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                let i: u32 = v.try_into().ok().ok_or(E::invalid_value(
                    Unexpected::Unsigned(v),
                    &"an unsigned integer up to 2^32 - 1",
                ))?;
                Ok(ModOrderEntry { index: ModIndex(i), enabled: true })
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut index = None;
                let mut enabled = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Index => {
                            if index.is_some() {
                                return Err(de::Error::custom("duplicate field `i` (index)"));
                            }
                            index = Some(map.next_value()?);
                        }
                        Field::Enabled => {
                            if enabled.is_some() {
                                return Err(de::Error::custom("duplicate field `e` (enabled)"));
                            }
                            enabled = Some(map.next_value()?);
                        }
                    }
                }
                let index = index.ok_or_else(|| de::Error::custom("missing field `i` (index)"))?;
                let enabled = enabled.ok_or_else(|| de::Error::custom("missing field `e` (enabled)"))?;
                Ok(ModOrderEntry { index, enabled })
            }
        }

        deserializer.deserialize_any(ModEntryVisitor)
    }
}

macro_rules! custom_index {
    ($name:ident, $doc:literal) => {
        // Not `usize` to reduce memory usage and cache locality. 2^32 - 1 mods are surely enough for everyone.
        #[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[doc = $doc]
        pub struct $name(u32);

        impl From<usize> for $name {
            fn from(value: usize) -> Self {
                Self(u32::try_from(value).expect("mod count does not exceed 2^32 - 1"))
            }
        }

        impl From<$name> for usize {
            fn from(value: $name) -> usize {
                value.0 as usize
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

custom_index!(ModIndex, "Index type for [`Instance::mods`].");
custom_index!(ModOrderIndex, "Index type for [`Instance::mod_order`].");
