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

//! Game identification and location data

use std::path::{Path, PathBuf};

use compact_str::CompactString;
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

/// Information about a game and where it comes from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Game {
    #[serde(deserialize_with = "deserialize_absolute_path")]
    path: PathBuf,
    provider: Option<Provider>,
}

impl Game {
    /// Creates a new `Game` from the provided information.
    ///
    /// # Errors
    ///
    /// Fails if `path` is not absolute.
    pub fn new(path: PathBuf, provider: Option<Provider>) -> Result<Self, PathIsNotAbsoluteError> {
        if path.is_relative() {
            return Err(PathIsNotAbsoluteError);
        }
        Ok(Self { path, provider })
    }

    /// Returns the absolute path to the game directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the provider that manages the game's installation, if any.
    #[must_use]
    pub fn provider(&self) -> Option<&Provider> {
        self.provider.as_ref()
    }
}

/// Error type returned by [`Game::new`].
#[derive(Debug, Copy, Clone, Error)]
#[error("the provided path is not absolute")]
pub struct PathIsNotAbsoluteError;

/// The thing that manages the installation of a game (game store, launcher, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Provider {
    Steam(SteamGame),
}

/// Data about a Steam game.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteamGame {
    /// The full name of the game.
    pub name: CompactString,
    /// The name of the game's installation directory.
    pub install_dir: CompactString,
    /// The ID that identifies the game on Steam.
    pub app_id: u32,
}

fn deserialize_absolute_path<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
where
    D: Deserializer<'de>,
{
    let path = PathBuf::deserialize(deserializer)?;
    if path.is_relative() {
        use serde::de::Error;
        return Err(D::Error::custom(format_args!(
            "invalid value: {}, expected an absolute path",
            path.display()
        )));
    }
    Ok(path)
}
