//! System libc detection: glibc, musl or bionic (Termux).
//!
//! Decisions:
//! - No FFI and no running of `ldd`: the crate forbids `unsafe`.
//! - On Linux, the presence of the musl loader (`/lib/ld-musl-<arch>.so.1`) is
//!   checked at runtime, as indicated by ARCHITECTURE.md §3.1 and
//!   R-09. If it is absent, glibc is assumed.
//! - `cfg(target_env)` is not the source of truth: it describes the libc the
//!   binary was compiled against, not the host's. A musl binary can run on a
//!   glibc host and vice versa.
//! - Android is always bionic; no probing is needed.
//!
//! Limitations:
//! - A glibc host that has the musl loader installed is detected as musl.
//! - Hosts with another libc (uClibc, etc.) are detected as glibc.
//! - The musl loader names for architectures other than x86_64 have not
//!   been tested in an Alpine container (R-09).

use crate::arch::{Arch, Os};
use std::path::Path;

/// System C library, depending on the platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Libc {
    /// GNU C Library (Linux with glibc).
    Glibc,
    /// musl libc (Alpine and similar).
    Musl,
    /// Bionic, the Android libc. Termux uses it.
    Bionic,
    /// macOS and Windows have no applicable libc concept here.
    NotApplicable,
}

/// Detects the host's libc.
///
/// `root` is the filesystem root (`/` in production). It is received as a
/// parameter so the detection can be tested with a temporary directory.
pub fn detect(os: Os, arch: Arch, root: &Path) -> Libc {
    match os {
        Os::Android => Libc::Bionic,
        Os::MacOs | Os::Windows => Libc::NotApplicable,
        Os::Linux => {
            if musl_loader_present(arch, root) {
                Libc::Musl
            } else {
                Libc::Glibc
            }
        }
    }
}

/// Name of the musl dynamic loader for each architecture.
fn musl_loader_name(arch: Arch) -> &'static str {
    match arch {
        Arch::X64 => "ld-musl-x86_64.so.1",
        Arch::Arm64 => "ld-musl-aarch64.so.1",
        Arch::Armv7l => "ld-musl-armhf.so.1",
        Arch::X86 => "ld-musl-i386.so.1",
        Arch::Ppc64le => "ld-musl-powerpc64le.so.1",
        Arch::S390x => "ld-musl-s390x.so.1",
        Arch::Riscv64 => "ld-musl-riscv64.so.1",
    }
}

/// Indicates whether the musl loader exists in `/lib` or `/usr/lib` under `root`.
fn musl_loader_present(arch: Arch, root: &Path) -> bool {
    let name = musl_loader_name(arch);
    ["lib", "usr/lib"]
        .iter()
        .any(|dir| root.join(dir).join(name).exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_is_bionic_and_others_not_applicable() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(detect(Os::Android, Arch::Arm64, root.path()), Libc::Bionic);
        assert_eq!(
            detect(Os::Windows, Arch::X64, root.path()),
            Libc::NotApplicable
        );
        assert_eq!(
            detect(Os::MacOs, Arch::Arm64, root.path()),
            Libc::NotApplicable
        );
    }

    #[test]
    fn linux_without_musl_loader_is_glibc() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(detect(Os::Linux, Arch::X64, root.path()), Libc::Glibc);
    }

    #[test]
    fn linux_with_musl_loader_is_musl() {
        let root = tempfile::tempdir().unwrap();
        let lib = root.path().join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(lib.join("ld-musl-x86_64.so.1"), b"").unwrap();
        assert_eq!(detect(Os::Linux, Arch::X64, root.path()), Libc::Musl);
    }

    #[test]
    fn musl_loader_is_matched_by_architecture() {
        let root = tempfile::tempdir().unwrap();
        let lib = root.path().join("usr").join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(lib.join("ld-musl-aarch64.so.1"), b"").unwrap();
        assert_eq!(detect(Os::Linux, Arch::Arm64, root.path()), Libc::Musl);
        assert_eq!(detect(Os::Linux, Arch::X64, root.path()), Libc::Glibc);
    }
}
