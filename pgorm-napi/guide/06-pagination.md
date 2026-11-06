# Pages, cursors and streams

A query whose result is too long to read at once can be read a piece at a
time, three ways:

- *Pages* by number, with [`paginate`](../lib/graphs.d.ts): page 3 of 20, the
  shape of a list with numbered pages. Each page is a `LIMIT`/`OFFSET`, so a
  deep page costs the database every row before it.
- *Cursors*, with [`cursor`](../lib/graphs.d.ts): the rows after (or before)
  a given row, the shape of an endless feed or a "next" link. Each window is a
  `WHERE` on the ordering columns, so a deep window costs no more than the
  first, and rows written between two reads do not shift it.
- *Streams*, with [`stream`](../lib/index.d.ts): every row of one query,
  decoded as it is pulled from the server instead of collected into an array.

The examples page through the bakery's five cakes. This chapter is the
JavaScript dialect of the [Rust guide's
chapter](../../docs/guide/05-pagination.md) of the same name.

## Pages

`paginate(pageSize)` turns a model query into a
[`Paginator`](../lib/graphs.d.ts). Pages are numbered from zero.
`fetchPage(db, page)` reads one; `numItems(db)`, `numPages(db)` and
`numItemsAndPages(db)` count, each running a `COUNT(*)` over the query. Rust
takes the page size as a `NonZeroU64`, so a size of zero does not compile; a
JavaScript page size that is not a whole number of at least one is a
`ConstructionError` as `paginate` is called.

Order the query before paginating it. Without an `ORDER BY`, PostgreSQL may
return the rows in a different order for each page, and a row can then
appear on two pages or on none.

```ts
import assert from "node:assert/strict";
import { ConstructionError } from "@necessary/pgorm";

const pages = Cake.find().orderBy(Cake.col("name").asc()).paginate(2);

assert.deepEqual(await pages.numItemsAndPages(pool), { items: 5, pages: 3 });

const second = await pages.fetchPage(pool, 1);
assert.deepEqual(second.map((cake) => cake.name), ["Chocolate Torte", "Lemon Drizzle"]);
assert.deepEqual(await pages.fetchPage(pool, 3), []);

assert.throws(() => Cake.find().paginate(0), ConstructionError);
```

A Rust paginator also keeps a current page, which `fetch` reads, `advance`
moves on and `fetch_and_next` does both. A JavaScript paginator holds no
position. Its `pages(db)` is the loop over every page instead: an async
generator that reads each page in turn from the first and ends at the first
empty one, as `fetch_and_next` and `into_stream` do.

```ts
import assert from "node:assert/strict";

const sizes: number[] = [];
for await (const page of Cake.find().orderBy(Cake.col("id").asc()).paginate(2).pages(pool)) {
  sizes.push(page.length);
}
assert.deepEqual(sizes, [2, 2, 1]);
```

A projection of some fields paginates the same way, and so does a
[graph](07-relations.md) of related rows; a read of a [disjoint
enum](15-disjoint.md) has its own `paginate`. `count(db)` on the query is the
paginator's count without the pages, and `inspect(page)` and
`inspect("count")` show what a paginator sends.

## Cursors

`cursor(...fields)` orders a query by one field or several, the cursor's
*key*, and returns a [`Cursor`](../lib/graphs.d.ts). The key should identify a
row, so end it with the primary key: rows that tie on the key cannot be told
apart, and a window that ends inside a run of them cannot say where it ended.

`after(...values)` and `before(...values)` take a row's key values and keep
the rows past it in the cursor's order. `first(n)` takes that many rows from
the start of what is left and `last(n)` from its end; `asc()` and `desc()` set
the direction. `all(db)` runs it, and always returns the rows in the cursor's
order, `last` included.

A feed of the cakes by price, cheapest first, reads a window, then the window
after its last row:

```ts
import assert from "node:assert/strict";

const names = (cakes: { name: string }[]) => cakes.map((cake) => cake.name);

const feed = Cake.find().cursor("price", "id");
const first = await feed.first(2).all(pool);
assert.deepEqual(names(first), ["Vanilla Sponge", "Lemon Drizzle"]);

// The next window starts after the last row read.
const lastSeen = first.at(-1)!;
const next = await feed.after(lastSeen.price, lastSeen.id).first(2).all(pool);
assert.deepEqual(names(next), ["Carrot Cake", "Chocolate Fudge"]);

// The two dearest, from the far end, still in ascending order.
const dearest = await Cake.find().cursor("price", "id").last(2).all(pool);
assert.deepEqual(names(dearest), ["Chocolate Fudge", "Chocolate Torte"]);
```

Every cursor method returns a new cursor, where Rust's change the cursor they
are called on. So one cursor value, `feed` above, serves a whole feed, each
window built from it with its own boundary.

The key's length fixes the boundary's: a cursor over `price` and `id` takes
two values. In Rust a single value does not compile. In JavaScript the
TypeScript declarations type the boundary from the key, and a boundary of
the wrong length is a `ConstructionError` when it is given:

```ts
import assert from "node:assert/strict";
import { ConstructionError, Decimal } from "@necessary/pgorm";

const feed = Cake.find().cursor("price", "id");
// @ts-expect-error: the key is two fields, so the boundary is two values.
assert.throws(() => feed.after(new Decimal("3.20")), ConstructionError);
```

The cursor writes the ordering and the boundary itself. A boundary over
several fields becomes the comparison that reads them in order:

```ts
import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";

const feed = Cake.select("name").cursor("price", "id").after(new Decimal("3.20"), 2).first(2);
assert.equal(
  feed.inspect().sql,
  `SELECT "cake"."name" FROM "cake" WHERE ("cake"."price" = $1 AND "cake"."id" > $2) OR "cake"."price" > $3 ORDER BY "cake"."price" ASC, "cake"."id" ASC LIMIT $4`,
);
```

A cursor over some fields, as above, decodes each row into a record of those
fields, as the query it came from does; Rust's needs `into_model` or
`into_partial_model` to say what into.

### Ties and secondary keys

In Rust, `add_secondary_order_by` appends a column to a cursor's ordering
after its key, which is how a cursor over a join is made total. `after` and
`before` still take the key alone, and so resume past every row sharing the
boundary's key, while `after_with` and `before_with` take the whole ordering
and resume from exactly one row.

A JavaScript model query's cursor orders by its key and nothing else, so its
key is the whole ordering: a cursor by bakery that should be total names the
id in its key, and its boundary, the bakery and the id, resumes from exactly
one row, as Rust's `after_with` does.

```ts
import assert from "node:assert/strict";

// By bakery, then by id within each bakery.
const byBakery = Cake.find().cursor("bakeryId", "id");

const first = await byBakery.first(2).all(pool);
assert.deepEqual(first.map((cake) => cake.id), [1, 2]);

// Bakery 1 has a third cake, which comes next.
const rest = await byBakery.after(1, 2).first(2).all(pool);
assert.deepEqual(rest.map((cake) => cake.id), [5, 3]);
```

**Not available in JavaScript yet:** a secondary ordering on a model query's
cursor. Rust's `add_secondary_order_by` breaks ties by a column of the
queried table or a joined one, apart from the key, so that `after` can resume
past a whole run of one key while `after_with` resumes inside it. A
JavaScript model cursor's `afterWith` and `beforeWith` take what `after` and
`before` take.

A [graph](07-relations.md) has cursors too, `cursor` by its root's fields and
`cursorOn` by one of its slots', as Rust's `cursor_by` and `cursor_by_on`, and
adds every decoded source's primary key to the ordering itself; there
`afterWith` takes the whole ordering.

## Paginating raw SQL

A query written as SQL pages and counts too. `fromStatement(sql, params)`
holds a statement whose rows are plain objects, and a model's
`findByStatement(sql, params)` one whose rows are its records ([Raw
SQL](10-raw-sql.md)); each is a `SelectorRaw`, whose `paginate(pageSize)`
gives the `Paginator` a model query gives and whose `count(db)` counts the
rows. As Rust's `SelectorRaw::paginate`, it wraps the statement as a subquery
and pages over that, so the statement's own `ORDER BY` still decides the
pages. pgorm reads the statement with PostgreSQL's own parser to do so, and
refuses anything but a single `SELECT` (a `WITH .. SELECT`, a `VALUES` list
and a `UNION` count as one) with a `ConstructionError` as `paginate` is
called, or as `count` runs:

```ts
import assert from "node:assert/strict";
import { ConstructionError, fromStatement } from "@necessary/pgorm";

const pages = fromStatement("SELECT name, price FROM cake WHERE bakery_id = $1 ORDER BY price", [1]).paginate(2);
assert.deepEqual(await pages.numItemsAndPages(pool), { items: 3, pages: 2 });
const first = await pages.fetchPage(pool, 0);
assert.deepEqual(first.map((row) => [row.name, String(row.price)]), [["Lemon Drizzle", "3.20"], ["Chocolate Fudge", "4.50"]]);

const cakes = Cake.findByStatement("SELECT * FROM cake WHERE gluten_free ORDER BY id").paginate(2);
assert.deepEqual((await cakes.fetchPage(pool, 1)).map((cake) => cake.name), ["Chocolate Torte"]);
assert.equal(await Cake.findByStatement("SELECT * FROM cake").count(pool), 5);

assert.throws(() => fromStatement("DELETE FROM cake").paginate(2), ConstructionError);
```

## Streams

`stream(db)` runs a query and yields its rows as an async iterator, a
[`RowStream`](../lib/index.d.ts), each row decoded when it is pulled. It pulls
one row from the server per `next()`, so a large result never sits in memory
whole, and an unread result holds the server back. `stream` exists on a model
query and on a [graph](07-relations.md), and a pool's, a connection's or a
transaction's `stream(sql, params)` streams the rows of any statement, SQL
text or built, as plain objects.

```ts
import assert from "node:assert/strict";

const prices: string[] = [];
for await (const cake of Cake.find().orderBy(Cake.col("id").asc()).stream(pool)) {
  prices.push(cake.price.toString());
}
assert.deepEqual(prices, ["4.50", "3.20", "3.80", "2.90", "6.00"]);

const names: string[] = [];
for await (const row of Cake.select("name").where(Cake.col("bakeryId").eq(2)).orderBy(Cake.col("name").asc()).stream(pool)) {
  names.push(row.name);
}
assert.deepEqual(names, ["Carrot Cake", "Vanilla Sponge"]);

const bakers: unknown[] = [];
for await (const row of pool.stream("SELECT name FROM baker WHERE bakery_id = $1 ORDER BY name", [1])) {
  bakers.push(row.name);
}
assert.deepEqual(bakers, ["Ada", "Bo"]);
```

A stream on a pool takes a connection of its own; a stream on a connection
holds that connection, which refuses other work with a `LifecycleError` until
the stream ends, and a stream in a transaction holds the transaction the same
way. The connection or the transaction is freed at the last row. Here
JavaScript differs from Rust in two ways:

- **Leaving early discards the connection.** A `for await` loop that breaks
  off, `return()` or `close()` before the last row closes the stream, and the
  connection it read from is discarded rather than returned, because rows
  were left unread on it; a transaction it read in ends with its connection,
  nothing committed. A Rust stream dropped early leaves its connection
  usable, which reads and discards the rest before its next statement.
- **A row that fails to decode ends the stream.** It rejects the pull it
  arrives on with a `DecodeError`, and a `for await` loop then throws. A Rust
  stream yields the failure as one `Err` and goes on to the rows after it.

```ts
import assert from "node:assert/strict";

const stream = Cake.find().orderBy(Cake.col("id").asc()).stream(pool);
const { value: first } = await stream.next();
assert.equal(first?.name, "Chocolate Fudge");

// Four rows were left unread, so closing discards the stream's connection.
await stream.close();
assert.equal(stream.closed, true);
```

A transaction streams as Rust's `stream` and `query_raw` read inside one. A
model query's, a graph's and a disjoint read's `stream` take the transaction
as they take a connection, and so does a transaction's own `stream(sql,
params)`. While the stream is open the transaction refuses other statements
with a `LifecycleError`; once its last row is read, the transaction runs
statements again and commits as its callback ends:

```ts
import assert from "node:assert/strict";
import { LifecycleError } from "@necessary/pgorm";

const names = await pool.transaction(async (tx) => {
  await tx.execute("UPDATE cake SET price = price + 1 WHERE bakery_id = 2");
  const names: string[] = [];
  for await (const cake of Cake.find().where(Cake.col("bakeryId").eq(2)).orderBy(Cake.col("name").asc()).stream(tx)) {
    names.push(`${cake.name} ${cake.price}`);
    await assert.rejects(tx.one("SELECT 1 AS one"), LifecycleError);
  }
  return names;
});
assert.deepEqual(names, ["Carrot Cake 4.80", "Vanilla Sponge 3.90"]);
```
