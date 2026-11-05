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

Rust's `add_option` adds a condition that may be absent. A JavaScript query
is an immutable value, so a condition that may be absent is added where it is
present, the query reassigned:

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
  let query = Cake.select("name").where(Cake.col("bakeryId").eq(1));
  if (maxPrice !== null) query = query.where(Cake.col("price").lte(maxPrice));
  return query;
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
removes either.

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

**Not available in JavaScript yet:** `distinct` and `distinct_on` on a model
query. Rust's `Select` drops repeated rows and still decodes into its models.
In JavaScript, the statement builder's `distinct()` does it over the query's
`statement`, whose rows are plain objects rather than checked records, as
[Joins](#joins) shows.

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

Rust's `select` also takes a column of another table or an expression, and
its `column_as` and `expr_as` name an item, with the rows then decoded by
`into_tuple` or `into_model`. A model query's projection is its own fields
only. A query whose rows hold anything else is the statement builder's
[`select(..)`](../lib/select.d.ts), which takes a model's columns, the
functions [`call`](../lib/expressions.d.ts) makes, and an `as(name)` on any
item. A pool, connection or transaction runs it with `query`, `one` and
`optional`, and each row is a plain object keyed by column name, its values
decoded by their PostgreSQL types ([Raw SQL](10-raw-sql.md)):

```ts
import assert from "node:assert/strict";
import { call, select } from "@necessary/pgorm";

const flavors = select(Cake.col("flavor"), call("count", Cake.col("id")).as("cakes"))
  .from(Cake.table)
  .groupBy(Cake.col("flavor"))
  .orderBy(Cake.col("flavor").asc());
assert.equal(
  flavors.inspect().sql,
  `SELECT "cake"."flavor", COUNT("cake"."id") AS "cakes" FROM "cake" GROUP BY "cake"."flavor" ORDER BY "cake"."flavor" ASC`,
);

const counted = await pool.query(flavors);
assert.deepEqual(counted[0], { flavor: "chocolate", cakes: 2n });
```

An enum sorts in the order of its labels, so chocolate comes first. A count is
an `int8`, a `bigint` in JavaScript. `Cake.table` is the model's table as the
statement builder names it.

## Decoding into your own types

Rust decodes a row into a struct of your own, by deriving `FromQueryResult`
for any query's rows or `PartialModel` for a struct that also says which
columns and expressions to select. JavaScript has no struct to derive onto,
and the two cases go two ways.

A shape made of some of a table's columns is a model of its own: declare a
second model over the same table with the columns it reads, under the field
names it wants. Its `find` selects those columns and decodes each row into its
records, checked as any model's are. A field of a nullable column reads
`NULL` as `null`.

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

A shape holding an expression, or a column renamed in the SQL, is a
statement builder query whose items carry the names the program reads. Its
rows are plain objects, and nothing checks them against a declaration of
yours; TypeScript sees each value as any value a row can hold:

```ts
import assert from "node:assert/strict";
import { call, select } from "@necessary/pgorm";

// A line of the price list.
const tags = await pool.query(
  select(Cake.col("name"), Cake.col("price").as("euros"))
    .from(Cake.table)
    .where(Cake.col("bakeryId").eq(2))
    .orderBy(Cake.col("name").asc()),
);
assert.equal(tags[0]!.name, "Carrot Cake");
assert.equal(String(tags[0]!.euros), "3.80");

// A cake on the menu board, its name shouted.
const shout = select(call("upper", Cake.col("name")).as("shout"), Cake.col("glutenFree").as("free_from_gluten"))
  .from(Cake.table)
  .where(Cake.col("id").eq(2));
assert.equal(
  shout.inspect().sql,
  `SELECT UPPER("cake"."name") AS "shout", "cake"."gluten_free" AS "free_from_gluten" FROM "cake" WHERE "cake"."id" = $1`,
);
assert.deepEqual(await pool.one(shout), { shout: "LEMON DRIZZLE", free_from_gluten: true });
```

**Not available in JavaScript yet:** a declared shape that holds an
expression and is checked as it decodes. Rust's `PartialModel` selects
`from_expr` items and decodes them into typed fields, and its `skip` fields
read no column; a JavaScript model's fields are columns only.

## Joins

A join follows a relation the models declare. `join(relation)` joins the
table at a relation's far end, from the query's model or from a table it
joined already, as an `INNER JOIN`; `{ kind: "left" }` makes it a
`LEFT JOIN`, and `{ alias }` joins the table under another name, for a table
joined twice. A JavaScript relation is a value with a direction, made from the
model it starts at ([Entities](03-entities.md)), so a join from the other
side is the other side's relation, `bakeryCakes` rather than `cakeBakery`,
where Rust names the reverse with the `_rev` joins.

A join adds rows to the query, not columns: the records stay the queried
model's. It is how a query filters on a related row:

```ts
import assert from "node:assert/strict";

const inOslo = Cake.find().join(cakeBakery).where(Bakery.col("city").eq("Oslo"));
assert.equal(
  inOslo.inspect().sql,
  `SELECT "cake"."id", "cake"."name", "cake"."flavor", "cake"."price", "cake"."gluten_free", "cake"."bakery_id" FROM "cake" INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" WHERE "bakery"."city" = $1`,
);
assert.equal((await inOslo.all(pool)).length, 3);
```

To read columns of the joined table too, take the query's `statement`, the
[`Select`](../lib/select.d.ts) it runs, and choose the columns with the
statement builder; its rows are plain objects. To read each row together with
its related record, use the graph in [Relations](07-relations.md) instead.

```ts
import assert from "node:assert/strict";

const menu = Cake.find()
  .join(cakeBakery)
  .where(Cake.col("glutenFree").eq(true))
  .statement.select(Bakery.col("city"), Cake.col("name"))
  .orderBy(Bakery.col("city").asc(), Cake.col("name").asc());
assert.equal(
  menu.inspect().sql,
  `SELECT "bakery"."city", "cake"."name" FROM "cake" INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" WHERE "cake"."gluten_free" = $1 ORDER BY "bakery"."city" ASC, "cake"."name" ASC`,
);
assert.deepEqual(await pool.query(menu), [
  { city: "Bergen", name: "Vanilla Sponge" },
  { city: "Oslo", name: "Chocolate Torte" },
  { city: "Oslo", name: "Lemon Drizzle" },
]);

// The bakeries that sell a chocolate cake, read from the bakery's side.
const chocolateShops = Bakery.find().join(bakeryCakes).where(Cake.col("name").startsWith("Chocolate"));
// Sunrise sells two, so the join reads it twice.
assert.deepEqual((await chocolateShops.all(pool)).map((bakery) => bakery.name), ["Sunrise", "Sunrise"]);
assert.deepEqual(await pool.query(chocolateShops.statement.distinct()), [{ id: 1, name: "Sunrise", city: "Oslo" }]);
```

## Grouping and counting

`count(db)` on any model query counts its rows, limit and offset aside, and
resolves with a `number`. Grouping is the statement builder's: `groupBy`
groups the rows, the aggregates `call("count", ..)`, `"sum"`, `"min"`,
`"max"` and `"avg"` summarize each group, and `having` filters the groups.

**Not available in JavaScript yet:** `group_by` and `having` on a model
query, with a column's aggregate methods (`count`, `sum`, `min`, `max`).
Rust's `Select` groups and decodes the groups with `into_tuple`; a
JavaScript model query does not group, and the statement builder does.

```ts
import assert from "node:assert/strict";
import { call, select } from "@necessary/pgorm";

const cakes = call("count", Cake.col("id"));
const busy = select(Bakery.col("name"), cakes.as("cakes"))
  .from(Cake.table)
  .join(Bakery.table, Cake.col("bakeryId").eq(Bakery.col("id")))
  .groupBy(Bakery.col("name"))
  .having(cakes.gt(2));
assert.deepEqual(await pool.query(busy), [{ name: "Sunrise", cakes: 3n }]);

const glutenFree = await Cake.find().where(Cake.col("glutenFree").eq(true)).count(pool);
assert.equal(glutenFree, 3);
```

## Locking rows

Inside a [transaction](08-transactions.md), a `SELECT .. FOR UPDATE` reads
rows so that no other transaction can change them until this one ends, and
`FOR SHARE` reads them so that none can delete or update them. `NOWAIT` and
`SKIP LOCKED` say what to do about a row another transaction holds, the
second of which suits a queue of work that several workers take from.

A model query does not lock. The statement builder's
[`lock`](../lib/select.d.ts) does, over the query's `statement`: its strength
is `"update"`, `"noKeyUpdate"`, `"share"` or `"keyShare"`, and `{ wait }`
takes `"nowait"` or `"skipLocked"`. Rust's `Select` locks with
`lock_exclusive`, `lock_shared` and `lock_with_behavior`, and still decodes
into models; in JavaScript the locked rows are plain objects.

```ts
import assert from "node:assert/strict";

const next = Order.select("id")
  .orderBy(Order.col("placedAt").asc())
  .limit(1)
  .statement.lock("update", { wait: "skipLocked" });
assert.equal(
  next.inspect().sql,
  `SELECT "order"."id" FROM "order" ORDER BY "order"."placed_at" ASC LIMIT $1 FOR UPDATE SKIP LOCKED`,
);

const taken = await pool.transaction(async (tx) => tx.one(next));
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
