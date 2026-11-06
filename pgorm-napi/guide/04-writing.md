# Writing rows

A model writes rows through three statements of its own:
[`insert`](../lib/models.d.ts), [`update`](../lib/models.d.ts) and
[`delete`](../lib/models.d.ts). This chapter writes to the bakery: it adds
cakes, changes their prices, removes them, and signs customers up without
creating duplicates. It is the JavaScript dialect of the [Rust guide's
chapter](../../docs/guide/03-writing.md) of the same name.

## Active models

A Rust program writes a row through an `ActiveModel`, a copy of the `Model`
in which each field is set to a value, left out, or marked unchanged. A
JavaScript model needs no second type for it: a write takes a plain object,
and the object says what to do with each field.

| In the object | Meaning |
| --- | --- |
| a field with a value | write that value to the column |
| a field set to `null` | write `NULL`, where the column is nullable |
| a field left out | leave the column out of the statement |

A field left out of an insert takes its column's default in the database,
which for the generated `id` is the next number. A field left out of an update
keeps the value the row has. The TypeScript declarations type each object
from the model: an insert's object must set every field that is neither
nullable, defaulted nor generated, and accepts only the fields the model has
([TypeScript](26-typescript.md)).

```ts
import assert from "node:assert/strict";
import { Decimal, type InsertOf } from "@necessary/pgorm";

const muffin: InsertOf<typeof Cake> = {
  name: "Blueberry Muffin",
  flavor: "vanilla",
  price: new Decimal("2.75"),
  glutenFree: false,
  bakeryId: 2,
};

// The id is left out, so the database chooses it.
const { sql } = Cake.insert(muffin).inspect();
assert.equal(
  sql,
  `INSERT INTO "cake" ("name", "flavor", "price", "gluten_free", "bakery_id") VALUES ($1, CAST($2::text AS public.flavor), $3, $4, $5)`,
);
```

There is no `into_active`, `set_if_not_equals`, `is_changed` or `reset` on a
declared model's rows, because there is no active model to ask: a record read
from the database is a plain object, and an update writes the fields its
object names. A Rust entity registered with the binding has its real
`ActiveModel` in JavaScript, with those methods and the entity's hooks
([Registered Rust entities](24-registered-entities.md)).

## Inserting a row

`insert(values)` builds an `INSERT` of one row. Its terminals say what comes
back:

| Terminal | Returns |
| --- | --- |
| `execute(db)` | the number of rows written, with no `RETURNING` |
| `returning(...fields).one(db)` | the row as it was stored, or the fields named |
| `returningUpsert(db)` | whether it inserted or updated, [below](#upserts) |

Rust's `exec_returning_pk` and `exec_returning_model` are both
`returning(..)` here: `returning("id")` reads the key back, and `returning()`
with no fields the whole record. The record comes back as the database stored
it, defaults and the generated `id` included. A declared model has no hooks,
so nothing runs around the statement.

```ts
import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";

const cupcake = (name: string, price: string) =>
  Cake.insert({ name, flavor: "chocolate", price: new Decimal(price), glutenFree: false, bakeryId: 1 });

const { id } = await cupcake("Mocha Cupcake", "3.10").returning("id").one(pool);
assert.equal(id, 6);

const cherry = await cupcake("Cherry Cupcake", "3.30").returning().one(pool);
assert.deepEqual([cherry.id, cherry.name], [7, "Cherry Cupcake"]);

const written = await cupcake("Plain Cupcake", "2.50").execute(pool);
assert.equal(written, 1);
```

The statement binds every value as a parameter. An enum value is cast to its
type, since it travels as text:

```ts
import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";

const { sql, values } = Cake.insert({
  name: "Mocha Cupcake",
  flavor: "chocolate",
  price: new Decimal("3.10"),
  glutenFree: false,
  bakeryId: 1,
}).inspect();
assert.equal(
  sql,
  `INSERT INTO "cake" ("name", "flavor", "price", "gluten_free", "bakery_id") VALUES ($1, CAST($2::text AS public.flavor), $3, $4, $5)`,
);
assert.equal(values.length, 5);
```

The cast names the type's schema, `public.flavor`, because a JavaScript model
declares its enum with one ([Entities](03-entities.md)). Rust's entity writes
the name it was given, `flavor`.

## Inserting many rows

`insertMany(rows)` writes a batch in one statement, Rust's `Insert::many`,
and `returning(..).all(db)` reads one record per row, in the order the rows
were given. `execute` returns the count. `add(values)` and `addMany(rows)`
grow a batch, and turn a one-row insert into a batch, as Rust's `add` and
`add_many` do.

Every row of a batch must set the same fields, since one `INSERT` has one
column list; a batch that does not is refused with a `ConstructionError`
before anything is sent. A batch of no rows sends nothing: `execute` resolves
with `0`, and `returning(..).all(db)` with an empty array.

```ts
import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";

const tart = (name: string, flavor: "lemon" | "chocolate" | "vanilla" | "carrot", price: string) => ({
  name,
  flavor,
  price: new Decimal(price),
  glutenFree: true,
  bakeryId: 2,
});

const ids = await Cake.insertMany([
  tart("Lemon Tart", "lemon", "4.20"),
  tart("Chocolate Tart", "chocolate", "4.50"),
]).returning("id").all(pool);
assert.deepEqual(ids.map((row) => row.id), [6, 7]);

const tarts = await Cake.insertMany([tart("Vanilla Tart", "vanilla", "4.00")])
  .add(tart("Carrot Tart", "carrot", "4.10"))
  .returning()
  .all(pool);
assert.deepEqual(tarts.map((row) => row.name), ["Vanilla Tart", "Carrot Tart"]);

assert.equal(await Cake.insertMany([]).execute(pool), 0);
```

A batch whose rows set different fields:

```ts
import assert from "node:assert/strict";
import { ConstructionError, Decimal } from "@necessary/pgorm";

const price = new Decimal("1.00");
assert.throws(
  () =>
    Cake.insertMany([
      { name: "Plain Bun", flavor: "vanilla", price, glutenFree: false, bakeryId: 1 },
      { id: 50, name: "Iced Bun", flavor: "vanilla", price, glutenFree: false, bakeryId: 1 },
    ]).inspect(),
  ConstructionError,
);
```

## Updating rows

`update(values)` builds an `UPDATE` of the fields its object names, and
`where(..)` says which rows. To change a row you have read, name it by its
primary key with [`key`](../lib/models.d.ts), which takes every key field and
no other, and set the fields that change. `returning()` reads the row as it
now is:

```ts
import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";

const fudge = await Cake.findByKey({ id: 1 }).one(pool);

const cheaper = Cake.update({ price: new Decimal("3.99") }).where(Cake.key({ id: fudge.id }));
assert.equal(cheaper.inspect().sql, `UPDATE "cake" SET "price" = $1 WHERE "cake"."id" = $2`);

const updated = await cheaper.returning().one(pool);
assert.equal(updated.price.toString(), "3.99");
assert.equal(updated.name, "Chocolate Fudge");
```

Rust has two builders here, `Update::one` for a row named by its model's key
and `Update::many` for every row a filter matches. A JavaScript update is one
builder whose `where` does either. It needs a `where`, or `allRows()` to say
it means every row: one with neither is refused with a `ConstructionError`
before anything is sent, so a forgotten condition never rewrites a table. A
key with a field missing, or with a field that is not part of the key, is a
`TypeError` as `key` is called, where Rust refuses a model whose key is not
set with `Error::PrimaryKeyNotSet`.

A value in the object can be an expression instead of a value, such as one
that reads the row's own columns. `execute` returns the number of rows
changed, and `returning(..).all(db)` the rows themselves:

```ts
import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";

// Sunrise raises every price by fifty cents.
const raise = Cake.update({ price: Cake.col("price").add(new Decimal("0.50")) })
  .where(Cake.col("bakeryId").eq(1));
assert.equal(
  raise.inspect().sql,
  `UPDATE "cake" SET "price" = "cake"."price" + $1::numeric WHERE "cake"."bakery_id" = $2`,
);
assert.equal(await raise.execute(pool), 3);

// Moonlight's cakes all become gluten-free.
const changed = await Cake.update({ glutenFree: true })
  .where(Cake.col("bakeryId").eq(2))
  .returning()
  .all(pool);
assert.ok(changed.every((cake) => cake.glutenFree));
assert.equal(changed.length, 2);
```

An update with nothing to set is refused with a `ConstructionError`, as
Rust's `Error::NothingToSet` refuses it, since there would be no statement to
send. An update whose `where` matches nothing writes nothing: `execute`
resolves with `0`. `UPDATE` takes no `ORDER BY` or `LIMIT` in PostgreSQL, so
the builder has neither; filter on a subquery instead.

```ts
import assert from "node:assert/strict";
import { ConstructionError } from "@necessary/pgorm";

assert.throws(() => Cake.update({}).where(Cake.key({ id: 1 })).inspect(), ConstructionError);
assert.throws(() => Cake.update({ glutenFree: true }).inspect(), ConstructionError);
assert.equal(await Cake.update({ glutenFree: true }).where(Cake.key({ id: 99 })).execute(pool), 0);
```

A model's update reads no other table. Rust's `Update::many(..).from(..)`,
which joins another table in the filter, is the statement builder's
`update(table).from(item)` here ([The query builder](11-query-builder.md)).

### Before and after

PostgreSQL 18's `RETURNING` can read a row both as it was and as the update
left it. `returningChange(db)` and `returningChanges(db)` return each row as a
[`Change`](../lib/models.d.ts), its `old` and `new` records, as Rust's
`exec_returning_change` and `exec_returning_changes` do. `returningChange`
wants exactly one row, and any other count is a `DecodeError`:

```ts
import assert from "node:assert/strict";
import { DecodeError } from "@necessary/pgorm";

const changes = await Cake.update({ price: Cake.col("price").mul(2) })
  .where(Cake.col("name").eq("Vanilla Sponge"))
  .returningChanges(pool);
assert.equal(changes.length, 1);
assert.equal(changes[0]!.old.price.toString(), "2.90");
assert.equal(changes[0]!.new.price.toString(), "5.80");

// The one row a key names, before and after; a key that matches nothing is no change.
const { old, new: now } = await Cake.update({ glutenFree: true }).where(Cake.key({ id: 1 })).returningChange(pool);
assert.deepEqual([old.glutenFree, now.glutenFree], [false, true]);
await assert.rejects(
  Cake.update({ glutenFree: true }).where(Cake.key({ id: 99 })).returningChange(pool),
  DecodeError,
);
```

## Deleting rows

`delete()` builds a `DELETE`, which needs `where(..)` or `allRows()` as an
update does. A row is deleted by its key, and `execute` returns the number of
rows deleted; a delete that matches nothing resolves with `0`. Rust's
`Model::delete`, `Delete::one`, `delete_by_id` and `Delete::many` are all this
one builder, and none runs hooks. A composite key names each of its fields,
where Rust's `delete_by_id` takes a tuple:

```ts
import assert from "node:assert/strict";
import { select } from "@necessary/pgorm";

// Cy no longer bakes the Vanilla Sponge.
const row = await CakeBaker.findByKey({ cakeId: 4, bakerId: 3 }).one(pool);
assert.equal(await CakeBaker.delete().where(CakeBaker.key(row)).execute(pool), 1);

// Bo leaves the Lemon Drizzle to Ada.
assert.equal(await CakeBaker.delete().where(CakeBaker.key({ cakeId: 2, bakerId: 2 })).execute(pool), 1);

// Moonlight stops selling the cakes nobody bakes.
const unbaked = Cake.delete()
  .where(Cake.col("bakeryId").eq(2))
  .where(Cake.col("id").isNotIn(select(CakeBaker.col("cakeId")).from(CakeBaker.table)));
assert.equal(await unbaked.execute(pool), 1);
assert.equal(await Cake.find().count(pool), 4);
```

`key` takes the key's fields and nothing else, so a cake_baker record, which
holds only its key, passes as it is, while a cake record is given as
`Cake.key({ id: cake.id })`. `returning(..)` reads the deleted rows as they
were. Rust's `Delete::many(..).using(..)`, PostgreSQL's `DELETE .. USING`, is
the statement builder's `deleteFrom(table).using(item)`.

## Upserts

An upsert inserts a row unless it would conflict with one already there, and
then does something else: updates the row that is there, or skips the new
one. `onConflict(..)` attaches PostgreSQL's `ON CONFLICT` clause, built with
[`Conflict`](../lib/writes.d.ts): the columns of the unique key that may
conflict, and what to do. `update(column)` takes a column's new value from
the row that failed to insert.

The conflict names SQL columns, not a model's fields: `Conflict.on("email")`,
and `"cake_id"` rather than `cakeId`. Rust's `OnConflict` names its entity's
`Column`s.

The bakery's customers are unique by email. A sign-up from an address already
known updates the name instead of failing. `returningUpsert(db)` says which
happened, as Rust's `exec_returning_upsert` does: `{ kind: "inserted", new }`
with the new row, or `{ kind: "updated", old, new }` with the row before and
after:

```ts
import assert from "node:assert/strict";
import { Conflict } from "@necessary/pgorm";

const signUp = (name: string, email: string) =>
  Customer.insert({ name, email }).onConflict(Conflict.on("email").update("name"));

const kim = await signUp("Kimberly", "kim@example.com").returningUpsert(pool);
assert.equal(kim?.kind, "updated");
if (kim?.kind === "updated") assert.deepEqual([kim.old.name, kim.new.name], ["Kim", "Kimberly"]);

const mo = await signUp("Mo", "mo@example.com").returningUpsert(pool);
// Kim's upsert drew id 3 before it conflicted, so Mo gets 4.
assert.equal(mo?.kind, "inserted");
assert.equal(mo?.new.id, 4);
```

An insert draws its generated key before it finds the conflict, so a
conflicting row uses up a number of the sequence, as Kim's did.
`returningUpserts(db)` on a batch, `insertMany(..)`, does the same row by row.
The other terminals work under `ON CONFLICT` too: `returning("id")` reads the
key of the row written, whether it was inserted or updated.

To skip conflicting rows instead, `Conflict.doNothing()` adds
`ON CONFLICT DO NOTHING`, for a conflict on any key, and
`Conflict.on(..).doNothing()` names one key. Rust turns such an insert into a
`TryInsert`, whose terminals answer `Inserted`, `Conflicted` or `Empty`. In
JavaScript the counts say it: `execute` resolves with `0` for a row that was
skipped, `returningUpsert` with `null`, and a batch's `returningUpserts`
leaves a skipped row out. An empty batch sends nothing: `execute` resolves
with `0` and `returningUpserts` with an empty array, which its length already
told you.

```ts
import assert from "node:assert/strict";
import { Conflict } from "@necessary/pgorm";

// Ada already bakes the Chocolate Fudge.
const again = CakeBaker.insert({ cakeId: 1, bakerId: 1 }).onConflict(Conflict.on("cake_id", "baker_id").doNothing());
assert.equal(
  again.inspect().sql,
  `INSERT INTO "cake_baker" ("cake_id", "baker_id") VALUES ($1, $2) ON CONFLICT ("cake_id", "baker_id") DO NOTHING`,
);
assert.equal(await again.execute(pool), 0);
assert.equal(await again.returningUpsert(pool), null);

// Bo starts baking the Chocolate Fudge too.
const bo = CakeBaker.insert({ cakeId: 1, bakerId: 2 }).onConflict(Conflict.doNothing());
assert.equal(await bo.execute(pool), 1);

assert.deepEqual(await CakeBaker.insertMany([]).returningUpserts(pool), []);
```

`returning(..).one(db)` over a row the conflict skipped finds no row, a
`DecodeError`; `returning(..).optional(db)` resolves with `null` instead.

## MERGE

PostgreSQL's `MERGE` reconciles a table with a source of rows in one
statement: it updates the rows that match, inserts the ones that do not, and
can delete. pgorm builds it with the statement builder's
[`merge`](../lib/merge.d.ts), not with a model; [The query
builder](11-query-builder.md) covers it.
