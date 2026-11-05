# TypeScript

*This chapter is JavaScript's own: the Rust guide has no counterpart of it.*
Rust checks a program's use of pgorm when it compiles: a record's fields, an
insert's columns, a key's parts and a comparison's operand are all types.
The package carries the same checks as TypeScript declarations, so a
TypeScript program is held to them by its type checker, and a JavaScript
program meets the same mistakes as run-time refusals.

## The declarations

The package ships a declaration file beside each module, `lib/index.d.ts`
and the files it re-exports, declaring every export, the error classes and
their fields included. Node.js tooling finds them through the package's
`types` entry, and Deno through the module's `@ts-self-types` directive, so
`deno check` and `tsc` check a program against them with no configuration.

Node.js runs a `.ts` file by stripping its types, and so runs only the
TypeScript that erases to JavaScript: no `enum`, no `namespace`, no
parameter properties, and a type imported by name is imported with `type`,
`import { type RowOf } from "@necessary/pgorm"`. Every example in this
guide is written that way, runs in both runtimes, and is type-checked by
`deno check`.

## Types from a declaration

A model's declaration is one object literal, and the declarations derive
every type of its reads and writes from it, so no type is written twice:

| Type | Is |
| --- | --- |
| `RowOf<typeof Model>` | a record, as a read decodes it: each field its kind's value, `\| null` where it is nullable, an array of `\| null` items where it is an array, an enum's listed labels as a union |
| `InsertOf<typeof Model>` | an insert's values: every field neither nullable, defaulted nor generated is required, the rest optional, and a field generated always is absent |
| `UpdateOf<typeof Model>` | an update's assignments: any field but one generated always |
| `KeyOf<typeof Model>` | a primary key: each key field's value |

```ts
import assert from "node:assert/strict";
import { type InsertOf, type KeyOf, type RowOf } from "@necessary/pgorm";

type CakeRecord = RowOf<typeof Cake>;

const lemon: CakeRecord = await Cake.findByKey({ id: 2 }).one(pool);
const flavor: "chocolate" | "vanilla" | "lemon" | "carrot" = lemon.flavor;
assert.equal(flavor, "lemon");

// An id and the notes may be left out: the table fills one, and the other is nullable.
const mo: InsertOf<typeof Customer> = { name: "Mo", email: "mo@example.com" };
const saved = await Customer.insert(mo).returning().one(pool);
const notes: string | null = saved.notes;
assert.equal(notes, null);

const key: KeyOf<typeof CakeBaker> = { cakeId: 2, bakerId: 2 };
assert.ok(await CakeBaker.findByKey(key).optional(pool));
```

Why a factory rather than a class: `model(name, { columns })` takes one
object literal and the declarations infer everything from it. A class
declared with decorators or static fields would need each type written a
second time, and decorators are not among what Node.js can strip.

## What the types refuse

What the package refuses at run time, the declarations refuse first where a
type can say it: a field the model does not have, a value of the wrong kind,
a label the enum does not list, a required field left out of an insert, a
write to a column generated always, a comparison with `null`. Each
`@ts-expect-error` below is a line the type checker refuses; `deno check`
fails if any of them stops being refused:

```ts
import assert from "node:assert/strict";
import { ConstructionError, Decimal, type InsertOf } from "@necessary/pgorm";

// @ts-expect-error: a customer needs an email
const incomplete: InsertOf<typeof Customer> = { name: "Mo" };
assert.equal(incomplete.name, "Mo");

assert.throws(() => {
  // @ts-expect-error: a cake has no field `colour`
  Cake.col("colour");
}, ConstructionError);

assert.throws(
  () =>
    Cake.update({
      // @ts-expect-error: a price is a Decimal, never a fractional number, which could round it
      price: 3.2,
    }).where(Cake.key({ id: 2 })),
  ConstructionError,
);

assert.throws(
  // @ts-expect-error: NULL is tested with isNull(), never compared
  () => Customer.find().where(Customer.col("notes").eq(null)),
  ConstructionError,
);

const fine = Cake.update({ price: new Decimal("3.25") }).where(Cake.key({ id: 2 }));
assert.equal(await fine.execute(pool), 1);
```

A JavaScript program has no type checker to refuse these lines, and each is
refused as it runs instead: a `TypeError` for an argument of a type the call
does not take, and a `ConstructionError` for one that cannot be used.

## Where the types stop

Some results have no declaration to be typed from:

- **SQL text and built statements** decode into `Row`, an object of
  `PlainValue`s keyed by column name, since no declaration says what SQL text
  returns. Narrow a value where it is used, or read through a model.
- **Registered Rust entities** read as `EntityRecord`, and a registered
  graph's rows as a record or a tuple of them, until the application
  generates declarations for its own registrations ([Generating
  registrations](25-generating-registrations.md)).
- **JSON columns** read as `JsonValue`, whatever shape their documents have.

A `bigint` is never a `number` in these types, and a `Decimal` never a
`number` either, because the values themselves are not ([Values and
types](13-values.md)).
