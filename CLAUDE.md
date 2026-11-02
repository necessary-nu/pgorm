# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

pgorm is a fork of SeaORM focused entirely on PostgreSQL support. It uses tokio-postgres as the database driver and deadpool for connection pooling, with significant performance and stability improvements over the original SeaORM.

## Commands

### Testing
- `cargo nextest run --workspace` - Run the test suite (the preferred runner; skips doctests)
- `cargo test --doc --workspace` - Run the doctests, which nextest skips entirely: a SQL golden written inside a doc example is checked here and nowhere else, so run this whenever rendering changes (~17 minutes). All nine crates pass — 543 examples, 9 of them `ignore`-fenced and never run. Two things to know when reading a result: cargo stops at the first crate that fails, so a failure count is a lower bound and later crates may simply not have run; and a doctest run shares `target/` with any other cargo invocation, so a concurrent `build`/`check`/`nextest` swaps the rlibs underneath it and manufactures failures that do not reproduce. Let it finish alone
- Tests require a running PostgreSQL instance. Set `DATABASE_URL` to the *server* URL with no database path, e.g. `DATABASE_URL=postgres://postgres:postgres@localhost:5432`
- `.env.local` and `.env` are loaded automatically via dotenvy, so `DATABASE_URL` can live in either

### Build and Development
- `cargo build` - Build the project
- `cargo check --workspace` - Check compilation without building
- `cargo clippy` - Run linter (the crate root denies `missing_debug_implementations`, `clippy::unwrap_used`, `clippy::missing_panics_doc`, and the print macros; `missing_docs` warns)
- `cargo fmt --all` - Format

## Architecture

### Workspace Structure
- `pgorm` (root) - Main ORM crate
- `pgorm-macros` - Derive macros for entities and models
- `pgorm-codegen` - Entity source generation from a described schema
- `pgorm-migration` - Migration runner
- `pgorm-pool` - Database connection pool (vendored deadpool-postgres fork)
- `pgorm-query` - SQL query builder (fork of sea-query, PostgreSQL-only), with its own `pgorm-query-attr`, `pgorm-query-derive` and `pgorm-query-derive-internals` members; the last is an ordinary library holding the `SqlName`/`StaticName` expansion, which pgorm-query-derive (naming `pgorm_query::`) and pgorm-macros (naming `pgorm::pgorm_query::`, so a crate depending only on pgorm can derive them) share
- `pgorm-test-support` - Unpublished test infrastructure: the run-scoped names live tests give their databases and roles, and the sweep of crashed runs' leftovers. It talks to the server through tokio-postgres and does not depend on pgorm, so pgorm's tests see one copy of pgorm's types
- `pgorm-sql-macro` - The `sql!` and `prql!` macros, both always in scope (`pgorm::sql`, `pgorm::prql`; there is no gating feature). `sql!` holds a raw SQL string literal to the real PostgreSQL grammar (libpg_query) at compile time; `prql!` compiles PRQL text through prqlc at build time, validates the emitted SQL the same way, and arity-checks its `$N` placeholders against the macro arguments, expanding to `(&'static str, pgorm::Values)`

`pgorm::pipeline` (in-crate module, always compiled) is a PRQL-shaped composable query frontend: relation-to-relation transforms compiled through prqlc's PL AST to PostgreSQL SQL, with bound parameters minted by a per-pipeline binder (branded lifetimes make cross-pipeline placeholder mixing a compile error) and terminals landing on the ordinary decode paths. prqlc (pure Rust) is a plain dependency, pinned to one revision of the necessary-nu fork (`prqlc = { git, rev }`); pg_query is pinned the same way to its fork.

Two dependencies are git forks pinned by exact rev, because a `[patch]` table reaches only the workspace declaring it: prqlc (`necessary-nu/prql`) and pg_query (`necessary-nu/pg_query.rs`, branch `pgorm/postgres-18`: upstream's unreleased 18.0.0 on libpg_query 18.1.0, so every parse in pgorm uses PostgreSQL 18.6's grammar). Each is declared at the same rev in every manifest that names it, allowed in `deny.toml`'s `allow-git`, and held there, with every committed lockfile including the detached workspaces under `security/` and `pgorm-python/`, by `tests/fork_pin_tests.rs`. Moving a fork's rev means editing every declaration and re-resolving every lockfile (`cargo metadata --manifest-path <dir>/Cargo.toml` re-locks one minimally). crates.io refuses git dependencies, so pgorm is not publishable there while they stand.

There is no CLI crate: the inherited `pgorm-cli` was retired (it targeted sqlx/sea-schema and a migration surface this fork dropped). Entity generation is available as a library through `pgorm-codegen`; a starter migration-crate template lives at `pgorm-migration/template/migration/`.

### Core Components
- **Entity System**: `src/entity/` - Entity definitions, active models, relations
- **Query System**: `src/query/` - Query builders
- **Executor**: `src/executor/` - Query execution and result handling
- **Database**: `src/database/` - Connection management and pooling
- **Schema**: `src/schema/` - DDL statement generation from entity definitions (`Schema::create_table_from_entity`, `create_enum_from_entity`, `create_index_from_entity`, `create_comments_from_entity`). This is generation only, not introspection.
- **Metrics**: `src/metric.rs` - Opt-in instrumentation wrappers

Row streaming is reachable through the public crate: `ConnectionTrait::query_raw` returns a tokio-postgres `RowStream`, and `src/executor/select.rs` decodes it into models — `stream` on `Select`, `SelectGraph`, `Selector`, and `SelectorRaw`, plus `stream_partial_model`, each yielding a `PinBoxSendStream`. pgorm no longer carries its own `Statement`/`StatementBuilder`: `src/database/statement.rs` was deleted, leaving `src/database/` as just `connection.rs` and `db_connection.rs`, and SQL now travels as text (`&str` or `&String`, the sealed `SqlText` bound) alongside its parameters.

### Key Differences from SeaORM
- PostgreSQL-only (no multi-database support)
- Uses tokio-postgres directly (no sqlx)
- deadpool for connection pooling
- Parameters are passed alongside the statement so it is prepared properly (no string interpolation)
- Transactions are opened only through a closure: `conn.transaction(async |tx| ..)` (the sealed `TransactionTrait`, a savepoint on a `DatabaseTransaction`) and `DatabaseConnection::transaction_with(mode, ..)` / `transaction_with_retry` give the closure a `&mut DatabaseTransaction<'_>` borrowing the parent exclusively; `Ok` commits, `Err` rolls back with the `ROLLBACK` awaited (one that fails comes back beside the closure's error as `TransactionError::RollbackFailed`, the connection's state then unknown), and a future dropped before it finishes rolls back as the handle drops. There is no public `begin`, `commit` or `rollback`. On a concrete receiver the call's future is `Send` whenever the closure's is, so it runs in `tokio::spawn`, savepoints nested inside included; code generic over `C: TransactionTrait` gets a future not known to be `Send`
- `DatabasePool` deliberately does **not** implement `ConnectionTrait` — you must `pool.get()` a connection first
- Derives are named for what they make, with no `Derive` prefix and no aliases for the old names: `#[derive(Entity)]` on the `Model` (the composite), `EntityTrait` (a hand-written `Entity` whose names are computed), `Model`, `Column`, `Relation`, `ActiveEnum` (its opt-in `display` key implements `Display`), `IntoActiveModel`, `PartialModel`, `FromQueryResult`, `ValueType`, `JsonValueType`, `CreatedRange`, `SqlName`/`StaticName` (pgorm-query's, `#[iden = ".."]`), and pgorm-migration's `MigrationName`. A derive shares its name with the trait it implements; they live in different namespaces. `impl ActiveModelBehavior for ActiveModel {}` is written by hand. COMING_FROM_SEAORM.md maps every old name
- ActiveValue fields are written with the free `set(..)` (`name: set("Apple")`) or `.into()`; both convert into the column type, so no `.to_owned()`. `ActiveValue::Set` is the pattern-matching spelling
- `ColumnTrait` carries `eq_col`/`ne_col`/`gt_col`/`gte_col`/`lt_col`/`lte_col`/`eq_expr` for column-to-column and column-to-expression predicates; `eq` and friends stay value-only so their `save_as` enum cast is never dropped
- `ModelTrait::into_active()` converts a model to its entity's ActiveModel with no destination annotation
- `select([..])` on the SELECT builders clears the default projection and projects the given list in one call — the pipeline's verb, in the ORM. A single item needs no wrapper, a homogeneous list is an array or `Vec`, a mixed list (two entities' columns, an expression, an alias token) is a tuple; a computed iterator stays `select_only()` + `columns(..)`, which are unchanged
- `Insert` is typed by row count: `Insert::one` keeps `exec_returning_pk` / `exec_returning_model`, while `Insert::many` (and `add`/`add_many`) returns every row's key or model in insertion order through `exec_returning_pks` / `exec_returning_models`
- A zero-model insert batch writes nothing and sends no SQL: `Insert::many(empty)` reports `Ok(0)` through `exec` and an empty `Vec` through the returning terminals; an explicitly supplied all-`NotSet` model still writes one default row, and `TryInsert` reports `Empty` for both states
- Composite primary keys take their parts borrowed (`find_by_id((1, "x"))` against an `(i32, String)` key), and one of their columns can be generated by the database with `#[pgorm(identity)]` / `#[pgorm(identity_by_default)]`
- Every value predicate carries `save_as` casts (`between`, `if_null` and the membership forms included), and keyset cursor boundaries bind under the same casts; graph cursors complete the ordered source's continuation key with its own primary key, and a tiebreak a caller appends with `Cursor::add_secondary_order_by(table, column)` carries its column's casts (`set_secondary_order_by`'s untyped entries bind bare)
- `load_one`/`load_many` evaluate the complete authored relation (`on_condition` and `condition_type` included) by riding the graph machinery, and `load_one` errors on an unmatched key instead of yielding `None` silently
- `RelationDef::rev()` preserves an attached `on_condition`'s authored argument roles
- Enum types can be schema-qualified end to end: `ColumnType::Enum` carries `schema`, the `ActiveEnum` derive takes `schema_name = "..."`, and codegen keys enums by full identity with no cross-schema fallback
- `Schema::create_table_from_entity` returns `Result<TableCreateStatement, Error>`: a belongs-to relation with a column typed as a domain over an enum on either side, which PostgreSQL cannot key (`42804`), is refused with `Error::Type` naming it rather than written
- Prefixed result-column names (the graph's `s{i}_` aliases and every prefixed decode) are bounded to PostgreSQL's 63-byte identifier limit through one shared composition

### Connections
`pgorm::connect(config: tokio_postgres::Config) -> DatabasePool` is infallible in its signature: pool construction failure panics rather than returning an `Error`. `connect_with_builder` takes a closure over the `PoolBuilder` for sizing and timeouts, and returns `Result` — the closure is caller input, so an unbuildable pool is an `Error`, not a panic.

COPY's data flow is `CopyTrait::copy_in` / `copy_out` (on `DatabaseConnection`, `DatabaseTransaction` and the metrics wrappers) over pgorm-query's typed `CopyFromStatement<F>` / `CopyToStatement<F>`, whose format type parameter (`CopyText`, `CopyCsv`, `CopyBinary`) decides which options exist. A binary COPY also runs as rows of `Value`s through `copy_in_rows` / `copy_out_rows` (`CopyRowSink`, `CopyRowStream`, models included), the column types learned from the server by `copy_in_columns` / `copy_out_columns`, which prepare a `SELECT` of the COPY's columns before it starts; `CopyRowEncoder` / `CopyRowDecoder` are the format's framing, which the bindings drive with their own values. Both borrow the connection exclusively. A COPY FROM the server refuses at its start closes its connection: tokio-postgres sends a `Sync` with the COPY and another when the sink ends, and a refused start answers both, which would leave every later response on the connection one request out of step.

LISTEN needs a connection of its own, since a pooled one is driven by a task that discards notifications and is lent onward once returned. `DatabasePool::listener(capacity)` (the pool's configuration and TLS connector, through `pgorm_pool::Manager::connect_raw`) and `connect_listener(config, tls, capacity)` each open one and return a `Listener` (`listen` / `unlisten` / `unlisten_all`, channel names sent as quoted identifiers, over 63 bytes refused) and its `Notifications` stream. The stream is bounded: past `capacity` unread the connection closes and the stream ends with `ListenError::Overflow`; a failed or terminated connection ends it with its error; nothing reconnects. `notify(db, channel, payload)` sends from any `ConnectionTrait` as `SELECT pg_notify($1, $2)`.

`DatabasePool::clear_statement_caches()` drops every pooled connection's cached prepared statements (deallocating them on the server), for DDL that changes what a plan was built from without PostgreSQL re-planning it, such as a domain's changed default. It advances a generation rather than clearing in place, so an idle connection is cleared before it is next lent and a checked-out one only once it is returned; it sends nothing and cannot fail. `InstrumentedPool` and the Python and JavaScript pools have it too, and pgorm-migration's `up` calls it after a batch that applied a migration commits. It reaches only this process's pool; to reach other processes' pools on the same database, `ManagerConfig::statement_cache_channel` (off by default; `statement_cache_channel=` in Python, `statementCacheChannel` in JavaScript) names a channel the pool listens on through one connection of its own, opened by its first `create`, never lent, reconnected with a backoff of 100 ms doubling to 10 s, closed by `DatabasePool::close` or the pool's drop. Every notification there clears, its payload ignored, and so does every `LISTEN` that takes effect, the first included, since a notification sent while it was down reached nothing. `up` notifies `MigratorTrait::statement_cache_channel()` (`Some(STATEMENT_CACHE_CHANNEL)`, `pgorm_statement_caches`, by default; `None` sends nothing) inside the batch's transaction, so it is delivered exactly at commit. `connect_with` refuses an empty channel or one over 63 bytes. The metrics layer does not see these clears.

`connect_with(config, tls, manager: ManagerConfig, build)` is the general entry point the other three delegate to, and the only route to TLS (any `MakeTlsConnect<Socket>` connector — `tokio-postgres-rustls`, `tokio-postgres-openssl`), to a `RecyclingMethod` other than `Fast`, or to a non-default `StatementCacheSize`. `ManagerConfig`, `RecyclingMethod`, `StatementCacheSize`, `PoolBuilder`, `NoTls`, `Socket`, `MakeTlsConnect` and `TlsConnect` are all re-exported from `pgorm`, so calling it needs no direct dependency on `pgorm-pool` or `tokio-postgres`. `DatabasePool` still has no public constructor beside it.

### Testing Setup
The tests target PostgreSQL 18, and nothing tests an older release. The local server is the docker container `pgorm-test`, created as:

```sh
docker volume create pgorm-test-18-data
docker run -d --name pgorm-test -p 54329:5432 -e POSTGRES_PASSWORD=postgres \
  -v pgorm-test-18-data:/var/lib/postgresql postgres:18
```

so `DATABASE_URL=postgres://postgres:postgres@127.0.0.1:54329`. The 18 image keeps its data under `/var/lib/postgresql/18/docker`, so a volume mounts at `/var/lib/postgresql`, not the `.../data` older images used, and a data directory from another major release cannot be reused. The same release runs everywhere else: CI's Python jobs install PostgreSQL 18 (the PostgreSQL apt repository on Ubuntu 24.04, `postgresql@18` on macOS), and pgorm-python's Docker wrapper (`pgorm-python/tests/with_postgres.py`) pins `postgres:18.6-bookworm` by digest.

PostgreSQL 18 is the floor and the default target, with no feature naming it. The `pg-19` cargo feature targets 19 (`sql.target` in `docs/spec/sql-ast.md`): pgorm-query declares it, pgorm and pgorm-python forward it, and `pgorm_query::POSTGRES_TARGET` reports the build's target. It is additive — 19-only surface arrives as new items or as new variants of `#[non_exhaustive]` public enums (the open vocabularies `sql.surface` lists), never as a variant on an exhaustive one, and no render changes. Where 18 and 19 answer a statement differently, the live test holds 18's answer by default and 19's under `#[cfg(feature = "pg-19")]` (pgorm-python's tests branch on `"pg-19" in capabilities()["features"]`), so each build runs against its own server: the default build against `pgorm-test`, and `cargo nextest run --workspace --features pg-19` against `pgorm-test-19` (`postgres:19rc1-bookworm` on port 54319, volume `pgorm-test-19-data`, created as above). Every parser (the render oracle, `sql!`, codegen) stays on 18's grammar until libpg_query has a 19 one, so no 19-only syntax is built before then.

Tests use a common setup pattern in `tests/common/setup/mod.rs` that:
- Creates a throwaway database per test through `TestContext::new(label)`, named for the test *and the process*: `{label}_{seq}__{owner}_{pid}_{stamp}` (owner hashes the host name and user; stamp is the process's first-use Unix second), cut to 63 bytes. No other run, from this checkout or another, can hold the name, so any number of runs share one server. `scoped_db_name` mints the same names for roles (`tests/view_tests.rs`)
- Never drops a database it did not create: there is no drop-a-prior-copy step. The first database a process creates sweeps the server of leftovers whose creator is provably gone (same host and user, pid no longer held or held by a later process, nobody connected), dropping them without `FORCE`
- Connects to the `postgres` maintenance database to do so, then returns a pool for the new database
- Sets up schema and test data, and tears the database down afterwards via `TestContext::delete`
- Uses `pretty_assertions` for better test output

The naming and the sweep live once, in the `pgorm-test-support` crate, which pgorm and pgorm-migration take as a dev-dependency; pgorm-napi's `tests/support.ts` (`scratchDatabase`) mints and sweeps the same names. The rule is `testing.isolation` in `docs/spec/testing.md`. A test that terminates a backend matches it by pid *and* `datname = current_database()`; tests that run in the shared `postgres` database (pgorm-pool's, `tests/pool_config_tests.rs`) create nothing there beyond their own session (temporary tables, session settings). Databases left under the older naming (`{test}_{8 hex}` from pgorm, a bare test name from pgorm-migration, `{prefix}_{node|deno}_{pid}` from pgorm-napi) are never swept; list them with `SELECT datname FROM pg_database WHERE datname !~ '__[0-9a-f]{8}_[0-9]+_[0-9a-f]{8}$' AND NOT datistemplate AND datname <> 'postgres'` and drop the ones no run is using by hand.

### Observability and Metrics

pgorm ships an opt-in metrics layer in `pgorm::metric` — see [METRICS.md](METRICS.md) for the full guide. The core types (`DatabasePool`, `DatabaseConnection`, `DatabaseTransaction`) carry no metrics hooks; instrumentation lives only in wrapper types the application chooses to construct, so unwrapped code pays nothing.

```rust
use pgorm::metric::{InstrumentedPool, LoggingMetrics};

let pool = InstrumentedPool::new(pgorm::connect(config), LoggingMetrics);
let conn = pool.get().await?; // records connection acquisition
```

`NoOpMetrics` and `LoggingMetrics` ship in-tree. Custom backends implement the `MetricsCollector` trait (async, `Clone + Send + Sync + 'static`, seven hooks, no defaults) rather than hand-rolling a wrapper.

The two query hooks take a `QueryContext<'_>` — `operation()`, `sql()`, and `fingerprint()` — instead of a bare operation name. `fingerprint()` is libpg_query's constants-normalized query identity, grouped by PostgreSQL 18's query-ID rules (an alias stands for its relation, schema qualifiers are ignored), and requires the off-by-default `metrics-fingerprint` feature; without it no statement is parsed for metrics and the answer is always `None` (pg_query is linked either way, for the paginator and the macros).

`transaction` on an `InstrumentedConnection` gives its closure an `InstrumentedTransaction`, so the statements and savepoints inside stay instrumented; a transaction whose closure's future is dropped records no ending.

#### PostgreSQL Native Observability
Still the best source of truth for query statistics:
- `log_statement = 'all'` - Log all SQL statements
- `log_duration = on` - Log statement execution times
- `pg_stat_statements` extension - Track query statistics and performance
- `auto_explain` - Log slow query execution plans
- Connection pool status via `DatabasePool::status()`
