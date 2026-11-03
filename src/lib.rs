//! An async, dynamic ORM for PostgreSQL, built on tokio-postgres and deadpool.
//!
//! Entities are declared as Rust types, queries are composed through the
//! [`pgorm_query`] builder and executed with their values bound as parameters
//! rather than interpolated.
//!
//! # PostgreSQL release
//!
//! A build targets PostgreSQL 18 unless it asks for 19 with the `pg-19`
//! feature; there is no feature for 18 and no support for anything older. The
//! feature only adds: what 19 alone accepts is built behind it, and nothing an
//! 18 build has changes, so a crate elsewhere in the dependency graph turning
//! it on cannot break this one's statements. Where the two releases' servers
//! answer one statement differently, the documentation gives both answers.
//! [`pgorm_query::POSTGRES_TARGET`] says which release a build targets.
//!
//! # Renaming the dependency
//!
//! Every derive expands to call-site-relative `pgorm::...` paths, so the name
//! `pgorm` has to resolve where the derive is written. Renaming the dependency
//! in `Cargo.toml` (`my_orm = { package = "pgorm", .. }`) therefore breaks the
//! derives with a loud `E0433` naming the unresolved `pgorm`. Restore the name
//! with an alias in the module the entities live in, or at the crate root:
//!
//! ```ignore
//! use my_orm as pgorm;
//! ```
//!
//! Renaming `pgorm-query` needs nothing — the derives reach it through this
//! crate's [`pgorm_query`] re-export. `pgorm-migration` is subject to the same
//! rule as `pgorm`, since its `MigrationName` derive names `pgorm_migration`.
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
#![deny(
    missing_debug_implementations,
    clippy::missing_panics_doc,
    clippy::unwrap_used,
    clippy::print_stderr,
    clippy::print_stdout
)]

// A module with no items, so that rustdoc renders the guide beside the API it
// describes and compiles its examples as doctests.
#[doc = include_str!("../COMING_FROM_SEAORM.md")]
pub mod coming_from_seaorm {}
mod database;
mod docs;
/// Module for the Entity type and operations
pub mod entity;
/// Error types for all database operations
pub mod error;
/// This module performs execution of queries on a Model or ActiveModel
mod executor;
/// Holds types and methods to perform metric collection
pub mod metric;
// The module documents itself. A second doc comment here would not merely
// duplicate the first line of it: an outer attribute on the `mod` item makes
// rustdoc resolve the whole merged doc in *this* scope, where none of the
// module's own types are, so every intra-doc link inside it breaks.
pub mod pipeline;
/// Holds types and methods to perform queries
pub mod query;
/// Holds types that defines the schemas of an Entity
pub mod schema;
#[doc(hidden)]
#[cfg(all(feature = "macros", feature = "tests-cfg"))]
pub mod tests_cfg;
mod util;

pub use database::*;
pub use entity::*;
pub use error::*;
pub use executor::*;
pub use query::*;
pub use schema::*;

/// Each derive is named for the trait it implements or the item it writes,
/// so most share a name with a trait the `entity` glob brings; a derive and a
/// trait live in different namespaces, and one `use` brings both.
/// `SqlName` and `StaticName` arrive through that glob too, `entity` pairing
/// the traits with pgorm-macros' derives.
// [spec:pgorm:req:macros.derive.names+2]
// [spec:pgorm:def:disjoint.declare+5]
// [spec:pgorm:def:disjoint.one-of+1]
#[cfg(feature = "macros")]
pub use pgorm_macros::{
    ActiveEnum, Column, CreatedRange, Disjoint, Entity, EntityTrait, FromQueryResult,
    IntoActiveModel, JsonValueType, Model, OneOf, PartialModel, Relation, ValueType,
};
#[cfg(feature = "macros")]
pub use tokio_postgres::row::RowIndex;

/// Hold a raw SQL string literal to the PostgreSQL grammar at compile time,
/// expanding to the literal unchanged.
///
/// libpg_query — the PostgreSQL server's own parser, which performs the
/// check — is compiled into every build regardless, since `SelectorRaw`
/// pagination parses raw statements with it at runtime. The check is syntax
/// only: unknown tables and columns pass.
///
/// ```
/// # use pgorm::sql;
/// const BY_ID: &str = sql!(r#"SELECT "id", "name" FROM "cake" WHERE "id" = $1"#);
/// ```
///
/// The escape hatches that take SQL as text are its call sites, and they are
/// findable rather than listed: on a connection, the rule is the
/// [`SqlText`] bound — every [`ConnectionTrait`] method taking
/// `&T where T: SqlText`, which is [`execute`], [`execute_raw`],
/// [`query_one`], [`query_opt`], [`query_all`] and [`query_raw`] — plus
/// [`batch_execute`], which takes its `&str` directly because it runs a whole
/// script through the simple-query protocol. Away from a connection a raw
/// statement travels as SQL text paired with its values, and the entry points
/// are [`Select::from_raw_sql`], the [`SelectorRaw`] constructors
/// ([`from_statement`], [`into_tuple`], [`with_columns`]), the [`DecodeRaw`]
/// impl on a `(sql, values)` pair ([`into_model`], [`DecodeRaw::into_tuple`],
/// [`into_values`]), and [`FromQueryResult::find_by_statement`]. Migration
/// bodies are the last of them.
///
/// [`SqlText`]: crate::SqlText
/// [`ConnectionTrait`]: crate::ConnectionTrait
/// [`execute`]: crate::ConnectionTrait::execute
/// [`execute_raw`]: crate::ConnectionTrait::execute_raw
/// [`query_one`]: crate::ConnectionTrait::query_one
/// [`query_opt`]: crate::ConnectionTrait::query_opt
/// [`query_all`]: crate::ConnectionTrait::query_all
/// [`query_raw`]: crate::ConnectionTrait::query_raw
/// [`batch_execute`]: crate::ConnectionTrait::batch_execute
/// [`Select::from_raw_sql`]: crate::Select::from_raw_sql
/// [`SelectorRaw`]: crate::SelectorRaw
/// [`from_statement`]: crate::SelectorRaw::from_statement
/// [`into_tuple`]: crate::SelectorRaw::into_tuple
/// [`with_columns`]: crate::SelectorRaw::with_columns
/// [`DecodeRaw`]: crate::DecodeRaw
/// [`into_model`]: crate::DecodeRaw::into_model
/// [`DecodeRaw::into_tuple`]: crate::DecodeRaw::into_tuple
/// [`into_values`]: crate::DecodeRaw::into_values
/// [`FromQueryResult::find_by_statement`]: crate::FromQueryResult::find_by_statement
// [spec:pgorm:def:macros.sql+5]
pub use pgorm_sql_macro::sql;

/// Compile a PRQL string literal to PostgreSQL SQL at build time, expanding
/// to `(&'static str, Values)` for the raw-SQL entry points.
///
/// The text sibling of [`pipeline`]: the same prqlc
/// compiler, the same libpg_query oracle over what it emits, but for
/// queries known whole at compile time. The arguments after the literal are
/// converted via `Into<`[`Value`]`>` in placeholder order,
/// and the macro refuses at compile time any PRQL prqlc rejects, any
/// emitted SQL the PostgreSQL grammar rejects, an argument count that does
/// not match the `$N` placeholders, or a gap in their numbering.
///
/// ```
/// use pgorm::prql;
///
/// let min_total = 5_i64;
/// let (sql, values) = prql!("from invoice | filter total > $1 | take 5", min_total);
/// assert_eq!(sql, "SELECT * FROM invoice WHERE total > $1 LIMIT 5");
/// assert_eq!(values.0.len(), 1);
/// ```
///
/// The result lands on the same escape hatches as [`sql!`](crate::sql):
///
/// ```no_run
/// # use pgorm::{prql, DecodeRaw, FromQueryResult};
/// # #[derive(FromQueryResult)]
/// # struct Invoice { total: i64 }
/// # async fn demo(db: pgorm::DatabaseConnection) -> Result<(), pgorm::Error> {
/// let (sql, values) = prql!("from invoice | filter total > $1 | take 5", 100_i64);
/// let rows: Vec<Invoice> = (sql, values).into_model::<Invoice>().all(&db).await?;
/// # Ok(())
/// # }
/// ```
// [spec:pgorm:def:macros.prql]
pub use pgorm_sql_macro::prql;

pub use pgorm_query;
pub use pgorm_query::{AliasName, Values, alias};

pub use tokio_postgres::types;
