# One-of references

A review at the bakery is of a cake or of a bakery, never both and never
neither. Rust says that with an enum. PostgreSQL says it with one nullable
foreign-key column per target, `cake_id` and `bakery_id`, and a `CHECK` that
exactly one of them is set. Each column is an ordinary foreign key, so the
database still knows what every review points at, and the `CHECK` keeps a row
from pointing at two things. This chapter is the JavaScript dialect of the
[Rust guide's chapter](../../docs/guide/15-one-of.md).

[`oneOf`](../lib/one-of.d.ts) declares such a reference over the model holding
it, and gives you three things: a record's reference read as which target and
which key, the fields a write sets to point a row at one target with the
others cleared, and the targets of many records loaded with one query per
target table. [`createOneOf`](../lib/one-of.d.ts) writes the `CHECK`.

## Declaring one

The model holding the reference has a nullable field for each target, holding
the target's key. `oneOf(name, model, targets)` names each target by a value
of your choosing, and gives each the field holding its key and the target's
model. `{ optional: true }` lets a row reference none of them, where Rust
writes a unit variant:

```ts
import assert from "node:assert/strict";
import { column, model, oneOf } from "@necessary/pgorm";

const Review = model("review", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    stars: column("i16"),
    body: column("text"),
    cakeId: column("i32", { name: "cake_id", nullable: true }),
    bakeryId: column("i32", { name: "bakery_id", nullable: true }),
  },
});

const Subject = oneOf("subject", Review, {
  cake: ["cakeId", Cake],
  bakery: ["bakeryId", Bakery],
});

const { targets } = Subject.describe();
assert.deepEqual(targets.map((target) => target.field), ["cakeId", "bakeryId"]);

const onFudge = { id: 1, stars: 5, body: "Rich and dark", cakeId: 1, bakeryId: null };
assert.deepEqual(Subject.key(onFudge), ["cake", 1]);
```

Rust's derive writes two enums beside the reference, `SubjectKey`, each
variant holding its target's key, and `SubjectModel`, each holding its
target's `Model`. JavaScript writes a reference as a pair instead: `key` reads
a record's as `[value, key]`, and `load` a target's as `[value, record]`. Each
pair is typed by its target, so TypeScript narrows the record by the value.

What does not fit the shape is a `ConstructionError` as the reference is
declared, where Rust's does not compile: a field that is not nullable or not
of its target's key type, two targets naming one field, or two naming one
model. A Rust reference also
needs a relation to each target; a JavaScript model declares no foreign keys,
so the reference pairs each field with its target's key itself.

```ts
import assert from "node:assert/strict";
import { column, ConstructionError, model, oneOf } from "@necessary/pgorm";

const Review = model("review", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    stars: column("i16"),
    cakeId: column("i32", { name: "cake_id", nullable: true }),
  },
});

assert.throws(
  () => oneOf("subject", Review, { cake: ["stars", Cake] }),
  (error: unknown) => error instanceof ConstructionError && /is not nullable/.test(error.message),
);
```

## The constraint

The table's foreign keys are its own, written with the table. `createOneOf`
writes the `CHECK` beside them, `num_nonnulls(..) = 1`, or `<= 1` where the
reference is optional. It is named `ck-{table}-{name}`:

```ts
import assert from "node:assert/strict";
import { CheckViolationError, column, createOneOf, model, oneOf } from "@necessary/pgorm";

const Review = model("review", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    stars: column("i16"),
    body: column("text"),
    cakeId: column("i32", { name: "cake_id", nullable: true }),
    bakeryId: column("i32", { name: "bakery_id", nullable: true }),
  },
});
const Subject = oneOf("subject", Review, { cake: ["cakeId", Cake], bakery: ["bakeryId", Bakery] });

await pool.execute(
  `CREATE TABLE review (id serial PRIMARY KEY, stars smallint NOT NULL, body varchar NOT NULL,
     cake_id integer REFERENCES cake (id), bakery_id integer REFERENCES bakery (id))`,
);

const check = createOneOf(Subject);
assert.equal(
  check.inspect().sql,
  `ALTER TABLE "review" ADD CONSTRAINT "ck-review-subject" CHECK (num_nonnulls("cake_id", "bakery_id") = 1)`,
);
await pool.execute(check);

// A review of a cake and a bakery at once is refused.
await assert.rejects(
  Review.insert({ stars: 3, body: "Both?", cakeId: 1, bakeryId: 1 }).execute(pool),
  (error: unknown) => error instanceof CheckViolationError && error.constraint === "ck-review-subject",
);
```

Rust's table comes from `create_table_from_entity`, which writes the foreign
keys from the entity's relations; a JavaScript model's table is written by hand
or with the DDL builders ([Schema generation](14-schema.md)).

## Reading and writing

`key(record)` reads a record's reference as its `[value, key]`. `set(value,
key)` gives the fields a write sets: the target's field to its key and every
other to `null`, so moving a review from a cake to a bakery is one call. Rust's
`set` changes an `ActiveModel` in place; JavaScript's returns the fields, to
spread into an insert or pass as an update. `load(db, records)` takes a list of
records and returns each one's `[value, target record]`, in order, reading
each target table once, with one query for the keys that target holds:

```ts
import assert from "node:assert/strict";
import { column, createOneOf, model, oneOf } from "@necessary/pgorm";

const Review = model("review", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    stars: column("i16"),
    body: column("text"),
    cakeId: column("i32", { name: "cake_id", nullable: true }),
    bakeryId: column("i32", { name: "bakery_id", nullable: true }),
  },
});
const Subject = oneOf("subject", Review, { cake: ["cakeId", Cake], bakery: ["bakeryId", Bakery] });
await pool.execute(
  `CREATE TABLE review (id serial PRIMARY KEY, stars smallint NOT NULL, body varchar NOT NULL,
     cake_id integer REFERENCES cake (id), bakery_id integer REFERENCES bakery (id))`,
);
await pool.execute(createOneOf(Subject));

await Review.insertMany([
  { stars: 5, body: "Rich and dark", ...Subject.set("cake", 1) },
  { stars: 4, body: "Friendly staff", ...Subject.set("bakery", 2) },
  { stars: 3, body: "A little dry", ...Subject.set("cake", 3) },
]).execute(pool);

const reviews = await Review.find().orderBy(Review.col("id").asc()).all(pool);
const subjects = await Subject.load(pool, reviews);
const named = subjects.map((subject) =>
  subject === null ? "nothing" : subject[0] === "cake" ? `cake ${subject[1].name}` : `bakery ${subject[1].name}`
);
assert.deepEqual(named, ["cake Chocolate Fudge", "bakery Moonlight", "cake Carrot Cake"]);

// The third review was really about the bakery that sells the cake.
const moved = await Review.update(Subject.set("bakery", 2))
  .where(Review.key({ id: reviews[2]!.id }))
  .returning()
  .one(pool);
assert.deepEqual([moved.cakeId, moved.bakeryId], [null, 2]);
assert.deepEqual(Subject.key(moved), ["bakery", 2]);
```

A row the `CHECK` rules out can still be met in a database without it: a
review with both fields set, or with neither where the reference is not
optional. `key` and `load` report it as a `DecodeError`, naming the table,
the row's key and the columns, where Rust's is `Error::Query`, rather than
read it as one of its targets. So does `load` for a key its target's table
does not hold:

```ts
import assert from "node:assert/strict";
import { column, DecodeError, model, oneOf } from "@necessary/pgorm";

const Review = model("review", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    stars: column("i16"),
    body: column("text"),
    cakeId: column("i32", { name: "cake_id", nullable: true }),
    bakeryId: column("i32", { name: "bakery_id", nullable: true }),
  },
});
const Subject = oneOf("subject", Review, { cake: ["cakeId", Cake], bakery: ["bakeryId", Bakery] });

const both = { id: 9, stars: 1, body: "Confused", cakeId: 1, bakeryId: 1 };
assert.throws(
  () => Subject.key(both),
  (error: unknown) => error instanceof DecodeError && /references more than one target/.test(error.message),
);
```

The targets of one record are also reachable the ordinary way: each field is
a relation's key, so `Review.belongsTo(Cake, { from: "cakeId", to: "id" })`
and the graph of [Relations](07-relations.md) read them too. `oneOf` is for
when the program wants the one target a row has, whichever it is.

A Rust application's own `#[derive(OneOf)]` enum is reached from JavaScript
through its registration, `oneOfEntity(name)`, whose `set` writes the entity's
ActiveModel ([Registered Rust entities](24-registered-entities.md)).

## Generated entities

Rust's pgorm-codegen recognises the shape in a schema: nullable foreign-key
columns covered by a `CHECK (num_nonnulls(..) = 1)` or `<= 1` generate a
`OneOf` enum, named for the constraint, beside the entity. JavaScript
generates no declarations from a schema ([Generating
entities](23-codegen.md)), so a reference is declared by hand, as above.
