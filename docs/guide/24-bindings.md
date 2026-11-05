# Python and JavaScript

pgorm runs from Python and from JavaScript as well as from Rust. Each language
has a package that wraps this crate through a native extension, loaded into
the program's own process:

- `pgorm` for Python, from `pgorm-python`, over a PyO3 extension, for CPython
  3.14;
- `@necessary/pgorm` for JavaScript, from `pgorm-napi`, over a Node-API
  addon that Node.js 26 or later and Deno 2.9.5 or later both load.

A binding does not reimplement pgorm. Every statement it builds is built by
pgorm-query's builders, rendered by pgorm's renderer, bound and executed
through pgorm's connections, and decoded by pgorm's decoders, in the program's
process over PostgreSQL's own protocol. There is no HTTP service and no query
server between the program and the database, and no second SQL renderer whose
output could drift from Rust's.

## What a binding offers

The two bindings offer the same operations, each spelled the way its language
spells names: a pool's `statement_cache_channel` option in Python is its
`statementCacheChannel` in JavaScript. Between them and this guide's chapters:

- connections and the pool, transactions and savepoints, `COPY`, `LISTEN` and
  `NOTIFY`, and statement caches, as in the chapters of those names;
- the [query builder](crate::guide::query_builder) and the
  [pipeline](crate::guide::pipeline), composed at run time over table and column
  names the program supplies;
- schema statements, the DDL builders of [Schema
  generation](crate::guide::schema);
- models the program declares over its own tables, read and written as
  [Reading rows](crate::guide::reading) and [Writing rows](crate::guide::writing) describe;
- registrations: a Rust application compiles its own entities, graphs and
  pipeline sources into its build of the binding, and the language reads and
  writes them through the real `Select<E>` and `ActiveModel` of each entity,
  hooks included. Code generation writes the registration crate and the
  typed Python or TypeScript declarations for it.

Values cross between the languages with their PostgreSQL types kept: an
`int2` stays apart from an `int8`, SQL `NULL` from JSON `null`, and a
`timestamp` from a `timestamptz`. Python carries dates and times as its
`datetime` values, and JavaScript as `Temporal` ones.

## One statement, three languages

The bakery's cakes over 4.00, built with the query builder in Rust:

```rust
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::Query;
use pgorm_guide_support::cake;

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("bindings_one_statement").await;
# let db = scratch.pool.get().await?;
let (sql, values) = Query::select()
    .column((cake::Entity, cake::Column::Name))
    .from(cake::Entity)
    .and_where(Expr::col((cake::Entity, cake::Column::Price)).gt(4))
    .build();
assert_eq!(sql, r#"SELECT "cake"."name" FROM "cake" WHERE "cake"."price" > $1"#);

let mut names: Vec<String> = (sql, values).into_tuple().all(&db).await?;
names.sort();
assert_eq!(names, ["Chocolate Fudge", "Chocolate Torte"]);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

The same statement in Python, through the same builder:

```python
from pgorm import Pool, Table, select

cake = Table("cake")
query = select(cake.col("name")).from_(cake).where(cake.col("price") > 4)

async with Pool(database_url) as pool:
    names = [row["name"] for row in await pool.query(query)]
```

And in JavaScript:

```javascript
import { Pool, Table, select } from "@necessary/pgorm";

const cake = new Table("cake");
const query = select(cake.col("name")).from(cake).where(cake.col("price").gt(4));

await using pool = new Pool(databaseUrl);
const names = (await pool.query(query)).map((row) => row.name);
```

Each binding's `inspect()` returns the SQL and the parameters its builder
holds, as Rust's [`build`](crate::QueryTrait::build) does.

## Installing

Neither package is published to a registry yet. Each is built from a checkout
of pgorm's repository:

- Python: `maturin build --manifest-path pgorm-python/Cargo.toml`, then
  install the wheel it writes. The Python guide's first chapter,
  `pgorm-python/guide/01-getting-started.md`, has the full commands.
- JavaScript: `node pgorm-napi/scripts/build.mjs` builds the addon into
  `pgorm-napi/lib/`, and `pgorm-napi/lib/index.js` is the module to import.
  Once published, `npm install @necessary/pgorm` installs the package and the
  prebuilt addon for the platform beside it.

Each binding is a Cargo workspace of its own, so building pgorm, or a program
that depends on it, never compiles Python, PyO3, Node-API or their linker
settings.

## The release a binding targets

A binding targets PostgreSQL 18 as pgorm does. pgorm-python forwards the
`pg-19` feature ([PostgreSQL 18 and 19](crate::guide::postgres_releases)) to pgorm,
and reports the features of its build through `pgorm.capabilities()`, which
also lists the operations and registrations the installed build has.

## Where their documentation lives

The bindings' guides follow this one chapter for chapter, in the order the
guide's chapter list gives, each in its own language. Each says where its
binding differs from Rust, in shape, spelling or reach, and adds what its
language needs: installing, the async runtime, typing, packaging. Each
lives in its binding's directory of pgorm's repository: the Python guide is
`pgorm-python/guide/`, whose index is `pgorm-python/guide/README.md`, and the
JavaScript documentation is `pgorm-napi/README.md`, `ENTITIES.md`, `CODEGEN.md`
and `DISTRIBUTION.md`. The rules both bindings
follow are in `docs/spec/bindings.md`, with what differs by language in
`docs/spec/python.md` and `docs/spec/napi.md`.
