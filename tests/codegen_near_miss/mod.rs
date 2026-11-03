//! The files pgorm-codegen generates from `schema.sql`, a base and a kind's
//! table one step short of a disjoint entity (the base has no generated key
//! columns to its kinds), declared as the module tree they are written into.
//! They generate as the ordinary entities they are, the base keeping its
//! composite unique keys, the one the kind's foreign key references among
//! them. pgorm-codegen's `tests/disjoint_tests.rs` holds the writer's output
//! to them.

pub mod book;
pub mod pgorm_active_enums;
pub mod shelf;
