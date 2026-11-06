# Reading rows

Every read of a model starts from its [`find`](../lib/models.d.ts), which
builds a [`ModelQuery`](../lib/models.d.ts): a `SELECT` of every column of the
table, decoded into the model's records. The builder methods narrow it, order
it, choose fields or join other tables, and a terminal runs it. This chapter
reads the bakery's cakes, bakers and orders. It is the JavaScript dialect of
the [Rust guide's chapter](../../docs/guide/04-reading.md) of the same name.

## Finding rows

A model query has three terminals, each taking a pool, a connection or a
transaction:

| Terminal | Resolves with |
| --- | --- |
| `all(db)` | every row, as an array of records |
| `one(db)` | the one row; none, or more than one, is a `DecodeError` |
| `optional(db)` | the one row, or `null` when there is none; more than one is a `DecodeError` |

Here JavaScript differs from Rust. Rust's `one` and `one_opt` add `LIMIT 1`
and read the first row, `one` failing with `Error::RecordNotFound` when there
is none. A model query's `one` and `optional` add nothing to the statement:
they say how many rows it must return, and a different number is an error. A
query for the first of many says `limit(1)` itself.

[`findByKey`](../lib/models.d.ts) filters on the primary key, naming each of
its fields, so a composite key is an object of its parts rather than Rust's
tuple:

```ts
import assert from "node:assert/strict";
import { DecodeError } from "@necessary/pgorm";

const cakes = await Cake.find().all(pool);
assert.equal(cakes.length, 5);

const torte = await Cake.findByKey({ id: 5 }).one(pool);
assert.equal(torte.name, "Chocolate Torte");

await assert.rejects(Cake.findByKey({ id: 99 }).one(pool), DecodeError);
assert.equal(await Cake.findByKey({ id: 99 }).optional(pool), null);

// Bo bakes the Lemon Drizzle.
assert.notEqual(await CakeBaker.findByKey({ cakeId: 2, bakerId: 2 }).optional(pool), null);
```

Each record is checked against the model as it decodes: the result's columns
must be the ones selected, each of its declared kind, with no `NULL` in a
field that is not nullable and no label an enum field does not list. A
mismatch is a `DecodeError`, never a half-read record.

## Filtering

`where(predicate)` adds a condition to the `WHERE` clause, and several `where`
calls must all hold. A condition is usually built from a model's column,
[`Cake.col(field)`](../lib/models.d.ts), through the methods every expression
has:

| Methods | SQL |
| --- | --- |
| `eq`, `ne`, `gt`, `gte`, `lt`, `lte` | `=`, `<>`, `>`, `>=`, `<`, `<=` against a value, a column or an expression |
| `between`, `notBetween` | `BETWEEN a AND b`, with `{ symmetric: true }` for `BETWEEN SYMMETRIC` |
| `isIn`, `isNotIn` | `IN (..)` against a list of values, or against a `select` |
| `eq(any([..]))`, `ne(all([..]))` | `= ANY($1)`, `<> ALL($1)` against one array parameter |
| `isNull`, `isNotNull` | `IS NULL`, `IS NOT NULL` |
| `isDistinctFrom`, `isNotDistinctFrom` | `IS [NOT] DISTINCT FROM`, which compares `NULL` as a value |
| `like`, `notLike`, `ilike`, `notIlike` | `LIKE` and `ILIKE` against a pattern you write |
| `startsWith`, `endsWith`, `containsText` | whether the text begins with, ends with or holds other text, read as text: `%` and `_` match themselves |
| `contains`, `containedBy`, `overlaps`, ... | the range and array operators ([Values and types](13-values.md)) |

Each value is bound as a parameter. A value compared with an enum column is
cast to the enum's type, and a label the enum does not list is refused with a
`ConstructionError` as the comparison is built.

Two things differ from Rust here. Rust's `eq` and its kin take values only,
so that the enum cast is never lost, and comparing two columns is `eq_col` or
`eq_expr`. A JavaScript column's `eq` takes either: a value is converted and
cast through the column's declared kind, and a column or an expression is
compared as it is written. And `startsWith`, `endsWith` and `containsText`
compare text with PostgreSQL's text functions, `STARTS_WITH`, `right` and
`strpos`, where Rust's escape the text into a `LIKE` pattern; both mean the
same.

```ts
import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";

const chocolateUnderFive = Cake.find()
  .where(Cake.col("flavor").eq("chocolate"))
  .where(Cake.col("price").lt(new Decimal("5.00")));
assert.equal(
  chocolateUnderFive.inspect().sql,
  `SELECT "cake"."id", "cake"."name", "cake"."flavor", "cake"."price", "cake"."gluten_free", "cake"."bakery_id" FROM "cake" WHERE "cake"."flavor" = CAST($1::text AS public.flavor) AND "cake"."price" < $2`,
);
const found = await chocolateUnderFive.all(pool);
assert.deepEqual(found.map((cake) => cake.name), ["Chocolate Fudge"]);

const midPriced = await Cake.find()
  .where(Cake.col("price").between(new Decimal("3.00"), new Decimal("4.00")))
  .count(pool);
assert.equal(midPriced, 2);

const named = await Cake.find()
  .where(Cake.col("name").startsWith("Chocolate"))
  .where(Cake.col("id").isIn([1, 2, 3]))
  .all(pool);
assert.equal(named.length, 1);
```

Comparing with `null` is refused with a `ConstructionError`, because
`= NULL` is never true; test for it with `isNull()`:

```ts
import assert from "node:assert/strict";
import { ConstructionError } from "@necessary/pgorm";

// @ts-expect-error: a column is never compared with null.
assert.throws(() => Customer.col("notes").eq(null), ConstructionError);

const noNotes = await Customer.find().where(Customer.col("notes").isNull()).all(pool);
assert.deepEqual(noNotes.map((customer) => customer.name), ["Kim"]);
```

### Any and all

[`Condition.any`](../lib/expressions.d.ts) joins conditions with `OR`, and
[`Condition.all`](../lib/expressions.d.ts) with `AND`. Each takes its
conditions as arguments or through `add`, nests inside the other, and goes to
`where` like a single condition. `Condition.all()` of nothing is true, and
`Condition.any()` of nothing false.

`addOption` adds a condition that may be absent, as Rust's `add_option`
does: given `null` or `undefined` it leaves the condition as it is, which
suits a search form whose fields are optional:

```ts
import assert from "node:assert/strict";
import { Condition, Decimal } from "@necessary/pgorm";

const lemonOrGlutenFree = Cake.select("name").where(
  Condition.any(Cake.col("flavor").eq("lemon"), Cake.col("glutenFree").eq(true)),
);
assert.equal(
  lemonOrGlutenFree.inspect().sql,
  `SELECT "cake"."name" FROM "cake" WHERE "cake"."flavor" = CAST($1::text AS public.flavor) OR "cake"."gluten_free" = $2`,
);

// A search with an optional upper price: no condition at all when it is absent.
function search(maxPrice: Decimal | null) {
  return Cake.select("name").where(
    Condition.all(Cake.col("bakeryId").eq(1)).addOption(maxPrice === null ? null : Cake.col("price").lte(maxPrice)),
  );
}
assert.equal(search(null).inspect().sql, `SELECT "cake"."name" FROM "cake" WHERE "cake"."bakery_id" = $1`);
assert.equal(
  search(new Decimal("4.00")).inspect().sql,
  `SELECT "cake"."name" FROM "cake" WHERE "cake"."bakery_id" = $1 AND "cake"."price" <= $2`,
);
```

Every builder method returns a new query and leaves its receiver as it was,
so one base query can be extended in two directions.

## Ordering, limits and offsets

`orderBy(..)` orders by columns or expressions, each made with `asc()` or
`desc()`, in the order given; `{ nulls: "first" }` or `{ nulls: "last" }`
says where `NULL`s go. `limit` and `offset` cut the result, and `null`
removes either. `distinct()` and `distinctOn(...columns)` drop repeated rows.

```ts
import assert from "node:assert/strict";

const twoDearest = await Cake.find()
  .orderBy(Cake.col("price").desc(), Cake.col("name").asc())
  .limit(2)
  .all(pool);
assert.deepEqual(twoDearest.map((cake) => cake.name), ["Chocolate Torte", "Chocolate Fudge"]);

const third = await Cake.find().orderBy(Cake.col("price").desc()).offset(2).limit(1).one(pool);
assert.equal(third.name, "Carrot Cake");
```

The `limit(1)` is what makes the third cake one row: without it the query
returns three, and `one` refuses them.

`distinctOn` keeps the first row of each run of rows equal on its columns,
so the ordering decides which: here the dearest cake of each flavor. Its
columns are the queried model's or a joined table's, and the rows are still
the model's records. The statement builder's [`select(..)`](../lib/select.d.ts)
has the same `distinct()` and `distinctOn`:

```ts
import assert from "node:assert/strict";

const dearest = Cake.select("flavor", "name")
  .distinctOn(Cake.col("flavor"))
  .orderBy(Cake.col("flavor").asc(), Cake.col("price").desc());
assert.equal(
  dearest.inspect().sql,
  `SELECT DISTINCT ON ("cake"."flavor") "cake"."flavor", "cake"."name" FROM "cake" ORDER BY "cake"."flavor" ASC, "cake"."price" DESC`,
);
assert.deepEqual(
  (await dearest.all(pool)).map((cake) => cake.name),
  ["Chocolate Torte", "Vanilla Sponge", "Lemon Drizzle", "Carrot Cake"],
);
```

A query read in pages is better served by [Pages, cursors and
streams](06-pagination.md) than by `offset`.

## Choosing columns

`Cake.select(...fields)` reads only the fields named, each row a record of
those fields alone, decoded and checked as a whole record is. The
TypeScript declarations type such a row as the record narrowed to its fields.

```ts
import assert from "node:assert/strict";

const names = await Cake.select("name").orderBy(Cake.col("id").asc()).all(pool);
assert.deepEqual(names[0], { name: "Chocolate Fudge" });

const priced = await Cake.select("name", "price")
  .where(Cake.col("glutenFree").eq(true))
  .orderBy(Cake.col("price").asc())
  .all(pool);
assert.equal(priced[0]!.name, "Vanilla Sponge");
assert.equal(priced[0]!.price.toString(), "2.90");
```

A query also becomes a *projection* of items the program chooses, as Rust's
`select` and `select_only` make one. `selectOnly()` clears the fields, and
`column`, `columns`, `columnAs` and `exprAs` add items: a column of the
queried model or of a table the query joins, decoded as its field declares,
keyed by its field and projected under its column's name, or under the name
`columnAs` gives it; or an expression under a name, decoded as the `column()`
given beside it declares. A model query takes the item methods directly too,
its fields staying the first items. A projection reads rows once one of three
methods says what each row becomes, as in Rust:

- `intoTuple()` reads the items' values in order into an array.
- `intoValues(...names)` reads the named items' values, in the order named.
- `intoModel()` reads each row into an object keyed by item name, as a
  `FromQueryResult` struct reads by name, and `intoModel(make)` gives what
  `make` makes of that object.

Each gives a [`ModelSelector`](../lib/models.d.ts), Rust's `Selector`, whose
`all`, `one`, `optional`, `stream`, `count`, `cursor` and `paginate` read as
a model query's do, and whose rows the TypeScript declarations type from the
items. A model query takes `intoTuple`, `intoValues` and `intoModel`
directly, reading its fields.

Here JavaScript differs from Rust in where an expression's type is stated.
Rust states it in the type the rows decode into, `Vec<(Flavor, i64)>`;
JavaScript states it beside the expression, `column("i64")` for a `COUNT`,
which PostgreSQL types `bigint`. A row is held to its items as a record is to
its model: a column of another type, a `NULL` an item does not declare
`nullable`, or a label it does not list, is a `DecodeError`. A Rust column's
`count()` is [`call("count", column)`](../lib/expressions.d.ts) here, and
`"sum"`, `"min"`, `"max"` and `"avg"` are called the same way.

```ts
import assert from "node:assert/strict";
import { call, column } from "@necessary/pgorm";

const names: [string][] = await Cake.select("name").orderBy(Cake.col("id").asc()).intoTuple().all(pool);
assert.deepEqual(names[0], ["Chocolate Fudge"]);

const flavors = Cake.select("flavor")
  .exprAs(call("count", Cake.col("id")), "cakes", column("i64"))
  .groupBy(Cake.col("flavor"))
  .orderBy(Cake.col("flavor").asc());
assert.equal(
  flavors.inspect().sql,
  `SELECT "cake"."flavor", COUNT("cake"."id") AS "cakes" FROM "cake" GROUP BY "cake"."flavor" ORDER BY "cake"."flavor" ASC`,
);
const counted = await flavors.intoTuple().all(pool);
assert.deepEqual(counted[0], ["chocolate", 2n]);
const byName: [bigint, string][] = await flavors.intoValues("cakes", "flavor").all(pool);
assert.deepEqual(byName[0], [2n, "chocolate"]);
```

An enum sorts in the order of its labels, so chocolate comes first. A count is
an `int8`, a `bigint` in JavaScript. `selectOnly()` with `column` and
`columns` builds a list computed at run time, as Rust's does. The statement
builder's [`select(..)`](../lib/select.d.ts) takes a model's `table` and its
columns too, for what a projection does not cover; a pool, connection or
transaction runs it with `query`, `one` and `optional`, and each row is a
plain object keyed by column name, its values decoded by their PostgreSQL
types ([Raw SQL](10-raw-sql.md)).

## Decoding into your own types

Rust decodes a row into a struct of your own, by deriving `FromQueryResult`
for any query's rows or `PartialModel` for a struct that also says which
columns and expressions to select. JavaScript has no struct to derive onto,
and its counterparts are a projection's `intoModel` and a partial model:

- `intoModel()` reads each row by name into an object of the items, as a
  `FromQueryResult` struct reads, and `intoModel(make)` makes a value of your
  own of it.
- [`partialModel(model, fields)`](../lib/models.d.ts) declares what to select
  and how to read it. Each field is one of the model's columns,
  `Cake.col("glutenFree")`, which is Rust's `from_col`, a field named for its
  column being Rust's plain field; an expression,
  `fromExpr(expr, column(kind))`, Rust's `from_expr`, decoded as the
  `column()` declares; or nothing, `skip(value)`, Rust's `skip`. A model
  query's `intoPartialModel(shape)` selects what the shape declares, the
  query's conditions and ordering kept, and decodes each row into an object
  of the shape's fields, in declaration order.

Here JavaScript differs from Rust in what a skipped field holds. Rust takes
the type's `Default`; JavaScript has no default of a type to take, so `skip`
holds the value it is given in every row.

```ts
import assert from "node:assert/strict";
import { call, column, fromExpr, partialModel, skip } from "@necessary/pgorm";

// A line of the price list.
const tags = await Cake.select("name")
  .columnAs(Cake.col("price"), "euros")
  .where(Cake.col("bakeryId").eq(2))
  .orderBy(Cake.col("name").asc())
  .intoModel()
  .all(pool);
assert.equal(tags[0]!.name, "Carrot Cake");
assert.equal(tags[0]!.euros.toString(), "3.80");

/** A cake on the menu board, its name shouted. */
const MenuLine = partialModel(Cake, {
  shout: fromExpr(call("upper", Cake.col("name")), column("text")),
  freeFromGluten: Cake.col("glutenFree"),
  featured: skip(false),
});

const lemon = Cake.find().where(Cake.col("id").eq(2)).intoPartialModel(MenuLine);
assert.equal(
  lemon.inspect().sql,
  `SELECT UPPER("cake"."name") AS "shout", "cake"."gluten_free" AS "freeFromGluten" FROM "cake" WHERE "cake"."id" = $1`,
);
assert.deepEqual(await lemon.one(pool), { shout: "LEMON DRIZZLE", freeFromGluten: true, featured: false });
```

A shape made of some of a table's columns is also a model of its own: declare
a second model over the same table with the columns it reads, under the
field names it wants. Its `find` selects those columns and decodes each row
into its records, checked as any model's are, and it writes as well:

```ts
import assert from "node:assert/strict";
import { column, model } from "@necessary/pgorm";

/** A cake on the menu board: two of the cake table's columns. */
const MenuLine = model("cake", {
  columns: {
    id: column("i32", { primaryKey: true }),
    name: column("text"),
    freeFromGluten: column("bool", { name: "gluten_free" }),
  },
});

const lemon = await MenuLine.find().where(MenuLine.col("id").eq(2)).one(pool);
assert.deepEqual(lemon, { id: 2, name: "Lemon Drizzle", freeFromGluten: true });
```

## Joins

A join follows a relation the models declare. `join(relation)` joins the
table at a relation's far end, from the query's model or from a table it
joined already, as an `INNER JOIN`; `{ kind: "left" }`, `"right"` or
`"full"` makes it another, as Rust's `left_join_rel` and `right_join_rel`
do, and `{ alias }` joins the table under another name, for a table joined
twice. A JavaScript relation is a value with a direction, made from the model
it starts at ([Entities](03-entities.md)); `joinRev(relation)` follows one
from its far end, as Rust's `_rev` joins do, so from the bakery
`joinRev(cakeBakery)` joins the cakes along the cake's `belongsTo`.

A join adds rows to the query, not columns: the records stay the queried
model's until a projection chooses columns of the joined table too. It is how
a query filters on a related row. A row a right or full join reads with no
match holds `NULL` in the queried model's columns, which its records refuse
where a field is not nullable, so such a join usually goes with a
projection. To read each row together with its related record, use the graph
in [Relations](07-relations.md) instead.

```ts
import assert from "node:assert/strict";

const inOslo = Cake.find().join(cakeBakery).where(Bakery.col("city").eq("Oslo"));
assert.equal(
  inOslo.inspect().sql,
  `SELECT "cake"."id", "cake"."name", "cake"."flavor", "cake"."price", "cake"."gluten_free", "cake"."bakery_id" FROM "cake" INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" WHERE "bakery"."city" = $1`,
);
assert.equal((await inOslo.all(pool)).length, 3);

const menu = Cake.find()
  .join(cakeBakery)
  .selectOnly()
  .columns(Bakery.col("city"), Cake.col("name"))
  .where(Cake.col("glutenFree").eq(true))
  .orderBy(Bakery.col("city").asc(), Cake.col("name").asc());
assert.equal(
  menu.inspect().sql,
  `SELECT "bakery"."city", "cake"."name" FROM "cake" INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" WHERE "cake"."gluten_free" = $1 ORDER BY "bakery"."city" ASC, "cake"."name" ASC`,
);
const rows: [string, string][] = await menu.intoTuple().all(pool);
assert.deepEqual(rows, [
  ["Bergen", "Vanilla Sponge"],
  ["Oslo", "Chocolate Torte"],
  ["Oslo", "Lemon Drizzle"],
]);

// The bakeries that sell a chocolate cake, read from the bakery's side.
const chocolateShops = Bakery.find()
  .joinRev(cakeBakery)
  .where(Cake.col("name").startsWith("Chocolate"))
  .distinct();
assert.deepEqual((await chocolateShops.all(pool)).map((bakery) => bakery.name), ["Sunrise"]);
```

## Grouping and counting

`count(db)` on any model query counts its rows, limit and offset aside, and
resolves with a `number`. `groupBy` groups the rows and `having` filters the
groups, as Rust's do, and an aggregate, `call("count", ..)`, `"sum"`,
`"min"`, `"max"` or `"avg"`, summarizes each group as an item `exprAs` names
and declares ([Choosing columns](#choosing-columns)). A grouped model query's
records stay the model's, which suits a query grouped by its primary key.

```ts
import assert from "node:assert/strict";
import { call, column } from "@necessary/pgorm";

const cakes = call("count", Cake.col("id"));
const busy = Cake.find()
  .join(cakeBakery)
  .selectOnly()
  .column(Bakery.col("name"))
  .exprAs(cakes, "cakes", column("i64"))
  .groupBy(Bakery.col("name"))
  .having(cakes.gt(2));
assert.equal(
  busy.inspect().sql,
  `SELECT "bakery"."name", COUNT("cake"."id") AS "cakes" FROM "cake" INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" GROUP BY "bakery"."name" HAVING COUNT("cake"."id") > $1`,
);
assert.deepEqual(await busy.intoTuple().all(pool), [["Sunrise", 3n]]);

const glutenFree = await Cake.find().where(Cake.col("glutenFree").eq(true)).count(pool);
assert.equal(glutenFree, 3);
```

## Locking rows

Inside a [transaction](08-transactions.md), a `SELECT .. FOR UPDATE` reads
rows so that no other transaction can change them until this one ends, and
`FOR SHARE` reads them so that none can delete or update them. `NOWAIT` and
`SKIP LOCKED` say what to do about a row another transaction holds, the
second of which suits a queue of work that several workers take from.

A model query's `lock(strength, { wait, of })` is the statement builder's
[`lock`](../lib/select.d.ts): its strength is `"update"`, `"noKeyUpdate"`,
`"share"` or `"keyShare"`, and `{ wait }` takes `"nowait"` or
`"skipLocked"`. Here JavaScript differs from Rust only in spelling: Rust's
`lock_exclusive()` is `lock("update")`, its `lock_shared()` is
`lock("share")`, and its `lock_with_behavior` is `lock` with `wait`. The
locked rows still decode as the model's records.

```ts
import assert from "node:assert/strict";

const next = Order.select("id")
  .orderBy(Order.col("placedAt").asc())
  .limit(1)
  .lock("update", { wait: "skipLocked" });
assert.equal(
  next.inspect().sql,
  `SELECT "order"."id" FROM "order" ORDER BY "order"."placed_at" ASC LIMIT $1 FOR UPDATE SKIP LOCKED`,
);

const taken = await pool.transaction(async (tx) => next.one(tx));
assert.deepEqual(taken, { id: 1 });
```

## Beyond the model builders

A model query is a statement builder [`Select`](../lib/select.d.ts)
underneath, which its `statement` hands out for what the model builders do
not cover: common table expressions, window functions, lateral joins,
grouping sets ([The query builder](11-query-builder.md)). A model's columns
and table work in any statement. A statement built there, or SQL written by
hand ([Raw SQL](10-raw-sql.md)), runs through the same terminals and decodes
into plain rows, and [the pipeline](12-pipeline.md) composes queries in
another style.
