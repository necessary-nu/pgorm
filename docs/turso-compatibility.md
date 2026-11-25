# Experimental native Turso compatibility

The `turso` feature uses Turso's embedded API at
`adf3b41f99ebb6b1cdd1b60ae08b483bc45686fa` (`0.9.0-pre.6`). PostgreSQL remains
the reference implementation. Backend identity comes from the connection
constructor; the driver does not inspect a server banner. The evidence below
applies to this exact native revision, independently of Turso's pgwire frontend.

See the [setup guide](turso.md) for connection examples and build choices, and
the [performance report](performance/turso-results.md) for measured costs.

## Operation matrix

“Supported” means the stated cases are exercised, not that all PostgreSQL
semantics are established. A driver gap is a missing integration; an engine gap
is a native refusal that a different client cannot repair. A mismatch is an
observed difference despite successful execution. Unverified behavior has no
compatibility claim.

| Operation | Status | Boundary and evidence |
| --- | --- | --- |
| Prepared queries, scalar parameters and owned rows | Supported subset | The scalar table below is the boundary. [Native codec tests](../crates/pgorm-driver/tests/native_types.rs), [execution tests](../crates/pgorm-driver/tests/native_execution.rs) and [ORM tests](../tests/turso_execution_tests.rs) exercise it. |
| Pooling and statement reuse | Supported subset | File and memory pools and hooks are covered by [pool tests](../tests/turso_pool_tests.rs); typed cache keys and invalidation by [ORM execution tests](../tests/turso_execution_tests.rs). |
| Transactions and savepoints | Supported subset | Default snapshot isolation and explicit driver `RepeatableRead`; other builder modes are refused before `BEGIN`. See [driver](../crates/pgorm-driver/tests/native_transactions.rs) and [ORM](../src/database/turso_transaction_tests.rs) tests and the transaction notes below. |
| Cancellation | Supported subset | `cancel_token().cancel_query(NoTls)` requests native cancellation; dropped work and streams use native cleanup. [Lifecycle](../crates/pgorm-driver/tests/native_lifecycle.rs) and [execution](../crates/pgorm-driver/tests/native_execution.rs) tests cover reuse and cancellation under backpressure. Cancellation does not promise to interrupt synchronous kernel I/O. |
| Network cancellation, portals and simple-query messages | Driver gap | These wire-specific APIs refuse locally. Use native cancellation, ordinary queries or `batch_execute` as appropriate. [Refusal tests](../crates/pgorm-driver/tests/native_unsupported.rs). |
| Streaming COPY and connection-backed COPY column/row helpers | Driver gap | Dedicated APIs refuse before SQL preparation or catalog work, preserving a usable connection and transaction. The native frontend exposes `CopyOut` rows, but the driver has no COPY transport integration. Standalone COPY codecs remain independent of the backend. [Public capability tests](../tests/turso_capability_tests.rs). |
| COPY through generic query/execute | Refused | The ORM's existing COPY guard returns `Error::CopyStatement`, including before any part of a script runs. Direct driver preparation refuses native `CopyOut` before stepping; a direct driver batch retains effects of earlier completed statements. [Driver](../crates/pgorm-driver/tests/native_unsupported.rs) and [public](../tests/turso_capability_tests.rs) tests. |
| Filesystem COPY via direct native SQL | Unverified | Acceptance of a native `Command` does not establish file-copy effects, format fidelity or affected-row counts. The ORM's generic COPY guard still applies. |
| Notifications | Engine and driver gaps | Native `LISTEN`, `NOTIFY` and `UNLISTEN` return `0A000`; `pg_notify` returns `42883`. The pool refuses listener construction and notification cache configuration locally. [Native probes](../crates/pgorm-driver/tests/native_capabilities.rs) and [public tests](../tests/turso_capability_tests.rs). |
| MERGE | Engine gap | The tested matched-update/unmatched-insert form returns `0A000` without changing the target. [Native probe](../crates/pgorm-driver/tests/native_capabilities.rs). |
| Selected `pg_catalog` queries | Supported values, metadata mismatch | The fixture's `pg_class(relname,relkind)`, `pg_attribute(attname,attnum)` and `pg_type(typname,typtype)` values are checked directly and through the driver. The native metadata uses TEXT/INT4 rather than PostgreSQL's name/char/int2 identities. This is not general catalog parity. [Native probes](../crates/pgorm-driver/tests/native_capabilities.rs). |
| `information_schema.tables` / `.columns` | Engine gap | The tested qualified queries, including the migrator checksum-column query, return `42P01`. [Native probes](../crates/pgorm-driver/tests/native_capabilities.rs). |
| `to_regclass` and `pg_advisory_xact_lock` | Engine gap | Tested literal and parameterized forms return `42883` during native preparation. Explicit parameter types do not repair these missing functions. [Native probes](../crates/pgorm-driver/tests/native_capabilities.rs). |
| LOCK TABLE | Known mismatch | Inside a transaction, `LOCK TABLE missing_relation IN ACCESS SHARE MODE` succeeds even though the relation does not exist. Outside a transaction the same statement returns `22000`. Do not rely on PostgreSQL lock semantics from successful execution. [Native probe](../crates/pgorm-driver/tests/native_capabilities.rs). |
| pgorm migrations | Engine gap | Fresh `Migrator::status` and `up` return `42883` before creating a ledger or running a migration body. Required catalog/functions and notifications are missing. [Public migration probes](../tests/turso_capability_tests.rs). |
| Code generation | Offline generation; native runtime unverified | `pgorm-codegen` consumes DDL and generates Rust models without connecting to a database. Its existing [DDL fixtures](../pgorm-codegen/tests/sql) do not establish that every emitted type or query works natively. Generated models are subject to the same scalar and SQL limits above. |

Driver refusals carry no invented SQLSTATE. Engine errors retain their native
SQLSTATE and diagnostics. Command statements expose no internal placeholder
columns or rows; native serial/identity prerequisites may change their compiled
program while executing, after which the prepared handle is invalidated.
There is no SQL translation layer and no emulated successful unsupported call.

The [paired compatibility corpus](../tests/turso_compatibility_tests.rs) runs
equivalent operations against real PostgreSQL, a fresh native file and isolated
native memory. It checks values, metadata, counts, transactions, cancellation
and concurrent pool borrowers; `metadata_mismatches_remain_explicit` records
the serial, VARCHAR, NUMERIC and REAL differences. Run it with a PostgreSQL
`DATABASE_URL` configured:

```sh
cargo test -p pgorm --features turso --test turso_compatibility_tests
```

The [experimental CI lane](../.github/workflows/turso.yml) runs the existing
PostgreSQL suite in both feature states and the native suite with `turso` enabled.
The native [worker tests](../crates/pgorm-driver/src/native/worker/tests.rs)
also check row and byte backpressure, including oversized rows.

## Opening a pool

The existing Rust entry points accept a driver `Config` parsed from a
PostgreSQL connection string, a file path or `:memory:`:

```rust,no_run
use pgorm::Connection;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let pool = pgorm::connect("./app.db".parse()?);
let connection = pool.get().await?;
let row = connection.query_one("SELECT 1", &[]).await?;
assert_eq!(row.get::<_, i32>(0), 1);
# Ok(())
# }
```

File opening requires the experimental `turso` Cargo feature. Parsing and pool
construction do no I/O; first acquisition opens the database, creates a client
and runs its creation hooks. Opening failures are acquisition errors. All
connections in one pool share the native database, including `:memory:`.
Separately constructed memory pools are isolated, including when their configs
were cloned. Repeated direct `Config::connect` calls on one retained native
config share its database; cloning that config copies settings and creates an
independent database owner. A cloned file configuration still names the same
file; an in-memory clone starts with its own empty database when opened.

`connect_with(config, NoTls, manager, build)` uses the existing `ManagerConfig`
and builder closure for tags, hooks, timeouts and statement-cache sizing.
PostgreSQL Clean recycling and notification-based cache invalidation are refused
before opening. Zero-capacity pools retain the ordinary lazy builder semantics.
A final native creation hook check refuses an open transaction, a closed client
or retained active work before lending the connection. Native
`pool.listener(...)` is explicitly unsupported.

PostgreSQL URLs and libpq key/value strings retain their parsing rules. Malformed
recognized PostgreSQL strings never fall back to creating a file. Explicit
relative (`./app.db`) and absolute paths can contain spaces or `=`; use a path
prefix when a filename resembles a libpq setting. `:memory:` selects an
independent in-memory target, with no file. Feature-disabled builds recognize
native targets and refuse them before opening.

Rust pool close rejects new borrowers and discards idle connections. Already
checked-out Rust connections remain usable until released. Python and JavaScript
pool close also cancels checked-out work and closes lent connections under the
bindings' lifecycle. Native worker destruction follows the driver lifecycle.

## Scalar codecs

| PostgreSQL parameter type | Native value | Result representation |
| --- | --- | --- |
| BOOL | Integer 0 or 1 | BOOL when column or compiled predicate metadata establishes it |
| INT2 | Signed integer | INT2 for declared `smallint` columns |
| INT4 | Signed integer | INT4 for native INTEGER columns and casts |
| INT8 | Signed integer | INT8 for declared `bigint` columns and counts |
| TEXT, VARCHAR, BPCHAR | UTF-8 text | TEXT, or VARCHAR for declared `varchar(n)` columns |
| BYTEA | Blob | BYTEA for declared `bytea`, native BLOB, and compiled BLOB casts |
| Finite FLOAT4, FLOAT8 | 64-bit floating value | FLOAT8 for native REAL columns and casts |
| A null with one of the types above | Null | Null with the prepared column's type |

An unmodified parameter projection can retain its explicitly supplied type.
Other results are described from native column and compiled expression metadata
before execution. Integer literals use the width established by their compiled
literal: INT4 when it fits i32, otherwise INT8 within i64. `COUNT(*)` and
`COUNT(column)` use INT8. Comparisons, logical predicates
and `EXISTS` use BOOL even where native affinity alone reports INTEGER.

The driver uses the shared binary `ToSql` and `FromSql` representations. Custom
codecs work when they accept a supported type and supply its binary format.
Text-format codecs, malformed payloads, out-of-range integers, nonfinite floats,
and conversions that would lose precision are refused. Nulls do not bypass type
validation, and empty text or bytes remain distinct from null.

NUMERIC, temporal values, UUID, JSON/JSONB, arrays, intervals, ranges,
multiranges, domains and other user-defined type identities have no native
driver codec in this subset. An unresolved expression is refused with its
column name and position; arbitrary arithmetic and aggregates other than count
are not inferred from returned cells.

## Metadata differences

The native compiler can erase PostgreSQL identities before exposing its
compiled representation. REAL identifies the engine's 64-bit value, including
columns originally declared with a floating type of another width. SMALLINT
casts and serial aliases in table declarations become INTEGER; bare
VARCHAR/BPCHAR declarations and
VARCHAR/BPCHAR, UUID, JSON and JSONB casts become TEXT. Unparameterized NUMERIC
declarations and NUMERIC casts become REAL/FLOAT8. A declared `numeric(p,s)`
retains its unsupported custom identity and is refused by the driver.

These canonical types do not establish fidelity to the original PostgreSQL
type. A TEXT result from an erased UUID cast is not UUID codec support, and a
Rust UUID reader must reject it. Likewise a NUMERIC expression lowered by the
engine to REAL does not preserve decimal precision. Explicit NUMERIC, UUID and
JSON parameter types remain unsupported regardless of nullness.

Table OIDs, column attribute numbers and PostgreSQL type modifiers are not
invented: native columns report `None`, `None` and `-1` respectively. Native
SQLSTATE and diagnostic fields are preserved where the engine supplies them.
Driver conversion and unsupported-operation errors carry no fabricated
SQLSTATE.

Rows own their binary payload and retain their prepared metadata. They remain
readable after the next native step, statement release or connection shutdown.
Native results do not fabricate pgwire messages or select a backend on each
cell read.

Prepared metadata is tied to the native compiler context. A schema or context
change invalidates the handle; `Error::is_statement_invalidated()` identifies
this refusal without a fabricated SQLSTATE. Discard that cached statement and
prepare it again for a later call. Invalidation can be detected after an engine
step, so this error does not prove that the operation had no effects and does
not authorize automatic retry.

At this engine revision, adopting another connection's schema can repeatedly
advance the native compilation context because of an internal schema-copy
identity mismatch. Before a fresh prepare in autocommit mode, the driver
synchronizes differing local/shared schema handles with a native `SELECT 1`.
This uses the ordinary worker cancellation and cleanup path, runs only for
that schema mismatch, and does not add a query or reparse on cache hits.

## Parameters and execution

Native parameters require explicit types through `prepare_typed` or the typed
driver query methods. Raw SQL with arbitrary `dyn ToSql` parameters cannot use
PostgreSQL's context inference. ORM-built queries retain supported `Value`
variants until native preparation, including typed nulls, and use the existing
connection-local cache keyed by SQL and ordered parameter types. PostgreSQL
does not resolve these hints or allocate their type vector.

ORM integer hints are INT2 for TinyInt/SmallInt, INT4 for Int, and INT8 for
BigInt/Unsigned/BigUnsigned. Values outside the selected representation still
fail the ordinary checked codec; a u64 above i64::MAX is refused. String/Char
use TEXT, Bytes uses BYTEA, and Float/Double retain FLOAT4/FLOAT8 parameter
types. Unsupported variants fail with the parameter position before execution.

Prepared calls accept one command. `batch_execute` runs a trusted script and
stops at failure; outside an explicit transaction, preceding completed
statements may remain committed. It does not provide PostgreSQL's implicit
script transaction. At the pinned revision, an empty script succeeds but a
comment-only nonempty script produces the native syntax error, SQLSTATE 42601.
`execute` reports completed command counts, including the returned row count
for SELECT.

The pool cache discards a known-invalid native handle before using it again.
An invalidation delivered while polling a stream marks the shared statement,
so its next cache lookup prepares afresh. The failed operation is not replayed
automatically. Native SQLSTATE 0A000 is a server refusal, not evidence that
PostgreSQL's cached-plan retry is safe. A conversion or cardinality error after
stepping likewise does not prove that a write had no effects.

## Transactions and reuse

Default native transactions use the engine's snapshot isolation. The driver
builder also accepts an explicit `RepeatableRead` isolation level. Other
isolation levels and explicit access or deferrability settings are refused
before beginning a transaction. In the ORM, `TransactionMode::Default` is the
supported initial mode; its other variants request explicit access settings.

Transactions and nested savepoints use the existing APIs. Commit and rollback
await native completion. A statement error can leave the native transaction
aborted even when the closure catches it: committing that closure rolls back
and reports `TransactionAborted`. An aborted savepoint rolls back to its
boundary, allowing the outer transaction to continue. The original refusal
and any additional rollback failure remain available to the caller.

Dropping a transaction queues rollback behind its preceding work. If the queue
cannot accept cleanup, cleanup fails, or a transaction end is abandoned with
an unknown outcome, the native client becomes unusable. Native Fast recycling
checks completed work and idle state without a query. A connection returned
with unfinished work, a retained active stream or an open transaction is
disposed rather than handed to another borrower. Verified recycling also
performs a native health operation; PostgreSQL's Clean session reset is
unsupported.

At the pinned revision, `ROLLBACK TO` changes the native preparation context
even without DDL. The worker marks affected prepared handles before returning
the rollback result. If drop cleanup is still queued, the ORM cache prepares
the requested key after it in FIFO order. Ordinary commits retain healthy
cached statements, and no executed write is retried to repair the cache.

Native SQLSTATEs remain diagnostic evidence. A cleanup failure prevents
transaction retry even if its primary error carries `40001` or `40P01`.
