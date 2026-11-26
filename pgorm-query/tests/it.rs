//! pgorm-query's integration tests: one binary, every suite a module of it.
//! The manifest turns cargo's test discovery off and names this file the
//! crate's one test target, so a file under `tests/` is compiled only once
//! it is declared here, which the workspace's `suite_declaration_tests`
//! holds every crate to.
//!
//! Each gate is the feature its suite reads: the `derive` macros, a default,
//! and `tests_cfg`'s fixtures, which the crate's dev-dependency on itself
//! turns on wherever this binary builds.

#[cfg(feature = "derive")]
pub mod derive;
#[cfg(feature = "tests-cfg")]
pub mod error;
#[cfg(feature = "tests-cfg")]
pub mod postgres;
pub mod surface;
