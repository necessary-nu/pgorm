# Statement caches and DDL

Every statement pgorm runs is prepared: the server parses and plans it once,
and the connection runs it again by name with new parameters. pgorm keeps the
statements each connection has prepared in a *statement cache* keyed by their
SQL text, so a query the program runs a thousand times is parsed once per
connection, not a thousand times. This is automatic, and most programs never
think about it.

A cached statement outlives the call that prepared it, and that is what this
chapter is about. A prepared statement was planned against the schema as it
was then. When DDL changes the schema under it, PostgreSQL usually notices and
plans the statement again, but not always. This chapter says when it does
not, and what to do.

## What the cache holds

Each connection has its own cache, shared by every transaction and savepoint
on it. Every [`ConnectionTrait`](crate::ConnectionTrait) method that runs a
statement looks its SQL text up there, and prepares it on a miss.
[`batch_execute`](crate::ConnectionTrait::batch_execute) is the exception: it
runs a script through the simple-query protocol and prepares nothing. A COPY
is prepared afresh each time, too ([COPY](crate::guide::copy)).

The cache is bounded, at 256 statements per connection unless the pool says
otherwise. One query can be many texts, such as an `IN` list rendered at every
length it is used with, and each cached statement is a live prepared statement
on the server, so an unbounded cache would hold statements the connection
never runs again for as long as it lives. A full cache evicts an entry to make
room. [`StatementCacheSize`](crate::StatementCacheSize) sets the bound, or
turns the cache off with `Disabled`, in which case every call prepares its
statement and closes it afterwards. It is set through the pool's
[`ManagerConfig`](crate::ManagerConfig) and
[`connect_with`](crate::connect_with)
([Connection configuration and TLS](crate::guide::configuration)):

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("statement_caches_size").await;
# let config = scratch.config();
use std::num::NonZeroUsize;

use pgorm::{ConnectionTrait, ManagerConfig, NoTls, StatementCacheSize, connect_with};

/// How many statements this session holds prepared whose text starts with `prefix`.
async fn prepared(db: &pgorm::DatabaseConnection, prefix: &str) -> Result<i64, pgorm::Error> {
    let row = db
        .query_one(
            "SELECT count(*) FROM pg_prepared_statements WHERE starts_with(statement, $1)",
            &[&prefix],
        )
        .await?;
    Ok(row.get(0))
}

for (cache, kept) in [
    (StatementCacheSize::Bounded(NonZeroUsize::new(64).expect("not zero")), 1),
    (StatementCacheSize::Disabled, 0),
] {
    let manager = ManagerConfig {
        statement_cache: cache,
        ..ManagerConfig::default()
    };
    let pool = connect_with(config.clone(), NoTls, manager, |builder| builder.max_size(1))?;
    let db = pool.get().await?;
    db.query_all("SELECT name FROM bakery", &[]).await?;
    assert_eq!(prepared(&db, "SELECT name FROM bakery").await?, kept);
#   drop(db);
#   pool.close();
}
# scratch.finish().await;
# Ok(())
# }
```

## When the server notices

PostgreSQL plans a cached statement again by itself when what it reads
changes: an index added, a table dropped and created again, a different table
found through a changed `search_path`. One kind of change it cannot hide. A
statement's result columns were described to the client when it was
prepared, and if DDL changes them, as adding a column under `SELECT *` does,
the server refuses to run it with SQLSTATE `0A000`, *cached plan must not
change result type*.

pgorm handles that refusal for you as far as it safely can. It always drops
the refused statement from the cache, so the next call prepares it afresh.
Outside a transaction it also prepares the statement again at once and runs
it, a single retry. Inside a transaction it cannot: the refusal has already
aborted the transaction, so it hands the `0A000` to you, and the recovery is
yours to run at the transaction's boundary, for example by retrying the
transaction ([Transactions](crate::guide::transactions)).

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("statement_caches_result_type").await;
# let pool = scratch.pool.clone();
use pgorm::{ConnectionTrait, Error, TransactionError, TransactionTrait};

let mut db = pool.get().await?;
let before = db.query_all("SELECT * FROM baker", &[]).await?;
assert_eq!(before[0].len(), 4);

db.batch_execute("ALTER TABLE baker ADD COLUMN apron text").await?;

// Outside a transaction, the refused statement is prepared again and run.
let after = db.query_all("SELECT * FROM baker", &[]).await?;
assert_eq!(after[0].len(), 5);

db.batch_execute("ALTER TABLE baker DROP COLUMN apron").await?;

// Inside one, the refusal is the transaction's error.
let refused = db
    .transaction(async |txn| txn.query_all("SELECT * FROM baker", &[]).await)
    .await;
assert!(matches!(
    refused,
    Err(TransactionError::Transaction(Error::Postgres(error)))
        if error.code().map(|code| code.code()) == Some("0A000")
));

// It was dropped from the cache, so running the transaction again succeeds.
let rows = db
    .transaction(async |txn| txn.query_all("SELECT * FROM baker", &[]).await)
    .await?;
assert_eq!(rows[0].len(), 4);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

Selecting named columns rather than `*`, as every query pgorm builds does,
keeps a statement's result from changing under it.

## When it does not: clearing the caches

Some DDL changes what a plan was built from without the server planning the
statement again. A domain's default is one: an `INSERT` prepared before
`ALTER DOMAIN .. SET DEFAULT` can go on writing the old default. For such
changes, [`DatabasePool::clear_statement_caches`](crate::DatabasePool::clear_statement_caches)
drops every cached statement of every connection in the pool, closing each on
the server, so each statement is prepared again the next time it runs.

The call does not clear anything in place, because a connection that is
checked out may be preparing a statement at that very moment, against the
schema the DDL just replaced. It marks every connection's cache as out of
date instead. An idle connection is cleared before the pool next lends it; a
connection checked out at the time keeps its statements until it is
returned, and is cleared before it is lent again. So code holding a
connection across the DDL returns it and takes another to see the change.
The call sends nothing, waits for nothing and cannot fail.

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("statement_caches_clear").await;
# let config = scratch.config();
use pgorm::ConnectionTrait;

const INSERT: &str = "INSERT INTO box (contents) VALUES ($1) RETURNING ribbon";
const PREPARED: &str =
    "SELECT count(*) FROM pg_prepared_statements WHERE starts_with(statement, 'INSERT INTO box')";

// One connection, so every checkout is the same session.
let pool = pgorm::connect_with_builder(config, |builder| builder.max_size(1))?;
let db = pool.get().await?;
db.batch_execute(
    "CREATE DOMAIN ribbon AS text DEFAULT 'red'; \
     CREATE TABLE box (id serial PRIMARY KEY, contents text, ribbon ribbon)",
)
.await?;
let ribbon: String = db.query_one(INSERT, &[&"truffles"]).await?.get(0);
assert_eq!(ribbon, "red");

db.batch_execute("ALTER DOMAIN ribbon SET DEFAULT 'gold'").await?;
pool.clear_statement_caches();

// The connection was checked out, so it keeps its statement until returned.
let kept: i64 = db.query_one(PREPARED, &[]).await?.get(0);
assert_eq!(kept, 1);
drop(db);

// Lent again, it was cleared first, and the insert is prepared afresh.
let db = pool.get().await?;
let kept: i64 = db.query_one(PREPARED, &[]).await?.get(0);
assert_eq!(kept, 0);
let ribbon: String = db.query_one(INSERT, &[&"fudge"]).await?.get(0);
assert_eq!(ribbon, "gold");
# drop(db);
# pool.close();
# scratch.finish().await;
# Ok(())
# }
```

[pgorm-migration](crate::guide::migrations)'s runner calls `clear_statement_caches`
on the pool it was given once a batch that applied a migration commits, so a
program that migrates through its own pool needs nothing more.

## Clearing other processes' caches

`clear_statement_caches` reaches the pool it is called on, in the process
that holds it. An application usually runs as several processes, and the one
that migrates is often none of them. For those, a pool can listen for a clear:
[`ManagerConfig::statement_cache_channel`](crate::ManagerConfig::statement_cache_channel)
names a channel, and a pool given one keeps a connection of its own
`LISTEN`ing there ([LISTEN and NOTIFY](crate::guide::listen_notify)). Every
notification on the channel clears the pool's caches as
`clear_statement_caches` does, whatever its payload.

It is off by default, because it costs each pool one connection, opened with
the pool's first. That connection is not lent, counts against no size limit,
and is reopened if it drops, with a wait of 100 ms that doubles to 10 s
between attempts; and since a notification sent while it was down reached
nobody, the caches are cleared each time its `LISTEN` takes effect, the first
time included. [`DatabasePool::close`](crate::DatabasePool::close), or
dropping the pool, closes it. Nothing travels on it between notifications, so
a network middlebox that drops idle connections drops it too; TCP keepalives
set on the pool's [`Config`](crate::Config) with
[`keepalives_idle`](crate::Config::keepalives_idle) keep it open.

[`STATEMENT_CACHE_CHANNEL`](crate::STATEMENT_CACHE_CHANNEL),
`pgorm_statement_caches`, is the channel pgorm-migration's runner notifies,
inside the transaction of a batch that applied a migration, so the
notification is delivered exactly when the migration commits. A process that
changes the schema some other way sends one itself with
[`notify`](crate::notify). A notification reaches only sessions on the
database it was sent in, so one channel name serves every database on a
server.

Here a second pool, standing for another process, writes a domain's default
and hears the clear the first sends after changing it:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("statement_caches_channel").await;
# let config = scratch.config();
use std::time::Duration;

use pgorm::{
    ConnectionTrait, ManagerConfig, NoTls, STATEMENT_CACHE_CHANNEL, connect_with, notify,
};

const INSERT: &str = "INSERT INTO box (contents) VALUES ($1) RETURNING ribbon";

// The process that migrates.
let migrator = pgorm::connect(config.clone());
let db = migrator.get().await?;
db.batch_execute(
    "CREATE DOMAIN ribbon AS text DEFAULT 'red'; \
     CREATE TABLE box (id serial PRIMARY KEY, contents text, ribbon ribbon)",
)
.await?;

// Another process's pool, listening for clears.
let manager = ManagerConfig {
    statement_cache_channel: Some(STATEMENT_CACHE_CHANNEL.to_owned()),
    ..ManagerConfig::default()
};
let shop = connect_with(config, NoTls, manager, |builder| builder.max_size(1))?;
let ribbon: String = shop.get().await?.query_one(INSERT, &[&"truffles"]).await?.get(0);
assert_eq!(ribbon, "red");

db.batch_execute("ALTER DOMAIN ribbon SET DEFAULT 'gold'").await?;
notify(&db, STATEMENT_CACHE_CHANNEL, "").await?;

// The clear arrives as a notification, so the shop sees it a moment later.
let ribbon = tokio::time::timeout(Duration::from_secs(10), async {
    loop {
        let ribbon: String = shop.get().await?.query_one(INSERT, &[&"fudge"]).await?.get(0);
        if ribbon == "gold" {
            return Ok::<_, pgorm::Error>(ribbon);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
})
.await??;
assert_eq!(ribbon, "gold");
# drop(db);
# shop.close();
# migrator.close();
# scratch.finish().await;
# Ok(())
# }
```

The clear is the same deferred one: a connection the other process holds
checked out for its whole life keeps its statements until it returns it.
`connect_with` refuses an empty channel name and one longer than 63 bytes,
which no notification could reach. The [metrics](crate::guide::metrics) layer does
not see these clears: the listener runs inside the pool and runs no
statement of yours.
