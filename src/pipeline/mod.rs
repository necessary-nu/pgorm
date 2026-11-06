//! A PRQL-shaped pipeline query API, compiled through prqlc's PL AST.
//!
//! A [`Pipeline`] is a sequence of relation-to-relation transforms in the
//! shape of a [PRQL](https://prql-lang.org) query: instead of assembling
//! `SELECT` clauses, you append `filter`, `derive`, `group`/`aggregate`,
//! `window`, `sort` and `take` stages, and clause placement falls out of
//! position — a filter after an aggregation becomes `HAVING`, a filter after
//! a window wraps the pipeline in a CTE. Because every stage maps relation
//! to relation, any `fn(Pipeline) -> Pipeline` is a reusable, composable
//! query scope.
//!
//! Columns are the entity's own column enums, names the pipeline introduces
//! are [`alias`] tokens bound once and referred to by value, and constants
//! are Rust literals — so a query reads about as densely as the PRQL it
//! compiles to. The two newest fruit per cake:
//!
//! ```
//! use pgorm::pipeline::{ExprOps, Pipeline, alias, by, row_number};
//! use pgorm::tests_cfg::fruit::{self, Column as F};
//!
//! let rn = alias("rn");
//! let (sql, _) = Pipeline::from(fruit::Entity)
//!     .window(row_number().as_(rn), by(F::CakeId).sort_by(F::Id.desc()))
//!     .filter(rn.lte(2))
//!     .select((F::CakeId, F::Name, rn))
//!     .into_sql()?;
//!
//! assert_eq!(
//!     sql,
//!     "WITH table_0 AS (SELECT cake_id, name, ROW_NUMBER() OVER \
//!      (PARTITION BY cake_id ORDER BY id DESC) AS rn FROM fruit) \
//!      SELECT cake_id, name, rn FROM table_0 WHERE rn <= 2"
//! );
//! # Ok::<_, pgorm::pipeline::PipelineError>(())
//! ```
//!
//! Runtime values are the one thing that never reads as a literal: they
//! enter through the [`Binder`] that the `_with` form of each transform
//! hands its closure. [`Binder::bind`] mints the `$N` placeholder and
//! records the value in one step, and the returned expression is branded so
//! it cannot leak into a different pipeline.
//!
//! ```
//! use pgorm::pipeline::{ExprOps, JoinSide, Pipeline, alias, count_rows};
//! use pgorm::tests_cfg::{cake, fruit};
//!
//! let fruits = alias("fruits");
//! let (sql, values) = Pipeline::from(cake::Entity)
//!     .join(
//!         JoinSide::Left,
//!         fruit::Entity,
//!         cake::Column::Id.eq(fruit::Column::CakeId),
//!     )
//!     .group(cake::Column::Name)
//!     .aggregate(count_rows().as_(fruits))
//!     .filter_with(|binder| fruits.gt(binder.bind(1_i64)))
//!     .sort(fruits.desc())
//!     .take(10)
//!     .into_sql()?;
//!
//! assert!(sql.contains("HAVING"));
//! assert_eq!(values.0.len(), 1);
//! # Ok::<_, pgorm::pipeline::PipelineError>(())
//! ```
//!
//! Pipelines also compose with each other: a whole [`Pipeline`] is a
//! relation, so it can be the source of [`from`](Pipeline::from), the
//! operand of a [`join`](Pipeline::join), or one side of
//! [`append`](Pipeline::append) / [`intersect`](Pipeline::intersect) /
//! [`remove`](Pipeline::remove). Embedding consumes the pipeline by value
//! and merges its bound values into the consumer's, placeholders renumbered
//! to match; prqlc lowers the embedded stages to a CTE. The top spenders,
//! joined back to their names:
//!
//! ```
//! use pgorm::pipeline::{ExprOps, JoinSide, Pipeline, alias, sum};
//! use pgorm::tests_cfg::{cake, fruit};
//!
//! let sweetness = alias("sweetness");
//! let sweetest = Pipeline::from(fruit::Entity)
//!     .group(fruit::Column::CakeId)
//!     .aggregate(sum(fruit::Column::Id).as_(sweetness))
//!     .filter_with(|binder| sweetness.gt(binder.bind(10_i64)));
//!
//! let (sql, values) = Pipeline::from(cake::Entity)
//!     .join(
//!         JoinSide::Inner,
//!         sweetest,
//!         cake::Column::Id.eq(alias("cake_id")),
//!     )
//!     .select((cake::Column::Name, sweetness))
//!     .into_sql()?;
//!
//! assert_eq!(
//!     sql,
//!     "WITH table_0 AS (SELECT cake_id, COALESCE(SUM(id), 0) AS sweetness \
//!      FROM fruit GROUP BY cake_id HAVING COALESCE(SUM(id), 0) > $1) \
//!      SELECT cake.name, table_0.sweetness FROM cake \
//!      INNER JOIN table_0 ON cake.id = table_0.cake_id"
//! );
//! assert_eq!(values.0.len(), 1);
//! # Ok::<_, pgorm::pipeline::PipelineError>(())
//! ```
//!
//! A relation can also be read under a name of your own, which is how one
//! table meets itself — the employee beside their manager, the message
//! beside its parent. [`IntoSource::named`] takes the name, and
//! [`col`] writes the far side's columns:
//!
//! ```
//! use pgorm::pipeline::{ExprOps, IntoSource, JoinSide, Pipeline, alias, col};
//! use pgorm::tests_cfg::fruit::{self, Column as F};
//!
//! let peer = alias("peer");
//! let (sql, _) = Pipeline::from(fruit::Entity)
//!     .join(
//!         JoinSide::Inner,
//!         fruit::Entity.named(peer),
//!         F::CakeId.eq(col(peer, alias("cake_id"))),
//!     )
//!     .filter(F::Id.lt(col(peer, alias("id"))))
//!     .select((F::Name, col(peer, alias("name"))))
//!     .into_sql()?;
//!
//! assert_eq!(
//!     sql,
//!     "SELECT fruit.name AS _expr_0, peer.name FROM fruit \
//!      INNER JOIN fruit AS peer ON fruit.cake_id = peer.cake_id \
//!      WHERE fruit.id < peer.id"
//! );
//! # Ok::<_, pgorm::pipeline::PipelineError>(())
//! ```
//!
//! When the rows you want are the entities themselves, the source-select
//! terminal arranges the projection instead of asking you for a row type:
//! restate the relations the pipeline read — an entity bare, or through the
//! same `named(..)` spelling the join used — and each listed source decodes
//! as an `Option` of its model, every position optional because a right or
//! full join can leave either side absent.
//!
//! ```
//! use pgorm::pipeline::{ExprOps, JoinSide, Pipeline};
//! use pgorm::tests_cfg::{cake, fruit};
//!
//! let selected = Pipeline::from(cake::Entity)
//!     .join(
//!         JoinSide::Left,
//!         fruit::Entity,
//!         cake::Column::Id.eq(fruit::Column::CakeId),
//!     )
//!     .select_sources((cake::Entity, fruit::Entity));
//! // selected.all(&db).await? decodes Vec<(Option<cake::Model>, Option<fruit::Model>)>
//! # let _ = selected;
//! ```
//!
//! The pipeline lowers typed Rust construction directly into prqlc's PL AST
//! (no PRQL text round-trip), then through `pl_to_rq` and `rq_to_sql` with
//! the PostgreSQL dialect. Every prqlc import lives in the private `adapter`
//! module, and the dependency is pinned exact. The pipeline is a permanent
//! part of the crate: prqlc is a plain dependency, compiled in every build.
//!
//! Everything fallible — reserved-alias screening, prqlc's resolution —
//! surfaces as a typed [`PipelineError`] from [`Pipeline::into_sql`] or the
//! terminal methods; nothing panics.
// [spec:pgorm:def:pipeline.adapter+2]

mod adapter;
mod binder;
mod builder;
mod census;
mod compare;
mod described;
mod error;
mod expr;
mod funcs;
mod naming;
mod sets;
mod sources;
mod terminal;
mod window;

pub use binder::Binder;
pub use builder::{Grouped, IntoSource, JoinSide, Pipeline, Source};
pub use described::{DescribedSource, col_typed};
pub use error::PipelineError;
pub use expr::{Expr, ExprList, ExprOps, col, that, this};
pub use funcs::{
    CastType, average, case, count, count_distinct, count_rows, first, lag, last, lead, max, min,
    null, rank, rank_dense, row_number, stddev, sum,
};
pub use pgorm_query::{AliasName, alias};
pub use sources::{Named, named_runtime};
pub use sources::{SelectableSource, SelectedSources, SourceList};
pub use window::{Over, by, over, sort_by};

#[cfg(test)]
mod tests;
