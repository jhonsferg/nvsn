//! nvsn-shell: generation of per-shell integration scripts.
//!
//! Crate with no internal dependencies. It generates text (environment scripts, wrappers,
//! hooks and profile blocks) from plain data: paths, active version
//! and shell name. The only I/O is reading and writing profile files,
//! and the home directory always arrives as a parameter.
//!
//! Supported shells: bash, zsh, fish and PowerShell (with a
//! directory-change hook), and POSIX sh and cmd.exe (basic level, no hook).

#![forbid(unsafe_code)]

pub mod profile;
pub mod shell;

pub use profile::{ProfileBlock, ShellProfile};
pub use shell::{
    available_shells, detect, from_str, inject_login_profile, inject_profile, is_available,
    is_dir_in_path, strip_profile, Bash, EnvContext, Fish, PowerShell, ShellConfig, ShellError,
    Zsh,
};
