# Errors

Every operation that can fail rejects, or throws as it is called, with an
error that says what failed in one of three places: the database refused a
statement, a value could not cross between JavaScript and PostgreSQL, or
pgorm refused something before sending anything.

Each failure pgorm reports is an instance of a subclass of
[`PgormError`](../lib/index.d.ts), and the class is the kind of failure. Rust
reports every failure as one `pgorm::Error` enum, whose variants and
classifiers a program matches on; JavaScript tells them apart with
`instanceof`. An argument of a type the call does not take, such as a number
where a name belongs, is JavaScript's own `TypeError`, thrown as the call is
made. This chapter is the JavaScript dialect of the [Rust guide's
chapter](../../docs/guide/08-errors.md), and goes through each kind.

| Class | What failed |
| --- | --- |
| [`DatabaseError`](../lib/index.d.ts) | the server refused a statement, with its SQLSTATE and diagnostics |
| [`ConnectionError`](../lib/index.d.ts) | the server could not be reached, or the connection to it broke |
| [`ConstructionError`](../lib/index.d.ts) | an argument could not become what pgorm sends: a value its type cannot hold, a statement that cannot be built |
| [`DecodeError`](../lib/index.d.ts) | a result could not become the JavaScript value asked for, or held another number of rows than the terminal admits |
| [`LifecycleError`](../lib/index.d.ts) | a pool, connection, transaction or stream was used after it closed, or while another operation held it |
| [`TimeoutError`](../lib/index.d.ts) | no connection came free within the pool's `acquireTimeout` |
| [`InternalError`](../lib/index.d.ts) | pgorm or the binding failed in a way no input should cause, such as a panic in its native code |

A message never holds the credentials of the connection string: a refused
password names the user as `[redacted]`.

## What the database refused

A statement the server rejects rejects with a `DatabaseError`, holding what
the server reported: `sqlstate`, its five-character code; `severity`; the
message; and `detail`, `hint`, `schema`, `table`, `column`, `datatype` and
`constraint` where the server named them, `null` otherwise.

The refusals a program most often has to answer are constraint violations: a
duplicate email address, an order line for a cake that does not exist. Each
rejects with a subclass of `DatabaseError` named for it, where Rust's
`sql_error` classifies the error as a `SqlError` variant:

| Class | SQLSTATE | Raised by |
| --- | --- | --- |
| [`UniqueViolationError`](../lib/index.d.ts) | `23505` | a duplicate in a unique key or primary key |
| [`ForeignKeyViolationError`](../lib/index.d.ts) | `23503` | a reference to a row that does not exist, or a delete of a row still referenced |
| [`CheckViolationError`](../lib/index.d.ts) | `23514` | a table's or a domain's `CHECK` |
| [`NotNullViolationError`](../lib/index.d.ts) | `23502` | a `NULL` in a `NOT NULL` column or domain |
| [`ExclusionViolationError`](../lib/index.d.ts) | `23P01` | an `EXCLUDE` constraint, or a key `WITHOUT OVERLAPS` |

A table's constraint names its `table` and `constraint`; a `NOT NULL` column
names its `table` and `column` and no constraint; a domain's names the domain
as `datatype`.

```ts
import assert from "node:assert/strict";
import { Decimal, ForeignKeyViolationError, NotNullViolationError, UniqueViolationError } from "@necessary/pgorm";

// Lee's address is taken.
const duplicate = await Customer.insert({ name: "Lee Two", email: "lee@example.com" })
  .execute(pool)
  .catch((error: unknown) => error);
assert.ok(duplicate instanceof UniqueViolationError);
assert.equal(duplicate.table, "customer");
assert.equal(duplicate.constraint, "customer_email_key");

// There is no bakery 99.
const orphan = await Cake.insert({
  name: "Ghost Cake",
  flavor: "vanilla",
  price: new Decimal("1.00"),
  glutenFree: false,
  bakeryId: 99,
}).execute(pool).catch((error: unknown) => error);
assert.ok(orphan instanceof ForeignKeyViolationError);
assert.equal(orphan.constraint, "fk-cake-bakery_id");

// A cake left without a name.
const nameless = await pool
  .execute("INSERT INTO cake (flavor, price, gluten_free, bakery_id) VALUES ('vanilla', 1.00, false, 1)")
  .catch((error: unknown) => error);
assert.ok(nameless instanceof NotNullViolationError);
assert.equal(nameless.column, "name");
```

The model refuses the nameless cake before the server can: `Cake.insert`
requires every field that is neither nullable nor defaulted, so leaving `name`
out of it does not type-check ([TypeScript](26-typescript.md)), which is why
the example writes that insert as SQL.

A check and an exclusion constraint answer the same way. The bakery's ovens
are booked by time, and an exclusion constraint keeps two bookings of one
oven from overlapping:

```ts
import assert from "node:assert/strict";
import { CheckViolationError, Decimal, ExclusionViolationError } from "@necessary/pgorm";

await pool.batchExecute(`
  ALTER TABLE cake ADD CONSTRAINT cake_price_positive CHECK (price > 0);
  CREATE EXTENSION IF NOT EXISTS btree_gist;
  CREATE TABLE oven_booking (
    oven integer NOT NULL,
    during tstzrange NOT NULL,
    EXCLUDE USING gist (oven WITH =, during WITH &&)
  )`);

const free = await pool
  .execute("UPDATE cake SET price = $1 WHERE id = 1", [new Decimal("0")])
  .catch((error: unknown) => error);
assert.ok(free instanceof CheckViolationError);
assert.equal(free.constraint, "cake_price_positive");

const book = "INSERT INTO oven_booking (oven, during) VALUES ($1, tstzrange($2, $3))";
const at = (time: string) => Temporal.Instant.from(`2026-11-04T${time}:00Z`);
await pool.execute(book, [1, at("06:00"), at("08:00")]);
await assert.rejects(pool.execute(book, [1, at("07:00"), at("09:00")]), ExclusionViolationError);
```

The three statements run as one script with `batchExecute`, as Rust runs
them with `batch_execute` ([Raw SQL](10-raw-sql.md)).

Any other refusal is read off the server's SQLSTATE. Rust compares the code
with tokio-postgres's `SqlState` constants, such as `SqlState::UNDEFINED_TABLE`,
and reads the server's whole report from `as_db_error`; JavaScript's
`DatabaseError` is that report, its `sqlstate` the five-character code:

```ts
import assert from "node:assert/strict";
import { DatabaseError } from "@necessary/pgorm";

const missing = await pool.query("SELECT * FROM pastry").catch((error: unknown) => error);
assert.ok(missing instanceof DatabaseError);
// 42P01 is undefined_table.
assert.equal(missing.sqlstate, "42P01");
assert.equal(missing.severity, "ERROR");
```

Every error's `isRetryable` answers whether it is one PostgreSQL expects the
client to retry the whole transaction after, as Rust's `is_retryable` does:
true for a serialization failure (`40001`) or a deadlock (`40P01`), and false
for every other, an error the program made among them.
[Transactions](08-transactions.md) shows `transactionWithRetry`, which reads
it:

```ts
import assert from "node:assert/strict";
import { DatabaseError, PgormError } from "@necessary/pgorm";

const missing = await pool.query("SELECT * FROM pastry").catch((error: unknown) => error);
assert.ok(missing instanceof DatabaseError);
assert.equal(missing.isRetryable, false);
assert.equal(new PgormError("made here").isRetryable, false);
```

## What could not cross

Some failures happen on the client, without the server refusing anything. A
value can be refused on its way in, because the parameter's type cannot hold
it, or on its way out, because JavaScript has no value that holds the
column's exactly. Rust reports both as driver errors, each with a classifier:
`bind_refusal` says which `$N` parameter was refused, and `decode_refusal`
what was being read. JavaScript reports them as a `ConstructionError` and a
`DecodeError`, whose messages say the same and whose fields are the
classifiers': a refused parameter's `parameter`, its one-based `$N`, and
`reason`; and an unread result's `reading` and `reason`. Any other
`ConstructionError` or `DecodeError` holds `null` in each:

```ts
import assert from "node:assert/strict";
import { ConstructionError, DecodeError } from "@necessary/pgorm";

// The parameter is a smallint, and 100000 is not one.
const tooWide = await pool.execute("SELECT $1::smallint", [100_000]).catch((error: unknown) => error);
assert.ok(tooWide instanceof ConstructionError);
assert.match(tooWide.message, /^parameter \$1 cannot be bound/);
assert.equal(tooWide.parameter, 1);
assert.equal(tooWide.reason, "value `100000` is out of range for Postgres type `int2`");

// No Temporal type is a time of day at a fixed offset.
const unreadable = await pool.one("SELECT '12:00+01'::timetz AS opens").catch((error: unknown) => error);
assert.ok(unreadable instanceof DecodeError);
assert.match(unreadable.message, /column "opens" of type `timetz` cannot be decoded/);
```

A row of SQL text decodes each column by the type the server reports for it,
so text always reads as a string and never as a number. Where a model reads a
result, every column is also held to the model's declaration: a column of
another type, or a `NULL` where the field is not nullable, is a `DecodeError`
naming the field, where Rust reports a `NULL` for a field that is not an
`Option` as `Error::Type`:

```ts
import assert from "node:assert/strict";
import { column, DecodeError, model } from "@necessary/pgorm";

// A model that forgets that a customer's notes may be missing.
const Contact = model("customer", {
  columns: { id: column("i32", { primaryKey: true }), notes: column("text") },
});
await assert.rejects(Contact.find().all(pool), (error) =>
  error instanceof DecodeError && error.message === "notes is not nullable, and the row holds NULL");
```

## What pgorm refused

The other failures are pgorm's own. Most say that a call could not do what it
promised, before or instead of sending a statement. Each of Rust's variants
has its JavaScript counterpart, or none where JavaScript cannot reach the
state it reports:

| Rust | JavaScript |
| --- | --- |
| `RecordNotFound` | a `DecodeError` from `one`, which wants exactly one row; `optional` answers `null` instead |
| `RecordNotInserted` | an upsert terminal's `null`, or a `DecodeError` from a registered entity's `returningPk` ([Writing rows](04-writing.md)) |
| `PrimaryKeyNotSet` | none: a model's update and delete name their rows with `where`, and refuse to run without it or `allRows()` as a `ConstructionError` |
| `NothingToSet` | a `ConstructionError` from an update that sets no field, when it is built |
| `AttrNotSet` | none: a write is a plain object, never converted into a record |
| `Type` | a `DecodeError` for a value a field cannot hold, or a `ConstructionError` for a value a parameter cannot |
| `Json`, `Conversion`, `UnpackInsertId` | a `ConstructionError` going in, or a `DecodeError` coming out |
| `RollbackFailed` | a `SuppressedError` from `transaction(fn)` or `transactionWithRetry(fn)`: the rollback's failure as its `error`, and what the callback threw as its `suppressed` ([Transactions](08-transactions.md)) |
| `Query` | a `DecodeError` for a read that cannot be carried out as asked, such as a relation's key that matches no row |
| `Pool` | a `ConnectionError` when a connection cannot be opened, a `DatabaseError` when the server refuses to open one, a `TimeoutError` when none comes free in time |
| `Verify` | a `DecodeError` from `verify`, naming the column at fault ([Raw SQL](10-raw-sql.md)) |
| `QueryBuilder` | a `ConstructionError`, thrown as a builder is called or when a statement is inspected or run |
| `Listen` | a `ConstructionError` for a channel name that is too long, and a `ConnectionError` for a listener that fell behind ([LISTEN and NOTIFY](18-listen-notify.md)) |
| `Custom` | a `ConstructionError` carrying a registered entity's hook's refusal ([Registered Rust entities](24-registered-entities.md)) |

```ts
import assert from "node:assert/strict";
import { ConstructionError, DecodeError } from "@necessary/pgorm";

// No cake 99.
await assert.rejects(Cake.findByKey({ id: 99 }).one(pool), DecodeError);
assert.equal(await Cake.findByKey({ id: 99 }).optional(pool), null);

// An update or delete names its rows, or says it means every row.
assert.throws(() => Customer.update({ name: "Kim" }).inspect(), ConstructionError);

// An update that sets nothing.
assert.throws(() => Cake.update({}), ConstructionError);

// Five rows, where `one` wants exactly one.
await assert.rejects(pool.one("SELECT name FROM cake"), DecodeError);
```

A pool opens its connections when they are first needed, so a pool that
cannot open one reports it at the first operation, as Rust's reports it at
`get`. What it rejects with says why: the server refusing the database is a
`DatabaseError`, a server that cannot be reached a `ConnectionError`, and a
pool that has no connection free in time a `TimeoutError`:

```ts
import assert from "node:assert/strict";
import { DatabaseError, Pool, TimeoutError } from "@necessary/pgorm";

const elsewhere = new URL(url);
elsewhere.pathname = "/no_such_bakery";
await using missing = new Pool(elsewhere.toString());
await assert.rejects(missing.one("SELECT 1"), (error) =>
  error instanceof DatabaseError && error.sqlstate === "3D000");

// One connection, held: the next operation waits 200 ms for it, then gives up.
await using small = new Pool(url, { maxSize: 1, acquireTimeout: 200 });
await using held = await small.acquire();
await assert.rejects(small.one("SELECT 1"), TimeoutError);
```

A pool's `TimeoutError` is pgorm's; an `AbortSignal` that times out rejects
with the signal's own `TimeoutError`, a `DOMException` of that name
([Promises, cancellation and cleanup](02-promises.md)).

## Matching errors

Each class above is exported, so a program matches a failure with
`instanceof`, most specific first: a `UniqueViolationError` is a
`DatabaseError` and a `PgormError` too. Kinds of failure may be added, as
Rust marks its error enums `#[non_exhaustive]`, so code that handles some
kinds lets the rest through:

```ts
import assert from "node:assert/strict";
import { DatabaseError, PgormError, UniqueViolationError } from "@necessary/pgorm";

async function signUp(name: string, email: string): Promise<"welcome" | "known"> {
  try {
    await Customer.insert({ name, email }).execute(pool);
    return "welcome";
  } catch (error) {
    if (error instanceof UniqueViolationError) return "known";
    throw error;
  }
}

assert.equal(await signUp("Mo", "mo@example.com"), "welcome");
assert.equal(await signUp("Kim again", "kim@example.com"), "known");

const refused = await pool.query("SELECT * FROM pastry").catch((error: unknown) => error);
assert.ok(refused instanceof DatabaseError && refused instanceof PgormError);
assert.equal(refused.name, "DatabaseError");
```

A callback run in a transaction rejects with its own error, which
`transaction` passes on unchanged, beside the failures of the transaction
itself; [Transactions](08-transactions.md) covers them.
