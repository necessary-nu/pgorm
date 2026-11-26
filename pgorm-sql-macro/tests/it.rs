//! pgorm-sql-macro's integration tests: one binary, every suite a module of it.
//! The manifest turns cargo's test discovery off and names this file the
//! crate's one test target, so a file under `tests/` is compiled only once
//! it is declared here, which the workspace's `suite_declaration_tests`
//! holds every crate to.

pub mod compile_fail_test;
pub mod prql_macro_test;
pub mod sql_macro_test;
