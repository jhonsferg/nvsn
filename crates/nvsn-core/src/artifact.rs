//! Distribution file name and binary path, per version and platform, following
//! the naming scheme published at `https://nodejs.org/dist/`.
//!
//! Two levels of checking:
//! - [`artifact_for`]: pure mapping from (version, platform) to names. Rejects
//!   platforms without an official binary (Android, musl outside x64, etc.).
//! - [`ensure_in_release`]: checks that the index for that version lists the
//!   token. Covers per-version cutoffs (e.g. `win-x86` only up to v22).
//!
//! The mapping from `osx-*` (index) to `darwin-*` (file) is done here (ADR-021).

use crate::remote::RemoteRelease;
use anyhow::{bail, Result};
use nvsn_platform::{Arch, Libc, Os, Platform};
use semver::Version;

/// Distribution file of a version for a platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    /// Platform token from the index (`linux-x64`, `osx-arm64-tar`, `win-x64-zip`).
    pub token: &'static str,
    /// File name in `dist/` (`node-v20.11.1-linux-x64.tar.gz`).
    pub file_name: String,
    /// Root directory inside the archive (`node-v20.11.1-linux-x64`).
    pub root_dir: String,
    /// Path of the executable relative to `root_dir`: `bin/node` or `node.exe`.
    pub bin_path: &'static str,
}

/// Platform data shared by all versions.
struct DistTarget {
    /// Index token.
    token: &'static str,
    /// `<os>-<arch>` part of the file name (`darwin-arm64`, `win-x64`).
    dist: &'static str,
    /// File extension: `tar.gz` or `zip`.
    extension: &'static str,
    /// Path of the executable inside the root directory.
    bin_path: &'static str,
}

/// Returns the artifact of `version` for `platform`.
///
/// # Errors
///
/// Returns an error if the platform has no official Node binary. On Android
/// the message points to the Termux provider (ADR-022).
pub fn artifact_for(version: &Version, platform: Platform) -> Result<Artifact> {
    let target = dist_target(platform)?;
    let root_dir = format!("node-v{version}-{}", target.dist);
    Ok(Artifact {
        token: target.token,
        file_name: format!("{root_dir}.{}", target.extension),
        root_dir,
        bin_path: target.bin_path,
    })
}

/// Returns the artifact of a specific index release for `platform`.
///
/// # Errors
///
/// Returns an error if the platform has no official binary, or if the index for
/// that release does not list the token (see [`ensure_in_release`]).
pub fn artifact_for_release(release: &RemoteRelease, platform: Platform) -> Result<Artifact> {
    let artifact = artifact_for(&release.version, platform)?;
    ensure_in_release(&artifact, release)?;
    Ok(artifact)
}

/// Checks that the index of `release` lists the artifact.
///
/// # Errors
///
/// Returns an error if the token does not appear in `release.files`.
pub fn ensure_in_release(artifact: &Artifact, release: &RemoteRelease) -> Result<()> {
    if release.has_file(artifact.token) {
        return Ok(());
    }
    bail!(
        "node {} does not publish the artifact {} ({}) for this platform",
        release.tag(),
        artifact.token,
        artifact.file_name
    )
}

/// Maps nvsn's platform to Node's distribution data.
fn dist_target(platform: Platform) -> Result<DistTarget> {
    match platform.os {
        Os::Android => bail!(
            "Node does not publish official binaries for Android. Use the Termux provider: \
             `pkg install nodejs-lts` (ADR-022)"
        ),
        Os::Linux => linux_target(platform),
        Os::MacOs => match platform.arch {
            Arch::Arm64 => Ok(unix("osx-arm64-tar", "darwin-arm64")),
            Arch::X64 => Ok(unix("osx-x64-tar", "darwin-x64")),
            other => unsupported(platform.os, other, platform.libc),
        },
        Os::Windows => match platform.arch {
            Arch::X64 => Ok(windows("win-x64-zip", "win-x64")),
            Arch::Arm64 => Ok(windows("win-arm64-zip", "win-arm64")),
            Arch::X86 => Ok(windows("win-x86-zip", "win-x86")),
            other => unsupported(platform.os, other, platform.libc),
        },
    }
}

/// Linux platforms: glibc for all architectures, musl only on x64.
fn linux_target(platform: Platform) -> Result<DistTarget> {
    if platform.libc == Libc::Musl {
        return match platform.arch {
            Arch::X64 => Ok(unix("linux-x64-musl", "linux-x64-musl")),
            other => unsupported(platform.os, other, platform.libc),
        };
    }
    match platform.arch {
        Arch::X64 => Ok(unix("linux-x64", "linux-x64")),
        Arch::Arm64 => Ok(unix("linux-arm64", "linux-arm64")),
        Arch::Armv7l => Ok(unix("linux-armv7l", "linux-armv7l")),
        Arch::X86 => Ok(unix("linux-x86", "linux-x86")),
        Arch::Ppc64le => Ok(unix("linux-ppc64le", "linux-ppc64le")),
        Arch::S390x => Ok(unix("linux-s390x", "linux-s390x")),
        other => unsupported(platform.os, other, platform.libc),
    }
}

/// Unix target: `tar.gz` and `bin/node`.
fn unix(token: &'static str, dist: &'static str) -> DistTarget {
    DistTarget {
        token,
        dist,
        extension: "tar.gz",
        bin_path: "bin/node",
    }
}

/// Windows target: `zip` and `node.exe` at the root of the directory.
fn windows(token: &'static str, dist: &'static str) -> DistTarget {
    DistTarget {
        token,
        dist,
        extension: "zip",
        bin_path: "node.exe",
    }
}

/// Uniform error for combinations without an official binary.
fn unsupported<T>(os: Os, arch: Arch, libc: Libc) -> Result<T> {
    bail!(
        "there is no official Node binary for {os:?}/{arch:?} ({libc:?}). \
         Check the platform or use another provider"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::parse_index;

    fn platform(os: Os, arch: Arch, libc: Libc) -> Platform {
        Platform { os, arch, libc }
    }

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn linux_glibc_x64_uses_tar_gz_and_bin_node() {
        let art = artifact_for(
            &version("20.11.1"),
            platform(Os::Linux, Arch::X64, Libc::Glibc),
        )
        .unwrap();
        assert_eq!(art.token, "linux-x64");
        assert_eq!(art.file_name, "node-v20.11.1-linux-x64.tar.gz");
        assert_eq!(art.root_dir, "node-v20.11.1-linux-x64");
        assert_eq!(art.bin_path, "bin/node");
    }

    #[test]
    fn linux_musl_x64_uses_musl_token() {
        let art = artifact_for(
            &version("22.0.0"),
            platform(Os::Linux, Arch::X64, Libc::Musl),
        )
        .unwrap();
        assert_eq!(art.token, "linux-x64-musl");
        assert_eq!(art.file_name, "node-v22.0.0-linux-x64-musl.tar.gz");
    }

    #[test]
    fn musl_outside_x64_is_error() {
        let err = artifact_for(
            &version("22.0.0"),
            platform(Os::Linux, Arch::Arm64, Libc::Musl),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("Arm64"), "{err}");
    }

    #[test]
    fn macos_maps_osx_token_to_darwin_file_name() {
        let arm = artifact_for(
            &version("24.21.0"),
            platform(Os::MacOs, Arch::Arm64, Libc::NotApplicable),
        )
        .unwrap();
        assert_eq!(arm.token, "osx-arm64-tar");
        assert_eq!(arm.file_name, "node-v24.21.0-darwin-arm64.tar.gz");
        assert_eq!(arm.bin_path, "bin/node");

        let x64 = artifact_for(
            &version("24.21.0"),
            platform(Os::MacOs, Arch::X64, Libc::NotApplicable),
        )
        .unwrap();
        assert_eq!(x64.token, "osx-x64-tar");
        assert_eq!(x64.file_name, "node-v24.21.0-darwin-x64.tar.gz");
    }

    #[test]
    fn windows_uses_zip_and_node_exe_at_root() {
        let art = artifact_for(
            &version("24.21.0"),
            platform(Os::Windows, Arch::X64, Libc::NotApplicable),
        )
        .unwrap();
        assert_eq!(art.token, "win-x64-zip");
        assert_eq!(art.file_name, "node-v24.21.0-win-x64.zip");
        assert_eq!(art.root_dir, "node-v24.21.0-win-x64");
        assert_eq!(art.bin_path, "node.exe");

        let x86 = artifact_for(
            &version("22.23.3"),
            platform(Os::Windows, Arch::X86, Libc::NotApplicable),
        )
        .unwrap();
        assert_eq!(x86.file_name, "node-v22.23.3-win-x86.zip");
    }

    #[test]
    fn android_points_to_termux_provider() {
        let err = artifact_for(
            &version("24.21.0"),
            platform(Os::Android, Arch::Arm64, Libc::Bionic),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("Termux"), "{err}");
        assert!(err.contains("ADR-022"), "{err}");
    }

    #[test]
    fn riscv_has_no_official_build_here() {
        assert!(artifact_for(
            &version("24.21.0"),
            platform(Os::Linux, Arch::Riscv64, Libc::Glibc),
        )
        .is_err());
    }

    #[test]
    fn release_must_list_the_token() {
        let releases = parse_index(
            r#"[
                {"version":"v22.23.3","date":"2026-09-23","files":["linux-x64","win-x86-zip"],"lts":"Jod"},
                {"version":"v23.0.0","date":"2024-10-16","files":["linux-x64","win-x64-zip"],"lts":false}
            ]"#,
        )
        .unwrap();
        let v22 = releases.iter().find(|r| r.version.major == 22).unwrap();
        let v23 = releases.iter().find(|r| r.version.major == 23).unwrap();

        let win_x86 = platform(Os::Windows, Arch::X86, Libc::NotApplicable);
        assert!(artifact_for_release(v22, win_x86).is_ok());

        let err = artifact_for_release(v23, win_x86).unwrap_err().to_string();
        assert!(err.contains("win-x86-zip"), "{err}");
        assert!(err.contains("v23.0.0"), "{err}");
    }

    #[test]
    fn ensure_in_release_checks_the_exact_token() {
        let releases = parse_index(
            r#"[{"version":"v20.11.1","date":"2024-02-14","files":["osx-arm64-tar"],"lts":"Iron"}]"#,
        )
        .unwrap();
        let art = artifact_for(
            &version("20.11.1"),
            platform(Os::MacOs, Arch::Arm64, Libc::NotApplicable),
        )
        .unwrap();
        assert!(ensure_in_release(&art, &releases[0]).is_ok());

        let x64 = artifact_for(
            &version("20.11.1"),
            platform(Os::MacOs, Arch::X64, Libc::NotApplicable),
        )
        .unwrap();
        assert!(ensure_in_release(&x64, &releases[0]).is_err());
    }
}
