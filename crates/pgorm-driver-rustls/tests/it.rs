//! pgorm-driver-rustls's integration tests: one binary, every suite a module of it.
//! The manifest turns cargo's test discovery off and names this file the
//! crate's one test target, so a file under `tests/` is compiled only once
//! it is declared here, which the workspace's `suite_declaration_tests`
//! holds every crate to.

// What suites share, and the modules of single suites: a suite's own file
// is not a directory module, so a module beside it is declared here.
#[cfg(any(feature = "aws-lc-rs", feature = "ring"))]
pub mod support;

pub mod integration;
pub mod make_tls_connect;
