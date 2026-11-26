//! pgorm-macros's integration tests: one binary, every suite a module of it.
//! The manifest turns cargo's test discovery off and names this file the
//! crate's one test target, so a file under `tests/` is compiled only once
//! it is declared here, which the workspace's `suite_declaration_tests`
//! holds every crate to.

pub mod compile_fail_test;
pub mod derive_active_enum_test;
pub mod derive_boilerplate_test;
pub mod derive_code_values_test;
pub mod derive_disjoint_test;
pub mod derive_entity_model_column_name_test;
pub mod derive_entity_model_test;
pub mod derive_iteration_test;
pub mod derive_json_value_type_test;
pub mod derive_model_test;
pub mod derive_names_test;
pub mod derive_one_of_test;
pub mod derive_relation_test;
pub mod derive_suite_test;
pub mod derive_value_type_test;
