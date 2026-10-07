//! Local store layout (ARCHITECTURE.md, section 8).
//!
//! [`Store`] only computes paths. The only I/O operation is
//! [`Store::create_dirs`]. The root is received from the caller: this module
//! does not depend on `nvsn-platform`.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// nvsn store root (`NVSN_DIR`) and derived paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// Creates a `Store` rooted at `root`. Does not touch the disk.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Returns the store root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory that contains one folder per installed version.
    pub fn versions_dir(&self) -> PathBuf {
        self.root.join("versions")
    }

    /// Directory of a specific version, e.g. `versions/v20.11.1`.
    ///
    /// `version` must already be validated by the version parser; this
    /// method does not check that it contains no path separators.
    pub fn version_dir(&self, version: &str) -> PathBuf {
        self.versions_dir().join(version)
    }

    /// Alias directory, one file per alias.
    pub fn aliases_dir(&self) -> PathBuf {
        self.root.join("aliases")
    }

    /// Cache directory (resumable downloads and the remote index).
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }

    /// Per-version lock directory.
    pub fn locks_dir(&self) -> PathBuf {
        self.root.join("locks")
    }

    /// File that stores the default version (plain text).
    pub fn default_file(&self) -> PathBuf {
        self.root.join("default")
    }

    /// Store lock, used for aliases and the default.
    pub fn lock_file(&self) -> PathBuf {
        self.root.join("lock")
    }

    /// Creates the top-level directories if they do not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if any directory cannot be created.
    pub fn create_dirs(&self) -> Result<()> {
        for dir in [
            self.versions_dir(),
            self.aliases_dir(),
            self.cache_dir(),
            self.locks_dir(),
        ] {
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("Failed to create {}", dir.display()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn paths_are_derived_from_root() {
        let store = Store::new("/data/nvsn");
        let root = Path::new("/data/nvsn");

        assert_eq!(store.root(), root);
        assert_eq!(store.versions_dir(), root.join("versions"));
        assert_eq!(
            store.version_dir("v20.11.1"),
            root.join("versions").join("v20.11.1")
        );
        assert_eq!(store.aliases_dir(), root.join("aliases"));
        assert_eq!(store.cache_dir(), root.join("cache"));
        assert_eq!(store.locks_dir(), root.join("locks"));
        assert_eq!(store.default_file(), root.join("default"));
        assert_eq!(store.lock_file(), root.join("lock"));
    }

    #[test]
    fn new_does_not_touch_disk() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("nvsn");
        let _store = Store::new(&root);
        assert!(!root.exists());
    }

    #[test]
    fn create_dirs_creates_layout() {
        let dir = tempdir().unwrap();
        let store = Store::new(dir.path().join("nvsn"));

        store.create_dirs().unwrap();
        assert!(store.versions_dir().is_dir());
        assert!(store.aliases_dir().is_dir());
        assert!(store.cache_dir().is_dir());
        assert!(store.locks_dir().is_dir());

        // Idempotent.
        store.create_dirs().unwrap();
    }
}
