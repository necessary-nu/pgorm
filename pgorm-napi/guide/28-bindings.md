# Python and JavaScript

pgorm runs from Rust, from Python and from JavaScript. Each language other
than Rust has a package that wraps the Rust crate through a native extension,
loaded into the program's own process:

- `pgorm` for Python, from `pgorm-python`, over a PyO3 extension, for CPython
  3.14;
- `@necessary/pgorm` for JavaScript, this package, from `pgorm-napi`, over a
  Node-API addon that Node.js 26 or later and Deno 2.9.5 or later both load.

This chapter says how this package relates to pgorm and to its Python
sibling. It is the JavaScript dialect of the [Rust guide's
chapter](../../docs/guide/24-bindings.md).

## One implementation

A binding does not reimplement pgorm. Every statement this package builds is
built by pgorm-query's builders, rendered by pgorm's renderer, bound and
executed through pgorm's connections, and decoded by pgorm's decoders, in the
program's process over PostgreSQL's own protocol. There is no HTTP service
and no query server between the program and the database, and no second SQL
renderer whose output could drift from Rust's. The package's own tests hold
many of its statements to the SQL the same statements render to in Rust.

## What the package offers

The two bindings offer the same operations, each spelled the way its
language spells names: a pool's `statement_cache_channel` option in Python
is its `statementCacheChannel` here, and both name Rust's `one_opt`
`optional`. Between them and the chapters of this guide:

- connections and the pool, transactions and savepoints, `COPY`, `LISTEN` and
  `NOTIFY`, and statement caches, as in the chapters of those names;
- the [query builder](11-query-builder.md) and the [pipeline](12-pipeline.md),
  composed at run time over table and column names the program supplies;
- schema statements, the DDL builders of [Schema generation](14-schema.md);
- models the program declares over its own tables, read and written as
  [Reading rows](05-reading.md) and [Writing rows](04-writing.md) describe;
- registrations: a Rust application compiles its own entities, graphs and
  pipeline sources into its build of the package, and JavaScript reads and
  writes them through the real `Select<E>` and `ActiveModel` of each entity,
  hooks included ([Registered Rust entities](24-registered-entities.md)).
  Code generation writes the registration crate and the TypeScript
  declarations for it ([Generating
  registrations](25-generating-registrations.md)).

Values cross between the languages with their PostgreSQL types kept: an
`int2` stays apart from an `int8`, SQL `NULL` from JSON `null`, and a
`timestamp` from a `timestamptz`. JavaScript carries dates and times as
`Temporal` values, and Python as its `datetime` ones ([Values and
types](13-values.md)).

What Rust has and this package does not yet, each chapter names where it
arises: among them the metrics layer, the migration runner and entity
generation from DDL, which are Rust crates of their own ([Metrics](21-metrics.md),
[Migrations](22-migrations.md), [Generating entities](23-codegen.md)).

## One statement, three languages

The bakery's cakes over 4.00, built with the query builder:

```ts
import assert from "node:assert/strict";
import { select, Table } from "@necessary/pgorm";

const cake = new Table("cake");
const query = select(cake.col("name")).from(cake).where(cake.col("price").gt(4));
assert.equal(query.inspect().sql, `SELECT "cake"."name" FROM "cake" WHERE "cake"."price" > $1`);

const names = (await pool.query(query)).map((row) => row.name).sort();
assert.deepEqual(names, ["Chocolate Fudge", "Chocolate Torte"]);
```

The same statement in Rust, through the same builder:

```rust
let (sql, values) = Query::select()
    .column((cake::Entity, cake::Column::Name))
    .from(cake::Entity)
    .and_where(Expr::col((cake::Entity, cake::Column::Price)).gt(4))
    .build();
```

And in Python:

```python
from pgorm import Table, select

cake = Table("cake")
query = select(cake.col("name")).from_(cake).where(cake.col("price") > 4)
names = [row["name"] for row in await pool.query(query)]
```

Each binding's `inspect()` returns the SQL and the parameters its builder
holds, as Rust's `build` does.

## Installing

The package is not published to a registry yet. It is built from a checkout
of pgorm's repository, `node pgorm-napi/scripts/build.mjs`, and
`pgorm-napi/lib/index.js` is the module to import ([Getting
started](01-getting-started.md#installing)). The package lives in a Cargo
workspace of its own, so building pgorm, or a program that depends on it,
never compiles Node-API or its linker settings.

## The release the package targets

The package targets PostgreSQL 18, as pgorm does by default ([PostgreSQL 18
and 19](27-postgres-releases.md)). Its `version` export is the pgorm release
the addon was built from:

```ts
import assert from "node:assert/strict";
import { version } from "@necessary/pgorm";

assert.match(version, /^\d+\.\d+\.\d+/);
```

## Where the documentation lives

This guide is the JavaScript dialect of [the Rust guide](../../docs/guide/README.md),
chapter for chapter. Python's guide is its Python dialect, in
[`pgorm-python/guide`](../../pgorm-python/guide/README.md). The rules both bindings follow are in
[`docs/spec/bindings.md`](../../docs/spec/bindings.md), and what differs for
JavaScript in [`docs/spec/napi.md`](../../docs/spec/napi.md).
[DISTRIBUTION.md](../DISTRIBUTION.md) covers this package's npm packages: the
platforms, the addon's contents, and how they are built and checked.
