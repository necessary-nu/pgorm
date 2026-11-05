# Errors

Every fallible call in pgorm returns a `Result` whose error is
[`pgorm::Error`](crate::Error), and [`pgorm::Result<T>`](crate::Result) is
short for `Result<T, pgorm::Error>`. An error says what failed in one of three
places: the database refused a statement, the driver refused a value on its
way in or out, or pgorm refused something before sending anything.

This chapter goes through each, and how a program tells them apart.

## What the database refused

A statement the server rejects comes back as
[`Error::Postgres`](crate::Error::Postgres), holding the tokio-postgres
error, [`PostgresError`](crate::PostgresError), with everything the server
reported: its SQLSTATE code, its message, and the table, column or
constraint involved.

The refusals a program most often has to answer are constraint violations: a
duplicate email address, an order line for a cake that does not exist.
[`sql_error`](crate::Error::sql_error) classifies them as a
[`SqlError`](crate::SqlError), and gives `None` for any other error:

| Variant | SQLSTATE | Raised by |
| --- | --- | --- |
| [`UniqueConstraintViolation`](crate::SqlError::UniqueConstraintViolation) | `23505` | a duplicate in a unique key or primary key |
| [`ForeignKeyConstraintViolation`](crate::SqlError::ForeignKeyConstraintViolation) | `23503` | a reference to a row that does not exist, or a delete of a row still referenced |
| [`CheckConstraintViolation`](crate::SqlError::CheckConstraintViolation) | `23514` | a table's or a domain's `CHECK` |
| [`NotNullConstraintViolation`](crate::SqlError::NotNullConstraintViolation) | `23502` | a `NULL` in a `NOT NULL` column or domain |
| [`ExclusionConstraintViolation`](crate::SqlError::ExclusionConstraintViolation) | `23P01` | an `EXCLUDE` constraint, or a key `WITHOUT OVERLAPS` |

Each carries a [`ConstraintViolation`](crate::ConstraintViolation): the
server's message, and the names it reported beside it. A table's constraint
names its `table` and `constraint`; a `NOT NULL` column names its `table` and
`column` and no constraint; a domain's names the domain as `datatype`.

```rust
# use pgorm_guide_support::{cake, customer};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("errors_constraints").await;
# let db = scratch.pool.get().await?;
use pgorm::{SqlError, entity::prelude::*};

// Lee's address is taken.
let duplicate = customer::ActiveModel {
    name: set("Lee Two"),
    email: set("lee@example.com"),
    notes: set(None),
    ..Default::default()
}
.insert(&db)
.await
.expect_err("the email is unique");
let Some(SqlError::UniqueConstraintViolation(violation)) = duplicate.sql_error() else {
    panic!("expected a unique violation, got {duplicate}");
};
assert_eq!(violation.table.as_deref(), Some("customer"));
assert_eq!(violation.constraint.as_deref(), Some("customer_email_key"));

// There is no bakery 99.
let orphan = cake::ActiveModel {
    name: set("Ghost Cake"),
    flavor: set(cake::Flavor::Vanilla),
    price: set(Decimal::new(100, 2)),
    gluten_free: set(false),
    bakery_id: set(99),
    ..Default::default()
}
.insert(&db)
.await
.expect_err("the bakery does not exist");
let Some(SqlError::ForeignKeyConstraintViolation(violation)) = orphan.sql_error() else {
    panic!("expected a foreign key violation, got {orphan}");
};
assert_eq!(violation.constraint.as_deref(), Some("fk-cake-bakery_id"));

// A cake left without a name.
let nameless = cake::ActiveModel {
    flavor: set(cake::Flavor::Vanilla),
    price: set(Decimal::new(100, 2)),
    gluten_free: set(false),
    bakery_id: set(1),
    ..Default::default()
}
.insert(&db)
.await
.expect_err("a cake has a name");
let Some(SqlError::NotNullConstraintViolation(violation)) = nameless.sql_error() else {
    panic!("expected a not-null violation, got {nameless}");
};
assert_eq!(violation.column.as_deref(), Some("name"));
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A check and an exclusion constraint answer the same way. The bakery's ovens
are booked by time, and an exclusion constraint keeps two bookings of one
oven from overlapping:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("errors_check_exclusion").await;
# let db = scratch.pool.get().await?;
use pgorm::{SqlError, entity::prelude::*};

db.batch_execute(
    "ALTER TABLE cake ADD CONSTRAINT cake_price_positive CHECK (price > 0);
     CREATE EXTENSION IF NOT EXISTS btree_gist;
     CREATE TABLE oven_booking (
         oven integer NOT NULL,
         during tstzrange NOT NULL,
         EXCLUDE USING gist (oven WITH =, during WITH &&)
     );",
)
.await?;

let free = db
    .execute("UPDATE cake SET price = $1 WHERE id = 1", &[&Decimal::ZERO])
    .await
    .expect_err("a cake costs something");
assert!(matches!(
    free.sql_error(),
    Some(SqlError::CheckConstraintViolation(v)) if v.constraint.as_deref() == Some("cake_price_positive")
));

let book = "INSERT INTO oven_booking (oven, during) VALUES ($1, tstzrange($2, $3))";
let at = |time: &str| format!("2026-11-04T{time}:00Z").parse::<DateTimeWithTimeZone>();
db.execute(book, &[&1_i32, &at("06:00")?, &at("08:00")?]).await?;
let clash = db
    .execute(book, &[&1_i32, &at("07:00")?, &at("09:00")?])
    .await
    .expect_err("oven 1 is booked from six to eight");
assert!(matches!(clash.sql_error(), Some(SqlError::ExclusionConstraintViolation(_))));
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

Any other refusal is read off the server's SQLSTATE. `Error::Postgres`
holds tokio-postgres's error, re-exported as
[`PostgresError`](crate::PostgresError), whose
[`code`](crate::PostgresError::code) is a [`SqlState`](crate::SqlState)
with a constant for every code PostgreSQL defines, and whose
[`as_db_error`](crate::PostgresError::as_db_error) is the whole of what the
server sent, a [`DbError`](crate::DbError):

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("errors_sqlstate").await;
# let db = scratch.pool.get().await?;
use pgorm::{ConnectionTrait, Error, SqlState};

let missing = db.query_all("SELECT * FROM pastry", &[]).await;
let Err(Error::Postgres(error)) = missing else {
    panic!("expected the server to refuse the query, got {missing:?}");
};
assert_eq!(error.code(), Some(&SqlState::UNDEFINED_TABLE));
assert_eq!(SqlState::UNDEFINED_TABLE.code(), "42P01");
let server = error.as_db_error().expect("the server refused it");
assert_eq!(server.message(), r#"relation "pastry" does not exist"#);
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

[`is_retryable`](crate::Error::is_retryable) answers whether an error is
one PostgreSQL expects the client to retry the whole transaction after: a
serialization failure (`40001`) or a deadlock (`40P01`).
[Transactions](crate::guide::transactions) shows the retrying.

## What the driver refused

Some failures happen on the client, without the server refusing anything. A
value can be refused on its way in, because the parameter's type cannot hold
it, or on its way out, because the field it is read into cannot hold the
column's value. Both arrive as `Error::Postgres` with no server code, and each
has a classifier:

- [`bind_refusal`](crate::Error::bind_refusal) gives a
  [`BindRefusal`](crate::BindRefusal): which `$N` parameter was refused, and
  why. Nothing was sent.
- [`decode_refusal`](crate::Error::decode_refusal) gives a
  [`DecodeRefusal`](crate::DecodeRefusal): what the driver was reading, such
  as a column, and why it could not.

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("errors_refusals").await;
# let db = scratch.pool.get().await?;
use pgorm::{ConnectionTrait, DecodeRaw, Values};

// The parameter is a smallint, and an i32 is not one.
let too_wide = db
    .execute("SELECT $1::smallint", &[&100_000_i32])
    .await
    .expect_err("an i32 does not bind as a smallint");
let refusal = too_wide.bind_refusal().expect("the driver refused the value");
assert_eq!(refusal.parameter, 1);
assert!(refusal.to_string().starts_with("parameter $1 cannot be bound"));

// The column is text, and an i32 cannot read it.
let unreadable = ("SELECT 'lemon'::text", Values(vec![]))
    .into_tuple::<i32>()
    .one(&db)
    .await
    .expect_err("text does not decode as an integer");
let refusal = unreadable.decode_refusal().expect("the driver refused the column");
assert_eq!(refusal.reading, "error deserializing column 0");
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A `NULL` read into a field that is not an `Option` is
[`Error::Type`](crate::Error::Type), saying which column held it, rather
than a refusal of the driver's.

## What pgorm refused

The other variants are pgorm's own. Most say that a call could not do what
it promised, before or instead of sending a statement:

| Variant | Means |
| --- | --- |
| [`RecordNotFound`](crate::Error::RecordNotFound) | `one` found no row; `one_opt` answers `None` instead |
| [`RecordNotInserted`](crate::Error::RecordNotInserted) | an insert that promised a key wrote no row, because `ON CONFLICT DO NOTHING` skipped it |
| [`PrimaryKeyNotSet`](crate::Error::PrimaryKeyNotSet) | a one-row update or delete was given a model whose key is not set, so there is no row to name |
| [`NothingToSet`](crate::Error::NothingToSet) | an update of many rows sets no column, so there is no statement to send |
| [`AttrNotSet`](crate::Error::AttrNotSet) | an `ActiveModel` with a column left out was converted into a `Model` |
| [`Type`](crate::Error::Type) | a value of the wrong type, such as a `NULL` for a field that is not an `Option`, or a foreign key [schema generation](crate::guide::schema) refuses to write |
| [`Json`](crate::Error::Json) | a JSON value that does not deserialize into its field |
| [`Conversion`](crate::Error::Conversion) | a value that did not fit the type it was converted into |
| [`UnpackInsertId`](crate::Error::UnpackInsertId) | the key an insert returned could not be read as the entity's key type |
| [`Query`](crate::Error::Query) | a read pgorm could not carry out as asked, with a message saying why, such as a loader whose keys do not match |
| [`Pool`](crate::Error::Pool) | the pool could not lend a connection: one could not be opened, or none came free in time |
| [`Verify`](crate::Error::Verify) | a decode target that does not match the statement it would read |
| [`QueryBuilder`](crate::Error::QueryBuilder) | the query builder refused its input, such as a template whose placeholders do not match its values |
| [`Listen`](crate::Error::Listen) | a `LISTEN` channel name that is too long, or a listener that fell behind ([LISTEN and NOTIFY](crate::guide::listen_notify)) |
| [`RollbackFailed`](crate::Error::RollbackFailed) | a transaction's closure failed and its `ROLLBACK` failed too, carried out of a [`TransactionError`](crate::TransactionError) by `?`: both errors, the closure's as `cause` |
| [`Custom`](crate::Error::Custom) | a message of your own, such as a hook's refusal |

```rust
# use pgorm_guide_support::{cake, customer};
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::bakery("errors_pgorm").await;
# let db = scratch.pool.get().await?;
use pgorm::{DecodeRaw, Values, entity::prelude::*};

// No cake 99.
let missing = cake::Entity::find_by_id(99).one(&db).await;
assert!(matches!(missing, Err(Error::RecordNotFound)));
assert_eq!(cake::Entity::find_by_id(99).one_opt(&db).await?, None);

// A one-row update needs the row's key.
let keyless = Update::one(customer::ActiveModel {
    name: set("Kim"),
    ..Default::default()
});
assert!(matches!(keyless, Err(Error::PrimaryKeyNotSet)));

// An update of many rows that sets nothing.
let empty = Update::many(cake::Entity)
    .filter(cake::Column::Id.eq(1))
    .exec(&db)
    .await;
assert!(matches!(empty, Err(Error::NothingToSet)));

// An active model with its key left out is not a whole model.
let draft = customer::ActiveModel {
    name: set("Mo"),
    email: set("mo@example.com"),
    notes: set(None),
    ..Default::default()
};
assert!(matches!(draft.try_into_model(), Err(Error::AttrNotSet(field)) if field == "id"));

// A NULL for a field that is not an Option.
let null = ("SELECT NULL::text", Values(vec![]))
    .into_tuple::<String>()
    .one(&db)
    .await;
assert!(matches!(null, Err(Error::Type(_))));
# drop(db);
# scratch.finish().await;
# Ok(())
# }
```

A pool that cannot open a connection reports it when one is asked for, at
[`get`](crate::DatabasePool::get), since building the pool connects to
nothing. The [`PoolError`](crate::PoolError) inside says why; one the server
refused holds the server's error:

```rust
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let scratch = pgorm_guide_support::Scratch::empty("errors_pool").await;
use pgorm::{Error, PoolError, SqlState};

let mut config: pgorm::Config = scratch.url().parse()?;
config.dbname("no_such_bakery");
let pool = pgorm::connect(config);

let refused = pool.get().await;
assert!(matches!(
    &refused,
    Err(Error::Pool(PoolError::Backend(error)))
        if error.code() == Some(&SqlState::INVALID_CATALOG_NAME)
));
# scratch.finish().await;
# Ok(())
# }
```

## Matching errors

`Error`, `SqlError` and the other error enums are `#[non_exhaustive]`: new
kinds of failure can be added, so a `match` on one ends in an arm for the
kinds it does not name. Two errors compare equal when their messages do.

A closure run in a transaction returns its own error, which comes back
inside a [`TransactionError`](crate::TransactionError) that says whether the
closure, the `BEGIN` or `COMMIT`, or the rollback failed. A
`TransactionError` converts into an `Error` for `?`, as a savepoint's does
inside the closure around it;
[Transactions](crate::guide::transactions) covers both.
