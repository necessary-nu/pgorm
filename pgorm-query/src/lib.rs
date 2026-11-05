#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_debug_implementations)]
// A SQL builder renders text; nothing it does needs the unsafe half of the
// language. The one block that existed compared trait-object vtable addresses
// for identifier equality, which `TypeId` answers with a guarantee behind it
// (`[spec:pgorm:def:sql.types+9]`).
#![forbid(unsafe_code)]

//! # pgorm-query
//!
//! A dynamic, PostgreSQL-only SQL query builder: expressions, queries and
//! schema statements built as abstract syntax trees through an ergonomic
//! API, rendered against the real PostgreSQL grammar.
//!
//! pgorm-query is the query layer of [pgorm](https://github.com/necessary-nu/pgorm),
//! an async PostgreSQL ORM for Rust, and lives in the same repository. It is
//! a hard fork of [SeaQuery](https://github.com/SeaQL/sea-query) by SeaQL,
//! rebuilt around a single database: everything non-Postgres is deleted, and
//! statements the server would reject are unrepresentable where the type
//! system can make them so.
//!
//! ## Install
//!
//! ```toml
//! # Cargo.toml
//! [dependencies]
//! pgorm-query = "0"
//! ```
//!
//! ### Feature flags
//!
//! Macro: `derive` `attr`
//!
//! Target release: `pg-19`. A build targets PostgreSQL 18 unless it says
//! otherwise, and there is no feature for 18; `pg-19` targets 19
//! ([`POSTGRES_TARGET`]). It only adds: what 19 alone accepts is built behind
//! it, and every 18 render stays as it is.
//!
//! Type support is unconditional: `jiff`, `serde_json`, `rust_decimal`, `uuid`,
//! `ipnetwork`, `mac_address`, `pgvector`, Postgres arrays and intervals.
//!
//! ## Usage
//!
//! Table of Content
//!
//! 1. Basics
//!
//!     1. [SqlName](#iden)
//!     1. [Expression](#expression)
//!     1. [Condition](#condition)
//!     1. [Statement Builders](#statement-builders)
//!
//! 1. Query Statement
//!
//!     1. [Query Select](#query-select)
//!     1. [Query Insert](#query-insert)
//!     1. [Query Update](#query-update)
//!     1. [Query Delete](#query-delete)
//!
//! 1. Advanced
//!     1. [Aggregate Functions](#aggregate-functions)
//!     1. [Casting](#casting)
//!     1. [Named Function](#named-function)
//!
//! 1. Schema Statement
//!
//!     1. [Table Create](#table-create)
//!     1. [Table Alter](#table-alter)
//!     1. [Table Drop](#table-drop)
//!     1. [Table Rename](#table-rename)
//!     1. [Table Truncate](#table-truncate)
//!     1. [Foreign Key Create](#foreign-key-create)
//!     1. [Foreign Key Drop](#foreign-key-drop)
//!     1. [Index Create](#index-create)
//!     1. [Index Drop](#index-drop)
//!
//! ### Motivation
//!
//! Why would you want to use a dynamic query builder?
//!
//! 1. Parameter bindings
//!
//! One of the headaches when using raw SQL is parameter binding. With pgorm-query you can:
//!
//! ```
//! # use pgorm_query::{*, tests_cfg::*};
//! assert_eq!(
//!     Query::select()
//!         .column(Character::Character)
//!         .from(Character::Table)
//!         .and_where(Expr::col(Character::Character).like("A"))
//!         .and_where(Expr::col(Character::Id).is_in([1, 2, 3]))
//!         .build(),
//!     (
//!         r#"SELECT "character" FROM "character" WHERE "character" LIKE $1 AND "id" IN ($2, $3, $4)"#
//!             .to_owned(),
//!         Values(vec![
//!             Value::String(Some(Box::new("A".to_owned()))),
//!             Value::Int(Some(1)),
//!             Value::Int(Some(2)),
//!             Value::Int(Some(3))
//!         ])
//!     )
//! );
//! ```
//!
//! 2. Dynamic query
//!
//! You can construct the query at runtime based on user inputs:
//!
//! ```
//! # use pgorm_query::{*, tests_cfg::*};
//! Query::select()
//!     .column(Character::Character)
//!     .from(Character::Table)
//!     .conditions(
//!         // some runtime condition
//!         true,
//!         // if condition is true then add the following condition
//!         |q| {
//!             q.and_where(Expr::col(Character::Id).eq(1));
//!         },
//!         // otherwise leave it as is
//!         |q| {},
//!     );
//! ```
//!
//! ### SqlName
//!
//! `SqlName` is a trait for identifiers used in any query statement.
//!
//! Commonly implemented by Enum where each Enum represents a table found in a database,
//! and its variants include table name and column name.
//!
//! [`SqlName::unquoted()`] must be implemented to provide a mapping between Enum variants and its
//! corresponding string value.
//!
//! ```rust
//! use pgorm_query::*;
//!
//! // For example Character table with column id, character, font_size...
//! pub enum Character {
//!     Table,
//!     Id,
//!     FontId,
//!     FontSize,
//! }
//!
//! // Mapping between Enum variant and its corresponding string value
//! impl SqlName for Character {
//!     fn unquoted(&self, s: &mut dyn std::fmt::Write) {
//!         write!(
//!             s,
//!             "{}",
//!             match self {
//!                 Self::Table => "character",
//!                 Self::Id => "id",
//!                 Self::FontId => "font_id",
//!                 Self::FontSize => "font_size",
//!             }
//!         )
//!         .unwrap();
//!     }
//! }
//! ```
//!
//! If you're okay with running another procedural macro, you can activate
//! the `derive` or `attr` feature on the crate to save you some boilerplate.
//! For more usage information, look at
//! [the derive examples](https://github.com/necessary-nu/pgorm/tree/main/pgorm-query/pgorm-query-derive/tests/pass)
//! or [the attribute examples](https://github.com/necessary-nu/pgorm/tree/main/pgorm-query/pgorm-query-attr/tests/pass).
//!
//! ```rust
//! #[cfg(feature = "derive")]
//! use pgorm_query::SqlName;
//!
//! // This will implement SqlName exactly as shown above
//! #[derive(SqlName)]
//! enum Character {
//!     Table,
//! }
//! assert_eq!(Character::Table.to_string(), "character");
//!
//! // You can also derive a unit struct
//! #[derive(SqlName)]
//! struct Glyph;
//! assert_eq!(Glyph.to_string(), "glyph");
//! ```
//!
//! ```rust
//! #[cfg(feature = "attr")]
//! # fn test() {
//! use pgorm_query::{enum_def, SqlName};
//!
//! #[enum_def]
//! struct Character {
//!     pub foo: u64,
//! }
//!
//! // It generates the following along with SqlName impl
//! # let not_real = || {
//! enum CharacterName {
//!     Table,
//!     Foo,
//! }
//! # };
//!
//! assert_eq!(CharacterName::Table.to_string(), "character");
//! assert_eq!(CharacterName::Foo.to_string(), "foo");
//! # }
//! # #[cfg(feature = "attr")]
//! # test();
//! ```
//!
//!
//! ### Expression
//!
//! Use [`Expr`] to construct select, join, where and having expression in query.
//!
//! ```rust
//! # use pgorm_query::{*, tests_cfg::*};
//! # fn main() -> pgorm_query::error::Result<()> {
//! assert_eq!(
//!     Query::select()
//!         .column(Character::Character)
//!         .from(Character::Table)
//!         .and_where(
//!             Expr::expr(Expr::col(Character::FontSize).add(1))
//!                 .mul(2)
//!                 .eq(Expr::expr(Expr::col(Character::FontSize).div(2)).sub(1))
//!         )
//!         .and_where(
//!             Expr::col(Character::FontSize).in_subquery(
//!                 Query::select()
//!                     .expr(Expr::template("ln($1 ^ $2)", [2.4, 1.2])?)
//!                     .take()
//!             )
//!         )
//!         .and_where(
//!             Expr::col(Character::Character)
//!                 .like("D")
//!                 .and(Expr::col(Character::Character).like("E"))
//!         )
//!         .to_string(),
//!     [
//!         r#"SELECT "character" FROM "character""#,
//!         r#"WHERE ("font_size" + 1) * 2 = ("font_size" / 2) - 1"#,
//!         r#"AND "font_size" IN (SELECT ln(2.4 ^ 1.2))"#,
//!         r#"AND ("character" LIKE 'D' AND "character" LIKE 'E')"#,
//!     ]
//!     .join(" ")
//! );
//! # Ok(())
//! # }
//! ```
//!
//! ### Condition
//!
//! If you have complex conditions to express, you can use the [`Condition`] builder,
//! usable for [`ConditionalStatement::cond_where`] and [`SelectStatement::cond_having`].
//!
//! ```
//! # use pgorm_query::{*, tests_cfg::*};
//! assert_eq!(
//!     Query::select()
//!         .column(Character::Id)
//!         .from(Character::Table)
//!         .cond_where(
//!             Condition::any()
//!                 .add(
//!                     Condition::all()
//!                         .add(Expr::col(Character::FontSize).is_null())
//!                         .add(Expr::col(Character::Character).is_null())
//!                 )
//!                 .add(
//!                     Condition::all()
//!                         .add(Expr::col(Character::FontSize).is_in([3, 4]))
//!                         .add(Expr::col(Character::Character).like("A%"))
//!                 )
//!         )
//!         .to_string(),
//!     [
//!         r#"SELECT "id" FROM "character""#,
//!         r#"WHERE"#,
//!         r#"("font_size" IS NULL AND "character" IS NULL)"#,
//!         r#"OR"#,
//!         r#"("font_size" IN (3, 4) AND "character" LIKE 'A%')"#,
//!     ]
//!     .join(" ")
//! );
//! ```
//!
//! Nested conditions compose the same way:
//!
//! ```
//! # use pgorm_query::{*, tests_cfg::*};
//! Query::select().cond_where(
//!     Condition::any()
//!         .add(Expr::col(Character::FontSize).is_in([3, 4]))
//!         .add(
//!             Condition::all()
//!                 .add(Expr::col(Character::FontSize).is_null())
//!                 .add(Expr::col(Character::Character).like("A%")),
//!         ),
//! );
//! ```
//!
//! ### Statement Builders
//!
//! Statements are divided into 2 categories: Query and Schema.
//!
//! Every statement has the value-inlined rendering — its
//! [`Display`](std::fmt::Display), reached as `to_string()` — which writes each
//! value into the SQL as an escaped literal. What differs is whether a statement
//! *also* has a bound rendering, and that follows from whether it binds anything:
//!
//! - **Query statements** — SELECT, INSERT, UPDATE and DELETE —
//!   bind their values. They expose `build() -> (String, Values)`, which emits
//!   `$N` placeholders and hands back the values, plus `build_collect(sink)` for
//!   a sink the caller owns. `build` is what you execute: the driver sends the
//!   values over the binary protocol, so nothing is re-parsed as SQL.
//! - **`CREATE TYPE ... AS ENUM` and `ALTER TYPE`** bind their enum labels, so
//!   they expose `build()` and `build_collect(sink)` too — but PostgreSQL takes
//!   no bind parameter in DDL, so their `$N` rendering is for inspection and the
//!   inlined one is what you execute.
//! - **Every other schema statement** — schema, table, index, foreign-key,
//!   sequence, view, materialized view, `CREATE TABLE AS`, comment, extension,
//!   `DROP TYPE` — binds nothing that can reach a placeholder sink: a view's query
//!   is stored as SQL text, so its values are written into it.
//!   The inlined rendering is the only one they have, and it is their `Display`.
//!
//! ```rust
//! # use pgorm_query::*;
//! # trait ExampleQueryBuilder {
//! fn build(&self) -> (String, Values);
//!
//! fn to_string(&self) -> String;
//! # }
//! ```
//!
//! The inlined form escapes what it writes, so it is not an injection hole; it is
//! the wrong choice because it re-parses on the server and loses the type pinning
//! a bound value carries. Reach for it in tests, goldens and logs.
//!
//! ### Query Select
//!
//! ```rust
//! # use pgorm_query::{*, tests_cfg::*};
//! let query = Query::select()
//!     .column(Character::Character)
//!     .column((Font::Table, Font::Name))
//!     .from(Character::Table)
//!     .left_join(Font::Table, Expr::col((Character::Table, Character::FontId)).equals((Font::Table, Font::Id)))
//!     .and_where(Expr::col(Character::FontSize).is_in([3, 4]))
//!     .and_where(Expr::col(Character::Character).like("A%"))
//!     .to_owned();
//!
//! assert_eq!(
//!     query.to_string(),
//!     r#"SELECT "character", "font"."name" FROM "character" LEFT JOIN "font" ON "character"."font_id" = "font"."id" WHERE "font_size" IN (3, 4) AND "character" LIKE 'A%'"#
//! );
//! ```
//!
//! ### Query Insert
//!
//! ```rust
//! # use pgorm_query::{*, tests_cfg::*};
//! let query = Query::insert()
//!     .into_table(Character::Table)
//!     .columns([Character::Id, Character::Character])
//!     .values_panic([1.into(), "A".into()])
//!     .to_owned();
//!
//! assert_eq!(
//!     query.to_string(),
//!     r#"INSERT INTO "character" ("id", "character") VALUES (1, 'A')"#
//! );
//! ```
//!
//! ### Query Update
//!
//! ```rust
//! # use pgorm_query::{*, tests_cfg::*};
//! let query = Query::update()
//!     .table(Character::Table)
//!     .values([(Character::Character, "A".into())])
//!     .and_where(Expr::col(Character::Id).eq(1))
//!     .to_owned();
//!
//! assert_eq!(
//!     query.to_string(),
//!     r#"UPDATE "character" SET "character" = 'A' WHERE "id" = 1"#
//! );
//! ```
//!
//! ### Query Delete
//!
//! ```rust
//! # use pgorm_query::{*, tests_cfg::*};
//! let query = Query::delete()
//!     .from_table(Character::Table)
//!     .cond_where(
//!         Condition::any()
//!             .add(Expr::col(Character::Id).lt(1))
//!             .add(Expr::col(Character::Id).gt(10)),
//!     )
//!     .to_owned();
//!
//! assert_eq!(
//!     query.to_string(),
//!     r#"DELETE FROM "character" WHERE "id" < 1 OR "id" > 10"#
//! );
//! ```
//!
//! ### Aggregate Functions
//!
//! `max`, `min`, `sum`, `avg`, `count` etc
//!
//! ```rust
//! # use pgorm_query::{*, tests_cfg::*};
//! let query = Query::select()
//!     .expr(Func::sum(Expr::col(Character::Id)))
//!     .from(Character::Table)
//!     .to_owned();
//!
//! assert_eq!(
//!     query.to_string(),
//!     r#"SELECT SUM("id") FROM "character""#
//! );
//! ```
//!
//! ### Casting
//!
//! ```rust
//! # use pgorm_query::{*, tests_cfg::*};
//! let query = Query::select()
//!     .expr(Expr::val("hello").cast_as(Name::runtime("my_type")))
//!     .to_owned();
//!
//! assert_eq!(
//!     query.to_string(),
//!     r#"SELECT CAST('hello' AS my_type)"#
//! );
//! ```
//!
//! ### Named Function
//!
//! ```rust
//! # use pgorm_query::{*, tests_cfg::*};
//! struct MyFunction;
//!
//! impl SqlName for MyFunction {
//!     fn unquoted(&self, s: &mut dyn std::fmt::Write) {
//!         write!(s, "my_function").unwrap();
//!     }
//! }
//!
//! let query = Query::select()
//!     .expr(Func::named(MyFunction).arg(Expr::val("hello")))
//!     .to_owned();
//!
//! assert_eq!(
//!     query.to_string(),
//!     r#"SELECT my_function('hello')"#
//! );
//! ```
//!
//! ### Table Create
//!
//! ```rust
//! # use pgorm_query::{*, tests_cfg::*};
//! let table = Table::create(Character::Table)
//!     .col(ColumnDef::new(Character::Id).integer().not_null())
//!     .col(ColumnDef::new(Character::Character).string().not_null())
//!     .to_owned();
//!
//! assert_eq!(
//!     table.to_string(),
//!     r#"CREATE TABLE "character" ( "id" integer NOT NULL, "character" varchar NOT NULL )"#
//! );
//! ```
//!
//! ### Table Alter
//!
//! ```rust
//! # use pgorm_query::{*, tests_cfg::*};
//! let table = Table::alter(Character::Table).add_column(
//!     ColumnDef::new(Name::runtime("new_col"))
//!         .integer()
//!         .not_null()
//!         .default(100),
//! );
//!
//! assert_eq!(
//!     table.to_string(),
//!     r#"ALTER TABLE "character" ADD COLUMN "new_col" integer NOT NULL DEFAULT 100"#
//! );
//! ```
//!
//! ### Table Drop
//!
//! ```rust
//! # use pgorm_query::{*, tests_cfg::*};
//! let table = Table::drop(Character::Table);
//!
//! assert_eq!(
//!     table.to_string(),
//!     r#"DROP TABLE "character""#
//! );
//! ```
//!
//! ### Table Rename
//!
//! ```rust
//! # use pgorm_query::{*, tests_cfg::*};
//! let table = Table::rename(Character::Table, Name::runtime("character_new"));
//!
//! assert_eq!(
//!     table.to_string(),
//!     r#"ALTER TABLE "character" RENAME TO "character_new""#
//! );
//! ```
//!
//! ### Table Truncate
//!
//! ```rust
//! # use pgorm_query::{*, tests_cfg::*};
//! let table = Table::truncate(Character::Table);
//!
//! assert_eq!(
//!     table.to_string(),
//!     r#"TRUNCATE TABLE "character""#
//! );
//! ```
//!
//! ### Foreign Key Create
//!
//! ```rust
//! # use pgorm_query::{*, tests_cfg::*};
//! let foreign_key = ForeignKey::create(
//!     Character::Table,
//!     Character::Id,
//!     Character::Table,
//!     Character::Id,
//! )
//! .name(Name::runtime("FK_character_id"))
//! .to_owned();
//!
//! assert_eq!(
//!     foreign_key.to_string(),
//!     r#"ALTER TABLE "character" ADD CONSTRAINT "FK_character_id" FOREIGN KEY ("id") REFERENCES "character" ("id")"#
//! );
//! ```
//!
//! ### Constraint Drop
//!
//! A foreign key is dropped by name, as a constraint of any kind is.
//!
//! ```rust
//! # use pgorm_query::{*, tests_cfg::*};
//! let drop = Table::alter(Character::Table).drop_constraint(Name::runtime("FK_character_id"));
//!
//! assert_eq!(
//!     drop.to_string(),
//!     r#"ALTER TABLE "character" DROP CONSTRAINT "FK_character_id""#
//! );
//! ```
//!
//! ### Index Create
//!
//! ```rust
//! # use pgorm_query::{*, tests_cfg::*};
//! let index = Index::create(Character::Table, Character::Id)
//!     .name(Name::runtime("idx-character-id"))
//!     .to_owned();
//!
//! assert_eq!(
//!     index.to_string(),
//!     r#"CREATE INDEX "idx-character-id" ON "character" ("id")"#
//! );
//! ```
//!
//! ### Index Drop
//!
//! ```rust
//! # use pgorm_query::{*, tests_cfg::*};
//! let index = Index::drop(Name::runtime("idx-character-id"))
//!     .table(Character::Table)
//!     .to_owned();
//!
//! assert_eq!(
//!     index.to_string(),
//!     r#"DROP INDEX "idx-character-id""#
//! );
//! ```
//!
//! ## The public surface
//!
//! Everything the crate exports is listed by name at the crate root, in one
//! block grouped by what the items are for: names, expressions, values, query
//! statements, schema statements, rendering. The modules behind that list are
//! private, so an internal type cannot become API by being declared `pub` in a
//! module someone can reach — `use pgorm_query::*` and the list are the same
//! set. Three modules stay public because a path through them is the spelling:
//! [`error`] (the crate's `Error` and `Result`), [`extension`] (PostgreSQL's
//! `CREATE EXTENSION` and `CREATE TYPE` surface, deliberately not flattened
//! into the root) and [`value`] (whose `value::with_array::NotU8` pgorm's
//! derives name in generated code).
//!
//! So a module path into the crate does not compile:
//!
//! ```compile_fail,E0603
//! let _: pgorm_query::types::Name;
//! ```
//!
//! ```compile_fail,E0603
//! let _ = pgorm_query::backend::QueryBuilder;
//! ```
//!
//! ## License
//!
//! Licensed under either of
//!
//! -   Apache License, Version 2.0
//!     ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
//! -   MIT license
//!     ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)
//!
//! at your option.
//!
//! ## Contribution
//!
//! Unless you explicitly state otherwise, any contribution intentionally submitted
//! for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
//! dual licensed as above, without any additional terms or conditions.
//!
//! ## Provenance
//!
//! This crate began as [SeaQuery](https://github.com/SeaQL/sea-query) by SeaQL
//! and its contributors; the fork keeps their license and gratefully builds on
//! their work.

// The crate's own modules are private: the `pub use` block below is the whole
// public surface, item by item, so nothing reaches the API by sitting in a
// module that happens to be reachable. Three modules stay public because a
// path through them is the documented spelling: `error` (the crate's
// `Error`/`Result`), `extension` (PostgreSQL's `CREATE EXTENSION` / `CREATE
// TYPE` surface, deliberately not flattened into the root), and `value`
// (`value::with_array::NotU8`, which pgorm's derives name in generated code).
// [spec:pgorm:req:sql.surface+39]
// [spec:pgorm:req:sql.surface+39/test]    the two `compile_fail,E0603` examples in
// the crate docs above, under "The public surface": a module path into the
// crate does not resolve. `cargo test --doc -p pgorm-query` runs them; nextest
// runs the rest of the rule's tests, in `tests/surface`.
//
// What this list does NOT name is a decision as much as what it does, and the
// constructs outside it are enumerated with their reasons rather than left to
// be discovered — each either deferred with its cost or ruled out with its
// argument. `Expr::raw`, `SqlTemplate` and the raw `FromItem` are the escape
// hatches that keep every one of them reachable, so the boundary is about
// which SQL gets a type here, never about which SQL a caller can send.
// [spec:pgorm:req:sql.scope+23]
mod backend;
mod comment;
mod copy;
mod domain;
pub mod error;
mod expr;
mod expr_trait;
pub mod extension;
mod foreign_key;
mod func;
mod index;
mod json;
mod key;
mod keywords;
mod prepare;
mod query;
mod schema;
mod sequence;
mod table;
mod target;
mod template;
mod token;
mod types;
pub mod value;
mod value_domain;
mod value_identity;
mod value_interval;
mod value_kind;
mod value_range;
mod value_range_text;
mod view;

#[doc(hidden)]
#[cfg(feature = "tests-cfg")]
pub mod tests_cfg;

// The internal shapes the rest of the crate reaches for as `crate::Thing`,
// and which stop at the crate boundary. Everything else a module names that
// way arrives through the public re-exports below. One to a line: the
// compile-fail snapshot in `tests/surface` quotes each item's own line, so
// adding a shape leaves the others' quotations as they are.
pub(crate) use backend::Oper;
pub(crate) use prepare::Write;
pub(crate) use query::ConditionHolder;
pub(crate) use query::InsertValueSource;
pub(crate) use query::JoinExpr;
pub(crate) use query::LockClause;
pub(crate) use query::ReturningItems;
pub(crate) use types::JoinKind;
pub(crate) use types::JoinOn;

// Names: what an identifier position holds, and the conversions into it —
// names themselves, then the table references and the join shape that reads
// them, then column references.
pub use types::{AliasName, IntoName, Name, SqlName, StaticName, TableName, TypeName, alias};
pub use types::{Asterisk, ColumnRef, IntoColumnRef};
pub use types::{Collation, IntoCollation};
pub use types::{FromItem, IntoFromItem, IntoNamedTable, IntoTableName, JoinType, NamedTable};

// Expressions: the operator vocabulary an expression tree is built from, and
// the two trees themselves.
pub use expr::{Expr, SimpleExpr};
pub use expr_trait::{ExprTrait, Subscript};
pub use func::{Func, FuncArgMod, Function, FunctionCall};
pub use json::{
    JsonArray, JsonArrayAgg, JsonArrayQuery, JsonExists, JsonExistsBehavior, JsonInput, JsonKind,
    JsonObject, JsonObjectAgg, JsonParse, JsonQuery, JsonQueryBehavior, JsonSerialize, JsonTest,
    JsonValue, JsonValueBehavior, JsonValueType, SqlJson,
};
pub use json::{
    JsonExistsColumn, JsonNestedColumns, JsonQueryColumn, JsonTable, JsonTableBehavior,
    JsonTableColumn, JsonValueColumn,
};
pub use types::{BinOper, IntoLikeExpr, Keyword, LikeExpr, SubQueryOper, UnOper};

// Values: what a bound parameter carries, the conversions in and out, and
// the type a value of a domain travels as.
pub use key::{IntoBoundary, IntoKey, Key};
pub use value::{
    ArrayType, IntoValueTuple, IpNetwork, MacAddress, Nullable, TryFromValueTuple, Value,
    ValueTuple, ValueTupleError, ValueType, ValueTypeError, Values, Vector,
};
pub use value_domain::base_type;
pub use value_interval::Interval;
pub use value_range::{Multirange, Range, RangeElement, RangeType};
pub use value_range_text::RangeSubtype;

// Query statements: the five DML builders, the clauses they take, and the
// traits that let a caller write against any of them.
pub use query::{
    AnyWithClause, CaseOperand, CaseStatement, CommonTableExpression, Condition,
    ConditionExpression, ConditionType, ConditionalStatement, Cycle, DeleteStatement,
    ExtendedWindow, FrameClause, FrameCurrentRow, FrameExclusion, FrameFollowing, FramePreceding,
    FrameStart, FrameType, Grouping, GroupingElement, GroupingSets, InsertStatement, IntoCondition,
    IntoSubQueryStatement, LockBehavior, LockType, OrderedStatement, OverStatement, Overriding,
    Query, QueryStatementBuilder, RecursiveWithClause, Returning, ReturningClause, ReturningRow,
    SampleMethod, Search, SearchOrder, SelectExpr, SelectStatement, SimpleCaseStatement,
    SubQueryStatement, TableFunction, TableSample, UnionType, UpdateStatement, WindowDefinition,
    WindowFunction, WindowSelectType, WindowStatement, WithClause,
};
pub use query::{
    ConflictAction, ConflictArbiter, ConflictAssignment, ConflictAssignments, ConflictConstraint,
    ConflictElement, ConflictTarget, ConflictUpdate, OnConflict,
};
pub use query::{
    MatchedAction, MergeInsert, MergeStatement, MergeUpdate, NotMatchedAction, PendingMerge,
};

// COPY: the two statements that open COPY's data flow, the formats a statement
// is typed by, and what a COPY TO reads.
pub use copy::{
    CopyBinary, CopyCsv, CopyDelimitedFormat, CopyFormat, CopyFromStatement, CopyIgnore,
    CopyLogVerbosity, CopySource, CopyText, CopyToStatement, IntoCopySource,
};
pub use types::{NullOrdering, Order, OrderExpr};

// Schema statements: the DDL builders and the column vocabulary they share.
pub use comment::{Comment, CommentStatement, CommentTarget};
pub use domain::{
    Domain, DomainAlterStatement, DomainCheck, DomainCreateStatement, DomainDropStatement,
    IntoDomainCheck, PendingDomainAlter,
};
pub use foreign_key::{
    Deferrability, ForeignKey, ForeignKeyAction, ForeignKeyCreateStatement, TableForeignKey,
};
pub use index::{
    Index, IndexColumn, IndexColumnTarget, IndexCreateStatement, IndexDropStatement, IndexKind,
    IndexOrder, IndexType, IntoIndexColumn, TableIndex,
};
pub use schema::{Schema, SchemaCreateStatement, SchemaDropStatement, SchemaRenameStatement};
pub use sequence::{
    PendingSequenceAlter, Sequence, SequenceAlterStatement, SequenceCreateStatement,
    SequenceDropStatement, SequenceOption, SequenceOptions, SequenceRenameStatement, SequenceType,
};
pub use table::TableCreateAsStatement;
pub use table::{
    AddColumnOption, Check, ColumnDef, ColumnRenameStatement, ColumnSpec, ColumnType,
    ConstraintChange, ConstraintDrop, ConstraintRenameStatement, DropBehavior, Enforcement,
    GeneratedKind, IdentityGeneration, IntervalPrecision, IntervalSpec, IntoAddedConstraint,
    IntoCheck, IntoColumnDef, IntoConstraintDrop, IntoKeyColumns, IntoTableKey, NotNullConstraint,
    NotValid, PendingTableAlter, PgInterval, Primary, StringLen, Table, TableAlterOption,
    TableAlterStatement, TableCreateStatement, TableDropOpt, TableDropStatement, TableKey,
    TableRenameStatement, TableTruncateStatement, Unique,
};
pub use view::{
    MaterializedView, MaterializedViewCreateStatement, MaterializedViewDropStatement,
    MaterializedViewRefreshStatement, RecursiveViewCreateStatement, View, ViewCheckOption,
    ViewCreateStatement, ViewDropStatement,
};

// The PostgreSQL release the build targets.
pub use target::POSTGRES_TARGET;

// Rendering: the sink a statement is written into, and the two entry points
// that are not a statement's own `build`.
pub use backend::QueryBuilder;
pub use prepare::{SqlWriter, SqlWriterValues, inject_parameters};
pub use template::SqlTemplate;

// Reachable but not API. Each of these is an internal shape that cannot be
// narrowed to `pub(crate)` today because nothing inside the crate reads it —
// hiding it would make it dead code, and removing dead surface is the deletion
// pass's job, not curation's. They are hidden from the documentation so the
// rendered API is the list above, and they are named here rather than left to
// a module glob so the debt is a list someone can work through.
//
// The statement-dispatch wrappers: no builder produces one and no renderer
// takes one, so they exist only to be matched on by a caller that already has
// the inner statement.
// The SELECT distinct flag: `distinct`/`distinct_on` are how a caller sets it,
// and `SelectDistinct::All` has no builder at all.
#[doc(hidden)]
pub use query::SelectDistinct;
// The lexer `inject_parameters` and `SqlTemplate` read SQL text with. Its
// classification accessors are exercised only by the conformance suite.
#[doc(hidden)]
pub use token::{Token, Tokenizer};

#[cfg(feature = "derive")]
pub use pgorm_query_derive::{SqlName, StaticName};

#[cfg(feature = "attr")]
pub use pgorm_query_attr::enum_def;
