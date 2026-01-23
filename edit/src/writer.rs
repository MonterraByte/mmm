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

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread;

use tracing::Level;
use tracing::{error, span};

use mmm_core::instance::data::INSTANCE_DATA_FILE;

#[derive(Debug)]
pub struct WriteRequest {
    pub content: Vec<u8>,
    pub target: WriteTarget,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum WriteTarget {
    InstanceData,
}

pub fn spawn_writer_thread(instance_dir: &Path) -> Result<Sender<WriteRequest>, io::Error> {
    let (sender, receiver) = mpsc::channel::<WriteRequest>();
    let paths = FilePaths::from_dir(instance_dir);

    thread::Builder::new().name("writer".to_owned()).spawn(move || {
        while let Ok(req) = receiver.recv() {
            let (path, tmp_path) = paths.path_of_target(req.target);
            let _span = span!(Level::TRACE, "writer", path = %path.display(), tmp_path = %tmp_path.display()).entered();

            let mut file = match File::create(tmp_path) {
                Ok(file) => file,
                Err(err) => {
                    error!("failed to create file: {}", err);
                    continue;
                }
            };

            if let Err(err) = file.write_all(&req.content) {
                error!("failed to write data to file: {}", err);
                continue;
            }

            if let Err(err) = file.sync_data() {
                error!("failed to sync file to disk: {}", err);
                continue;
            }

            drop(file);

            if let Err(err) = fs::rename(tmp_path, path) {
                error!("failed to rename temp file over target file: {}", err);
            }
        }
    })?;

    Ok(sender)
}

struct FilePaths {
    data_file: PathBuf,
    data_file_tmp: PathBuf,
}

impl FilePaths {
    fn from_dir(instance_dir: &Path) -> Self {
        let data_file = instance_dir.join(INSTANCE_DATA_FILE);
        let data_file_tmp = data_file.with_added_extension("tmp");
        Self { data_file, data_file_tmp }
    }

    fn path_of_target(&self, target: WriteTarget) -> (&Path, &Path) {
        match target {
            WriteTarget::InstanceData => (&self.data_file, &self.data_file_tmp),
        }
    }
}
