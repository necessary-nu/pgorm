# Relations

A relation says how two tables' rows belong together: a cake belongs to a
bakery, a bakery has many cakes, a cake has many bakers through the
`cake_baker` table. [Entities](03-entities.md) declares each one as a value,
made from the model it starts at with `belongsTo`, `hasOne` or `hasMany`, and
the bakery's are named for where they go: `cakeBakery` is a cake's bakery,
`bakeryCakes` a bakery's cakes. This chapter reads along them. It is the
JavaScript dialect of the [Rust guide's
chapter](../../docs/guide/06-relations.md) of the same name.

pgorm offers three ways to do it, one for each shape of question:

- one row's related rows: [`relation.find(row)`](../lib/models.d.ts), a query
  of its own;
- the related rows of a list of rows, in one query rather than one per row:
  [`relation.load(db, rows)`](../lib/models.d.ts);
- rows read together with their related rows, in one joined query: the
  graph, [`Model.graph()`](../lib/graphs.d.ts).

## The related rows of one model

`relation.find(row)` turns a relation into a query for one record's related
rows: the rows at the relation's far end whose key fields equal the record's.
The query is an ordinary [`ModelQuery`](../lib/models.d.ts) of the far end's
model, so it can be filtered and ordered before it runs. A record with a
`NULL` in its key fields has no related rows.

Rust reaches a related entity through `Related<E>`, which an entity
implements for each entity one relation reaches, and `find_related(E)` names
the entity alone. A JavaScript model has no such table of reachable models:
the program names the relation it means.

```ts
import assert from "node:assert/strict";

const sunrise = await Bakery.findByKey({ id: 1 }).one(pool);
const cakes = await bakeryCakes.find(sunrise).orderBy(Cake.col("price").asc()).all(pool);
assert.deepEqual(cakes.map((cake) => cake.name), ["Lemon Drizzle", "Chocolate Fudge", "Chocolate Torte"]);

const lemonDrizzle = cakes[0]!;
const bakery = await cakeBakery.find(lemonDrizzle).one(pool);
assert.equal(bakery.name, "Sunrise");
```

A many-to-many relation goes through a junction table. Rust's cake declares a
`Related<baker::Entity>` whose `via` names the junction, and `find_related`
joins through it. A JavaScript relation is one hop, so the many-to-many read
is a query of the far end joined to the junction: a cake's bakers are the
bakers whose `cake_baker` rows name the cake.

```ts
import assert from "node:assert/strict";

// A cake's bakers, through the cake_baker junction.
const lemonDrizzlesBakers = Baker.find()
  .join(bakerCakeBakers)
  .where(CakeBaker.col("cakeId").eq(2))
  .orderBy(Baker.col("name").asc());
assert.equal(
  lemonDrizzlesBakers.inspect().sql,
  `SELECT "baker"."id", "baker"."name", "baker"."hired_on", "baker"."bakery_id" FROM "baker" INNER JOIN "cake_baker" ON "baker"."id" = "cake_baker"."baker_id" WHERE "cake_baker"."cake_id" = $1 ORDER BY "baker"."name" ASC`,
);
const names = (await lemonDrizzlesBakers.all(pool)).map((baker) => baker.name);
assert.deepEqual(names, ["Ada", "Bo"]);
```

## Longer paths

Rust writes a path of more than one relation as a `Linked` type, listing its
relations in order, which `find_linked` reads along. In JavaScript a path is
a chain of joins: `join` takes a relation from the query's model or from any
table already joined, so each hop starts where the last one ended. Here a
baker's customers are the people who ordered a cake the baker bakes:

```ts
import assert from "node:assert/strict";

// From a customer, through what they ordered, to who bakes it.
const bakedForCy = Customer.find()
  .join(customerOrders)
  .join(orderLines)
  .join(lineCake)
  .join(cakeCakeBakers)
  .where(CakeBaker.col("bakerId").eq(3));
assert.equal(
  bakedForCy.inspect().sql,
  `SELECT "customer"."id", "customer"."name", "customer"."email", "customer"."notes" FROM "customer" INNER JOIN "order" ON "customer"."id" = "order"."customer_id" INNER JOIN "order_line" ON "order"."id" = "order_line"."order_id" INNER JOIN "cake" ON "order_line"."cake_id" = "cake"."id" INNER JOIN "cake_baker" ON "cake"."id" = "cake_baker"."cake_id" WHERE "cake_baker"."baker_id" = $1`,
);
assert.deepEqual((await bakedForCy.all(pool)).map((customer) => customer.name), ["Lee"]);
```

Each hop joins its table under the table's own name, so a filter on it names
the far end's model as usual. A chain that passes through one table twice
joins the second visit with `{ alias }`, and its columns are read through the
model under that name, `Model.as(alias).col(field)`. Rust's chain joins every
hop under an alias of its own, `r0`, `r1` and so on, which `LinkedAlias::hop`
mints.

## Loading related rows for many models

Reading each record's related rows with `find` runs one query per record.
`relation.load(db, rows)` reads them for a whole list in one query, and
resolves with one entry per input row, in the input's order:

- a relation to at most one row, a `belongsTo` or a `hasOne`, gives a record
  per row, or `null` where the row's key holds a `NULL`, as Rust's
  `load_one` does;
- a `hasMany` gives an array per row, as Rust's `load_many` does.

```ts
import assert from "node:assert/strict";

const bakeries = await Bakery.find().orderBy(Bakery.col("id").asc()).all(pool);

// Each bakery's cakes.
const cakes = await bakeryCakes.load(pool, bakeries);
assert.deepEqual(
  cakes.map((list) => list.map((cake) => cake.name).sort()),
  [["Chocolate Fudge", "Chocolate Torte", "Lemon Drizzle"], ["Carrot Cake", "Vanilla Sponge"]],
);

// Each of Sunrise's cakes, with its bakery.
const theirBakery = await cakeBakery.load(pool, cakes[0]!);
assert.ok(theirBakery.every((bakery) => bakery?.name === "Sunrise"));
```

Two things are errors rather than silent answers, a `DecodeError` each: a key
nothing at the far end matches, which a relation to one row says cannot
happen, and two rows for one key under a `hasOne`. A row in `rows` needs only
the fields the relation reads, not a whole record.

**Not available in JavaScript yet:** loading a filtered or ordered query of
the related model, and loading through a junction. Rust's loaders take the
related entity or a `Select` of it, so `load_many` can load each bakery's
gluten-free cakes cheapest first, and `load_many_via` loads each cake's
bakers through `cake_baker`. A JavaScript relation's `load` reads every
related row, in no promised order. Many-to-many rows are read with a graph's
`via` instead, [below](#junctions-and-other-paths).

## Reading rows with their related rows

The graph reads rows of one model, the *root*, with rows of related models
joined beside them, in one query. Start it with `graph()` where you would
write `find()`, and add each related model as a *slot*, through a relation
from the root or from a slot before it. A slot is required or optional:

| Method | Joins | Decodes as |
| --- | --- | --- |
| `joinOne(relation)` | the relation, `INNER JOIN` | the far end's record |
| `joinMaybe(relation)` | the relation, `LEFT JOIN` | the far end's record, or `null` |

Each row is a tuple of the root's record and one item per slot, in the order
the slots were added, and a graph with no slot decodes the root's record
alone. The TypeScript declarations type the tuple from the slots. Rust also
has `related_one::<F>()` and `related_maybe::<F>()`, which follow the path
`Related<F>` describes; a JavaScript slot always names its relation.

```ts
import assert from "node:assert/strict";

const orders = await Order.graph()
  .joinOne(orderCustomer)
  .joinOne(orderBakery)
  .orderBy(Order.col("placedAt").asc())
  .all(pool);

const summary = orders.map(([order, customer, bakery]) => [order.id, customer.name, bakery.name]);
assert.deepEqual(summary, [[1, "Kim", "Sunrise"], [2, "Lee", "Moonlight"]]);
```

A required slot leaves out a root row with nothing to join; an optional one
keeps it and decodes `null`. A graph with one optional slot can also be read
grouped: `allGrouped(db)` returns each root once, with its related records
collected in an array, an empty one when nothing matched. It orders by your
ordering first and the root's primary key after it, so your ordering decides
the order of both the roots and each list.

```ts
import assert from "node:assert/strict";

// A third bakery, which sells nothing yet.
await Bakery.insert({ name: "Noon", city: "Tromsø" }).execute(pool);

const rows = await Bakery.graph()
  .joinMaybe(bakeryCakes)
  .where(Bakery.col("name").ne("Sunrise"))
  .orderBy(Bakery.col("id").asc(), Cake.col("name").asc())
  .all(pool);
assert.deepEqual(
  rows.map(([bakery, cake]) => [bakery.name, cake?.name ?? null]),
  [["Moonlight", "Carrot Cake"], ["Moonlight", "Vanilla Sponge"], ["Noon", null]],
);

const grouped = await Bakery.graph().joinMaybe(bakeryCakes).orderBy(Cake.col("price").asc()).allGrouped(pool);
assert.deepEqual(
  grouped.map(([bakery, cakes]) => [bakery.name, cakes.length]),
  [["Moonlight", 2], ["Sunrise", 3], ["Noon", 0]],
);
```

The roots come in the order of their first row. Ordering by price put
Moonlight's Vanilla Sponge first, so Moonlight comes first; Noon has no row
with a price, and PostgreSQL sorts its `NULL` last.

A Rust graph has no `one`, because one root can make several rows. A
JavaScript graph's `one(db)` wants exactly one row and `optional(db)` at most
one, as a model query's do, any other count a `DecodeError`. `stream(db)`
reads the rows one at a time, as [Pages, cursors and
streams](06-pagination.md) describes for a model query.

### Junctions and other paths

`via(relation)` joins a table the path runs through without decoding it, such
as a junction: a `LEFT JOIN` no record reads. The many-to-many read is a
`via` to the junction and a slot from it:

```ts
import assert from "node:assert/strict";

const withBakers = Cake.graph().via(cakeCakeBakers).joinMaybe(cakeBakerBaker);
assert.equal(
  withBakers.inspect().sql,
  `SELECT "cake"."id" AS "s0_id", "cake"."name" AS "s0_name", "cake"."flavor" AS "s0_flavor", "cake"."price" AS "s0_price", "cake"."gluten_free" AS "s0_gluten_free", "cake"."bakery_id" AS "s0_bakery_id", "baker"."id" AS "s1_id", "baker"."name" AS "s1_name", "baker"."hired_on" AS "s1_hired_on", "baker"."bakery_id" AS "s1_bakery_id" FROM "cake" LEFT JOIN "cake_baker" ON "cake"."id" = "cake_baker"."cake_id" LEFT JOIN "baker" ON "cake_baker"."baker_id" = "baker"."id"`,
);

// Each of Sunrise's cakes, with its bakers.
const bakers = await withBakers
  .where(Cake.col("bakeryId").eq(1))
  .orderBy(Cake.col("id").asc(), Baker.col("name").asc())
  .allGrouped(pool);
assert.deepEqual(
  bakers.map(([cake, list]) => [cake.name, list.map((baker) => baker.name)]),
  [["Chocolate Fudge", ["Ada"]], ["Lemon Drizzle", ["Ada", "Bo"]], ["Chocolate Torte", ["Bo"]]],
);
```

The graph names its result columns `s0_` for the root and `s1_`, `s2_` and on
for each slot, as Rust's does; `via` hops are not decoded and take no prefix.

A path that passes through one table twice binds the second visit to an
alias: `{ alias }` on a slot or a `via` joins its table under that name, and
`{ from }` makes a slot's relation start at a decoded source other than the
first table of its model, `0` the root and `i` the i-th slot.
`graph.col(source, field)` and `Model.as(alias).col(field)` name an aliased
table's columns in a filter or an ordering.

**Not available in JavaScript yet:** a relation that starts from an aliased
`via` hop. Rust's `via_as` and `from_alias` let a path run on from a junction
it joined under an alias and decoded nowhere. A JavaScript `from` names a
decoded source only, so a path that needs to start from the second visit to a
junction decodes that visit as a slot of its own.

Here each cake reads the other cakes one of its bakers bakes: from the cake
through `cake_baker` to the baker, and back through `cake_baker` a second
time, under an alias, to the other cakes. The second `cake_baker` visit is a
slot, so the sibling's relation can start from it:

```ts
import assert from "node:assert/strict";

const sibling = Cake.as("sibling");
const siblings = await Cake.graph()
  .via(cakeCakeBakers)
  .joinOne(cakeBakerBaker)
  .joinMaybe(bakerCakeBakers, { alias: "shared", from: 1 })
  .joinMaybe(cakeBakerCake, { alias: "sibling", from: 2 })
  .where(sibling.col("id").ne(Cake.col("id")))
  .orderBy(sibling.col("name").asc())
  .all(pool);

const lemonDrizzle = siblings.filter(([cake]) => cake.id === 2).map(([, , , other]) => other?.name);
assert.deepEqual(lemonDrizzle, ["Chocolate Fudge", "Chocolate Torte"]);
```

### Narrowing a join

A `where` lands in the query's `WHERE`, where it applies to the joined row as
a whole: under a `LEFT JOIN`, a root whose slot did not match has `NULL`s
there, fails the predicate, and drops out, as if the join were `INNER`.

**Not available in JavaScript yet:** narrowing which rows a slot matches.
Rust's `join_maybe_filtered` adds a predicate to the join's `ON`, so every
bakery is read with its carrot cakes, if it has any, and the bakeries with
none keep an empty list. A JavaScript slot joins on its relation's fields
alone.

### Limits, pages and cursors

`limit` and `offset` count the rows of the join, not the roots: a bakery with
three cakes spends three rows of a limit. A limited graph therefore has no
`allGrouped`, whose last entry could hold only part of its root's rows, and no
`paginate`, `count` or `cursor`, which set the rows themselves. Rust's
compiler refuses them, and so does TypeScript's: `limit` and `offset` return a
graph whose third type parameter is `true`, which those methods do not take.
Run anyway, each throws a `ConstructionError`:

```ts
import assert from "node:assert/strict";
import { ConstructionError } from "@necessary/pgorm";

const limited = Bakery.graph().joinMaybe(bakeryCakes).limit(2);
// @ts-expect-error a limited graph has no allGrouped
await assert.rejects(limited.allGrouped(pool), ConstructionError);
// @ts-expect-error nor a count, which would ignore the limit
await assert.rejects(limited.count(pool), ConstructionError);
```

To read the first `n` roots with all of their related rows, limit the roots
and load the rest:

```ts
import assert from "node:assert/strict";

const first = await Bakery.find().orderBy(Bakery.col("id").asc()).limit(1).all(pool);
const cakes = await bakeryCakes.load(pool, first);
assert.deepEqual([first.length, cakes[0]!.length], [1, 3]);
```

An unlimited graph pages as a model query does, by rows of the join, through
`paginate` and `cursor`. `cursor(...fields)` orders by fields of the root and
breaks ties by the primary key of every source it decodes, so rows that share
an ordering value still page correctly; `afterWith` and `beforeWith` take that
whole ordering, the fields and then each source's key, and resume from
exactly one row. [Pages, cursors and streams](06-pagination.md) covers both.

```ts
import assert from "node:assert/strict";

// Every cake with its bakery, two rows at a time, cheapest first.
const cursor = Cake.graph().joinOne(cakeBakery).cursor("price");
const page = await cursor.first(2).all(pool);
assert.deepEqual(
  page.map(([cake, bakery]) => [cake.name, bakery.name]),
  [["Vanilla Sponge", "Moonlight"], ["Lemon Drizzle", "Sunrise"]],
);

const lastPrice = page[1]![0].price;
const next = await cursor.after(lastPrice).first(2).all(pool);
assert.deepEqual(next.map(([cake]) => cake.name), ["Carrot Cake", "Chocolate Fudge"]);
```

**Not available in JavaScript yet:** a graph cursor ordered by a slot's
columns. Rust's `cursor_by_on` orders by the columns of one of the graph's
related tables; a JavaScript graph's `cursor` takes the root's fields.
