//! Maintenance of the download cache: listing and clearing (ARCHITECTURE 8.2).
//!
//! The cache has two areas under `<NVSN_DIR>/cache/`:
//! - `downloads/`: resumable downloads (`.part` files and archives).
//! - `index/`: the cached remote `index.json` and its metadata.
//!
//! Installed versions (`versions/`) are never touched by this module.

use crate::store::Store;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// The two areas of the download cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheDirs {
    /// `cache/downloads`: resumable downloads.
    pub downloads: PathBuf,
    /// `cache/index`: cached remote index files.
    pub index: PathBuf,
}

/// Which cache area a file belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheKind {
    /// A file in `cache/downloads`.
    Download,
    /// A file in `cache/index`.
    Index,
}

/// One regular file in the cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheEntry {
    /// Full path of the file.
    pub path: PathBuf,
    /// Area the file belongs to.
    pub kind: CacheKind,
    /// Size in bytes.
    pub bytes: u64,
}

/// Result of [`cache_clear`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheClearReport {
    /// Number of regular files removed.
    pub files: usize,
    /// Total size of the removed files, in bytes.
    pub bytes: u64,
}

/// Returns the two cache areas of `store`. Does not touch the disk.
pub fn cache_dirs(store: &Store) -> CacheDirs {
    CacheDirs {
        downloads: store.cache_dir().join("downloads"),
        index: store.cache_dir().join("index"),
    }
}

/// Lists every regular file in both cache areas, sorted by path.
///
/// Missing directories are treated as empty. Symbolic links are not followed
/// and not listed.
///
/// # Errors
///
/// Returns an error if a directory exists but cannot be read.
pub fn cache_list(store: &Store) -> Result<Vec<CacheEntry>> {
    let dirs = cache_dirs(store);
    let mut entries = Vec::new();
    collect(&dirs.downloads, CacheKind::Download, &mut entries)?;
    collect(&dirs.index, CacheKind::Index, &mut entries)?;
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

/// Deletes all downloads and the cached index, then recreates both empty areas.
///
/// Only `cache/downloads` and `cache/index` are removed. `versions/`,
/// aliases, the default and the config file are left as they are.
///
/// # Errors
///
/// Returns an error if the cache cannot be listed or an area cannot be removed
/// or recreated. Files removed before a failure stay removed.
pub fn cache_clear(store: &Store) -> Result<CacheClearReport> {
    let entries = cache_list(store)?;
    let report = CacheClearReport {
        files: entries.len(),
        bytes: entries.iter().map(|e| e.bytes).sum(),
    };

    let dirs = cache_dirs(store);
    for dir in [&dirs.downloads, &dirs.index] {
        if dir.exists() {
            std::fs::remove_dir_all(dir)
                .with_context(|| format!("failed to remove {}", dir.display()))?;
        }
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create {}", dir.display()))?;
    }
    Ok(report)
}

/// Adds the regular files under `dir` (recursively) to `out`.
fn collect(dir: &Path, kind: CacheKind, out: &mut Vec<CacheEntry>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    let read =
        std::fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))?;
    for entry in read {
        let entry = entry.with_context(|| format!("failed to read {}", dir.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", entry.path().display()))?;
        let path = entry.path();
        if file_type.is_dir() {
            collect(&path, kind, out)?;
        } else if file_type.is_file() {
            let bytes = entry
                .metadata()
                .with_context(|| format!("failed to inspect {}", path.display()))?
                .len();
            out.push(CacheEntry { path, kind, bytes });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    #[test]
    fn cache_dirs_are_under_the_store_cache_directory() {
        let store = Store::new("/data/nvsn");
        let dirs = cache_dirs(&store);
        assert_eq!(dirs.downloads, Path::new("/data/nvsn/cache/downloads"));
        assert_eq!(dirs.index, Path::new("/data/nvsn/cache/index"));
    }

    #[test]
    fn list_on_an_empty_store_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        assert!(cache_list(&store).unwrap().is_empty());
    }

    #[test]
    fn list_reports_both_areas_with_sizes_sorted_by_path() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let dirs = cache_dirs(&store);
        write(&dirs.downloads.join("node-v20.zip.part"), 10);
        write(&dirs.downloads.join("nested").join("big.tar.gz"), 25);
        write(&dirs.index.join("abc.json"), 7);

        let entries = cache_list(&store).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries.iter().map(|e| e.bytes).sum::<u64>(), 42);
        assert_eq!(
            entries
                .iter()
                .filter(|e| e.kind == CacheKind::Index)
                .count(),
            1
        );
        let mut sorted = entries.clone();
        sorted.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(entries, sorted);
    }

    #[test]
    fn clear_removes_downloads_and_index_but_keeps_versions() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let dirs = cache_dirs(&store);
        write(&dirs.downloads.join("a.part"), 4);
        write(&dirs.index.join("abc.json"), 6);
        write(&dirs.index.join("abc.meta.json"), 2);
        let installed = store.version_dir("v20.11.1").join("node");
        write(&installed, 9);

        let report = cache_clear(&store).unwrap();
        assert_eq!(
            report,
            CacheClearReport {
                files: 3,
                bytes: 12
            }
        );
        assert!(cache_list(&store).unwrap().is_empty());
        assert!(dirs.downloads.is_dir());
        assert!(dirs.index.is_dir());
        assert!(installed.is_file(), "versions/ must not be touched");
    }

    #[test]
    fn clear_on_an_empty_store_creates_the_areas_and_reports_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let report = cache_clear(&store).unwrap();
        assert_eq!(report, CacheClearReport::default());
        let dirs = cache_dirs(&store);
        assert!(dirs.downloads.is_dir());
        assert!(dirs.index.is_dir());
    }
}
