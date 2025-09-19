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

use std::io;
use std::path::Path;

use rustix::fs::CWD;
use rustix::io::Errno;
use rustix::mount::{
    FsMountFlags, FsOpenFlags, MountAttrFlags, MoveMountFlags, UnmountFlags, fsconfig_create, fsconfig_set_string,
    fsmount, fsopen, move_mount, unmount,
};
use rustix::process::{getgid, getuid};
use tempfile::TempDir;
use thiserror::Error;

fn mount_tmpfs(dir: &Path) -> Result<(), MountError> {
    let uid = getuid();
    let gid = getgid();

    let fs_fd = fsopen("tmpfs", FsOpenFlags::FSOPEN_CLOEXEC).map_err(MountError::FsOpen)?;
    fsconfig_set_string(&fs_fd, "source", "tmpfs").map_err(MountError::FsConfigSet)?;
    fsconfig_set_string(&fs_fd, "uid", uid.to_string()).map_err(MountError::FsConfigSet)?;
    fsconfig_set_string(&fs_fd, "gid", gid.to_string()).map_err(MountError::FsConfigSet)?;
    fsconfig_set_string(&fs_fd, "mode", "750").map_err(MountError::FsConfigSet)?;
    fsconfig_create(&fs_fd).map_err(MountError::FsConfigCreate)?;

    let mfd = fsmount(
        &fs_fd,
        FsMountFlags::FSMOUNT_CLOEXEC,
        MountAttrFlags::MOUNT_ATTR_NODEV | MountAttrFlags::MOUNT_ATTR_NOSUID | MountAttrFlags::MOUNT_ATTR_NOATIME,
    )
    .map_err(MountError::FsMount)?;

    move_mount(mfd, "", CWD, dir, MoveMountFlags::MOVE_MOUNT_F_EMPTY_PATH).map_err(MountError::MoveMount)
}

#[derive(Copy, Clone, Debug, Error)]
pub enum MountError {
    #[error("fsconfig_create failed: {0}")]
    FsConfigCreate(#[source] Errno),
    #[error("fsconfig_set_* failed: {0}")]
    FsConfigSet(#[source] Errno),
    #[error("fsmount failed: {0}")]
    FsMount(#[source] Errno),
    #[error("fsopen failed: {0}")]
    FsOpen(#[source] Errno),
    #[error("move_mount failed: {0}")]
    MoveMount(#[source] Errno),
}

pub struct TempMount(Option<TempDir>);

impl TempMount {
    pub fn new() -> Result<Self, TempMountCreationError> {
        let temp_dir = TempDir::with_prefix("mmm-").map_err(TempMountCreationError::TempDir)?;
        mount_tmpfs(temp_dir.path())?;
        Ok(Self(Some(temp_dir)))
    }

    pub fn path(&self) -> &Path {
        self.0.as_ref().expect("not dropped yet").path()
    }

    pub fn unmount(mut self) -> Result<(), TempMountUnmountError> {
        self.unmount_inner().map_err(TempMountUnmountError::Unmount)?;
        if let Some(path) = self.0.take() {
            path.close().map_err(TempMountUnmountError::TempDir)?;
        }

        Ok(())
    }

    fn unmount_inner(&mut self) -> Result<(), Errno> {
        unmount(self.path(), UnmountFlags::DETACH | UnmountFlags::NOFOLLOW)
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
    Mount(#[from] MountError),
    #[error("failed to create temporary directory: {0}")]
    TempDir(#[source] io::Error),
}

#[derive(Debug, Error)]
pub enum TempMountUnmountError {
    #[error("failed to delete temporary directory: {0}")]
    TempDir(#[source] io::Error),
    #[error("failed to unmount tmpfs: {0}")]
    Unmount(#[source] Errno),
}
