//! OS, architecture and libc detection using Node's naming.
//!
//! The values from `std::env::consts` are translated to the names used by
//! Node's distribution files (`node-v<ver>-<os>-<arch>`). The mapping
//! functions are pure and take strings, so they can be tested on any host.
//!
//! Limitations:
//! - `std::env::consts::ARCH` is `arm` for both ARMv6 and ARMv7. The
//!   `armv7l` build is used. An ARMv6 host (e.g. Raspberry Pi 1) receives a
//!   binary that does not match it.
//! - Node does not publish big-endian `ppc64`; it is rejected with a clear error.
//! - SunOS, AIX, FreeBSD and other unsupported systems produce an error.

use crate::env::{EnvSource, ProcessEnv};
use crate::libc::{self, Libc};
use anyhow::{bail, Result};
use std::path::Path;

/// Operating system supported by nvsn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    /// Desktop or server Linux (glibc or musl).
    Linux,
    /// macOS.
    MacOs,
    /// Windows.
    Windows,
    /// Android with Termux. Node does not publish binaries for Android: the
    /// provider is Termux (ADR-022).
    Android,
}

impl Os {
    /// Operating system name in Node's distribution files.
    /// Windows uses `win` in the files, even though `process.platform` is `win32`.
    pub fn dist_name(self) -> &'static str {
        match self {
            Os::Linux => "linux",
            Os::MacOs => "darwin",
            Os::Windows => "win",
            Os::Android => "android",
        }
    }
}

/// CPU architecture using Node's names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    /// x86_64 (`x64` in Node).
    X64,
    /// AArch64 (`arm64` in Node).
    Arm64,
    /// 32-bit ARM with FPU (`armv7l`).
    Armv7l,
    /// 32-bit x86 (`x86` in Node's files; `ia32` in `process.arch`).
    X86,
    /// 64-bit PowerPC little-endian.
    Ppc64le,
    /// IBM z/Architecture.
    S390x,
    /// 64-bit RISC-V.
    Riscv64,
}

impl Arch {
    /// Architecture name in Node's distribution files.
    pub fn dist_name(self) -> &'static str {
        match self {
            Arch::X64 => "x64",
            Arch::Arm64 => "arm64",
            Arch::Armv7l => "armv7l",
            Arch::X86 => "x86",
            Arch::Ppc64le => "ppc64le",
            Arch::S390x => "s390x",
            Arch::Riscv64 => "riscv64",
        }
    }
}

/// Parses an architecture override, as accepted by `NVSN_ARCH` and `config.toml`.
///
/// Accepts Node's names (`x64`, `arm64`, `armv7l`, `x86`, `ppc64le`, `s390x`,
/// `riscv64`) and the nvm-windows aliases `32` (x86) and `64` (x64). Matching
/// is case-insensitive and surrounding whitespace is ignored.
///
/// # Errors
///
/// Returns an error listing the accepted names if `value` is not one of them.
pub fn parse_arch_override(value: &str) -> Result<Arch> {
    match value.trim().to_ascii_lowercase().as_str() {
        "x64" | "64" => Ok(Arch::X64),
        "arm64" => Ok(Arch::Arm64),
        "armv7l" => Ok(Arch::Armv7l),
        "x86" | "32" => Ok(Arch::X86),
        "ppc64le" => Ok(Arch::Ppc64le),
        "s390x" => Ok(Arch::S390x),
        "riscv64" => Ok(Arch::Riscv64),
        _ => bail!(
            "unsupported architecture '{}': expected x64, arm64, armv7l, x86, ppc64le, s390x, riscv64, 32 or 64",
            value.trim()
        ),
    }
}

/// Effective architecture: the override when one is set, the detected one otherwise.
///
/// An empty or blank override counts as not set. The override is not checked
/// against the host: it selects which binary is downloaded, so it may differ
/// from `detected` on purpose (for example, x64 binaries on a machine that runs
/// them under emulation).
///
/// # Errors
///
/// Returns an error if the override is set but is not a supported name
/// (see [`parse_arch_override`]).
pub fn effective_arch(override_value: Option<&str>, detected: Arch) -> Result<Arch> {
    match override_value {
        Some(value) if !value.trim().is_empty() => parse_arch_override(value),
        _ => Ok(detected),
    }
}

/// Host platform: system, architecture and libc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Platform {
    /// Operating system. On Termux this is [`Os::Android`], even though the binary is linux.
    pub os: Os,
    /// CPU architecture.
    pub arch: Arch,
    /// System C library.
    pub libc: Libc,
}

impl Platform {
    /// Detects the platform of the current process.
    ///
    /// # Errors
    ///
    /// Returns an error if the operating system or the architecture is not
    /// supported by nvsn.
    pub fn detect() -> Result<Self> {
        detect_from(
            std::env::consts::OS,
            std::env::consts::ARCH,
            cfg!(target_endian = "little"),
            &ProcessEnv,
            Path::new("/"),
        )
    }
}

/// Core of [`Platform::detect`] with all inputs explicit.
fn detect_from(
    raw_os: &str,
    raw_arch: &str,
    little_endian: bool,
    env: &dyn EnvSource,
    root: &Path,
) -> Result<Platform> {
    let os = os_from(raw_os, env, root)?;
    let arch = arch_from(raw_arch, little_endian)?;
    let libc = libc::detect(os, arch, root);
    Ok(Platform { os, arch, libc })
}

/// Maps `std::env::consts::OS` to [`Os`], detecting Termux on Linux.
fn os_from(raw: &str, env: &dyn EnvSource, root: &Path) -> Result<Os> {
    match raw {
        "linux" if is_termux(env, root) => Ok(Os::Android),
        "linux" => Ok(Os::Linux),
        "android" => Ok(Os::Android),
        "macos" => Ok(Os::MacOs),
        "windows" => Ok(Os::Windows),
        other => bail!(
            "unsupported operating system: {other} (nvsn supports linux, macos, windows and Android with Termux)"
        ),
    }
}

/// Indicates whether the process runs under Termux.
///
/// Termux sets `TERMUX_VERSION` in its sessions. As a fallback, the app
/// directory (`/data/data/com.termux`) is checked, in case the variable does
/// not reach the process.
fn is_termux(env: &dyn EnvSource, root: &Path) -> bool {
    env.var("TERMUX_VERSION").is_some() || root.join("data/data/com.termux").is_dir()
}

/// Maps `std::env::consts::ARCH` to [`Arch`].
fn arch_from(raw: &str, little_endian: bool) -> Result<Arch> {
    match raw {
        "x86_64" => Ok(Arch::X64),
        "aarch64" => Ok(Arch::Arm64),
        "arm" => Ok(Arch::Armv7l),
        "x86" => Ok(Arch::X86),
        "powerpc64" if little_endian => Ok(Arch::Ppc64le),
        "powerpc64" => {
            bail!("PowerPC 64 big-endian is not supported: Node does not publish binaries for it")
        }
        "s390x" => Ok(Arch::S390x),
        "riscv64" => Ok(Arch::Riscv64),
        other => bail!("unsupported architecture: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::MapEnv;

    #[test]
    fn linux_without_termux_is_linux() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            os_from("linux", &MapEnv::new(&[]), root.path()).unwrap(),
            Os::Linux
        );
    }

    #[test]
    fn linux_with_termux_version_is_android() {
        let root = tempfile::tempdir().unwrap();
        let env = MapEnv::new(&[("TERMUX_VERSION", "0.118.0")]);
        assert_eq!(os_from("linux", &env, root.path()).unwrap(), Os::Android);
    }

    #[test]
    fn linux_with_termux_app_dir_is_android() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("data/data/com.termux")).unwrap();
        assert_eq!(
            os_from("linux", &MapEnv::new(&[]), root.path()).unwrap(),
            Os::Android
        );
    }

    #[test]
    fn other_operating_systems_map_or_fail_clearly() {
        let root = tempfile::tempdir().unwrap();
        let env = MapEnv::new(&[]);
        assert_eq!(os_from("android", &env, root.path()).unwrap(), Os::Android);
        assert_eq!(os_from("macos", &env, root.path()).unwrap(), Os::MacOs);
        assert_eq!(os_from("windows", &env, root.path()).unwrap(), Os::Windows);
        let err = os_from("freebsd", &env, root.path()).unwrap_err();
        assert!(err.to_string().contains("freebsd"));
    }

    #[test]
    fn architectures_map_to_node_names() {
        assert_eq!(arch_from("x86_64", true).unwrap(), Arch::X64);
        assert_eq!(arch_from("aarch64", true).unwrap(), Arch::Arm64);
        assert_eq!(arch_from("arm", true).unwrap(), Arch::Armv7l);
        assert_eq!(arch_from("x86", true).unwrap(), Arch::X86);
        assert_eq!(arch_from("powerpc64", true).unwrap(), Arch::Ppc64le);
        assert_eq!(arch_from("s390x", true).unwrap(), Arch::S390x);
        assert_eq!(arch_from("riscv64", true).unwrap(), Arch::Riscv64);
    }

    #[test]
    fn big_endian_ppc64_and_unknown_arch_fail() {
        assert!(arch_from("powerpc64", false).is_err());
        let err = arch_from("sparc64", true).unwrap_err();
        assert!(err.to_string().contains("sparc64"));
    }

    #[test]
    fn dist_names_match_node_distribution_files() {
        assert_eq!(Os::Windows.dist_name(), "win");
        assert_eq!(Os::MacOs.dist_name(), "darwin");
        assert_eq!(Arch::X86.dist_name(), "x86");
        assert_eq!(Arch::X64.dist_name(), "x64");
        assert_eq!(Arch::Armv7l.dist_name(), "armv7l");
    }

    #[test]
    fn override_accepts_node_names_and_nvm_windows_aliases() {
        assert_eq!(parse_arch_override("x64").unwrap(), Arch::X64);
        assert_eq!(parse_arch_override("arm64").unwrap(), Arch::Arm64);
        assert_eq!(parse_arch_override("armv7l").unwrap(), Arch::Armv7l);
        assert_eq!(parse_arch_override("x86").unwrap(), Arch::X86);
        assert_eq!(parse_arch_override("ppc64le").unwrap(), Arch::Ppc64le);
        assert_eq!(parse_arch_override("s390x").unwrap(), Arch::S390x);
        assert_eq!(parse_arch_override("riscv64").unwrap(), Arch::Riscv64);
        assert_eq!(parse_arch_override("32").unwrap(), Arch::X86);
        assert_eq!(parse_arch_override("64").unwrap(), Arch::X64);
    }

    #[test]
    fn override_ignores_case_and_surrounding_whitespace() {
        assert_eq!(parse_arch_override("  ARM64 ").unwrap(), Arch::Arm64);
        assert_eq!(parse_arch_override("X86").unwrap(), Arch::X86);
    }

    #[test]
    fn override_rejects_unknown_names_with_the_accepted_list() {
        let err = parse_arch_override("mips").unwrap_err().to_string();
        assert!(err.contains("mips"));
        assert!(err.contains("riscv64"));
        assert!(parse_arch_override("").is_err());
        assert!(parse_arch_override("amd64").is_err());
        assert!(parse_arch_override("16").is_err());
    }

    #[test]
    fn effective_arch_prefers_a_set_override_over_detection() {
        assert_eq!(effective_arch(Some("x86"), Arch::X64).unwrap(), Arch::X86);
        assert_eq!(effective_arch(Some("64"), Arch::Arm64).unwrap(), Arch::X64);
    }

    #[test]
    fn effective_arch_falls_back_to_detection_when_unset_or_blank() {
        assert_eq!(effective_arch(None, Arch::Arm64).unwrap(), Arch::Arm64);
        assert_eq!(effective_arch(Some(""), Arch::X64).unwrap(), Arch::X64);
        assert_eq!(effective_arch(Some("   "), Arch::X64).unwrap(), Arch::X64);
    }

    #[test]
    fn effective_arch_fails_on_an_invalid_override() {
        assert!(effective_arch(Some("sparc"), Arch::X64).is_err());
    }

    #[test]
    fn detect_from_combines_os_arch_and_libc() {
        let root = tempfile::tempdir().unwrap();
        let platform =
            detect_from("linux", "x86_64", true, &MapEnv::new(&[]), root.path()).unwrap();
        assert_eq!(
            platform,
            Platform {
                os: Os::Linux,
                arch: Arch::X64,
                libc: Libc::Glibc,
            }
        );

        let termux = detect_from(
            "linux",
            "aarch64",
            true,
            &MapEnv::new(&[("TERMUX_VERSION", "0.118.0")]),
            root.path(),
        )
        .unwrap();
        assert_eq!(termux.os, Os::Android);
        assert_eq!(termux.arch, Arch::Arm64);
        assert_eq!(termux.libc, Libc::Bionic);
    }
}
