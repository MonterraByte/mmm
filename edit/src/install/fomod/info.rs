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

//! `info.xml` parser

use std::num::ParseIntError;

use roxmltree::{Document, Node};
use smallvec::SmallVec;
use thiserror::Error;

use super::{OptionExt, XmlError, get_attribute_str, get_text_str};
use crate::util::{FromUtf8OrUtf16Error, SharedStr, utf8_or_utf16_bytes_to_string};

pub type IError = XmlError<InfoError>;
type WarningVec = Vec<IError>;

/// Information about the contained mod.
#[derive(Debug, Default)]
pub struct Info {
    /// The name of the mod.
    pub name: Option<SharedStr>,

    /// The version of the mod.
    pub version: Version,

    /// The ID of the mod, presumably on NexusMods.
    pub id: Option<u32>,

    /// The name of the mod author.
    pub author: Option<SharedStr>,

    /// URL of the mod's website or web page.
    pub website: Option<SharedStr>,

    /// A list of categories that describe the contents of the mod.
    ///
    /// Observation shows that the vast majority of mods have either zero or one categories.
    pub groups: SmallVec<[SharedStr; 1]>,

    /// A description of the mod.
    pub description: Option<SharedStr>,
}

/// The version of a mod.
#[derive(Debug, Default)]
pub struct Version {
    /// A human-readable representation of the version.
    pub version: Option<SharedStr>,

    /// A machine-readable representation of the version.
    ///
    /// It is unspecified. Hopefully it's SemVer!
    pub machine_version: Option<SharedStr>,
}

impl Info {
    /// Parses bytes read from an `info.xml` into an instance of `Info`.
    ///
    /// Additionally, it also returns a vector of warnings emitted during parsing.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<(Self, WarningVec), InfoFromBytesError> {
        let str = utf8_or_utf16_bytes_to_string(bytes)?;
        let doc = Document::parse(&str)?;
        Self::from_doc(&doc).map_err(Into::into)
    }

    /// Parses the specified XML document into an instance of `Info`.
    ///
    /// Additionally, it also returns a vector of warnings emitted during parsing.
    pub fn from_doc(document: &Document) -> Result<(Self, WarningVec), IError> {
        let mut info = Self::default();
        let mut warnings = Vec::new();

        let fomod_node = document
            .root()
            .children()
            .find(|node| node.is_element() && node.tag_name().name() == "fomod")
            .ok_or_else(|| XmlError::new(&document.root(), InfoError::MissingFomodElement))?;

        for child in fomod_node.children().filter(Node::is_element) {
            let repeated = match child.tag_name().name() {
                "Name" => info.name.is_some_or_set_opt(|| get_text_str(&child)),
                "Version" => {
                    let mut repeated = false;

                    let v = get_text_str(&child);
                    if v.is_some() {
                        repeated |= info.version.version.is_some();
                        if info.version.version.is_none() {
                            info.version.version = v;
                        }
                    }

                    let mv = get_attribute_str(&child, "MachineVersion");
                    if mv.is_some() {
                        repeated |= info.version.machine_version.is_some();
                        if info.version.machine_version.is_none() {
                            info.version.machine_version = mv;
                        }
                    }

                    repeated
                }
                "Id" => {
                    if info.id.is_none() {
                        if let Some(text) = get_text_str(&child) {
                            match text.parse() {
                                Ok(num) => info.id = Some(num),
                                Err(err) => warnings.push(XmlError::new(&child, InfoError::InvalidId(err))),
                            }
                        }
                        false
                    } else {
                        true
                    }
                }
                "Author" => info.author.is_some_or_set_opt(|| get_text_str(&child)),
                "Website" => info.website.is_some_or_set_opt(|| get_text_str(&child)),
                "Groups" => {
                    for grandchild in child.children().filter(Node::is_element) {
                        if !grandchild.tag_name().name().eq_ignore_ascii_case("element") {
                            warnings.push(XmlError::new(
                                &grandchild,
                                InfoError::UnknownGroupsElement(SharedStr::new(grandchild.tag_name().name())),
                            ));
                            continue;
                        }

                        if let Some(category) = get_text_str(&grandchild)
                            && !info.groups.contains(&category)
                        {
                            info.groups.push(category);
                        }
                    }
                    false
                }
                "Description" => info.description.is_some_or_set_opt(|| get_text_str(&child)),
                other => {
                    warnings.push(XmlError::new(&child, InfoError::UnknownElement(SharedStr::new(other))));
                    false
                }
            };

            if repeated {
                warnings.push(XmlError::new(
                    &child,
                    InfoError::RepeatedElement(SharedStr::new(child.tag_name().name())),
                ));
            }
        }

        Ok((info, warnings))
    }
}

/// Error type returned when parsing an XML document into an [`Info`].
#[derive(Debug, Error)]
pub enum InfoError {
    #[error("missing fomod element")]
    MissingFomodElement,
    #[error("invalid mod ID: {0}")]
    InvalidId(ParseIntError),
    #[error("repeated instance of {0} element (which will be ignored)")]
    RepeatedElement(SharedStr),
    #[error("invalid element tag {0}, expected Name, Version, Id, Author, Website, Groups or Description")]
    UnknownElement(SharedStr),
    #[error("invalid element tag {0}, expected element")]
    UnknownGroupsElement(SharedStr),
}

/// Error type returned when parsing bytes from an `info.xml` into an [`Info`].
#[derive(Debug, Error)]
pub enum InfoFromBytesError {
    #[error("failed to parse document")]
    Document(#[from] IError),
    #[error("failed to decode XML")]
    Encoding(#[from] FromUtf8OrUtf16Error),
    #[error("failed to parse XML")]
    Xml(#[from] roxmltree::Error),
}
