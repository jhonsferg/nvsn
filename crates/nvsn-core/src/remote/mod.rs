//! Node.js remote data: the index published at nodejs.org (ADR-009, ADR-012).

pub mod cache;
pub mod index;

pub use cache::fetch_index;
pub use index::{parse_index, LtsField, RemoteRelease};
