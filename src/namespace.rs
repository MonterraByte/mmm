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

use std::fmt::Display;

use rustix::fs::{Mode, OFlags, open};
use rustix::io::{Errno, write};
use rustix::process::{Gid, Uid, getgid, getuid};
use rustix::thread::{self, UnshareFlags};
use thiserror::Error;

use crate::caps::have_cap_sys_admin;

pub fn enter_namespace() -> Result<(), EnterNamespaceError> {
    let uid = getuid();
    let gid = getgid();

    const FLAGS: UnshareFlags = UnshareFlags::NEWUSER.union(UnshareFlags::NEWNS); // implies CLONE_THREAD and CLONE_FS, respectively.
    const _: () = assert!(!FLAGS.contains(UnshareFlags::FILES));

    // SAFETY: UnshareFlags::FILES is not used.
    unsafe { thread::unshare_unsafe(FLAGS).map_err(EnterNamespaceError::Unshare)? }
    set_up_uid_and_gid_map(uid, gid)?;

    assert_eq!(getuid(), uid);
    assert_eq!(getgid(), gid);
    assert!(have_cap_sys_admin());
    Ok(())
}

fn set_up_uid_and_gid_map(uid: Uid, gid: Gid) -> Result<(), EnterNamespaceError> {
    write_map("/proc/self/uid_map", uid).map_err(EnterNamespaceError::WriteUidMap)?;
    write_file("/proc/self/setgroups", "deny").map_err(EnterNamespaceError::WriteSetgroups)?;
    write_map("/proc/self/gid_map", gid).map_err(EnterNamespaceError::WriteGidMap)
}

fn write_map<Id: Display>(path: &str, id: Id) -> Result<(), WriteFileError> {
    let map = format!("{0} {0} 1\n", id);
    write_file(path, &map)
}

fn write_file(path: &str, value: &str) -> Result<(), WriteFileError> {
    let fd = open(path, OFlags::WRONLY | OFlags::CLOEXEC, Mode::empty()).map_err(WriteFileError::Open)?;
    let written = write(&fd, value.as_bytes()).map_err(WriteFileError::Write)?;
    if written != value.len() {
        return Err(WriteFileError::IncompleteWrite(value.len() - written));
    }
    Ok(())
}

#[derive(Copy, Clone, Debug, Error)]
pub enum EnterNamespaceError {
    #[error("unshare failed: {0}")]
    Unshare(Errno),
    #[error("failed to write uid map: {0}")]
    WriteUidMap(WriteFileError),
    #[error("failed to write gid map: {0}")]
    WriteGidMap(WriteFileError),
    #[error("failed to write setgroups: {0}")]
    WriteSetgroups(WriteFileError),
}

#[derive(Copy, Clone, Debug, Error)]
pub enum WriteFileError {
    #[error("open failed: {0}")]
    Open(Errno),
    #[error("write failed: {0}")]
    Write(Errno),
    #[error("incomplete write ({0} bytes were not written)")]
    IncompleteWrite(usize),
}
