//! nvsn-core: nvsn's domain (extraction, locks, checksums, store layout).
//!
//! This crate knows nothing about the CLI or the network. The store paths come
//! from the caller through [`Store::new`].

#![forbid(unsafe_code)]

pub mod archive;
pub mod artifact;
pub mod cache;
pub mod checksum;
pub mod config;
pub mod install;
pub mod lock;
pub mod nvmrc;
pub mod packages;
pub mod remote;
pub mod spec;
pub mod store;
pub mod tempdir;

pub use archive::extract::unpack;
pub use artifact::{artifact_for, Artifact};
pub use cache::{
    cache_clear, cache_dirs, cache_list, CacheClearReport, CacheDirs, CacheEntry, CacheKind,
};
pub use checksum::{parse_shasums, verify_sha256};
pub use config::{load_settings, resolve_settings, Config, Overrides, Settings};
pub use lock::FileLock;
pub use nvmrc::{find_version_file, VersionFile};
pub use packages::{install_latest_npm, list_global_packages, reinstall_packages};
pub use remote::{parse_index, RemoteRelease};
pub use spec::{parse_spec, resolve, VersionSpec};
pub use store::Store;
pub use tempdir::TempDir;
