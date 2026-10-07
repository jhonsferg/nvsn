//! Temporary directories with automatic cleanup, built on `tempfile`.
//!
//! Adapted from gvsn (MIT, same author).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use tempfile::TempDir as StdTempDir;

/// Temporary directory that is deleted on drop, unless [`TempDir::keep`] is called.
#[derive(Debug)]
pub struct TempDir {
    inner: StdTempDir,
}

impl TempDir {
    /// Creates a temporary directory in the system's temporary location.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created.
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            inner: StdTempDir::new()?,
        })
    }

    /// Creates a temporary directory inside `parent`, with the given prefix.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created.
    pub fn new_in(parent: impl AsRef<Path>, prefix: impl AsRef<OsStr>) -> anyhow::Result<Self> {
        Ok(Self {
            inner: StdTempDir::with_prefix_in(prefix, parent)?,
        })
    }

    /// Returns the path of the temporary directory.
    pub fn path(&self) -> &Path {
        self.inner.path()
    }

    /// Disables automatic deletion and returns the directory path.
    pub fn keep(self) -> PathBuf {
        self.inner.keep()
    }
}

impl AsRef<Path> for TempDir {
    fn as_ref(&self) -> &Path {
        self.path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tempdir_creates_and_cleans_up() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        assert!(path.exists());

        drop(dir);
        assert!(!path.exists());
    }

    #[test]
    fn tempdir_in_parent() {
        let parent = TempDir::new().unwrap();
        let child = TempDir::new_in(parent.path(), "child-").unwrap();
        assert!(child.path().starts_with(parent.path()));
        assert!(child
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("child-"));
    }

    #[test]
    fn tempdir_keep() {
        let dir = TempDir::new().unwrap();
        let path = dir.keep();
        assert!(path.exists());
        std::fs::remove_dir_all(&path).unwrap();
    }
}
