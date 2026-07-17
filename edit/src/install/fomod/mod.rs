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

//! [FOMOD](https://fomod-docs.readthedocs.io/en/latest/index.html) installer support

#![expect(unused)]

pub mod info;
pub mod module_config;

use std::fmt::{self, Debug, Display};

use nary_tree::NodeId;
use roxmltree::{Node, TextPos};

use mmm_core::file_tree::util::OptionExt as _;

use self::info::{IError, Info, InfoFromBytesError};
use self::module_config::{McError, ModuleConfig, ModuleConfigFromBytesError};
use crate::archive::{Archive, FileReadMap};
use crate::util::{SharedStr, find_child_with_case_insensitive_name, find_entry_in_nested_outer_directories};

/// The location of FOMOD files within an archive.
#[derive(Debug)]
pub(super) struct FomodFiles {
    /// The root directory of the installer (the parent of the `fomod` directory).
    root: NodeId,
    /// The node of the `info.xml` file.
    info: Option<NodeId>,
    /// The node of the `ModuleConfig.xml` file.
    module_config: Option<NodeId>,
}

impl FomodFiles {
    /// How many files to read from the archive.
    pub const FILE_COUNT: usize = 2;

    /// Finds the FOMOD files in the specified archive, if any.
    #[must_use]
    pub fn probe(archive: &Archive) -> Option<Self> {
        let (fomod_id, root) = find_entry_in_nested_outer_directories(
            archive.tree(),
            archive.tree().root_id().expect("has root node"),
            "fomod",
        )?;

        let fomod_dir = archive.tree().get(fomod_id).expect("node exists");
        let info = find_child_with_case_insensitive_name(&fomod_dir, "info.xml").node_id();
        let module_config = find_child_with_case_insensitive_name(&fomod_dir, "ModuleConfig.xml").node_id();

        Some(Self { root, info, module_config })
    }

    /// The `NodeId`s of the files that should be read from the archive.
    pub fn ids(&self) -> impl Iterator<Item = &NodeId> {
        self.info.iter().chain(self.module_config.iter())
    }

    /// Takes the metadata file from the set of files read from the archive and parses it.
    pub fn get_metadata(&self, map: &mut FileReadMap) -> Option<Result<(Info, Vec<IError>), InfoFromBytesError>> {
        let data = map.remove(&self.info?)?;
        Some(Info::from_bytes(data))
    }
}

#[derive(Debug)]
pub struct XmlError<K: Debug + Display> {
    pub location: TextPos,
    pub kind: K,
}

impl<K: Debug + Display> XmlError<K> {
    fn new(node: &Node, kind: K) -> Self {
        std::hint::cold_path();

        let location = node.document().text_pos_at(node.range().start);
        Self { location, kind }
    }
}

impl<K: Debug + Display> Display for XmlError<K> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "line {}, col {}: {}",
            self.location.row, self.location.col, self.kind
        )
    }
}

impl<K: Debug + Display> std::error::Error for XmlError<K> {}

fn get_text_str(node: &Node) -> Option<SharedStr> {
    node.text().map(str::trim).filter(|s| !s.is_empty()).map(SharedStr::new)
}

fn get_attribute_str(node: &Node, attribute_name: &'static str) -> Option<SharedStr> {
    node.attribute(attribute_name).map(SharedStr::new)
}

fn get_attribute_str_or_empty(node: &Node, attribute_name: &'static str) -> SharedStr {
    node.attribute(attribute_name).map(SharedStr::new).unwrap_or_default()
}

#[allow(
    clippy::wrong_self_convention,
    reason = "these methods both check and alter the contained value"
)]
trait OptionExt<T> {
    /// Executes the provided closure if `self` is `None`, setting `self` to a `Some` containing its result.
    ///
    /// Returns `true` if `self` was `Some` beforehand, or `false` otherwise.
    fn is_some_or_set(&mut self, f: impl FnMut() -> T) -> bool;

    /// Executes the provided closure if `self` is `None`, setting `self` to its result if successful.
    ///
    /// If the closure fails, its error is returned.
    /// Returns `Ok(true)` if `self` was `Some` beforehand, or `Ok(false)` otherwise.
    fn is_some_or_set_ok<E>(&mut self, f: impl FnMut() -> Result<T, E>) -> Result<bool, E>;

    /// Executes the provided closure if `self` is `None`, setting `self` to its result.
    ///
    /// Returns `true` if `self` was `Some` beforehand, or `false` otherwise.
    fn is_some_or_set_opt(&mut self, f: impl FnMut() -> Option<T>) -> bool;
}

impl<T> OptionExt<T> for Option<T> {
    fn is_some_or_set(&mut self, mut f: impl FnMut() -> T) -> bool {
        if self.is_none() {
            *self = Some(f());
            false
        } else {
            true
        }
    }

    fn is_some_or_set_ok<E>(&mut self, mut f: impl FnMut() -> Result<T, E>) -> Result<bool, E> {
        if self.is_none() {
            *self = Some(f()?);
            Ok(false)
        } else {
            Ok(true)
        }
    }

    fn is_some_or_set_opt(&mut self, mut f: impl FnMut() -> Option<T>) -> bool {
        if self.is_none() {
            *self = f();
            false
        } else {
            true
        }
    }
}
