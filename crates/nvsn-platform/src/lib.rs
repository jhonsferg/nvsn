//! nvsn-platform: where and how nvsn runs.
//!
//! Detects operating system, architecture and libc using Node's names,
//! resolves nvsn's root directory, provides the fixed `current` link (ADR-034)
//! and, on Windows, the user PATH from the registry. It contains no business logic.

pub mod arch;
pub mod env;
pub mod libc;
pub mod links;
pub mod paths;
pub mod user_path;

pub use arch::{effective_arch, parse_arch_override, Arch, Os, Platform};
pub use env::{EnvSource, ProcessEnv};
pub use libc::Libc;
pub use links::set_version_link;
pub use paths::{default_nvsn_dir, resolve_nvsn_dir};
