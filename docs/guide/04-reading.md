# Reading rows

Every read starts from an entity's [`find`](crate::EntityTrait::find), which
builds a [`Select`](crate::Select): a `SELECT` of every column of the table,
decoded into its `Model`. The builder methods narrow it, order it, choose
other columns or join other tables, and a terminal runs it. This chapter
reads the bakery's cakes, bakers and orders.

## Finding rows

A `Select` has three terminals:

| Terminal | Returns |
| --- | --- |
| [`all`](crate::Select::all) | every row, as a `Vec` |
| [`one`](crate::Select::one) | the first row, or [`Error::RecordNotFound`](crate::Error::RecordNotFound) when there is none |
| [`one_opt`](crate::Select::one_opt) | the first row, or `None` |

`one` and `one_opt` ask for a single row with `LIMIT 1`. Use `one` where a
missing row is a mistake, and `one_opt` where it is an ordinary answer.
[`find_by_id`](crate::EntityTrait::find_by_id) filters on the primary key, and
takes a composite key as a tuple:

```rust
# use pgorm_guide_support::{cake, cake_baker};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("reading_find").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;

let cakes: Vec<cake::Model> = cake::Entity::find().all(&db).await?;
assert_eq!(cakes.len(), 5);

let torte: cake::Model = cake::Entity::find_by_id(5).one(&db).await?;
assert_eq!(torte.name, "Chocolate Torte");

let missing = cake::Entity::find_by_id(99).one(&db).await;
assert!(matches!(missing, Err(Error::RecordNotFound)));
assert_eq!(cake::Entity::find_by_id(99).one_opt(&db).await?, None);

// Bo bakes the Lemon Drizzle.
assert!(cake_baker::Entity::find_by_id((2, 2)).one_opt(&db).await?.is_some());
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## Filtering

[`filter`](crate::QueryFilter::filter) adds a condition to the `WHERE`
clause, and several `filter` calls must all hold. A condition is usually
built from a column, through the methods of
[`ColumnTrait`](crate::ColumnTrait):

| Methods | SQL |
| --- | --- |
| `eq`, `ne`, `gt`, `gte`, `lt`, `lte` | `=`, `<>`, `>`, `>=`, `<`, `<=` against a value |
| `between`, `not_between` | `BETWEEN a AND b` |
| `is_in`, `is_not_in` | `IN (..)` against a list of values |
| `eq_any`, `ne_all` | `= ANY($1)`, `<> ALL($1)` against one array parameter |
| `in_subquery`, `not_in_subquery` | `IN (SELECT ..)` |
| `is_null`, `is_not_null` | `IS NULL`, `IS NOT NULL` |
| `is_distinct_from`, `is_not_distinct_from` | `IS [NOT] DISTINCT FROM`, which compares `NULL` as a value |
| `like`, `not_like` | `LIKE` against a pattern you write |
| `starts_with`, `ends_with`, `contains` | `LIKE` against text, with `%` and `_` in it escaped |
| `eq_col`, `ne_col`, `gt_col`, `gte_col`, `lt_col`, `lte_col` | against another column |
| `eq_expr` | against an expression |
| `includes`, `contained_by`, `overlaps`, ... | the range and array operators ([Values and types](crate::guide::values)) |

Each value is bound as a parameter. A value compared with an enum column is
cast to the enum's type, which the value methods do for you and which is why
they take values only: comparing two columns is `eq_col`, and comparing with
an expression is `eq_expr`.

```rust
# use pgorm_guide_support::cake::{self, Flavor};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("reading_filter").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;

let chocolate_under_five = cake::Entity::find()
    .filter(cake::Column::Flavor.eq(Flavor::Chocolate))
    .filter(cake::Column::Price.lt(Decimal::new(500, 2)));
assert_eq!(
    chocolate_under_five.clone().build().0,
    r#"SELECT "cake"."id", "cake"."name", CAST("cake"."flavor" AS text), "cake"."price", "cake"."gluten_free", "cake"."bakery_id" FROM "cake" WHERE "cake"."flavor" = CAST($1::text AS flavor) AND "cake"."price" < $2"#
);
let found = chocolate_under_five.all(&db).await?;
assert_eq!(found.len(), 1);
assert_eq!(found[0].name, "Chocolate Fudge");

let mid_priced = cake::Entity::find()
    .filter(cake::Column::Price.between(Decimal::new(300, 2), Decimal::new(400, 2)))
    .count(&db)
    .await?;
assert_eq!(mid_priced, 2);

let named = cake::Entity::find()
    .filter(cake::Column::Name.starts_with("Chocolate"))
    .filter(cake::Column::Id.is_in([1, 2, 3]))
    .all(&db)
    .await?;
assert_eq!(named.len(), 1);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

### Any and all

[`Condition::any`](crate::Condition::any) joins conditions with `OR`, and
[`Condition::all`](crate::Condition::all) with `AND`. Each takes conditions
through [`add`](crate::Condition::add), nests inside the other, and goes to
`filter` like a single condition. [`add_option`](crate::Condition::add_option)
adds a condition that may be absent, which suits a search form whose fields
are optional:

```rust
# use pgorm_guide_support::cake::{self, Flavor};
use pgorm::entity::prelude::*;

let lemon_or_gluten_free = cake::Entity::find()
    .select(cake::Column::Name)
    .filter(
        Condition::any()
            .add(cake::Column::Flavor.eq(Flavor::Lemon))
            .add(cake::Column::GlutenFree.eq(true)),
    );
assert_eq!(
    lemon_or_gluten_free.build().0,
    r#"SELECT "cake"."name" FROM "cake" WHERE "cake"."flavor" = CAST($1::text AS flavor) OR "cake"."gluten_free" = $2"#
);

// A search with an optional upper price: no condition at all when it is absent.
let max_price: Option<Decimal> = None;
let search = cake::Entity::find()
    .select(cake::Column::Name)
    .filter(
        Condition::all()
            .add(cake::Column::BakeryId.eq(1))
            .add_option(max_price.map(|price| cake::Column::Price.lte(price))),
    );
assert_eq!(
    search.build().0,
    r#"SELECT "cake"."name" FROM "cake" WHERE "cake"."bakery_id" = $1"#
);
```

## Ordering, limits and offsets

[`order_by_asc`](crate::QueryOrder::order_by_asc) and
[`order_by_desc`](crate::QueryOrder::order_by_desc) order by a column or an
expression, in the order they are called.
[`order_by_with_nulls`](crate::QueryOrder::order_by_with_nulls) says where
`NULL`s go. [`limit`](crate::QuerySelect::limit) and
[`offset`](crate::QuerySelect::offset) cut the result, and
[`distinct`](crate::QuerySelect::distinct) and
[`distinct_on`](crate::QuerySelect::distinct_on) drop repeated rows.

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("reading_order").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;

let two_dearest = cake::Entity::find()
    .order_by_desc(cake::Column::Price)
    .order_by_asc(cake::Column::Name)
    .limit(2);
let names: Vec<String> = two_dearest
    .all(&db)
    .await?
    .into_iter()
    .map(|cake| cake.name)
    .collect();
assert_eq!(names, ["Chocolate Torte", "Chocolate Fudge"]);

let third: cake::Model = cake::Entity::find()
    .order_by_desc(cake::Column::Price)
    .offset(2)
    .one(&db)
    .await?;
assert_eq!(third.name, "Carrot Cake");
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A query read in pages is better served by [Pages, cursors and
streams](crate::guide::pagination) than by `offset`.

## Choosing columns

[`select`](crate::Select::select) replaces the table's columns with a list of
your own: one column alone, a homogeneous list as an array, or a mixed list,
columns of two tables or a column beside an expression, as a tuple. The rows
then no longer have the `Model`'s shape, so the query has no `all` or `one`
until you say what each row decodes into:

- [`into_tuple`](crate::SelectProjected::into_tuple) reads the columns in
  order into a value or a tuple.
- [`into_model`](crate::SelectProjected::into_model) reads them by name into
  a type deriving [`FromQueryResult`](derive@crate::FromQueryResult).
- [`into_values`](crate::SelectProjected::into_values) reads them by the
  names a [`Column`](derive@crate::Column) enum gives.

[`column_as`](crate::QuerySelect::column_as) and
[`expr_as`](crate::QuerySelect::expr_as) add an item under a name, which an
[`alias`](crate::alias) gives.

```rust
# use pgorm_guide_support::cake::{self, Flavor};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("reading_select").await;
# let db = scratch.pool.get().await?;
use pgorm::alias;
use pgorm::entity::prelude::*;

let names: Vec<String> = cake::Entity::find()
    .select(cake::Column::Name)
    .order_by_asc(cake::Column::Id)
    .into_tuple()
    .all(&db)
    .await?;
assert_eq!(names[0], "Chocolate Fudge");

let priced: Vec<(String, Decimal)> = cake::Entity::find()
    .select([cake::Column::Name, cake::Column::Price])
    .filter(cake::Column::GlutenFree.eq(true))
    .order_by_asc(cake::Column::Price)
    .into_tuple()
    .all(&db)
    .await?;
assert_eq!(priced[0], ("Vanilla Sponge".to_owned(), Decimal::new(290, 2)));

let flavors = cake::Entity::find()
    .select(cake::Column::Flavor)
    .column_as(cake::Column::Id.count(), alias("cakes"))
    .group_by(cake::Column::Flavor)
    .order_by_asc(cake::Column::Flavor);
assert_eq!(
    flavors.clone().build().0,
    r#"SELECT CAST("cake"."flavor" AS text), COUNT("cake"."id") AS "cakes" FROM "cake" GROUP BY "cake"."flavor" ORDER BY "cake"."flavor" ASC"#
);
let counted: Vec<(Flavor, i64)> = flavors.into_tuple().all(&db).await?;
assert_eq!(counted[0], (Flavor::Chocolate, 2));
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

`select_only` clears the columns without adding any, for a list built up
afterwards with [`column`](crate::QuerySelect::column),
[`columns`](crate::QuerySelect::columns) and
[`expr`](crate::QuerySelect::expr), such as one computed at run time.

## Decoding into your own types

A struct deriving [`FromQueryResult`](derive@crate::FromQueryResult) decodes
a row by column name, so its fields name the columns or the aliases the
query gives them. A struct deriving [`PartialModel`](derive@crate::PartialModel)
goes further: it also says which columns and expressions to select, so the
query and the struct cannot disagree.
[`into_partial_model`](crate::Select::into_partial_model) selects what the
struct asks for and decodes into it.

```rust
# use pgorm_guide_support::cake;
use pgorm::entity::prelude::*;
use pgorm::alias;

/// A line of the price list.
#[derive(Debug, PartialEq, FromQueryResult)]
struct PriceTag {
    name: String,
    euros: Decimal,
}

/// A cake on the menu board, its name shouted.
#[derive(Debug, PartialEq, PartialModel)]
#[pgorm(entity = cake::Entity)]
struct MenuLine {
    #[pgorm(from_expr = Expr::expr(Func::upper(Expr::col((cake::Entity, cake::Column::Name)))))]
    shout: String,
    #[pgorm(from_col = cake::Column::GlutenFree)]
    free_from_gluten: bool,
}

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("reading_decode").await;
# let db = scratch.pool.get().await?;
let tags: Vec<PriceTag> = cake::Entity::find()
    .select(cake::Column::Name)
    .column_as(cake::Column::Price, alias("euros"))
    .filter(cake::Column::BakeryId.eq(2))
    .order_by_asc(cake::Column::Name)
    .into_model()
    .all(&db)
    .await?;
assert_eq!(
    tags[0],
    PriceTag {
        name: "Carrot Cake".to_owned(),
        euros: Decimal::new(380, 2),
    }
);

// `select_cols` is the projection `into_partial_model` selects.
use pgorm::PartialModelTrait;

let lemon = cake::Entity::find().filter(cake::Column::Id.eq(2));
assert_eq!(
    MenuLine::select_cols(lemon.clone().select_only()).build().0,
    r#"SELECT UPPER("cake"."name") AS "shout", "cake"."gluten_free" AS "free_from_gluten" FROM "cake" WHERE "cake"."id" = $1"#
);
assert_eq!(
    lemon.into_partial_model::<MenuLine>().one(&db).await?,
    MenuLine {
        shout: "LEMON DRIZZLE".to_owned(),
        free_from_gluten: true,
    }
);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A field marked `#[pgorm(skip)]` reads no column and takes its type's
default. An `Option` field reads `NULL` as `None`.

## Joins

A join follows a relation the entities declare.
[`inner_join_rel`](crate::QuerySelect::inner_join_rel),
[`left_join_rel`](crate::QuerySelect::left_join_rel) and
[`right_join_rel`](crate::QuerySelect::right_join_rel) join the table a
relation of the queried entity points at. The `_rev` forms take a relation
declared on the other entity and join that entity, so
`bakery::Entity::find().inner_join_rel_rev(cake::Relation::Bakery)` joins each
bakery to its cakes through the cake's `belongs_to`.
[`join`](crate::QuerySelect::join) takes the join type and any
[`RelationDef`](crate::RelationDef), and
[`join_as`](crate::QuerySelect::join_as) joins a table under an alias, for a
table joined twice.

A join adds rows to the query, not columns: the select list stays the
queried entity's until you choose columns from the joined table too. To read
each row together with its related model, use the graph in
[Relations](crate::guide::relations) instead.

```rust
# use pgorm_guide_support::{bakery, cake};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("reading_join").await;
# let db = scratch.pool.get().await?;
use pgorm::entity::prelude::*;

let menu = cake::Entity::find()
    .inner_join_rel(cake::Relation::Bakery)
    .select((bakery::Column::City, cake::Column::Name))
    .filter(cake::Column::GlutenFree.eq(true))
    .order_by_asc(bakery::Column::City)
    .order_by_asc(cake::Column::Name);
assert_eq!(
    menu.clone().build().0,
    r#"SELECT "bakery"."city", "cake"."name" FROM "cake" INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" WHERE "cake"."gluten_free" = $1 ORDER BY "bakery"."city" ASC, "cake"."name" ASC"#
);
let rows: Vec<(String, String)> = menu.into_tuple().all(&db).await?;
assert_eq!(
    rows,
    [
        ("Bergen".to_owned(), "Vanilla Sponge".to_owned()),
        ("Oslo".to_owned(), "Chocolate Torte".to_owned()),
        ("Oslo".to_owned(), "Lemon Drizzle".to_owned()),
    ]
);

// The bakeries that sell a chocolate cake, read from the bakery's side.
let chocolate_shops: Vec<bakery::Model> = bakery::Entity::find()
    .inner_join_rel_rev(cake::Relation::Bakery)
    .filter(cake::Column::Name.starts_with("Chocolate"))
    .distinct()
    .all(&db)
    .await?;
assert_eq!(chocolate_shops.len(), 1);
assert_eq!(chocolate_shops[0].name, "Sunrise");
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## Grouping and counting

[`group_by`](crate::QuerySelect::group_by) groups the rows, and the
aggregate methods of a column, `count`, `sum`, `min` and `max`, summarize
each group. [`having`](crate::QuerySelect::having) filters the groups.
[`count`](crate::PaginatorTrait::count) on any query counts its rows.

```rust
# use pgorm_guide_support::{bakery, cake};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("reading_group").await;
# let db = scratch.pool.get().await?;
use pgorm::alias;
use pgorm::entity::prelude::*;

let busy: Vec<(String, i64)> = cake::Entity::find()
    .inner_join_rel(cake::Relation::Bakery)
    .select(bakery::Column::Name)
    .column_as(cake::Column::Id.count(), alias("cakes"))
    .group_by(bakery::Column::Name)
    .having(Expr::expr(cake::Column::Id.count()).gt(2))
    .into_tuple()
    .all(&db)
    .await?;
assert_eq!(busy, [("Sunrise".to_owned(), 3)]);

let gluten_free = cake::Entity::find()
    .filter(cake::Column::GlutenFree.eq(true))
    .count(&db)
    .await?;
assert_eq!(gluten_free, 3);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## Locking rows

Inside a [transaction](crate::guide::transactions),
[`lock_exclusive`](crate::QuerySelect::lock_exclusive) reads rows
`FOR UPDATE`, so no other transaction can change them until this one ends,
and [`lock_shared`](crate::QuerySelect::lock_shared) reads them `FOR SHARE`.
[`lock_with_behavior`](crate::QuerySelect::lock_with_behavior) adds
`NOWAIT` or `SKIP LOCKED`, the second of which suits a queue of work that
several workers take from:

```rust
# use pgorm_guide_support::order;
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{LockBehavior, LockType};

let next = order::Entity::find()
    .select(order::Column::Id)
    .order_by_asc(order::Column::PlacedAt)
    .limit(1)
    .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked);
assert_eq!(
    next.build().0,
    r#"SELECT "order"."id" FROM "order" ORDER BY "order"."placed_at" ASC LIMIT $1 FOR UPDATE SKIP LOCKED"#
);
```

## Beyond the entity builders

A `Select` is a [`SelectStatement`](crate::pgorm_query::SelectStatement) of
the [query builder](crate::guide::query_builder) underneath, which
[`QueryTrait::query`](crate::QueryTrait::query) hands out for what the entity
builders do not cover: common table expressions, window functions, lateral
joins, grouping sets. A statement built there, or SQL written by hand, still
decodes into models and tuples ([Raw SQL](crate::guide::raw_sql)), and
[the pipeline](crate::guide::pipeline) composes queries in another style.
