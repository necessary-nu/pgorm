//! pgorm-driver's integration tests: one binary, every suite a module of it.
//! The manifest turns cargo's test discovery off and names this file the
//! crate's one test target, so a file under `tests/` is compiled only once
//! it is declared here, which the workspace's `suite_declaration_tests`
//! holds every crate to.
//!
//! The `native_*` suites gate themselves on `turso` with an inner `#![cfg]`.

pub mod config_paths;
pub mod native_capabilities;
pub mod native_execution;
pub mod native_lifecycle;
pub mod native_transactions;
pub mod native_types;
pub mod native_unsupported;
/// The upstream tokio-postgres suite, which needs its own authentication
/// fixture on port 5433 (`tests/upstream/sql_setup.sh`).
#[cfg(feature = "upstream-tests")]
pub mod upstream;
