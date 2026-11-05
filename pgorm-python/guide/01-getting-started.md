# Getting started

pgorm is an async ORM for PostgreSQL, and for PostgreSQL alone. The `pgorm`
Python package is pgorm itself, the Rust library, loaded into your Python
process as a native extension. You declare each table as a *model*, and pgorm
writes the SQL that reads and writes it. Every value travels to the server as
a bound parameter, never pasted into the SQL text, and each statement is
prepared once per connection and reused.

The package runs on asyncio. A query is a value you build in Python; awaiting
it hands the statement to pgorm's Rust connections, which talk to PostgreSQL
over its own protocol. There is no HTTP service and no query server in
between, and Python holds no SQL renderer of its own: the SQL a Python program
sends is the SQL the Rust library renders. Because pgorm targets one
database, it offers what PostgreSQL has and other databases lack:
`RETURNING`, `ON CONFLICT`, `MERGE`, `COPY`, `LISTEN`/`NOTIFY`, enums,
domains, ranges, arrays and JSON among them.

This guide is the Python dialect of [the Rust guide](../../docs/guide/README.md).
It has the same chapters in the same order, and says where Python differs.
This chapter installs the package, connects to a database and runs a first
query.

## Installing the package

The package is not published to a registry yet. Build its wheel from a
checkout of pgorm's repository with [maturin](https://www.maturin.rs) and
install it. From the repository root:

```sh
uv venv --python 3.14 .venv
uv pip install --python .venv/bin/python 'maturin>=1.15,<2'
.venv/bin/maturin build --manifest-path pgorm-python/Cargo.toml \
  --interpreter .venv/bin/python --out dist --locked
uv pip install --python .venv/bin/python dist/pgorm-*.whl
```

The package needs CPython 3.14, with its global interpreter lock.
Free-threaded Python and subinterpreters are not supported. The build needs a
Rust toolchain, but a Python program that imports the package does not.

The wheel holds the whole of pgorm compiled in, with no optional parts to
choose. `pgorm.capabilities()` reports what the installed build has: its
version, the PostgreSQL release it targets, and the operations it offers.
[PostgreSQL 18 and 19](26-postgres-releases.md) covers the one build option,
the `pg-19` feature, and [Packaging and distribution](28-distribution.md)
covers source distributions and the platforms a wheel is built for.

```python
import pgorm

capabilities = pgorm.capabilities()
assert capabilities["package_version"] == pgorm.__version__
assert capabilities["transport"] == "in-process"
```

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
are customers, with one order each.

The module [`bakery.py`](bakery.py) beside this guide declares these tables
as models, which [Models and entities](03-entities.md) explains. It also has
`create`, which makes the tables in an empty database, and `seed`, which
fills them with the rows above. They are the same tables the Rust guide's
entities describe, so a Rust and a Python program can share one bakery.

Every example in this guide is a whole program. One that talks to the
database reads the database's URL from the `DATABASE_URL` environment
variable and imports what it needs from `bakery`. The guide's tests run each
example against a fresh copy of the bakery, so what one example writes no
other example sees. To run one yourself, put `bakery.py` beside it, create
the bakery with `create` and `seed`, and set `DATABASE_URL`.

## Connecting

A `Pool` holds connections to one database. It
takes a connection URL, and options for its size, its timeouts and TLS, which
[Connection configuration and TLS](20-configuration.md) covers. Constructing a
pool connects to nothing. Using it as an `async with` block checks a
connection when the block starts, so a wrong host or password fails there
rather than at the first query, and closes the pool when the block ends.

```python
import asyncio
import os

from pgorm import Pool, RawSQL


async def main() -> None:
    # The URL of the bakery database, such as
    # "postgres://postgres:postgres@localhost:5432/bakery".
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        row = await pool.one(RawSQL("SELECT count(*) AS cakes FROM cake"))
        assert row["cakes"] == 5


asyncio.run(main())
```

A pool verifies the server's TLS certificate by default. A URL that says
`sslmode=disable`, or `Pool(url, tls="disable")`, connects without TLS, as a
server on your own machine usually needs.

Keep one pool for the life of the program and share it. Here Python differs
from Rust. In Rust a pool is not a connection: a query runs on a connection
checked out of the pool. A Python pool runs a query itself: `pool.one(...)`,
`Cake.find().all(pool)` and the rest each borrow a connection for that one
statement and return it when the statement finishes. Two statements run that
way may run on two different connections. When statements must share one
connection, check one out with `pool.connection()`:

```python
import asyncio
import os

from pgorm import Pool, RawSQL


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        async with pool.connection() as connection:
            await connection.execute(RawSQL("SET application_name = 'counter'"))
            row = await connection.one(RawSQL("SELECT current_setting('application_name') AS name"))
            assert row["name"] == "counter"


asyncio.run(main())
```

The connection returns to the pool when its block ends. A connection runs one
statement at a time: starting a second before the first has finished is an
error, not a queue. A [transaction](08-transactions.md) also holds one
connection for its whole block.

## A first query

The bakery's models are values in the `bakery` module, one per table: `Cake`
for `cake`, `Bakery` for `bakery`, and so on. `find()` starts a `SELECT` of
every field, the builder methods refine it, and a terminal such as `all` runs
it:

```python
import asyncio
import os

from pgorm import Pool

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        gluten_free = await (
            Cake.find()
            .where(Cake.col("gluten_free").eq(True))
            .order_by(Cake.col("name").asc())
            .all(pool)
        )
        names = [cake["name"] for cake in gluten_free]
        assert names == ["Chocolate Torte", "Lemon Drizzle", "Vanilla Sponge"]


asyncio.run(main())
```

`Cake.col("gluten_free")` names a column, and its methods build conditions;
Python's comparison operators build the same ones, so
`Cake.col("gluten_free") == True` is `eq(True)`. A row comes back as a record,
read like a dictionary by field name.

A query is a value until it runs, and it can be rendered without a database.
`inspect()` returns the SQL and the values it binds:

```python
from bakery import Cake

query = Cake.find().where(Cake.col("gluten_free").eq(True)).order_by(Cake.col("name").asc())
compiled = query.inspect()

assert compiled.sql == (
    'SELECT "cake"."id", "cake"."name", "cake"."flavor", "cake"."price", '
    '"cake"."gluten_free", "cake"."bakery_id" FROM "cake" '
    'WHERE "cake"."gluten_free" = $1 ORDER BY "cake"."name" ASC'
)
assert [value.value for value in compiled.params] == [True]
```

The flavor column is a PostgreSQL enum. Rust's entities read an enum through
its text, `CAST("cake"."flavor" AS text)`; a Python model reads the column as
it is, and the driver gives back the label.

## A first write

Writes take a dictionary of the fields to set. A field left out takes its
default in the database, so the cake's id is chosen by the server:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Pool

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        scone = await Cake.insert({
            "name": "Lemon Scone",
            "flavor": "lemon",
            "price": Decimal("2.50"),
            "gluten_free": False,
            "bakery_id": 1,
        }).returning().one(pool)

        # The database chose the id; the row comes back as it was stored.
        assert scone["id"] == 6
        assert str(scone["price"]) == "2.50"


asyncio.run(main())
```

Rust writes through an `ActiveModel`, a row in which each column is set or
left out. A Python model's dictionary says the same thing: a key present is a
column set, a key absent is a column left out. [Writing rows](04-writing.md)
covers inserts, updates, deletes and upserts in full.

## Where the other languages fit

pgorm is a Rust library first, and the Rust guide is the main one. This
package wraps that library, and the npm package `@necessary/pgorm` wraps it
for JavaScript: the same pool, the same query builder and pipeline, the same
transactions and `COPY`, each in its own language. [Python and
JavaScript](27-bindings.md) says how the bindings relate, and [Registering
Rust entities](24-registrations.md) how a Python program uses entities a Rust
crate declares.

## Where to go next

The chapters build on one another, but each can be read on its own. [Python
and asyncio](02-asyncio.md) covers what the package asks of an asyncio
program. [Models and entities](03-entities.md) declares the bakery's tables;
[Writing rows](04-writing.md) and [Reading rows](05-reading.md) cover the
everyday work; and the chapters after them each take one feature in turn.
