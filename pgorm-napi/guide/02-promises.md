# Promises, cancellation and cleanup

*This chapter is JavaScript's own: the Rust guide has no counterpart of it.*
It says how every operation of the package settles, how one is stopped
before it finishes, and how what a pool, a connection or a stream holds is
released. The other chapters rely on it.

## Every operation is a Promise

pgorm's work happens off the JavaScript thread. The addon runs it on a
runtime of its own, and every operation that waits on the server or the pool
returns a Promise: it resolves with the result, or rejects with an error, a
subclass of `PgormError` ([Errors](09-errors.md)). Building a statement waits
on nothing, so a builder returns its result at once and throws where its
input is wrong.

A pool lends a connection to each statement it runs, so statements on a pool
run side by side, each on a connection of its own:

```ts
import assert from "node:assert/strict";

const [cakes, bakers] = await Promise.all([
  pool.one("SELECT count(*) AS n FROM cake"),
  pool.one("SELECT count(*) AS n FROM baker"),
]);
assert.deepEqual([cakes.n, bakers.n], [5n, 3n]);
```

A [`Connection`](../lib/index.d.ts), a [`Transaction`](../lib/index.d.ts) and
a stream run one operation at a time. Another started while one runs is
refused with a `LifecycleError` rather than queued behind it, since a queue
would hide which statement ran in which state:

```ts
import assert from "node:assert/strict";
import { LifecycleError } from "@necessary/pgorm";

await using db = await pool.acquire();
const slow = db.execute("SELECT pg_sleep(0.2)");
await assert.rejects(db.execute("SELECT 1"), LifecycleError);
await slow;

// Once the first has settled, the connection takes the next.
assert.equal((await db.one("SELECT 1 AS one")).one, 1);
```

Rust holds a program to the same rule at compile time: a connection is
borrowed by the statement running on it, and a transaction borrows its
connection exclusively. JavaScript checks it when the second operation
starts.

## Stopping an operation

An operation that waits on the server or the pool takes an `AbortSignal` as
its `signal` option: acquiring a connection, running a statement, opening a
transaction or a savepoint, pulling a stream's rows, a `COPY`, a listener's
operations and `notify`. Aborting rejects the operation with the signal's
reason, so `AbortSignal.timeout(ms)` makes a timeout that rejects with a
`DOMException` named `TimeoutError`:

```ts
import assert from "node:assert/strict";

await assert.rejects(
  pool.execute("SELECT pg_sleep(10)", [], { signal: AbortSignal.timeout(100) }),
  (error: unknown) => error instanceof DOMException && error.name === "TimeoutError",
);
```

An operation stopped part way has an outcome no one knows: a write may or may
not have happened. So the connection it ran on is discarded rather than
returned to the pool, and a transaction it ran in ends with nothing
committed. That holds even where the work had finished just before the abort:
a promise whose signal fired rejects with the signal's reason, and its result
is thrown away.

```ts
import assert from "node:assert/strict";

const stop = new AbortController();
const order = pool.transaction(async (tx) => {
  await tx.execute(`INSERT INTO "order" (customer_id, bakery_id, placed_at) VALUES (1, 2, now())`);
  stop.abort(new Error("the customer left"));
  await tx.execute("SELECT pg_sleep(10)", [], { signal: stop.signal });
});
await assert.rejects(order, { message: "the customer left" });

// The insert went with the transaction.
assert.equal((await pool.one(`SELECT count(*) AS n FROM "order"`)).n, 2n);
```

Rust stops an operation by dropping its future, as `tokio::time::timeout`
does when it fires, and a transaction whose future is dropped rolls back as
its handle is dropped ([Transactions](08-transactions.md)). A signal is
JavaScript's way to say the same.

## Releasing what a handle holds

A pool holds connections; a connection checked out holds one of them; a
stream, a `COPY` and a listener hold the connection they read from. Each has
`close()`, and each works with `await using`, which closes it when the block
that declared it ends, however the block ends:

- Closing a **connection** returns it to its pool at once, ending a
  transaction or stream that holds it.
- Closing a **pool** refuses new work, cancels what runs, and resolves once
  every connection and listener is released.
- A **stream** is an async iterator. Its last row frees its connection;
  closing it before then, which a `for await` loop does when it leaves early,
  discards the connection, because rows were left unread on it.
- A **transaction** has no `close()`: it ends when its callback settles
  ([Transactions](08-transactions.md)).

A handle used after it closed is refused with a `LifecycleError`:

```ts
import assert from "node:assert/strict";
import { LifecycleError } from "@necessary/pgorm";

const db = await pool.acquire();
await db.close();
assert.equal(db.closed, true);
await assert.rejects(db.execute("SELECT 1"), LifecycleError);

// A loop that leaves early closes its stream.
let first: string | undefined;
for await (const cake of pool.stream("SELECT name FROM cake ORDER BY id")) {
  first = String(cake.name);
  break;
}
assert.equal(first, "Chocolate Fudge");
```

Nothing waits on garbage collection. A handle the program drops without
closing it releases, once it is collected, only what is idle: an idle
transaction is rolled back, an idle connection returned to its pool, a
listener's connection closed. It never cuts short work still running on what
it held. Close what you open, and let `await using` do it where it can.

## The process exits by itself

Loading the addon does not keep a process alive, and neither does an idle
pool. A script whose work has settled exits on its own, in Node.js and in
Deno, with its ordinary status, even with a pool still open: an open pool
holds idle connections and keeps nothing waiting. An operation still in
flight when the script's last statement runs keeps the process up until it
settles. A process that exits explicitly while operations run, with
`process.exit()` or `Deno.exit()`, neither waits for them nor crashes, and a
rejection nothing handles ends the process as any unhandled rejection does.
