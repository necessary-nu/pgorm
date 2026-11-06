# LISTEN and NOTIFY

PostgreSQL can tell a session when something happens. A session that has run
`LISTEN orders` receives a *notification* each time any session sends one on
the `orders` channel, with a text payload. It is how one process wakes
another: a new order is placed, and the kitchen's display hears about it at
once instead of polling the table. This chapter is the JavaScript dialect of
the [Rust guide's chapter](../../docs/guide/17-listen-notify.md) of the same
name.

A notification is delivered to the session that listened, outside the result
of any statement. A pooled connection cannot receive one: the pool's task
that drives it discards such messages, and the pool lends the connection to
someone else once it is returned. So a listener is a connection of its own,
opened for the purpose and owned by the listener alone.

## Listening

`pool.listener()` opens a listening connection with the pool's configuration
and TLS. The connection is not taken from the pool and does not count against
its size, and it closes with the pool. It resolves with a
[`Listener`](../lib/listen.d.ts), which is both halves of Rust's pair in one
object:

- it chooses the channels, with `listen`, `unlisten` and `unlistenAll`;
- it is an async iterator of each [`Notification`](../lib/listen.d.ts) the
  connection receives, a frozen `{ channel, payload, processId }`, the last
  the process id of the session that sent it, one per `next()`.

`notify(channel, payload)` on a pool, a connection or a transaction sends a
notification:

```ts
import assert from "node:assert/strict";

await using listener = await pool.listener();
await listener.listen("orders");

await pool.notify("orders", "order 3 placed");

const heard = await listener.next({ signal: AbortSignal.timeout(5000) });
assert.ok(!heard.done);
assert.equal(heard.value.channel, "orders");
assert.equal(heard.value.payload, "order 3 placed");
```

The listener's connection is driven by the addon, which queues each
notification as it arrives, so `listen` and `unlisten` complete whether or
not anyone is reading, and may run while a pull waits. One pull runs at a
time; a second meanwhile is a `LifecycleError`. A pull takes an
`AbortSignal`, and an aborted pull leaves the listener open, so the
notification it waited for is the next pull's ([Promises, cancellation and
cleanup](02-promises.md)). `close()`, `await using` and a `for await` loop
that leaves early close the listener and its connection; after that `listen`
and `unlisten` reject with a `LifecycleError`, and a pull is done.

Once `listen` resolves, every notification on the channel from a transaction
that commits afterwards is delivered. Once `unlisten` resolves, none is,
though one already received is still waiting to be read.

A listener can also belong to no pool:
[`connectListener(dsn, options)`](../lib/listen.d.ts) opens one on a
connection of its own, as Rust's `connect_listener` does, reading the
connection string, `connectTimeout` and the TLS options as a pool's
([Connection configuration and TLS](20-configuration.md)) and `capacity` as
`listener()`'s. Only its own close, or its iteration left early, closes it.

```ts
import assert from "node:assert/strict";
import { connectListener, LifecycleError } from "@necessary/pgorm";

// `url` names the database; no pool holds this listener.
const listener = await connectListener(url, { capacity: 64 });
await listener.listen("deliveries");
await pool.notify("deliveries", "flour");
const { value } = await listener.next();
assert.equal(value?.payload, "flour");

// Closing the listener closes its connection.
await listener.close();
await assert.rejects(listener.listen("deliveries"), LifecycleError);
```

## Channel names

A channel name is exact. `listen` sends it as a quoted identifier, so its
case and punctuation are kept: `Orders` and `orders` are two channels, and
the name is the same text `notify` sends to. PostgreSQL keeps 63 bytes of an
identifier and would quietly listen on a shortened name nothing can send to,
so a longer name is refused with a `ConstructionError` before anything is
sent, where Rust's is `ListenError::ChannelTooLong`:

```ts
import assert from "node:assert/strict";
import { ConstructionError } from "@necessary/pgorm";

await using listener = await pool.listener();
await listener.listen("Orders");

await pool.notify("orders", "not heard: another channel");
await pool.notify("Orders", "heard");
const heard = await listener.next({ signal: AbortSignal.timeout(5000) });
assert.ok(!heard.done);
assert.equal(heard.value.payload, "heard");

await assert.rejects(listener.listen("o".repeat(64)), ConstructionError);
```

## Notifications and transactions

`notify` runs `SELECT pg_notify($1, $2)` with the channel and payload bound
as parameters, so neither is ever written into the SQL, and the statement is
prepared once per connection. `NOTIFY` itself takes no parameters, which is
why pgorm does not send it.

A notification is delivered when the transaction that sent it commits: at
once outside a transaction, and never if the transaction rolls back. That
makes it safe to notify beside the write it announces, since listeners hear
of the order only once the order is there to read. Notifications arrive in
commit order across transactions and in sending order within one, and one
that repeats an earlier notification of the same transaction, the same
channel and the same payload, is folded into it:

```ts
import assert from "node:assert/strict";

await using listener = await pool.listener();
await listener.listen("orders");

// Rolled back: never sent.
await assert.rejects(
  pool.transaction(async (tx) => {
    await tx.notify("orders", "order 3 placed");
    throw new Error("the card was declined");
  }),
  /the card was declined/,
);

// Committed: sent at the commit, the repeated payload once.
await pool.transaction(async (tx) => {
  await tx.notify("orders", "order 4 placed");
  await tx.notify("orders", "order 4 placed");
  await tx.notify("orders", "order 4 paid");
});

const payloads: string[] = [];
for (let read = 0; read < 2; read++) {
  const heard = await listener.next({ signal: AbortSignal.timeout(5000) });
  assert.ok(!heard.done);
  payloads.push(heard.value.payload);
}
assert.deepEqual(payloads, ["order 4 placed", "order 4 paid"]);

// Nothing else came: an aborted pull leaves the listener open.
await assert.rejects(listener.next({ signal: AbortSignal.timeout(200) }), { name: "TimeoutError" });
assert.equal(listener.closed, false);
```

The server refuses an empty channel name, one longer than 63 bytes, and a
payload of 8000 bytes or more, each with a `DatabaseError`. A payload is best
kept small, such as the id of the row that changed, with the listener reading
the row itself.

## When a listener stops

A listener's iteration ends only after a failure, or once it is closed, and
nothing reconnects it. A notification sent while no session is listening
reaches nobody, so a listener that reconnected silently would hide the
notifications it missed. When it ends, open a new listener, listen again, and
then re-read whatever the notifications were telling you about, since some of
it may have changed in between. It fails in two ways:

- The connection fails or is ended, such as by a server restart. The listener
  yields every notification it had already received, then its next pull
  rejects with the failure, a `DatabaseError` or a `ConnectionError`, and
  the one after that is done. Rust's stream yields the `Error::Postgres`.
- The reader falls behind. The listener holds at most `capacity` unread
  notifications, 1024 by default. When that many are waiting and another
  arrives, the listener closes its connection at once, yields those waiting,
  and its next pull rejects with a `ConnectionError`, where Rust's yields
  `ListenError::Overflow`.

Closing on overflow rather than waiting is deliberate. A listener that
stopped reading its connection would stop consuming PostgreSQL's notification
queue, which every database on the server shares, and once that queue fills
every `NOTIFY` on the server fails at commit. 1024 is more than a reader that
keeps up ever holds.

Here a listener of capacity two is sent three notifications before it reads
any:

```ts
import assert from "node:assert/strict";
import { ConnectionError } from "@necessary/pgorm";

await using listener = await pool.listener({ capacity: 2 });
await listener.listen("orders");
await pool.transaction(async (tx) => {
  for (const order of [3, 4, 5]) await tx.notify("orders", `order ${order} placed`);
});

// Each `listen` is a round trip, answered after the listener has been sent
// what is pending, so this waits until the overflow has closed it.
const deadline = AbortSignal.timeout(5000);
for (;;) {
  try {
    await listener.listen("orders", { signal: deadline });
  } catch {
    break;
  }
}

const first = await listener.next();
const second = await listener.next();
assert.ok(!first.done && !second.done);
assert.deepEqual([first.value.payload, second.value.payload], ["order 3 placed", "order 4 placed"]);
await assert.rejects(listener.next(), ConnectionError);
assert.equal((await listener.next()).done, true);
```

And here the listener's session is ended from another connection, as a
server shutting down would end it. The listener reports the failure, and a
new listener takes over:

```ts
import assert from "node:assert/strict";
import { PgormError } from "@necessary/pgorm";

let listener = await pool.listener();
await listener.listen("kitchen");

const ended = await pool.execute(
  `SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = current_database() AND query = 'LISTEN "kitchen"'`,
);
assert.equal(ended, 1);

await assert.rejects(listener.next({ signal: AbortSignal.timeout(5000) }), PgormError);
assert.equal((await listener.next()).done, true);

// Nothing reconnects: open a new listener and listen again.
listener = await pool.listener();
await listener.listen("kitchen");
await pool.notify("kitchen", "back");
const heard = await listener.next({ signal: AbortSignal.timeout(5000) });
assert.ok(!heard.done);
assert.equal(heard.value.payload, "back");
await listener.close();
```

## A listener runs no statements

A `Listener` sends nothing but `LISTEN` and `UNLISTEN`, and it has none of
the methods that run statements. A statement on its connection would wait
behind the notifications arriving there, and a transaction would hold their
delivery back until it ended. Read and write through the pool or a connection
taken from it, and send notifications from one with `notify`. Rust's
`Listener` does not implement `ConnectionTrait`, so such a call does not
compile; TypeScript refuses it the same way, and at run time the method is
simply not there:

```ts
import assert from "node:assert/strict";

await using listener = await pool.listener();
assert.equal("query" in listener, false);
assert.equal("execute" in listener, false);
assert.equal("notify" in listener, false);
```
