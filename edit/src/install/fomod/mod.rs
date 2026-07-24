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

use foldhash::{HashMap, HashSet};
use nary_tree::NodeId;
use roxmltree::{Node, TextPos};
use tracing::{error, warn};

use mmm_core::file_tree::TreeNodeKind;
use mmm_core::file_tree::util::OptionExt as _;

use self::info::{IError, Info, InfoFromBytesError};
use self::module_config::{
    Dependency, DependencyBlock, FileGroup, FileGroupType, FlagName, FlagValue, GroupName, InstallFileKind,
    InstallStep, McError, ModuleConfig, ModuleConfigFromBytesError, Operator, Plugin, PluginName, PluginType,
};
use crate::EditableInstance;
use crate::archive::{Archive, ExtractSelection, FileReadMap};
use crate::util::{
    SharedStr, find_child_with_case_insensitive_name, find_entry_in_nested_outer_directories,
    find_node_by_case_insensitive_path, node_ord,
};

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

    /// Takes the installer file from the set of files read from the archive and parses it.
    pub fn get_installer(
        &self,
        map: &mut FileReadMap,
    ) -> Option<Result<(FomodInstaller, Vec<McError>), ModuleConfigFromBytesError>> {
        let data = map.remove(&self.module_config?)?;
        Some(ModuleConfig::from_bytes(data).map(|(mc, warnings)| (FomodInstaller::new(mc, self.root), warnings)))
    }
}

/// FOMOD installer.
#[derive(Debug)]
pub struct FomodInstaller {
    module_config: ModuleConfig,
    root: NodeId,

    install_step_states: Box<[InstallStepState]>,
    current_step: InstallerState,

    allow_disabling_required_plugins: bool,
    allow_unusable_plugins: bool,
}

impl FomodInstaller {
    fn new(module_config: ModuleConfig, root: NodeId) -> Self {
        let install_steps = module_config.install_steps.as_ref().len();

        Self {
            module_config,
            root,
            install_step_states: vec![InstallStepState::default(); install_steps].into_boxed_slice(),
            current_step: InstallerState::Uninitialized,
            allow_disabling_required_plugins: false,
            allow_unusable_plugins: false,
        }
    }

    #[must_use]
    pub(super) fn mod_name(&self) -> Option<SharedStr> {
        self.module_config.name.0.clone()
    }

    /// Returns the data of the current step of the installer.
    #[must_use]
    pub fn current_step(&self) -> Option<(&InstallStep, &InstallStepState)> {
        let InstallerState::Step(idx) = self.current_step else {
            return None;
        };

        Some((
            &self.module_config.install_steps.as_ref()[idx],
            &self.install_step_states[idx],
        ))
    }

    /// Prepares the current step's state for usage, by carrying forward flags and calculating selection constraints.
    ///
    /// Returns whether the current step is visible or not.
    /// (Invisible steps are skipped and do not apply flags or install files.)
    fn prepare_current_step(&mut self, instance: &EditableInstance) -> Visibility {
        let InstallerState::Step(step_idx) = self.current_step else {
            panic!("can't call prepare_current_step if state isn't InstallerState::Step");
        };

        self.install_step_states[step_idx].clear();
        if let Some(prev_idx) = step_idx.checked_sub(1) {
            let [prev_state, current_state] = self
                .install_step_states
                .get_disjoint_mut([prev_idx, step_idx])
                .expect("indices are valid");
            current_state
                .flags
                .extend(prev_state.flags.iter().map(|(k, v)| (k.clone(), v.clone())));
        }

        let step = &mut self.module_config.install_steps.as_mut()[step_idx];
        let step_state = &mut self.install_step_states[step_idx];

        // Selection constraints need to be calculated even for invisible steps, for `installIfUsable`.
        for group in step.file_groups.as_mut() {
            for plugin in group.plugins.as_ref() {
                let mut constraint = plugin.type_descriptor.default;
                for pattern in plugin.type_descriptor.patterns.as_ref() {
                    if evaluate_block(instance, &step_state.flags, &pattern.condition) {
                        constraint = pattern.then;
                    }
                }
                step_state
                    .selection_constraints
                    .insert((group.name.clone(), plugin.name.clone()), constraint);
            }
        }

        let visible = evaluate_block(instance, &step_state.flags, &step.visible);
        if !visible {
            return Visibility::Invisible;
        }

        for group in step.file_groups.as_mut() {
            let selection = step_state.selections.entry(group.name.clone()).or_default();
            selection.extend(
                group
                    .plugins
                    .as_ref()
                    .iter()
                    .filter(|plugin| {
                        *step_state
                            .selection_constraints
                            .get(&(group.name.clone(), plugin.name.clone()))
                            .expect("selection constrains were already set for each plugin")
                            == PluginType::Required
                    })
                    .map(|plugin| plugin.name.clone()),
            );

            match group.ty {
                FileGroupType::SelectAtLeastOne | FileGroupType::SelectExactlyOne => {
                    match (group.ty, selection.len()) {
                        (FileGroupType::SelectAtLeastOne | FileGroupType::SelectExactlyOne, 0) => {}
                        (FileGroupType::SelectExactlyOne, 1) | (FileGroupType::SelectAtLeastOne, _) => continue,
                        (FileGroupType::SelectExactlyOne, _) => selection.clear(),
                        _ => unreachable!(),
                    }

                    let mut selected = None;
                    let mut selected_type = PluginType::NotUsable;
                    for plugin in group.plugins.as_ref() {
                        let ty = *step_state
                            .selection_constraints
                            .get(&(group.name.clone(), plugin.name.clone()))
                            .expect("selection constrains were already set for each plugin");
                        if selected.is_none() || ty > selected_type {
                            selected = Some(plugin.name.clone());
                            selected_type = ty;
                        }
                    }

                    if selected_type == PluginType::NotUsable && !self.allow_unusable_plugins {
                        selected = None;
                    }

                    if let Some(value) = selected {
                        selection.insert(value);
                    }
                }
                FileGroupType::SelectAll => {
                    selection.extend(
                        group
                            .plugins
                            .as_ref()
                            .iter()
                            .filter(|p| {
                                self.allow_unusable_plugins
                                    || *step_state
                                        .selection_constraints
                                        .get(&(group.name.clone(), p.name.clone()))
                                        .expect("selection constrains were already set for each plugin")
                                        != PluginType::NotUsable
                            })
                            .map(|p| p.name.clone()),
                    );
                }
                FileGroupType::SelectAtMostOne | FileGroupType::SelectAny => (),
            }
        }

        Visibility::Visible
    }

    /// Toggles the specified plugin from the specified group.
    ///
    /// This function may change the state of other plugins in the group, or do nothing at all,
    /// depending on group type and plugin constraints.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "SharedStr is reference-counted, so it doesn't matter"
    )]
    pub fn toggle(&mut self, group: GroupName, plugin: PluginName) {
        let InstallerState::Step(idx) = self.current_step else {
            panic!("can't call toggle outside of install steps");
        };
        let step = &self.module_config.install_steps.as_ref()[idx];
        let step_state = &mut self.install_step_states[idx];
        let constraint = *step_state
            .selection_constraints
            .get(&(group.clone(), plugin.clone()))
            .expect("selection constrains were already set for each plugin");

        let group_type = step
            .file_groups
            .as_ref()
            .iter()
            .find(|g| g.name == group)
            .expect("group exists")
            .ty;
        let selected = step_state
            .selections
            .get_mut(&group)
            .expect("step is visible, so it has a selection hashset");

        if selected.contains(&plugin) {
            if ((selected.len() > 1 || group_type.allows_none())
                && (constraint != PluginType::Required || self.allow_disabling_required_plugins)
                && (group_type != FileGroupType::SelectAll))
                || constraint == PluginType::NotUsable
            {
                selected.remove(&plugin);
            }
        } else {
            if constraint == PluginType::NotUsable && !self.allow_unusable_plugins {
                return;
            }

            if !group_type.allows_multiple() {
                selected.retain(|s| {
                    *step_state
                        .selection_constraints
                        .get(&(group.clone(), s.clone()))
                        .expect("selection constrains were already set for each plugin")
                        == PluginType::Required
                });

                if !selected.is_empty() {
                    return;
                }
            }

            selected.insert(plugin);
        }
    }

    /// Returns `true` if the specified plugin's state cannot be changed.
    #[must_use]
    pub fn can_never_be_toggled(&self, group: &FileGroup, plugin: &Plugin, selected: bool) -> bool {
        let InstallerState::Step(idx) = self.current_step else {
            panic!("can't call can_never_be_toggled outside of install steps");
        };

        let state = &self.install_step_states[idx];
        let constraint = *state
            .selection_constraints
            .get(&(group.name.clone(), plugin.name.clone()))
            .expect("selection constrains were already set for each plugin");

        (group.ty == FileGroupType::SelectAll && selected && constraint != PluginType::NotUsable)
            || (constraint == PluginType::NotUsable && !selected && !self.allow_unusable_plugins)
            || (constraint == PluginType::Required && selected && !self.allow_disabling_required_plugins)
    }

    /// Advances to the next install step, or returns the resulting `ExtractSelection` if it has reached the end.
    pub fn next(
        &mut self,
        archive: &Archive,
        instance: &EditableInstance,
    ) -> Result<Option<ExtractSelection>, Box<str>> {
        let mut invisible = false;
        loop {
            match self.current_step {
                InstallerState::Uninitialized => {
                    let requirements_met =
                        evaluate_block(instance, &HashMap::default(), &self.module_config.dependencies);
                    if !requirements_met {
                        break Err(format!(
                            "This mod cannot be installed because the following requirements are not met:\n{:#?}",
                            self.module_config.dependencies,
                        )
                        .into_boxed_str());
                    }

                    if self.module_config.install_steps.as_ref().is_empty() {
                        self.current_step = InstallerState::Finished;
                        continue;
                    }
                    self.current_step = InstallerState::Step(0);
                }
                InstallerState::Step(idx) => {
                    if !invisible {
                        if !self.can_go_forward() {
                            error!("cannot go forward");
                            break Ok(None);
                        }

                        let step = &self.module_config.install_steps.as_mut()[idx];
                        let step_state = &mut self.install_step_states[idx];
                        Self::apply_flags(step, step_state);
                    }

                    if idx + 1 >= self.module_config.install_steps.as_ref().len() {
                        self.current_step = InstallerState::Finished;
                        continue;
                    }

                    self.current_step = InstallerState::Step(idx + 1);
                }
                InstallerState::Finished => break Ok(Some(self.finalize(archive, instance)?)),
            }

            match self.prepare_current_step(instance) {
                Visibility::Visible => break Ok(None),
                Visibility::Invisible => invisible = true,
            }
        }
    }

    /// Checks if the requirements for each group of the current step are met, in order to move forward.
    ///
    /// See [`next`](Self::next).
    #[must_use]
    pub fn can_go_forward(&self) -> bool {
        let InstallerState::Step(step_idx) = self.current_step else {
            panic!("can't call can_go_forward if state isn't InstallerState::Step");
        };

        let step = &self.module_config.install_steps.as_ref()[step_idx];
        let step_state = &self.install_step_states[step_idx];

        for group in step.file_groups.as_ref() {
            let total = group.plugins.as_ref().len();
            if total == 0 {
                continue;
            }

            let (required, not_usable) = {
                let mut required: usize = 0;
                let mut not_usable: usize = 0;
                for plugin in group.plugins.as_ref() {
                    let ty = *step_state
                        .selection_constraints
                        .get(&(group.name.clone(), plugin.name.clone()))
                        .expect("selection constrains were already set for each plugin");
                    match ty {
                        PluginType::Required => required += 1,
                        PluginType::NotUsable => not_usable += 1,
                        PluginType::Optional | PluginType::Recommended => {}
                    }
                }
                (required, not_usable)
            };

            let selection = step_state
                .selections
                .get(&group.name)
                .expect("step is visible, so it has a selection hashset");
            let selected = selection.len();

            let valid = match group.ty {
                FileGroupType::SelectAny => true,
                FileGroupType::SelectAll => {
                    selected
                        >= total
                            .checked_sub(not_usable)
                            .expect("total is always greater or equal to not_usable")
                }
                FileGroupType::SelectAtLeastOne => selected > 0 || not_usable == total,
                FileGroupType::SelectAtMostOne => selected <= 1 || selected <= required,
                FileGroupType::SelectExactlyOne => match selected {
                    0 => not_usable == total,
                    1 => true,
                    more => more <= required,
                },
            };

            if !valid {
                return false;
            }
        }

        true
    }

    /// Sets the flags specified by each selected plugin in the step.
    fn apply_flags(step: &InstallStep, step_state: &mut InstallStepState) {
        for group in step.file_groups.as_ref() {
            let Some(selected) = step_state.selections.get(&group.name) else {
                continue;
            };

            for plugin in group.plugins.as_ref() {
                if selected.contains(&plugin.name) {
                    for flag in plugin.condition_flags.as_ref() {
                        step_state.flags.insert(flag.name.clone(), flag.value.clone());
                    }
                }
            }
        }
    }

    /// Moves back a step in the installer, if possible.
    pub fn back(&mut self, instance: &EditableInstance) {
        if let Some(idx) = self.previous_visible_step(instance) {
            self.current_step = InstallerState::Step(idx);
        }
    }

    /// Checks if there's a previous installer step.
    ///
    /// See [`back`](Self::back).
    #[must_use]
    pub fn can_go_back(&self, instance: &EditableInstance) -> bool {
        self.previous_visible_step(instance).is_some()
    }

    fn previous_visible_step(&self, instance: &EditableInstance) -> Option<usize> {
        let mut idx = match self.current_step {
            InstallerState::Uninitialized => return None,
            InstallerState::Step(idx) => idx,
            InstallerState::Finished => self.module_config.install_steps.as_ref().len(),
        };

        while let Some(sub) = idx.checked_sub(1) {
            idx = sub;

            let step = &self.module_config.install_steps.as_ref()[idx];
            let step_state = &self.install_step_states[idx];

            let visible = evaluate_block(instance, &step_state.flags, &step.visible);
            if visible {
                return Some(idx);
            }
        }

        None
    }

    /// Determines which files should be installed, given the various conditions and selected options.
    fn finalize(&mut self, archive: &Archive, instance: &EditableInstance) -> Result<ExtractSelection, Box<str>> {
        let empty_map = HashMap::default();
        let flags = self.install_step_states.last().map_or(&empty_map, |s| &s.flags);

        let mut install_queue = Vec::new();

        for file in self.module_config.required_install_files.as_ref() {
            install_queue.push(file);
        }

        for (idx, step) in self.module_config.install_steps.as_ref().iter().enumerate() {
            let step_state = &self.install_step_states[idx];
            for group in step.file_groups.as_ref() {
                let selection = step_state.selections.get(&group.name);
                for plugin in group.plugins.as_ref() {
                    let usable = *step_state
                        .selection_constraints
                        .get(&(group.name.clone(), plugin.name.clone()))
                        .expect("selection constrains were already set for each plugin")
                        != PluginType::NotUsable;
                    let selected = selection.is_some_and(|sel| sel.contains(&plugin.name));

                    for file in plugin.files.as_ref() {
                        if file.always_install || (file.install_if_usable && usable) || selected {
                            install_queue.push(file);
                        }
                    }
                }
            }
        }

        for entry in self.module_config.conditional_file_installs.as_ref() {
            if evaluate_block(instance, flags, &entry.condition) {
                for file in entry.files.as_ref() {
                    install_queue.push(file);
                }
            }
        }

        install_queue.sort_by_key(|f| f.priority);

        let mut extract_selection = ExtractSelection::new();
        for file in install_queue {
            let Some(node) = find_node_by_case_insensitive_path(archive.tree(), self.root, file.source()) else {
                return Err(format!("file '{}' not found in archive", file.source()).into_boxed_str());
            };

            match (node.data().kind, file.kind) {
                (TreeNodeKind::Dir, InstallFileKind::File) => {
                    warn!(
                        "FOMOD expects to install a file, but the archive contains a directory at that location; treating it as a directory"
                    );
                }
                (TreeNodeKind::File(()), InstallFileKind::Folder) => {
                    warn!(
                        "FOMOD expects to install a directory, but the archive contains a file at that location; treating it as a file"
                    );
                }
                _ => (),
            }

            extract_selection
                .add_to_selection(archive, &node.node_id(), file.destination())
                .map_err(|err| {
                    format!(
                        "couldn't select file for installation:\n\t{}\n\tsource: {}\n\tdestination: {}",
                        err,
                        file.source(),
                        file.destination(),
                    )
                    .into_boxed_str()
                })?;
        }

        extract_selection
            .tree()
            .root_mut()
            .expect("has root node")
            .sort_recursive_by(node_ord);
        Ok(extract_selection)
    }
}

fn evaluate_block(instance: &EditableInstance, flags: &Flags, block: &DependencyBlock) -> bool {
    let mut iter = block.deps.iter();
    let eval = |dep| evaluate(instance, flags, dep);
    match block.operator {
        Operator::And => iter.all(eval),
        Operator::Or => iter.any(eval),
    }
}

fn evaluate(instance: &EditableInstance, flags: &Flags, dependency: &Dependency) -> bool {
    match dependency {
        Dependency::Dependencies(block) => evaluate_block(instance, flags, block),
        Dependency::File(_file_dependency) => {
            warn!("file dependency not implemented");
            true
        }
        Dependency::Flag(flag) => flags.get(&flag.name).is_some_and(|value| value == &flag.value),
        Dependency::Game(_version) => {
            warn!("game dependency not implemented");
            true
        }
        Dependency::Fomm => true,
    }
}

#[derive(Copy, Clone, Debug)]
enum InstallerState {
    Uninitialized,
    Step(usize),
    Finished,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Visibility {
    Visible,
    Invisible,
}

/// The state of the current installer step.
#[derive(Clone, Debug, Default)]
pub struct InstallStepState {
    flags: Flags,
    selections: HashMap<GroupName, HashSet<PluginName>>,
    selection_constraints: HashMap<(GroupName, PluginName), PluginType>,
}

type Flags = HashMap<FlagName, FlagValue>;

impl InstallStepState {
    fn clear(&mut self) {
        self.flags.clear();
        self.selection_constraints.clear();
    }

    /// Returns the set of currently selected plugins from the specified group.
    #[must_use]
    pub fn selection(&self, group: &GroupName) -> Option<&HashSet<PluginName>> {
        self.selections.get(group)
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
