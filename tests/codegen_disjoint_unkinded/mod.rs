//! The files pgorm-codegen generates from `schema.sql`, tables keyed by a
//! foreign key from their primary key onto a base's and no kind column, with
//! the shop's and the account's tables named as unkinded disjoint entities,
//! declared as the module tree they are written into. pgorm-codegen's
//! `tests/disjoint_unkinded_tests.rs` holds the writer's output to them.

pub mod account;
pub mod company;
pub mod person;
pub mod shop;
pub mod storefront;
pub mod website;
