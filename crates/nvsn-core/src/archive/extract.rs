//! Extraction of `.tar.gz` and `.zip` archives with limits and path checks.
//!
//! Adapted from gvsn (MIT, same author). The progress spinner was removed:
//! presentation lives in `nvsn-cli` (ADR-011).
//!
//! Protection (ARCHITECTURE 6, R-05, R-06, ADR-019):
//! - Limits on decompressed bytes and entry count, configurable with
//!   [`ExtractLimits`]. By default, 2 GiB and 100000 entries.
//! - Absolute paths and `..` components are rejected.
//! - Symbolic and hard links must point inside the destination. No entry
//!   may be written through a link that has already been extracted.
//! - Symbolic links in zip archives are rejected (Node's zip files do not contain them).

use anyhow::{anyhow, bail, Context, Result};
use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

/// Maximum number of links followed when resolving a path (prevents loops).
const MAX_LINK_HOPS: usize = 255;

/// Default maximum decompressed bytes: 2 GiB.
pub const DEFAULT_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Default maximum number of entries.
pub const DEFAULT_MAX_ENTRIES: u64 = 100_000;

/// Limits of an extraction. They protect against zip bombs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractLimits {
    /// Total decompressed bytes accepted. For tar, this also counts the headers
    /// and the final block.
    pub max_bytes: u64,
    /// Number of entries (files, directories and links) accepted.
    pub max_entries: u64,
}

impl Default for ExtractLimits {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_MAX_BYTES,
            max_entries: DEFAULT_MAX_ENTRIES,
        }
    }
}

/// Extracts `archive` into the `dest` directory with [`ExtractLimits::default`].
///
/// The format is determined by the file extension:
/// - `.tar.gz`: decompression with `flate2` and reading with `tar`.
/// - `.zip`: entry-by-entry extraction with the `zip` crate.
/// - `.tar.xz`: rejected for now (there is no xz backend, ADR-006).
///
/// # Errors
///
/// Returns an error if the extension is not valid, the file is malformed,
/// it exceeds the limits, it contains unsafe paths or links, or `dest`
/// cannot be written to.
pub fn unpack(archive: &Path, dest: &Path) -> Result<()> {
    unpack_with_limits(archive, dest, ExtractLimits::default())
}

/// Like [`unpack`], but with explicit limits.
///
/// # Errors
///
/// The same as [`unpack`]. It also returns an error if `limits` are exceeded.
pub fn unpack_with_limits(archive: &Path, dest: &Path, limits: ExtractLimits) -> Result<()> {
    let name = archive
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();

    if name.ends_with(".tar.gz") {
        unpack_tar_gz(archive, dest, limits)
    } else if name.ends_with(".zip") {
        unpack_zip(archive, dest, limits)
    } else if name.ends_with(".tar.xz") {
        bail!("unsupported archive format: .tar.xz (use .tar.gz or .zip)")
    } else {
        bail!("unsupported archive format: {name} (use .tar.gz or .zip)")
    }
}

/// Reader that fails as soon as more than `max` bytes are read.
///
/// It is placed on top of the decompressor, so it counts decompressed bytes.
struct LimitedReader<R> {
    inner: R,
    max: u64,
    read: u64,
}

impl<R: Read> Read for LimitedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read = self.read.saturating_add(n as u64);
        if self.read > self.max {
            return Err(io::Error::other(format!(
                "archive exceeds extraction limit of {} bytes",
                self.max
            )));
        }
        Ok(n)
    }
}

/// Extracts a `.tar.gz` into `dest`.
///
/// The gzip compression is decompressed on the fly; no uncompressed temporary file
/// is written.
///
/// After all entries are extracted, each link is resolved component by component
/// against the final tree. This detects chains such as `x -> y/..`
/// followed by `y -> ..`, which each individual check accepted. If the
/// extraction fails in that phase, `dest` may contain the part already written;
/// the caller must use a temporary directory.
///
/// # Errors
///
/// Returns an error if the file cannot be opened, is malformed, exceeds
/// the limits, or contains unsafe entries.
fn unpack_tar_gz(archive: &Path, dest: &Path, limits: ExtractLimits) -> Result<()> {
    let file =
        File::open(archive).with_context(|| format!("Failed to open {}", archive.display()))?;
    let reader = LimitedReader {
        inner: flate2::read::GzDecoder::new(file),
        max: limits.max_bytes,
        read: 0,
    };
    let mut tar = tar::Archive::new(reader);

    let mut links: Vec<PathBuf> = Vec::new();
    let mut count = 0u64;
    for entry in tar.entries().context("Failed to read tar archive")? {
        let mut entry = entry.context("Failed to read tar entry")?;
        count += 1;
        if count > limits.max_entries {
            bail!(
                "archive exceeds extraction limit of {} entries",
                limits.max_entries
            );
        }
        if let Some(resolve) = check_tar_entry(&entry, dest)? {
            links.push(resolve);
        }
        let extracted = entry
            .unpack_in(dest)
            .context("Failed to extract tar entry")?;
        if !extracted {
            bail!("unsafe entry rejected by the tar reader");
        }
    }

    // With the tree already written, each link is followed to its real target.
    for link in &links {
        ensure_resolves_within(dest, link)?;
    }
    Ok(())
}

/// Validates a tar entry before writing it.
///
/// If the entry is a link, returns the path (relative to `dest`) that must be
/// resolved at the end of the extraction.
fn check_tar_entry<R: Read>(entry: &tar::Entry<'_, R>, dest: &Path) -> Result<Option<PathBuf>> {
    let path = entry
        .path()
        .context("invalid path in tar entry")?
        .into_owned();
    check_relative_path(&path)?;
    ensure_no_symlink_ancestors(dest, &path)?;

    let kind = entry.header().entry_type();
    if kind.is_symlink() {
        let target = link_target(entry, &path)?;
        let base = path.parent().unwrap_or_else(|| Path::new(""));
        check_link_target(base, &target)?;
        Ok(Some(base.join(&target)))
    } else if kind.is_hard_link() {
        let target = link_target(entry, &path)?;
        check_link_target(Path::new(""), &target)?;
        Ok(Some(target))
    } else {
        Ok(None)
    }
}

/// One component of an already decomposed link path.
enum PathPart {
    /// Normal directory or file name.
    Name(OsString),
    /// `..` component.
    Parent,
}

/// Decomposes `path` into components. Rejects roots and prefixes.
fn to_parts(path: &Path) -> Result<Vec<PathPart>> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => parts.push(PathPart::Name(name.to_os_string())),
            Component::CurDir => {}
            Component::ParentDir => parts.push(PathPart::Parent),
            Component::RootDir | Component::Prefix(_) => {
                bail!("absolute link target: {}", path.display())
            }
        }
    }
    Ok(parts)
}

/// Resolves `rel` (relative to `dest`) against the current state of the disk and
/// checks that the result is still inside `dest`.
///
/// Each component is looked up on disk. If it is a link, it is replaced by its
/// target and resolution continues from there. Unlike [`check_link_target`],
/// which only sees the links that already exist when the link is created, this
/// check is done against the final tree: a link created later can change the
/// resolution of an earlier one.
fn ensure_resolves_within(dest: &Path, rel: &Path) -> Result<()> {
    let mut pending: VecDeque<PathPart> = to_parts(rel)?.into();
    let mut resolved = PathBuf::new();
    let mut hops = 0usize;

    while let Some(part) = pending.pop_front() {
        match part {
            PathPart::Parent => {
                if !resolved.pop() {
                    bail!("link escapes the destination: {}", rel.display());
                }
            }
            PathPart::Name(name) => {
                resolved.push(name);
                let on_disk = dest.join(&resolved);
                if is_symlink(&on_disk) {
                    hops += 1;
                    if hops > MAX_LINK_HOPS {
                        bail!("too many symlinks while resolving {}", rel.display());
                    }
                    let target = std::fs::read_link(&on_disk)
                        .with_context(|| format!("Failed to read link {}", on_disk.display()))?;
                    resolved.pop();
                    for part in to_parts(&target)?.into_iter().rev() {
                        pending.push_front(part);
                    }
                }
            }
        }
    }
    Ok(())
}

/// Returns the target of a tar link, or an error if it has none.
fn link_target<R: Read>(entry: &tar::Entry<'_, R>, path: &Path) -> Result<PathBuf> {
    entry
        .link_name()
        .context("invalid link name in tar entry")?
        .map(|t| t.into_owned())
        .ok_or_else(|| anyhow!("link entry {} has no target", path.display()))
}

/// Rejects absolute paths and `..` components.
fn check_relative_path(path: &Path) -> Result<()> {
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => {
                bail!("unsafe path in archive (contains ..): {}", path.display())
            }
            Component::RootDir | Component::Prefix(_) => {
                bail!("unsafe absolute path in archive: {}", path.display())
            }
        }
    }
    Ok(())
}

/// Checks that no ancestor directory of `rel` (inside `dest`) is already a
/// symbolic link. Prevents writing through a link.
fn ensure_no_symlink_ancestors(dest: &Path, rel: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    let components: Vec<&std::ffi::OsStr> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(name) => Some(name),
            _ => None,
        })
        .collect();
    for name in components.iter().take(components.len().saturating_sub(1)) {
        current.push(name);
        if is_symlink(&dest.join(&current)) {
            bail!(
                "entry {} would be written through the symlink {}",
                rel.display(),
                current.display()
            );
        }
    }
    Ok(())
}

/// Checks lexically that a link with target `target` does not leave `dest`:
/// rejects absolute paths and `..` that climb above the root of the extraction.
///
/// `base` is the directory of the link, relative to `dest`. The actual resolution
/// of link chains (`a -> b -> ..`) is done by [`ensure_resolves_within`] on the
/// final tree, because a link created later can change an earlier one.
///
/// # Errors
///
/// Returns an error if the path is absolute or if it leaves `dest`.
fn check_link_target(base: &Path, target: &Path) -> Result<()> {
    let mut depth = base.components().count();
    for component in target.components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir => {
                if depth == 0 {
                    bail!("link target escapes the destination: {}", target.display());
                }
                depth -= 1;
            }
            Component::RootDir | Component::Prefix(_) => {
                bail!("absolute link target: {}", target.display());
            }
        }
    }
    Ok(())
}

/// Indicates whether `path` exists and is a symbolic link. Does not follow links.
fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// Extracts a `.zip` into `dest`, entry by entry, with limits.
///
/// # Errors
///
/// Returns an error if the file cannot be opened, is malformed, exceeds
/// the limits, or contains unsafe paths or a symbolic link.
fn unpack_zip(archive: &Path, dest: &Path, limits: ExtractLimits) -> Result<()> {
    let file =
        File::open(archive).with_context(|| format!("Failed to open {}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(file).context("Failed to read zip archive")?;

    // Early rejection: the central directory declares the size and number of entries.
    let count = zip.len() as u64;
    if count > limits.max_entries {
        bail!(
            "archive exceeds extraction limit of {} entries",
            limits.max_entries
        );
    }
    let mut declared = 0u64;
    for i in 0..zip.len() {
        let entry = zip.by_index(i).context("Failed to read zip entry")?;
        declared = declared.saturating_add(entry.size());
    }
    if declared > limits.max_bytes {
        bail!(
            "archive declares {declared} bytes, over the extraction limit of {} bytes",
            limits.max_bytes
        );
    }

    // Real count: the declared size may be wrong.
    let mut written = 0u64;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).context("Failed to read zip entry")?;
        if entry.is_symlink() {
            bail!(
                "symbolic links are not supported in zip archives: {}",
                entry.name()
            );
        }
        let rel = entry
            .enclosed_name()
            .ok_or_else(|| anyhow!("unsafe path in zip archive: {:?}", entry.name()))?;
        let out = dest.join(&rel);

        if entry.is_dir() {
            std::fs::create_dir_all(&out)
                .with_context(|| format!("Failed to create {}", out.display()))?;
            continue;
        }
        ensure_no_symlink_ancestors(dest, &rel)?;
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }

        let mut file =
            File::create(&out).with_context(|| format!("Failed to create {}", out.display()))?;
        // At most one byte above the budget is read: enough to detect the excess.
        let budget = limits.max_bytes.saturating_sub(written).saturating_add(1);
        let copied = io::copy(&mut (&mut entry).take(budget), &mut file)
            .with_context(|| format!("Failed to extract {}", out.display()))?;
        written = written.saturating_add(copied);
        if written > limits.max_bytes {
            bail!(
                "archive exceeds extraction limit of {} bytes",
                limits.max_bytes
            );
        }

        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode & 0o755))
                .with_context(|| format!("Failed to set permissions on {}", out.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    /// Raw tar entry: the name and the link target are set byte by byte,
    /// without going through the checks of `tar::Builder`.
    struct RawEntry<'a> {
        name: &'a str,
        kind: tar::EntryType,
        link: Option<&'a str>,
        data: &'a [u8],
    }

    fn write_raw_tar_gz(path: &Path, entries: &[RawEntry<'_>]) {
        let file = File::create(path).unwrap();
        let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(gz);
        for entry in entries {
            let mut header = tar::Header::new_gnu();
            {
                let old = header.as_old_mut();
                old.name[..entry.name.len()].copy_from_slice(entry.name.as_bytes());
                if let Some(link) = entry.link {
                    old.linkname[..link.len()].copy_from_slice(link.as_bytes());
                }
            }
            header.set_entry_type(entry.kind);
            header.set_size(entry.data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append(&header, entry.data).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
    }

    fn file_entry<'a>(name: &'a str, data: &'a [u8]) -> RawEntry<'a> {
        RawEntry {
            name,
            kind: tar::EntryType::Regular,
            link: None,
            data,
        }
    }

    fn link_entry<'a>(name: &'a str, kind: tar::EntryType, target: &'a str) -> RawEntry<'a> {
        RawEntry {
            name,
            kind,
            link: Some(target),
            data: &[],
        }
    }

    fn make_tar_gz(path: &Path, entry_name: &str, contents: &[u8]) {
        write_raw_tar_gz(path, &[file_entry(entry_name, contents)]);
    }

    fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        for (name, contents) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(contents).unwrap();
        }
        writer.finish().unwrap();
    }

    fn fresh_dest(dir: &tempfile::TempDir) -> PathBuf {
        let dest = dir.path().join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        dest
    }

    fn small_limits() -> ExtractLimits {
        ExtractLimits {
            max_bytes: 1_000,
            max_entries: 100,
        }
    }

    #[test]
    fn unpack_tar_gz_extracts_files() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("node.tar.gz");
        make_tar_gz(&archive, "node/bin/hello.txt", b"hello from tar");

        let dest = fresh_dest(&dir);
        unpack(&archive, &dest).unwrap();

        let extracted = dest.join("node").join("bin").join("hello.txt");
        assert_eq!(
            std::fs::read_to_string(extracted).unwrap(),
            "hello from tar"
        );
    }

    #[test]
    fn unpack_zip_extracts_files() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("node.zip");
        make_zip(&archive, &[("node/bin/hello.txt", b"hello from zip")]);

        let dest = fresh_dest(&dir);
        unpack(&archive, &dest).unwrap();

        let extracted = dest.join("node").join("bin").join("hello.txt");
        assert_eq!(
            std::fs::read_to_string(extracted).unwrap(),
            "hello from zip"
        );
    }

    #[test]
    fn unpack_rejects_unsupported_extension() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("node.rar");
        std::fs::write(&archive, b"not a real archive").unwrap();

        let dest = dir.path().join("dest");
        let err = unpack(&archive, &dest).unwrap_err();
        assert!(err.to_string().contains("unsupported archive format"));
    }

    #[test]
    fn unpack_rejects_tar_xz_with_clear_message() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("node.tar.xz");
        std::fs::write(&archive, b"not a real archive").unwrap();

        let dest = dir.path().join("dest");
        let err = unpack(&archive, &dest).unwrap_err();
        assert_eq!(
            err.to_string(),
            "unsupported archive format: .tar.xz (use .tar.gz or .zip)"
        );
    }

    #[test]
    fn unpack_tar_gz_errors_on_corrupt_archive() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("corrupt.tar.gz");
        std::fs::write(&archive, b"not actually gzip data").unwrap();

        let dest = dir.path().join("dest");
        assert!(unpack(&archive, &dest).is_err());
    }

    #[test]
    fn default_limits_are_2_gib_and_100000_entries() {
        let limits = ExtractLimits::default();
        assert_eq!(limits.max_bytes, 2 * 1024 * 1024 * 1024);
        assert_eq!(limits.max_entries, 100_000);
    }

    #[test]
    fn tar_over_byte_limit_fails() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("big.tar.gz");
        let payload = vec![b'a'; 10_000];
        make_tar_gz(&archive, "node/big.bin", &payload);

        let dest = fresh_dest(&dir);
        let err = unpack_with_limits(&archive, &dest, small_limits()).unwrap_err();
        assert!(format!("{err:#}").contains("extraction limit"), "{err:#}");
    }

    #[test]
    fn tar_over_entry_limit_fails() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("many.tar.gz");
        write_raw_tar_gz(
            &archive,
            &[
                file_entry("node/a", b"a"),
                file_entry("node/b", b"b"),
                file_entry("node/c", b"c"),
            ],
        );

        let dest = fresh_dest(&dir);
        let limits = ExtractLimits {
            max_bytes: 1_000_000,
            max_entries: 2,
        };
        let err = unpack_with_limits(&archive, &dest, limits).unwrap_err();
        assert!(format!("{err:#}").contains("entries"), "{err:#}");
    }

    #[test]
    fn zip_over_byte_limit_fails() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("big.zip");
        let payload = vec![b'z'; 5_000];
        make_zip(&archive, &[("node/big.bin", payload.as_slice())]);

        let dest = fresh_dest(&dir);
        let err = unpack_with_limits(&archive, &dest, small_limits()).unwrap_err();
        assert!(format!("{err:#}").contains("extraction limit"), "{err:#}");
    }

    #[test]
    fn zip_over_entry_limit_fails() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("many.zip");
        make_zip(&archive, &[("a", b"a"), ("b", b"b"), ("c", b"c")]);

        let dest = fresh_dest(&dir);
        let limits = ExtractLimits {
            max_bytes: 1_000_000,
            max_entries: 2,
        };
        assert!(unpack_with_limits(&archive, &dest, limits).is_err());
    }

    #[test]
    fn tar_within_limits_succeeds() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("ok.tar.gz");
        make_tar_gz(&archive, "node/ok.txt", b"small");

        // The count includes tar headers and the final block (~2 KiB).
        let limits = ExtractLimits {
            max_bytes: 64 * 1024,
            max_entries: 100,
        };
        let dest = fresh_dest(&dir);
        unpack_with_limits(&archive, &dest, limits).unwrap();
        assert!(dest.join("node").join("ok.txt").is_file());
    }

    #[test]
    fn tar_rejects_parent_dir_entry() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("evil.tar.gz");
        write_raw_tar_gz(&archive, &[file_entry("node/../../evil.txt", b"x")]);

        let dest = fresh_dest(&dir);
        assert!(unpack(&archive, &dest).is_err());
        assert!(!dir.path().join("evil.txt").exists());
        assert!(!dest.join("evil.txt").exists());
    }

    #[test]
    fn tar_rejects_absolute_path_entry() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("abs.tar.gz");
        write_raw_tar_gz(&archive, &[file_entry("/tmp/nvsn-evil.txt", b"x")]);

        let dest = fresh_dest(&dir);
        let err = unpack(&archive, &dest).unwrap_err();
        assert!(format!("{err:#}").contains("absolute"), "{err:#}");
    }

    #[test]
    fn tar_rejects_symlink_escaping_destination() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("link.tar.gz");
        write_raw_tar_gz(
            &archive,
            &[link_entry(
                "node/bin/evil",
                tar::EntryType::Symlink,
                "../../../../etc/passwd",
            )],
        );

        let dest = fresh_dest(&dir);
        assert!(unpack(&archive, &dest).is_err());
        assert!(std::fs::symlink_metadata(dest.join("node/bin/evil")).is_err());
    }

    #[test]
    fn tar_rejects_absolute_symlink_target() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("abslink.tar.gz");
        write_raw_tar_gz(
            &archive,
            &[link_entry("node/root", tar::EntryType::Symlink, "/")],
        );

        let dest = fresh_dest(&dir);
        assert!(unpack(&archive, &dest).is_err());
    }

    #[test]
    fn tar_rejects_hard_link_outside_destination() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("hard.tar.gz");
        write_raw_tar_gz(
            &archive,
            &[link_entry(
                "node/copy",
                tar::EntryType::Link,
                "../../outside",
            )],
        );

        let dest = fresh_dest(&dir);
        assert!(unpack(&archive, &dest).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn tar_accepts_symlink_inside_destination() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("inside.tar.gz");
        write_raw_tar_gz(
            &archive,
            &[
                file_entry("node/lib/cli.js", b"cli"),
                link_entry("node/bin/npm", tar::EntryType::Symlink, "../lib/cli.js"),
            ],
        );

        let dest = fresh_dest(&dir);
        unpack(&archive, &dest).unwrap();

        let npm = dest.join("node").join("bin").join("npm");
        assert_eq!(
            std::fs::read_link(&npm).unwrap(),
            PathBuf::from("../lib/cli.js")
        );
        assert_eq!(std::fs::read_to_string(&npm).unwrap(), "cli");
    }

    #[cfg(unix)]
    #[test]
    fn tar_rejects_write_through_an_extracted_symlink() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("chain.tar.gz");
        write_raw_tar_gz(
            &archive,
            &[
                link_entry("node/a", tar::EntryType::Symlink, "."),
                link_entry("node/a/b", tar::EntryType::Symlink, "../../x"),
            ],
        );

        let dest = fresh_dest(&dir);
        assert!(unpack(&archive, &dest).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn tar_rejects_link_target_through_an_extracted_symlink() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("target.tar.gz");
        write_raw_tar_gz(
            &archive,
            &[
                link_entry("node/d", tar::EntryType::Symlink, "."),
                link_entry("node/e", tar::EntryType::Symlink, "d/../../.."),
            ],
        );

        let dest = fresh_dest(&dir);
        assert!(unpack(&archive, &dest).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn tar_rejects_chained_symlinks_that_escape_once_both_exist() {
        // x -> y/.. is accepted while y is not a link. Afterwards y -> ..
        // turns x into dest/.., and each individual check accepted it.
        let dir = tempdir().unwrap();
        let archive = dir.path().join("chain-escape.tar.gz");
        write_raw_tar_gz(
            &archive,
            &[
                link_entry("node/x", tar::EntryType::Symlink, "y/.."),
                link_entry("node/y", tar::EntryType::Symlink, ".."),
            ],
        );

        let dest = fresh_dest(&dir);
        let err = unpack(&archive, &dest).unwrap_err();
        assert!(
            format!("{err:#}").contains("escapes the destination"),
            "{err:#}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn tar_accepts_chained_symlinks_that_stay_inside() {
        // Real Node pattern: bin/npm -> ../lib/cli.js, with lib already a directory.
        let dir = tempdir().unwrap();
        let archive = dir.path().join("chain-ok.tar.gz");
        write_raw_tar_gz(
            &archive,
            &[
                file_entry("node/lib/cli.js", b"cli"),
                link_entry("node/bin/npm", tar::EntryType::Symlink, "../lib/cli.js"),
                link_entry("node/bin/npx", tar::EntryType::Symlink, "npm"),
            ],
        );

        let dest = fresh_dest(&dir);
        unpack(&archive, &dest).unwrap();
        assert_eq!(
            std::fs::read_to_string(dest.join("node/bin/npx")).unwrap(),
            "cli"
        );
    }

    #[cfg(unix)]
    #[test]
    fn tar_rejects_symlink_cycle() {
        // a -> b, then b -> a: resolving the second link finds a cycle.
        let dir = tempdir().unwrap();
        let archive = dir.path().join("cycle.tar.gz");
        write_raw_tar_gz(
            &archive,
            &[
                link_entry("node/a", tar::EntryType::Symlink, "b"),
                link_entry("node/b", tar::EntryType::Symlink, "a"),
            ],
        );

        let dest = fresh_dest(&dir);
        let err = unpack(&archive, &dest).unwrap_err();
        assert!(format!("{err:#}").contains("too many symlinks"), "{err:#}");
    }

    #[test]
    fn zip_rejects_parent_dir_entry() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("evil.zip");
        make_zip(&archive, &[("../evil.txt", b"x")]);

        let dest = fresh_dest(&dir);
        assert!(unpack(&archive, &dest).is_err());
        assert!(!dir.path().join("evil.txt").exists());
    }
}
