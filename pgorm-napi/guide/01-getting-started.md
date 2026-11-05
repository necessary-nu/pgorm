# Getting started

pgorm is an async ORM for PostgreSQL, and for PostgreSQL alone. The npm
package `@necessary/pgorm` runs it from Node.js and Deno. You declare each
table as a *model*, a JavaScript value naming its columns, and pgorm writes
the SQL that reads and writes it. Every value travels to the server as a
bound parameter, never pasted into the SQL text, and each statement is
prepared once per connection and reused.

The package is pgorm itself, not a port of it. A native addon holds pgorm's
Rust code: its pool, its query builder and pipeline, its transactions and
`COPY`, run on a runtime of its own inside your process. Every operation
returns a Promise, and the process still exits by itself when its work is
done. Because pgorm targets one database, it can offer what PostgreSQL has
and other databases lack: `RETURNING`, `ON CONFLICT`, `MERGE`, `COPY`,
`LISTEN`/`NOTIFY`, enums, domains, ranges, arrays and JSON among them.

This chapter installs the package, connects to a database and runs a first
query. It is the JavaScript dialect of the [Rust guide's
chapter](../../docs/guide/01-getting-started.md) of the same name.

## What it needs

The package needs a runtime with `Temporal` as a global, because dates and
times cross into JavaScript as Temporal values: Node.js 26 or later, or Deno
2.9.5 or later. The addon is prebuilt for macOS on arm64 and Linux on x86-64
with glibc; on any other platform, build it from a checkout as
[below](#building-from-a-checkout).

A pool targets PostgreSQL 18 or later, as pgorm does ([PostgreSQL 18 and
19](27-postgres-releases.md)).

## Installing

The package is a main package, `@necessary/pgorm`, of JavaScript and type
declarations, and one package per platform holding its prebuilt addon, which
npm and Deno install beside it. Nothing is published to npm yet. Until it is,
build the addon from a checkout. Once published, Node.js installs it with npm:

```sh
npm install @necessary/pgorm
```

A Deno project names it in its `deno.json`, and `deno install` fetches it
with the platform package that matches the machine:

```json
{
  "nodeModulesDir": "auto",
  "imports": { "@necessary/pgorm": "npm:@necessary/pgorm@^0.2.0" }
}
```

`import { connect } from "npm:@necessary/pgorm@^0.2.0"` works without the
import map too.

### Deno's permissions

Deno opens a native library only with `--allow-ffi`, and finds the addon only
with `--allow-read`:

```sh
deno run --allow-ffi --allow-read app.ts
```

Native code runs outside Deno's permission checks. Once a program may load
the addon, the addon can do anything the process can, whatever `--allow-net`
or `--deny-*` say. That is why pgorm needs no `--allow-net` to reach
PostgreSQL, and why `--allow-net` does not limit what it can reach. Granting
`--allow-ffi` trusts the addon as fully as running the program under Node.js
does. [DISTRIBUTION.md](../DISTRIBUTION.md) covers the packages, the
platforms and what is compiled into the addon.

### Building from a checkout

```sh
node pgorm-napi/scripts/build.mjs            # a debug build
node pgorm-napi/scripts/build.mjs --release  # the release build the packages ship
```

The script runs Cargo and copies the library it builds to
`pgorm-napi/lib/pgorm_napi.node`. The module loads a library it finds there
before it looks for an installed platform package, so a checkout runs its own
build. Import `pgorm-napi/lib/index.js` from the checkout where an installed
program imports `@necessary/pgorm`. Never import the `.node` file itself.

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
are customers, with one order each. [Entities](03-entities.md) declares a
model for each of these tables.

Each example runs against a fresh copy of the bakery in a database of its
own, so what one example writes no other example sees. An example names the
bakery's models, `Bakery`, `Cake` and the rest, without importing them, and
names `pool`, a pool connected to its copy, and `url`, that database's
connection string, without making them. The guide's tests supply all three,
as the Rust guide's hidden lines do, so the examples show only the code each
chapter teaches. The models' full source is
[`guide/bakery.ts`](bakery.ts).

## Connecting

A [`Pool`](../lib/index.d.ts) is a set of connections to one database. Its
constructor takes a connection string and sends nothing: the pool opens
connections when they are first needed, so a wrong host or password shows up
at the first statement. [`connect`](../lib/index.d.ts) builds a pool and
pings the server, so a mistake fails at startup instead:

```ts
import assert from "node:assert/strict";
import { connect } from "@necessary/pgorm";

// The URL of the bakery database, such as
// "postgres://postgres:postgres@localhost:5432/bakery?sslmode=disable".
await using pool = await connect(url);

const row = await pool.one("SELECT count(*) AS cakes FROM cake");
assert.equal(row.cakes, 5n);
```

`await using` closes the pool when the block it is declared in ends, as
`pool.close()` does. A pool connects over TLS and verifies the server's
certificate and host name unless the connection string says
`sslmode=disable`, and it never falls back to plain text. Rust's
`pgorm::connect` connects in plain text unless it is given a TLS connector;
[Connection configuration and TLS](20-configuration.md) covers both.

Keep the pool for the life of the program and share it. A `count(*)` is an
`int8`, which JavaScript reads as a `bigint`, `5n`, so that no integer is
ever rounded ([Values and types](13-values.md)).

### The pool runs statements too

Here JavaScript differs from Rust. Rust's `DatabasePool` is not a
connection, and a query runs on a connection taken from it. A JavaScript
`Pool` runs statements itself: each statement run on the pool takes a
connection for itself alone, and returns it when the statement is done. Two
statements on a pool may therefore run on two different connections.

Work that has to share one session, such as a `SET` that the next statement
relies on, takes a [`Connection`](../lib/index.d.ts) of its own with
`acquire()`. It is the caller's until it is closed, and runs one operation at
a time:

```ts
import assert from "node:assert/strict";

await using db = await pool.acquire();
await db.execute("SET application_name = 'bakery-web'");
const row = await db.one("SELECT current_setting('application_name') AS name");
assert.equal(row.name, "bakery-web");
```

A transaction always holds one connection, so the statements inside it share
a session ([Transactions](08-transactions.md)).

## A first query

A model is a value, and each of its methods starts a statement over its
table. [`find`](../lib/models.d.ts) starts a `SELECT` of every column, the
builder methods refine it, and a terminal such as `all` runs it on a pool, a
connection or a transaction:

```ts
import assert from "node:assert/strict";

const glutenFree = await Cake.find()
  .where(Cake.col("glutenFree").eq(true))
  .orderBy(Cake.col("name").asc())
  .all(pool);

assert.deepEqual(
  glutenFree.map((cake) => cake.name),
  ["Chocolate Torte", "Lemon Drizzle", "Vanilla Sponge"],
);
```

Each row decodes into a *record*, a plain object keyed by the model's field
names, checked against the declaration as it decodes. `Cake.col("glutenFree")`
names the field; the model knows that its column is `gluten_free`.

A query is a value until it runs, and it renders without a database.
`inspect()` returns the SQL and the values it binds:

```ts
import assert from "node:assert/strict";

const { sql, values } = Cake.find()
  .where(Cake.col("glutenFree").eq(true))
  .orderBy(Cake.col("name").asc())
  .inspect();

assert.equal(
  sql,
  `SELECT "cake"."id", "cake"."name", "cake"."flavor", "cake"."price", "cake"."gluten_free", "cake"."bakery_id" FROM "cake" WHERE "cake"."gluten_free" = $1 ORDER BY "cake"."name" ASC`,
);
assert.deepEqual(values.map((value) => value.value), [true]);
```

The flavor column is a PostgreSQL enum. A Rust entity selects it cast to
`text`; a JavaScript model selects it as it is and reads its label, checking
that the result's type is the enum the model declares.

## A first write

A write takes a plain object of the fields it sets. A field left out takes
its column's default in the database, and `returning()` reads the row back as
it was stored:

```ts
import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";

const scone = await Cake.insert({
  name: "Lemon Scone",
  flavor: "lemon",
  price: new Decimal("2.50"),
  glutenFree: false,
  bakeryId: 1,
}).returning().one(pool);

// The database chose the id; the row comes back as it was stored.
assert.equal(scone.id, 6);
assert.equal(scone.price.toString(), "2.50");
```

A price is a `Decimal`, never a `number`, because a float cannot hold most
amounts of money exactly. Rust writes through an `ActiveModel`, a row whose
columns are each set or left out; a JavaScript model's write is the object
itself, and [Writing rows](04-writing.md) covers inserts, updates, deletes and
upserts in full.

## Where the bindings fit

pgorm also runs from Python, and from Rust itself. The three share one
implementation, so a statement built here renders the same SQL there.
[Python and JavaScript](28-bindings.md) introduces the bindings, and each
chapter of this guide follows the Rust chapter of the same name.

## Where to go next

The chapters build on one another, but each can be read on its own. [Promises,
cancellation and cleanup](02-promises.md) says how every operation settles,
stops and releases what it holds; [Entities](03-entities.md) declares the
bakery's models; [Writing rows](04-writing.md) and [Reading
rows](05-reading.md) cover the everyday work; and the chapters after them each
take one feature in turn. A chapter the Rust guide has no counterpart of says
so in its first lines.
