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

//! `ModuleConfig.xml` parser

use std::cmp::Ordering;
use std::debug_assert_matches;

use camino::Utf8Path;
use nary_tree::NodeId;
use roxmltree::{Document, Node};
use thiserror::Error;

use super::{OptionExt, XmlError, get_attribute_str, get_attribute_str_or_empty, get_text_str};
use crate::util::{FromUtf8OrUtf16Error, SharedStr, utf8_or_utf16_bytes_to_string};

pub type McError = XmlError<ModuleConfigError>;
type Result<T, E = McError> = std::result::Result<T, E>;
type WarningVec = Vec<McError>;

/// FOMOD installer data.
#[derive(Debug, Default)]
pub(super) struct ModuleConfig {
    /// The name of the mod.
    pub name: Name,

    /// Mod image.
    pub image: Option<Image>,

    /// Conditions required for the mod to be installed.
    ///
    /// If not met, the installation process should be aborted.
    pub dependencies: DependencyBlock,

    /// Files that are always selected for installation.
    pub required_install_files: InstallFiles,

    /// The steps, or pages, of the installer.
    pub install_steps: InstallSteps,

    /// Files that are selected for installation if the specified conditions are met.
    pub conditional_file_installs: ConditionalFileInstalls,
}

impl ModuleConfig {
    /// Parses bytes read from an `ModuleConfig.xml` into an instance of `ModuleConfig`.
    ///
    /// Additionally, it also returns a vector of warnings emitted during parsing.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<(Self, WarningVec), ModuleConfigFromBytesError> {
        let str = utf8_or_utf16_bytes_to_string(bytes)?;
        let doc = Document::parse(&str)?;
        Self::from_doc(&doc).map_err(Into::into)
    }

    /// Parses the specified XML document into an instance of `ModuleConfig`.
    ///
    /// Additionally, it also returns a vector of warnings emitted during parsing.
    pub fn from_doc(document: &Document) -> Result<(Self, WarningVec)> {
        let mut name = None;
        let mut image = None;
        let mut dependencies = None;
        let mut required_install_files = None;
        let mut install_steps = None;
        let mut conditional_file_installs = None;

        let mut warnings = Vec::new();

        let config_node = document
            .root()
            .children()
            .find(|node| node.is_element() && node.tag_name().name() == "config")
            .ok_or_else(|| {
                XmlError::new(
                    &document.root(),
                    ModuleConfigError::MissingElement {
                        name: "config",
                        parent: SharedStr::from_static_str("root"),
                    },
                )
            })?;

        for child in config_node.children().filter(Node::is_element) {
            let repeated = match child.tag_name().name() {
                "moduleName" => name.is_some_or_set(|| Name::from_node(&child)),
                "moduleImage" => image.is_some_or_set_opt(|| Image::from_node(&child)),
                "moduleDependencies" => {
                    dependencies.is_some_or_set_ok(|| DependencyBlock::from_node(&child, &mut warnings))?
                }
                "requiredInstallFiles" => {
                    required_install_files.is_some_or_set_ok(|| InstallFiles::from_node(&child, &mut warnings))?
                }
                "installSteps" => install_steps.is_some_or_set_ok(|| InstallSteps::from_node(&child, &mut warnings))?,
                "conditionalFileInstalls" => conditional_file_installs
                    .is_some_or_set_ok(|| ConditionalFileInstalls::from_node(&child, &mut warnings))?,
                other => {
                    warnings.push(invalid_element(&child, "moduleName, moduleImage, moduleDependencies, requiredInstallFiles, installSteps or conditionalFileInstalls", other));
                    false
                }
            };

            if repeated {
                warnings.push(repeated_element(&child));
            }
        }

        let config = Self {
            name: name.unwrap_or_default(),
            image,
            dependencies: dependencies.unwrap_or_default(),
            required_install_files: required_install_files.unwrap_or_default(),
            install_steps: install_steps.unwrap_or_default(),
            conditional_file_installs: conditional_file_installs.unwrap_or_default(),
        };
        Ok((config, warnings))
    }
}

/// The name of the mod.
#[derive(Debug, Default)]
pub(super) struct Name(pub Option<SharedStr>);

impl Name {
    pub fn from_node(node: &Node) -> Self {
        debug_assert_eq!(node.tag_name().name(), "moduleName");
        Self(get_text_str(node))
    }
}

/// Set of files that can be installed.
#[derive(Debug, Default)]
pub(super) struct InstallFiles(Vec<InstallFile>);

impl InstallFiles {
    pub fn from_node(node: &Node, warnings: &mut WarningVec) -> Result<Self> {
        debug_assert_matches!(node.tag_name().name(), "files" | "requiredInstallFiles");
        let mut files = Vec::new();

        for child in node.children().filter(Node::is_element) {
            match InstallFile::from_node(&child) {
                Ok(file) => files.push(file),
                Err(
                    err @ XmlError {
                        kind:
                            ModuleConfigError::InvalidElementTag { .. }
                            | ModuleConfigError::MissingAttribute { name: "source", .. },
                        ..
                    },
                ) => warnings.push(err),
                Err(err) => return Err(err),
            }
        }

        // make sure to use a stable sort to preserve overwrites between entries with the same priority
        files.sort_by_key(|f| f.priority);

        Ok(Self(files))
    }
}

impl AsRef<[InstallFile]> for InstallFiles {
    fn as_ref(&self) -> &[InstallFile] {
        self.0.as_slice()
    }
}

/// A file (or directory) that can be installed.
#[derive(Debug)]
pub(super) struct InstallFile {
    /// Specifies if this is a file or a directory/folder.
    ///
    /// Given that we can more accurately know this by looking up the source path in the archive,
    /// it does not make sense to rely on this value for anything.
    pub kind: InstallFileKind,

    /// The path to the file within the archive, from the FOMOD root.
    source: SharedStr,

    /// The path relative to the game directory where this file should be installed.
    destination: SharedStr,

    /// The priority of the file. Used to select which file gets installed
    /// if multiple `InstallFiles` have the same destination.
    ///
    /// A higher number means greater priority.
    pub priority: i32,

    /// If `true`, this file is always installed.
    ///
    /// This holds even if it belongs to a hidden [`InstallStep`] or a "not usable" [`Plugin`].
    pub always_install: bool,

    /// If `true`, this file is always installed, unless it is from a plugin determined "not usable".
    ///
    /// This holds even if it belongs to a hidden [`InstallStep`].
    pub install_if_usable: bool,
}

#[derive(Copy, Clone, Debug)]
pub(super) enum InstallFileKind {
    File,
    Folder,
}

impl InstallFile {
    fn from_node(node: &Node) -> Result<Self> {
        let kind = match node.tag_name().name() {
            "file" => InstallFileKind::File,
            "folder" => InstallFileKind::Folder,
            other => return Err(invalid_element(node, "file or folder", other)),
        };

        let Some(source) = get_attribute_path(node, "source") else {
            return Err(missing_attribute(node, "source"));
        };
        let destination = get_attribute_path(node, "destination").unwrap_or_default();
        let priority = get_attribute_i32(node, "priority")?.unwrap_or(0);
        let always_install = get_attribute_bool(node, "alwaysInstall")?.unwrap_or(false);
        let install_if_usable = get_attribute_bool(node, "installIfUsable")?.unwrap_or(false);

        Ok(Self {
            kind,
            source,
            destination,
            priority,
            always_install,
            install_if_usable,
        })
    }

    /// The path to the file within the archive, from the FOMOD root.
    pub fn source(&self) -> &Utf8Path {
        Utf8Path::new(self.source.as_ref())
    }

    /// The path relative to the game directory where this file should be installed.
    pub fn destination(&self) -> &Utf8Path {
        Utf8Path::new(self.destination.as_ref())
    }
}

/// A group of [dependencies](Dependency).
#[derive(Debug, Default)]
pub(super) struct DependencyBlock {
    pub operator: Operator,
    pub deps: Vec<Dependency>,
}

impl DependencyBlock {
    fn from_node(node: &Node, warnings: &mut WarningVec) -> Result<Self> {
        debug_assert_matches!(
            node.tag_name().name(),
            "moduleDependencies" | "dependencies" | "visible"
        );

        let operator = Operator::from_node(node)?;
        let deps = parse_children_with(node, warnings, Dependency::from_node)?;

        Ok(Self { operator, deps })
    }
}

#[derive(Copy, Clone, Debug, Default)]
pub(super) enum Operator {
    #[default]
    And,
    Or,
}

impl Operator {
    fn from_node(node: &Node) -> Result<Self> {
        const ATTRIBUTE: &str = "operator";

        match node.attribute(ATTRIBUTE) {
            Some("And" | "") | None => Ok(Operator::And),
            Some("Or") => Ok(Operator::Or),
            Some(other) => Err(invalid_attribute_value(node, ATTRIBUTE, "And or Or", other)),
        }
    }
}

#[derive(Debug)]
pub(super) enum Dependency {
    Dependencies(DependencyBlock),
    File(FileDependency),
    Flag(Flag),
    Game(GameVersion),
    Fomm,
}

impl Dependency {
    fn from_node(node: &Node, warnings: &mut WarningVec) -> Result<Self> {
        match node.tag_name().name() {
            "dependencies" => Ok(Self::Dependencies(DependencyBlock::from_node(node, warnings)?)),
            "fileDependency" => Ok(Self::File(FileDependency::from_node(node)?)),
            "flagDependency" => Ok(Self::Flag(Flag::from_dependency_node(node))),
            "gameDependency" => Ok(Self::Game(GameVersion::from_node(node)?)),
            "fommDependency" => Ok(Self::Fomm),
            other => Err(invalid_element(
                node,
                "dependencies, fileDependency, flagDependency or gameDependency",
                other,
            )),
        }
    }
}

/// Dependency on a file in the game directory.
#[expect(unused)]
#[derive(Debug, Default)]
pub(super) struct FileDependency {
    /// Path to the file from the game root.
    file: SharedStr,

    /// The state required to fulfill the dependency.
    state: DependencyState,
}

impl FileDependency {
    fn from_node(node: &Node) -> Result<Self> {
        debug_assert_eq!(node.tag_name().name(), "fileDependency");

        let Some(file) = get_attribute_path(node, "file") else {
            return Err(missing_attribute(node, "file"));
        };
        let state = DependencyState::from_node(node)?;

        Ok(Self { file, state })
    }
}

/// The state specified by a [`FileDependency`].
#[derive(Copy, Clone, Debug, Default)]
pub enum DependencyState {
    /// The file is present and enabled.
    ///
    /// The meaning of "enabled" depends on the game, and perhaps the file type.
    Active,

    /// The file is present, but not enabled.
    #[default]
    Inactive,

    /// The file is missing.
    Missing,
}

impl DependencyState {
    fn from_node(node: &Node) -> Result<Self> {
        const ATTRIBUTE: &str = "state";

        match node.attribute(ATTRIBUTE) {
            Some("Active") => Ok(Self::Active),
            Some("Inactive" | "") | None => Ok(Self::Inactive),
            Some("Missing") => Ok(Self::Missing),
            Some(other) => Err(invalid_attribute_value(
                node,
                ATTRIBUTE,
                "Active, Inactive or Missing",
                other,
            )),
        }
    }
}

/// A key-value pair used for conditional logic in the installer.
///
/// Both values are case-sensitive.
#[derive(Debug)]
pub(super) struct Flag {
    pub name: FlagName,
    pub value: FlagValue,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub(super) struct FlagName(SharedStr);

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub(super) struct FlagValue(SharedStr);

impl Flag {
    fn from_condition_flag_node(node: &Node) -> Self {
        debug_assert_eq!(node.tag_name().name(), "flag");

        Self {
            name: FlagName(get_attribute_str_or_empty(node, "name")),
            value: FlagValue(get_text_str(node).unwrap_or_default()),
        }
    }

    fn from_dependency_node(node: &Node) -> Self {
        debug_assert_eq!(node.tag_name().name(), "flagDependency");

        Self {
            name: FlagName(get_attribute_str_or_empty(node, "flag")),
            value: FlagValue(get_attribute_str_or_empty(node, "value")),
        }
    }
}

/// String describing a game's version number.
#[expect(unused)]
#[derive(Debug)]
pub(super) struct GameVersion(pub SharedStr);

impl GameVersion {
    fn from_node(node: &Node) -> Result<Self> {
        debug_assert_eq!(node.tag_name().name(), "gameDependency");

        const ATTRIBUTE: &str = "version";
        if let Some(version) = get_attribute_str(node, ATTRIBUTE) {
            Ok(Self(version))
        } else {
            Err(missing_attribute(node, ATTRIBUTE))
        }
    }
}

/// The steps, or pages, of the installer.
#[derive(Debug, Default)]
pub(super) struct InstallSteps(Vec<InstallStep>);

/// The order in which items are displayed and processed.
#[derive(Copy, Clone, Debug, Default)]
pub(super) enum Order {
    /// Sort in alphabetical order.
    #[default]
    Ascending,

    /// Sort in reverse alphabetical order.
    Descending,

    /// Keep the order as it is in the XML document.
    Explicit,
}

impl Order {
    fn from_node(node: &Node) -> Result<Self> {
        const ATTRIBUTE: &str = "order";

        match node.attribute(ATTRIBUTE) {
            Some("Ascending" | "") | None => Ok(Self::Ascending),
            Some("Descending") => Ok(Self::Descending),
            Some("Explicit") => Ok(Self::Explicit),
            Some(other) => Err(invalid_attribute_value(
                node,
                ATTRIBUTE,
                "Ascending, Descending or Explicit",
                other,
            )),
        }
    }
}

impl InstallSteps {
    fn from_node(node: &Node, warnings: &mut WarningVec) -> Result<Self> {
        debug_assert_eq!(node.tag_name().name(), "installSteps");

        let order = Order::from_node(node)?;
        let mut steps = parse_children_with_filter(node, warnings, InstallStep::from_node, "installStep")?;

        match order {
            Order::Ascending => steps.sort_by(|a, b| a.name.cmp(&b.name)),
            Order::Descending => steps.sort_by(|a, b| b.name.cmp(&a.name)),
            Order::Explicit => (),
        }

        Ok(Self(steps))
    }
}

impl AsMut<[InstallStep]> for InstallSteps {
    fn as_mut(&mut self) -> &mut [InstallStep] {
        &mut self.0
    }
}

impl AsRef<[InstallStep]> for InstallSteps {
    fn as_ref(&self) -> &[InstallStep] {
        &self.0
    }
}

/// A single step, or page, of the installer.
#[derive(Debug)]
pub struct InstallStep {
    pub name: SharedStr,
    pub(super) visible: DependencyBlock,
    pub file_groups: FileGroups,
}

impl InstallStep {
    fn from_node(node: &Node, warnings: &mut WarningVec) -> Result<Self> {
        debug_assert_eq!(node.tag_name().name(), "installStep");

        let name = get_attribute_str(node, "name").unwrap_or_default();
        let mut visible = None;
        let mut file_groups = None;

        for child in node.children().filter(Node::is_element) {
            let repeated = match child.tag_name().name() {
                "visible" => visible.is_some_or_set_ok(|| DependencyBlock::from_node(&child, warnings))?,
                "optionalFileGroups" => file_groups.is_some_or_set_ok(|| FileGroups::from_node(&child, warnings))?,
                other => {
                    warnings.push(invalid_element(&child, "visible or optionalFileGroups", other));
                    false
                }
            };

            if repeated {
                warnings.push(repeated_element(&child));
            }
        }

        Ok(Self {
            name,
            visible: visible.unwrap_or_default(),
            file_groups: file_groups.unwrap_or_default(),
        })
    }
}

#[derive(Debug, Default)]
pub struct FileGroups(Vec<FileGroup>);

impl FileGroups {
    fn from_node(node: &Node, warnings: &mut WarningVec) -> Result<Self> {
        debug_assert_eq!(node.tag_name().name(), "optionalFileGroups");

        let order = Order::from_node(node)?;
        let mut groups = parse_children_with_filter(node, warnings, FileGroup::from_node, "group")?;

        match order {
            Order::Ascending => groups.sort_by(|a, b| a.name.cmp(&b.name)),
            Order::Descending => groups.sort_by(|a, b| b.name.cmp(&a.name)),
            Order::Explicit => (),
        }

        Ok(Self(groups))
    }
}

impl AsMut<[FileGroup]> for FileGroups {
    fn as_mut(&mut self) -> &mut [FileGroup] {
        &mut self.0
    }
}

impl AsRef<[FileGroup]> for FileGroups {
    fn as_ref(&self) -> &[FileGroup] {
        &self.0
    }
}

/// A section in a step of the installer, in which a selection of [`Plugin`]s can be picked for installation.
#[derive(Debug)]
pub struct FileGroup {
    pub name: GroupName,
    pub ty: FileGroupType,
    pub plugins: Plugins,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct GroupName(pub SharedStr);

impl FileGroup {
    fn from_node(node: &Node, warnings: &mut WarningVec) -> Result<Self> {
        debug_assert_eq!(node.tag_name().name(), "group");

        let name = GroupName(get_attribute_str(node, "name").unwrap_or_default());
        let ty = FileGroupType::from_node(node)?;
        let plugins = parse_single_child_with(node, warnings, Plugins::from_node, "plugins")?.unwrap_or_default();

        Ok(Self { name, ty, plugins })
    }
}

/// Criteria for selecting plugins in a [`FileGroup`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FileGroupType {
    SelectAtLeastOne,
    SelectAtMostOne,
    SelectExactlyOne,
    SelectAll,
    SelectAny,
}

impl FileGroupType {
    fn from_node(node: &Node) -> Result<Self> {
        const ATTRIBUTE: &str = "type";

        match node.attribute(ATTRIBUTE) {
            Some("SelectAtLeastOne") => Ok(Self::SelectAtLeastOne),
            Some("SelectAtMostOne") => Ok(Self::SelectAtMostOne),
            Some("SelectExactlyOne") => Ok(Self::SelectExactlyOne),
            Some("SelectAll") => Ok(Self::SelectAll),
            Some("SelectAny") => Ok(Self::SelectAny),
            Some(other) => Err(invalid_attribute_value(
                node,
                ATTRIBUTE,
                "SelectAtLeastOne, SelectAtMostOne, SelectExactlyOne, SelectAll or SelectAny",
                other,
            )),
            None => Err(missing_attribute(node, ATTRIBUTE)),
        }
    }

    #[must_use]
    pub const fn allows_multiple(&self) -> bool {
        matches!(self, Self::SelectAny | Self::SelectAll | Self::SelectAtLeastOne)
    }

    #[must_use]
    pub const fn allows_none(&self) -> bool {
        matches!(self, Self::SelectAny | Self::SelectAtMostOne)
    }
}

#[derive(Debug, Default)]
pub struct Plugins(Vec<Plugin>);

impl Plugins {
    fn from_node(node: &Node, warnings: &mut WarningVec) -> Result<Self> {
        debug_assert_eq!(node.tag_name().name(), "plugins");

        let order = Order::from_node(node)?;
        let mut plugins = parse_children_with_filter(node, warnings, Plugin::from_node, "plugin")?;

        match order {
            Order::Ascending => plugins.sort_by(|a, b| a.name.cmp(&b.name)),
            Order::Descending => plugins.sort_by(|a, b| b.name.cmp(&a.name)),
            Order::Explicit => (),
        }

        Ok(Self(plugins))
    }
}

impl AsMut<[Plugin]> for Plugins {
    fn as_mut(&mut self) -> &mut [Plugin] {
        &mut self.0
    }
}

impl AsRef<[Plugin]> for Plugins {
    fn as_ref(&self) -> &[Plugin] {
        &self.0
    }
}

/// An item that can be selected for installation.
#[derive(Debug)]
pub struct Plugin {
    /// The name of the plugin.
    pub name: PluginName,

    /// The description of the plugin.
    pub description: SharedStr,

    /// Image that describes this plugin.
    pub image: Option<Image>,

    /// The [`InstallFile`]s to install when this plugin is selected.
    pub(super) files: InstallFiles,

    /// Flag values to be set if this plugin is selected.
    pub(super) condition_flags: ConditionFlags,

    /// Expression that determines this plugin's [`PluginType`].
    pub(super) type_descriptor: TypeDescriptor,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PluginName(pub SharedStr);

impl Plugin {
    fn from_node(node: &Node, warnings: &mut WarningVec) -> Result<Self> {
        debug_assert_eq!(node.tag_name().name(), "plugin");

        let name = PluginName(get_attribute_str(node, "name").unwrap_or_default());
        let mut description = None;
        let mut image = None;
        let mut files = None;
        let mut condition_flags = None;
        let mut type_descriptor = None;

        for child in node.children().filter(Node::is_element) {
            let repeated = match child.tag_name().name() {
                "description" => description.is_some_or_set(|| get_text_str(&child).unwrap_or_default()),
                "image" => image.is_some_or_set_opt(|| Image::from_node(&child)),
                "files" => files.is_some_or_set_ok(|| InstallFiles::from_node(&child, warnings))?,
                "conditionFlags" => condition_flags.is_some_or_set(|| ConditionFlags::from_node(&child, warnings)),
                "typeDescriptor" => {
                    type_descriptor.is_some_or_set_ok(|| TypeDescriptor::from_node(&child, warnings))?
                }
                other => {
                    warnings.push(invalid_element(
                        &child,
                        "description, image, files, conditionFlags or typeDescriptor",
                        other,
                    ));
                    false
                }
            };

            if repeated {
                warnings.push(repeated_element(&child));
            }
        }

        Ok(Self {
            name,
            description: description.unwrap_or_default(),
            image,
            files: files.unwrap_or_default(),
            condition_flags: condition_flags.unwrap_or_default(),
            type_descriptor: type_descriptor.unwrap_or_default(),
        })
    }
}

#[derive(Debug, Default)]
pub(super) struct ConditionFlags(Vec<Flag>);

impl ConditionFlags {
    pub fn from_node(node: &Node, warnings: &mut WarningVec) -> Self {
        debug_assert_eq!(node.tag_name().name(), "conditionFlags");

        const TAG: &str = "flag";
        let mut flags = Vec::new();

        for child in node.children().filter(Node::is_element) {
            let tag = child.tag_name().name();
            if tag == TAG {
                flags.push(Flag::from_condition_flag_node(&child));
            } else {
                warnings.push(invalid_element(&child, TAG, tag));
            }
        }

        Self(flags)
    }
}

impl AsRef<[Flag]> for ConditionFlags {
    fn as_ref(&self) -> &[Flag] {
        self.0.as_slice()
    }
}

/// Expression that determines which [`PluginType`] to assign to a [`Plugin`].
#[derive(Debug, Default)]
pub(super) struct TypeDescriptor {
    pub default: PluginType,
    pub patterns: TypePatterns,
}

impl TypeDescriptor {
    pub fn from_node(node: &Node, warnings: &mut WarningVec) -> Result<Self> {
        const TAG: &str = "typeDescriptor";
        const EXPECTED: &str = "type or dependencyType";
        debug_assert_eq!(node.tag_name().name(), TAG);

        let mut children = node.children().filter(Node::is_element);
        let child = children.next().ok_or_else(|| missing_element(node, EXPECTED))?;
        children.for_each(|other| warnings.push(repeated_element(&other)));

        let mut default = None;
        let mut patterns = None;

        match child.tag_name().name() {
            "type" => default = Some(PluginType::from_node(&child)?),
            "dependencyType" => {
                for grandchild in child.children().filter(Node::is_element) {
                    let repeated = match grandchild.tag_name().name() {
                        "defaultType" => default.is_some_or_set_ok(|| PluginType::from_node(&grandchild))?,
                        "patterns" => patterns.is_some_or_set_ok(|| TypePatterns::from_node(&grandchild, warnings))?,
                        other => {
                            warnings.push(invalid_element(&grandchild, "defaultType or patterns", other));
                            false
                        }
                    };

                    if repeated {
                        warnings.push(repeated_element(&grandchild));
                    }
                }
            }
            other => return Err(invalid_element(&child, EXPECTED, other)),
        }

        Ok(Self {
            default: default.unwrap_or_default(),
            patterns: patterns.unwrap_or_default(),
        })
    }
}

/// Specifies if a plugin can, cannot, should, or must be selected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PluginType {
    /// Can be selected, but isn't by default.
    #[default]
    Optional,

    /// Selected by default.
    Recommended,

    /// Always selected.
    Required,

    /// Cannot be selected.
    NotUsable,
}

impl PluginType {
    fn from_node(node: &Node) -> Result<Self> {
        const ATTRIBUTE: &str = "name";

        match node.attribute(ATTRIBUTE) {
            Some("Optional" | "CouldBeUsable" | "") | None => Ok(Self::Optional),
            Some("Recommended") => Ok(Self::Recommended),
            Some("Required") => Ok(Self::Required),
            Some("NotUsable") => Ok(Self::NotUsable),
            Some(other) => Err(invalid_attribute_value(
                node,
                ATTRIBUTE,
                "Optional, Recommended, Required or NotUsable",
                other,
            )),
        }
    }
}

impl PartialOrd for PluginType {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PluginType {
    fn cmp(&self, other: &Self) -> Ordering {
        use PluginType::{NotUsable, Optional, Recommended, Required};

        #[allow(clippy::match_same_arms)]
        match (self, other) {
            (Optional, Optional) | (Recommended, Recommended) | (Required, Required) | (NotUsable, NotUsable) => {
                Ordering::Equal
            }
            (Required, _) => Ordering::Greater,
            (_, Required) => Ordering::Less,
            (Recommended, _) => Ordering::Greater,
            (_, Recommended) => Ordering::Less,
            (Optional, NotUsable) => Ordering::Greater,
            (NotUsable, Optional) => Ordering::Less,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct TypePatterns(Vec<TypePattern>);

impl TypePatterns {
    pub fn from_node(node: &Node, warnings: &mut WarningVec) -> Result<Self> {
        debug_assert_eq!(node.tag_name().name(), "patterns");

        parse_children_with_filter(node, warnings, TypePattern::from_node, "pattern").map(Self)
    }
}

impl AsRef<[TypePattern]> for TypePatterns {
    fn as_ref(&self) -> &[TypePattern] {
        self.0.as_slice()
    }
}

/// [`PluginType`] to be applied if the specified condition is met.
#[derive(Debug)]
pub(super) struct TypePattern {
    pub condition: DependencyBlock,
    pub then: PluginType,
}

impl TypePattern {
    pub fn from_node(node: &Node, warnings: &mut WarningVec) -> Result<Self> {
        debug_assert_eq!(node.tag_name().name(), "pattern");

        let mut condition = None;
        let mut then = None;

        for child in node.children().filter(Node::is_element) {
            let repeated = match child.tag_name().name() {
                "dependencies" => condition.is_some_or_set_ok(|| DependencyBlock::from_node(&child, warnings))?,
                "type" => then.is_some_or_set_ok(|| PluginType::from_node(&child))?,
                other => {
                    warnings.push(invalid_element(&child, "dependencies or type", other));
                    false
                }
            };

            if repeated {
                warnings.push(repeated_element(&child));
            }
        }

        let condition = condition.ok_or_else(|| missing_element(node, "dependencies"))?;
        let then = then.ok_or_else(|| missing_element(node, "type"))?;

        Ok(Self { condition, then })
    }
}

#[derive(Debug, Default)]
pub(super) struct ConditionalFileInstalls(Vec<ConditionalFileInstall>);

impl ConditionalFileInstalls {
    pub fn from_node(node: &Node, warnings: &mut WarningVec) -> Result<Self> {
        debug_assert_eq!(node.tag_name().name(), "conditionalFileInstalls");

        fn parse(node: &Node, warnings: &mut WarningVec) -> Result<Vec<ConditionalFileInstall>> {
            parse_children_with_filter(node, warnings, ConditionalFileInstall::from_node, "pattern")
        }

        parse_single_child_with(node, warnings, parse, "patterns")
            .and_then(|opt| opt.ok_or_else(|| missing_element(node, "patterns")))
            .map(Self)
    }
}

impl AsRef<[ConditionalFileInstall]> for ConditionalFileInstalls {
    fn as_ref(&self) -> &[ConditionalFileInstall] {
        self.0.as_slice()
    }
}

/// Set of [`InstallFile`]s to be installed if the specified condition is met.
#[derive(Debug)]
pub(super) struct ConditionalFileInstall {
    pub condition: DependencyBlock,
    pub files: InstallFiles,
}

impl ConditionalFileInstall {
    pub fn from_node(node: &Node, warnings: &mut WarningVec) -> Result<Self> {
        debug_assert_eq!(node.tag_name().name(), "pattern");

        let mut condition = None;
        let mut files = None;

        for child in node.children().filter(Node::is_element) {
            let repeated = match child.tag_name().name() {
                "dependencies" => condition.is_some_or_set_ok(|| DependencyBlock::from_node(&child, warnings))?,
                "files" => files.is_some_or_set_ok(|| InstallFiles::from_node(&child, warnings))?,
                other => {
                    warnings.push(invalid_element(&child, "dependencies or files", other));
                    false
                }
            };

            if repeated {
                warnings.push(repeated_element(&child));
            }
        }

        let condition = condition.ok_or_else(|| missing_element(node, "dependencies"))?;
        let files = files.ok_or_else(|| missing_element(node, "files"))?;

        Ok(Self { condition, files })
    }
}

/// Image contained in the FOMOD installer.
#[derive(Debug, Clone)]
pub struct Image {
    /// Path to the image from the FOMOD root.
    pub path: SharedStr,

    /// `NodeId` of the image in the archive.
    ///
    /// Always `None` after parsing, gets populated by [`resolve_images`](super::FomodInstaller::resolve_images).
    pub node: Option<NodeId>,
}

impl Image {
    const fn new(path: SharedStr) -> Self {
        Self { path, node: None }
    }

    pub fn from_node(node: &Node) -> Option<Self> {
        debug_assert_matches!(node.tag_name().name(), "image" | "moduleImage");
        get_attribute_path(node, "path").map(Self::new)
    }
}

fn parse_children_with<T>(
    node: &Node,
    warnings: &mut WarningVec,
    f: fn(&Node, &mut WarningVec) -> Result<T>,
) -> Result<Vec<T>> {
    let mut values = Vec::new();

    for child in node.children().filter(Node::is_element) {
        match f(&child, warnings) {
            Ok(file) => values.push(file),
            Err(
                err @ XmlError {
                    kind: ModuleConfigError::InvalidElementTag { .. }, ..
                },
            ) => warnings.push(err),
            Err(err) => return Err(err),
        }
    }

    Ok(values)
}

fn parse_children_with_filter<T>(
    node: &Node,
    warnings: &mut WarningVec,
    f: fn(&Node, &mut WarningVec) -> Result<T>,
    filter: &'static str,
) -> Result<Vec<T>> {
    let mut values = Vec::new();

    for child in node.children().filter(Node::is_element) {
        let tag = child.tag_name().name();
        if filter.contains(tag) {
            values.push(f(&child, warnings)?);
        } else {
            warnings.push(invalid_element(&child, filter, tag));
        }
    }

    Ok(values)
}

fn parse_single_child_with<T>(
    node: &Node,
    warnings: &mut WarningVec,
    f: fn(&Node, &mut WarningVec) -> Result<T>,
    tag_name: &'static str,
) -> Result<Option<T>> {
    let mut result = None;

    for child in node.children().filter(Node::is_element) {
        let tag = child.tag_name().name();
        if tag == tag_name {
            if result.is_none() {
                result = Some(f(&child, warnings)?);
            } else {
                warnings.push(XmlError::new(
                    &child,
                    ModuleConfigError::TooManyElements {
                        parent: SharedStr::new(node.tag_name().name()),
                        name: SharedStr::from_static_str(tag_name),
                    },
                ));
            }
        } else {
            warnings.push(invalid_element(&child, tag_name, tag));
        }
    }

    Ok(result)
}

fn get_attribute_bool(node: &Node, attribute_name: &'static str) -> Result<Option<bool>> {
    let Some(value) = node.attribute(attribute_name) else {
        return Ok(None);
    };

    match value {
        "true" | "1" => Ok(Some(true)),
        "false" | "0" => Ok(Some(false)),
        other => Err(invalid_attribute_value(node, attribute_name, "true or false", other)),
    }
}

fn get_attribute_i32(node: &Node, attribute_name: &'static str) -> Result<Option<i32>> {
    let Some(value) = node.attribute(attribute_name) else {
        return Ok(None);
    };

    if let Ok(num) = value.parse() {
        Ok(Some(num))
    } else {
        Err(invalid_attribute_value(node, attribute_name, "integer", value))
    }
}

fn get_attribute_path(node: &Node, attribute_name: &'static str) -> Option<SharedStr> {
    node.attribute(attribute_name)
        .filter(|s| !s.is_empty())
        .map(convert_path)
}

fn convert_path(path: &str) -> SharedStr {
    if path.contains('\\') {
        SharedStr::new(path.replace('\\', "/"))
    } else {
        SharedStr::new(path)
    }
}

/// Error type returned when parsing an XML document into a [`ModuleConfig`].
#[derive(Debug, Error)]
pub enum ModuleConfigError {
    #[error("invalid value in {name} attribute in {tag_name} element: expected {expected}, got '{actual}'")]
    InvalidAttributeValue {
        name: &'static str,
        tag_name: SharedStr,
        expected: &'static str,
        actual: SharedStr,
    },
    #[error("invalid element tag in {parent} element: expected {expected}, got '{actual}'")]
    InvalidElementTag { expected: &'static str, actual: SharedStr, parent: SharedStr },
    #[error("missing {name} attribute in {tag_name}")]
    MissingAttribute { name: &'static str, tag_name: SharedStr },
    #[error("missing {name} element in {parent}")]
    MissingElement { name: &'static str, parent: SharedStr },
    #[error("expected a single {name} element in {parent}, but found multiple (which will be ignored)")]
    TooManyElements { name: SharedStr, parent: SharedStr },
}

/// Error type returned when parsing bytes from a `ModuleConfig.xml` into a [`ModuleConfig`].
#[derive(Debug, Error)]
pub enum ModuleConfigFromBytesError {
    #[error("failed to parse document")]
    Document(#[from] McError),
    #[error("failed to decode XML")]
    Encoding(#[from] FromUtf8OrUtf16Error),
    #[error("failed to parse XML")]
    Xml(#[from] roxmltree::Error),
}

fn invalid_element(node: &Node, expected: &'static str, actual: &str) -> McError {
    XmlError::new(
        node,
        ModuleConfigError::InvalidElementTag {
            expected,
            actual: SharedStr::new(actual),
            parent: SharedStr::new(parent_tag(node)),
        },
    )
}

fn invalid_attribute_value(node: &Node, name: &'static str, expected: &'static str, value: &str) -> McError {
    XmlError::new(
        node,
        ModuleConfigError::InvalidAttributeValue {
            name,
            tag_name: SharedStr::new(node.tag_name().name()),
            expected,
            actual: SharedStr::new(value),
        },
    )
}

fn missing_attribute(node: &Node, name: &'static str) -> McError {
    XmlError::new(
        node,
        ModuleConfigError::MissingAttribute {
            name,
            tag_name: SharedStr::new(node.tag_name().name()),
        },
    )
}

fn missing_element(node: &Node, name: &'static str) -> McError {
    XmlError::new(
        node,
        ModuleConfigError::MissingElement { name, parent: SharedStr::new(parent_tag(node)) },
    )
}

fn repeated_element(node: &Node) -> McError {
    XmlError::new(
        node,
        ModuleConfigError::TooManyElements {
            name: SharedStr::new(node.tag_name().name()),
            parent: SharedStr::new(parent_tag(node)),
        },
    )
}

fn parent_tag<'a>(node: &'a Node) -> &'a str {
    node.parent().expect("has parent").tag_name().name()
}
