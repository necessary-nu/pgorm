# COPY

`COPY` moves rows between a table and the client in bulk. It is the fastest
way to load many rows into PostgreSQL, or to read many out: the rows stream
over the connection as one data flow, with no statement per row. A pool, a
connection and a transaction (and so a savepoint) each run one, through
`copyIn` and `copyOut`. This chapter is the JavaScript dialect of the [Rust
guide's chapter](../../docs/guide/16-copy.md) of the same name.

A COPY is described by a statement from the query builder:
[`copyFrom(table, format)`](../lib/copy.d.ts) builds `COPY .. FROM STDIN`,
rows sent into a table, and [`copyTo(source, format)`](../lib/copy.d.ts)
builds `COPY .. TO STDOUT`, rows read out of a table or a query. Each names
its format, one of PostgreSQL's three:

| Format | Named | The data |
| --- | --- | --- |
| text | `"text"` | tab-separated lines, `\N` for NULL |
| CSV | `"csv"` | comma-separated values with quoting |
| binary | `"binary"` | each field in its type's binary form |

The format is the statement's type, `CopyFrom<"csv">` say, so the options a
format takes are the methods of its type and only of its type: a CSV-only
option such as `quote` is no method of a text COPY, and a binary COPY has no
delimiter, header or NULL string to set. Rust makes the format a type
parameter in the same way, and refuses such a call when the program compiles.
TypeScript refuses it when `deno check` or the editor runs, and the call made
anyway, from JavaScript or past the types, throws a `ConstructionError`:

```ts
import assert from "node:assert/strict";
import { ConstructionError, copyFrom, copyTo, type CopyFrom } from "@necessary/pgorm";

const text: CopyFrom<"text"> = copyFrom("cake", "text");
assert.throws(() => (text as CopyFrom<any>).quote("'"), ConstructionError);
assert.throws(() => (copyTo("cake", "binary") as any).header(), ConstructionError);
```

A COPY takes no bound parameters, so a statement has one rendering:
`inspect()` gives its SQL, with any value in it written as an escaped
literal, and no values. The terminals that run other statements, `execute`,
`query` and the rest, refuse a COPY with a `TypeError`, since it carries data
only `copyIn` and `copyOut` move. SQL text holding a `COPY` given to them is
refused too, with a `ConstructionError` saying to use `copyIn` or `copyOut`,
before anything is sent, as Rust refuses it with `Error::CopyStatement`:

```ts
import assert from "node:assert/strict";
import { ConstructionError } from "@necessary/pgorm";

await assert.rejects(pool.query("COPY cake TO STDOUT"), ConstructionError);
```

## Loading rows

`copyIn(statement, source)` runs a `COPY .. FROM STDIN` and resolves with the
number of rows it inserted. The source is what you send: the format's own
encoding of the rows, split anywhere, since a chunk need not end at a row. It
is a `Uint8Array`, or an iterable or async iterable of them, which covers a
generator, a web `ReadableStream` and a Node.js stream of buffers. The source
is pulled only as the COPY takes the chunk before, so a large source streams
rather than piling up in memory. Rust's `copy_in` hands back a sink to send
into instead, and its `finish` returns the count.

The bakery's new cakes arrive as tab-separated text. A column list names the
columns each line holds; the others take their defaults, so the database
assigns each cake its id:

```ts
import assert from "node:assert/strict";
import { copyFrom } from "@necessary/pgorm";

const copy = copyFrom("cake", "text").columns(["name", "flavor", "price", "gluten_free", "bakery_id"]);
assert.equal(
  copy.inspect().sql,
  `COPY "cake" ("name", "flavor", "price", "gluten_free", "bakery_id") FROM STDIN WITH (FORMAT text)`,
);

const lines = ["Lemon Tart\tlemon\t4.10\tf\t1\n", "Carrot Muffin\tcarrot\t2.20\tt\t2\n"];
const encoder = new TextEncoder();
const written = await pool.copyIn(copy, lines.map((line) => encoder.encode(line)));
assert.equal(written, 2);

const cakes = await Cake.find().count(pool);
assert.equal(cakes, 7);
```

A COPY in progress holds what it runs on. A statement sent while it is open
would wait behind data not yet sent, and so would the `COMMIT` of a
transaction the COPY runs in. A pool's COPY takes a connection of its own; a
connection or a transaction refuses any other work until the COPY ends,
rejecting it with a `LifecycleError` rather than letting it hang. Rust's
`copy_in` borrows the connection exclusively for as long as the sink lives,
which makes the same mistake a compile error:

```ts
import assert from "node:assert/strict";
import { copyFrom, LifecycleError } from "@necessary/pgorm";

await using db = await pool.acquire();
const copy = copyFrom("bakery", "text").columns(["name", "city"]);

const { promise: sent, resolve: send } = Promise.withResolvers<void>();
async function* bakeries() {
  await sent;
  yield new TextEncoder().encode("Noon\tStavanger\n");
}
const copying = db.copyIn(copy, bakeries());

// The COPY still holds the connection.
await assert.rejects(db.one("SELECT count(*) AS bakeries FROM bakery"), LifecycleError);

send();
assert.equal(await copying, 1);
const { bakeries: count } = await db.one("SELECT count(*) AS bakeries FROM bakery");
assert.equal(count, 3n);
```

A row the server refuses, such as one that breaks a constraint or a field
that does not parse, is not reported while the data is still being sent, since
the protocol carries no error back to a client that is sending. `copyIn`
rejects with its `DatabaseError` once the data is sent, and the COPY inserts
nothing. A source that throws, or yields anything but a `Uint8Array`,
abandons the COPY: nothing is written, and the promise rejects with that
error. Inside a transaction either one aborts the transaction, as any failed
statement does.

```ts
import assert from "node:assert/strict";
import { copyFrom, DatabaseError } from "@necessary/pgorm";

const copy = copyFrom("bakery", "csv").columns(["name", "city"]);
const encoder = new TextEncoder();

// The second line has a third field the column list does not hold.
const refused = pool.copyIn(copy, encoder.encode("Noon,Stavanger\nDusk,Tromsø,extra\n"));
await assert.rejects(refused, DatabaseError);

// A source that fails part way writes nothing either.
async function* failing() {
  yield encoder.encode("Noon,Stavanger\n");
  throw new Error("the upload was cut off");
}
await assert.rejects(pool.copyIn(copy, failing()), /the upload was cut off/);

assert.equal(await Bakery.find().count(pool), 2);
```

## Reading rows

`copyOut(statement)` runs a `COPY .. TO STDOUT` and resolves, once the server
accepts it, with a [`CopyOutStream`](../lib/copy.d.ts): an async iterator of
`Uint8Array` chunks in the statement's format. The server sends rows as the
stream is read, so a slow reader holds the server back rather than filling
memory. A chunk is a piece of the byte stream, not a row.

A COPY TO reads a whole table, or the rows of a `select`. A `select` is also
how to read some of a table's columns or rows. Here the Sunrise bakery's price
list is written as CSV with a header line:

```ts
import assert from "node:assert/strict";
import { copyTo, select, Table } from "@necessary/pgorm";

const cake = new Table("cake");
const priceList = select(cake.col("name"), cake.col("price"))
  .from(cake)
  .where(cake.col("bakery_id").eq(1))
  .orderBy(cake.col("name").asc());
const copy = copyTo(priceList, "csv").header();
assert.equal(
  copy.inspect().sql,
  `COPY (SELECT "cake"."name", "cake"."price" FROM "cake" WHERE "cake"."bakery_id" = 1 ORDER BY "cake"."name" ASC) TO STDOUT WITH (FORMAT csv, HEADER true)`,
);

const decoder = new TextDecoder();
let csv = "";
for await (const chunk of await pool.copyOut(copy)) csv += decoder.decode(chunk, { stream: true });
csv += decoder.decode();
assert.equal(csv, "name,price\nChocolate Fudge,4.50\nChocolate Torte,6.00\nLemon Drizzle,3.20\n");
```

A model's query is a source too, through its `statement`, so
`copyTo(Cake.find().where(..).statement, "csv")` copies the rows the query
reads.

A write is a source when it has a `RETURNING` list: COPY TO copies the rows it
returns. A write without one returns nothing to copy, and `copyTo` refuses it
with a `ConstructionError` as it is called, where Rust converts it with
`CopySource::try_from`, which refuses it there:

```ts
import assert from "node:assert/strict";
import { col, ConstructionError, copyTo, Table, update } from "@necessary/pgorm";

const cake = new Table("cake");
const raise = update(cake).set("price", col("price").add(1)).where(col("bakery_id").eq(2));
assert.throws(() => copyTo(raise, "csv"), ConstructionError);

const raised = copyTo(raise.returning([col("name"), col("price")]), "csv");
const decoder = new TextDecoder();
let csv = "";
for await (const chunk of await pool.copyOut(raised)) csv += decoder.decode(chunk, { stream: true });
assert.deepEqual(csv.trim().split("\n").sort(), ["Carrot Cake,4.80", "Vanilla Sponge,3.90"]);
```

A `select` copies as it would run, a tie limit (`limitWithTies`) and a
`window` clause included. One PostgreSQL would refuse, with a tie limit but
no `orderBy` to measure its ties on, or a `window` clause that names a window
twice or refers to one it does not define, is refused by `copyTo` with a
`ConstructionError` as it is called, where Rust's `copy_out` refuses it
before it sends anything. Here the cheapest of Sunrise's cakes, and any that
tie with it, are copied:

```ts
import assert from "node:assert/strict";
import { ConstructionError, copyTo, select, Table } from "@necessary/pgorm";

const cake = new Table("cake");
const sunrise = select(cake.col("name")).from(cake).where(cake.col("bakery_id").eq(1));
assert.throws(() => copyTo(sunrise.limitWithTies(1), "csv"), ConstructionError);

const cheapest = copyTo(sunrise.orderBy(cake.col("price").asc()).limitWithTies(1), "csv");
const decoder = new TextDecoder();
let csv = "";
for await (const chunk of await pool.copyOut(cheapest)) csv += decoder.decode(chunk, { stream: true });
assert.equal(csv, "Lemon Drizzle\n");
```

Leaving a `CopyOutStream` before its end, with `close()`, `return()` or a
`for await` loop that breaks, discards the connection it ran on, since data
was left unread on it; in a transaction that ends the transaction. Here
JavaScript differs from Rust, whose connection reads and throws away the rest
of the data before its next statement runs. Read a COPY to its end, or narrow
its `select` to the rows wanted.

## The text and CSV options

The two text formats share most of their options, and CSV adds a few of its
own. On COPY FROM:

| Option | Formats | Does |
| --- | --- | --- |
| `delimiter(c)` | text, CSV | separates the fields |
| `nullString(s)` | text, CSV | the field that means NULL |
| `defaultString(s)` | text, CSV | the field that means the column's default |
| `header()`, `headerMatch()` | text, CSV | skips a header line; `headerMatch` also checks it names the columns |
| `encoding(e)` | text, CSV | the data's character encoding |
| `onErrorIgnore(options)` | text, CSV | skips a row whose field does not convert, rather than failing the COPY |
| `quote(c)`, `escape(c)` | CSV | the quoting and escape characters |
| `forceNull(columns)`, `forceNotNull(columns)` | CSV | for the columns given, reads a field matching the NULL string as NULL even when quoted, or never as NULL; `forceNullAll()` and `forceNotNullAll()` for every column |
| `freeze()` | all | writes the rows frozen, into a table created or emptied in the same transaction |
| `where(predicate)` | all | inserts only the rows the condition holds for |

On COPY TO the text formats take `delimiter`, `nullString`, `header` and
`encoding`, and CSV adds `quote`, `escape` and `forceQuote(columns)`, which
quotes every non-NULL field of the columns given (`forceQuoteAll()` for every
column). Each option's name is Rust's in camelCase.

`onErrorIgnore` takes the options Rust's `CopyIgnore` holds: `rejectLimit`
fails the COPY after all once more than that many rows have been skipped, and
`logVerbosity` is how much the server reports about them. Here a price list
from a supplier has one line whose price is not a number, and the COPY loads
the others:

```ts
import assert from "node:assert/strict";
import { copyFrom } from "@necessary/pgorm";

const copy = copyFrom("cake", "csv")
  .columns(["name", "flavor", "price", "gluten_free", "bakery_id"])
  .header()
  .quote("'")
  .onErrorIgnore();
assert.equal(
  copy.inspect().sql,
  `COPY "cake" ("name", "flavor", "price", "gluten_free", "bakery_id") FROM STDIN WITH (FORMAT csv, HEADER true, QUOTE E'\\'', ON_ERROR ignore)`,
);

const data = "name,flavor,price,gluten_free,bakery_id\n" +
  "'Vanilla Slice, iced',vanilla,3.40,false,2\n" +
  "Lemon Bun,lemon,ask,false,1\n";
// The second line's price did not convert, so it was skipped.
assert.equal(await pool.copyIn(copy, new TextEncoder().encode(data)), 1);

const slice = await Cake.find().where(Cake.col("name").eq("Vanilla Slice, iced")).one(pool);
assert.equal(slice.price.toString(), "3.40");
```

## Rows of values

The binary format sends each field in its type's binary form, the form a bound
parameter travels in, so a binary COPY can carry rows of values with no text
to write or parse. The format itself names no column types, so pgorm asks the
server: before the COPY starts it prepares a `SELECT` of the COPY's columns,
and binds each value against the type the server describes, as a parameter is
bound.

`copyInRows(statement, rows)` and `copyOutRows(statement)` take a binary
statement and run it that way:

- `rows` is an iterable or async iterable of arrays, each holding one value
  for every column, in order, each read as a statement's parameter is
  ([Values and types](13-values.md)). A row of another width, or a value its
  column's type does not take, is a `ConstructionError` that abandons the
  COPY, so nothing is written. Here JavaScript differs from Rust, whose
  `CopyRowSink` refuses that one row with `Error::Type` and goes on taking
  rows.
- `copyOutRows` resolves with a [`CopyRowStream`](../lib/copy.d.ts) of
  arrays, each field decoded as a result column of its type is, and its
  `columns` naming the columns each row holds. A field is typed as any value
  a result may hold, so TypeScript asks for each one's type before a row read
  out is written back in, as below.

Here every cake at Sunrise is copied out and copied back in as a new cake at
Moonlight. The COPY FROM lists every column but the id, so the database
assigns new ids:

```ts
import assert from "node:assert/strict";
import { copyFrom, copyTo, type Decimal } from "@necessary/pgorm";

const sunrise = copyTo(
  Cake.select("name", "flavor", "price", "glutenFree").where(Cake.col("bakeryId").eq(1)).statement,
  "binary",
);
const out = await pool.copyOutRows(sunrise);
assert.deepEqual(out.columns, ["name", "flavor", "price", "gluten_free"]);

const into = copyFrom("cake", "binary").columns(["name", "flavor", "price", "gluten_free", "bakery_id"]);
async function* atMoonlight() {
  for await (const [name, flavor, price, glutenFree] of out) {
    yield [name as string, flavor as string, price as Decimal, glutenFree as boolean, 2];
  }
}
// The binary COPY TO reads on a connection of its own, while the COPY FROM writes on another.
assert.equal(await pool.copyInRows(into, atMoonlight()), 3);

assert.equal(await Cake.find().where(Cake.col("bakeryId").eq(2)).count(pool), 5);
```

Each field reads as its column's type reads as a result column: the text
types and an enum's label as a `string`, an `int8` as a `bigint`, a `numeric`
as a `Decimal`, an array as an array. A `numeric` field reads exactly, its
scale included, and one a `Decimal` cannot hold exactly is a `DecodeError`
rather than rounded. A row of the wrong width is refused before any of the
COPY is written:

```ts
import assert from "node:assert/strict";
import { ConstructionError, copyFrom, copyTo, Decimal } from "@necessary/pgorm";

const bakeries = copyFrom("bakery", "binary").columns(["name", "city"]);
await assert.rejects(pool.copyInRows(bakeries, [["Daybreak", "Trondheim"], ["Dusk"]]), ConstructionError);
assert.equal(await pool.copyInRows(bakeries, [["Daybreak", "Trondheim"]]), 1);

const rows = [];
for await (const row of await pool.copyOutRows(copyTo("bakery", "binary"))) rows.push(row);
assert.deepEqual(rows[2], [3, "Daybreak", "Trondheim"]);

const prices = [];
const lines = copyTo(OrderLine.select("price").where(OrderLine.col("orderId").eq(1)).statement, "binary");
for await (const [price] of await pool.copyOutRows(lines)) prices.push(String(price as Decimal));
assert.deepEqual(prices.sort(), ["3.20", "4.50"]);
```

**Not available in JavaScript yet:** Rust describes a COPY's columns without
running it, `copy_in_columns` and `copy_out_columns`, each column a
`CopyColumn` with its name and its type, and a `CopyRowSink` takes each row
one call at a time, `send`, or a model whose fields name the COPY's columns,
`send_model`, while a `CopyRowStream` reads each row into an entity's model
with `models`. JavaScript's rows are arrays alone: it has no description of a
COPY's column types, no records in or out of a COPY, and a `CopyRowStream`'s
`columns` gives the names only.

## Encoding rows yourself

**Not available in JavaScript yet:** Rust's `CopyRowEncoder` and
`CopyRowDecoder` are the binary format's two sides over a list of columns, for
code that moves the bytes itself, such as writing a COPY file to disk or
reading one produced elsewhere: the encoder writes the header, each row of
values and the trailer into a buffer, and the decoder takes the data in
chunks split anywhere and yields each row once it has arrived. JavaScript has
neither. A binary COPY's bytes can still be moved as they are, `copyOut` to a
file and `copyIn` from it, since both directions speak the same format:

```ts
import assert from "node:assert/strict";
import { copyFrom, copyTo } from "@necessary/pgorm";

const chunks: Uint8Array[] = [];
for await (const chunk of await pool.copyOut(copyTo(Bakery.select("name", "city").statement, "binary"))) {
  chunks.push(chunk);
}
const written = await pool.copyIn(copyFrom("bakery", "binary").columns(["name", "city"]), chunks);
assert.equal(written, 2);
assert.equal(await Bakery.find().count(pool), 4);
```

## When the server refuses a COPY

A COPY the server refuses at its start, such as one naming a table or column
that does not exist, rejects `copyIn` or `copyOut` with its `DatabaseError`.
The two directions leave the connection differently:

- A refused COPY TO leaves the connection as it was.
- A refused COPY FROM closes its connection, and with it any transaction open
  on it, which ends without committing. The driver has by then sent a message
  whose answer nothing is waiting for, and that answer would reach the next
  request on the connection, leaving every later answer one request out of
  step. Closing is the only safe way out. A pool's COPY ran on a connection of
  its own, and the pool lends a fresh one next time; a connection the caller
  holds refuses every later call with a `LifecycleError`, as closed.

A generated column takes no data, and PostgreSQL refuses a column list that
names one, at the COPY's start. List only the columns the data supplies, or
give no list, which leaves the generated columns out and lets the server
compute them. pgorm does not check the list first, since that would cost a
catalogue read before every COPY.

```ts
import assert from "node:assert/strict";
import { copyFrom, DatabaseError, LifecycleError } from "@necessary/pgorm";

await using db = await pool.acquire();
await assert.rejects(db.copyIn(copyFrom("pastry", "text"), new Uint8Array()), DatabaseError);

// The connection is closed now.
await assert.rejects(db.one("SELECT 1 AS one"), LifecycleError);

// The pool does not lend it again.
const { one } = await pool.one("SELECT 1 AS one");
assert.equal(one, 1);
```
