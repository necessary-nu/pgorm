# Disjoint enums

Some things come in kinds, each kind with data of its own. The bakery takes
payments by card, in cash or with a gift voucher: every payment has an
amount, a card payment also has the card's last four digits, a voucher
payment the voucher's code, and a cash payment nothing more. In Rust that is
an enum. In PostgreSQL it is a *base* table holding what every payment has,
with a column saying which kind each row is, and a table for each kind that
carries data, keyed by the base row's key. This chapter is the JavaScript
dialect of the [Rust guide's chapter](../../docs/guide/14-disjoint.md).

[`disjoint`](../lib/disjoint.d.ts) declares such a value over models, and
pgorm writes the tables and the constraints that hold them together, reads a
row with its kind's row in one statement, and writes both in one statement.
It is pgorm's own disjoint shape, so every statement is the one Rust's
`#[derive(Disjoint)]` declaration of the same tables sends. The constraints
are PostgreSQL's own, with no triggers, so a state the declaration cannot
describe cannot be written, by pgorm or by anything else connected to the
database: a card payment without its card row, a card row whose payment is
cash, a payment with rows in two kinds' tables.

## Declaring one

The base is an ordinary model whose `kind` field holds the kind, a column of
an enum of the kinds. Each kind with data is an ordinary model too, keyed by
the same types as the base, its key not filled by the database, since it is
its base row's. `disjoint(name, base, kinds)` lists the kinds, in order, each
value naming the model of its table, or `null` for a kind without one. Rust
declares the same as an enum: a variant holding a kind entity's `Model` is a
kind with a table, and a unit variant one without.

```ts
import assert from "node:assert/strict";
import { column, disjoint, model, TypeName } from "@necessary/pgorm";

const paymentKind = new TypeName("payment_kind", { schema: "public" });

const Payment = model("payment", {
  columns: {
    id: column("i32", { primaryKey: true, generated: "byDefault" }),
    kind: column(paymentKind, { values: ["card", "cash", "voucher"] }),
    orderId: column("i32", { name: "order_id" }),
    amount: column("decimal"),
  },
});
const CardPayment = model("card_payment", {
  columns: {
    paymentId: column("i32", { name: "payment_id", primaryKey: true }),
    lastFour: column("text", { name: "last_four" }),
  },
});
const VoucherPayment = model("voucher_payment", {
  columns: {
    paymentId: column("i32", { name: "payment_id", primaryKey: true }),
    code: column("text"),
  },
});

const Payments = disjoint(
  "payment",
  Payment,
  { card: CardPayment, cash: null, voucher: VoucherPayment },
  { schemaName: "public" },
);

const { kindField, kindType, kinds } = Payments.describe();
assert.equal(kindField, "kind");
assert.deepEqual(kindType, { name: "payment_kind", schema: "public" });
assert.deepEqual(kinds.map((kind) => kind.value), ["card", "cash", "voucher"]);
```

The kind type is named `{name}_kind`, and the base's kind field is `kind`.
The options rename them: `kind` names another field, `enumName` and
`schemaName` the kind type. An enum column of a model names its schema, so
the declaration says `schemaName: "public"` for the kind type to be the one
the column holds; Rust's kind type is `payment_kind`, unqualified, and the
derive writes the kind type itself, `PaymentKind`, an `ActiveEnum` beside the
enum, and `PaymentActiveModel` for its writes. JavaScript names a kind by its
value, a string.

Rust checks the shape with `check_disjoint` when it runs, as every read and
write does first. JavaScript checks it as `disjoint` is called: a kind column
that is nullable, generated or not of the kind type, and a kind model whose
key the database fills, by a sequence, an identity or a default, are each a
`ConstructionError` there:

```ts
import assert from "node:assert/strict";
import { column, ConstructionError, disjoint, model, TypeName } from "@necessary/pgorm";

const paymentKind = new TypeName("payment_kind", { schema: "public" });
const Payment = model("payment", {
  columns: {
    id: column("i32", { primaryKey: true, generated: "byDefault" }),
    kind: column(paymentKind, { values: ["card", "cash"], nullable: true }),
  },
});
const CardPayment = model("card_payment", {
  columns: { paymentId: column("i32", { name: "payment_id", primaryKey: true }) },
});

assert.throws(
  () => disjoint("payment", Payment, { card: CardPayment, cash: null }, { schemaName: "public" }),
  (error: unknown) => error instanceof ConstructionError && /is nullable/.test(error.message),
);
```

## Creating the tables

[`createDisjoint`](../lib/disjoint.d.ts) returns the statements that create
the kind type and the tables, in the order they run: the kind type, the base
table, each kind's table, and one `ALTER TABLE` on the base adding what points
back at the kinds' tables. It is Rust's `Schema::create_disjoint`. Each
table's columns are written as [`fromModel`](14-schema.md) writes a model's:
typed from the models' kinds unless a column's `columnType` names another,
so a `text` field is a `text` column and a `decimal` one a `numeric` of no
fixed scale, with the default, generation expression or identity the column
states. A model's indexes and comments are not among the statements;
`fromModel` of the model gives those. The base's key here is an identity
column, which a model declares `generated: "byDefault"`:

<!-- example: empty -->
```ts
import assert from "node:assert/strict";
import { column, createDisjoint, disjoint, model, TypeName } from "@necessary/pgorm";

const paymentKind = new TypeName("payment_kind", { schema: "public" });
const Payment = model("payment", {
  columns: {
    id: column("i32", { primaryKey: true, generated: "byDefault" }),
    kind: column(paymentKind, { values: ["card", "cash", "voucher"] }),
    orderId: column("i32", { name: "order_id" }),
    amount: column("decimal"),
  },
});
const CardPayment = model("card_payment", {
  columns: {
    paymentId: column("i32", { name: "payment_id", primaryKey: true }),
    lastFour: column("text", { name: "last_four" }),
  },
});
const VoucherPayment = model("voucher_payment", {
  columns: {
    paymentId: column("i32", { name: "payment_id", primaryKey: true }),
    code: column("text"),
  },
});
const Payments = disjoint(
  "payment",
  Payment,
  { card: CardPayment, cash: null, voucher: VoucherPayment },
  { schemaName: "public" },
);

const statements = createDisjoint(Payments);
assert.deepEqual(statements.map((statement) => statement.inspect().sql), [
  `CREATE TYPE "public"."payment_kind" AS ENUM ('card', 'cash', 'voucher')`,
  [
    `CREATE TABLE "payment" ( "id" integer NOT NULL GENERATED BY DEFAULT AS IDENTITY, "kind" public.payment_kind NOT NULL,`,
    `"order_id" integer NOT NULL, "amount" decimal NOT NULL, PRIMARY KEY ("id"),`,
    `CONSTRAINT "uq-payment-id-kind" UNIQUE ("id", "kind") )`,
  ].join(" "),
  [
    `CREATE TABLE "card_payment" ( "payment_id" integer NOT NULL, "last_four" text NOT NULL,`,
    `"kind" public.payment_kind GENERATED ALWAYS AS ('card') STORED, PRIMARY KEY ("payment_id"),`,
    `CONSTRAINT "fk-card_payment-payment_id-kind" FOREIGN KEY ("payment_id", "kind")`,
    `REFERENCES "payment" ("id", "kind") ON DELETE CASCADE DEFERRABLE INITIALLY IMMEDIATE )`,
  ].join(" "),
  [
    `CREATE TABLE "voucher_payment" ( "payment_id" integer NOT NULL, "code" text NOT NULL,`,
    `"kind" public.payment_kind GENERATED ALWAYS AS ('voucher') STORED, PRIMARY KEY ("payment_id"),`,
    `CONSTRAINT "fk-voucher_payment-payment_id-kind" FOREIGN KEY ("payment_id", "kind")`,
    `REFERENCES "payment" ("id", "kind") ON DELETE CASCADE DEFERRABLE INITIALLY IMMEDIATE )`,
  ].join(" "),
  [
    `ALTER TABLE "payment"`,
    `ADD COLUMN "card_id" integer GENERATED ALWAYS AS ((CASE WHEN ("kind" = 'card') THEN "id" END)) STORED,`,
    `ADD COLUMN "voucher_id" integer GENERATED ALWAYS AS ((CASE WHEN ("kind" = 'voucher') THEN "id" END)) STORED,`,
    `ADD CONSTRAINT "fk-payment-card_id" FOREIGN KEY ("card_id") REFERENCES "card_payment" ("payment_id") DEFERRABLE INITIALLY IMMEDIATE,`,
    `ADD CONSTRAINT "fk-payment-voucher_id" FOREIGN KEY ("voucher_id") REFERENCES "voucher_payment" ("payment_id") DEFERRABLE INITIALLY IMMEDIATE`,
  ].join(" "),
]);

for (const statement of statements) await pool.execute(statement);
```

The constraints work like this. The base has a unique key over its key and
its kind. Each kind's table has a kind column of its own, generated as its
one kind, and a foreign key on its key and that column to the base's unique
key, so its row can only belong to a base row of its kind. The base has, for
each kind with a table, generated columns holding its key when the row is of
that kind and `NULL` otherwise, and a foreign key on them to that kind's
table, so a base row of that kind must have its row. Each foreign key is
`DEFERRABLE INITIALLY IMMEDIATE` and checked at the end of the statement,
which is how pgorm writes both rows in one statement. Deleting a base row
deletes its kind's row with it.

The generated columns and the kind tables' kind columns are bookkeeping: no
model has a field for them, and no read or write names them. A model's
`uniqueKeys` are written into its table here too.

## Writing

Every write is one statement, which applies whole or not at all, with no
transaction needed. Each is a [`DisjointWrite`](../lib/disjoint.d.ts), whose
terminal `returningModel` sends it and returns the row as a read would: the
base's record, the kind, and the kind's record or `null`. Rust's writes take
the base's `ActiveModel` and a `PaymentActiveModel`; JavaScript's take the
fields to write, as a model's writes do, and the kind by its value:

- `insert(base, kind, values)` inserts the base row from `base` and, for a
  kind with a table, its row from `values`. The base's kind field and the
  kind row's key are left out: the kind comes from `kind`, and the key from
  the inserted base row.
- `update(base, kind, values)` changes the given fields of a row and of its
  kind's row, found by the base's key, which `base` holds. Updating a row as
  another kind changes nothing, and is a `DecodeError`, where Rust's is
  `Error::RecordNotFound`.
- `changeKind(key, kind, values)` moves a row to another kind: it deletes the
  old kind's row, inserts the new one's, and sets the base row's kind.

Deleting needs nothing of its own: delete the base row as any row is deleted,
and its kind's row goes with it. Here `Payments` and its models are the ones
declared above, and their tables exist:

```ts
import assert from "node:assert/strict";
import {
  column, createDisjoint, DecodeError, Decimal, disjoint, ForeignKeyViolationError, model, TypeName,
} from "@necessary/pgorm";

const paymentKind = new TypeName("payment_kind", { schema: "public" });
const Payment = model("payment", {
  columns: {
    id: column("i32", { primaryKey: true, generated: "byDefault" }),
    kind: column(paymentKind, { values: ["card", "cash", "voucher"] }),
    orderId: column("i32", { name: "order_id" }),
    amount: column("decimal"),
  },
});
const CardPayment = model("card_payment", {
  columns: {
    paymentId: column("i32", { name: "payment_id", primaryKey: true }),
    lastFour: column("text", { name: "last_four" }),
  },
});
const VoucherPayment = model("voucher_payment", {
  columns: {
    paymentId: column("i32", { name: "payment_id", primaryKey: true }),
    code: column("text"),
  },
});
const Payments = disjoint(
  "payment",
  Payment,
  { card: CardPayment, cash: null, voucher: VoucherPayment },
  { schemaName: "public" },
);
for (const statement of createDisjoint(Payments)) await pool.execute(statement);

const [base, kind, card] = await Payments
  .insert({ orderId: 1, amount: new Decimal("12.20") }, "card", { lastFour: "4242" })
  .returningModel(pool);
assert.equal(kind, "card");
assert.deepEqual(card, { paymentId: base.id, lastFour: "4242" });

// A new card number: the kind's row changes, found by the base's key.
const [, , updated] = await Payments.update({ id: base.id }, "card", { lastFour: "4343" }).returningModel(pool);
assert.deepEqual(updated, { paymentId: base.id, lastFour: "4343" });

// An update as another kind finds no row of that kind.
await assert.rejects(
  Payments.update({ id: base.id, amount: new Decimal("1") }, "cash").returningModel(pool),
  DecodeError,
);

const [cash] = await Payments.insert({ orderId: 2, amount: new Decimal("3.80") }, "cash").returningModel(pool);

// The customer paid with a voucher after all.
const [moved, movedKind, voucher] = await Payments
  .changeKind({ id: cash.id }, "voucher", { code: "XMAS-2026" })
  .returningModel(pool);
assert.equal(moved.kind, "voucher");
assert.equal(movedKind, "voucher");
assert.deepEqual(voucher, { paymentId: cash.id, code: "XMAS-2026" });

// A card payment cannot lose its card row behind pgorm's back.
await assert.rejects(
  pool.execute(`DELETE FROM "card_payment" WHERE "payment_id" = $1`, [base.id]),
  ForeignKeyViolationError,
);

// Deleting the payment deletes its card row with it.
await Payment.delete().where(Payment.key({ id: base.id })).execute(pool);
assert.equal(await CardPayment.find().count(pool), 0);
```

A row's type is a union over the kinds, `[payment, "card", card] | [payment,
"cash", null] | ...`, so TypeScript narrows the kind's record by the kind.

## Reading

`find()` reads each base row with its kind's row in one statement, the base
`LEFT JOIN`ed to each kind's table, and decodes each row as the triple of the
base's record, the kind and the kind's record. Rust decodes the pair of the
base's `Model` and the enum. It takes what a model's `find()` takes: `where`
and `orderBy` on the base's columns, `limit` and `offset`, `count`, pages and
keyset cursors, and `findByKey`. Its terminals are `all`, `one` and `stream`.
Three methods are its own:

- `kind` and `kinds` read only rows of the given kinds;
- `filterKind(kind, predicate)` reads only rows of that kind whose kind row
  passes a condition.

```ts
import assert from "node:assert/strict";
import { column, createDisjoint, Decimal, disjoint, model, TypeName } from "@necessary/pgorm";

const paymentKind = new TypeName("payment_kind", { schema: "public" });
const Payment = model("payment", {
  columns: {
    id: column("i32", { primaryKey: true, generated: "byDefault" }),
    kind: column(paymentKind, { values: ["card", "cash", "voucher"] }),
    orderId: column("i32", { name: "order_id" }),
    amount: column("decimal"),
  },
});
const CardPayment = model("card_payment", {
  columns: {
    paymentId: column("i32", { name: "payment_id", primaryKey: true }),
    lastFour: column("text", { name: "last_four" }),
  },
});
const VoucherPayment = model("voucher_payment", {
  columns: {
    paymentId: column("i32", { name: "payment_id", primaryKey: true }),
    code: column("text"),
  },
});
const Payments = disjoint(
  "payment",
  Payment,
  { card: CardPayment, cash: null, voucher: VoucherPayment },
  { schemaName: "public" },
);
for (const statement of createDisjoint(Payments)) await pool.execute(statement);
await Payments.insert({ orderId: 1, amount: new Decimal("12.20") }, "card", { lastFour: "4242" }).returningModel(pool);
await Payments.insert({ orderId: 2, amount: new Decimal("3.80") }, "cash").returningModel(pool);
await Payments.insert({ orderId: 1, amount: new Decimal("4.50") }, "card", { lastFour: "1111" }).returningModel(pool);

const every = await Payments.find().orderBy(Payment.col("id").asc()).all(pool);
assert.deepEqual(every.map(([, kind]) => kind), ["card", "cash", "card"]);

assert.equal(await Payments.find().kind("card").count(pool), 2);

const [base, kind, card] = await Payments.find()
  .filterKind("card", CardPayment.col("lastFour").endsWith("42"))
  .one(pool);
assert.equal(base.orderId, 1);
assert.equal(kind, "card");
assert.deepEqual(card, { paymentId: base.id, lastFour: "4242" });
```

A disjoint read's pages count their rows as a model query's do:
`numItemsAndPages` gives `{ items, pages }`.

A row the constraints rule out can still be met in a database pgorm did not
create, such as a card payment with no card row. A read reports it as a
`DecodeError`, naming the table, the key and the kinds, where Rust's is
`Error::Query`, and never decodes it as some kind or drops it.

## Changing the kinds

A program grows new kinds, and the database has to follow. Each change is
written from the declaration as it will be, as a `DisjointChange`: a list of
`steps`, each holding `statements` run in one transaction, committed before
the next step starts. Some changes need two steps because PostgreSQL will not
use an enum value in the transaction that added it.

| Change | What it does |
| --- | --- |
| `addDisjointKind` | adds a kind: `ALTER TYPE .. ADD VALUE`, then the kind's table, in two steps |
| `addDisjointKindRecreating` | the same change in one step, by recreating the kind type and rewriting the tables once |
| `removeDisjointKind` | removes a kind no row has any more, recreating the type; it fails while a row still has the kind |
| `addDisjointKindTable` | gives a kind without a table a table, leaving a gap in which the program writes each existing row's kind row |
| `removeDisjointKindTable` | drops a kind's table, keeping the kind without one |

A step after which the program has rows to write carries a `gap`, `{ kind,
table }`, saying which; any other step's is `null`. A change is written from
the declaration, and the declaration keeps changing after the change ships,
so a migration must not compute it again each time it runs: freeze it when it
is made, as [below](#freezing-a-change-into-migrations).

Here the bakery began taking payments by card and in cash, and adds vouchers.
The tables are first created as the earlier declaration had them:

<!-- example: empty -->
```ts
import assert from "node:assert/strict";
import { addDisjointKind, column, createDisjoint, Decimal, disjoint, model, TypeName } from "@necessary/pgorm";

const paymentKind = new TypeName("payment_kind", { schema: "public" });
const payment = (kinds: string[]) =>
  model("payment", {
    columns: {
      id: column("i32", { primaryKey: true, generated: "byDefault" }),
      kind: column(paymentKind, { values: kinds }),
      orderId: column("i32", { name: "order_id" }),
      amount: column("decimal"),
    },
  });
const CardPayment = model("card_payment", {
  columns: {
    paymentId: column("i32", { name: "payment_id", primaryKey: true }),
    lastFour: column("text", { name: "last_four" }),
  },
});
const VoucherPayment = model("voucher_payment", {
  columns: {
    paymentId: column("i32", { name: "payment_id", primaryKey: true }),
    code: column("text"),
  },
});

const before = disjoint("payment", payment(["card", "cash"]), { card: CardPayment, cash: null }, {
  schemaName: "public",
});
for (const statement of createDisjoint(before)) await pool.execute(statement);

const Payments = disjoint(
  "payment",
  payment(["card", "cash", "voucher"]),
  { card: CardPayment, cash: null, voucher: VoucherPayment },
  { schemaName: "public" },
);

const { steps } = addDisjointKind(Payments, "voucher");
assert.equal(steps.length, 2);
assert.equal(
  steps[0]!.statements[0]!.inspect().sql,
  `ALTER TYPE "public"."payment_kind" ADD VALUE 'voucher' AFTER 'cash'`,
);
assert.ok(steps.every((step) => step.gap === null));

for (const step of steps) {
  await pool.transaction(async (tx) => {
    for (const statement of step.statements) await tx.execute(statement);
  });
  pool.clearStatementCaches();
}

const [, kind, voucher] = await Payments
  .insert({ orderId: 2, amount: new Decimal("3.80") }, "voucher", { code: "XMAS-2026" })
  .returningModel(pool);
assert.equal(kind, "voucher");
assert.deepEqual(voucher, { paymentId: 1, code: "XMAS-2026" });
```

A step that recreates the kind type changes the type prepared statements were
planned with, so a program running steps itself clears its pool's statement
caches after each, as above ([Statement caches and
DDL](19-statement-caches.md)). Rust's pgorm-migration runs each step as one
migration and clears the caches itself; JavaScript has no migration runner
([Migrations](22-migrations.md)), so the program runs the steps.

### Freezing a change into migrations

A migration that called `addDisjointKind` as it ran would compute the change
from the declaration as it stands on the day it runs. So would the migration
before it that called `createDisjoint`, which on a database built from
nothing would already create the voucher kind the next migration adds, and
the history would fail to replay. A change is therefore frozen into
migration files when it is made, and the migrations are the files, which
nothing computes again.

[`migrationFiles(change, name)`](../lib/disjoint.d.ts) freezes a change as
pgorm does, one `MigrationFile` per step, named `name_1`, `name_2` and on, or
`name` for a change of one step. [`migrationFile(name, statements, {
commitsAfter })`](../lib/disjoint.d.ts) freezes any other statements, such
as the tables `createDisjoint` writes. A file holds its `name`, its `text`,
each statement ending `;` on a line of its own, and `commitsAfter`: every
step but the last opens with the line `-- pgorm: commits-after`, which Rust's
pgorm-migration reads as `commits_after()`, and a step leaving rows to the
program ends with a comment naming them. `freeze(directory)` writes
`<name>.sql` into the directory only when no such file is there, and answers
whether it wrote one. So the code that froze each change can stay and run
again after the declaration has moved on: it writes what is new and keeps
what has shipped. Run it when the change is made, from a test or a small
script, and the new files join the migrations.

The bakery froze its payment tables when it opened and the voucher kind when
it added it, into [`guide/migrations/`](migrations), so freezing them again
writes nothing:

```ts
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  addDisjointKind, column, createDisjoint, disjoint, migrationFile, migrationFiles, model, TypeName,
} from "@necessary/pgorm";

const paymentKind = new TypeName("payment_kind", { schema: "public" });
const payment = (kinds: string[]) =>
  model("payment", {
    columns: {
      id: column("i32", { primaryKey: true, generated: "byDefault" }),
      kind: column(paymentKind, { values: kinds }),
      orderId: column("i32", { name: "order_id" }),
      amount: column("decimal"),
    },
  });
const CardPayment = model("card_payment", {
  columns: {
    paymentId: column("i32", { name: "payment_id", primaryKey: true }),
    lastFour: column("text", { name: "last_four" }),
  },
});
const VoucherPayment = model("voucher_payment", {
  columns: {
    paymentId: column("i32", { name: "payment_id", primaryKey: true }),
    code: column("text"),
  },
});
const opening = disjoint("payment", payment(["card", "cash"]), { card: CardPayment, cash: null }, {
  schemaName: "public",
});
const withVouchers = disjoint(
  "payment",
  payment(["card", "cash", "voucher"]),
  { card: CardPayment, cash: null, voucher: VoucherPayment },
  { schemaName: "public" },
);

// The directory this program keeps its migrations in.
const migrations = "guide/migrations";

const opened = migrationFile("m20260201_000001_create_payment", createDisjoint(opening));
const added = migrationFiles(addDisjointKind(withVouchers, "voucher"), "m20260301_000001_add_voucher");
assert.deepEqual(added.map((file) => file.name), ["m20260301_000001_add_voucher_1", "m20260301_000001_add_voucher_2"]);
assert.equal(
  added[0]!.text,
  `-- pgorm: commits-after\nALTER TYPE "public"."payment_kind" ADD VALUE 'voucher' AFTER 'cash';\n`,
);
assert.deepEqual(added.map((file) => file.commitsAfter), [true, false]);

for (const file of [opened, ...added]) {
  // Frozen when each change was made, so there is nothing to write.
  assert.equal(file.freeze(migrations), false);
}
assert.equal(readFileSync(`${migrations}/m20260301_000001_add_voucher_1.sql`, "utf8"), added[0]!.text);
```

A file is one migration for a tool that runs a directory of SQL files, each in
its transaction, such as Rust's pgorm-migration, which lists each with
`sql_migration!` ([Migrations](22-migrations.md#generated-ddl-in-a-migration)).
JavaScript cannot run such a file itself: a file of several statements is a
script, which no JavaScript terminal runs in one call ([Raw SQL](10-raw-sql.md)).

## Tables pgorm did not write

A schema written by hand may hold the same shape without a kind column: a base
table, and tables keyed by a foreign key onto it. Declare it with
`{ unkinded: true }`. A row's kind is then the kind whose table has a row for
it, and the kind declared `null`, of which there is at most one, is a row with
a row in no kind's table.

Nothing in such a schema holds a row to one kind, so pgorm reads it and never
writes it. Rust's unkinded enum has no `insert`, `update` or `change_kind`,
and no `create_disjoint`, and a call to one does not compile. JavaScript's
declaration is typed as an `UnkindedDisjoint`, which has none of them either,
and each refuses with a `ConstructionError` when it is called anyway:

```ts
import assert from "node:assert/strict";
import { column, ConstructionError, createDisjoint, disjoint, model } from "@necessary/pgorm";

await pool.execute("CREATE TABLE staff (id serial PRIMARY KEY, name text NOT NULL)");
await pool.execute("CREATE TABLE driver (staff_id integer PRIMARY KEY REFERENCES staff, licence text NOT NULL)");
await pool.execute("INSERT INTO staff (name) VALUES ('Eli'), ('Fay')");
await pool.execute("INSERT INTO driver VALUES (1, 'B-1234')");

const Staff = model("staff", {
  columns: { id: column("i32", { primaryKey: true, default: true }), name: column("text") },
});
const Driver = model("driver", {
  columns: { staffId: column("i32", { name: "staff_id", primaryKey: true }), licence: column("text") },
});
const Roles = disjoint("staff", Staff, { driver: Driver, counter: null }, { unkinded: true });

const staff = await Roles.find().orderBy(Staff.col("id").asc()).all(pool);
assert.deepEqual(staff, [
  [{ id: 1, name: "Eli" }, "driver", { staffId: 1, licence: "B-1234" }],
  [{ id: 2, name: "Fay" }, "counter", null],
]);
assert.equal(await Roles.find().kind("counter").count(pool), 1);

assert.throws(() => createDisjoint(Roles as never), ConstructionError);
```

A row with rows in two kinds' tables, or in none when no kind is `null`, is a
`DecodeError` when it is read.

## Registered enums

A Rust application's own `#[derive(Disjoint)]` enum is reached from
JavaScript through its registration, `disjointEntity(name)`, its records the
entities' own and its writes their ActiveModels, the application's hooks
included ([Registered Rust entities](24-registered-entities.md)).
