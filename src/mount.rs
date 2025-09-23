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
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

use rustix::fs::{Mode, OFlags, fstat, open};
use rustix::io::Errno;
use rustix::mount::{
    FsMountFlags, FsOpenFlags, MountAttrFlags, MoveMountFlags, UnmountFlags, fsconfig_create, fsconfig_set_fd,
    fsconfig_set_string, fsmount, fsopen, move_mount, unmount,
};
use rustix::process::{getgid, getuid};
use tempfile::TempDir;
use thiserror::Error;

use crate::caps::ElevatedCaps;

fn mount_overlayfs(staging_path: &Path, game_path: &Path) -> Result<(), MountError> {
    assert!(staging_path.is_absolute());
    let game_dir = open_dir_and_check_ownership(game_path)?;
    let _caps = ElevatedCaps::raise();

    let fs_fd = fsopen("overlay", FsOpenFlags::FSOPEN_CLOEXEC).map_err(MountError::FsOpen)?;
    fsconfig_set_string(&fs_fd, "source", "overlay").map_err(MountError::FsConfigSet)?;
    fsconfig_set_string(&fs_fd, "lowerdir+", staging_path).map_err(MountError::FsConfigSet)?;
    fsconfig_set_fd(&fs_fd, "lowerdir+", &game_dir).map_err(MountError::FsConfigSet)?;
    fsconfig_create(&fs_fd).map_err(MountError::FsConfigCreate)?;

    let mfd = fsmount_with_flags(&fs_fd)?;
    move_mount_fds(&mfd, &game_dir)
}

fn mount_tmpfs(path: &Path) -> Result<(), MountError> {
    let dir = open_dir_and_check_ownership(path)?;
    let _caps = ElevatedCaps::raise();

    let fs_fd = fsopen("tmpfs", FsOpenFlags::FSOPEN_CLOEXEC).map_err(MountError::FsOpen)?;
    fsconfig_set_string(&fs_fd, "source", "tmpfs").map_err(MountError::FsConfigSet)?;
    fsconfig_set_string(&fs_fd, "uid", getuid().to_string()).map_err(MountError::FsConfigSet)?;
    fsconfig_set_string(&fs_fd, "gid", getgid().to_string()).map_err(MountError::FsConfigSet)?;
    fsconfig_set_string(&fs_fd, "mode", "750").map_err(MountError::FsConfigSet)?;
    fsconfig_create(&fs_fd).map_err(MountError::FsConfigCreate)?;

    let mfd = fsmount_with_flags(&fs_fd)?;
    move_mount_fds(&mfd, &dir)
}

fn open_dir_and_check_ownership(path: &Path) -> Result<OwnedFd, MountError> {
    let fd = open(
        path,
        OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(MountError::Open)?;

    let stat = fstat(&fd).map_err(MountError::Fstat)?;
    if stat.st_uid != getuid().as_raw() {
        return Err(MountError::NotOwned);
    }

    Ok(fd)
}

fn fsmount_with_flags(fs_fd: &OwnedFd) -> Result<OwnedFd, MountError> {
    fsmount(
        fs_fd,
        FsMountFlags::FSMOUNT_CLOEXEC,
        MountAttrFlags::MOUNT_ATTR_NODEV | MountAttrFlags::MOUNT_ATTR_NOSUID | MountAttrFlags::MOUNT_ATTR_NOATIME,
    )
    .map_err(MountError::FsMount)
}

fn move_mount_fds(from_fd: &OwnedFd, to_fd: &OwnedFd) -> Result<(), MountError> {
    move_mount(
        from_fd,
        "",
        to_fd,
        "",
        MoveMountFlags::MOVE_MOUNT_F_EMPTY_PATH | MoveMountFlags::MOVE_MOUNT_T_EMPTY_PATH,
    )
    .map_err(MountError::MoveMount)
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
    #[error("failed to fstat mount target directory: {0}")]
    Fstat(#[source] Errno),
    #[error("move_mount failed: {0}")]
    MoveMount(#[source] Errno),
    #[error("target directory is not owned by the user")]
    NotOwned,
    #[error("failed to open mount target directory: {0}")]
    Open(#[source] Errno),
}

#[derive(Debug)]
pub struct OverlayMount(UnmountWrapper<PathBuf>);

impl OverlayMount {
    pub fn new(staging_dir: &Path, game_dir: &Path) -> Result<Self, MountError> {
        mount_overlayfs(staging_dir, game_dir)?;
        Ok(Self(UnmountWrapper::new(game_dir.to_owned())))
    }

    pub fn path(&self) -> &Path {
        self.0.path()
    }

    pub fn unmount(self) -> Result<(), Errno> {
        self.0.unmount().and(Ok(()))
    }
}

#[derive(Debug)]
pub struct TempMount(UnmountWrapper<TempDir>);

impl TempMount {
    pub fn new() -> Result<Self, TempMountCreationError> {
        let temp_dir = TempDir::with_prefix("mmm-").map_err(TempMountCreationError::TempDir)?;
        mount_tmpfs(temp_dir.path())?;
        Ok(Self(UnmountWrapper::new(temp_dir)))
    }

    pub fn path(&self) -> &Path {
        self.0.path()
    }

    pub fn unmount(self) -> Result<(), TempMountUnmountError> {
        let temp_dir = self.0.unmount().map_err(TempMountUnmountError::Unmount)?;
        temp_dir.close().map_err(TempMountUnmountError::TempDir)
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

#[derive(Debug)]
struct UnmountWrapper<P: AsRef<Path>>(Option<P>);

impl<P: AsRef<Path>> UnmountWrapper<P> {
    pub fn new(path: P) -> Self {
        Self(Some(path))
    }

    pub fn path(&self) -> &Path {
        self.0.as_ref().expect("not dropped yet").as_ref()
    }

    pub fn unmount(mut self) -> Result<P, Errno> {
        self.unmount_inner()?;
        Ok(self.0.take().expect("not dropped yet"))
    }

    fn unmount_inner(&self) -> Result<(), Errno> {
        let _caps = ElevatedCaps::raise();
        unmount(self.path(), UnmountFlags::DETACH | UnmountFlags::NOFOLLOW)
    }
}

impl<P: AsRef<Path>> Drop for UnmountWrapper<P> {
    fn drop(&mut self) {
        if self.0.is_none() {
            // already unmounted
            return;
        }
        let _ = self.unmount_inner();
    }
}
