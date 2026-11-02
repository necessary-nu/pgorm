//! The files pgorm-codegen generates from `schema.sql`, a table of every kind
//! of column type it reads, declared as the module tree they are written
//! into: the entity names its enum and its range type's newtype through
//! `super::`. pgorm-codegen's `tests/column_types_tests.rs` holds the writer's
//! output to them.

pub mod pgorm_active_enums;
pub mod pgorm_range_types;
pub mod specimen;
