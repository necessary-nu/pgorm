//! The files pgorm-codegen generates from `schema.sql`, declared as the
//! module tree they are written into. The sale's `Relation` enum and the
//! `Outlet` enum each have three variants ending `Shop`, which clippy's
//! `enum_variant_names` lints in a module that is not exported, as this one
//! is not.

pub mod closed_shop;
pub mod pgorm_active_enums;
pub mod pop_up_shop;
pub mod sale;
pub mod web_shop;
