#![doc = include_str!("../README.md")]
// A library reports through `tracing`: its caller's stdout may be a build
// script's, which cargo reads as directives.
#![deny(clippy::print_stderr, clippy::print_stdout, clippy::dbg_macro)]

mod entity;
mod error;
mod sql_schema;
mod util;

pub use entity::*;
pub use error::*;
// [spec:pgorm:req:codegen.features]
#[cfg(feature = "rust")]
pub use sql_schema::entities_from_sql;
pub use sql_schema::parse_schema;

#[cfg(all(test, feature = "rust"))]
mod tests_cfg;
