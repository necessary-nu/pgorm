# Python and JavaScript

pgorm is a Rust library, and it runs from Python and from JavaScript as
well. Each language has a package that wraps the Rust library through a
native extension, loaded into the program's own process:

- `pgorm` for Python, this package, from `pgorm-python`, over a PyO3
  extension, for CPython 3.14;
- `@necessary/pgorm` for JavaScript, from `pgorm-napi`, over a Node-API
  addon that Node.js 26 or later and Deno 2.9.5 or later both load.

Neither binding reimplements pgorm. Every statement a Python program builds
is built by pgorm-query's builders, rendered by pgorm's renderer, bound and
executed through pgorm's connections, and decoded by pgorm's decoders, in the
Python process, over PostgreSQL's own protocol. There is no HTTP service and
no query server between the program and the database, and no second SQL
renderer whose output could drift from Rust's. What the package writes in
Python is the asyncio side of each operation, Python's types, and a few
conveniences, such as the models of [Models and entities](03-entities.md),
each of which builds its statement with pgorm-query's builders.

The capability manifest names the Rust API behind each operation:

```python
import pgorm

operations = pgorm.capabilities()["operations"]
assert operations["pool"]["rust_api"] == "pgorm::connect_with"
assert operations["pool.clear_statement_caches"]["rust_api"] == "pgorm::DatabasePool::clear_statement_caches"
assert operations["model.insert"]["python_convenience"] is True
```

## What a binding offers

The two bindings offer the same operations, each spelled the way its
language spells names: a pool's `statement_cache_channel` option in Python
is its `statementCacheChannel` in JavaScript. Between them and this guide's
chapters:

- connections and the pool, [transactions](08-transactions.md) and
  savepoints, [COPY](17-copy.md), [LISTEN and NOTIFY](18-listen-notify.md),
  and [statement caches](19-statement-caches.md);
- the [query builder](11-query-builder.md) and the
  [pipeline](12-pipeline.md), composed at run time over table and column
  names the program supplies;
- schema statements, the DDL builders of [Schema generation](14-schema.md);
- models the program declares over its own tables, read and written as
  [Reading rows](05-reading.md) and [Writing rows](04-writing.md) describe;
- registrations: a Rust application compiles its own entities, graphs and
  pipeline sources into its build of the binding, and the program reads and
  writes them through the real `Select<E>` and `ActiveModel` of each entity,
  hooks included. Code generation writes the registration crate and the
  typed Python or TypeScript declarations for it ([Registering Rust
  entities](24-registrations.md)).

## One statement, three languages

The bakery's cakes over 4.00, built with the query builder in Rust, where
`db` is a connection checked out of the pool:

```rust
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::Query;
use pgorm_guide_support::cake;

let (sql, values) = Query::select()
    .column((cake::Entity, cake::Column::Name))
    .from(cake::Entity)
    .and_where(Expr::col((cake::Entity, cake::Column::Price)).gt(4))
    .build();
assert_eq!(sql, r#"SELECT "cake"."name" FROM "cake" WHERE "cake"."price" > $1"#);

let mut names: Vec<String> = (sql, values).into_tuple().all(&db).await?;
names.sort();
assert_eq!(names, ["Chocolate Fudge", "Chocolate Torte"]);
```

The same statement in Python, through the same builder:

```python
import asyncio
import os

from pgorm import Pool, Table, select


async def main() -> None:
    cake = Table("cake")
    query = select(cake.col("name")).from_(cake).where(cake.col("price") > 4)
    assert query.inspect().sql == 'SELECT "cake"."name" FROM "cake" WHERE "cake"."price" > $1'

    async with Pool(os.environ["DATABASE_URL"]) as pool:
        names = sorted(row["name"] for row in await pool.query(query))
        assert names == ["Chocolate Fudge", "Chocolate Torte"]


asyncio.run(main())
```

And in JavaScript:

```javascript
import { Pool, Table, select } from "@necessary/pgorm";

const cake = new Table("cake");
const query = select(cake.col("name")).from(cake).where(cake.col("price").gt(4));

await using pool = new Pool(databaseUrl);
const names = (await pool.query(query)).map((row) => row.name);
```

Each binding's `inspect()` returns the SQL and the values its builder
holds, as Rust's `build` does: in Python a `Compiled`, with `sql` and
`params`; in JavaScript an object of `sql` and `values`. The values differ
in one way here: Python reads a plain `4` as an `int8` and Rust's `4` is an
`int4`, so each sends the kind its language's integer is. The SQL is the
same.

## Names and shapes in each language

Each binding spells a shared name by its language's rule:

- Python writes functions, methods and keyword arguments in snake_case,
  and JavaScript in camelCase: `clear_statement_caches()` and
  `clearStatementCaches()`, `insert_many` and `insertMany`.
- A call's options are keyword arguments in Python and the keys of an
  options object, its last argument, in JavaScript.
- A name that is a keyword takes a trailing underscore in Python (`from_`,
  `as_`, `with_`, `except_`); JavaScript spells `delete` as `deleteFrom`.
- A closed choice is a class of constants in Python (`Join.Left`,
  `Direction.Asc`) and a string in JavaScript (`"left"`).
- Python makes an object by calling its class (`Model(...)`, `Column(...)`,
  `CopyFrom(...)`); JavaScript calls a function named for it (`model(...)`,
  `column(...)`, `copyFrom(...)`).
- Python's operators build expressions (`==`, `>`, `&`, `+`), beside the
  methods (`eq`, `gt`); JavaScript has the methods alone.

The shapes follow each language too. In Python a pool, a connection and a
stream are closed by an `async with` block, and a transaction is one, `async
with pool.transaction() as tx:`; in JavaScript a pool is closed by `await
using`, and a transaction is a callback, `pool.transaction(async (tx) =>
...)`. Timeouts are seconds in Python and milliseconds in JavaScript. SQL
text runs as a `RawSQL(sql, values)` in Python ([Raw SQL](10-raw-sql.md)),
and as text and parameters handed to the terminal in JavaScript. A
registered entity's model is a mapping with methods of its own in Python
(`into_active`, `with_value`, `tagged`); in JavaScript it is a frozen plain
object, and those operations are its entity's.

## Values keep their PostgreSQL types

Values cross between the languages with their PostgreSQL types kept: an
`int2` stays apart from an `int8`, SQL `NULL` from JSON `null`, and a
`timestamp` from a `timestamptz`. Python carries dates and times as its
`datetime` values, and JavaScript as `Temporal` ones. Where Python's own
value cannot say which, a record's `tagged(name)` gives the column as a
`Value` with its kind ([Values and types](13-values.md)):

```python
import asyncio
import os
from datetime import datetime, timezone

from pgorm import Pool, RawSQL, Value


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        row = await pool.one(RawSQL(
            "SELECT 1::int2 AS small, 1::int8 AS big, 'null'::json AS json_null, NULL::json AS sql_null, "
            "'2026-11-02 09:30'::timestamp AS local, '2026-11-02 09:30Z'::timestamptz AS utc"
        ))

        # Both are Python ints; each keeps its width.
        assert row["small"] == row["big"] == 1
        assert row.tagged("small") == Value(1, "i16")
        assert row.tagged("big") == Value(1, "i64")

        # Both read as None; one is JSON's null, the other SQL's NULL.
        assert row["json_null"] is None and row["sql_null"] is None
        assert row.tagged("json_null") == Value.json(None)
        assert row.tagged("sql_null") == Value.null("json")

        # A timestamp is a naive datetime, a timestamptz an aware one in UTC.
        assert row["local"] == datetime(2026, 11, 2, 9, 30)
        assert row["utc"] == datetime(2026, 11, 2, 9, 30, tzinfo=timezone.utc)


asyncio.run(main())
```

## Installing

Neither package is published to a registry yet. Each is built from a
checkout of pgorm's repository:

- Python: `maturin build --manifest-path pgorm-python/Cargo.toml`, then
  install the wheel it writes. [Getting started](01-getting-started.md) has
  the full commands, and [Packaging and distribution](28-distribution.md)
  what a wheel holds.
- JavaScript: `node pgorm-napi/scripts/build.mjs` builds the addon into
  `pgorm-napi/lib/`, and `pgorm-napi/lib/index.js` is the module to import.
  Once published, `npm install @necessary/pgorm` installs the package and
  the prebuilt addon for the platform beside it.

Each binding is a Cargo workspace of its own, so building pgorm, or a Rust
program that depends on it, never compiles Python, PyO3, Node-API or their
linker settings.

## What a build reports

The Python package targets PostgreSQL 18 as pgorm does, and forwards the
`pg-19` feature ([PostgreSQL 18 and 19](26-postgres-releases.md)) to pgorm.
`pgorm.capabilities()` reports the installed build: its version and
pgorm's, the features it was built with, `pg-19` among them in a 19 build,
the operations it offers with the Rust API behind each, and the entities,
graphs and other registrations an application wheel compiled in.

Here the two bindings differ. The capability manifest, `capabilities()` and
`require_capability()`, is the Python package's alone, and so is the
`UnsupportedCapabilityError` an operation outside it raises. The JavaScript
module exports its `version`, the pgorm release it is built from, and
targets PostgreSQL 18 alone: it has no `pg-19` feature yet.

## Where their documentation lives

A binding's guide follows [the Rust guide](../../docs/guide/README.md)
chapter for chapter, in its own language, says where its binding differs
from Rust, in shape, spelling or reach, and adds what its language needs:
installing, the async runtime, typing and packaging. This guide is the
Python one. The JavaScript guide is [`pgorm-napi/guide`](../../pgorm-napi/guide/README.md),
with the same chapters for Node.js and Deno, and
[DISTRIBUTION.md](../../pgorm-napi/DISTRIBUTION.md) beside it covers its npm
packages.

The rules both bindings follow are in
[docs/spec/bindings.md](../../docs/spec/bindings.md), with what differs by
language in [docs/spec/python.md](../../docs/spec/python.md) and
[docs/spec/napi.md](../../docs/spec/napi.md).
[docs/spec/bindings-surface.json](../../docs/spec/bindings-surface.json)
lists the names both bindings offer, and each name one binding offers alone,
with the reason the other has no counterpart.
