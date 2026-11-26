//! pgorm-python's integration tests: one binary, every suite a module of it.
//! The manifest turns cargo's test discovery off and names this file the
//! crate's one test target, so a file under `tests/` is compiled only once
//! it is declared here, which the workspace's `suite_declaration_tests`
//! holds every crate to.

pub mod direct_builders;
pub mod lifecycle;
pub mod pipeline;
pub mod schema;
pub mod schema_objects;
pub mod values;
