//! Presentation errors and exit codes (ADR-002, ARCHITECTURE 7.2).
//!
//! Exit code table (part of the public API; stable under semver):
//!
//! | Code | Error code (JSON) | [`ErrorKind`] variant | Meaning |
//! |---|---|---|---|
//! | 0 | - | - | Success, including "already installed", "no active version" and `--help` |
//! | 1 | `general` | `General` | Unclassified error |
//! | 2 | `usage` | `Usage` | Incorrect usage or unsupported spec (reserved alias) |
//! | 3 | `version-file-missing`, `version-file-invalid` | `VersionFileMissing`, `VersionFileInvalid` | `.nvmrc` / `.node-version` is missing or invalid |
//! | 4 | `no-artifact` | `NoArtifact` | Node does not publish a binary for this platform |
//! | 5 | `offline-no-cache` | `OfflineNoCache` | `--offline` without an index in the cache |
//! | 6 | `version-not-found` | `VersionNotFound` | Version or alias not found |
//! | 7 | `download-failed`, `checksum-mismatch` | `DownloadFailed`, `ChecksumMismatch` | Download or SHA-256 verification failed |
//! | 8 | `lock-busy` | `LockBusy` | The lock was not acquired in time |
//! | 9 | `confirmation-required` | `ConfirmationRequired` | Confirmation needed: pass `--yes` outside a TTY |
//! | 10 | `unsupported` | `Unsupported` | Operation not supported on this platform (e.g. Termux, `system`) |
//!
//! This table is the source of truth for the CLI. ARCHITECTURE 7.3 lists it
//! differently and must be corrected to match this one.
//!
//! Errors from `nvsn-core` and `nvsn-net` arrive as `anyhow`. They are converted
//! to [`CliError`] only at the boundary, with [`CliError::classify`].

use std::fmt;

/// Success.
pub const SUCCESS: u8 = 0;
/// Unclassified general error.
pub const GENERAL: u8 = 1;
/// Incorrect usage.
pub const USAGE: u8 = 2;
/// Version file missing or invalid.
pub const VERSION_FILE: u8 = 3;
/// No binary for the platform.
pub const NO_ARTIFACT: u8 = 4;
/// Offline mode without cache.
pub const OFFLINE_NO_CACHE: u8 = 5;
/// Version or alias not found.
pub const NOT_FOUND: u8 = 6;
/// Download or checksum failed.
pub const DOWNLOAD: u8 = 7;
/// Lock not acquired.
pub const LOCK_BUSY: u8 = 8;
/// Confirmation required.
pub const CONFIRMATION: u8 = 9;
/// Operation not supported on the platform.
pub const UNSUPPORTED: u8 = 10;

/// Error class of the CLI. Each variant fixes its JSON code and its exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Unclassified error (1).
    General,
    /// Incorrect usage (2).
    Usage,
    /// `.nvmrc` or `.node-version` not found (3).
    VersionFileMissing,
    /// `.nvmrc` or `.node-version` invalid (3).
    VersionFileInvalid,
    /// Node does not publish a binary for the platform (4).
    NoArtifact,
    /// `--offline` without an index in the cache (5).
    OfflineNoCache,
    /// Version or alias not found (6).
    VersionNotFound,
    /// Download failed (7).
    DownloadFailed,
    /// SHA-256 checksum does not match or cannot be verified (7).
    ChecksumMismatch,
    /// Version lock not acquired in time (8).
    LockBusy,
    /// Confirmation required without a TTY or `--yes` (9).
    ConfirmationRequired,
    /// Operation not supported on the platform (10).
    Unsupported,
}

impl ErrorKind {
    /// Stable error code for the JSON output (lowercase string with hyphens).
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Usage => "usage",
            Self::VersionFileMissing => "version-file-missing",
            Self::VersionFileInvalid => "version-file-invalid",
            Self::NoArtifact => "no-artifact",
            Self::OfflineNoCache => "offline-no-cache",
            Self::VersionNotFound => "version-not-found",
            Self::DownloadFailed => "download-failed",
            Self::ChecksumMismatch => "checksum-mismatch",
            Self::LockBusy => "lock-busy",
            Self::ConfirmationRequired => "confirmation-required",
            Self::Unsupported => "unsupported",
        }
    }

    /// Process exit code for this class.
    #[must_use]
    pub fn exit_code(self) -> u8 {
        match self {
            Self::General => GENERAL,
            Self::Usage => USAGE,
            Self::VersionFileMissing | Self::VersionFileInvalid => VERSION_FILE,
            Self::NoArtifact => NO_ARTIFACT,
            Self::OfflineNoCache => OFFLINE_NO_CACHE,
            Self::VersionNotFound => NOT_FOUND,
            Self::DownloadFailed | Self::ChecksumMismatch => DOWNLOAD,
            Self::LockBusy => LOCK_BUSY,
            Self::ConfirmationRequired => CONFIRMATION,
            Self::Unsupported => UNSUPPORTED,
        }
    }
}

/// Error shown to the user by the CLI, with cause, class and suggestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    kind: ErrorKind,
    /// Cause of the error.
    pub message: String,
    /// How to fix it, if known.
    pub hint: Option<String>,
}

impl CliError {
    /// Creates an error of class `kind` with its cause.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            hint: None,
        }
    }

    /// Adds the suggestion on how to fix it.
    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Stable error code (JSON).
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.kind.code()
    }

    /// Process exit code.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        self.kind.exit_code()
    }

    /// General error (code 1).
    pub fn general(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::General, message)
    }

    /// Incorrect usage (code 2).
    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Usage, message)
    }

    /// Version or alias not found (code 6).
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::VersionNotFound, message)
    }

    /// Operation not supported on the platform (code 10).
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unsupported, message)
    }

    /// Boundary with `nvsn-core` and `nvsn-net`: converts their error into a [`CliError`].
    ///
    /// Since those crates return `anyhow` without typed variants, the class is
    /// inferred from the text of the cause chain. This is the only place that
    /// inspects text; the rest of the CLI works with [`ErrorKind`]. The texts
    /// it recognizes are pinned by the tests in this module.
    pub fn classify(err: impl fmt::Display) -> Self {
        let text = format!("{err:#}");
        let lower = text.to_ascii_lowercase();
        // Download goes first: its context ("Failed to download <url>") can carry
        // words such as SHASUMS256 in the URL, which do not imply a checksum.
        let kind = if lower.contains("failed to download")
            || lower.contains("failed to fetch")
            || lower.contains("failed to connect")
            || lower.contains("download failed after")
        {
            ErrorKind::DownloadFailed
        } else if lower.contains("checksum") || lower.contains("sha-256") {
            ErrorKind::ChecksumMismatch
        } else if lower.contains("failed to acquire lock") || lower.contains("waiting for the lock")
        {
            ErrorKind::LockBusy
        } else if lower.contains("termux") {
            ErrorKind::Unsupported
        } else if lower.contains("does not publish the artifact") {
            // Spanish on purpose: this is the literal message emitted by
            // nvsn-core (artifact.rs). Do not translate it without changing that message.
            ErrorKind::NoArtifact
        } else {
            ErrorKind::General
        };
        Self::new(kind, text)
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [ErrorKind; 12] = [
        ErrorKind::General,
        ErrorKind::Usage,
        ErrorKind::VersionFileMissing,
        ErrorKind::VersionFileInvalid,
        ErrorKind::NoArtifact,
        ErrorKind::OfflineNoCache,
        ErrorKind::VersionNotFound,
        ErrorKind::DownloadFailed,
        ErrorKind::ChecksumMismatch,
        ErrorKind::LockBusy,
        ErrorKind::ConfirmationRequired,
        ErrorKind::Unsupported,
    ];

    #[test]
    fn every_kind_maps_to_the_documented_exit_code() {
        let codes: Vec<u8> = ALL.iter().map(|kind| kind.exit_code()).collect();
        assert_eq!(codes, vec![1, 2, 3, 3, 4, 5, 6, 7, 7, 8, 9, 10]);
    }

    #[test]
    fn json_codes_are_unique_and_lowercase() {
        let mut codes: Vec<&str> = ALL.iter().map(|kind| kind.code()).collect();
        assert!(codes
            .iter()
            .all(|c| c.chars().all(|ch| ch.is_ascii_lowercase() || ch == '-')));
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), ALL.len());
    }

    #[test]
    fn classifies_checksum_errors() {
        let err = CliError::classify("Checksum mismatch for x.tar.gz!");
        assert_eq!(err.kind, ErrorKind::ChecksumMismatch);
        assert_eq!(err.exit_code(), DOWNLOAD);
    }

    #[test]
    fn classifies_download_errors_from_chain() {
        let err = CliError::classify("Failed to download https://x/y: connection refused");
        assert_eq!(err.kind, ErrorKind::DownloadFailed);
        assert_eq!(err.code(), "download-failed");
    }

    #[test]
    fn failed_shasums_download_is_a_download_not_a_checksum() {
        let err = CliError::classify(
            "Failed to download https://nodejs.org/dist/v20.0.0/SHASUMS256.txt: HTTP 404",
        );
        assert_eq!(err.kind, ErrorKind::DownloadFailed);
    }

    #[test]
    fn classifies_retries_exhausted_as_download() {
        let err = CliError::classify("Download failed after 3 retries: Failed to connect to x");
        assert_eq!(err.kind, ErrorKind::DownloadFailed);
    }

    #[test]
    fn classifies_lock_timeout() {
        let err = CliError::classify(
            "timed out after 600 s waiting for the lock on v20.0.0; another nvsn process",
        );
        assert_eq!(err.kind, ErrorKind::LockBusy);
        assert_eq!(err.exit_code(), LOCK_BUSY);
    }

    #[test]
    fn classifies_missing_artifact() {
        // Input mirrors the message emitted by nvsn-core (artifact.rs), which is in Spanish.
        let err = CliError::classify("node v26.0.0 does not publish the artifact linux-x64 (x)");
        assert_eq!(err.kind, ErrorKind::NoArtifact);
    }

    #[test]
    fn classifies_termux_as_unsupported() {
        let err = CliError::classify("Android without Termux is not supported");
        assert_eq!(err.kind, ErrorKind::Unsupported);
    }

    #[test]
    fn unknown_errors_are_general() {
        let err = CliError::classify("something else");
        assert_eq!(err.kind, ErrorKind::General);
        assert_eq!(err.exit_code(), GENERAL);
    }

    #[test]
    fn hint_is_kept() {
        let err = CliError::usage("bad").with_hint("try this");
        assert_eq!(err.hint.as_deref(), Some("try this"));
    }
}
