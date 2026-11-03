//! The files pgorm-codegen generates from `schema.sql`, the store's tables
//! named as an unkinded disjoint entity, declared as the module tree they are
//! written into. Every variant of each disjoint enum ends with the enum's
//! name, which clippy's `enum_variant_names` lints in a module that is not
//! exported, as this one is not.

pub mod pop_up_shop;
pub mod pop_up_store;
pub mod shop;
pub mod store;
pub mod web_shop;
pub mod web_store;
