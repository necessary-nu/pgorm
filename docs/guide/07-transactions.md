# Transactions

A transaction groups statements so that they take effect together or not at
all. Placing an order writes the order and each of its lines; a transaction
makes sure no one ever sees an order with half its lines, and that a failure
part-way leaves nothing behind.

In pgorm a transaction exists only while a closure runs. You hand
[`transaction`](crate::Transact::transaction) an async closure, and
what the closure returns ends the transaction: `Ok` commits it, `Err` rolls it
back. There is no handle to commit later and no transaction to forget to end.

## Running a closure in a transaction

[`transaction`](crate::Transact::transaction) is a method of
[`Transact`](crate::Transact), which the prelude brings in. It
takes the connection by `&mut`, so the connection must be mutable. The
closure receives a [`DatabaseTransaction`](crate::DatabaseTransaction), a
[`Connection`](crate::Connection) like the connection, and every
statement it runs is part of the transaction:

```rust
# use pgorm_guide_support::{order, order_line};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("transactions_commit").await;
# let pool = scratch.pool.clone();
use pgorm::entity::prelude::*;

let placed_at: DateTimeWithTimeZone = "2026-11-04T10:00:00Z".parse()?;
let mut db = pool.get().await?;
let order = db
    .transaction(async |txn| {
        let order = order::ActiveModel {
            customer_id: set(2),
            bakery_id: set(1),
            placed_at: set(placed_at),
            ..Default::default()
        }
        .insert(&txn)
        .await?;
        Insert::many([
            order_line::ActiveModel {
                order_id: set(order.id),
                cake_id: set(5),
                quantity: set(1),
                price: set(Decimal::new(600, 2)),
                ..Default::default()
            },
            order_line::ActiveModel {
                order_id: set(order.id),
                cake_id: set(2),
                quantity: set(2),
                price: set(Decimal::new(320, 2)),
                ..Default::default()
            },
        ])
        .exec(&txn)
        .await?;
        Ok::<_, Error>(order)
    })
    .await?;

let lines = order_line::Entity::find()
    .filter(order_line::Column::OrderId.eq(order.id))
    .count(&db)
    .await?;
assert_eq!((order.id, lines), (3, 2));
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

The closure's error type is yours: a closure whose every failure goes
through `?` and that ends in a bare `Ok(value)` leaves nothing to infer it
from, so the example writes `Ok::<_, Error>(order)`.

While the transaction is open it borrows the connection exclusively. Nothing
else can run on that connection until the closure has finished, and the
compiler holds you to it:

```rust,compile_fail,E0502
# use pgorm_guide_support::cake;
# async fn run(mut db: pgorm::DatabaseConnection) -> Result<(), pgorm::TransactionError<pgorm::Error>> {
use pgorm::entity::prelude::*;

db.transaction(async |txn| {
    // `db` is the transaction's for now: use `txn`.
    cake::Entity::find().all(&db).await
})
.await?;
# Ok(())
# }
```

## Rolling back

When the closure returns `Err`, the transaction rolls back, and the rollback
has finished by the time `transaction` returns. The error comes back as a
[`TransactionError`](crate::TransactionError), which keeps apart the ways a
transaction can fail:

| Variant | What failed |
| --- | --- |
| [`Transaction(e)`](crate::TransactionError::Transaction) | the closure returned `e`, and the transaction was rolled back |
| [`Connection(e)`](crate::TransactionError::Connection) | the transaction itself: its `BEGIN` or its `COMMIT` |
| [`RollbackFailed { cause, rollback }`](crate::TransactionError::RollbackFailed) | the closure returned `cause`, and then the `ROLLBACK` failed with `rollback` |

Here the second line names a cake that does not exist, so its insert fails,
and the order written before it is rolled back with it:

```rust
# use pgorm_guide_support::{order, order_line};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("transactions_rollback").await;
# let pool = scratch.pool.clone();
use pgorm::{SqlError, TransactionError, entity::prelude::*};

let placed_at: DateTimeWithTimeZone = "2026-11-04T10:00:00Z".parse()?;
let mut db = pool.get().await?;
let outcome = db
    .transaction(async |txn| {
        let order = order::ActiveModel {
            customer_id: set(1),
            bakery_id: set(1),
            placed_at: set(placed_at),
            ..Default::default()
        }
        .insert(&txn)
        .await?;
        order_line::ActiveModel {
            order_id: set(order.id),
            cake_id: set(99),
            quantity: set(1),
            price: set(Decimal::new(100, 2)),
            ..Default::default()
        }
        .insert(&txn)
        .await?;
        Ok::<_, Error>(order)
    })
    .await;

match outcome {
    Err(TransactionError::Transaction(error)) => {
        assert!(matches!(error.sql_error(), Some(SqlError::ForeignKeyConstraintViolation(_))));
    }
    other => panic!("expected the closure's own error, got {other:?}"),
}
assert_eq!(order::Entity::find().count(&db).await?, 2);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A `RollbackFailed` means that neither you nor pgorm knows how the transaction
ended, nor what state its connection is in. Its `cause` is still the reason
the transaction failed; `rollback` says why it could not be undone. Give such
a connection back with [`discard`](crate::DatabaseConnection::discard), so
the pool closes it rather than lending it again:

```rust
use pgorm::{DatabaseConnection, Error, TransactionError};

/// Hand a connection back after a transaction, closing it when its state
/// is unknown.
fn give_back<T>(db: DatabaseConnection, outcome: &Result<T, TransactionError<Error>>) -> bool {
    match outcome {
        Ok(_) | Err(TransactionError::Transaction(_)) => {
            drop(db);
            true
        }
        // A COMMIT that rolled back, because a failed statement had aborted
        // the transaction, leaves the connection idle.
        Err(TransactionError::Connection(Error::TransactionAborted(_))) => {
            drop(db);
            true
        }
        // A failed BEGIN or COMMIT, a failed ROLLBACK, and any kind of
        // failure added later: the connection's state is unknown.
        Err(_) => {
            db.discard();
            false
        }
    }
}
# let _ = give_back::<()>;
# assert!(matches!(
#     TransactionError::<Error>::Transaction(Error::RecordNotFound),
#     TransactionError::Transaction(_)
# ));
```

`TransactionError` is `#[non_exhaustive]`: kinds of failure grow, so a
`match` on it ends in an arm for the ones it does not know.

## Savepoints

A transaction is itself a `Transact`. Its `transaction` opens a
savepoint: a transaction nested inside the first, which commits into it or
rolls back alone. A savepoint's `transaction` returns a `TransactionError`
like any other, and a `TransactionError` converts into an
[`Error`](crate::Error), so `?` passes a savepoint's failure on to the
closure around it as it passes a statement's. Here the order survives a line
naming a cake that does not exist, and any other failure ends the whole
order:

```rust
# use pgorm_guide_support::{order, order_line};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("transactions_savepoint").await;
# let pool = scratch.pool.clone();
use pgorm::{SqlError, TransactionError, entity::prelude::*};

let placed_at: DateTimeWithTimeZone = "2026-11-04T10:00:00Z".parse()?;
let mut db = pool.get().await?;
let (order, skipped) = db
    .transaction(async |txn| {
        let order = order::ActiveModel {
            customer_id: set(1),
            bakery_id: set(2),
            placed_at: set(placed_at),
            ..Default::default()
        }
        .insert(&txn)
        .await?;
        let mut skipped = Vec::new();
        for cake_id in [3, 99, 4] {
            let line = txn
                .transaction(async |savepoint| {
                    order_line::ActiveModel {
                        order_id: set(order.id),
                        cake_id: set(cake_id),
                        quantity: set(1),
                        price: set(Decimal::new(300, 2)),
                        ..Default::default()
                    }
                    .insert(&savepoint)
                    .await
                })
                .await;
            match line {
                Ok(_) => {}
                // No such cake: the savepoint rolled the line back, and the
                // order goes on without it.
                Err(TransactionError::Transaction(error))
                    if matches!(
                        error.sql_error(),
                        Some(SqlError::ForeignKeyConstraintViolation(_))
                    ) =>
                {
                    skipped.push(cake_id)
                }
                // Anything else is the order's failure too.
                Err(other) => return Err(other.into()),
            }
        }
        Ok::<_, Error>((order, skipped))
    })
    .await?;

let lines = order_line::Entity::find()
    .filter(order_line::Column::OrderId.eq(order.id))
    .count(&db)
    .await?;
assert_eq!((lines, skipped), (2, vec![99]));
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

Without the savepoint, the failed insert would have aborted the whole
transaction: PostgreSQL refuses every later statement in a transaction one
statement has failed in, until it rolls back.

## A caught error still aborts

Catching a failed statement's error inside the closure does not undo what the
failure did to the transaction. PostgreSQL has aborted it, and answers its
`COMMIT` by rolling it back. pgorm asks before it commits, so a closure that
returns `Ok` after catching such an error does not get its value back: the
transaction comes back as
`Connection(`[`Error::TransactionAborted`](crate::Error::TransactionAborted)`)`,
rolled back, nothing it wrote kept:

```rust
# use pgorm_guide_support::order;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("transactions_aborted").await;
# let pool = scratch.pool.clone();
use pgorm::{PostgresError, SqlState, TransactionError, entity::prelude::*};

let placed_at: DateTimeWithTimeZone = "2026-11-04T10:00:00Z".parse()?;
let mut db = pool.get().await?;
let outcome = db
    .transaction(async |txn| {
        order::ActiveModel {
            customer_id: set(1),
            bakery_id: set(1),
            placed_at: set(placed_at),
            ..Default::default()
        }
        .insert(&txn)
        .await?;
        // A mistake: the failure is caught, and the closure carries on.
        let refused = order::ActiveModel {
            customer_id: set(99),
            bakery_id: set(1),
            placed_at: set(placed_at),
            ..Default::default()
        }
        .insert(&txn)
        .await;
        assert!(refused.is_err());
        Ok::<_, Error>(())
    })
    .await;

match outcome {
    Err(TransactionError::Connection(error @ Error::TransactionAborted(_))) => {
        let refusal = std::error::Error::source(&error)
            .and_then(|source| source.downcast_ref::<PostgresError>())
            .and_then(PostgresError::code);
        assert_eq!(refusal, Some(&SqlState::IN_FAILED_SQL_TRANSACTION));
    }
    other => panic!("expected the aborted transaction's rollback, got {other:?}"),
}
// Neither order was written, and the connection is ready for the next one.
assert_eq!(order::Entity::find().count(&db).await?, 2);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

The connection is left idle, so it goes back to the pool as usual. A
savepoint whose closure returns `Ok` after catching an error is rolled back
to in the same way, and its `transaction` returns the same error, so the
transaction around it carries on; `?` passes the error on to that
transaction's closure, which then rolls back too. Put the statement that may
fail in a savepoint of its own, as above, and handle its error outside the
savepoint's closure.

Asking costs a transaction's `COMMIT` nothing: it travels as `SELECT; COMMIT`
in one round trip, and an aborted transaction refuses the empty `SELECT`,
which skips the `COMMIT`. A savepoint's `RELEASE` takes one more round trip,
since the `SELECT` goes ahead of it on its own. Both statements show in the
server's statement log.

The conversion keeps what each variant says. `Transaction(e)` becomes `e`,
and `Connection(e)`, a `SAVEPOINT` or `RELEASE` that failed, becomes its
error. A savepoint whose `ROLLBACK TO SAVEPOINT` failed becomes
[`Error::RollbackFailed`](crate::Error::RollbackFailed), holding the
closure's error as `cause` and the rollback's as `rollback`: the enclosing
transaction is then in a state nobody knows, and `?` carries that out to it
rather than reporting a savepoint that rolled back cleanly. Returned from the
enclosing closure, it rolls that transaction back too.

The same `?` works on a transaction opened on a connection, in a function
whose error is `Error`:

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("transactions_question_mark").await;
# let pool = scratch.pool.clone();
use pgorm::entity::prelude::*;

/// Raise the price of every cake a bakery sells by `cents`, returning how
/// many changed.
async fn raise_prices(db: &mut DatabaseConnection, bakery: i32, cents: i64) -> Result<u64, Error> {
    let raised = db
        .transaction(async |txn| {
            Update::many(cake::Entity)
                .col_expr(
                    cake::Column::Price,
                    Expr::col(cake::Column::Price).add(Decimal::new(cents, 2)),
                )
                .filter(cake::Column::BakeryId.eq(bakery))
                .exec(&txn)
                .await
        })
        .await?;
    Ok(raised)
}

let mut db = pool.get().await?;
assert_eq!(raise_prices(&mut db, 1, 10).await?, 3);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A closure whose error is a type of your own gets the same `?` by converting
a `TransactionError` of it into it, as `Error` does: keep `RollbackFailed`'s
two errors rather than only its cause.

## Choosing the isolation level

[`transaction_with`](crate::DatabaseConnection::transaction_with) opens a
transaction that may write in a [`TransactionMode`](crate::TransactionMode):

| Mode | Opens |
| --- | --- |
| `Default` | a transaction under the session's defaults, as `transaction` does |
| `ReadWrite { isolation }` | `READ WRITE`, at the [`IsolationLevel`](crate::IsolationLevel) given or the session's |

`transaction_with` is a method of a connection alone, `DatabaseConnection`
and the metrics layer's
[`InstrumentedConnection`](crate::metric::InstrumentedConnection): a
savepoint takes no isolation level or access mode. A transaction that may
only read is not one of its modes, since its handle would offer writes the
server then refuses; it has methods of its own, which the next section shows.

## Read-only transactions

[`read_only_transaction`](crate::DatabaseConnection::read_only_transaction)
opens `READ ONLY` at the isolation level given, and
[`snapshot`](crate::DatabaseConnection::snapshot) opens `SERIALIZABLE READ
ONLY DEFERRABLE`, which may wait for a safe snapshot and then cannot fail with
a serialization error. Their closure is given a
[`ReadOnlyTransaction`](crate::ReadOnlyTransaction), which runs every read and
no write.

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("transactions_read_only").await;
# let pool = scratch.pool.clone();
use pgorm::{IsolationLevel, entity::prelude::*};

let mut db = pool.get().await?;

// Two reads that see the same snapshot of the database.
let (cakes, total) = db
    .read_only_transaction(Some(IsolationLevel::RepeatableRead), async |txn| {
        let cakes = cake::Entity::find().count(&txn).await?;
        let total: Option<Decimal> = cake::Entity::find()
            .select(cake::Column::Price.sum())
            .into_tuple()
            .one(&txn)
            .await?;
        Ok::<_, Error>((cakes, total))
    })
    .await?;
assert_eq!((cakes, total.map(|t| t.to_string())), (5, Some("20.40".to_owned())));

let names = db
    .snapshot(async |txn| {
        cake::Entity::find()
            .order_by_asc(cake::Column::Id)
            .limit(2)
            .all(&txn)
            .await
    })
    .await?;
assert_eq!(names.len(), 2);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A write inside one does not compile: a `ReadOnlyTransaction` implements
[`Connection`](crate::Connection), what reads, and not
[`ConnectionMut`](crate::ConnectionMut), what may write, and the compiler
says so in those words, "`` `&mut ReadOnlyTransaction<'_>` may only read, so
it cannot run this write ``".

```rust,compile_fail,E0277
# use pgorm_guide_support::cake;
# use pgorm::entity::prelude::*;
# async fn purge(db: &mut DatabaseConnection) -> Result<(), pgorm::TransactionError<Error>> {
db.read_only_transaction(None, async |txn| {
    cake::Entity::delete_by_id(1).exec(txn).await?;
    Ok(())
})
.await
# }
```

## Connections that may only read

Every read pgorm builds takes a [`Connection`](crate::Connection) — finds,
counts, pages, cursors, graphs, loaders and pipelines — and every write a
[`ConnectionMut`](crate::ConnectionMut), which each connection that may
write implements beside it: inserts, updates and deletes, an `ActiveModel`'s
`save`, `batch_execute`, `notify` and migrations. A function that only reads
says so in its bound, and then cannot write through what it is given, a
connection that may write included:

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("transactions_connection_reads").await;
# let pool = scratch.pool.clone();
use pgorm::entity::prelude::*;

async fn menu(db: &impl Connection) -> Result<Vec<String>, Error> {
    Ok(cake::Entity::find()
        .order_by_asc(cake::Column::Name)
        .all(db)
        .await?
        .into_iter()
        .map(|cake| cake.name)
        .collect())
}

let db = pool.get().await?;
assert_eq!(menu(&db).await?.len(), 5);
// A connection that may write, borrowed as one that may only read, by its
// type alone.
assert_eq!(menu(&db.read_only()).await?.len(), 5);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

[`connect_read_only`](crate::connect_read_only) builds a
[`ReadOnlyPool`](crate::ReadOnlyPool), with
[`connect_read_only_with_builder`](crate::connect_read_only_with_builder)
and [`connect_read_only_with`](crate::connect_read_only_with) beside it as
`connect_with_builder` and `connect_with` are beside `connect`. Its
connections implement `Connection` alone, and each session opens with
`default_transaction_read_only = on`, so the server refuses what the types
cannot see — a `SELECT` calling a function that writes, a row lock taken
through the builder — with SQLSTATE `25006`, which
[`Error::read_only_refusal`](crate::Error::read_only_refusal) matches. A
replica, or a role granted only `SELECT`, is what such a pool fits; the role
is the boundary, since SQL text the server is handed can set the session
back.

SQL text is held to what it is known to do. [`sql!`](crate::sql) classifies
its statement as the crate compiles: a `SELECT`, `VALUES`, `TABLE`, `SHOW`,
or `EXPLAIN` without `ANALYZE`, with no data-modifying `WITH` and no row
lock, is a read, a [`Sql<Reads>`](crate::Sql), which runs on any
connection; anything else is a `Sql<Writes>`. Text known only at run time is
not known to read, so it runs on a connection that may write, or through a
read-only connection's [`unchecked`](crate::ReadOnlyConnection::unchecked),
where the server refuses a write:

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("transactions_read_only_pool").await;
use pgorm::{connect_read_only, entity::prelude::*, sql};

let replica = connect_read_only(scratch.config())?;
let mut ro = replica.get().await?;

assert_eq!(cake::Entity::find().count(&ro).await?, 5);
let shown: String = ro
    .query_one(sql!("SHOW default_transaction_read_only"), &[])
    .await?
    .get(0);
assert_eq!(shown, "on");

let table = "cake";
let refused = ro
    .unchecked()
    .execute(&format!("DELETE FROM {table}"), &[])
    .await
    .expect_err("the server refuses the delete");
assert!(refused.read_only_refusal().is_some());

// A transaction on a read-only connection reads under the session's default.
let cakes = ro
    .transaction(async |txn| cake::Entity::find().count(&txn).await)
    .await?;
assert_eq!(cakes, 5);
# drop(ro);
# drop(replica);
# scratch.finish().await;
# Ok(())
# }
```

Text on a connection that may only read does not compile:

```rust,compile_fail,E0277
# use pgorm::{Connection, Error, ReadOnlyConnection};
async fn count(db: &ReadOnlyConnection, table: &str) -> Result<i64, Error> {
    let sql = format!("SELECT count(*) FROM {table}");
    Ok(db.query_one(&sql, &[]).await?.get(0))
}
```

The [metrics](crate::guide::metrics) layer keeps a read-only pool
read-only: `InstrumentedPool::new(replica, collector)` lends instrumented
connections and transactions that still implement `Connection` alone. COPY
splits the same way: [`CopyConnection`](crate::CopyConnection)'s
`copy_out` reads and runs on a read-only connection, while its `copy_in`
runs only on a [`CopyConnectionMut`](crate::CopyConnectionMut), as
`batch_execute` runs only on a `ConnectionMut`.

## Retrying serializable transactions

Under `SERIALIZABLE` and `REPEATABLE READ`, PostgreSQL refuses a transaction
whose result another concurrent transaction has made inconsistent, with a
serialization failure (SQLSTATE `40001`), and expects the client to run it
again. It does the same to one side of a deadlock (`40P01`).
[`transaction_with_retry`](crate::DatabaseConnection::transaction_with_retry)
does that for you: it runs the whole transaction again, from `BEGIN`, while
it fails with one of those two errors, up to the number of retries you give.

The closure may run more than once, so it is an `AsyncFnMut`, and anything
it does outside the database happens again on each attempt. Here another
connection changes the cake's price after the first attempt has read it, and
the first attempt's update is refused; the second attempt reads the new price
and succeeds:

```rust
# use pgorm_guide_support::cake;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("transactions_retry").await;
# let pool = scratch.pool.clone();
use pgorm::{IsolationLevel, TransactionMode, entity::prelude::*};

let mut db = pool.get().await?;
let other = pool.get().await?;
let serializable = TransactionMode::ReadWrite {
    isolation: Some(IsolationLevel::Serializable),
};

let mut attempts = 0;
let doubled = db
    .transaction_with_retry(serializable, 3, async |txn| {
        attempts += 1;
        let fudge = cake::Entity::find_by_id(1).one(&txn).await?;
        if attempts == 1 {
            // Someone else's change commits after this transaction read the row.
            other
                .execute("UPDATE cake SET price = price + 1 WHERE id = 1", &[])
                .await?;
        }
        let price = fudge.price * Decimal::from(2);
        let mut changed = fudge.into_active();
        changed.price = set(price);
        Ok::<_, Error>(changed.update(&txn).await?.price)
    })
    .await?;
assert_eq!((attempts, doubled.to_string()), (2, "11.00".to_owned()));
# drop((db, other));
# scratch.finish().await;
# Ok(())
# }
```

Its closure's error type must say which of its errors are worth a retry: it
implements [`RetryableError`](crate::RetryableError), as
[`Error`](crate::Error) does through
[`is_retryable`](crate::Error::is_retryable). Implement it for an error type
of your own to use that type instead. An attempt whose `ROLLBACK` fails ends
the retrying with `RollbackFailed`.

## A dropped transaction rolls back

A transaction also ends if the future running it is dropped before the
closure finishes, as when a timeout fires or a task is cancelled. The
transaction then rolls back as its handle is dropped. That rollback is sent
without waiting for the server's answer, and pgorm logs a warning that the
transaction was dropped; the next statement on the connection runs after the
rollback, so it never sees the transaction's writes:

```rust
# use pgorm_guide_support::customer;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("transactions_dropped").await;
# let pool = scratch.pool.clone();
use std::time::Duration;

use pgorm::entity::prelude::*;

let mut db = pool.get().await?;
let slow = db.transaction(async |txn| {
    customer::ActiveModel {
        name: set("Mo"),
        email: set("mo@example.com"),
        notes: set(None),
        ..Default::default()
    }
    .insert(&txn)
    .await?;
    tokio::time::sleep(Duration::from_secs(30)).await;
    Ok::<_, Error>(())
});
let timed_out = tokio::time::timeout(Duration::from_millis(100), slow).await;
assert!(timed_out.is_err());

// Mo's insert was rolled back with the transaction.
assert_eq!(customer::Entity::find().count(&db).await?, 2);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

## Transactions in spawned tasks

On a connection or a transaction, the future `transaction` returns is `Send`
whenever the closure's is, savepoints nested inside included. A transaction
therefore runs inside `tokio::spawn`, or in a web framework's handler that
must be `Send`:

```rust
# use pgorm_guide_support::bakery;
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("transactions_spawn").await;
# let pool = scratch.pool.clone();
use pgorm::{TransactionError, entity::prelude::*};

let task = tokio::spawn(async move {
    let mut db = pool.get().await.map_err(TransactionError::Connection)?;
    db.transaction(async |txn| {
        bakery::ActiveModel {
            name: set("Noon"),
            city: set("Tromsø"),
            ..Default::default()
        }
        .insert(&txn)
        .await
    })
    .await
});
let noon = task.await??;
assert_eq!((noon.id, noon.name.as_str()), (3, "Noon"));
# scratch.finish().await;
# Ok(())
# }
```

Code generic over `C: Transact`, rather than written for a concrete
connection type, gets a future the compiler does not know to be `Send`: Rust
cannot yet state that an async closure's future is `Send` in a trait's
signature. Write such code for `DatabaseConnection` or `DatabaseTransaction`
where it has to run in a spawned task.

The metrics wrappers of [Metrics](crate::guide::metrics) are `Transact`s
too, and their transactions stay instrumented.
