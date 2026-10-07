//! Model and parser for `https://nodejs.org/dist/index.json`.
//!
//! This module only parses a JSON body that has already been downloaded. It does no
//! networking. Unknown fields (`v8`, `uv`, `zlib`, `openssl`, `modules`...) are
//! ignored.
//!
//! The index order is not by date, so [`parse_index`] sorts the
//! releases by semver version in descending order.

use anyhow::{bail, Context, Result};
use semver::Version;
use serde::Deserialize;

/// `lts` field of the index: `false` (not LTS) or the codename of the LTS line.
///
/// A `true` does not appear in the real index, but it is accepted as LTS without
/// a codename so that a future index is not rejected.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum LtsField {
    /// Not LTS (`false`) or, if `true`, LTS without a codename.
    Flag(bool),
    /// Codename of the LTS line, e.g. `Hydrogen`. Case-insensitive.
    Codename(String),
}

impl LtsField {
    /// Indicates whether the release belongs to an LTS line.
    pub fn is_lts(&self) -> bool {
        match self {
            LtsField::Flag(flag) => *flag,
            LtsField::Codename(_) => true,
        }
    }

    /// LTS codename, if there is one.
    pub fn codename(&self) -> Option<&str> {
        match self {
            LtsField::Codename(name) => Some(name.as_str()),
            LtsField::Flag(_) => None,
        }
    }
}

/// One entry of the Node distribution index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRelease {
    /// Semver version, without the `v` prefix used by the index.
    pub version: Version,
    /// Publication date as it appears in the index (`YYYY-MM-DD`).
    pub date: String,
    /// Platform tokens of the artifacts (`linux-x64`, `osx-arm64-tar`...).
    pub files: Vec<String>,
    /// Bundled npm version, if the index includes it.
    pub npm: Option<String>,
    /// LTS line of the release.
    pub lts: LtsField,
    /// Indicates whether the release is a security release.
    pub security: bool,
}

impl RemoteRelease {
    /// Returns the version label with the `v` prefix (`v20.11.1`).
    pub fn tag(&self) -> String {
        format!("v{}", self.version)
    }

    /// Indicates whether the index lists an artifact with that platform token.
    pub fn has_file(&self, token: &str) -> bool {
        self.files.iter().any(|f| f == token)
    }
}

/// Raw shape of a JSON entry, before the version is validated.
#[derive(Debug, Deserialize)]
struct RawRelease {
    version: String,
    date: String,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    npm: Option<String>,
    lts: LtsField,
    #[serde(default)]
    security: bool,
}

/// Parses the JSON body of `index.json` and returns the releases sorted
/// by semver version, from highest to lowest.
///
/// # Errors
///
/// Returns an error if the JSON does not have the expected shape, or if any
/// entry has a `version` that is not valid semver (with or without the `v` prefix).
pub fn parse_index(body: &str) -> Result<Vec<RemoteRelease>> {
    let raw: Vec<RawRelease> =
        serde_json::from_str(body).context("index.json does not have the expected format")?;

    let mut releases = Vec::with_capacity(raw.len());
    for entry in raw {
        let version = parse_version_tag(&entry.version)?;
        releases.push(RemoteRelease {
            version,
            date: entry.date,
            files: entry.files,
            npm: entry.npm,
            lts: entry.lts,
            security: entry.security,
        });
    }
    releases.sort_by(|a, b| b.version.cmp(&a.version));
    Ok(releases)
}

/// Converts `v20.11.1` (or `20.11.1`) into a [`Version`].
fn parse_version_tag(tag: &str) -> Result<Version> {
    let bare = tag.strip_prefix('v').unwrap_or(tag);
    match Version::parse(bare) {
        Ok(version) => Ok(version),
        Err(_) => bail!("invalid version in index.json: {tag:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small fixture with the real shape of the index (fields trimmed).
    const FIXTURE: &str = r#"[
        {"version":"v22.11.0","date":"2024-10-25","files":["linux-x64","osx-arm64-tar","win-x64-zip"],
         "npm":"10.9.0","v8":"12.4","uv":null,"zlib":"1.3","openssl":"3.0.15","modules":"127",
         "lts":"Jod","security":false},
        {"version":"v23.0.0","date":"2024-10-16","files":["linux-x64","src"],
         "npm":null,"lts":false,"security":false},
        {"version":"v20.11.1","date":"2024-02-14","files":["linux-x64","headers"],
         "npm":"10.2.4","lts":"Iron","security":true},
        {"version":"v18.20.8","date":"2025-03-27","files":["linux-x64","win-x86-zip"],
         "npm":"10.8.2","lts":"Hydrogen","security":false},
        {"version":"v24.0.0-rc.1","date":"2025-04-01","files":["linux-x64"],
         "npm":"11.0.0","lts":false,"security":false}
    ]"#;

    #[test]
    fn parses_fixture_and_sorts_descending() {
        let releases = parse_index(FIXTURE).unwrap();
        let versions: Vec<String> = releases.iter().map(|r| r.tag()).collect();
        assert_eq!(
            versions,
            vec![
                "v24.0.0-rc.1",
                "v23.0.0",
                "v22.11.0",
                "v20.11.1",
                "v18.20.8"
            ]
        );
    }

    #[test]
    fn lts_field_accepts_false_and_codename() {
        let releases = parse_index(FIXTURE).unwrap();
        let jod = releases.iter().find(|r| r.version.major == 22).unwrap();
        assert_eq!(jod.lts.codename(), Some("Jod"));
        assert!(jod.lts.is_lts());

        let twenty_three = releases.iter().find(|r| r.version.major == 23).unwrap();
        assert_eq!(twenty_three.lts, LtsField::Flag(false));
        assert!(!twenty_three.lts.is_lts());
        assert_eq!(twenty_three.lts.codename(), None);
    }

    #[test]
    fn keeps_files_security_and_npm_and_ignores_extra_fields() {
        let releases = parse_index(FIXTURE).unwrap();
        let iron = releases.iter().find(|r| r.version.major == 20).unwrap();
        assert!(iron.security);
        assert_eq!(iron.npm.as_deref(), Some("10.2.4"));
        assert!(iron.has_file("headers"));
        assert!(!iron.has_file("osx-x64-tar"));

        let v23 = releases.iter().find(|r| r.version.major == 23).unwrap();
        assert_eq!(v23.npm, None);
        assert!(!v23.security);
    }

    #[test]
    fn rejects_bad_version_and_bad_shape() {
        let bad_version = r#"[{"version":"vx.y","date":"2024-01-01","files":[],"lts":false}]"#;
        assert!(parse_index(bad_version).is_err());

        let bad_shape = r#"{"version":"v20.0.0"}"#;
        assert!(parse_index(bad_shape).is_err());
    }

    #[test]
    fn empty_index_is_empty_list() {
        assert!(parse_index("[]").unwrap().is_empty());
    }
}
