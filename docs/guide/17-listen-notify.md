# LISTEN and NOTIFY

PostgreSQL can tell a session when something happens. A session that has run
`LISTEN orders` receives a *notification* each time any session sends one on
the `orders` channel, with a text payload. It is how one process wakes
another: a new order is placed, and the kitchen's display hears about it at
once instead of polling the table.

A notification is delivered to the session that listened, outside the result
of any statement. A pooled connection cannot receive one: the pool's task
that drives it discards such messages, and the pool lends the connection to
someone else once it is returned. So a listener is a connection of its own,
opened for the purpose and owned by the listener alone.

## Listening

[`DatabasePool::listener`](crate::DatabasePool::listener) opens a listening
connection with the pool's configuration. The connection is not taken from
the pool and does not count against its size. It returns two halves:

- a [`Listener`](crate::Listener), which chooses the channels:
  [`listen`](crate::Listener::listen),
  [`unlisten`](crate::Listener::unlisten) and
  [`unlisten_all`](crate::Listener::unlisten_all);
- its [`Notifications`](crate::Notifications), a `Stream` of each
  [`Notification`](crate::Notification) the connection receives, with its
  channel, its payload and the process id of the session that sent it.

[`notify`](crate::notify) sends a notification from any connection or
transaction:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("listen_basic").await;
# let pool = scratch.pool.clone();
use std::time::Duration;

use futures::StreamExt;
use pgorm::{Listener, notify};

let (listener, mut notifications) = pool.listener(Listener::DEFAULT_CAPACITY).await?;
listener.listen("orders").await?;

let db = pool.get().await?;
notify(&db, "orders", "order 3 placed").await?;

let heard = tokio::time::timeout(Duration::from_secs(5), notifications.next())
    .await?
    .expect("the stream is open")?;
assert_eq!(heard.channel(), "orders");
assert_eq!(heard.payload(), "order 3 placed");
# drop((listener, notifications, db));
# scratch.finish().await;
# Ok(())
# }
```

The listener's connection is driven by a task of its own, which queues each
notification as it arrives, so `listen` and `unlisten` complete whether or
not anyone is reading the stream. A `Listener` is cheap to clone, and every
clone changes the same set of channels, so one task can add channels while
another reads. The stream holds the connection open while it lives; dropping
it closes the connection, after which every `Listener` method fails.

Once `listen` returns, every notification on the channel from a transaction
that commits afterwards is delivered. Once `unlisten` returns, none is,
though one already received is still in the stream.

To listen through a configuration of your own rather than a pool's,
[`connect_listener`](crate::connect_listener) takes a
[`Config`](crate::Config) and a TLS connector, as
[`connect_with`](crate::connect_with) does:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("listen_connect").await;
# let config = scratch.config();
use pgorm::{Listener, NoTls, connect_listener};

let (listener, notifications) =
    connect_listener(config, NoTls, Listener::DEFAULT_CAPACITY).await?;
listener.listen("deliveries").await?;
listener.unlisten_all().await?;

// Dropping the stream closes the connection.
drop(notifications);
assert!(listener.listen("deliveries").await.is_err());
# scratch.finish().await;
# Ok(())
# }
```

## Channel names

A channel name is exact. `listen` sends it as a quoted identifier, so its
case and punctuation are kept: `Orders` and `orders` are two channels, and
the name is the same text `notify` sends to. PostgreSQL keeps 63 bytes of an
identifier and would quietly listen on a shortened name nothing can send to,
so a longer name is refused with
[`ListenError::ChannelTooLong`](crate::ListenError::ChannelTooLong) before
anything is sent:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("listen_names").await;
# let pool = scratch.pool.clone();
use std::time::Duration;

use futures::StreamExt;
use pgorm::{Error, ListenError, Listener, notify};

let (listener, mut notifications) = pool.listener(Listener::DEFAULT_CAPACITY).await?;
listener.listen("Orders").await?;

let db = pool.get().await?;
notify(&db, "orders", "not heard: another channel").await?;
notify(&db, "Orders", "heard").await?;
let heard = tokio::time::timeout(Duration::from_secs(5), notifications.next())
    .await?
    .expect("the stream is open")?;
assert_eq!(heard.payload(), "heard");

let long = "o".repeat(64);
let refused = listener.listen(&long).await;
assert!(matches!(refused, Err(Error::Listen(ListenError::ChannelTooLong { .. }))));
# drop((listener, notifications, db));
# scratch.finish().await;
# Ok(())
# }
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

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("listen_transactions").await;
# let pool = scratch.pool.clone();
use std::time::Duration;

use futures::StreamExt;
use pgorm::{Error, Listener, TransactionTrait, notify};

let (listener, mut notifications) = pool.listener(Listener::DEFAULT_CAPACITY).await?;
listener.listen("orders").await?;
let mut db = pool.get().await?;

// Rolled back: never sent.
let refused = db
    .transaction(async |txn| {
        notify(&txn, "orders", "order 3 placed").await?;
        Err::<(), _>(Error::Custom("the card was declined".to_owned()))
    })
    .await;
assert!(refused.is_err());

// Committed: sent at the commit, the repeated payload once.
db.transaction(async |txn| {
    notify(&txn, "orders", "order 4 placed").await?;
    notify(&txn, "orders", "order 4 placed").await?;
    notify(&txn, "orders", "order 4 paid").await?;
    Ok::<_, Error>(())
})
.await?;

let mut payloads = Vec::new();
for _ in 0..2 {
    let heard = tokio::time::timeout(Duration::from_secs(5), notifications.next())
        .await?
        .expect("the stream is open")?;
    payloads.push(heard.payload().to_owned());
}
assert_eq!(payloads, ["order 4 placed", "order 4 paid"]);
# drop((listener, notifications, db));
# scratch.finish().await;
# Ok(())
# }
```

The server refuses an empty channel name, one longer than 63 bytes, and a
payload of 8000 bytes or more. A payload is best kept small, such as the id
of the row that changed, with the listener reading the row itself.

## When a listener stops

A listener's stream ends only after an error, and nothing reconnects it. A
notification sent while no session is listening reaches nobody, so a listener
that reconnected silently would hide the notifications it missed. When the
stream ends, open a new listener, listen again, and then re-read whatever the
notifications were telling you about, since some of it may have changed in
between. The stream ends in two ways:

- The connection fails or is ended, such as by a server restart. The stream
  yields every notification it had already received, then the
  [`Error::Postgres`](crate::Error::Postgres) the connection failed with,
  then ends.
- The reader falls behind. The stream holds at most `capacity` unread
  notifications. When that many are waiting and another arrives, the
  listener closes its connection at once, and the stream yields those
  waiting, then [`ListenError::Overflow`](crate::ListenError::Overflow), then
  ends.

Closing on overflow rather than waiting is deliberate. A listener that
stopped reading its connection would stop consuming PostgreSQL's
notification queue, which every database on the server shares, and once that
queue fills every `NOTIFY` on the server fails at commit.
[`Listener::DEFAULT_CAPACITY`](crate::Listener::DEFAULT_CAPACITY), 1024, is
more than a reader that keeps up ever holds.

Here a listener of capacity two is sent three notifications before it reads
any:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("listen_overflow").await;
# let pool = scratch.pool.clone();
use std::num::NonZeroUsize;
use std::time::Duration;

use futures::StreamExt;
use pgorm::{Error, ListenError, TransactionTrait, notify};

let capacity = NonZeroUsize::new(2).expect("two is not zero");
let (listener, mut notifications) = pool.listener(capacity).await?;
listener.listen("orders").await?;
let mut db = pool.get().await?;
db.transaction(async |txn| {
    for order in [3, 4, 5] {
        notify(&txn, "orders", &format!("order {order} placed")).await?;
    }
    Ok::<_, Error>(())
})
.await?;

// Each `listen` is a round trip, answered after the listener has been sent
// what is pending, so this waits until the overflow has closed it.
tokio::time::timeout(Duration::from_secs(5), async {
    while listener.listen("orders").await.is_ok() {}
})
.await?;

let first = notifications.next().await.expect("one is waiting")?;
let second = notifications.next().await.expect("two are waiting")?;
assert_eq!((first.payload(), second.payload()), ("order 3 placed", "order 4 placed"));
assert!(matches!(
    notifications.next().await,
    Some(Err(Error::Listen(ListenError::Overflow { capacity: 2 })))
));
assert!(notifications.next().await.is_none());
# drop((listener, notifications, db));
# scratch.finish().await;
# Ok(())
# }
```

And here the listener's session is ended from another connection, as a
server shutting down would end it. The stream reports the server's error,
and a new listener takes over:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("listen_terminated").await;
# let pool = scratch.pool.clone();
use std::time::Duration;

use futures::StreamExt;
use pgorm::{ConnectionTrait, Error, Listener, notify};

let (listener, mut notifications) = pool.listener(Listener::DEFAULT_CAPACITY).await?;
listener.listen("kitchen").await?;

let db = pool.get().await?;
let ended = db
    .execute(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
         WHERE datname = current_database() AND query = 'LISTEN \"kitchen\"'",
        &[],
    )
    .await?;
assert_eq!(ended, 1);

let failure = tokio::time::timeout(Duration::from_secs(5), notifications.next())
    .await?
    .expect("the stream reports why it ends");
assert!(matches!(failure, Err(Error::Postgres(_))));
assert!(notifications.next().await.is_none());

// Nothing reconnects: open a new listener and listen again.
let (listener, mut notifications) = pool.listener(Listener::DEFAULT_CAPACITY).await?;
listener.listen("kitchen").await?;
notify(&db, "kitchen", "back").await?;
let heard = tokio::time::timeout(Duration::from_secs(5), notifications.next())
    .await?
    .expect("the stream is open")?;
assert_eq!(heard.payload(), "back");
# drop((listener, notifications, db));
# scratch.finish().await;
# Ok(())
# }
```

## A listener runs no statements

A `Listener` sends nothing but `LISTEN` and `UNLISTEN`, and it does not
implement [`ConnectionTrait`](crate::ConnectionTrait). A statement on its
connection would wait behind the notifications arriving there, and a
transaction would hold their delivery back until it ended. Read and write
through a pooled connection, and send notifications from one with `notify`.
The [metrics](crate::guide::metrics) layer does not see a listener either; a
`notify` through an instrumented connection is recorded as the statement it
is.

```rust,compile_fail,E0277
use pgorm::{ConnectionTrait, Listener};

async fn count(listener: &Listener) {
    let _ = listener.query_one("SELECT count(*) FROM cake", &[]).await;
}
```
