//! pgorm-codegen's integration tests: one binary, every suite a module of it.
//! The manifest turns cargo's test discovery off and names this file the
//! crate's one test target, so a file under `tests/` is compiled only once
//! it is declared here, which the workspace's `suite_declaration_tests`
//! holds every crate to.

// What suites share, and the modules of single suites: a suite's own file
// is not a directory module, so a module beside it is declared here.
pub mod common;
pub mod disjoint_serde;
pub mod disjoint_variant_names;
pub mod serial_keys;
pub mod variant_names;

pub mod column_types_tests;
pub mod created_range_tests;
pub mod created_schema_tests;
pub mod disjoint_tests;
pub mod disjoint_unkinded_tests;
pub mod domain_tests;
pub mod enum_array_tests;
pub mod enums_keywords_tests;
pub mod expression_tests;
pub mod features_tests;
pub mod format_tests;
pub mod interval_tests;
pub mod models_tests;
pub mod one_of_tests;
pub mod pipeline_tests;
pub mod relations_tests;
pub mod schema_qualifier_tests;
pub mod serde_tests;
pub mod serde_values_tests;
pub mod sql_schema_tests;
pub mod transform_tests;
pub mod types_tests;
