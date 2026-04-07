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

//! Miscellaneous functions.

use std::path::{Path, PathBuf};

/// A `PathBuf` that can be truncated to a base path without re-allocating.
pub struct ResettablePathBuf {
    buffer: PathBuf,
    base_length: usize,
}

impl ResettablePathBuf {
    /// Creates a new `ResettablePathBuf` with the provided path as the base.
    #[must_use]
    pub fn new(base: PathBuf) -> Self {
        Self { base_length: base.as_os_str().len(), buffer: base }
    }

    /// Truncates the path to its base.
    pub fn reset_to_base(&mut self) {
        self.buffer.as_mut_os_string().truncate(self.base_length);
    }

    /// Sets the current path as the new base.
    pub fn set_base_to_current(&mut self) {
        self.base_length = self.buffer.as_os_str().len();
    }

    /// Extends the path with the specified relative path.
    #[inline]
    pub fn push<P: AsRef<Path>>(&mut self, path: P) -> &Path {
        self.push_inner(path.as_ref())
    }

    fn push_inner(&mut self, path: &Path) -> &Path {
        assert!(path.is_relative());
        self.buffer.push(path);
        &self.buffer
    }
}

impl AsRef<Path> for ResettablePathBuf {
    fn as_ref(&self) -> &Path {
        &self.buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn resettable_path_buf() {
        let base = Path::new("/a/b/c");
        let mut p = ResettablePathBuf::new(base.to_path_buf());
        assert_eq!(p.as_ref(), base);

        assert_eq!(p.push("d/e"), Path::new("/a/b/c/d/e"));
        assert_eq!(p.push("f"), Path::new("/a/b/c/d/e/f"));

        p.reset_to_base();
        assert_eq!(p.as_ref(), base);
        assert_eq!(p.push("0"), Path::new("/a/b/c/0"));

        p.set_base_to_current();
        assert_eq!(p.push("123"), Path::new("/a/b/c/0/123"));
        p.reset_to_base();
        assert_eq!(p.as_ref(), Path::new("/a/b/c/0"));
    }

    #[test]
    #[should_panic(expected = "assertion failed: path.is_relative()")]
    fn resettable_path_buf_push_absolute() {
        let mut p = ResettablePathBuf::new(PathBuf::from("/a/b/c"));
        p.push("/etc/passwd");
    }
}
