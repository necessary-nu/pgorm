//! The files pgorm-codegen generates from `schema.sql`, the statements
//! `create_table_from_entity` and `Schema::create_one_of` write for the
//! comment fixture, declared as the module tree they are written into: the
//! comment's module holds its `#[derive(OneOf)]` enum, whose variants name
//! their targets' modules through `super::`. pgorm-codegen's
//! `tests/one_of_tests.rs` holds the writer's output to them.

pub mod comment;
pub mod long_names;
pub mod photo;
pub mod post;
pub mod self_reference;
