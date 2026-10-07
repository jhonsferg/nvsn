//! SHA-256 verification of downloaded files (ADR-007, ADR-033).
//!
//! Adapted from gvsn (MIT, same author). Unlike gvsn, a missing checksum is an
//! error: nothing is installed without verification.

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

/// Length in hexadecimal characters of a SHA-256 digest.
const SHA256_HEX_LEN: usize = 64;

/// Verifies the SHA-256 of `file` against the hexadecimal digest `expected`.
///
/// The comparison is case-insensitive. If `expected` is empty (or contains only
/// spaces), the verification is rejected: there is no warning and no skip mode
/// (ADR-033). The file is not deleted here; the caller decides that.
///
/// # Errors
///
/// Returns an error if:
/// - `expected` is empty.
/// - `file` cannot be read.
/// - The computed digest does not match `expected`.
pub fn verify_sha256(file: &Path, expected: &str) -> Result<()> {
    let expected = expected.trim().to_ascii_lowercase();
    if expected.is_empty() {
        bail!(
            "no published SHA-256 for {}; refusing to install unverified archive",
            file.display()
        );
    }

    let mut hasher = Sha256::new();
    let mut f = std::fs::File::open(file)
        .with_context(|| format!("Cannot open {} for checksum", file.display()))?;
    let mut buf = [0u8; 65_536];
    loop {
        let n = f
            .read(&mut buf)
            .context("Failed to read file for checksum")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    let actual: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    if actual != expected {
        bail!(
            "Checksum mismatch for {}!\n  expected: {expected}\n  got:      {actual}",
            file.display()
        );
    }
    Ok(())
}

/// Looks up the digest of `filename` in the text of a Node `SHASUMS256.txt`.
///
/// The format is `<sha256>  <name>` (two spaces). Rules:
/// - The name comparison is exact, not substring-based.
/// - Lines whose name contains `/` are discarded (e.g. `win-x64/node.exe`).
/// - A hash that does not have 64 hexadecimal characters is discarded.
///
/// Returns the digest in lowercase, or `None` if there is no match.
pub fn parse_shasums(text: &str, filename: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        let Some((hash, name)) = line.split_once("  ") else {
            continue;
        };
        if name.contains('/') || name != filename {
            continue;
        }
        if hash.len() == SHA256_HEX_LEN && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Some(hash.to_ascii_lowercase());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const HELLO_SHA256: &str = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
    const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn verify_sha256_accepts_matching_digest() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("data.bin");
        std::fs::write(&file, b"hello world").unwrap();

        verify_sha256(&file, HELLO_SHA256).unwrap();
    }

    #[test]
    fn verify_sha256_accepts_uppercase_digest() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("data.bin");
        std::fs::write(&file, b"hello world").unwrap();

        verify_sha256(&file, &HELLO_SHA256.to_ascii_uppercase()).unwrap();
    }

    #[test]
    fn verify_sha256_rejects_mismatched_digest() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("data.bin");
        std::fs::write(&file, b"hello world").unwrap();

        let err = verify_sha256(
            &file,
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap_err();
        assert!(err.to_string().contains("Checksum mismatch"));
    }

    #[test]
    fn verify_sha256_refuses_empty_expected() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("data.bin");
        std::fs::write(&file, b"anything").unwrap();

        let err = verify_sha256(&file, "").unwrap_err();
        assert!(err
            .to_string()
            .contains("refusing to install unverified archive"));
    }

    #[test]
    fn verify_sha256_refuses_whitespace_expected() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("data.bin");
        std::fs::write(&file, b"anything").unwrap();

        assert!(verify_sha256(&file, "   ").is_err());
    }

    #[test]
    fn verify_sha256_errors_when_file_missing() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("missing.bin");
        let err = verify_sha256(&file, HELLO_SHA256).unwrap_err();
        assert!(err.to_string().contains("Cannot open"));
    }

    #[test]
    fn parse_shasums_finds_exact_name() {
        let text = format!(
            "{HASH_A}  node-v20.11.1-linux-x64.tar.gz\n{HASH_B}  node-v20.11.1-win-x64.zip\n"
        );
        assert_eq!(
            parse_shasums(&text, "node-v20.11.1-win-x64.zip"),
            Some(HASH_B.to_string())
        );
    }

    #[test]
    fn parse_shasums_handles_crlf_and_uppercase_hash() {
        let text = format!("{}  node.tar.gz\r\n", HASH_A.to_ascii_uppercase());
        assert_eq!(
            parse_shasums(&text, "node.tar.gz"),
            Some(HASH_A.to_string())
        );
    }

    #[test]
    fn parse_shasums_ignores_subdirectory_entries() {
        let text = format!("{HASH_A}  win-x64/node.exe\n");
        assert_eq!(parse_shasums(&text, "node.exe"), None);
        assert_eq!(parse_shasums(&text, "win-x64/node.exe"), None);
    }

    #[test]
    fn parse_shasums_does_not_match_by_substring() {
        let text = format!("{HASH_A}  node-v20.11.1-linux-x64.tar.gz.sig\n");
        assert_eq!(parse_shasums(&text, "node-v20.11.1-linux-x64.tar.gz"), None);
        assert_eq!(parse_shasums(&text, "linux-x64.tar.gz"), None);
    }

    #[test]
    fn parse_shasums_rejects_malformed_hash() {
        let text = "not-a-hash  node.tar.gz\nabc  node.tar.gz\n";
        assert_eq!(parse_shasums(text, "node.tar.gz"), None);
    }

    #[test]
    fn parse_shasums_returns_none_when_missing() {
        let text = format!("{HASH_A}  other.tar.gz\n");
        assert_eq!(parse_shasums(&text, "node.tar.gz"), None);
    }
}
