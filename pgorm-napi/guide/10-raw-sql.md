# Raw SQL

The models and the query builder cover most statements, but not all of them,
and some SQL is clearer written out. pgorm runs SQL you write as readily as
SQL it builds: the statement is text, its values travel beside it as
parameters, and the rows come back decoded into JavaScript values by the
types the server reports.

This chapter runs SQL text on a pool, a connection or a transaction, reads
its rows, decodes them into a model's records, checks a statement against a
model before it runs, and holds SQL and PRQL text to PostgreSQL's grammar,
the run-time counterparts of the Rust guide's compile-time checks. It is the
JavaScript dialect of the [Rust guide's chapter](../../docs/guide/09-raw-sql.md).

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
| `stream` | a stream of its rows, read as they arrive |
| `batchExecute` | nothing: it runs a whole script of `;`-separated statements, with no parameters |

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
prepared statement holds one command. `batchExecute`, as Rust's
`batch_execute`, sends a whole script as it is through PostgreSQL's
simple-query protocol, preparing nothing. It takes no parameters, so it is
for DDL and fixed scripts, never for values a user supplied. A pool runs it
on a connection of its own; a connection and a transaction run it on what
they hold:

```ts
import assert from "node:assert/strict";
import { DatabaseError } from "@necessary/pgorm";

const script = "CREATE TABLE tray (id int); CREATE TABLE rack (id int)";
await assert.rejects(
  pool.execute(script),
  (error) => error instanceof DatabaseError && error.sqlstate === "42601",
);

await pool.batchExecute(script);
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

## Decoding rows into models

A model's `findByStatement(sql, params)` decodes the rows of SQL you wrote
into the model's records, as Rust's `FromQueryResult::find_by_statement`
does. Each field is read from the column of its SQL name, `bakery_id` for
`bakeryId`, and held to the kind the model declares, as the model's own
reads hold it ([Reading rows](05-reading.md)); a column no field reads is
left unread, and a field whose column the statement does not return is a
`DecodeError`. An enum column reads as its label, so `SELECT *` decodes into
`Cake` with no cast, where a Rust entity's model reads the enum from its
text. It gives a `SelectorRaw`, whose `all`, `one`, `optional` and `stream`
run the statement as written; pgorm adds nothing to it, so a query for the
first of many says `LIMIT 1` itself:

```ts
import assert from "node:assert/strict";
import { DecodeError } from "@necessary/pgorm";

const glutenFree = await Cake.findByStatement(
  "SELECT *, 'unread' AS note FROM cake WHERE gluten_free = $1 ORDER BY id",
  [true],
).all(pool);
assert.deepEqual(glutenFree.map((cake) => cake.name), ["Lemon Drizzle", "Vanilla Sponge", "Chocolate Torte"]);

const lemon = await Cake.findByStatement("SELECT * FROM cake WHERE id = $1", [2]).one(pool);
assert.equal(lemon.flavor, "lemon");
assert.equal(lemon.bakeryId, 1);

// The statement returns no price, which Cake reads.
await assert.rejects(Cake.findByStatement("SELECT id, name FROM cake").all(pool), DecodeError);
```

Rust starts the same decode from an entity's `find()` too, with
`Select::from_raw_sql`; a JavaScript model's reads are built, and
`findByStatement` is how a statement you wrote becomes its records.
`fromStatement(sql, params)` gives a `SelectorRaw` of plain rows, as Rust's
`SelectorRaw::from_statement` holds a statement: it runs as the terminals
run the text, and pages and counts ([Pages, cursors and
streams](06-pagination.md)).

## Checking a statement before it runs

Rust's `verify` asks PostgreSQL to describe a statement's result without
running it, and checks every column a decode target reads against that
description, so a mistake shows up before a query that happens to return no
rows hides it.

`verify(sql, model)` on a pool, a connection or a transaction does the same
for a declared model: it asks the server to describe the statement through
pgorm's `VerifyStatement::describe`, and checks that each field's column is
there and of a type the field's kind reads, the test a row's decode applies.
A field that fails is a `DecodeError` naming the column. Nothing runs, so the
statement's parameters are neither bound nor checked:

```ts
import assert from "node:assert/strict";
import { column, DecodeError, model } from "@necessary/pgorm";

const PriceList = model("cake", {
  columns: { name: column("text"), priceInCents: column("i64", { name: "price_in_cents" }) },
});

await assert.rejects(
  pool.verify("SELECT name, price FROM cake WHERE bakery_id = $1", PriceList),
  (error) =>
    error instanceof DecodeError &&
    error.message === "`cake` reads column `price_in_cents`, which the statement does not return; it returns: name, price",
);
await assert.rejects(
  pool.verify("SELECT name, price AS price_in_cents FROM cake WHERE bakery_id = $1", PriceList),
  (error) =>
    error instanceof DecodeError &&
    error.message === "`cake` decodes column `price_in_cents` as `i64`, which cannot read PostgreSQL type `numeric`",
);

await pool.verify("SELECT name, (price * 100)::bigint AS price_in_cents FROM cake WHERE bakery_id = $1", PriceList);
```

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
compile step for a macro to run in, so without a check a program's SQL text
is checked by the server, when it runs:

```ts
import assert from "node:assert/strict";
import { DatabaseError } from "@necessary/pgorm";

await assert.rejects(
  pool.query("SELEC name FROM baker WHERE bakery_id = $1", [1]),
  (error) => error instanceof DatabaseError && error.sqlstate === "42601",
);
```

`sql(text)` is the check `sql!` makes, run when it is called: it holds the
text to PostgreSQL 18's grammar with the parser pgorm links, the code `sql!`
runs, and returns it unchanged. Called where a module defines its statements,
it checks each once, as the module loads, rather than at its first run. SQL
the grammar rejects is a `ConstructionError` that reprints the line with a
caret under the token the parser stopped at; a `;`-separated script is held to
the grammar statement by statement, for `batchExecute`:

```ts
import assert from "node:assert/strict";
import { ConstructionError, sql } from "@necessary/pgorm";

const BAKERS_AT = sql("SELECT name FROM baker WHERE bakery_id = $1 ORDER BY name");

const names = (await pool.query(BAKERS_AT, [1])).map((row) => row.name);
assert.deepEqual(names, ["Ada", "Bo"]);

assert.throws(
  () => sql("SELECT name FROM baker\nORDER name"),
  (error) =>
    error instanceof ConstructionError &&
    error.message === 'PostgreSQL rejected this SQL: syntax error at or near "name"\n\n  ORDER name\n        ^',
);
```

The check is of the grammar alone, as `sql!`'s is: a table or a column that
does not exist passes, and is the server's to refuse.

## PRQL

[PRQL](https://prql-lang.org) is a query language that reads as a pipeline of
steps, each taking the relation the one before it produced. Rust's `prql!`
macro compiles a PRQL literal to PostgreSQL SQL while the crate compiles,
checks it against the grammar, and arity-checks its `$N` placeholders against
its arguments. `prql(text, values)` makes the same checks when it is called,
through the same compiler: it holds the SQL to the grammar, and holds the
values to the placeholders, one value for each distinct placeholder in order.
It gives the pair `prql!` expands to, `{ sql, values }`, which runs as SQL
text and its array do:

```ts
import assert from "node:assert/strict";
import { Decimal, prql } from "@necessary/pgorm";

const under = new Decimal("4.00");
const { sql, values } = prql("from cake | filter price < $1 | sort name | select {name, price}", [under]);
assert.equal(sql, "SELECT name, price FROM cake WHERE price < $1 ORDER BY name");
assert.deepEqual(values, [under]);

const rows = await pool.query(sql, values);
assert.deepEqual(rows.map((row) => row.name), ["Carrot Cake", "Lemon Drizzle", "Vanilla Sponge"]);
```

`prql` refuses with a `ConstructionError` what `prql!` refuses at compile
time: PRQL the compiler rejects, `take $1` among it, SQL the grammar
rejects, a count of values that does not match the placeholders, and
placeholders numbered with a gap:

```ts
import assert from "node:assert/strict";
import { ConstructionError, prql } from "@necessary/pgorm";

// `$1` is used, and nothing binds it.
assert.throws(
  () => prql("from cake | filter price < $1"),
  (error) =>
    error instanceof ConstructionError &&
    error.message === "the query has 1 placeholder(s) but 0 argument(s) were given: nothing binds $1",
);
assert.throws(() => prql("from cake | filter id > $1 | filter id < $3", [1, 2]), ConstructionError);
```

Like `prql!`, it does not know the schema. For a query assembled from parts
at run time, such as a filter a user chose, [the pipeline](12-pipeline.md) is
the same compiler with a builder in front of it, which binds each value
through a binder of the step that takes it:

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
