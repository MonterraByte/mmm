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

//! Provides a list of known instances for displaying and selecting in a UI

use std::fs::{self, File};
use std::io::{self, BufReader, ErrorKind, Write};
use std::iter;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::thread;

use cbor4ii::serde::DecodeError;
use foldhash::HashMap;
use thiserror::Error;
use tracing::{Level, debug, error, span, trace};

use mmm_core::instance::Instance;
use mmm_core::instance::data::{INSTANCE_DATA_FILE, InstanceDataOpenError, InstanceMetadata};

use crate::util::{ErrorChainDisplay, LockExt, move_multiple};
use crate::{APP_NAME, EditableInstance};

pub type InstancesShared = Arc<Mutex<Instances>>;
pub type Metadata = Result<InstanceMetadata, InstanceDataOpenError>;

/// Returns the path to the directory where instances are created by default.
#[must_use]
pub fn default_instances_dir() -> PathBuf {
    let mut path = dirs::data_local_dir().expect("system has local dir");
    path.push(APP_NAME);
    path.push("instances");
    path
}

fn instance_locations_path() -> PathBuf {
    let mut path = dirs::state_dir()
        .or_else(dirs::config_local_dir)
        .expect("system has config dir");
    path.push(APP_NAME);
    path.push("instance_locations.cbor");
    path
}

/// Contains the locations of known instances in this machine.
pub struct Instances {
    locations: Result<Locations, LocationsReadError>,
    metadata: HashMap<Arc<Path>, Metadata>,
    writer: Sender<Vec<u8>>,
}

type Locations = Vec<Arc<Path>>;

impl Instances {
    /// Loads instance locations from disk.
    pub fn get() -> InstancesShared {
        let locations_path = instance_locations_path();
        let locations = read_locations(&locations_path);
        if let Err(err) = &locations {
            error!("failed to load instance locations: {}", ErrorChainDisplay(err));
        }
        let fetch_metadata = locations.as_ref().is_ok_and(|l| !l.is_empty());

        let writer = spawn_locations_writer(locations_path);
        let instances = Arc::new(Mutex::new(Self { locations, metadata: HashMap::default(), writer }));

        if fetch_metadata {
            let instances = Arc::downgrade(&instances);
            thread::spawn(move || metadata_reader(instances));
        }

        instances
    }

    /// Returns an iterator over instance locations and their metadata.
    ///
    /// # Errors
    ///
    /// If reading instance locations failed, this method returns [`LocationsReadError`] instead.
    #[allow(clippy::iter_not_returning_iterator, reason = "yes it does")]
    pub fn iter(&self) -> Result<impl ExactSizeIterator<Item = (&Arc<Path>, Option<&Metadata>)>, &LocationsReadError> {
        let locations = self.locations.as_ref()?;

        let iter = locations.iter().map(|path| {
            let metadata = self.metadata.get(path);
            (path, metadata)
        });
        Ok(iter)
    }

    /// Adds an instance to the locations list, or moves it to the top if already present.
    ///
    /// If reading instance locations failed, this method creates a new list.
    pub fn add_or_touch(&mut self, instance: &EditableInstance) {
        let path = instance.dir();
        if let Ok(locations) = &mut self.locations {
            if let Some(idx) = locations.iter().position(|p| p.as_ref() == path) {
                move_multiple(locations, iter::once(idx), 0);
            } else {
                locations.insert(0, instance.arc_dir());
                self.metadata.insert(instance.arc_dir(), Ok(instance.metadata()));
            }
        } else {
            self.locations = Ok(vec![instance.arc_dir()]);
            self.metadata.insert(instance.arc_dir(), Ok(instance.metadata()));
        }

        self.save();
    }

    /// Removes the specified instance path from the list.
    pub fn remove(&mut self, path: &Path) {
        if let Ok(locations) = &mut self.locations {
            locations.retain(|p| p.as_ref() != path);
            self.save();
        }
        self.metadata.remove(path);
    }

    fn save(&self) {
        if let Ok(locations) = &self.locations {
            let contents = cbor4ii::serde::to_vec(Vec::new(), locations).unwrap();
            if self.writer.send(contents).is_err() {
                error!("couldn't write instance locations, writer thread panicked");
            }
        }
    }
}

fn read_locations(path: &Path) -> Result<Locations, LocationsReadError> {
    let mut file = match File::open(path) {
        Ok(file) => BufReader::new(file),
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Locations::default()),
        Err(source) => return Err(LocationsReadError::Open { source, path: Box::from(path) }),
    };

    cbor4ii::serde::from_reader(&mut file)
        .map_err(|source| LocationsReadError::Deserialize { source, path: Box::from(path) })
}

/// Error type returned by [`Instances::iter`].
#[derive(Debug, Error)]
pub enum LocationsReadError {
    #[error("failed to deserialize instance location data in '{path}'")]
    Deserialize { source: DecodeError<io::Error>, path: Box<Path> },
    #[error("failed to open instance location file '{path}'")]
    Open { source: io::Error, path: Box<Path> },
}

fn spawn_locations_writer(path: PathBuf) -> Sender<Vec<u8>> {
    let (sender, receiver) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let tmp_path = path.with_added_extension("tmp");
        let _span =
            span!(Level::TRACE, "locations writer", path = %path.display(), tmp_path = %tmp_path.display()).entered();

        if let Err(err) = fs::create_dir_all(path.parent().unwrap()) {
            error!("failed to create directory: {}", err);
        }

        while let Ok(content) = receiver.recv() {
            let mut file = match File::create(&tmp_path) {
                Ok(file) => file,
                Err(err) => {
                    error!("failed to create file: {}", err);
                    continue;
                }
            };

            if let Err(err) = file.write_all(&content) {
                error!("failed to write data to file: {}", err);
                continue;
            }

            if let Err(err) = file.sync_data() {
                error!("failed to sync file to disk: {}", err);
                continue;
            }

            drop(file);

            if let Err(err) = fs::rename(&tmp_path, &path) {
                error!("failed to rename temp file over target file: {}", err);
            }
        }

        trace!("instance locations writer thread quitting");
    });

    sender
}

#[expect(clippy::needless_pass_by_value, reason = "thread needs to own the reference")]
fn metadata_reader(instances: Weak<Mutex<Instances>>) {
    let mut data_path = PathBuf::new();
    let mut instance_path = None;
    let mut metadata = None;
    while let Some(instances) = Weak::upgrade(&instances) {
        {
            let mut instances = instances.lock_expect();

            if let Some(metadata) = metadata {
                let path = instance_path.expect("was set earlier");
                let value = instances.metadata.insert(path, metadata);
                assert!(value.is_none());
            }

            if let Some(next) = instances
                .locations
                .as_ref()
                .expect("config is OK")
                .iter()
                .find(|p| !instances.metadata.contains_key(*p))
            {
                instance_path = Some(Arc::clone(next));
            } else {
                break;
            }
        }
        let instance_path = instance_path.as_deref().expect("was set earlier");

        data_path.clear();
        data_path.push(instance_path);
        data_path.push(INSTANCE_DATA_FILE);

        let result = InstanceMetadata::from_file(&data_path);
        if let Err(err) = &result {
            error!(
                "failed to get metadata from instance data file '{}': {}",
                data_path.display(),
                ErrorChainDisplay(err),
            );
        } else {
            debug!("read metadata of instance '{}'", instance_path.display());
        }

        metadata = Some(result);
    }

    trace!("finished reading metadata of known instances");
}
