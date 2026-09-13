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

//! Interfaces for discovering installed games.

use std::path::{Path, PathBuf};

use compact_str::CompactString;
use tracing::error;

use mmm_core::game::SteamGame;

use crate::util::{ErrorChainDisplay, str_ord};

/// Steam games installed on the machine.
pub struct Steam {
    libraries: Vec<Box<Path>>,
    games: Vec<(SteamGame, usize)>,
}

impl Steam {
    /// Reads the installed games from the Steam libraries on the machine.
    pub fn get() -> Self {
        let mut steam = Self { libraries: Vec::new(), games: Vec::new() };

        let steam_dir = match steamlocate::locate() {
            Ok(s) => s,
            Err(err) => {
                error!("failed to locate Steam: {}", ErrorChainDisplay(&err));
                return steam;
            }
        };

        let library_iter = match steam_dir.libraries() {
            Ok(iter) => iter,
            Err(err) => {
                error!("failed to get Steam library paths: {}", ErrorChainDisplay(&err));
                return steam;
            }
        };

        for library in library_iter {
            match library {
                Ok(library) => {
                    steam.libraries.push(Box::from(library.path()));
                    let library_idx = steam
                        .libraries
                        .len()
                        .checked_sub(1)
                        .expect("there's at least one element");

                    for app in library.apps() {
                        let app = match app {
                            Ok(app) => app,
                            Err(err) => {
                                error!("failed to get Steam app information: {}", ErrorChainDisplay(&err));
                                continue;
                            }
                        };

                        // TODO: skip tools

                        let name = if let Some(name) = app.name {
                            CompactString::from(name)
                        } else {
                            CompactString::from(&app.install_dir)
                        };

                        steam.games.push((
                            SteamGame {
                                name,
                                install_dir: CompactString::from(app.install_dir),
                                app_id: app.app_id,
                            },
                            library_idx,
                        ));
                    }
                }
                Err(err) => error!("failed to open Steam library: {}", ErrorChainDisplay(&err)),
            }
        }
        steam.games.sort_by(|left, right| str_ord(&left.0.name, &right.0.name));

        steam
    }

    /// Returns `true` if a Steam library with games was found.
    #[must_use]
    pub fn found(&self) -> bool {
        !self.games.is_empty()
    }

    /// Returns an iterator over the discovered games.
    pub fn games(&self) -> impl ExactSizeIterator<Item = InstalledSteamGame<'_>> {
        self.games.iter().map(|(data, library_idx)| InstalledSteamGame {
            data,
            library: self.libraries[*library_idx].as_ref(),
        })
    }

    /// Returns the data for the game with the specified index (from the `Steam::games` iterator).
    #[must_use]
    pub fn game(&self, idx: usize) -> &SteamGame {
        &self.games[idx].0
    }
}

/// An installed Steam game.
pub struct InstalledSteamGame<'a> {
    data: &'a SteamGame,
    library: &'a Path,
}

impl InstalledSteamGame<'_> {
    /// Returns the data for this game.
    #[must_use]
    pub fn data(&self) -> &SteamGame {
        self.data
    }

    /// Writes the path to the game installation into the provided `PathBuf`.
    pub fn push_path(&self, path: &mut PathBuf) {
        path.push(self.library);
        path.push("steamapps");
        path.push("common");
        path.push(self.data.install_dir.as_str());
    }
}
