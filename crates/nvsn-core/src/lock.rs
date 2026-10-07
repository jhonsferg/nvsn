//! Advisory inter-process locks with `fs2`.
//!
//! Adapted from gvsn (MIT, same author). The migration to `fs4` (ADR-010)
//! will be done as a separate task.

use anyhow::{Context, Result};
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::path::Path;

/// System advisory exclusive lock.
///
/// Uses `flock` on Unix and `LockFileEx` on Windows. The lock is released when
/// the `FileLock` is dropped, and the system releases it if the process dies.
#[derive(Debug)]
pub struct FileLock {
    file: File,
}

impl FileLock {
    /// Acquires an exclusive lock on `path`, blocking until it is obtained.
    ///
    /// Creates the lock file if it does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be opened or the lock fails.
    pub fn lock(path: &Path) -> Result<Self> {
        let file = open_lock_file(path)?;
        file.lock_exclusive()
            .with_context(|| format!("Failed to acquire lock on {}", path.display()))?;
        Ok(Self { file })
    }

    /// Tries to acquire an exclusive lock without waiting.
    ///
    /// Returns `Ok(Some(lock))` if it was obtained, `Ok(None)` if another process
    /// holds it, and `Err` in any other case.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be opened or fails for another reason.
    pub fn try_lock(path: &Path) -> Result<Option<Self>> {
        let file = open_lock_file(path)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { file })),
            Err(e) if is_lock_busy(&e) => Ok(None),
            Err(e) => Err(e).with_context(|| format!("Failed to try lock on {}", path.display())),
        }
    }
}

/// Windows `ERROR_LOCK_VIOLATION` code: `LockFileEx` with `LOCKFILE_FAIL_IMMEDIATELY`
/// returns this error (and not `WouldBlock`) when another process holds the lock.
#[cfg(windows)]
const WINDOWS_LOCK_VIOLATION: i32 = 33;

/// Indicates whether the `try_lock_exclusive` error means "lock busy" rather than a real failure.
#[cfg(windows)]
fn is_lock_busy(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::WouldBlock || e.raw_os_error() == Some(WINDOWS_LOCK_VIOLATION)
}

/// On Unix, `flock` with `LOCK_NB` already reports `WouldBlock`.
#[cfg(not(windows))]
fn is_lock_busy(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::WouldBlock
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// Opens the lock file, creating it if necessary.
fn open_lock_file(path: &Path) -> Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .with_context(|| format!("Failed to open lock file {}", path.display()))
}

/// Runs `f` while holding an exclusive lock on `path`.
///
/// The lock is released when `f` returns, including if `f` panics.
///
/// # Errors
///
/// Returns an error if the lock cannot be acquired or if `f` returns an error.
pub fn with_lock<F, T>(path: &Path, f: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    let _lock = FileLock::lock(path)?;
    f()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn lock_prevents_concurrent_access() {
        let dir = tempdir().unwrap();
        let lock_path = dir.path().join("test.lock");

        let lock1 = FileLock::lock(&lock_path).unwrap();

        let lock_path_clone = lock_path.clone();
        let handle = thread::spawn(move || {
            // On Windows, try_lock may return an error while the lock is held;
            // on Unix it returns Ok(None). Both are accepted.
            match FileLock::try_lock(&lock_path_clone) {
                Ok(None) => None,
                Ok(Some(_)) => Some(()),
                Err(_) => None,
            }
        });

        thread::sleep(Duration::from_millis(50));
        let result = handle.join().unwrap();
        assert!(result.is_none(), "Second lock should fail");

        drop(lock1);

        let lock2 = FileLock::try_lock(&lock_path).unwrap();
        assert!(
            lock2.is_some(),
            "Should acquire lock after first is released"
        );
    }

    #[test]
    fn with_lock_releases_on_panic() {
        let dir = tempdir().unwrap();
        let lock_path = dir.path().join("panic.lock");

        let result = std::panic::catch_unwind(|| {
            let _ = with_lock(&lock_path, || -> Result<()> {
                panic!("intentional panic");
            });
        });
        assert!(result.is_err());

        let lock = FileLock::try_lock(&lock_path).unwrap();
        assert!(lock.is_some());
    }

    #[test]
    fn with_lock_works_normally() {
        let dir = tempdir().unwrap();
        let lock_path = dir.path().join("normal.lock");

        let result = with_lock(&lock_path, || Ok(42));

        assert_eq!(result.unwrap(), 42);
    }
}
