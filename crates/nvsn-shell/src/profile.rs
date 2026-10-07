//! Structured handling of shell profile files.
//!
//! Parses the profile into header, nvsn-managed blocks and footer, modifies
//! the blocks and serializes it again. It avoids fragile text substitutions:
//! only markers that occupy a full line are recognized.

use crate::shell::quote;
use crate::shell::ShellError;
use std::fmt;
use std::path::Path;

/// A block managed by nvsn inside a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileBlock {
    /// Marker that identifies the block (e.g. `"# nvsn init"`).
    pub marker: String,
    /// Content lines, without the marker line.
    pub lines: Vec<String>,
}

impl ProfileBlock {
    /// Creates a block with its marker and its content lines.
    pub fn new(
        marker: impl Into<String>,
        lines: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            marker: marker.into(),
            lines: lines.into_iter().map(Into::into).collect(),
        }
    }
}

impl fmt::Display for ProfileBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.marker)?;
        for line in &self.lines {
            writeln!(f, "{line}")?;
        }
        Ok(())
    }
}

/// Parsed profile.
#[derive(Debug, Clone, Default)]
pub struct ShellProfile {
    /// Lines before the first nvsn block.
    pub header: Vec<String>,
    /// nvsn blocks, in order.
    pub blocks: Vec<ProfileBlock>,
    /// Lines after the last nvsn block.
    pub footer: Vec<String>,
}

/// Block markers known to nvsn.
pub const MARKERS: &[&str] = &["# nvsn init", "# nvsn wrapper", "# nvsn path"];

impl ShellProfile {
    /// Parses the content of a profile.
    pub fn parse(content: &str) -> Self {
        let mut profile = Self::default();
        let mut current_block: Option<ProfileBlock> = None;
        let mut in_nvsn_block = false;
        let mut header_done = false;

        for line in content.lines() {
            let trimmed = line.trim();

            // Markers are always written as a full line, so the comparison is exact. A
            // `starts_with` would capture user comments that start the same way (e.g. "# nvsn initially I used zsh").
            if let Some(marker) = MARKERS.iter().find(|m| trimmed == **m) {
                if let Some(block) = current_block.take() {
                    profile.blocks.push(block);
                }
                current_block = Some(ProfileBlock::new(*marker, std::iter::empty::<String>()));
                in_nvsn_block = true;
                header_done = true;
                continue;
            }

            if in_nvsn_block {
                // A blank line closes the block.
                if trimmed.is_empty() {
                    if let Some(block) = current_block.take() {
                        profile.blocks.push(block);
                    }
                    in_nvsn_block = false;
                    // The blank line is a separator: it is not part of the footer.
                    continue;
                }
                if let Some(ref mut block) = current_block {
                    block.lines.push(line.to_string());
                }
            } else if header_done {
                profile.footer.push(line.to_string());
            } else {
                profile.header.push(line.to_string());
            }
        }

        if let Some(block) = current_block {
            profile.blocks.push(block);
        }

        profile
    }

    /// Returns the block with that marker.
    pub fn get_block(&self, marker: &str) -> Option<&ProfileBlock> {
        self.blocks.iter().find(|b| b.marker == marker)
    }

    /// Adds a block or replaces the existing one with the same marker.
    pub fn set_block(&mut self, block: ProfileBlock) {
        if let Some(existing) = self.blocks.iter_mut().find(|b| b.marker == block.marker) {
            *existing = block;
        } else {
            self.blocks.push(block);
        }
    }

    /// `true` if the block exists and its content is exactly `expected_lines`.
    pub fn has_block_with_content(&self, marker: &str, expected_lines: &[String]) -> bool {
        self.get_block(marker)
            .is_some_and(|b| b.lines == expected_lines)
    }

    fn write_to_string<W: fmt::Write>(&self, f: &mut W) -> fmt::Result {
        for line in &self.header {
            writeln!(f, "{line}")?;
        }

        let mut wrote_something = !self.header.is_empty();
        for (i, block) in self.blocks.iter().enumerate() {
            if i > 0 || !self.header.is_empty() {
                writeln!(f)?;
            }
            write!(f, "{block}")?;
            wrote_something = true;
        }

        if !self.footer.is_empty() {
            if wrote_something {
                writeln!(f)?;
            }
            for line in &self.footer {
                writeln!(f, "{line}")?;
            }
        }

        Ok(())
    }
}

impl fmt::Display for ShellProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_to_string(f)
    }
}

fn io_error(path: &Path, source: std::io::Error) -> ShellError {
    ShellError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Loads a profile from disk. If it does not exist, returns an empty one.
///
/// # Errors
///
/// Returns [`ShellError::Io`] if the file exists and cannot be read.
pub fn load_profile(path: &Path) -> Result<ShellProfile, ShellError> {
    if path.exists() {
        let content = std::fs::read_to_string(path).map_err(|e| io_error(path, e))?;
        Ok(ShellProfile::parse(&content))
    } else {
        Ok(ShellProfile::default())
    }
}

/// Saves a profile to disk, creating the parent directories if needed.
///
/// # Errors
///
/// Returns [`ShellError::Io`] if the directory cannot be created or the file written.
pub fn save_profile(path: &Path, profile: &ShellProfile) -> Result<(), ShellError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
    }
    std::fs::write(path, profile.to_string()).map_err(|e| io_error(path, e))
}

/// Path of `path` relative to `home` as a POSIX word `"$HOME/..."`, with the
/// rest escaped for double quotes. If `path` is not inside `home`,
/// returns the absolute path in single quotes.
pub fn home_relative(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rel) => {
            // Always the `/` separator: the word is read by a POSIX shell, also on Windows.
            let parts: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            format!("\"$HOME/{}\"", quote::posix_dq_body(&parts.join("/")))
        }
        Err(_) => quote::posix(&path.display().to_string()),
    }
}

/// Updates the `# nvsn path` block of the login profile with a static line
/// that points to the `<root>/current/bin` link (ADR-034). The link is moved by
/// `nvsn default`, so the profile does not change when the version changes and
/// graphical applications always see the global version.
/// Returns `Ok(true)` if the file changed. It replaces the old blocks that
/// read `<root>/default` on every startup.
///
/// # Errors
///
/// Returns [`ShellError::Io`] if the file cannot be read or written.
pub fn update_path_block(path: &Path, home: &Path, root: &Path) -> Result<bool, ShellError> {
    const MARKER: &str = "# nvsn path";
    let expected_lines = vec![path_export_line(&root.join("current").join("bin"), home)];

    let mut profile = load_profile(path)?;
    let changed = !profile.has_block_with_content(MARKER, &expected_lines);
    if changed {
        profile.set_block(ProfileBlock::new(MARKER, expected_lines));
        save_profile(path, &profile)?;
    }
    Ok(changed)
}

/// `export PATH="..."` line for `bin`. Inside `home` it uses `$HOME`, like the
/// rest of the blocks; outside `home`, the absolute path escaped for quotes.
fn path_export_line(bin: &Path, home: &Path) -> String {
    match bin.strip_prefix(home) {
        Ok(rel) => {
            // Always the `/` separator: the line is read by a POSIX shell.
            let parts: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            format!(
                "export PATH=\"$HOME/{}:$PATH\"",
                quote::posix_dq_body(&parts.join("/"))
            )
        }
        Err(_) => format!(
            "export PATH=\"{}:$PATH\"",
            quote::posix_dq_body(&bin.display().to_string())
        ),
    }
}

/// Removes all blocks managed by nvsn. Returns `Ok(true)` if the file changed.
///
/// # Errors
///
/// Returns [`ShellError::Io`] if the file cannot be read or written.
pub fn strip_nvsn_blocks(path: &Path) -> Result<bool, ShellError> {
    if !path.exists() {
        return Ok(false);
    }
    let mut profile = load_profile(path)?;
    let changed = !profile.blocks.is_empty();
    if changed {
        profile.blocks.clear();
        save_profile(path, &profile)?;
    }
    Ok(changed)
}

/// Ensures the profile has the `# nvsn init` and `# nvsn wrapper` blocks
/// with the expected content. Returns `Ok(true)` if the file changed.
///
/// # Errors
///
/// Returns [`ShellError::Io`] if the file cannot be read or written.
pub fn ensure_profile(
    path: &Path,
    init_content: &str,
    wrapper_content: &str,
) -> Result<bool, ShellError> {
    let mut profile = load_profile(path)?;

    let expected_init: Vec<String> = init_content.lines().map(String::from).collect();
    let expected_wrapper: Vec<String> = wrapper_content.lines().map(String::from).collect();

    let mut modified = false;

    if !profile.has_block_with_content("# nvsn init", &expected_init) {
        profile.set_block(ProfileBlock::new("# nvsn init", expected_init));
        modified = true;
    }

    if !profile.has_block_with_content("# nvsn wrapper", &expected_wrapper) {
        profile.set_block(ProfileBlock::new("# nvsn wrapper", expected_wrapper));
        modified = true;
    }

    if modified {
        save_profile(path, &profile)?;
    }

    Ok(modified)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn parse_empty() {
        let profile = ShellProfile::parse("");
        assert!(profile.header.is_empty());
        assert!(profile.blocks.is_empty());
        assert!(profile.footer.is_empty());
    }

    #[test]
    fn parse_header_only() {
        let profile = ShellProfile::parse("export FOO=bar\nalias ll='ls -la'\n");
        assert_eq!(profile.header.len(), 2);
        assert!(profile.blocks.is_empty());
    }

    #[test]
    fn parse_single_block() {
        let profile = ShellProfile::parse("# nvsn init\neval \"$(nvsn env --shell bash)\"\n");
        assert_eq!(profile.blocks.len(), 1);
        assert_eq!(profile.blocks[0].marker, "# nvsn init");
        assert_eq!(
            profile.blocks[0].lines,
            vec!["eval \"$(nvsn env --shell bash)\""]
        );
    }

    #[test]
    fn parse_ignores_user_comment_with_marker_prefix() {
        // A user comment that only starts the same way as a marker must stay
        // in the header or footer, not become a block.
        let content = "# nvsn initially I used zsh\nexport FOO=bar\n";
        let profile = ShellProfile::parse(content);
        assert!(profile.blocks.is_empty());
        assert_eq!(
            profile.header,
            vec![
                "# nvsn initially I used zsh".to_string(),
                "export FOO=bar".to_string(),
            ]
        );
    }

    #[test]
    fn parse_multiple_blocks() {
        let content = r#"# user config
# nvsn init
eval "$(nvsn env --shell bash)"

# nvsn wrapper
nvsn() { command nvsn "$@"; }

# more config
"#;
        let profile = ShellProfile::parse(content);
        assert_eq!(profile.header.len(), 1);
        assert_eq!(profile.blocks.len(), 2);
        assert_eq!(profile.blocks[0].marker, "# nvsn init");
        assert_eq!(profile.blocks[1].marker, "# nvsn wrapper");
        assert_eq!(profile.footer.len(), 1);
    }

    #[test]
    fn block_replacement() {
        let mut profile = ShellProfile::parse("# nvsn init\nold content\n");
        profile.set_block(ProfileBlock::new(
            "# nvsn init",
            vec!["new content".to_string()],
        ));
        assert_eq!(profile.blocks[0].lines, vec!["new content"]);
    }

    #[test]
    fn serialization_roundtrip() {
        let content = r#"# header
# nvsn init
eval "$(nvsn env --shell bash)"

# nvsn wrapper
nvsn() { command nvsn "$@"; }

# footer
"#;
        let profile = ShellProfile::parse(content);
        let serialized = profile.to_string();
        let reparsed = ShellProfile::parse(&serialized);
        assert_eq!(profile.blocks.len(), reparsed.blocks.len());
        assert!(reparsed.header.starts_with(&["# header".to_string()]));
        assert_eq!(profile.footer, reparsed.footer);
    }

    #[test]
    fn file_operations() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("profile");
        fs::write(&path, "# header\n# nvsn init\nold\n").unwrap();

        let mut profile = load_profile(&path).unwrap();
        assert_eq!(profile.blocks.len(), 1);
        profile.set_block(ProfileBlock::new("# nvsn init", vec!["new".to_string()]));
        save_profile(&path, &profile).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("new"));
        assert!(!content.contains("old"));
    }

    #[test]
    fn home_relative_uses_home_variable_inside_home() {
        let home = Path::new("/home/jhon");
        let inside = home.join(".nvsn").join("bin");
        assert_eq!(home_relative(&inside, home), "\"$HOME/.nvsn/bin\"");
        assert_eq!(home_relative(Path::new("/opt/x"), home), "'/opt/x'");
    }

    #[test]
    fn update_path_block_adds_block_to_new_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("profile");

        let root = dir.path().join(".nvsn");
        assert!(update_path_block(&path, dir.path(), &root).unwrap());

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("# nvsn path"));
        assert!(content.contains("export PATH=\"$HOME/.nvsn/current/bin:$PATH\""));
        assert!(
            !content.contains("default"),
            "does not read the default file"
        );
    }

    #[test]
    fn update_path_block_uses_absolute_path_outside_home() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("profile");
        let home = dir.path().join("home");
        let root = dir.path().join("data").join("nvsn");

        update_path_block(&path, &home, &root).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let bin = root.join("current").join("bin").display().to_string();
        let expected = format!("export PATH=\"{}:$PATH\"", quote::posix_dq_body(&bin));
        assert!(content.contains(&expected), "{content}");
    }

    #[test]
    fn update_path_block_migrates_the_old_default_block() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("profile");
        fs::write(
            &path,
            "# user\nexport FOO=bar\n\n# nvsn path\n_nvsn_default=\"$(cat \"$HOME/.nvsn\"/default 2>/dev/null)\"\n[ -n \"$_nvsn_default\" ] && export PATH=x\nunset _nvsn_default\n",
        )
        .unwrap();

        let root = dir.path().join(".nvsn");
        assert!(update_path_block(&path, dir.path(), &root).unwrap());

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("export FOO=bar"));
        assert!(!content.contains("_nvsn_default"));
        assert_eq!(content.matches("# nvsn path").count(), 1);
        assert!(content.contains("export PATH=\"$HOME/.nvsn/current/bin:$PATH\""));
    }

    #[test]
    fn update_path_block_is_idempotent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("profile");

        let root = dir.path().join(".nvsn");
        assert!(update_path_block(&path, dir.path(), &root).unwrap());
        assert!(!update_path_block(&path, dir.path(), &root).unwrap());

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content.matches("# nvsn path").count(), 1);
    }

    #[test]
    fn update_path_block_replaces_when_root_changes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("profile");
        let old_root = dir.path().join("old");
        let new_root = dir.path().join("new");

        update_path_block(&path, dir.path(), &old_root).unwrap();
        assert!(update_path_block(&path, dir.path(), &new_root).unwrap());

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("$HOME/new/current/bin"));
        assert!(!content.contains("old"));
    }

    #[test]
    fn update_path_block_preserves_existing_content() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("profile");
        fs::write(&path, "# my custom profile\nexport FOO=bar\n").unwrap();

        update_path_block(&path, dir.path(), &dir.path().join("bin")).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("export FOO=bar"));
        assert!(content.contains("# nvsn path"));
    }

    #[test]
    fn ensure_profile_creates_both_blocks_on_new_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("profile");

        let modified = ensure_profile(
            &path,
            "eval \"$(nvsn env --shell bash)\"",
            "nvsn() { command nvsn \"$@\"; }",
        )
        .unwrap();
        assert!(modified);

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("# nvsn init"));
        assert!(content.contains("eval \"$(nvsn env --shell bash)\""));
        assert!(content.contains("# nvsn wrapper"));
        assert!(content.contains("nvsn() { command nvsn \"$@\"; }"));
    }

    #[test]
    fn ensure_profile_is_idempotent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("profile");
        let init = "eval \"$(nvsn env --shell bash)\"";
        let wrapper = "nvsn() { command nvsn \"$@\"; }";

        assert!(ensure_profile(&path, init, wrapper).unwrap());
        assert!(!ensure_profile(&path, init, wrapper).unwrap());
    }

    #[test]
    fn ensure_profile_updates_stale_content() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("profile");
        let old_wrapper = "nvsn() { command nvsn \"$@\"; }";
        let new_wrapper = "nvsn() { command nvsn \"$@\"; case \"$1\" in use) nvsn env;; esac; }";

        ensure_profile(&path, "eval init", old_wrapper).unwrap();
        assert!(ensure_profile(&path, "eval init", new_wrapper).unwrap());

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains(new_wrapper));
        assert!(!content.contains(old_wrapper));
    }

    #[test]
    fn strip_profile_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("profile");
        // The blank line before the footer makes the parser treat it as footer.
        fs::write(
            &path,
            "# header\n# nvsn init\ncontent\n\n# nvsn wrapper\nmore\n\n# footer\n",
        )
        .unwrap();

        assert!(strip_nvsn_blocks(&path).unwrap());

        let content = fs::read_to_string(&path).unwrap();
        assert!(!content.contains("# nvsn init"));
        assert!(!content.contains("# nvsn wrapper"));
        assert!(content.contains("# header"));
        assert!(content.contains("# footer"));
    }
}
