//! vcli-store — SQLite persistence + content-addressed asset store.
//!
//! See the v0 design spec at `docs/superpowers/specs/2026-04-16-vcli-design.md`
//! §Persistence for the authoritative schema and semantics this crate implements.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod error;
pub mod paths;

pub use error::{StoreError, StoreResult};
pub use paths::{asset_blob_path, assets_root, db_path};
