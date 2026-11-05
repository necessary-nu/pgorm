# The pipeline

A pipeline is a query written as a list of steps. Each step takes a relation
and returns a relation: keep these rows, add this column, group by that key,
sort, take the first ten. [`pgorm::pipeline`](crate::pipeline) compiles the
steps to one PostgreSQL statement, in the shape of a [PRQL] query.

Where each step lands in the SQL follows from where it stands in the list. A
filter before an aggregation becomes `WHERE`, a filter after one becomes
`HAVING`, and a filter after a window function wraps the steps before it in a
`WITH` query, because SQL cannot filter on a window in the same `SELECT`. You
write the steps in the order you think of them, and the compiler writes the
clauses.

Reach for a pipeline for reports and analysis: aggregates you filter on,
rankings, running totals, a relation built from other relations. A
[`Select`](crate::Select) reads an entity's rows into its models; the
[query builder](crate::guide::query_builder) writes SQL clause by clause; a pipeline
sits between them, composing relations rather than clauses.

[PRQL]: https://prql-lang.org

## A first pipeline

[`Pipeline::from`](crate::pipeline::Pipeline::from) starts a pipeline from an
entity, and each method appends one step.
[`into_sql`](crate::pipeline::Pipeline::into_sql) compiles it, returning the
SQL and the values it binds:

```rust
use pgorm::pipeline::{ExprOps, Pipeline};
use pgorm_guide_support::cake::{self, Column as C};

let (sql, values) = Pipeline::from(cake::Entity)
    .filter(C::GlutenFree.eq(true))
    .sort(C::Price.desc())
    .take(2)
    .select((C::Name, C::Price))
    .into_sql()?;

assert_eq!(
    sql,
    "SELECT name, price FROM cake WHERE gluten_free = true ORDER BY price DESC LIMIT 2"
);
assert!(values.0.is_empty());
# Ok::<(), pgorm::pipeline::PipelineError>(())
```

Columns are the entity's own `Column` variants, and constants are Rust
literals, written into the SQL as literals. The comparison and arithmetic
operators come from [`ExprOps`](crate::pipeline::ExprOps). Its comparisons
share their names with [`ColumnTrait`](crate::ColumnTrait)'s; with both in
scope, a column resolves to `ExprOps`, so a module that writes pipelines
imports `ExprOps` and writes `ColumnTrait::eq(&column, value)` where it means
the other.

A pipeline runs through its terminals. [`all`](crate::pipeline::Pipeline::all),
[`one`](crate::pipeline::Pipeline::one) and
[`one_opt`](crate::pipeline::Pipeline::one_opt) run it and decode rows into a
[`FromQueryResult`](crate::FromQueryResult) type, and
[`into_model`](crate::pipeline::Pipeline::into_model) and
[`into_tuple`](crate::pipeline::Pipeline::into_tuple) stage a read, by column
name or by column position, for the decode terminals of
[Reading rows](crate::guide::reading):

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("pipeline_first").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::Decimal;
use pgorm::pipeline::{ExprOps, Pipeline};
use pgorm_guide_support::cake::{self, Column as C};

let dearest: Vec<(String, Decimal)> = Pipeline::from(cake::Entity)
    .filter(C::GlutenFree.eq(true))
    .sort(C::Price.desc())
    .take(2)
    .select((C::Name, C::Price))
    .into_tuple()?
    .all(&db)
    .await?;

assert_eq!(
    dearest,
    [
        ("Chocolate Torte".to_owned(), Decimal::new(600, 2)),
        ("Lemon Drizzle".to_owned(), Decimal::new(320, 2)),
    ]
);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

An entity's own model decodes straight out of a pipeline over the entity. A
pipeline that projects nothing selects `*`, which sends an enum column as the
enum, while the model reads it as its text, as the entity's own queries
select it. So when the type you decode reads a column through a cast, the
terminal ends the pipeline by selecting the columns the type reads, each
cast column under its cast:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("pipeline_models").await;
# let db = scratch.pool.get().await?;
use pgorm::pipeline::{ExprOps, Pipeline};
use pgorm_guide_support::cake::{self, Column as C, Flavor};

let lemon: cake::Model = Pipeline::from(cake::Entity)
    .filter(C::Flavor.eq("lemon"))
    .one(&db)
    .await?;
assert_eq!(lemon.name, "Lemon Drizzle");
assert_eq!(lemon.flavor, Flavor::Lemon);

let (sql, _) = Pipeline::from(cake::Entity)
    .filter(C::Flavor.eq("lemon"))
    .into_sql()?;
assert_eq!(sql, "SELECT * FROM cake WHERE flavor = CAST('lemon' AS flavor)");
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

`into_sql` shows the pipeline as written; the cake read runs
`SELECT id, name, CAST(flavor AS text) AS flavor, price, gluten_free, bakery_id`
over it. A type that reads no column through a cast decodes from the
pipeline's own SQL. The added projection names columns bare, the way the
type reads them, so it needs a relation in which each of those names means
one column. After a join of two tables that share one, such as `id`, it is
refused as ambiguous, and
[`select_sources`](crate::pipeline::Pipeline::select_sources), below, is the
way to decode each table's model.

## The steps

| Step | What it does | SQL it usually becomes |
| --- | --- | --- |
| [`from`](crate::pipeline::Pipeline::from) | the relation to start from | `FROM` |
| [`filter`](crate::pipeline::Pipeline::filter) | keep the rows a condition holds for | `WHERE`, or `HAVING` after an aggregation |
| [`derive`](crate::pipeline::Pipeline::derive) | add computed columns, keeping the others | a projection |
| [`select`](crate::pipeline::Pipeline::select) | replace the columns with these | a projection |
| [`group`](crate::pipeline::Pipeline::group) and [`aggregate`](crate::pipeline::Grouped::aggregate) | one row per key, with aggregates | `GROUP BY` |
| [`window`](crate::pipeline::Pipeline::window) | add columns computed over a window | `OVER (..)` |
| [`sort`](crate::pipeline::Pipeline::sort) | order the rows | `ORDER BY` |
| [`take`](crate::pipeline::Pipeline::take), [`take_range`](crate::pipeline::Pipeline::take_range) | keep the first rows, or a 1-based range of them | `LIMIT`, `OFFSET` |
| [`join`](crate::pipeline::Pipeline::join) | combine with another relation on a condition | `JOIN` |
| [`append`](crate::pipeline::Pipeline::append), [`intersect`](crate::pipeline::Pipeline::intersect), [`remove`](crate::pipeline::Pipeline::remove) | combine with another relation's rows | `UNION ALL`, `INTERSECT ALL`, `EXCEPT ALL` |
| [`distinct`](crate::pipeline::Pipeline::distinct) | keep one copy of each row | `DISTINCT` |

A step that takes several expressions takes one bare, a homogeneous list as an
array, or a mixed list as a tuple: `.select(C::Name)`,
`.select([C::Name, C::City])`, `.select((C::Name, total))`.

## Names

A column a step introduces needs a name the later steps can refer to. An
[`alias`](crate::pipeline::alias) token is that name: it is declared once,
attached with [`as_`](crate::pipeline::ExprOps::as_), and used by value from
then on, as an expression like any column. Grouping the cakes by bakery and
keeping the bakeries with more than two:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("pipeline_names").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::Decimal;
use pgorm::pipeline::{ExprOps, Pipeline, alias, average, count_rows};
use pgorm_guide_support::cake::{self, Column as C};

let (cakes, mean) = (alias("cakes"), alias("mean_price"));
let busy = Pipeline::from(cake::Entity)
    .group(C::BakeryId)
    .aggregate((count_rows().as_(cakes), average(C::Price).as_(mean)))
    .filter(cakes.gt(2));

let (sql, _) = busy.clone().into_sql()?;
assert_eq!(
    sql,
    "SELECT bakery_id, COUNT(*) AS cakes, AVG(price) AS mean_price FROM cake \
     GROUP BY bakery_id HAVING COUNT(*) > 2"
);

let rows: Vec<(i32, i64, Decimal)> = busy.into_tuple()?.all(&db).await?;
assert_eq!(rows.len(), 1);
assert_eq!((rows[0].0, rows[0].1), (1, 3));
assert_eq!(rows[0].2.round_dp(2), Decimal::new(457, 2));
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

The aggregates are [`count_rows`](crate::pipeline::count_rows) (`COUNT(*)`),
[`count`](crate::pipeline::count) (`COUNT(expr)`, the rows it is not null on),
[`count_distinct`](crate::pipeline::count_distinct),
[`sum`](crate::pipeline::sum), [`min`](crate::pipeline::min),
[`max`](crate::pipeline::max), [`average`](crate::pipeline::average) and
[`stddev`](crate::pipeline::stddev). [`case`](crate::pipeline::case) writes a
`CASE` with a mandatory fallback, [`null`](crate::pipeline::null) is the
`NULL` literal, and [`cast`](crate::pipeline::ExprOps::cast) converts to one
of the [`CastType`](crate::pipeline::CastType)s.

[`derive`](crate::pipeline::Pipeline::derive) adds a column and keeps the
rest. Each cake's tier and its flavor as text, for the second and third cakes
by id:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("pipeline_derive").await;
# let db = scratch.pool.get().await?;
use pgorm::pipeline::{CastType, ExprOps, Pipeline, alias, case};
use pgorm_guide_support::cake::{self, Column as C};

let (tier, flavor) = (alias("tier"), alias("flavor_name"));
let tiers = Pipeline::from(cake::Entity)
    .derive((
        case([(C::Price.gte(4), "premium")], "standard").as_(tier),
        C::Flavor.cast(CastType::Text).as_(flavor),
    ))
    .sort(C::Id)
    .take_range(2..=3)
    .select((C::Name, tier, flavor));

let (sql, _) = tiers.clone().into_sql()?;
assert_eq!(
    sql,
    "WITH table_0 AS (SELECT name, CASE WHEN price >= 4 THEN 'premium' ELSE 'standard' END \
     AS tier, CAST(flavor AS text) AS flavor_name, id FROM cake ORDER BY id LIMIT 2 OFFSET 1) \
     SELECT name, tier, flavor_name FROM table_0 ORDER BY id"
);

let rows: Vec<(String, String, String)> = tiers.into_tuple()?.all(&db).await?;
assert_eq!(
    rows,
    [
        ("Lemon Drizzle".to_owned(), "standard".to_owned(), "lemon".to_owned()),
        ("Carrot Cake".to_owned(), "standard".to_owned(), "carrot".to_owned()),
    ]
);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

An alias that collides with a name PRQL itself defines, such as `sum`,
`first` or `from`, is refused when the pipeline compiles, because it would
hide the built-in from every later step:

```rust
use pgorm::pipeline::{ExprOps, Pipeline, PipelineError, alias, count_rows};
use pgorm_guide_support::cake::{self, Column as C};

let refused = Pipeline::from(cake::Entity)
    .group(C::BakeryId)
    .aggregate(count_rows().as_(alias("sum")))
    .into_sql();
assert_eq!(refused, Err(PipelineError::ReservedAlias("sum".to_owned())));
```

## Values from the program

A Rust literal of type `i32`, `i64`, `f64`, `bool` or `&str` converts into an
expression and is written into the SQL as a constant, quoted so that it means
exactly the value given. That suits constants of the query. A value that
comes from the program at run time, such as a price a customer typed, is
bound as a parameter instead: the statement's text then stays the same from
one value to the next, so a connection prepares it once, and any value pgorm
can bind can be compared, a `Decimal` or a date among them. Each step has a
`_with` form that hands its closure a [`Binder`](crate::pipeline::Binder),
and [`bind`](crate::pipeline::Binder::bind) records the value and returns its
`$N` placeholder in one step:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("pipeline_values").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::Decimal;
use pgorm::pipeline::{ExprOps, Pipeline};
use pgorm_guide_support::cake::{self, Column as C};

let budget = Decimal::new(350, 2);
let affordable = Pipeline::from(cake::Entity)
    .filter_with(|binder| C::Price.lte(binder.bind(budget)))
    .sort(C::Price)
    .select(C::Name);

let (sql, values) = affordable.clone().into_sql()?;
assert_eq!(
    sql,
    "WITH table_0 AS (SELECT name, price FROM cake WHERE price <= $1) \
     SELECT name FROM table_0 ORDER BY price"
);
assert_eq!(values.0, [budget.into()]);

let names: Vec<(String,)> = affordable.into_tuple()?.all(&db).await?;
assert_eq!(names, [("Vanilla Sponge".to_owned(),), ("Lemon Drizzle".to_owned(),)]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A bound placeholder belongs to the pipeline that minted it. The closure's
binder carries a lifetime of its own, and an expression holding one of its
placeholders cannot leave the closure, so it cannot reach a second pipeline
whose values would not hold it:

```rust,compile_fail,E0521
use pgorm::entity::prelude::Decimal;
use pgorm::pipeline::{Expr, ExprOps, Pipeline};
use pgorm_guide_support::cake::{self, Column as C};

let mut leaked: Option<Expr<'static>> = None;
let first = Pipeline::from(cake::Entity).filter_with(|binder| {
    let budget = binder.bind(Decimal::new(350, 2));
    leaked = Some(budget.clone());
    C::Price.lte(budget)
});
let second = Pipeline::from(cake::Entity).filter(C::Price.gt(leaked.unwrap()));
```

A `take` takes a number, not an expression: PRQL has no bound row count.

## Joins

[`join`](crate::pipeline::Pipeline::join) takes which rows to keep, a
[`JoinSide`](crate::pipeline::JoinSide), the relation to join and the
condition. An entity column carries its table, so a condition across two
entities is qualified as written:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("pipeline_joins").await;
# let db = scratch.pool.get().await?;
use pgorm::pipeline::{ExprOps, JoinSide, Pipeline, alias};
use pgorm_guide_support::{bakery, cake};

let cake_name = alias("cake");
let in_bergen = Pipeline::from(cake::Entity)
    .join(
        JoinSide::Inner,
        bakery::Entity,
        cake::Column::BakeryId.eq(bakery::Column::Id),
    )
    .filter(bakery::Column::City.eq("Bergen"))
    .select((cake::Column::Name.as_(cake_name), bakery::Column::Name))
    .sort(cake_name);

let (sql, _) = in_bergen.clone().into_sql()?;
assert_eq!(
    sql,
    "SELECT cake.name AS cake, bakery.name FROM cake INNER JOIN bakery \
     ON cake.bakery_id = bakery.id WHERE bakery.city = 'Bergen' ORDER BY cake"
);

let rows: Vec<(String, String)> = in_bergen.into_tuple()?.all(&db).await?;
assert_eq!(
    rows,
    [
        ("Carrot Cake".to_owned(), "Moonlight".to_owned()),
        ("Vanilla Sponge".to_owned(), "Moonlight".to_owned()),
    ]
);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

To decode the joined entities themselves rather than columns of your
choosing, end with
[`select_sources`](crate::pipeline::Pipeline::select_sources), listing the
relations the pipeline read. It projects each source's columns the way the
entity's own queries read them, an enum column as its text among them, each
source under a prefix of its own, so two tables sharing a column name stay
apart. Each listed source decodes as an `Option` of its model, because a
right or full join can leave either side absent:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("pipeline_sources").await;
# let db = scratch.pool.get().await?;
use pgorm::pipeline::{ExprOps, JoinSide, Pipeline};
use pgorm_guide_support::{bakery, cake};

let rows: Vec<(Option<cake::Model>, Option<bakery::Model>)> = Pipeline::from(cake::Entity)
    .join(
        JoinSide::Inner,
        bakery::Entity,
        cake::Column::BakeryId.eq(bakery::Column::Id),
    )
    .filter(cake::Column::Flavor.eq("chocolate"))
    .sort(cake::Column::Id)
    .select_sources((cake::Entity, bakery::Entity))
    .all(&db)
    .await?;

let names: Vec<(String, String)> = rows
    .into_iter()
    .map(|(cake, bakery)| (cake.unwrap().name, bakery.unwrap().name))
    .collect();
assert_eq!(
    names,
    [
        ("Chocolate Fudge".to_owned(), "Sunrise".to_owned()),
        ("Chocolate Torte".to_owned(), "Sunrise".to_owned()),
    ]
);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A step that replaces the sources' own columns, such as `select` or an
aggregation, leaves nothing to project per source, so `select_sources` after
one is refused with
[`PipelineError::ReshapedSources`](crate::pipeline::PipelineError::ReshapedSources).
The [relational-read graph](crate::guide::relations) reads related models with
types that say which side may be missing; `select_sources` is for the reads
the graph does not make, such as a right join or models beside a computed
column.

### A table meeting itself

A relation can be read under a name of your own with
[`named`](crate::pipeline::IntoSource::named), which is how a table is joined
to itself. The name is then the relation's only name, and
[`Named::col`](crate::pipeline::Named::col) writes its columns. The pairs of
cakes sold by the same bakery:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("pipeline_self_join").await;
# let db = scratch.pool.get().await?;
use pgorm::pipeline::{ExprOps, IntoSource, JoinSide, Pipeline, alias};
use pgorm_guide_support::cake::{self, Column as C};

let other = cake::Entity.named(alias("other"));
let pairs = Pipeline::from(cake::Entity)
    .join(JoinSide::Inner, other.clone(), C::BakeryId.eq(other.col(C::BakeryId)))
    .filter(C::Id.lt(other.col(C::Id)))
    .sort((C::Id, other.col(C::Id)))
    .select((C::Name.as_(alias("earlier")), other.col(C::Name).as_(alias("later"))));

let rows: Vec<(String, String)> = pairs.into_tuple()?.all(&db).await?;
assert_eq!(
    rows,
    [
        ("Chocolate Fudge".to_owned(), "Lemon Drizzle".to_owned()),
        ("Chocolate Fudge".to_owned(), "Chocolate Torte".to_owned()),
        ("Lemon Drizzle".to_owned(), "Chocolate Torte".to_owned()),
        ("Carrot Cake".to_owned(), "Vanilla Sponge".to_owned()),
    ]
);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

[`col`](crate::pipeline::col) writes a column of a table no entity describes,
`col(table, column)`, and [`Pipeline::from`](crate::pipeline::Pipeline::from)
takes such a table as an alias token, or
[`from_schema`](crate::pipeline::Pipeline::from_schema) as a schema and a
table.

A program that knows such a table's column types at run time, as a language
binding knows its declared models', reads it through the same writers an
entity's columns go through.
[`col_typed`](crate::pipeline::col_typed) types a column by the
[`ColumnType`](crate::pgorm_query::ColumnType) given, so it compares as an
entity column of that type does, and
[`select_described`](crate::pipeline::Pipeline::select_described) is
`select_sources` for relations listed column by column, each read under its
`s{i}_` prefix, with decoding left to the program:

```rust
use pgorm::pgorm_query::{ColumnType, Name};
use pgorm::pipeline::{DescribedSource, ExprOps, Pipeline, alias, col_typed};

let flavor = ColumnType::Enum {
    name: Name::runtime("flavor"),
    schema: None,
    variants: vec![Name::runtime("lemon")],
};
let cake = alias("cake");
let (sql, _) = Pipeline::from(cake)
    .filter(col_typed(cake, alias("flavor"), &flavor).eq("lemon"))
    .select_described([DescribedSource::new(cake)
        .column(alias("id"), None)
        .column(alias("name"), None)])?
    .into_sql()?;
assert_eq!(
    sql,
    "SELECT id AS s0_id, name AS s0_name FROM cake WHERE flavor = CAST('lemon' AS flavor)"
);
# Ok::<(), pgorm::pipeline::PipelineError>(())
```

## Windows

[`window`](crate::pipeline::Pipeline::window) adds columns computed over a
window of rows: partitioned with [`by`](crate::pipeline::by), ordered with
[`sort_by`](crate::pipeline::sort_by), or over the whole relation with
[`over`](crate::pipeline::over). The window functions are
[`row_number`](crate::pipeline::row_number), [`rank`](crate::pipeline::rank),
[`rank_dense`](crate::pipeline::rank_dense), [`lag`](crate::pipeline::lag),
[`lead`](crate::pipeline::lead), [`first`](crate::pipeline::first) and
[`last`](crate::pipeline::last), and any aggregate. The dearest cake at each
bakery:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("pipeline_window").await;
# let db = scratch.pool.get().await?;
use pgorm::pipeline::{ExprOps, Pipeline, alias, by, row_number};
use pgorm_guide_support::cake::{self, Column as C};

let place = alias("place");
let dearest = Pipeline::from(cake::Entity)
    .window(row_number().as_(place), by(C::BakeryId).sort_by(C::Price.desc()))
    .filter(place.eq(1))
    .sort(C::BakeryId)
    .select((C::BakeryId, C::Name));

let (sql, _) = dearest.clone().into_sql()?;
assert_eq!(
    sql,
    "WITH table_0 AS (SELECT bakery_id, name, ROW_NUMBER() OVER (PARTITION BY bakery_id \
     ORDER BY price DESC) AS _expr_0 FROM cake) SELECT bakery_id, name FROM table_0 \
     WHERE _expr_0 = 1 ORDER BY bakery_id"
);

let rows: Vec<(i32, String)> = dearest.into_tuple()?.all(&db).await?;
assert_eq!(
    rows,
    [(1, "Chocolate Torte".to_owned()), (2, "Carrot Cake".to_owned())]
);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A window can be narrowed to a frame:
[`rows`](crate::pipeline::Over::rows) counts rows from the current one, and
[`range`](crate::pipeline::Over::range) counts values. Each bound is an offset
from the current row, negative before it and positive after, and `None`
leaves that side unbounded. A running total of a bakery's prices, cheapest
first:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("pipeline_frame").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::Decimal;
use pgorm::pipeline::{ExprOps, Pipeline, alias, by, sum};
use pgorm_guide_support::cake::{self, Column as C};

let running = alias("running");
let rows: Vec<(String, Decimal)> = Pipeline::from(cake::Entity)
    .filter(C::BakeryId.eq(1))
    .window(
        sum(C::Price).as_(running),
        by(C::BakeryId).sort_by(C::Price).rows(None, Some(0)),
    )
    .sort(C::Price)
    .select((C::Name, running))
    .into_tuple()?
    .all(&db)
    .await?;

assert_eq!(
    rows,
    [
        ("Lemon Drizzle".to_owned(), Decimal::new(320, 2)),
        ("Chocolate Fudge".to_owned(), Decimal::new(770, 2)),
        ("Chocolate Torte".to_owned(), Decimal::new(1370, 2)),
    ]
);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## Composing pipelines

Because every step maps a relation to a relation, a function from
`Pipeline` to `Pipeline` is a reusable piece of a query, which any pipeline
can apply:

```rust
use pgorm::pipeline::{ExprOps, Pipeline};
use pgorm_guide_support::cake::{self, Column as C};

/// The cakes a customer who avoids gluten can eat.
fn gluten_free(cakes: Pipeline) -> Pipeline {
    cakes.filter(C::GlutenFree.eq(true))
}

let (sql, _) = gluten_free(Pipeline::from(cake::Entity))
    .select(C::Name)
    .into_sql()?;
assert_eq!(sql, "SELECT name FROM cake WHERE gluten_free = true");
# Ok::<(), pgorm::pipeline::PipelineError>(())
```

A whole pipeline is a relation too: it can be the source of another, the
relation a join reads, or the other side of a set operation. Embedding one
takes it by value, values and all; its placeholders are renumbered after the
consumer's, so the merged SQL and values still line up. Each cake's units
sold, from the order lines, joined back to the cake's name:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("pipeline_compose").await;
# let db = scratch.pool.get().await?;
use pgorm::pipeline::{ExprOps, JoinSide, Pipeline, alias, sum};
use pgorm_guide_support::{cake, order_line};

let sold = alias("sold");
let units = Pipeline::from(order_line::Entity)
    .group(order_line::Column::CakeId)
    .aggregate(sum(order_line::Column::Quantity).as_(sold))
    .filter_with(|binder| sold.gte(binder.bind(1_i64)));

let report = Pipeline::from(cake::Entity)
    .filter_with(|binder| cake::Column::BakeryId.eq(binder.bind(1)))
    .join(JoinSide::Inner, units, cake::Column::Id.eq(alias("cake_id")))
    .sort(sold.desc())
    .select((cake::Column::Name, sold));

let (_, values) = report.clone().into_sql()?;
assert_eq!(values.0, [1.into(), 1_i64.into()]);

let rows: Vec<(String, i64)> = report.into_tuple()?.all(&db).await?;
assert_eq!(
    rows,
    [("Chocolate Fudge".to_owned(), 2), ("Lemon Drizzle".to_owned(), 1)]
);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

An embedded pipeline's columns are referred to by their names, as
`alias("cake_id")` above. Where a name exists on both sides of a join,
[`that`](crate::pipeline::that) names the joined relation's column and
[`this`](crate::pipeline::this) the pipeline's own.

[`append`](crate::pipeline::Pipeline::append) puts another relation's rows
after this one's, matching columns by position, and
[`distinct`](crate::pipeline::Pipeline::distinct) after it makes the
`UNION ALL` a `UNION`:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("pipeline_append").await;
# let db = scratch.pool.get().await?;
use pgorm::pipeline::{ExprOps, Pipeline};
use pgorm_guide_support::{bakery, customer};

let everyone = Pipeline::from(bakery::Entity)
    .select(bakery::Column::Name)
    .append(Pipeline::from(customer::Entity).select(customer::Column::Name))
    .distinct();

let mut names: Vec<(String,)> = everyone.into_tuple()?.all(&db).await?;
names.sort();
assert_eq!(
    names,
    [
        ("Kim".to_owned(),),
        ("Lee".to_owned(),),
        ("Moonlight".to_owned(),),
        ("Sunrise".to_owned(),),
    ]
);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A step after a combination reads the whole combined relation. The
combination is written as a `WITH` query first and the step reads it, so a
projection or a grouping after an `append` applies to the rows of both
relations rather than to the first one's alone. Every cake, counted twice:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("pipeline_append_then_group").await;
# let db = scratch.pool.get().await?;
use pgorm::pipeline::{ExprOps, Pipeline, alias, count_rows};
use pgorm_guide_support::cake::{self, Column as C};

let cakes = alias("cakes");
let twice = Pipeline::from(cake::Entity)
    .append(cake::Entity)
    .group(C::BakeryId)
    .aggregate(count_rows().as_(cakes));

let (sql, _) = twice.clone().into_sql()?;
assert!(
    sql.starts_with("WITH table_0 AS (SELECT * FROM cake UNION ALL SELECT * FROM cake")
        && sql.ends_with("FROM table_0 GROUP BY bakery_id"),
    "{sql}"
);
let mut rows: Vec<(i32, i64)> = twice.into_tuple()?.all(&db).await?;
rows.sort();
assert_eq!(rows, [(1, 6), (2, 4)]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## When a pipeline fails

Building a pipeline cannot fail: every method appends one whole step. What
can go wrong is found when it compiles, in
[`into_sql`](crate::pipeline::Pipeline::into_sql) or a terminal, and comes back
as a [`PipelineError`](crate::pipeline::PipelineError), never a panic:

- `ReservedAlias`: a name you gave collides with one PRQL defines.
- `UnquotableIdentifier`: a table, column or alias name cannot be written
  safely as a name in SQL: it holds a double quote or a NUL byte, begins with
  `$`, or is `*`.
- `Compile`: the PRQL compiler refused the pipeline, such as a column read
  after a `select` that dropped it. The message is the compiler's own.
- `ReshapedSources`: `select_sources` after a step that replaced the sources'
  columns.

A terminal that runs the pipeline returns [`pgorm::Error`](crate::Error),
into which a `PipelineError` converts. A name no step introduced and no
source describes is not caught here: the compiler has no catalog, so it
takes the name as a column of the source, and the server answers for it when
the statement runs.

For a query known whole when the program is written, the
[`prql!`](crate::prql) macro compiles PRQL text at build time instead
([Raw SQL](crate::guide::raw_sql)).
