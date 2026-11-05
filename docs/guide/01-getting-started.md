# Getting started

pgorm is an async ORM for PostgreSQL, and for PostgreSQL alone. You declare
each table as a Rust type, an *entity*, and pgorm writes the SQL that reads
and writes it. Every value travels to the server as a bound parameter, never
pasted into the SQL text, and each statement is prepared once per connection
and reused.

pgorm runs on [tokio-postgres] and a [deadpool] connection pool, on the tokio
runtime. Because it targets one database, it can offer what PostgreSQL has and
other databases lack: `RETURNING`, `ON CONFLICT`, `MERGE`, `COPY`,
`LISTEN`/`NOTIFY`, enums, domains, ranges, arrays and JSON among them.

This chapter adds pgorm to a project, connects to a database and runs a first
query.

[tokio-postgres]: https://docs.rs/tokio-postgres
[deadpool]: https://docs.rs/deadpool

## Adding the dependency

pgorm depends on forks of two crates, pinned to a git revision, and crates.io
does not accept git dependencies, so pgorm is not published there. Depend on
it by git:

```toml
[dependencies]
pgorm = { git = "https://github.com/necessary-nu/pgorm" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

pgorm needs Rust 1.89 or later. Its features are:

| Feature | Default | What it gives |
| --- | --- | --- |
| `macros` | on | The derives: `Entity`, `ActiveEnum` and the rest |
| `with-json` | on | `serde_json::Value` columns, and `Serialize`/`Deserialize` for pgorm's own value types |
| `with-jiff` | on | `date`, `time`, `timestamp` and `timestamptz` columns as [jiff] types |
| `with-uuid` | on | `uuid` columns as [`uuid::Uuid`](https://docs.rs/uuid) |
| `postgres-array` | on | Array columns as `Vec<T>` |
| `metrics-fingerprint` | off | Query fingerprints in the [metrics](crate::guide::metrics) layer |
| `pg-19` | off | What only PostgreSQL 19 accepts ([PostgreSQL 18 and 19](crate::guide::postgres_releases)) |

A build targets PostgreSQL 18 unless it turns on `pg-19`. Nothing older than
18 is supported.

[jiff]: https://docs.rs/jiff

## The bakery

The examples in this guide share one small schema: a chain of bakeries, the
bakers who work there, the cakes they sell, and the customers who order them.

| Table | Holds |
| --- | --- |
| `bakery` | a shop: its name and city |
| `baker` | a baker, the bakery they work at, and when they were hired |
| `cake` | a cake: its name, flavor, price, whether it is gluten-free, and its bakery |
| `cake_baker` | which baker bakes which cake |
| `customer` | a customer, with a unique email address |
| `order` | an order a customer placed at a bakery |
| `order_line` | the cakes an order holds, with their quantity and price |

There are two bakeries, Sunrise in Oslo and Moonlight in Bergen. Sunrise sells
Chocolate Fudge, Lemon Drizzle and Chocolate Torte; Moonlight sells Carrot Cake
and Vanilla Sponge. Ada and Bo bake at Sunrise, Cy at Moonlight. Kim and Lee
are customers, with one order each. [Entities](crate::guide::entities) declares these
tables. Each example runs against a fresh copy of the bakery in a database of
its own, so what one example writes no other example sees.

## Connecting

[`pgorm::connect`](crate::connect) takes a tokio-postgres
[`Config`](crate::Config), which parses from a connection URL, and returns a
[`DatabasePool`](crate::DatabasePool). Building a pool connects to nothing:
the pool opens connections when they are first asked for, so a wrong host or
password shows up at the first [`get`](crate::DatabasePool::get). To fail at
startup instead, take one connection there.

A query runs on a [`DatabaseConnection`](crate::DatabaseConnection), which is
one connection checked out of the pool. It goes back to the pool when it is
dropped.

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("getting_started_connect").await;
# let url = scratch.url();
use pgorm::ConnectionTrait;

// The URL of the bakery database, such as
// "postgres://postgres:postgres@localhost:5432/bakery".
let config: pgorm::Config = url.parse()?;
let pool = pgorm::connect(config);

// Fail now rather than at the first request.
let db = pool.get().await?;
let row = db.query_one("SELECT count(*) FROM cake", &[]).await?;
let cakes: i64 = row.get(0);
assert_eq!(cakes, 5);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

Keep the pool for the life of the program. It is cheap to clone, and every
clone shares the same connections. Take a connection for each unit of work,
such as one request, and let it drop when the work is done.

The pool is not itself a connection. `DatabasePool` does not implement
[`ConnectionTrait`](crate::ConnectionTrait), so a query cannot run on
"whichever connection the pool hands out":

```rust,compile_fail,E0277
use pgorm::entity::prelude::*;
use pgorm_guide_support::cake;

async fn cakes(pool: &DatabasePool) -> Result<Vec<cake::Model>, Error> {
    // The pool is not a connection: take one with `pool.get().await?`.
    cake::Entity::find().all(pool).await
}
```

This keeps the connections a piece of code uses visible in the code. Two
statements on one connection run one after the other. Two that should run at
the same time take two connections.

## A first query

The bakery's entities are Rust modules, one per table. Each holds a `Model`,
the struct a row decodes into, and an `Entity`, the type queries start from.
[`find`](crate::EntityTrait::find) starts a `SELECT`, the builder methods
refine it, and a terminal such as [`all`](crate::Select::all) runs it:

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("getting_started_first_query").await;
# let pool = scratch.pool.clone();
use pgorm::entity::prelude::*;

let db = pool.get().await?;
let gluten_free: Vec<cake::Model> = cake::Entity::find()
    .filter(cake::Column::GlutenFree.eq(true))
    .order_by_asc(cake::Column::Name)
    .all(&db)
    .await?;

let names: Vec<&str> = gluten_free.iter().map(|cake| cake.name.as_str()).collect();
assert_eq!(names, ["Chocolate Torte", "Lemon Drizzle", "Vanilla Sponge"]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

`use pgorm::entity::prelude::*` brings in what a module that talks to the
database needs: the entity traits and derives, the query traits whose methods
the builders use, the connection types, [`Error`](crate::Error), and the
column value types.

A query is a value until it runs, and it can be rendered without a database.
[`build`](crate::QueryTrait::build) returns the SQL and the values it binds:

```rust
# use pgorm_guide_support::cake;
use pgorm::entity::prelude::*;

let (sql, values) = cake::Entity::find()
    .filter(cake::Column::GlutenFree.eq(true))
    .order_by_asc(cake::Column::Name)
    .build();

assert_eq!(
    sql,
    r#"SELECT "cake"."id", "cake"."name", CAST("cake"."flavor" AS text), "cake"."price", "cake"."gluten_free", "cake"."bakery_id" FROM "cake" WHERE "cake"."gluten_free" = $1 ORDER BY "cake"."name" ASC"#
);
assert_eq!(values.0, [true.into()]);
```

The flavor column is a PostgreSQL enum, and pgorm reads an enum as its text,
so a row decodes without knowing the type's internal id.

## A first write

Writes go through an [`ActiveModel`](crate::ActiveModelTrait), a row in which
each column is either set to a value or left out. [`set`](crate::set) sets
one, and a column left out takes its default in the database:

```rust
# use pgorm_guide_support::cake::{self, Flavor};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("getting_started_first_write").await;
# let pool = scratch.pool.clone();
use pgorm::entity::prelude::*;

let db = pool.get().await?;
let scone = cake::ActiveModel {
    name: set("Lemon Scone"),
    flavor: set(Flavor::Lemon),
    price: set(Decimal::new(250, 2)),
    gluten_free: set(false),
    bakery_id: set(1),
    ..Default::default()
}
.insert(&db)
.await?;

// The database chose the id; the row comes back as it was stored.
assert_eq!(scone.id, 6);
assert_eq!(scone.price.to_string(), "2.50");
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

[Writing rows](crate::guide::writing) covers inserts, updates, deletes and upserts
in full.

## Where the bindings fit

pgorm also runs from Python and from JavaScript. The Python package `pgorm`
and the npm package `@necessary/pgorm` each wrap this crate: the same pool,
the same query builder and pipeline, the same transactions and `COPY`, in
their own language. Their guides are dialects of this one, with the same
chapters in the same order, and [Python and JavaScript](crate::guide::bindings)
introduces them.

## Where to go next

The chapters build on one another, but each can be read on its own.
[Entities](crate::guide::entities) declares the bakery's tables;
[Writing rows](crate::guide::writing) and [Reading rows](crate::guide::reading) cover the
everyday work; and the chapters after them each take one feature in turn.
