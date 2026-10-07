//! `.nvmrc` and `.node-version` version files (ADR-023, ARCHITECTURE 4.4).
//!
//! Strict format: exactly one useful value per file. A UTF-8 BOM, CRLF and LF,
//! blank lines, spaces and comments starting with `#` are tolerated. An empty
//! file, a file with several values, or a file with `key=value` is an error.
//!
//! The search walks up from a directory to the root, or up to `stop_at` if given
//! (e.g. the home directory). Within each directory, `.nvmrc` takes precedence
//! over `.node-version`.

use crate::spec::{parse_spec, VersionSpec};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// File names searched in each directory, in order of precedence.
pub const VERSION_FILE_NAMES: [&str; 2] = [".nvmrc", ".node-version"];

/// Version file found, with its content already interpreted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionFile {
    /// Path of the file that was read.
    pub path: PathBuf,
    /// Value as it appears in the file, without comments or spaces.
    pub raw: String,
    /// Specification interpreted from `raw`.
    pub spec: VersionSpec,
}

/// Extracts the only useful value from the text of a version file.
///
/// # Errors
///
/// Returns an error if there is no value, if there is more than one, or if the
/// value is `key=value` (ADR-023: keys are not supported).
pub fn parse_version_content(content: &str) -> Result<String> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);

    let mut values: Vec<&str> = Vec::new();
    for line in content.lines() {
        let without_comment = line.split('#').next().unwrap_or_default();
        let value = without_comment.trim();
        if !value.is_empty() {
            values.push(value);
        }
    }

    match values.as_slice() {
        [] => bail!("the version file contains no version"),
        [single] => {
            if let Some((key, _)) = single.split_once('=') {
                if is_key(key) {
                    bail!(
                        "{single:?}: key/value pairs in version files are not supported (ADR-023); write only the version"
                    );
                }
            }
            Ok((*single).to_string())
        }
        many => bail!(
            "the version file has {} lines with a value; it must have exactly one",
            many.len()
        ),
    }
}

/// Indicates whether a string has the form of a key (`node`, `lts_name`, `a-b`).
fn is_key(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Reads and parses a specific version file.
///
/// # Errors
///
/// Returns an error if the file cannot be read (including invalid UTF-8),
/// if its content is not valid, or if the value is not a recognized
/// specification. The message includes the path.
pub fn read_version_file(path: &Path) -> Result<VersionFile> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let raw = parse_version_content(&content).with_context(|| path.display().to_string())?;
    let spec = parse_spec(&raw).with_context(|| path.display().to_string())?;
    Ok(VersionFile {
        path: path.to_path_buf(),
        raw,
        spec,
    })
}

/// Finds the first version file by walking up from `start`.
///
/// Walks `start` and its ancestors. In each directory it tries
/// [`VERSION_FILE_NAMES`] in order. If `stop_at` is `Some(dir)`, the walk ends
/// after checking that directory (ADR-023: the home directory is included). If
/// it is `None`, it walks up to the root. A file found with invalid content is
/// an error: the search does not skip to an ancestor.
///
/// `start` must be absolute. Comparisons with `stop_at` are lexical.
///
/// # Errors
///
/// Returns an error if the file found is not valid (see
/// [`read_version_file`]).
pub fn find_version_file(start: &Path, stop_at: Option<&Path>) -> Result<Option<VersionFile>> {
    for dir in start.ancestors() {
        for name in VERSION_FILE_NAMES {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return read_version_file(&candidate).map(Some);
            }
        }
        if stop_at.is_some_and(|stop| stop == dir) {
            break;
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(content: &str) -> String {
        parse_version_content(content).unwrap()
    }

    #[test]
    fn accepts_lf_crlf_and_bom() {
        assert_eq!(value("v18.17.0\n"), "v18.17.0");
        assert_eq!(value("v18.17.0\r\n"), "v18.17.0");
        assert_eq!(value("\u{feff}18.17.0\r\n"), "18.17.0");
        assert_eq!(value("18.17.0"), "18.17.0");
    }

    #[test]
    fn ignores_blank_lines_spaces_and_comments() {
        assert_eq!(value("\n\n   lts/hydrogen   \n\n"), "lts/hydrogen");
        assert_eq!(value("# project X\n\n  20  # LTS\n"), "20");
        assert_eq!(value("# comment only\n18 # inline\n"), "18");
    }

    #[test]
    fn accepts_each_supported_value_form() {
        for raw in ["v18.17.0", "18", "lts/*", "lts/hydrogen", "node", "system"] {
            assert_eq!(value(&format!("{raw}\n")), raw);
        }
    }

    #[test]
    fn empty_or_comment_only_is_error() {
        assert!(parse_version_content("").is_err());
        assert!(parse_version_content("\n  \n# nothing\n").is_err());
        assert!(parse_version_content("\u{feff}").is_err());
    }

    #[test]
    fn several_values_are_error() {
        let err = parse_version_content("18\n20\n").unwrap_err().to_string();
        assert!(err.contains("2 lines"));
    }

    #[test]
    fn key_value_is_error_but_ranges_are_not_keys() {
        assert!(parse_version_content("node=18\n").is_err());
        assert!(parse_version_content("version = 18\n").is_err());
        assert_eq!(value(">=18\n"), ">=18");
    }

    #[test]
    fn read_file_parses_spec_and_keeps_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".nvmrc");
        std::fs::write(&path, "\u{feff}lts/hydrogen\r\n").unwrap();

        let file = read_version_file(&path).unwrap();
        assert_eq!(file.path, path);
        assert_eq!(file.raw, "lts/hydrogen");
        assert_eq!(file.spec, VersionSpec::LtsCodename("hydrogen".to_string()));
    }

    #[test]
    fn read_file_rejects_invalid_spec_with_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".nvmrc");
        std::fs::write(&path, "iojs\n").unwrap();

        let err = format!("{:#}", read_version_file(&path).unwrap_err());
        assert!(err.contains("io.js"), "{err}");
        assert!(err.contains(".nvmrc"), "{err}");
    }

    #[test]
    fn find_walks_up_and_returns_the_first_file() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let deep = project.join("src").join("module");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(project.join(".node-version"), "20\n").unwrap();

        let found = find_version_file(&deep, None).unwrap().unwrap();
        assert_eq!(found.path, project.join(".node-version"));
        assert_eq!(
            found.spec,
            VersionSpec::Partial {
                major: 20,
                minor: None
            }
        );
    }

    #[test]
    fn nvmrc_wins_over_node_version_in_the_same_directory() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(".node-version"), "18\n").unwrap();
        std::fs::write(root.path().join(".nvmrc"), "20\n").unwrap();

        let found = find_version_file(root.path(), None).unwrap().unwrap();
        assert_eq!(found.path, root.path().join(".nvmrc"));
    }

    #[test]
    fn nearest_directory_wins_over_ancestors() {
        let root = tempfile::tempdir().unwrap();
        let child = root.path().join("child");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(root.path().join(".nvmrc"), "18\n").unwrap();
        std::fs::write(child.join(".nvmrc"), "22\n").unwrap();

        let found = find_version_file(&child, None).unwrap().unwrap();
        assert_eq!(found.raw, "22");
    }

    #[test]
    fn stop_at_excludes_directories_above_it() {
        let root = tempfile::tempdir().unwrap();
        let stop = root.path().join("home");
        let work = stop.join("work");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(root.path().join(".nvmrc"), "18\n").unwrap();

        assert!(find_version_file(&work, Some(&stop)).unwrap().is_none());
        // Control: with a stop further up, the same file is found.
        assert!(find_version_file(&work, Some(root.path()))
            .unwrap()
            .is_some());
    }

    #[test]
    fn stop_at_includes_the_stop_directory_itself() {
        let root = tempfile::tempdir().unwrap();
        let stop = root.path().join("home");
        let work = stop.join("work");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(stop.join(".nvmrc"), "lts/*\n").unwrap();

        let found = find_version_file(&work, Some(&stop)).unwrap().unwrap();
        assert_eq!(found.path, stop.join(".nvmrc"));
    }

    #[test]
    fn invalid_closest_file_is_error_not_skipped() {
        let root = tempfile::tempdir().unwrap();
        let child = root.path().join("child");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(root.path().join(".nvmrc"), "18\n").unwrap();
        std::fs::write(child.join(".nvmrc"), "\n").unwrap();

        assert!(find_version_file(&child, None).is_err());
    }

    #[test]
    fn no_file_is_none() {
        let root = tempfile::tempdir().unwrap();
        let stop = root.path();
        let work = root.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        assert!(find_version_file(&work, Some(stop)).unwrap().is_none());
    }
}
