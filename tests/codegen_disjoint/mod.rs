//! The files pgorm-codegen generates from `schema.sql`, the statements
//! `Schema::create_disjoint` writes for the vehicle and listing fixtures,
//! declared as the module tree they are written into: each base's module
//! holds its `#[derive(Disjoint)]` enum, whose variants name their kinds'
//! modules through `super::`. pgorm-codegen's `tests/disjoint_tests.rs` holds
//! the writer's output to them.

pub mod car;
pub mod flat;
pub mod listing;
pub mod truck;
pub mod vehicle;
