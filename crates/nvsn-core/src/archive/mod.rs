//! Extraction of downloaded archives.
//!
//! Supported formats: `.tar.gz` and `.zip`. `.tar.xz` is explicitly rejected
//! until an xz backend exists (ADR-006).

pub mod extract;
