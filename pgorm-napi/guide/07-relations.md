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
joins through it. The bakery declares the same path as `cakeBakers`,
`Cake.related(cakeBakerBaker, { via: cakeCakeBakers })`
([Entities](03-entities.md#relations)), and its `find` joins through the
junction, keying the cake by its primary key, as Rust's `find_related`
writes it:

```ts
import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";

// A cake's bakers, through the cake_baker junction.
const lemonDrizzle = {
  id: 2,
  name: "Lemon Drizzle",
  flavor: "lemon",
  price: new Decimal("3.20"),
  glutenFree: true,
  bakeryId: 1,
} as const;
const lemonDrizzlesBakers = cakeBakers.find(lemonDrizzle).orderBy(Baker.col("name").asc());
assert.equal(
  lemonDrizzlesBakers.inspect().sql,
  `SELECT "baker"."id", "baker"."name", "baker"."hired_on", "baker"."bakery_id" FROM "baker" INNER JOIN "cake_baker" ON "cake_baker"."baker_id" = "baker"."id" INNER JOIN "cake" ON "cake"."id" = "cake_baker"."cake_id" WHERE "cake"."id" = $1 ORDER BY "baker"."name" ASC`,
);
const names = (await lemonDrizzlesBakers.all(pool)).map((baker) => baker.name);
assert.deepEqual(names, ["Ada", "Bo"]);
```

A relation with an `onCondition` ([Entities](03-entities.md#relations)) is
read whole the same way: its `find` joins the model it starts at back, so the
condition can read both ends.

## Longer paths

A path of more than one relation is a `Linked` chain, which a model's
`linked(...relations)` makes, as a Rust `Linked` type lists its relations:
each starting where the one before it ends. `find(row)` reads along it from
one record, as Rust's `find_linked` does. Here a baker's customers are the
people who ordered a cake the baker bakes:

```ts
import assert from "node:assert/strict";

// From a baker to the customers who ordered what they bake.
const bakedFor = Baker.linked(bakerCakeBakers, cakeBakerCake, cakeLines, lineOrder, orderCustomer);

const cy = await Baker.findByKey({ id: 3 }).one(pool);
assert.deepEqual((await bakedFor.find(cy).all(pool)).map((customer) => customer.name), ["Lee"]);
```

Each hop joins its table under an alias of its own, `r0`, `r1` and so on,
walking back from the far end, so a chain may pass through one table twice.
A filter or an ordering on a hop's fields names the hop: `hop(i)` is the model
read at hop `i` under that alias, as Rust's `LinkedAlias::hop` names it, and
`lastHop()` the starting model's, as `last_hop_alias` names it.
`findLinked()` reads every row the chain reaches, from any starting row.
Rust's `hop` and `last_hop_alias` give the alias's name; a JavaScript hop is
the model under it, whose `col(field)` is qualified by it:

```ts
import assert from "node:assert/strict";

const bakes = Baker.linked(bakerCakeBakers, cakeBakerCake);
assert.equal(bakes.lastHop().alias, "r1");
assert.equal(
  bakes.find({ id: 1 }).inspect().sql,
  `SELECT "cake"."id", "cake"."name", "cake"."flavor", "cake"."price", "cake"."gluten_free", "cake"."bakery_id" FROM "cake" INNER JOIN "cake_baker" AS "r0" ON "r0"."cake_id" = "cake"."id" INNER JOIN "baker" AS "r1" ON "r1"."id" = "r0"."baker_id" WHERE "r1"."id" = $1`,
);
const adas = bakes.findLinked().where(bakes.lastHop().col("name").eq("Ada")).orderBy(Cake.col("id").asc());
assert.deepEqual((await adas.all(pool)).map((cake) => cake.name), ["Chocolate Fudge", "Lemon Drizzle"]);
```

A path a related path already describes needs no chain of its own: Rust's
`RelatedLink` is that chain, and a JavaScript related path has its own
`find`. A path can also be a chain of joins on a model query: `join` takes a
relation from the query's model or from any table already joined, so each
hop starts where the last one ended:

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

Each of these joins its table under the table's own name, so a filter on it
names the far end's model as usual. A chain that passes through one table
twice joins the second visit with `{ alias }`, its columns read through the
model under that name, `Model.as(alias).col(field)`, and goes on from it with
`relation.fromAlias(alias)`, as Rust's `RelationDef::from_alias` points a
relation at an aliased table.

## Loading related rows for many models

Reading each record's related rows with `find` runs one query per record.
`relation.load(db, rows)` reads them for a whole list in one query, and
resolves with one entry per input row, in the input's order:

- a relation to at most one row, a `belongsTo` or a `hasOne`, gives a record
  per row, or `null` where nothing matched or the row's key holds a `NULL`,
  as Rust's `load_one` does;
- a `hasMany` gives an array per row, as Rust's `load_many` does;
- a related path gives an array per row, as Rust's `load_many_via` does.

Each takes `{ query }`, a query of the related model, to filter and order
what is loaded, as Rust's loaders take a `Select`:

```ts
import assert from "node:assert/strict";

const bakeries = await Bakery.find().orderBy(Bakery.col("id").asc()).all(pool);

// Each bakery's gluten-free cakes, cheapest first.
const glutenFree = Cake.find().where(Cake.col("glutenFree").eq(true)).orderBy(Cake.col("price").asc());
const cakes = await bakeryCakes.load(pool, bakeries, { query: glutenFree });
assert.deepEqual(cakes.map((list) => list.map((cake) => cake.name)), [["Lemon Drizzle", "Chocolate Torte"], ["Vanilla Sponge"]]);

// Each of Sunrise's cakes, with its bakery and its bakers.
const sunriseCakes = cakes[0]!;
const theirBakery = await cakeBakery.load(pool, sunriseCakes);
assert.ok(theirBakery.every((bakery) => bakery?.name === "Sunrise"));

const bakers = await cakeBakers.load(pool, sunriseCakes, { query: Baker.find().orderBy(Baker.col("name").asc()) });
assert.deepEqual(bakers.map((list) => list.map((baker) => baker.name)), [["Ada", "Bo"], ["Bo"]]);
```

A loader reads the relation whole, its `onCondition` included: it joins the
model the relation starts at back under `pgorm_loader_src`, as Rust's does,
and an ordering on its query orders every record's list. Two things are
errors rather than silent answers, a `DecodeError` each: two different rows
for one key of a relation to one row, which contradicts it, and a related row
whose key matches in SQL but not as a JavaScript value. A row in `rows` needs
only the fields the relation reads, not a whole record.

## Reading rows with their related rows

The graph reads rows of one model, the *root*, with rows of related models
joined beside them, in one query. Start it with `graph()` where you would
write `find()`, and add each related model as a *slot*, through a relation
from the root or from a slot before it. A slot is required or optional:

| Method | Joins | Decodes as |
| --- | --- | --- |
| `relatedOne(path)` | a relation or a related path, `INNER JOIN` | the far end's record |
| `relatedMaybe(path)` | the same path, `LEFT JOIN` | the far end's record, or `null` |
| `joinOne(relation)` | one relation, `INNER JOIN` | the far end's record |
| `joinMaybe(relation)` | one relation, `LEFT JOIN` | the far end's record, or `null` |

Each row is a tuple of the root's record and one item per slot, in the order
the slots were added, and a graph with no slot decodes the root's record
alone. The TypeScript declarations type the tuple from the slots. Rust's
`related_one::<F>()` and `related_maybe::<F>()` follow the path `Related<F>`
describes; a JavaScript slot names its relation or its path.

```ts
import assert from "node:assert/strict";

const orders = await Order.graph()
  .relatedOne(orderCustomer)
  .relatedOne(orderBakery)
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
as a junction: a `LEFT JOIN` no record reads. `relatedMaybe` and `relatedOne`
add a related path's junction themselves, `relatedOne` by an `INNER JOIN` and
`relatedMaybe` by a `LEFT JOIN`; written by hand, the many-to-many read is a
`via` to the junction and a slot from it:

```ts
import assert from "node:assert/strict";

const withBakers = Cake.graph().via(cakeCakeBakers).joinMaybe(cakeBakerBaker);
assert.equal(Cake.graph().relatedMaybe(cakeBakers).inspect().sql, withBakers.inspect().sql);
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
`relation.fromAlias(alias)`, as Rust's `from_alias` does, makes the next
relation start from the table answering to it, a `via` hop included.
Otherwise a relation starts from the first table of its model the graph
reads, or from the decoded source `{ from }` names, `0` the root and `i` the
i-th slot. `graph.col(source, field)` and `Model.as(alias).col(field)` name an
aliased table's columns in a filter or an ordering.

Here each cake reads the other cakes one of its bakers bakes: from the cake
through `cake_baker` to the baker, and back through `cake_baker` a second
time, under an alias, to the other cakes:

```ts
import assert from "node:assert/strict";

const sibling = Cake.as("sibling");
const siblings = await Cake.graph()
  .via(cakeCakeBakers)
  .via(cakeBakerBaker)
  .via(bakerCakeBakers, { alias: "shared" })
  .joinMaybe(cakeBakerCake.fromAlias("shared"), { alias: "sibling" })
  .where(sibling.col("id").ne(Cake.col("id")))
  .orderBy(sibling.col("name").asc())
  .allGrouped(pool);

const lemonDrizzle = siblings.find(([cake]) => cake.id === 2)?.[1].map((other) => other.name);
assert.deepEqual(lemonDrizzle, ["Chocolate Fudge", "Chocolate Torte"]);
```

### Narrowing a join

A `where` lands in the query's `WHERE`, where it applies to the joined row as
a whole: under a `LEFT JOIN`, a root whose slot did not match has `NULL`s
there, fails the predicate, and drops out, as if the join were `INNER`. To
narrow which rows *match* instead, keeping the unmatched roots,
`joinMaybeFiltered(relation, condition)` adds the condition to the join's
`ON`, as Rust's `join_maybe_filtered` does. Its function receives the join's
two ends, each the model as the join names its table, so it qualifies its
fields through them:

```ts
import assert from "node:assert/strict";

// Every bakery, with its carrot cakes if it has any.
const carrot = await Bakery.graph()
  .joinMaybeFiltered(bakeryCakes, (_bakery, cake) => cake.col("flavor").eq("carrot"))
  .allGrouped(pool);
assert.deepEqual(carrot.map(([bakery, cakes]) => [bakery.name, cakes.length]), [["Sunrise", 0], ["Moonlight", 1]]);
```

### Limits, pages and cursors

`limit` and `offset` count the rows of the join, not the roots: a bakery with
three cakes spends three rows of a limit. A limited graph's `allGrouped`
could therefore end with only part of its last root's rows, and a count, a
page or a cursor sets the rows itself.

**Not available in JavaScript yet:** refusing those reads of a limited graph.
Rust's compiler refuses `all_grouped`, `paginate`, `count` and a cursor on a
graph that has a `limit`. JavaScript runs them: `allGrouped` reads the limited
rows, `count` counts every row as if there were no limit, and a page or a
cursor replaces the limit with its own.

```ts
import assert from "node:assert/strict";

// Sunrise has three cakes, and the limit cut its list short.
const cut = await Bakery.graph().joinMaybe(bakeryCakes).limit(2).allGrouped(pool);
assert.deepEqual(cut.map(([bakery, cakes]) => [bakery.name, cakes.length]), [["Sunrise", 2]]);
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
exactly one row. `cursorOn(source, ...fields)` orders by fields of slot
`source` instead, counting the first slot as `1`, as Rust's `cursor_by_on`
does, and breaks ties by that slot's primary key, then the root's and every
other slot's. [Pages, cursors and streams](06-pagination.md) covers both.

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

// By the bakery's name, its id and then the cake's breaking ties.
const byBakery = Cake.graph().joinOne(cakeBakery).cursorOn(1, "name");
assert.ok(byBakery.first(2).inspect().sql.endsWith(`ORDER BY "bakery"."name" ASC, "bakery"."id" ASC, "cake"."id" ASC LIMIT $1`));
const moonlight = await byBakery.first(2).all(pool);
assert.deepEqual(moonlight.map(([cake, bakery]) => [cake.name, bakery.name]), [["Carrot Cake", "Moonlight"], ["Vanilla Sponge", "Moonlight"]]);
```
