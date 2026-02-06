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

use std::io;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

pub type StatusString = Arc<Mutex<String>>;
pub type BackgroundTask = Box<dyn FnOnce(&StatusString) + Send>;

pub fn spawn_background_thread() -> Result<(Sender<BackgroundTask>, StatusString), io::Error> {
    let (sender, receiver) = mpsc::channel::<BackgroundTask>();
    let status = Arc::new(Mutex::new(String::new()));
    let status_clone = Arc::clone(&status);

    thread::Builder::new().name("background".to_owned()).spawn(move || {
        while let Ok(req) = receiver.recv() {
            req(&status);
            status.lock().expect("lock is not poisoned").clear();
        }
    })?;

    Ok((sender, status_clone))
}
