# Coming from SeaORM

pgorm began as a fork of SeaORM 1.0. It keeps SeaORM's shape — entities,
models, active models, relations, the query builder — but runs on PostgreSQL
only, and many names have changed or gone. This guide maps each removed or
renamed SeaORM API to what replaces it in pgorm, and says why where the
reason is not obvious.

When a method is gone, the compiler says only "no method named
`find_also_related` found". Stable Rust gives a library no way to add a
hint to that error, so the compiler cannot tell you what to write instead.
This guide can, and so can the documentation search: each old name that
has a replacement is a search alias on it, so typing `find_also_related`
into the search box of pgorm's API documentation finds `related_maybe`.

The examples use the `cake`, `fruit`, `filling` and `vendor` entities from
pgorm's own test fixtures (`pgorm::tests_cfg`), so that every example here
is compiled with the crate. Swap in your own entities. Code blocks that
begin with `// SeaORM` are SeaORM code, shown for comparison and not
compiled.

## Contents

- [Quick reference](#quick-reference)
- [Dependencies and features](#dependencies-and-features)
- [Connecting](#connecting)
- [The pool and a connection are different types](#the-pool-and-a-connection-are-different-types)
- [Running SQL by hand](#running-sql-by-hand)
- [Transactions](#transactions)
- [Entities and their attributes](#entities-and-their-attributes)
- [Derive names](#derive-names)
- [Attribute values that are Rust](#attribute-values-that-are-rust)
- [Active models](#active-models)
- [Insert, update and delete](#insert-update-and-delete)
- [Reading rows](#reading-rows)
- [Related rows: the graph](#related-rows-the-graph)
- [Loading related rows in batches](#loading-related-rows-in-batches)
- [The query builder (sea-query)](#the-query-builder-sea-query)
- [Errors](#errors)
- [Migrations](#migrations)
- [Generating entities](#generating-entities)
- [Testing without a database](#testing-without-a-database)
- [Logging and metrics](#logging-and-metrics)
- [What pgorm adds](#what-pgorm-adds)

## Quick reference

| SeaORM | pgorm |
| --- | --- |
| `sea_orm`, `sea_orm::prelude` | `pgorm`, `pgorm::entity::prelude` |
| `#[sea_orm(...)]` | `#[pgorm(...)]` |
| the expanded entity format | `#[derive(Entity)]` on `Model` and `#[derive(Relation)]` on `Relation` only |
| `DeriveEntityModel`, `DeriveRelation`, `DeriveActiveEnum`, ... | the same without `Derive`: see [Derive names](#derive-names) |
| `DerivePrimaryKey`, `DeriveActiveModel`, `DeriveCustomColumn` | gone: `#[derive(Entity)]` writes the key and the active model |
| `column_type = "Text"`, `belongs_to = "super::cake::Entity"`, ... | Rust written bare: see [Attribute values that are Rust](#attribute-values-that-are-rust) |
| a hand-written `ColumnTrait::select_as` / `save_as` | `select_with = f`; `save_with = f, save_array_with = g` |
| `sea_query` | `pgorm::pgorm_query` |
| `Database::connect(url)` | `pgorm::connect(config)` |
| `ConnectOptions` | `connect_with_builder`, `connect_with` |
| `DatabaseConnection` (a pool) | `DatabasePool` |
| a connection from the pool | `pool.get().await?`, a `DatabaseConnection` |
| `DbConn` | `DatabasePool` |
| `DbErr` | `Error` |
| `SqlErr`, `err.sql_err()` | `SqlError`, `err.sql_error()` |
| `DbBackend`, `DatabaseBackend` | gone: PostgreSQL only |
| `Statement`, `StatementBuilder` | SQL text plus its parameters |
| `Statement::from_sql_and_values(..)` | `(sql, values)` |
| `execute_unprepared` | `batch_execute` |
| `db.stream(stmt)` | `query_raw` |
| `query_one` (at most one row) | `query_opt` |
| `db.begin()`, `txn.commit()` | `conn.transaction(async \|txn\| ..)`: `Ok` commits |
| `begin_with_config`, `transaction_with_config` | `transaction_with(TransactionMode, ..)` |
| `Set(value.to_owned())` | `set(value)` or `value.into()` |
| `active_model.save(db)` | `insert(db)` or `update(db)` |
| `Entity::insert(am)` | `Insert::one(am)` |
| `Entity::insert_many(iter)` | `Insert::many(iter)` |
| `Entity::update(am)` | `Update::one(am)?` |
| `Entity::update_many()` | `Update::many(Entity)` |
| `Entity::delete(am)` | `Delete::one(am)?` |
| `Entity::delete_many()` | `Delete::many(Entity)` |
| `exec(db)` returning `InsertResult` | `exec_returning_pk(db)` |
| `res.last_insert_id` | the value `exec_returning_pk` returns |
| `exec_with_returning` | `exec_returning_model`, `exec_returning_models` |
| `exec_without_returning` | `exec` |
| `UpdateResult`, `DeleteResult` | the `u64` `exec` returns |
| `Insert::do_nothing` | `on_empty_do_nothing` |
| `find().one(db)` returning `Option` | `one_opt(db)`; `one(db)` now errors on no row |
| `into_json()` | decode into a model and serialize it |
| `find_also_related(F)` | `graph().related_maybe::<F>()` |
| `find_with_related(F)` | `graph().related_maybe::<F>().all_grouped(db)` |
| `find_also_linked(L)` | `graph()` with `via(..)` or `via_as(..)` hops and `join_maybe` |
| `find_with_linked(L)` | the same, then `all_grouped(db)` |
| `find_also_related(F).limit(n)` | `graph().related_maybe::<F>().limit(n)`: `n` rows |
| `find_with_related(F).limit(n)` | `find().limit(n)`, then `load_many(F, db)` |
| `select_also(F)`, `select_with(F)` | `join_maybe::<F>(rel)` |
| `SelectTwo`, `SelectTwoMany` | `SelectGraph` |
| `cursor_by_other(col)` | `cursor_by_on::<1, _>(col)` |
| `load_many_to_many(F, Via, db)` | `load_many_via(F, db)` |
| `paginate(db, 50)` | `paginate(db, NonZeroU64)` |
| `Paginator::next()` | `Paginator::advance()` |
| `Iden`, `#[derive(DeriveIden)]` | `SqlName`, `#[derive(SqlName)]` |
| `Alias::new("x")` | `alias("x")`, or `Name::runtime(text)` |
| `DynIden`, `SeaRc<dyn Iden>` | `Name` |
| `IntoIden` | `IntoName` |
| `TableRef` | `TableName` (DDL) and `FromItem` (queries) |
| `Expr::cust(..)` | `Expr::raw(..)` |
| `Expr::cust_with_values(..)` | `Expr::template(..)` |
| `stmt.build(PostgresQueryBuilder)` | `stmt.build()` |
| `stmt.to_string(PostgresQueryBuilder)` | `stmt.to_string()` |
| `Schema::new(DbBackend::Postgres)` | `Schema::new()` |
| `SchemaManager` in migrations | the `DatabaseTransaction` itself |
| `down`, `fresh`, `refresh`, `reset` | gone: migrations run forward only |
| `seaql_migrations` | `pgorm_migrations`, adopted on the first `up` |
| `sea-orm-cli generate entity` | `pgorm_codegen::entities_from_sql` |
| `sea-orm-cli generate entity --expanded-format` | gone: codegen writes the compact form only |
| `sea-orm-cli migrate` | a small binary of your own |
| `MockDatabase` | `build()` to check the SQL, or a real database |
| `set_metric_callback` | `metric::InstrumentedPool` |
| `chrono`, `time` types | `jiff` types |
| `BigDecimal` | `rust_decimal::Decimal` |

## Dependencies and features

pgorm is one crate plus a few siblings in the same repository:
`pgorm-migration` for migrations, `pgorm-codegen` for generating entities,
and `pgorm-query`, the fork of sea-query, which pgorm re-exports as
`pgorm::pgorm_query`. pgorm depends on two forks pinned to a git revision,
and crates.io does not accept git dependencies, so pgorm is not published
there; depend on it by git or path.

SeaORM's driver and runtime features are gone. pgorm runs on tokio-postgres
with a deadpool pool, always on tokio, so there is no `sqlx-postgres`, no
`runtime-tokio-rustls` and no `mock`. TLS is a connector you pass in (see
[Connecting](#connecting)), not a feature.

| SeaORM feature | pgorm |
| --- | --- |
| `sqlx-postgres`, `sqlx-mysql`, `sqlx-sqlite` | gone: PostgreSQL through tokio-postgres |
| `runtime-tokio-rustls` and the other `runtime-*` features | gone: pgorm always runs on tokio |
| `with-chrono`, `with-time` | `with-jiff` (on by default) |
| `with-bigdecimal` | gone: `rust_decimal::Decimal` is always available |
| `with-rust_decimal` | gone: always on |
| `with-json`, `with-uuid`, `postgres-array`, `macros` | the same names, on by default |
| `mock`, `proxy` | gone |
| `seaography` | gone |

## Connecting

SeaORM connects with a URL and checks the connection:

```text
// SeaORM
let db: DatabaseConnection = Database::connect("postgres://...").await?;
```

pgorm takes a `tokio_postgres::Config` (re-exported as `pgorm::Config`) and
builds a pool. Building a pool connects to nothing: a wrong host or password
shows up at the first `pool.get()`. If you want to fail at startup, get one
connection there and drop it.

```rust,no_run
use pgorm::DatabasePool;

async fn start() -> Result<DatabasePool, Box<dyn std::error::Error>> {
    let config: pgorm::Config = "postgres://postgres:postgres@localhost/app".parse()?;
    let pool = pgorm::connect(config);

    // Fail now rather than at the first request.
    drop(pool.get().await?);
    Ok(pool)
}
```

`ConnectOptions` is replaced by a closure over deadpool's `PoolBuilder`:

```rust,no_run
use pgorm::connect_with_builder;

fn start() -> Result<pgorm::DatabasePool, Box<dyn std::error::Error>> {
    let config: pgorm::Config = "postgres://postgres:postgres@localhost/app".parse()?;
    Ok(connect_with_builder(config, |builder| builder.max_size(16))?)
}
```

| SeaORM | pgorm |
| --- | --- |
| `max_connections(n)` | `builder.max_size(n)` |
| `connect_timeout`, `acquire_timeout`, `idle_timeout` | deadpool's `create_timeout`, `wait_timeout`, `recycle_timeout`, which also need `builder.runtime(..)` |
| `set_schema_search_path("s")` | `config.options("-c search_path=s")` on the `Config` |
| TLS through an sqlx feature | `connect_with(config, tls, manager, build)` with any tokio-postgres connector, such as `tokio-postgres-rustls` |
| `sqlx_logging(true)` | `metric::LoggingMetrics`, or `log_statement` in PostgreSQL |
| `db.ping()` | `pool.get().await?`, which opens or reuses a connection |
| `db.close()` | `pool.close()` |

`connect_with` is the one entry point that takes everything: the TLS
connector, the `ManagerConfig` (how connections are recycled, how many
prepared statements each caches) and the builder closure. Every type it
needs is re-exported from `pgorm`.

## The pool and a connection are different types

This is the change most likely to surprise you. In SeaORM a
`DatabaseConnection` is really a pool: you clone it, keep it in your
application state, and pass `&db` to every query. In pgorm the pool is
`DatabasePool`, and a `DatabaseConnection` is one connection checked out of
it, returned to the pool when it is dropped.

Keep the `DatabasePool` in your application state (it is cheap to clone)
and take a connection for each unit of work:

```rust,no_run
use pgorm::{entity::prelude::*, tests_cfg::cake};

async fn list_cakes(pool: &DatabasePool) -> Result<Vec<cake::Model>, Error> {
    let db = pool.get().await?;
    cake::Entity::find().all(&db).await
}
```

`DatabasePool` deliberately does not implement `ConnectionTrait`, so
`Entity::find().all(&pool)` does not compile. Running each statement on
whatever connection the pool hands out hides how many connections a request
uses, and it breaks any reasoning about which statements ran together. Two
queries that should run at the same time take two connections:

```rust,no_run
use pgorm::{entity::prelude::*, tests_cfg::{cake, fruit}};

async fn both(pool: &DatabasePool) -> Result<(), Error> {
    let (a, b) = futures::try_join!(pool.get(), pool.get())?;
    let (cakes, fruits) = futures::try_join!(
        cake::Entity::find().all(&a),
        fruit::Entity::find().all(&b),
    )?;
    let _: (Vec<cake::Model>, Vec<fruit::Model>) = (cakes, fruits);
    Ok(())
}
```

## Running SQL by hand

SeaORM wraps SQL in a `Statement` built for a backend. pgorm has no
`Statement`, `StatementBuilder` or `DbBackend`: SQL is text (`&str` or
`String`), and its parameters travel beside it, so the statement is prepared
once per connection and the values are never pasted into the text.

```text
// SeaORM
db.execute(Statement::from_sql_and_values(
    DbBackend::Postgres,
    "UPDATE cake SET name = $1 WHERE id = $2",
    ["Lemon".into(), 1.into()],
)).await?;
```

```rust,no_run
use pgorm::entity::prelude::*;

async fn rename(db: &DatabaseConnection) -> Result<u64, Error> {
    // The number of rows the statement changed.
    db.execute("UPDATE cake SET name = $1 WHERE id = $2", &[&"Lemon", &1_i32])
        .await
}
```

The methods of `ConnectionTrait` follow tokio-postgres: `execute` returns
the row count; `query_one` expects exactly one row, `query_opt` at most one,
`query_all` any number, each as a `tokio_postgres::Row`; `query_raw` streams
the rows, where SeaORM had `stream`. `execute_unprepared` is `batch_execute`,
which runs a whole script of `;`-separated statements and takes no
parameters. There is no `get_database_backend` and no `support_returning`:
the answer is always PostgreSQL, and it always supports `RETURNING`.

To decode raw SQL into a model, pair the text with its values:

```rust,no_run
use pgorm::{entity::prelude::*, pgorm_query::Values, tests_cfg::cake};

async fn by_name(db: &DatabaseConnection) -> Result<Vec<cake::Model>, Error> {
    let sql = r#"SELECT "id", "name" FROM "cake" WHERE "name" = $1"#;
    (sql, Values(vec!["Lemon".into()]))
        .into_model::<cake::Model>()
        .all(db)
        .await
}
```

`cake::Entity::find().from_raw_sql(sql, values)` and
`Model::find_by_statement(sql, values)` likewise take the SQL text and its
values where SeaORM took a `Statement`. A statement built with pgorm-query
gives the pair from `build()`; to execute one directly, wrap each value in
`ValueHolder`:

```rust,no_run
use pgorm::{ValueHolder, entity::prelude::*, pgorm_query::Query, tests_cfg::cake};

async fn rename(db: &DatabaseConnection) -> Result<u64, Error> {
    let (sql, values) = Query::update()
        .table(cake::Entity)
        .value(cake::Column::Name, "Lemon")
        .and_where(cake::Column::Id.eq(1))
        .build();
    db.execute_raw(&sql, values.into_iter().map(ValueHolder)).await
}
```

The `sql!` macro checks a raw SQL literal against PostgreSQL's own grammar at
compile time, if you want a typo in hand-written SQL to fail the build.

## Transactions

| SeaORM | pgorm |
| --- | --- |
| `db.begin()`, then `txn.commit()` or `txn.rollback()` | `conn.transaction(async \|txn\| { .. })`: `Ok` commits, `Err` rolls back |
| `txn.begin()` (savepoint) | `txn.transaction(async \|savepoint\| { .. })` |
| `db.transaction::<_, _, E>(\|txn\| Box::pin(async move { .. }))` | `conn.transaction(async \|txn\| { .. })` |
| `begin_with_config(isolation, access_mode)` | `transaction_with(TransactionMode::.., async \|txn\| { .. })` |
| `transaction_with_config` | `transaction_with` |
| `TransactionError<E>` | the same name and variants, and `RollbackFailed { cause, rollback }` for a `ROLLBACK` that failed after the closure's error |

A transaction exists only inside a closure. There is no `begin` returning a
handle, and no `commit` or `rollback` to call: what the closure returns ends
the transaction, and a closure whose future is dropped before it finishes — a
timeout, a cancelled task — rolls back. The closure's transaction borrows its
connection exclusively, so nothing else can run on that connection until the
transaction ends, and the compiler checks it. It takes an async closure;
there is no `Box::pin`, and the call runs inside `tokio::spawn`:

```rust,no_run
use pgorm::{TransactionError, entity::prelude::*, tests_cfg::cake};

async fn add(pool: &DatabasePool) -> Result<cake::Model, TransactionError<Error>> {
    let mut conn = pool.get().await.map_err(TransactionError::Connection)?;
    conn.transaction(async |txn| {
        cake::ActiveModel {
            name: set("Lemon"),
            ..Default::default()
        }
        .insert(&txn)
        .await
    })
    .await
}
```

A savepoint is the same call on the transaction, and rolls back alone:

```rust,no_run
use pgorm::{TransactionError, entity::prelude::*, tests_cfg::cake};

async fn add(pool: &DatabasePool) -> Result<(), TransactionError<Error>> {
    let mut conn = pool.get().await.map_err(TransactionError::Connection)?;
    conn.transaction(async |txn| {
        let refused = txn
            .transaction(async |savepoint| {
                cake::ActiveModel {
                    name: set("Lemon"),
                    ..Default::default()
                }
                .insert(&savepoint)
                .await
            })
            .await;
        if refused.is_err() {
            cake::ActiveModel {
                name: set("Plain"),
                ..Default::default()
            }
            .insert(&txn)
            .await?;
        }
        Ok::<_, Error>(())
    })
    .await
}
```

`transaction_with` takes a `TransactionMode`, which lists only the combinations
PostgreSQL acts on: `Default`, `ReadWrite { isolation }`,
`ReadOnly { isolation }` and `DeferrableSnapshot`.
`transaction_with_retry` reruns a closure when a serializable transaction
fails with a retryable error; SeaORM has no equivalent.

## Entities and their attributes

An entity is written one way: SeaORM's compact form, the composite derive
`Entity` (SeaORM's `DeriveEntityModel`) on the `Model` struct and `Relation`
on the `Relation` enum, with `#[pgorm(...)]` in place of `#[sea_orm(...)]`.
The expanded form, an `Entity` struct, `Column` and `PrimaryKey` enums and
their trait impls written out by hand, is gone, and with it
`DerivePrimaryKey`, `DeriveActiveModel` and `DeriveCustomColumn`. What an
expanded entity said in a trait method, an attribute says:

- `column_type` takes any expression of type `ColumnType`, written bare:
  `column_type = ColumnType::Text`, or a helper's call. A string there is a
  compile error.
- `select_with = f` reads a column through `fn f(Expr) -> SimpleExpr`, and
  `save_with = f` writes a value through one, beside `save_array_with = g`
  for the array `= ANY` and `<> ALL` compare it with. They stand for the
  `select_as` and `save_as` methods; the attributes of those names still take
  a type to cast to.
- `on_condition` takes a closure of the two tables as the join binds them,
  `|left, right| ..`, as well as an expression that ignores them.
- An entity whose table or schema name is computed at run time keeps a
  hand-written part: its own `Entity` struct holding what the name is computed
  from, an `EntityName` impl, and the `EntityTrait` derive on the struct,
  beside an `Entity` derive given no `table_name`.

```rust
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::SimpleExpr;

#[derive(Copy, Clone, Default, Debug, EntityTrait)]
pub struct Entity {
    pub tenant: &'static str,
}

impl EntityName for Entity {
    fn schema_name(&self) -> Option<&str> {
        Some(self.tenant)
    }

    fn table_name(&self) -> &str {
        "cake"
    }
}

fn as_label(expr: Expr) -> SimpleExpr {
    expr.cast_as(Name::runtime("label"))
}

#[derive(Clone, Debug, PartialEq, Entity)]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    #[pgorm(column_type = ColumnType::String(StringLen::N(40)), select_with = as_label)]
    pub name: String,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
```

Some details differ besides:

- An unknown key in `#[pgorm(...)]` is a compile error. SeaORM ignored it, so
  a typo such as `auto_incremnt = false` silently changed the column.
- Date and time fields use jiff, not chrono or time. The prelude names are
  `Date`, `Time` and `DateTime` (`jiff::civil`) for `date`, `time` and
  `timestamp`, and `DateTimeWithTimeZone`, which is `jiff::Timestamp`, for
  `timestamptz`.
- `numeric` is `rust_decimal::Decimal`; there is no `BigDecimal`.
- Integer columns are `i16`, `i32` and `i64`. PostgreSQL has no unsigned
  integers, so a `u8`, `u16` or `u64` field cannot be read, and a `u32`
  field reads an `oid` column. `i8` and `char` fields no longer get a column
  type of their own.
- `DeriveRelatedEntity` and everything Seaography-related are gone.
- `RelationDef::rev()` keeps an `on_condition` closure's two arguments in the
  roles they were written for: a predicate written for `(source, target)`
  still gets the source first after the relation is reversed. SeaORM handed
  it the two swapped, so drop any code that swapped them back.
- A primary key part can be generated by the database with
  `#[pgorm(identity)]` or `#[pgorm(identity_by_default)]`, even inside a
  composite key, and `find_by_id` takes a composite key's parts by reference:
  `find_by_id((1, "x"))` rather than `find_by_id((1, "x".to_owned()))`.
- A column can be generated with `generated_stored = expr` or
  `generated_virtual = expr`, the expression written bare.

An enum can live in a schema of its own. The `ActiveEnum` derive takes
`schema_name`, and `ColumnType::Enum` carries a `schema` field next to `name`
and `variants`, so code that builds or matches `ColumnType::Enum` by hand
needs the new field:

```rust
use pgorm::entity::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, ActiveEnum)]
#[pgorm(rs_type = String, enum_name = "tea", schema_name = "kitchen")]
pub enum Tea {
    #[pgorm(string_value = "EverydayTea")]
    EverydayTea,
    #[pgorm(string_value = "BreakfastTea")]
    BreakfastTea,
}
```

## Derive names

Each derive is named for what it makes or the trait it implements, as Rust's
own derives and serde's are, so most share a name with a trait: a derive and
a trait live in different namespaces, and one `use` brings both. The
`Derive`-prefixed names are gone, with no alias; each replaced by a derive is
a search alias on it. The table includes the names earlier pgorm releases
gave derives SeaORM does not have.

| Old name | pgorm |
| --- | --- |
| `DeriveEntityModel` | `Entity` |
| `DeriveEntity` | `EntityTrait`, on a hand-written `Entity` whose names are computed |
| `DeriveModel` | `Model` |
| `DeriveColumn` | `Column` |
| `DeriveRelation` | `Relation` |
| `DeriveActiveEnum` | `ActiveEnum` |
| `DeriveDisplay` | `ActiveEnum`'s `display` key: `#[pgorm(rs_type = .., db_type = .., display)]` |
| `DeriveIntoActiveModel` | `IntoActiveModel` |
| `DerivePartialModel` | `PartialModel`, which implements `FromQueryResult` too: derive it alone |
| `FromQueryResult` | the same |
| `DeriveValueType` | `ValueType` |
| `FromJsonQueryResult` | `JsonValueType` |
| `DeriveCreatedRange` (pgorm's) | `CreatedRange` |
| `DeriveMigrationName` | `MigrationName`, from `pgorm_migration` |
| `DeriveIden`, and pgorm's `DeriveSqlName` | `SqlName`, pgorm-query's, with `#[iden = ".."]` for `#[sea_orm(iden = "..")]` |
| `DeriveActiveModelBehavior` | `impl ActiveModelBehavior for ActiveModel {}`, written out |
| `DerivePrimaryKey`, `DeriveActiveModel`, `DeriveCustomColumn` | gone: `Entity` writes the key and the active model |
| `DeriveRelatedEntity` | gone, with Seaography |

`display` is opt-in so an enum can implement `Display` itself: with it, each
variant writes its `display_value`, or else its name, and a `display_value`
without it is a compile error. `SqlName` and `StaticName` are the derives
pgorm-query exports, reached through pgorm with no dependency on
pgorm-query; their variant named `Table` is the type's own name, and
`#[iden = ".."]`, `#[method = ".."]` and `#[iden(flatten)]` say what else a
name is.

```rust
use pgorm::entity::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, ActiveEnum)]
#[pgorm(rs_type = String, enum_name = "tea", display)]
pub enum Tea {
    #[pgorm(string_value = "EverydayTea", display_value = "Everyday")]
    EverydayTea,
    #[pgorm(string_value = "BreakfastTea")]
    BreakfastTea,
}

#[derive(SqlName)]
enum Post {
    Table,
    #[iden = "headline"]
    Title,
}

assert_eq!(Tea::EverydayTea.to_string(), "Everyday");
assert_eq!(Tea::BreakfastTea.to_string(), "BreakfastTea");
assert_eq!(Post::Table.to_string(), "post");
assert_eq!(Post::Title.to_string(), "headline");
```

## Attribute values that are Rust

An attribute value that is Rust code is written as Rust code: a path as a
path, a type as a type, an expression as an expression. It is taken as
written, with no prefix the derive adds, so `on_delete` names
`ForeignKeyAction::Cascade` where SeaORM's string named `Cascade`, and the
prelude exports `ForeignKeyAction`, `Deferrability`, `Enforcement`,
`ConditionType` and `ArrayType` for the attributes to name. A string where
code belongs is a compile error at the string, saying to remove the quotes.

| SeaORM | pgorm |
| --- | --- |
| `belongs_to = "super::cake::Entity"` (and `has_one`, `has_many`) | `belongs_to = super::cake::Entity` |
| `from = "Column::CakeId"`, `to = "super::cake::Column::Id"` | `from = Column::CakeId`, `to = super::cake::Column::Id` |
| `from = "(Column::A, Column::B)"` | `from = (Column::A, Column::B)` |
| `on_update = "Cascade"`, `on_delete = "SetNull"` | `on_update = ForeignKeyAction::Cascade`, `on_delete = ForeignKeyAction::SetNull` |
| `on_condition = r#"Column::Name.like("%a%")"#` | `on_condition = Column::Name.like("%a%")`, or `\|left, right\| ..` |
| `condition_type = "any"` | `condition_type = ConditionType::Any` |
| `from_period = "Column::During"` (pgorm's) | `from_period = Column::During` |
| `enforcement = "NotEnforced"`, `deferrability = "DeferrableInitiallyDeferred"` (pgorm's) | `enforcement = Enforcement::NotEnforced`, `deferrability = Deferrability::DeferrableInitiallyDeferred` |
| `column_type = "Text"` | `column_type = ColumnType::Text` |
| `rs_type = "String"` | `rs_type = String` |
| `db_type = "Integer"`, `db_type = "String(StringLen::N(1))"` | `db_type = ColumnType::Integer`, `db_type = ColumnType::String(StringLen::N(1))` |
| `db_type = "Enum"` | no `db_type`: an enum of strings with none is a PostgreSQL enum |
| `array_type = "Int"` | `array_type = ArrayType::Int` |
| `default_expr = "Expr::current_timestamp()"` | `default_expr = Expr::current_timestamp()` |
| `default_value = "-1"` | `default_value = -1`; a string default stays a string, `default_value = "untitled"` |
| `generated_stored = "Expr::col(Column::A).mul(2)"` (pgorm's) | `generated_stored = Expr::col(Column::A).mul(2)` |
| a field's `enum_name = "Name"` | `variant = Name` |
| `#[sea_orm(entity = "Entity")]` on a partial model | `#[pgorm(entity = Entity)]` |
| `from_col = "name"` | `from_col = Column::Name` |
| `from_expr = "Expr::val(1)"` | `from_expr = Expr::val(1)` |

What is SQL or a name stays a string: `table_name`, `schema_name`,
`column_name`, `fk_name`, an `ActiveEnum`'s `enum_name` (the type's SQL
name), `string_value`, `display_value`, `range_name`, `multirange_name`,
`comment`, and the `select_as` and `save_as` cast types; so do the case
styles of `rename` and `rename_all`, which are not Rust.

Each derive refuses a key it does not read, where SeaORM dropped it: a
`table_name` on a `Relation` enum or on a further `Model` is a compile error
naming where it goes. `PartialModel` implements `FromQueryResult` itself, so
the two are not derived together, and `FromQueryResult` alone refuses
`from_col` and `from_expr`.

```rust
use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Entity)]
#[pgorm(table_name = "fruit")]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub parent_id: Option<i32>,
    #[pgorm(default_value = -1)]
    pub rank: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, Relation)]
pub enum Relation {
    #[pgorm(
        belongs_to = Entity,
        from = Column::ParentId,
        to = Column::Id,
        on_delete = ForeignKeyAction::Cascade,
        deferrability = Deferrability::DeferrableInitiallyDeferred
    )]
    Parent,
}

impl ActiveModelBehavior for ActiveModel {}

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, ActiveEnum)]
#[pgorm(rs_type = i32, db_type = ColumnType::Integer)]
pub enum Ripeness {
    #[pgorm(num_value = 0)]
    Green,
    #[pgorm(num_value = 1)]
    Ripe,
}
```

## Active models

| SeaORM | pgorm |
| --- | --- |
| `name: Set("Apple".to_owned())` | `name: set("Apple")` or `name: "Apple".into()` |
| `ActiveValue::Set(v)` in a `match` | the same |
| `model.into_active_model()` | the same, or `model.into_active()` with no type annotation |
| `active_model.save(db)` | `insert(db)` or `update(db)` |

`set(..)` converts into the column's type, so a `&str` fills a `String`
column without `.to_owned()`.

`save` is gone. It guessed between insert and update by asking whether the
primary key was set, and it could not tell a key you set from a key read
from the database, so a model with a hand-chosen key could never be inserted
through it. Say which one you mean:

```rust,no_run
use pgorm::{entity::prelude::*, tests_cfg::cake};

async fn insert_then_update(db: &DatabaseConnection) -> Result<(), Error> {
    let lemon = cake::ActiveModel {
        name: set("Lemon"),
        ..Default::default()
    }
    .insert(db)
    .await?;

    let mut renamed = lemon.into_active();
    renamed.name = set("Lemon drizzle");
    renamed.update(db).await?;
    Ok(())
}
```

## Insert, update and delete

The CRUD shortcuts on `Entity` are gone. The builders are named for what
they build, and each comes in a one-row and a many-row form. The terminals
say what they return:

| Terminal | Returns |
| --- | --- |
| `exec` | the number of rows the statement changed, as `u64`, with no `RETURNING` |
| `exec_returning_pk` | the one row's primary key |
| `exec_returning_pks` | every row's primary key, in insertion order |
| `exec_returning_model` | the one row as a model |
| `exec_returning_models` | every row as a model |

```rust,no_run
use pgorm::{entity::prelude::*, tests_cfg::cake};

async fn crud(db: &DatabaseConnection) -> Result<(), Error> {
    // SeaORM: cake::Entity::insert(am).exec(db).await?.last_insert_id
    let id: i32 = Insert::one(cake::ActiveModel {
        name: set("Lemon"),
        ..Default::default()
    })
    .exec_returning_pk(db)
    .await?;

    // SeaORM: cake::Entity::insert_many(models).exec_with_returning(db)
    let models: Vec<cake::Model> = Insert::many([
        cake::ActiveModel { name: set("Apple"), ..Default::default() },
        cake::ActiveModel { name: set("Pear"), ..Default::default() },
    ])
    .exec_returning_models(db)
    .await?;

    // SeaORM: cake::Entity::update(am).exec(db)
    let updated: cake::Model = Update::one(cake::ActiveModel {
        id: set(id),
        name: set("Lemon drizzle"),
    })?
    .exec_returning_model(db)
    .await?;

    // SeaORM: cake::Entity::update_many().col_expr(..).filter(..).exec(db).await?.rows_affected
    let renamed: u64 = Update::many(cake::Entity)
        .col_expr(cake::Column::Name, Expr::value("Pear tart"))
        .filter(cake::Column::Name.eq("Pear"))
        .exec(db)
        .await?;

    // SeaORM: cake::Entity::delete(am).exec(db).await?.rows_affected
    let deleted: u64 = Delete::one(updated)?.exec(db).await?;

    // SeaORM: cake::Entity::delete_many().filter(..).exec(db)
    Delete::many(cake::Entity)
        .filter(cake::Column::Name.starts_with("Apple"))
        .exec(db)
        .await?;

    let _ = (models, renamed, deleted);
    Ok(())
}
```

Points to watch:

- There is no `InsertResult`, `UpdateResult` or `DeleteResult`, and no
  `last_insert_id`. The key comes from `RETURNING`, so it is the key of the
  row that was written, even when `ON CONFLICT DO UPDATE` updated an existing
  row instead.
- `Insert::one` has only the one-row terminals and `Insert::many` only the
  many-row ones. SeaORM offered the one-row terminals on a batch too, where
  `exec` reported only the last row's key.
- `Update::one` and `Delete::one` return a `Result`: a model whose primary
  key is not set is refused when the statement is built
  (`Error::PrimaryKeyNotSet`), not when it runs. `UpdateOne` has no `exec`,
  because updating one row by key always reads the row back.
- An update or delete that matches nothing returns `Ok(0)`;
  `DbErr::RecordNotUpdated` is gone. `UpdateOne::exec_returning_model` still
  fails with `Error::RecordNotFound` when the row is not there.
- `Insert::many` over an empty list writes nothing, sends no SQL, and returns
  `Ok(0)` from `exec` (an empty `Vec` from the returning terminals). SeaORM
  returned an error.
- `Insert::do_nothing` is `on_empty_do_nothing`, which answers an empty
  insert with `TryInsertResult::Empty`. `on_conflict_do_nothing` adds
  `ON CONFLICT (primary key) DO NOTHING`; both return a `TryInsert`.
  `OnConflict` needs no `.to_owned()` before it is passed to `on_conflict`.
- `UPDATE` and `DELETE` take no `ORDER BY` or `LIMIT`, because PostgreSQL
  refuses both. Filter on a subquery instead.

## Reading rows

`one` has changed meaning. In SeaORM it returns `Option<Model>`. In pgorm it
returns the model and fails with `Error::RecordNotFound` when no row
matches; `one_opt` returns the `Option`:

```rust,no_run
use pgorm::{entity::prelude::*, tests_cfg::cake};

async fn read(db: &DatabaseConnection) -> Result<(), Error> {
    let first: cake::Model = cake::Entity::find_by_id(1).one(db).await?;
    let maybe: Option<cake::Model> = cake::Entity::find_by_id(2).one_opt(db).await?;
    let _ = (first, maybe);
    Ok(())
}
```

Porting tip: search for `.one(` and decide at each call site whether no row
is an error (keep `one`, drop the `Option` handling) or a normal answer
(use `one_opt`).

Projections have a one-call form. `select([..])` clears the default column
list and projects what you give it, so it replaces `select_only()` followed
by `column` calls (both still exist). Once the columns are yours, the rows no
longer have the model's shape, so `all` and `one` are not available until
you name what to decode into with `into_tuple`, `into_model` or
`into_values`. SeaORM let `all` compile and failed at run time.

```rust
use pgorm::{alias, entity::prelude::*, tests_cfg::cake};

let query = cake::Entity::find()
    .select(cake::Column::Name)
    .column_as(cake::Column::Id.count(), alias("count"))
    .group_by(cake::Column::Name);

assert_eq!(
    query.build().0,
    r#"SELECT "cake"."name", COUNT("cake"."id") AS "count" FROM "cake" GROUP BY "cake"."name""#
);
let _decoded = query.into_tuple::<(String, i64)>();
```

An alias is no longer a plain string: `column_as(expr, "count")` is
`column_as(expr, alias("count"))`, and a name computed at run time is
`Name::runtime(text)`. pgorm will not turn an arbitrary `String` into an
identifier by inference, so every place one does is easy to find.

Comparing two columns used to need `Expr::col(..).equals(..)`. Columns now
have `eq_col`, `ne_col`, `gt_col`, `gte_col`, `lt_col` and `lte_col`, and
`eq_expr` for an expression:

```rust
use pgorm::{entity::prelude::*, tests_cfg::{cake, fruit}};

assert_eq!(
    cake::Entity::find()
        .filter(cake::Column::Id.eq_col(fruit::Column::CakeId))
        .build()
        .0,
    r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."id" = "fruit"."cake_id""#
);
```

Other changes:

- `into_json()` is gone. Derive `serde::Serialize` on the model (pgorm-codegen
  can do it for you) and serialize what you read.
- `paginate` takes its page size as a `NonZeroU64`, so a page size of zero
  cannot be written. `Paginator::next()` is `advance()`; it never did any
  I/O, and next to `fetch_and_next` the old name read like a fetch.
- Paginating raw SQL can fail: `SelectorRaw::paginate` and `count` return a
  `Result`, because pgorm parses the SQL to wrap it in a page.
- Cursor pagination (`cursor_by`, `before`, `after`, `first`, `last`) is
  unchanged; `before_with` and `after_with` also take the tiebreak columns,
  so a page can end in the middle of rows that share an order value.

```rust
use std::num::NonZeroU64;

// A page size of zero fails to compile here.
const PAGE_SIZE: NonZeroU64 = NonZeroU64::new(50).unwrap();
```

## Related rows: the graph

This is the largest change. SeaORM reads related rows with
`find_also_related`, `find_with_related`, `find_also_linked` and
`find_with_linked`, which return `SelectTwo` or `SelectTwoMany`. pgorm
replaces all four, and both types, with one builder: the graph. You start it
with `Entity::graph()` instead of `Entity::find()`, then add each related
entity as a *slot*:

| Method | Join | Each row decodes as |
| --- | --- | --- |
| `related_maybe::<F>()` | the path `Related<F>` describes, `LEFT JOIN` | `Option<F::Model>` |
| `related_one::<F>()` | the same path, every hop `INNER JOIN` | `F::Model` |
| `join_maybe::<F>(rel)` | `LEFT JOIN` on one relation | `Option<F::Model>` |
| `join_one::<F>(rel)` | `INNER JOIN` on one relation | `F::Model` |
| `via(rel)` | a hop joined but not decoded, such as a junction table | nothing |
| `via_as(rel, alias)` | the same, with the table under an alias | nothing |
| `join_maybe_as`, `join_one_as` | the same, with the table under an alias | as above |

A graph holds up to six slots, so one query can read a row with up to six
related entities. The row type follows from the slots, so the compiler
checks it.

### `find_also_related`

```text
// SeaORM
let rows: Vec<(cake::Model, Option<fruit::Model>)> =
    cake::Entity::find().find_also_related(fruit::Entity).all(db).await?;
```

```rust,no_run
use pgorm::{entity::prelude::*, tests_cfg::{cake, fruit}};

async fn cakes_and_fruit(db: &DatabaseConnection) -> Result<(), Error> {
    let rows: Vec<(cake::Model, Option<fruit::Model>)> = cake::Entity::graph()
        .related_maybe::<fruit::Entity>()
        .all(db)
        .await?;
    let _ = rows;
    Ok(())
}
```

`related_maybe` follows `Related<F>`, junction included, exactly as
`find_also_related` did. When every row must have a match, use
`related_one`: it follows the same path with every hop an `INNER JOIN`, so a
row with no match is left out and the slot decodes without the `Option`:

```rust,no_run
use pgorm::{entity::prelude::*, tests_cfg::{cake, fruit}};

async fn fruit_with_cake(db: &DatabaseConnection) -> Result<(), Error> {
    let rows: Vec<(fruit::Model, cake::Model)> = fruit::Entity::graph()
        .related_one::<cake::Entity>()
        .all(db)
        .await?;
    let _ = rows;
    Ok(())
}
```

`join_one` does the same for a relation that `Related` doesn't describe.

### `find_with_related`

`all_grouped` returns each root once with its related models collected
beneath it. It exists when the graph has exactly one optional slot, `via`
hops allowed, so a many-to-many through a junction works the same way:

```rust,no_run
use pgorm::{entity::prelude::*, tests_cfg::{cake, filling, fruit}};

async fn grouped(db: &DatabaseConnection) -> Result<(), Error> {
    let with_fruit: Vec<(cake::Model, Vec<fruit::Model>)> = cake::Entity::graph()
        .related_maybe::<fruit::Entity>()
        .order_by_asc(cake::Column::Name)
        .all_grouped(db)
        .await?;

    // Through the cake_filling junction, which Related<filling::Entity> names.
    let with_filling: Vec<(cake::Model, Vec<filling::Model>)> = cake::Entity::graph()
        .related_maybe::<filling::Entity>()
        .all_grouped(db)
        .await?;
    let _ = (with_fruit, with_filling);
    Ok(())
}
```

SeaORM put its own `ORDER BY` on the primary key *before* yours, so its
order won and yours only broke ties. `all_grouped` puts the primary key
*after* yours, so your ordering wins and the key only breaks ties.

### `find_also_linked` and `find_with_linked`

A `Linked` chain is a list of relations. In a graph, every hop but the last
is a `via`, and the last is the slot:

```rust,no_run
use pgorm::{entity::prelude::*, tests_cfg::{cake, cake_filling, filling, vendor}};

async fn cake_vendors(db: &DatabaseConnection) -> Result<(), Error> {
    // SeaORM: cake::Entity::find().find_also_linked(CakeToFillingVendor)
    let rows: Vec<(cake::Model, Option<vendor::Model>)> = cake::Entity::graph()
        .via(cake_filling::Relation::Cake.def().rev())
        .via(cake_filling::Relation::Filling.def())
        .join_maybe::<vendor::Entity>(filling::Relation::Vendor.def())
        .all(db)
        .await?;
    let _ = rows;
    Ok(())
}
```

A `via` hop joins its table under the table's own name, so a chain that
passes through the same table twice needs the second visit aliased.
`via_as` is `via` with an alias, as `join_maybe_as` and `join_one_as` are for
a slot. The relation after an aliased hop starts from the alias, which you
say with `from_alias`. Here each cake reads the other cakes that share a
filling with it, through `cake_filling` twice:

```rust,no_run
use pgorm::{alias, entity::prelude::*, tests_cfg::{cake, cake_filling}};

async fn sharing_a_filling(db: &DatabaseConnection) -> Result<(), Error> {
    let shared = alias("shared");
    let siblings: Vec<(cake::Model, Vec<cake::Model>)> = cake::Entity::graph()
        .via(cake_filling::Relation::Cake.def().rev())
        .via(cake_filling::Relation::Filling.def())
        .via_as(cake_filling::Relation::Filling.def().rev(), shared)
        .join_maybe_as::<cake::Entity>(
            cake_filling::Relation::Cake.def().from_alias(shared),
            alias("sibling"),
        )
        .all_grouped(db)
        .await?;
    let _ = siblings;
    Ok(())
}
```

If you already have `Linked` types, one small function turns any of them
into a graph. Like SeaORM's linked joins, it gives every hop its own alias
(`r0`, `r1`, ...), so a chain that visits a table twice works too:

```rust,no_run
use pgorm::{LinkedAlias, Opt, SelectGraph, entity::prelude::*, tests_cfg::{cake, entity_linked, vendor}};

fn linked<L: Linked>(link: L) -> Option<SelectGraph<L::FromEntity, (Opt<L::ToEntity>,)>> {
    let mut hops = link.link();
    let last = hops.pop()?;
    let end = hops.len();
    let mut graph = L::FromEntity::graph();
    for (i, hop) in hops.into_iter().enumerate() {
        graph = graph.via_as(from_previous(hop, i), LinkedAlias::hop(i));
    }
    Some(graph.join_maybe_as::<L::ToEntity>(from_previous(last, end), LinkedAlias::hop(end)))
}

// Hop `i` starts from the alias of hop `i - 1`; the first starts at the root.
fn from_previous(hop: RelationDef, i: usize) -> RelationDef {
    match i.checked_sub(1) {
        Some(previous) => hop.from_alias(LinkedAlias::hop(previous)),
        None => hop,
    }
}

async fn cake_vendors(db: &DatabaseConnection) -> Result<(), Error> {
    if let Some(graph) = linked(entity_linked::CakeToFillingVendor) {
        // find_also_linked:
        let rows: Vec<(cake::Model, Option<vendor::Model>)> = graph.clone().all(db).await?;
        // find_with_linked:
        let grouped: Vec<(cake::Model, Vec<vendor::Model>)> = graph.all_grouped(db).await?;
        let _ = (rows, grouped);
    }
    Ok(())
}
```

A filter or an ordering on an aliased table names the alias, as in
`Expr::col((LinkedAlias::hop(2), vendor::Column::Name))`, as it did with
SeaORM's `r0`, `r1` aliases.

### Moving the rest of the query over

- There is no conversion from `Select<E>` to a graph. Start from
  `Entity::graph()` and move the `filter` and `order_by` calls across; the
  graph has both.
- A graph's columns come from its slots, so it has no `select_only`,
  `column` or `select`. To read a different shape, use a `Select` with joins
  and `into_model`.
- A graph has no `one`, because one root can produce several rows. Use
  `one_opt` for the first row.
- `limit` and `offset` work on a graph, and count rows rather than roots
  (see below).
- `cursor_by_other` is `cursor_by_on::<1, _>(column)`, where `1` is the slot's
  position (the root is `0`). The cursor adds every slot's primary key as a
  tiebreak, so rows that share an order value still page correctly.
- `ModelTrait::find_related` and `find_linked`, which read one model's
  related rows, are unchanged.

`limit` and `offset` on a graph count rows of the join, as they did on
`SelectTwo`: a cake with three fruit uses three rows of `limit(10)`. A graph
you have limited or offset has no `all_grouped`, `paginate`, `count` or
`cursor_by`. Each of those sets the rows itself, and a grouped read of ten
rows would end with a cake whose fruit list is cut short, which is what
`find_with_related(..).limit(10)` silently gave you in SeaORM. To read ten
cakes with all their fruit, limit the cakes and load the fruit:

```rust,no_run
use pgorm::{entity::prelude::*, tests_cfg::{cake, fruit}};

async fn first_ten(db: &DatabaseConnection) -> Result<(), Error> {
    // Ten rows: a cake appears once for each fruit it has.
    let rows: Vec<(cake::Model, Option<fruit::Model>)> = cake::Entity::graph()
        .related_maybe::<fruit::Entity>()
        .order_by_asc(cake::Column::Id)
        .limit(10)
        .all(db)
        .await?;

    // Ten cakes, each with all of its fruit.
    let cakes: Vec<cake::Model> = cake::Entity::find()
        .order_by_asc(cake::Column::Id)
        .limit(10)
        .all(db)
        .await?;
    let fruit: Vec<Vec<fruit::Model>> = cakes.load_many(fruit::Entity, db).await?;
    let _ = (rows, fruit);
    Ok(())
}
```

For pages, use `paginate` or `cursor_by(..).first(n)` on a graph without a
limit; they count rows in the same way.

## Loading related rows in batches

`LoaderTrait` keeps `load_one` and `load_many`.
`load_many_to_many(F, Via, db)` is `load_many_via(F, db)`: the junction is
the one `Related<F>` already names, so it is not passed in.

```rust,no_run
use pgorm::{entity::prelude::*, tests_cfg::{cake, filling, fruit}};

async fn load(db: &DatabaseConnection) -> Result<(), Error> {
    let cakes: Vec<cake::Model> = cake::Entity::find().all(db).await?;
    let fruit: Vec<Vec<fruit::Model>> = cakes.load_many(fruit::Entity, db).await?;
    let fillings: Vec<Vec<filling::Model>> = cakes.load_many_via(filling::Entity, db).await?;
    let _ = (fruit, fillings);
    Ok(())
}
```

The loaders now honour the whole relation, including an `on_condition` and
`condition_type` written on it. `load_one` still gives `None` to a model
with no related row, but two things that SeaORM let pass are now errors:
two different related rows for one key, which contradicts a has-one
relation, and a row whose key does not equal any input key as a Rust value
(for example an `i64` column read back against an `i32` field), which SeaORM
silently turned into `None`.

## The query builder (sea-query)

pgorm-query is a fork of sea-query with everything that is not PostgreSQL
removed. It is re-exported as `pgorm::pgorm_query`, and the prelude brings
in `Expr`, `Func`, `Name` and `alias`.

| sea-query | pgorm-query |
| --- | --- |
| `Iden`, `#[derive(Iden)]` | `SqlName`, `#[derive(SqlName)]`, also reached as `pgorm::SqlName` |
| `IdenStatic` | `StaticName` |
| `Alias::new("x")` | `alias("x")` for a name written in the program, `Name::runtime(text)` for one computed at run time |
| `DynIden`, `SeaRc<dyn Iden>` | `Name` |
| `IntoIden` | `IntoName` |
| a `&str` or `String` used as a name | `alias(..)` or `Name::runtime(..)`: no implicit conversion |
| `TableRef` | `TableName` in DDL, `FromItem` in queries |
| `Expr::cust("..")` | `Expr::raw("..")`, which takes a `&'static str` |
| `Expr::cust_with_values(..)`, `cust_with_expr(s)` | `Expr::template(..)`, `template_with_expr(s)`, which check the placeholders |
| `Expr::asterisk()` | `Asterisk` |
| `build(PostgresQueryBuilder)` | `build()` |
| `to_string(PostgresQueryBuilder)` | `to_string()` |
| `or_where`, `Cond`, `any!`, `all!` | `Condition::any()` and `Condition::all()` |
| `limit` and `order_by` on `UPDATE`/`DELETE` | gone: PostgreSQL refuses them |
| MySQL- and SQLite-only types and options | gone |

`build()` gives the SQL with `$1`, `$2`, ... placeholders and the values to
bind; it is what you execute. `to_string()` writes the values into the SQL,
which is useful in tests and logs.

## Errors

| SeaORM | pgorm |
| --- | --- |
| `DbErr` | `Error` |
| `DbErr::RecordNotFound(String)` | `Error::RecordNotFound` |
| `DbErr::RecordNotUpdated` | gone: an update that matches nothing returns `Ok(0)` |
| `DbErr::Conn`, `DbErr::Exec`, `DbErr::Query` from the database | `Error::Postgres`, the server's or the driver's error |
| `DbErr::ConnectionAcquire` | `Error::Pool` |
| `DbErr::Custom`, `Type`, `Json`, `AttrNotSet`, `RecordNotInserted`, `UnpackInsertId`, `ConvertFromU64` | the same names |
| `DbErr::UpdateGetPrimaryKey` | `Error::PrimaryKeyNotSet`, raised when the statement is built |
| `err.sql_err()` → `SqlErr` | `err.sql_error()` → `SqlError` |
| `SqlErr::UniqueConstraintViolation(String)` | `SqlError::UniqueConstraintViolation(ConstraintViolation)`, with the constraint and table names |

`SqlError` also classifies foreign-key, check, not-null and exclusion
violations. Builder mistakes caught before anything is sent are
`Error::QueryBuilder`.

## Migrations

`pgorm-migration` replaces `sea-orm-migration`. It runs migrations forward
only.

| sea-orm-migration | pgorm-migration |
| --- | --- |
| `async fn up(&self, manager: &SchemaManager)` | `async fn up(&self, tx: &DatabaseTransaction<'_>)` |
| `async fn down(..)` | gone |
| `manager.create_table(stmt)` | `tx.batch_execute(&stmt.to_string())` |
| `manager.has_table(..)`, `has_column(..)` | query `information_schema` yourself |
| `Migrator::up(&db, None)` | `Migrator::up(pool, None)` with the `DatabasePool` |
| `Migrator::down`, `fresh`, `refresh`, `reset` | gone |
| `cli::run_cli(Migrator)` | a small `main` of your own; the template has one |
| `seaql_migrations` table | `pgorm_migrations`, adopted automatically |
| `#[derive(DeriveMigrationName)]` | `#[derive(MigrationName)]` |
| `#[derive(DeriveIden)]` | `#[derive(SqlName)]` |

A migration gets the transaction the whole batch runs in and builds its SQL
with pgorm-query:

```rust
use pgorm_migration::prelude::*;

#[derive(MigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, tx: &DatabaseTransaction<'_>) -> Result<(), Error> {
        let table = Table::create(Post::Table)
            .if_not_exists()
            .col(ColumnDef::new(Post::Id).integer().not_null().auto_increment())
            .primary_key(Post::Id)
            .col(ColumnDef::new(Post::Title).string().not_null())
            .to_owned();
        tx.batch_execute(&table.to_string()).await
    }
}

#[derive(SqlName)]
enum Post {
    Table,
    Id,
    Title,
}

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(Migration)]
    }
}

async fn migrate(pool: DatabasePool) -> Result<(), Error> {
    Migrator::up(pool, None).await
}
```

There is no `SchemaManager`: it existed mostly to make reversible DDL
expressible, and `down` is gone. A `down` written months earlier is rarely
run, cannot bring back data its `up` dropped, and gives false confidence
when something goes wrong. Fix a mistake with a new migration.

`up` runs every pending migration in one transaction, under an advisory lock
so that two deploys starting together queue instead of colliding. If any
migration fails, the whole batch rolls back.

Your existing history carries over. A migration's name is its file name, as
in SeaORM, and the ledger keeps the same `version` and `applied_at` columns.
The first `up` finds the `seaql_migrations` table and renames it to
`pgorm_migrations` in place, so nothing re-runs. To keep the old name
instead, return it from `migration_table_name()`. A migration can also
report a `checksum()`, so that editing a migration after it ran is caught.

The starter crate at `pgorm-migration/template/migration/` has a `main`
that runs `up` and `status`.

## Generating entities

There is no CLI. Entity generation is the `pgorm-codegen` library: give it
your schema as SQL, the `CREATE` statements for its tables and types, and
write the files it returns. It is a separate crate, so this example is not
compiled here:

```ignore
use pgorm_codegen::{EntityWriterOptions, WithSerde, entities_from_sql};

let ddl = std::fs::read_to_string("schema.sql")?;
let output = entities_from_sql(&ddl, EntityWriterOptions {
    with_serde: WithSerde::Both,
    ..Default::default()
})?;
for file in output.files {
    std::fs::write(format!("src/entities/{}", file.name), file.content)?;
}
```

The schema is read with PostgreSQL's own parser, so it is read as
PostgreSQL 18 reads it. Enums are told apart by schema as well as name.
What it cannot carry into an entity is refused by name rather than skipped,
and that includes the output of `pg_dump --schema-only`, which adds its keys
with `ALTER TABLE` and opens with `SET`s. Write the keys into each
`CREATE TABLE` instead.

## Testing without a database

`MockDatabase` and its mock results are gone, together with the `mock`
feature. To check the SQL a query produces, `build()` it and compare; no
database is needed:

```rust
use pgorm::{entity::prelude::*, pgorm_query::Value, tests_cfg::cake};

let (sql, values) = cake::Entity::find()
    .filter(cake::Column::Name.contains("chocolate"))
    .build();
assert_eq!(
    sql,
    r#"SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."name" LIKE $1"#
);
assert_eq!(values.0, [Value::String(Some(Box::new("%chocolate%".to_owned())))]);
```

To test behaviour, run against a real PostgreSQL. pgorm's own tests create a
throwaway database per test.

## Logging and metrics

`set_metric_callback` and `sqlx_logging` are replaced by an opt-in metrics
layer, `pgorm::metric`. Wrap the pool, and every connection taken from it
reports to the collector you choose; code that does not wrap pays nothing:

```rust,no_run
use pgorm::{entity::prelude::*, metric::{InstrumentedPool, LoggingMetrics}};

async fn instrumented(pool: DatabasePool) -> Result<(), Error> {
    let pool = InstrumentedPool::new(pool, LoggingMetrics);
    let _conn = pool.get().await?;
    Ok(())
}
```

Write your own collector by implementing `MetricsCollector`. See
[METRICS.md](https://github.com/pgorm-rs/pgorm/blob/main/METRICS.md) for the
full guide. For logging every statement, PostgreSQL's own `log_statement`
and `pg_stat_statements` remain the most complete record.

## What pgorm adds

Things with no SeaORM counterpart, worth knowing once the port compiles:

- `RETURNING` the row before and after a write: `exec_returning_change(s)`,
  and `exec_returning_upsert(s)` for `ON CONFLICT DO UPDATE`.
- `pgorm::pipeline`, a composable query builder in the style of PRQL, and the
  `prql!` and `sql!` macros, both checked at compile time.
- `COPY` in and out, as bytes or as rows of values (`CopyTrait`).
- `LISTEN` and `NOTIFY` (`DatabasePool::listener`, `notify`).
- `MERGE`, domains, schemas, ranges and multiranges, and PostgreSQL 18's
  virtual generated columns and temporal keys.
- `DatabasePool::clear_statement_caches()`, for DDL that changes what a
  prepared statement was planned with.
