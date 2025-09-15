// Copyright © 2025 Joaquim Monteiro
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

use std::ffi::OsString;
use std::io;
use std::path::Path;

use nix::mount::{MntFlags, MsFlags, mount, umount2};
use nix::unistd::{Gid, Uid};
use tempfile::TempDir;
use thiserror::Error;

fn mount_tmpfs(dir: &Path) -> Result<(), nix::Error> {
    let uid = Uid::current();
    let gid = Gid::current();
    let mount_args: OsString = format!("uid={uid},gid={gid},mode=750").into();
    mount(
        Some("tmpfs"),
        dir,
        Some("tmpfs"),
        MsFlags::MS_NODEV | MsFlags::MS_NOSUID | MsFlags::MS_NOATIME,
        Some(mount_args.as_os_str()),
    )
}

pub struct TempMount(Option<TempDir>);

impl TempMount {
    pub fn new() -> Result<Self, TempMountCreationError> {
        let temp_dir = TempDir::with_prefix("mmm-").map_err(TempMountCreationError::TempDir)?;
        mount_tmpfs(temp_dir.path()).map_err(TempMountCreationError::Mount)?;
        Ok(Self(Some(temp_dir)))
    }

    pub fn path(&self) -> &Path {
        self.0.as_ref().expect("not dropped yet").path()
    }

    pub fn unmount(mut self) -> Result<(), TempMountUnmountError> {
        self.unmount_inner().map_err(TempMountUnmountError::Mount)?;
        if let Some(path) = self.0.take() {
            path.close().map_err(TempMountUnmountError::TempDir)?;
        }

        Ok(())
    }

    fn unmount_inner(&mut self) -> Result<(), nix::Error> {
        umount2(self.path(), MntFlags::MNT_DETACH | MntFlags::UMOUNT_NOFOLLOW)
    }
}

impl Drop for TempMount {
    fn drop(&mut self) {
        if self.0.is_none() {
            // already unmounted
            return;
        }
        let _ = self.unmount_inner();
    }
}

#[derive(Debug, Error)]
pub enum TempMountCreationError {
    #[error("failed to mount tmpfs: {0}")]
    Mount(#[source] nix::Error),
    #[error("failed to create temporary directory: {0}")]
    TempDir(#[source] io::Error),
}

#[derive(Debug, Error)]
pub enum TempMountUnmountError {
    #[error("failed to unmount tmpfs: {0}")]
    Mount(#[source] nix::Error),
    #[error("failed to delete temporary directory: {0}")]
    TempDir(#[source] io::Error),
}
