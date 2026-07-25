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

//! Mod installation functionality.

pub mod fomod;
pub mod staging;

use std::collections::BTreeMap;

use arrayvec::ArrayVec;
use foldhash::HashMap;
use nary_tree::NodeId;
use thiserror::Error;

use crate::EditableInstance;
use crate::archive::{Archive, ExtractSelection};
use crate::install::fomod::info::InfoFromBytesError;
use crate::install::fomod::module_config::ModuleConfigFromBytesError;
use crate::install::fomod::{FomodFiles, FomodInstaller};
use crate::util::SharedStr;

pub type Warnings = BTreeMap<WarningSource, Vec<Box<str>>>;

/// Mod installation data read from an archive.
#[derive(Debug, Default)]
pub struct InstallableArchive {
    pub installer: Installer,
    pub metadata: Metadata,
    pub warnings: Warnings,
    pub images: HashMap<NodeId, Vec<u8>>,
}

impl InstallableArchive {
    pub fn from_archive(archive: &mut Archive) -> Result<Box<Self>, FromArchiveError> {
        let mut installer = Installer::default();
        let mut metadata = Metadata::default();
        let mut warnings = BTreeMap::new();
        let mut images = HashMap::default();

        if let Some(fomod) = FomodFiles::probe(archive) {
            let file_ids: ArrayVec<_, { FomodFiles::FILE_COUNT }> = fomod.ids().collect();
            let mut files = archive
                .read_files(file_ids.as_slice())
                .map_err(FromArchiveError::ArchiveRead)?;

            if let Some(result) = fomod.get_installer(&mut files) {
                let (mut mc, mc_warnings) = result?;

                let fomod_image_ids = mc.resolve_images(archive);
                let fomod_images = archive
                    .read_files(&fomod_image_ids)
                    .map_err(FromArchiveError::ArchiveRead)?;
                images.extend(fomod_images);

                installer = Installer::Fomod(mc);

                if !mc_warnings.is_empty() {
                    let mc_warnings = mc_warnings
                        .into_iter()
                        .map(|e| e.to_string().into_boxed_str())
                        .collect();
                    warnings.insert(WarningSource::FomodModuleConfig, mc_warnings);
                }
            }

            if let Some(result) = fomod.get_metadata(&mut files) {
                let (info, info_warnings) = result?;
                metadata.fomod = Some(info);

                if !info_warnings.is_empty() {
                    let info_warnings = info_warnings
                        .into_iter()
                        .map(|e| e.to_string().into_boxed_str())
                        .collect();
                    warnings.insert(WarningSource::FomodInfo, info_warnings);
                }
            }
        }

        Ok(Box::new(Self { installer, metadata, warnings, images }))
    }
}

impl InstallableArchive {
    /// The name of the mod contained in this archive.
    #[must_use]
    pub fn mod_name(&self) -> Option<SharedStr> {
        self.installer.mod_name().or_else(|| self.metadata.mod_name())
    }
}

#[allow(
    clippy::large_enum_variant,
    reason = "don't think the additional indirection is justified, the outer InstallableArchive is already in a Box"
)]
#[derive(Debug, Default)]
pub enum Installer {
    Fomod(FomodInstaller),
    #[default]
    Plain,
}

impl Installer {
    fn mod_name(&self) -> Option<SharedStr> {
        match self {
            Installer::Fomod(fomod) => fomod.mod_name(),
            Installer::Plain => None,
        }
    }

    /// Initializes the installer with the provided context.
    ///
    /// If the installer is done at this point, returns the [`ExtractSelection`] that should be used to install the mod.
    pub fn init(
        &mut self,
        archive: &Archive,
        instance: &EditableInstance,
    ) -> Result<Option<ExtractSelection>, Box<str>> {
        match self {
            Installer::Fomod(fomod) => fomod.next(archive, instance),
            Installer::Plain => Ok(Some(ExtractSelection::entire_archive(archive))),
        }
    }

    /// Prepares the installer for being used again after it has been completed.
    pub fn back(&mut self, instance: &EditableInstance) {
        match self {
            Installer::Fomod(fomod) => fomod.back(instance),
            Installer::Plain => panic!("there is no installer"),
        }
    }

    /// Returns if it's possible to go back into the installer after it has been completed.
    #[must_use]
    pub fn can_go_back(&self, instance: &EditableInstance) -> bool {
        match self {
            Installer::Fomod(fomod) => fomod.can_go_back(instance),
            Installer::Plain => false,
        }
    }
}

/// Metadata about the contained mod.
#[derive(Debug, Default)]
pub struct Metadata {
    pub fomod: Option<fomod::info::Info>,
}

impl Metadata {
    fn mod_name(&self) -> Option<SharedStr> {
        if let Some(info) = &self.fomod
            && let Some(name) = &info.name
        {
            return Some(name.clone());
        }

        None
    }
}

/// The part of the archive associated with a specific warning.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum WarningSource {
    FomodModuleConfig,
    FomodInfo,
}

impl WarningSource {
    /// User-facing name for the source of the warning.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            WarningSource::FomodModuleConfig => "fomod/ModuleConfig.xml",
            WarningSource::FomodInfo => "fomod/info.xml",
        }
    }
}

/// Error type returned by [`InstallableArchive::from_archive`].
#[derive(Debug, Error)]
pub enum FromArchiveError {
    #[error("failed to read files from archive")]
    ArchiveRead(#[source] anyhow::Error),
    #[error("failed to obtain FOMOD info data")]
    FomodInfo(#[from] InfoFromBytesError),
    #[error("failed to obtain FOMOD ModuleConfig data ")]
    FomodModuleConfig(#[from] ModuleConfigFromBytesError),
}
