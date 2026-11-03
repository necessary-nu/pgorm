//! The files pgorm-codegen generates from the vehicle's part of
//! `tests/codegen_disjoint/schema.sql` under `WithSerde::Both`, declared as
//! the module tree they are written into, so the serde derives the writer
//! gives the base's `Model`, its disjoint enum and its kind type are derives
//! their fields satisfy.

pub mod car;
pub mod truck;
pub mod vehicle;
