# Raw SQL

The query builders cover most statements, but not all of them, and some SQL
is clearer written out. pgorm runs SQL you write as readily as SQL it builds:
the statement is text, its values travel beside it as parameters, and the
rows come back decoded into the same models, tuples and structs.

This chapter runs SQL text on a connection, decodes its rows, and checks it
at compile time with the [`sql!`](crate::sql) and [`prql!`](crate::prql)
macros.

## Running a statement

Every [`ConnectionTrait`](crate::ConnectionTrait), a connection or a
transaction, runs SQL text. The statement is a `&str` or a `&String`, its
parameters are written `$1`, `$2` and so on, and their values are passed
beside it:

| Method | Runs the statement and returns |
| --- | --- |
| [`execute`](crate::ConnectionTrait::execute) | the number of rows it changed |
| [`query_one`](crate::ConnectionTrait::query_one) | its one row, and an error for any other number |
| [`query_opt`](crate::ConnectionTrait::query_opt) | its row, if any, and an error for more than one |
| [`query_all`](crate::ConnectionTrait::query_all) | all of its rows |
| [`query_raw`](crate::ConnectionTrait::query_raw) | a stream of its rows, read as they arrive |
| [`execute_raw`](crate::ConnectionTrait::execute_raw) | as `execute`, with the values as an iterator |
| [`batch_execute`](crate::ConnectionTrait::batch_execute) | nothing: it runs a whole script of `;`-separated statements, with no parameters |

The rows are tokio-postgres's [`Row`](crate::Row), which pgorm re-exports,
whose `get` and `try_get` read a column by position or by name:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("raw_sql_connection").await;
# let db = scratch.pool.get().await?;
use pgorm::{ConnectionTrait, entity::prelude::Decimal};

let raised = db
    .execute(
        "UPDATE cake SET price = price + $1 WHERE bakery_id = $2",
        &[&Decimal::new(50, 2), &2_i32],
    )
    .await?;
assert_eq!(raised, 2);

let row = db
    .query_one("SELECT name, price FROM cake WHERE id = $1", &[&3_i32])
    .await?;
let (name, price): (String, Decimal) = (row.get("name"), row.get("price"));
assert_eq!((name.as_str(), price.to_string().as_str()), ("Carrot Cake", "4.30"));

let none = db
    .query_opt("SELECT name FROM cake WHERE id = $1", &[&99_i32])
    .await?;
assert!(none.is_none());
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

Every method but `batch_execute` prepares its statement on the connection
once and reuses it the next time the same text runs there, so a statement
written once and run often is parsed and planned once per connection.
[Statement caches and DDL](crate::guide::statement_caches) covers what that means
after a schema change. `batch_execute` sends its script as it is, preparing
nothing; it takes no parameters, so it is for DDL and fixed scripts, never
for values a user supplied.

A statement is text and only text: the statement methods take `&str` and
`&String` and nothing else. A tokio-postgres `Statement` prepared elsewhere
belongs to the one connection it was prepared on, so it cannot be passed.

`query_raw` streams the rows instead of collecting them, for a result too
large to hold at once. Its values are an iterator, as `execute_raw`'s are:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("raw_sql_stream").await;
# let db = scratch.pool.get().await?;
use futures::TryStreamExt;
use pgorm::ConnectionTrait;

let rows = db
    .query_raw("SELECT name FROM cake WHERE gluten_free = $1 ORDER BY name", [true])
    .await?;
let names: Vec<String> = rows.map_ok(|row| row.get(0)).try_collect().await?;
assert_eq!(names, ["Chocolate Torte", "Lemon Drizzle", "Vanilla Sponge"]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## Decoding rows into models

A statement's text and its values together are a pair, `(sql, values)`, and
[`DecodeRaw`](crate::DecodeRaw), in the prelude, decodes the rows of such a
pair by naming what each row becomes:

- [`into_model::<M>()`](crate::DecodeRaw::into_model) decodes each row into a
  [`FromQueryResult`](trait@crate::FromQueryResult) type, such as an entity's
  `Model` or a struct deriving `FromQueryResult`, by column name;
- [`into_tuple::<T>()`](crate::DecodeRaw::into_tuple) decodes each row into a
  value or a tuple, by column position;
- [`into_values::<T, C>()`](crate::DecodeRaw::into_values) decodes each row
  into a tuple, by the column names of a column enum `C` deriving
  [`Column`](derive@crate::Column).

The values are a [`Values`](crate::Values), a list of
[`Value`](crate::Value)s, each converted from a Rust value with `into()`. The
result is a selector with the terminals a query has, `all`, `one`, `one_opt`
and `stream`. pgorm adds nothing to SQL you wrote, so `one` and `one_opt`
add no `LIMIT`: a statement that returns more than one row is an error
there, and a query for the first of many says `LIMIT 1` itself.

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("raw_sql_decode").await;
# let db = scratch.pool.get().await?;
use pgorm::{Values, entity::prelude::*};

#[derive(Debug, PartialEq, FromQueryResult)]
struct Menu {
    bakery: String,
    cakes: i64,
}

let sql = "SELECT bakery.name AS bakery, count(cake.id) AS cakes
           FROM bakery JOIN cake ON cake.bakery_id = bakery.id
           WHERE cake.price < $1
           GROUP BY bakery.name ORDER BY bakery.name";
let menus: Vec<Menu> = (sql, Values(vec![Decimal::new(500, 2).into()]))
    .into_model::<Menu>()
    .all(&db)
    .await?;
assert_eq!(
    menus,
    [
        Menu { bakery: "Moonlight".to_owned(), cakes: 2 },
        Menu { bakery: "Sunrise".to_owned(), cakes: 2 },
    ]
);

let cheapest: (String, Decimal) = ("SELECT name, price FROM cake ORDER BY price LIMIT 1", Values(vec![]))
    .into_tuple()
    .one(&db)
    .await?;
assert_eq!(cheapest.0, "Vanilla Sponge");

// Five rows, where `one` expects at most one.
let many = ("SELECT name FROM cake", Values(vec![])).into_tuple::<String>().one(&db).await;
assert!(many.is_err());

// An entity's own model reads its enum columns as text, so the SQL casts them.
let lemon: cake::Model = (
    "SELECT id, name, flavor::text AS flavor, price, gluten_free, bakery_id FROM cake WHERE id = $1",
    Values(vec![2.into()]),
)
    .into_model()
    .one(&db)
    .await?;
assert_eq!(lemon.flavor, cake::Flavor::Lemon);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

An entity's `Model` decodes a PostgreSQL enum column from its text, which is
why a query pgorm builds selects `CAST("cake"."flavor" AS text)`. SQL you
write for such a model casts the column the same way, as the last query
above does.

The same raw statement can start from other places, each taking the SQL as a
`String`:

- [`Select::from_raw_sql`](crate::Select::from_raw_sql), on an entity's
  `find()`, decodes into the entity's `Model`:
  `cake::Entity::find().from_raw_sql(sql, values)`;
- [`FromQueryResult::find_by_statement`](crate::FromQueryResult::find_by_statement)
  decodes into the type it is called on: `Menu::find_by_statement(sql, values)`;
- the constructors of [`SelectorRaw`](crate::SelectorRaw), the type all of
  these return.

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("raw_sql_entry_points").await;
# let db = scratch.pool.get().await?;
use pgorm::{Values, entity::prelude::*};

#[derive(Debug, PartialEq, FromQueryResult)]
struct Named {
    name: String,
}

let gluten_free = cake::Entity::find()
    .from_raw_sql(
        "SELECT id, name, flavor::text AS flavor, price, gluten_free, bakery_id
         FROM cake WHERE gluten_free ORDER BY id"
            .to_owned(),
        Values(vec![]),
    )
    .all(&db)
    .await?;
assert_eq!(gluten_free.len(), 3);

let bakers = Named::find_by_statement(
    "SELECT name FROM baker WHERE bakery_id = $1 ORDER BY name",
    vec![1.into()],
)
.all(&db)
.await?;
assert_eq!(bakers, [Named { name: "Ada".to_owned() }, Named { name: "Bo".to_owned() }]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## Checking a decode target against a statement

A struct that names a column the statement does not return fails only when a
row arrives, so a query that happens to return nothing hides the mistake.
[`verify`](crate::VerifyStatement::verify) asks PostgreSQL to describe the
statement's result without running it, and checks every column the type reads
against that description: present, and of a type its field can decode.

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("raw_sql_verify").await;
# let db = scratch.pool.get().await?;
use pgorm::{Error, FromQueryResult, VerifyError, VerifyStatement};

#[derive(Debug, FromQueryResult)]
struct PriceList {
    name: String,
    price_in_cents: i64,
}

let sql = "SELECT name, price FROM cake WHERE bakery_id = $1";
let mismatch = db.verify::<PriceList>(sql).await;
assert!(matches!(
    mismatch,
    Err(Error::Verify(VerifyError::ColumnMissing { column: "price_in_cents", .. }))
));

let fixed = "SELECT name, (price * 100)::bigint AS price_in_cents FROM cake WHERE bakery_id = $1";
db.verify::<PriceList>(fixed).await?;
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## Statements the query builder built

A statement built with [the query builder](crate::guide::query_builder) renders to
the same pair through [`build`](crate::pgorm_query::SelectStatement::build),
so it decodes with `DecodeRaw` too. To run one directly on a connection,
whose methods take tokio-postgres parameters, wrap each value in a
[`ValueHolder`](crate::ValueHolder):

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("raw_sql_built").await;
# let db = scratch.pool.get().await?;
use pgorm::{ValueHolder, entity::prelude::*, pgorm_query::Query};

let (sql, values) = Query::update()
    .table(cake::Entity)
    .value(cake::Column::Name, "Lemon Drizzle Loaf")
    .and_where(cake::Column::Id.eq(2))
    .build();
assert_eq!(sql, r#"UPDATE "cake" SET "name" = $1 WHERE "cake"."id" = $2"#);

let renamed = db.execute_raw(&sql, values.into_iter().map(ValueHolder)).await?;
assert_eq!(renamed, 1);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## SQL checked at compile time

A typo in a SQL string normally shows up when the statement first runs.
[`sql!`](crate::sql) moves it to compile time: it holds a string literal to
PostgreSQL 18's own grammar while your crate compiles, and expands to the
literal unchanged, a `&'static str` that costs nothing at run time.

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("raw_sql_sql_macro").await;
# let db = scratch.pool.get().await?;
use pgorm::{ConnectionTrait, sql};

const BAKERS_AT: &str = sql!("SELECT name FROM baker WHERE bakery_id = $1 ORDER BY name");

let rows = db.query_all(BAKERS_AT, &[&1_i32]).await?;
let names: Vec<String> = rows.iter().map(|row| row.get(0)).collect();
assert_eq!(names, ["Ada", "Bo"]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

SQL the grammar rejects does not compile; the error reprints the line, with
a caret under the token the parser stopped at:

```rust,compile_fail
const BAKERS_AT: &str = pgorm::sql!("SELEC name FROM baker WHERE bakery_id = $1");
```

The check is of the grammar alone. `sql!` knows nothing of your schema, so
a table or column that does not exist still compiles, and is the server's to
refuse. It takes no values: write `$1` placeholders in the SQL and pass the
values where it runs. A literal holding a `;`-separated script is checked
whole, for `batch_execute`.

## PRQL compiled at compile time

[PRQL](https://prql-lang.org) is a query language that reads as a pipeline
of steps, each taking the relation the one before it produced.
[`prql!`](crate::prql) compiles a PRQL literal to PostgreSQL SQL while your
crate compiles, checks the SQL against PostgreSQL's grammar as `sql!` does,
and expands to the `(sql, values)` pair, its arguments converted into the
`Values` in the order of their `$N` placeholders:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("raw_sql_prql").await;
# let db = scratch.pool.get().await?;
use pgorm::{entity::prelude::*, prql};

let under = Decimal::new(400, 2);
let (sql, values) = prql!(
    "from cake | filter price < $1 | sort name | select {name, price}",
    under
);
assert_eq!(sql, "SELECT name, price FROM cake WHERE price < $1 ORDER BY name");
assert_eq!(values.0, [under.into()]);

let cheap: Vec<(String, Decimal)> = (sql, values).into_tuple().all(&db).await?;
let names: Vec<&str> = cheap.iter().map(|(name, _)| name.as_str()).collect();
assert_eq!(names, ["Carrot Cake", "Lemon Drizzle", "Vanilla Sponge"]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

`prql!` refuses at compile time PRQL the compiler rejects, SQL the grammar
rejects, a number of arguments that does not match the placeholders, and
placeholders numbered with a gap:

```rust,compile_fail
// `$1` is used, and nothing binds it.
let (sql, values) = pgorm::prql!("from cake | filter price < $1");
```

Like `sql!`, it does not know the schema. For a query assembled from parts
at run time, such as a filter a user chose, [the pipeline](crate::guide::pipeline)
is the same compiler with a builder in front of it.
