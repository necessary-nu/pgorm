# Raw SQL

The models and the query builder cover most statements, but not all of them,
and some SQL is clearer written out. pgorm runs SQL you write as readily as
SQL it builds: the statement is text, its values travel beside it as
parameters, and the rows come back decoded into JavaScript values by the
types the server reports.

This chapter runs SQL text on a pool, a connection or a transaction, reads
its rows, and says where the Rust guide's compile-time checks stand in
JavaScript. It is the JavaScript dialect of the [Rust guide's
chapter](../../docs/guide/09-raw-sql.md).

## Running a statement

A pool, a connection and a transaction each run SQL text through the same
four terminals, which every [`Queryable`](../lib/index.d.ts) has. The
statement's parameters are written `$1`, `$2` and so on, and their values are
passed beside it as an array:

| Method | Runs the statement and resolves with |
| --- | --- |
| `execute` | the number of rows it changed |
| `one` | its one row; any other number of rows is a [`DecodeError`](../lib/index.d.ts) |
| `optional` | its row, or `null`; more than one row is a `DecodeError` |
| `query` | all of its rows |
| `stream` | a stream of its rows, read as they arrive, on a pool or a connection |

Each row is a plain object keyed by column name, in column order, each value
decoded by its column's type: a `numeric` as a `Decimal`, an `int4` as a
`number`, an enum as its label ([Values and types](13-values.md)). Rust's rows
are tokio-postgres `Row`s, read by position or name with `get`:

```ts
import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";

const raised = await pool.execute(
  "UPDATE cake SET price = price + $1 WHERE bakery_id = $2",
  [new Decimal("0.50"), 2],
);
assert.equal(raised, 2);

const row = await pool.one("SELECT name, price FROM cake WHERE id = $1", [3]);
assert.equal(row.name, "Carrot Cake");
assert.equal(String(row.price), "4.30");

const none = await pool.optional("SELECT name FROM cake WHERE id = $1", [99]);
assert.equal(none, null);
```

A parameter's value is bound as the type the server reads the placeholder as,
from a JavaScript value of a type that can hold it: the `2` above binds as an
`integer` because `bakery_id` is one, and the `Decimal` as a `numeric`. Where
nothing around a placeholder gives it a type, cast it in the SQL, `$1::numeric`.
A value its type cannot take is refused before anything runs, as [Errors](09-errors.md)
shows.

Each terminal prepares its statement on the connection once and reuses it
the next time the same text runs there, so a statement written once and run
often is parsed and planned once per connection. [Statement caches and
DDL](19-statement-caches.md) covers what that means after a schema change.

```ts
import assert from "node:assert/strict";

await using db = await pool.acquire();
await db.query("SELECT name FROM bakery");
await db.query("SELECT name FROM bakery");

const prepared = await db.one(
  "SELECT count(*) AS n FROM pg_prepared_statements WHERE starts_with(statement, $1)",
  ["SELECT name FROM bakery"],
);
assert.equal(prepared.n, 1n);
```

Because each terminal prepares what it runs, a statement is one statement. A
script of several, separated by `;`, is refused by the server, since a
prepared statement holds one command; run each statement of a script in
turn.

**Not available in JavaScript yet:** running a whole script of
`;`-separated statements in one call, which Rust's `batch_execute` does
through PostgreSQL's simple-query protocol, preparing nothing and taking no
parameters.

```ts
import assert from "node:assert/strict";
import { DatabaseError } from "@necessary/pgorm";

await assert.rejects(
  pool.execute("CREATE TABLE tray (id int); CREATE TABLE rack (id int)"),
  (error) => error instanceof DatabaseError && error.sqlstate === "42601",
);

for (const statement of ["CREATE TABLE tray (id int)", "CREATE TABLE rack (id int)"]) {
  await pool.execute(statement);
}
const { tables } = await pool.one(
  "SELECT count(*) AS tables FROM pg_tables WHERE tablename = ANY($1)",
  [["tray", "rack"]],
);
assert.equal(tables, 2n);
```

`stream` reads the rows one at a time instead of collecting them, for a result
too large to hold at once. It is an async iterator, and leaving the loop
early releases what it holds ([Pages, cursors and streams](06-pagination.md)):

```ts
import assert from "node:assert/strict";

const names: string[] = [];
for await (const row of pool.stream("SELECT name FROM cake WHERE gluten_free = $1 ORDER BY name", [true])) {
  names.push(String(row.name));
}
assert.deepEqual(names, ["Chocolate Torte", "Lemon Drizzle", "Vanilla Sponge"]);
```

Rust's `execute_raw` and `query_raw` take their values as an iterator; a
JavaScript statement's values are always an array, so there is no second
form.

## Reading rows

Rust decodes the rows of SQL you wrote into a type you name: a struct by
column name, a tuple by position, an entity's model. JavaScript decodes every
row the same way, into an object by column name, so a statement's column
names are what the program reads. Name each column distinctly: a result with
two columns of one name is a `DecodeError` asking for them to be aliased
apart, where an object would silently keep one of them.

```ts
import assert from "node:assert/strict";
import { Decimal, DecodeError } from "@necessary/pgorm";

const menus = await pool.query(
  `SELECT bakery.name AS bakery, count(cake.id) AS cakes
   FROM bakery JOIN cake ON cake.bakery_id = bakery.id
   WHERE cake.price < $1
   GROUP BY bakery.name ORDER BY bakery.name`,
  [new Decimal("5.00")],
);
assert.deepEqual(menus, [
  { bakery: "Moonlight", cakes: 2n },
  { bakery: "Sunrise", cakes: 2n },
]);

// The first of many is a LIMIT of the SQL's own: pgorm adds none.
const cheapest = await pool.one("SELECT name, price FROM cake ORDER BY price LIMIT 1");
assert.equal(cheapest.name, "Vanilla Sponge");

// Five rows, where `one` wants exactly one.
await assert.rejects(pool.one("SELECT name FROM cake"), DecodeError);

// Two columns called `name`.
await assert.rejects(
  pool.one("SELECT cake.name, bakery.name FROM cake JOIN bakery ON bakery.id = cake.bakery_id WHERE cake.id = 1"),
  DecodeError,
);
```

An enum column reads as its label, so SQL you write selects it as it is.
Rust's entity models read an enum from its text, which is why SQL written for
one casts the column, `flavor::text`; JavaScript needs no cast.

`{ tagged: true }` gives each column as a [`Value`](../lib/index.d.ts)
carrying its kind, which tells an `int2` from an `int8`, an enum's type from
text, and SQL `NULL` from JSON's `null`:

```ts
import assert from "node:assert/strict";

const row = await pool.one(
  "SELECT flavor, price, NULL::text AS note FROM cake WHERE id = $1",
  [2],
  { tagged: true },
);
assert.equal(row.flavor.kind, "enum");
assert.equal(String(row.flavor.typeName), `"public"."flavor"`);
assert.equal(row.flavor.value, "lemon");
assert.equal(row.price.kind, "decimal");
assert.ok(row.note.isNull);
```

**Not available in JavaScript yet:** decoding the rows of SQL you wrote into
a model's records, held to the model's declaration as its own reads are,
which Rust does with `into_model`, `Select::from_raw_sql` and
`FromQueryResult::find_by_statement`. A model's records come from the
model's own reads ([Reading rows](05-reading.md)); SQL text gives plain rows.

## Checking a statement before it runs

Rust's `verify` asks PostgreSQL to describe a statement's result without
running it, and checks every column a decode target reads against that
description, so a mistake shows up before a query that happens to return no
rows hides it.

**Not available in JavaScript yet:** describing a statement's result without
running it, and checking a decode target against the description, as Rust's
`verify` does. A JavaScript row has no decode target to check: a column a
program reads that the statement does not return is `undefined` in the row.

## Statements the query builder built

A statement built with [the query builder](11-query-builder.md) runs through
the same terminals, given in place of the text and its array, and is built as
`inspect()` builds it. `inspect()` returns the same pair of SQL and values,
and the values, `Value`s, are parameters like any other, so the pair runs as
SQL text too. Rust needs each value wrapped in a `ValueHolder` to pass it to
tokio-postgres; JavaScript needs nothing:

```ts
import assert from "node:assert/strict";
import { col, Table, update } from "@necessary/pgorm";

const rename = update(new Table("cake")).set("name", "Lemon Drizzle Loaf").where(col("id").eq(2));
const { sql, values } = rename.inspect();
assert.equal(sql, `UPDATE "cake" SET "name" = $1 WHERE "id" = $2`);

assert.equal(await pool.execute(sql, values), 1);
assert.equal(await pool.execute(rename), 1);
```

## SQL checked before it runs

A typo in a SQL string shows up when the statement first runs, as the server's
`DatabaseError` with SQLSTATE `42601`. Rust's `sql!` macro moves the check to
compile time: it holds a string literal to PostgreSQL 18's own grammar while
the crate compiles, and expands to the literal unchanged. JavaScript has no
compile step for a macro to run in; a program's SQL text is checked by the
server, when it runs:

```ts
import assert from "node:assert/strict";
import { DatabaseError } from "@necessary/pgorm";

await assert.rejects(
  pool.query("SELEC name FROM baker WHERE bakery_id = $1", [1]),
  (error) => error instanceof DatabaseError && error.sqlstate === "42601",
);
```

**Not available in JavaScript yet:** holding SQL text to PostgreSQL's grammar
before it is sent, as Rust's `sql!` does with the parser pgorm links, so a
module could check its statements once, when it loads, rather than each at
its first run.

## PRQL

[PRQL](https://prql-lang.org) is a query language that reads as a pipeline of
steps. Rust's `prql!` macro compiles a PRQL literal to PostgreSQL SQL while
the crate compiles, checks it against the grammar, and arity-checks its `$N`
placeholders against its arguments. JavaScript reaches the same compiler
through [the pipeline](12-pipeline.md), a builder that compiles when it runs,
which binds each value through a binder of the step that takes it:

```ts
import assert from "node:assert/strict";
import { Decimal, pipeline as pl, Table } from "@necessary/pgorm";

const under = new Decimal("4.00");
const cheap = pl.from(new Table("cake"))
  .filterWith((binder) => pl.col("cake", "price").lt(binder.bind(under)))
  .sort(pl.col("cake", "name"))
  .select(pl.col("cake", "name"), pl.col("cake", "price"));

const { sql, values } = cheap.inspect();
assert.equal(sql, "SELECT name, price FROM cake WHERE price < $1 ORDER BY name");
assert.deepEqual(values.map((value) => String(value.value)), ["4.00"]);

const rows = await cheap.all(pool);
assert.deepEqual(rows.map((row) => row.name), ["Carrot Cake", "Lemon Drizzle", "Vanilla Sponge"]);
```

**Not available in JavaScript yet:** compiling PRQL text to SQL, which
Rust's `prql!` does for a PRQL literal and its arguments. JavaScript builds a
PRQL query with the pipeline's steps only.
