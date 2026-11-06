# Transactions

A transaction groups statements so that they take effect together or not at
all. Placing an order writes the order and each of its lines; a transaction
makes sure no one ever sees an order with half its lines, and that a failure
part-way leaves nothing behind.

In pgorm a transaction exists only while a callback runs. You hand
`transaction` an async function, and how that function settles ends the
transaction: resolving commits it, throwing rolls it back. There is no handle
to commit later and no transaction to forget to end. This chapter is the
JavaScript dialect of the [Rust guide's
chapter](../../docs/guide/07-transactions.md) of the same name, where a
closure's `Ok` and `Err` play the parts that resolving and throwing play here.

## Running a callback in a transaction

[`transaction`](../lib/index.d.ts) is a method of a pool, a connection and a
transaction. On a pool, it takes a connection of its own for the
transaction's whole life and returns it once the transaction ends. The
callback receives a [`Transaction`](../lib/index.d.ts), which runs statements
as a pool or connection does, and every statement run on it is part of the
transaction. `transaction` resolves with what the callback resolved with:

```ts
import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";

const placedAt = Temporal.Instant.from("2026-11-04T10:00:00Z");
const order = await pool.transaction(async (tx) => {
  const order = await Order.insert({ customerId: 2, bakeryId: 1, placedAt }).returning().one(tx);
  await OrderLine.insertMany([
    { orderId: order.id, cakeId: 5, quantity: 1, price: new Decimal("6.00") },
    { orderId: order.id, cakeId: 2, quantity: 2, price: new Decimal("3.20") },
  ]).execute(tx);
  return order;
});

const lines = await OrderLine.find().where(OrderLine.col("orderId").eq(order.id)).count(pool);
assert.deepEqual([order.id, lines], [3, 2]);
```

Rust's `transaction` takes the connection by `&mut`, so the connection has to
be mutable, and the closure's error type has to be spelled out. JavaScript has
neither: any value the callback throws rolls the transaction back.

While the transaction is open it holds its connection exclusively. Nothing
else can run on that connection until the callback has settled. Rust's
compiler refuses code that uses the connection inside the closure; JavaScript
refuses it as it runs, with a [`LifecycleError`](../lib/index.d.ts), never by
queueing the statement or racing it:

```ts
import assert from "node:assert/strict";
import { LifecycleError } from "@necessary/pgorm";

await using db = await pool.acquire();
await db.transaction(async (tx) => {
  // The connection is the transaction's for now: use `tx`.
  await assert.rejects(Cake.find().all(db), LifecycleError);
  assert.equal(await Cake.find().count(tx), 5);
});
```

## Rolling back

When the callback throws, the transaction rolls back, and the rollback has
finished by the time `transaction` rejects. It rejects with the callback's own
error, so a caller catches the error it threw, as it would without a
transaction around it. Rust wraps the closure's error in a `TransactionError`
whose variant says which part failed; JavaScript keeps the three apart by
what it rejects with:

| Rejects with | What failed |
| --- | --- |
| what the callback threw | the callback, and the transaction was rolled back |
| the failure itself, a [`DatabaseError`](../lib/index.d.ts) or [`ConnectionError`](../lib/index.d.ts) | the transaction itself: its `BEGIN` or its `COMMIT`, the `COMMIT` of a transaction a caught failure aborted among them ([A caught error still aborts](#a-caught-error-still-aborts)) |
| a `SuppressedError` | the callback threw, and then the `ROLLBACK` failed: `error` is the rollback's failure and `suppressed` what the callback threw |

Here the second line names a cake that does not exist, so its insert fails,
and the order written before it is rolled back with it:

```ts
import assert from "node:assert/strict";
import { Decimal, ForeignKeyViolationError } from "@necessary/pgorm";

const placedAt = Temporal.Instant.from("2026-11-04T10:00:00Z");
await assert.rejects(
  pool.transaction(async (tx) => {
    const order = await Order.insert({ customerId: 1, bakeryId: 1, placedAt }).returning().one(tx);
    await OrderLine.insert({ orderId: order.id, cakeId: 99, quantity: 1, price: new Decimal("1.00") }).execute(tx);
    return order;
  }),
  ForeignKeyViolationError,
);

assert.equal(await Order.find().count(pool), 2);
```

A failed `COMMIT` or `ROLLBACK`, an aborted transaction's apart, means that
nobody knows how the transaction ended, nor what state its connection is in. The binding discards that
connection itself, closing it rather than returning it to its pool, so
nothing else is lent a connection in that state. Rust leaves the choice to the
program, which gives such a connection back with `discard`; JavaScript has no
`discard`, because every case in which a connection's state is unknown
discards it already.

## Savepoints

A transaction has a `transaction` method of its own, which opens a savepoint:
a transaction nested inside the first, which commits into it or rolls back
alone. A savepoint's failure is a rejection like any other, so the callback
around it catches the failures it can live with and lets the rest end the
whole transaction, as Rust's `?` passes a savepoint's `TransactionError` on.
Here the order survives a line naming a cake that does not exist, and any
other failure ends the whole order:

```ts
import assert from "node:assert/strict";
import { Decimal, ForeignKeyViolationError } from "@necessary/pgorm";

const placedAt = Temporal.Instant.from("2026-11-04T10:00:00Z");
const { order, skipped } = await pool.transaction(async (tx) => {
  const order = await Order.insert({ customerId: 1, bakeryId: 2, placedAt }).returning().one(tx);
  const skipped: number[] = [];
  for (const cakeId of [3, 99, 4]) {
    try {
      await tx.transaction((savepoint) =>
        OrderLine.insert({ orderId: order.id, cakeId, quantity: 1, price: new Decimal("3.00") }).execute(savepoint)
      );
    } catch (error) {
      // No such cake: the savepoint rolled the line back, and the order goes
      // on without it. Anything else is the order's failure too.
      if (!(error instanceof ForeignKeyViolationError)) throw error;
      skipped.push(cakeId);
    }
  }
  return { order, skipped };
});

const lines = await OrderLine.find().where(OrderLine.col("orderId").eq(order.id)).count(pool);
assert.deepEqual([lines, skipped], [2, [99]]);
```

A savepoint whose `ROLLBACK TO SAVEPOINT` fails rejects with a
`SuppressedError`, the rollback's failure as its `error` and the callback's as
its `suppressed`, where Rust's becomes `Error::RollbackFailed`. The enclosing
transaction is then in a state nobody knows; letting the rejection through
rolls it back too.

Without the savepoint, the failed insert would have aborted the whole
transaction: PostgreSQL refuses every later statement in a transaction one
statement has failed in, until it rolls back.

## A caught error still aborts

Catching a failed statement's error inside the callback does not undo what
the failure did to the transaction. PostgreSQL has aborted it, and answers its
`COMMIT` by rolling it back. pgorm asks before it commits, so a callback that
returns after catching such an error does not resolve `transaction`: it
rejects with a `DatabaseError` whose `sqlstate` is `25P02`, saying the
transaction was rolled back, with nothing it wrote kept:

```ts
import assert from "node:assert/strict";
import { DatabaseError, ForeignKeyViolationError } from "@necessary/pgorm";

const placedAt = Temporal.Instant.from("2026-11-04T10:00:00Z");
await assert.rejects(
  pool.transaction(async (tx) => {
    await Order.insert({ customerId: 1, bakeryId: 1, placedAt }).execute(tx);
    try {
      await Order.insert({ customerId: 99, bakeryId: 1, placedAt }).execute(tx);
    } catch (error) {
      // A mistake: the callback carries on.
      if (!(error instanceof ForeignKeyViolationError)) throw error;
    }
  }),
  (error) =>
    error instanceof DatabaseError && error.sqlstate === "25P02" &&
    error.message.startsWith("the transaction was rolled back, not committed"),
);

// Neither order was written.
assert.equal(await Order.find().count(pool), 2);
```

The connection is in a known state, so it goes back to its pool as usual. A
savepoint whose callback returns after catching an error rejects with the
same `DatabaseError`, and the savepoint is rolled back to, so the transaction
around it carries on unless the rejection leaves its callback too. Put the
statement that may fail in a savepoint of its own, as above, and catch its
error outside the savepoint's callback.

Rust returns this as `TransactionError::Connection(Error::TransactionAborted)`;
JavaScript rejects with the `DatabaseError` it carries, its message pgorm's
account of the rollback rather than the server's.

A savepoint borrows its transaction as a transaction borrows its connection.
While the savepoint is open, its transaction refuses statements and other
savepoints with a `LifecycleError`, and the refusal leaves both open:

```ts
import assert from "node:assert/strict";
import { LifecycleError } from "@necessary/pgorm";

await pool.transaction(async (tx) => {
  await tx.transaction(async (savepoint) => {
    assert.equal(savepoint.parent, tx);
    await assert.rejects(tx.query("SELECT 1"), LifecycleError);
    assert.equal(await Cake.find().count(savepoint), 5);
  });
});
```

## Choosing the isolation level and access mode

The options of `transaction` on a pool or a connection open the transaction
in a mode PostgreSQL acts on. `mode` is one of:

| `mode` | Opens |
| --- | --- |
| `"default"`, when left out | a transaction under the session's defaults |
| `"readWrite"` | `READ WRITE`, at the `isolation` given or the session's |
| `"readOnly"` | `READ ONLY`, in which every write is refused |
| `"deferrable"` | `SERIALIZABLE READ ONLY DEFERRABLE`: it may wait for a safe snapshot, and then no serialization failure can end it |

`isolation` is `"readUncommitted"`, `"readCommitted"`, `"repeatableRead"` or
`"serializable"`, and goes only with `"readWrite"` or `"readOnly"`. These are
Rust's `TransactionMode` and `IsolationLevel`, spelled as strings. A
savepoint takes neither, so a transaction's own `transaction` has no such
options, as Rust's `transaction_with` is a method of `DatabaseConnection`
alone.

```ts
import assert from "node:assert/strict";
import { DatabaseError } from "@necessary/pgorm";

const report = { mode: "readOnly", isolation: "repeatableRead" } as const;

// Two reads that see the same snapshot of the database.
const [cakes, total] = await pool.transaction(async (tx) => {
  const cakes = await Cake.find().count(tx);
  const { total } = await tx.one("SELECT sum(price) AS total FROM cake");
  return [cakes, String(total)];
}, report);
assert.deepEqual([cakes, total], [5, "20.40"]);

// A write in a read-only transaction is refused.
await assert.rejects(
  pool.transaction((tx) => Cake.delete().where(Cake.col("id").eq(1)).execute(tx), report),
  (error) => error instanceof DatabaseError && error.sqlstate === "25006",
);
```

## Retrying serializable transactions

Under `SERIALIZABLE` and `REPEATABLE READ`, PostgreSQL refuses a transaction
whose result another concurrent transaction has made inconsistent, with a
serialization failure (SQLSTATE `40001`), and expects the client to run it
again. It does the same to one side of a deadlock (`40P01`).

**Not available in JavaScript yet:** running a transaction again while it
fails with one of those two errors, which Rust's `transaction_with_retry`
does: from `BEGIN`, up to a number of retries the program gives, asking the
error whether it is worth a retry (`RetryableError`, `Error::is_retryable`).

Until it is, the program writes the loop itself, reading each failure's
`sqlstate`. The callback may then run more than once, so anything it does
outside the database happens again on each attempt. Here another connection
changes the cake's price after the first attempt has read it, and the first
attempt's update is refused; the second attempt reads the new price and
succeeds:

```ts
import assert from "node:assert/strict";
import { DatabaseError, Decimal } from "@necessary/pgorm";

const retryable = (error: unknown) =>
  error instanceof DatabaseError && (error.sqlstate === "40001" || error.sqlstate === "40P01");

await using other = await pool.acquire();
let attempts = 0;
let doubled: Decimal | undefined;
for (let retries = 3; doubled === undefined; retries--) {
  try {
    doubled = await pool.transaction(async (tx) => {
      attempts += 1;
      const fudge = await Cake.findByKey({ id: 1 }).one(tx);
      if (attempts === 1) {
        // Someone else's change commits after this transaction read the row.
        await other.execute("UPDATE cake SET price = price + 1 WHERE id = 1");
      }
      const price = new Decimal(String(Number(fudge.price.toString()) * 2));
      const changed = await Cake.update({ price }).where(Cake.key({ id: 1 })).returning("price").one(tx);
      return changed.price;
    }, { mode: "readWrite", isolation: "serializable" });
  } catch (error) {
    if (!retryable(error) || retries === 0) throw error;
  }
}
assert.deepEqual([attempts, String(doubled)], [2, "11.00"]);
```

`Number(..)` is safe here only because the prices have two places and are
small; [Values and types](13-values.md) says why a `Decimal` never becomes a
number by itself.

## An aborted transaction rolls back

A callback cannot be dropped half way, as a Rust future can when a timeout
fires. What stops a JavaScript transaction early is an `AbortSignal` given to
one of its statements. Aborting a statement leaves its outcome unknown, so
the binding ends the whole transaction at once, with nothing committed, and
discards its connection. The statement rejects with the signal's reason, and
so does `transaction`, unless the callback catches it and throws something
else:

```ts
import assert from "node:assert/strict";

await assert.rejects(
  pool.transaction(async (tx) => {
    await Customer.insert({ name: "Mo", email: "mo@example.com" }).execute(tx);
    await tx.execute("SELECT pg_sleep(30)", [], { signal: AbortSignal.timeout(100) });
  }),
  (error) => error instanceof DOMException && error.name === "TimeoutError",
);

// Mo's insert was rolled back with the transaction.
assert.equal(await Customer.find().count(pool), 2);
```

The options of `transaction` take a `signal` too, which aborts opening the
transaction. [Promises, cancellation and cleanup](02-promises.md) covers
signals, and what each kind of handle does when one fires.

## Transactions that run at the same time

Rust's section here is about running a transaction inside a spawned task,
whose future must be `Send`. JavaScript has no such bound: every transaction
is a Promise, and transactions run at the same time whenever their Promises
do. Each `transaction` on a pool takes a connection of its own, so two of
them are two sessions:

```ts
import assert from "node:assert/strict";

const [noon, dusk] = await Promise.all([
  pool.transaction((tx) => Bakery.insert({ name: "Noon", city: "Tromsø" }).returning().one(tx)),
  pool.transaction((tx) => Bakery.insert({ name: "Dusk", city: "Bodø" }).returning().one(tx)),
]);

assert.deepEqual([noon.name, dusk.name], ["Noon", "Dusk"]);
assert.deepEqual([noon.id, dusk.id].sort(), [3, 4]);
```

Two transactions on one connection cannot run at the same time: a second
`transaction` on a connection is refused with a `LifecycleError` while the
first holds it.

Rust's [metrics](21-metrics.md) wrappers report every transaction to a
collector; that chapter says what JavaScript has in their place.
