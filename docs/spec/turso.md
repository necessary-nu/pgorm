# Experimental native Turso

This contract describes an opt-in embedded target. PostgreSQL remains the
reference for PostgreSQL behavior; enabling Turso does not weaken existing
PostgreSQL requirements. The native compatibility matrix records the supported
subset and known differences at an exact engine revision. An experiment does
not need complete PostgreSQL parity, but a successful operation must not hide a
conversion failure, unsupported operation, or unknown transaction outcome.

## Feature and ownership

> [spec:pgorm:req:turso.feature]
>
> The `turso` Cargo feature MUST be experimental and disabled by default in
> pgorm, pgorm-pool, pgorm-driver and each language binding. Enabling it in a
> consumer MUST propagate through direct dependencies to the existing
> pgorm-driver. Feature-off builds MUST neither compile nor link Turso. The
> dependency MUST identify an exact upstream revision, including any local
> changes and their provenance; detached binding workspaces MUST resolve the
> same revision and the same shared postgres-types identity.
>
> Native execution MUST reside inside pgorm-driver and use the embedded API
> in-process. The public `Client`, `Statement`, `Row`, `RowStream`, `Error` and
> `ToSql`/`FromSql` types remain the common contract. No second ORM or pool
> backend, embedded pgwire server, duplex transport, or required text-codec
> traits are introduced. Existing PostgreSQL constructors, TLS, codecs,
> statement pipelining and cancellation retain their behavior.

> [spec:pgorm:sem:turso.lifecycle]
>
> Opening a native database MUST own one database handle, with a distinct
> native connection for each driver client. All clients in one pool MUST share
> that database handle, including `:memory:`; independent in-memory opens MUST
> be isolated. File-backed changes committed successfully MUST survive closing
> and reopening the database with the same settings.
>
> Database opening, stepping, waiting and cleanup MUST follow the native API's
> ownership and threading requirements. Work that can block MUST NOT block an
> async executor's worker indefinitely. Native I/O waiting MUST cooperate with
> wakeups, cancellation and backpressure; spawning a task or blocking job per
> row is not the execution model. Opening failure and close MUST release owned
> native resources. A closed or unusable client MUST report `is_closed()` and
> reject later work through the existing driver error surface.

## Parameters, rows and errors

> [spec:pgorm:req:turso.parameters]
>
> Every native parameter MUST have a known PostgreSQL `Type` before its
> `ToSql` codec is called. `prepare_typed` and the driver's typed-query methods
> MUST accept explicit types and preserve them in `Statement::params()`.
> Untyped preparation MAY use type information supplied by the native compiler;
> if it cannot establish every parameter type, it MUST fail with an actionable
> unsupported-typing error before executing the statement. The driver MUST NOT
> discover types by trial encoding, Rust trait-object inspection, parsing SQL
> strings itself, or inspecting a first result row.
>
> For ORM-built queries, the types of supported pgorm `Value` variants MUST be
> retained before the values are erased to `dyn ToSql`, and supplied through
> the existing connection-local typed statement cache. Native type hints MUST
> be resolved lazily only for native execution; PostgreSQL MUST retain its
> existing context inference and cache keys, without allocating a native hint
> vector. SQL text and its ordered parameter-type list remain the native cache
> key. Untyped nulls and unsupported value
> types MUST fail explicitly when no reliable type is available. Raw Rust calls
> with arbitrary `dyn ToSql` values are not promised inference; their supported
> route is explicit driver or pool typed preparation until native inference is
> available. Parameter count and codec acceptance MUST be checked before
> native execution, retaining the existing parameter-count and bind-refusal
> classifications and one-based parameter context.

> [spec:pgorm:req:turso.values]
>
> The initial native parameter set MUST support typed nulls, BOOL, INT2, INT4,
> INT8, TEXT, VARCHAR, BPCHAR, BYTEA and finite FLOAT4/FLOAT8. The native result
> set MUST support nulls, booleans, signed integers with a known supported
> width, UTF-8 text, bytes and finite FLOAT8. The initial matrix MUST state the
> exact native metadata-to-`Type` mapping. Native `REAL`, which denotes a
> 64-bit engine value, maps to FLOAT8; it does not establish original
> PostgreSQL FLOAT4 identity. A result whose required identity was lost MUST
> not be advertised as preserving it.
>
> Conversion MUST use the existing `ToSql`/`FromSql` binary representations at
> the driver boundary and the native engine's values internally. Null is
> distinct from an empty string or blob. Integer bounds, boolean values and
> floating-point bounds MUST be checked; precision MUST NOT be silently lost
> to make an unsupported value fit. NaN and infinities are initially refused.
> Text and JSON MUST NOT become universal intermediate formats. Type-specific
> native text representations MAY be supported later with exact codec tests.
>
> NUMERIC, temporal values, UUID, JSON/JSONB, arrays, intervals, ranges,
> multiranges and user-defined types are initially unsupported unless a
> separately tested matrix entry establishes their exact native conversion.
> Unsupported types and formats MUST fail explicitly, including nulls carrying
> those types. A custom `ToSql`/`FromSql` implementation using a supported binary
> representation MUST work through the same checked codec path; custom types
> or text encodings without a native mapping MUST be refused. PostgreSQL's
> existing support for these types remains unchanged.

> [spec:pgorm:req:turso.rows]
>
> Column order, names and supported `Type` values MUST be established from
> native preparation/compiler metadata or explicit result typing before rows
> are consumed. Empty results and all-null results MUST expose the same
> metadata as nonempty results of that prepared statement. Metadata MUST NOT
> be guessed from a runtime cell, an alias, or the Rust destination type.
>
> The initial ORM result surface MUST include table columns in the supported
> type set, supported explicit casts, `COUNT(*)`/`COUNT(column)` as INT8, and
> boolean predicates and `EXISTS` as BOOL. Missing or misleading native
> metadata for these expressions MUST be resolved from native compiled
> expression information, including an upstream metadata correction where
> necessary. A width or type erased before that information is exposed cannot
> be reconstructed by guessing; unresolved expressions MUST fail explicitly
> and be listed as unsupported or a known mismatch.
>
> Native rows MUST remain the existing `Row` and preserve index/name lookup,
> null handling and `FromSql` refusal behavior. A returned row owns or shares
> its backing storage independently of the next native step. Conversion to the
> common row representation MUST occur at most once per materialized value;
> accessing a row MUST NOT dispatch to the native backend per cell or
> manufacture pgwire messages.

> [spec:pgorm:req:turso.errors]
>
> Native failures MUST use the existing driver `Error` surface. Structured
> native SQLSTATE, message, detail, hint, table, column and constraint fields
> MUST be retained where supplied, and exposed through `DbError` so existing
> pgorm SQLSTATE classification continues to work. Missing diagnostics remain
> absent; the driver MUST NOT invent a SQLSTATE or claim unavailable field
> fidelity. The original native cause MUST remain available where the native
> API provides it. Driver conversion, lifecycle and unsupported-operation
> errors MUST remain distinguishable from a native database refusal and retain
> the operation or parameter/column context that made them actionable.

## Execution and transactions

> [spec:pgorm:sem:turso.execution]
>
> Native clients MUST implement supported preparation, typed preparation,
> execute/affected-row counts, query, query_one, query_opt and query_raw through
> the existing driver methods. A prepared statement belongs to its creating
> native connection; using a foreign, closed or invalidated handle MUST fail
> predictably before unrelated native work occurs. Prepared statements MUST
> be reusable without reparsing their SQL on every execution.
>
> Single-statement methods MUST reject a second command. `batch_execute` MUST
> execute a trusted multi-statement script using native parsing, stop at the
> first error and return no rows. Already-applied effects and transaction
> boundaries MUST follow the documented native behavior; the implementation
> MUST NOT imply PostgreSQL's implicit script atomicity without verifying or
> implementing it. A row-count or conversion failure after an engine step
> MUST NOT be described as proof that the statement had no side effects.

> [spec:pgorm:sem:turso.streaming]
>
> Native `RowStream` MUST pull incrementally, obey poll/backpressure semantics,
> and use buffering bounded independently of the total result count. Errors
> encountered after emitted rows MUST reach the stream consumer; a partial
> result MUST NOT become a successful complete result. Early stream drop,
> task cancellation and connection close MUST release the native statement
> and establish cleanup ordering before later work, as `turso.cleanup`
> requires. Empty results and partially consumed streams MUST be tested.

> [spec:pgorm:sem:turso.transactions]
>
> Native execution MUST preserve the existing transaction and nested
> savepoint API for modes that the engine actually supports. Commit, rollback,
> release and rollback-to-savepoint MUST await the corresponding native
> outcome. An unsupported requested mode MUST be refused, never silently
> downgraded. The initial default is the engine's documented snapshot
> isolation; explicit REPEATABLE READ is supported. Other isolation/access
> modes MUST remain refused until their semantics are established at the
> pinned revision.
>
> Transaction failure state MUST be observed from the native connection. A
> native COMMIT that instead rolls back an aborted transaction MUST surface
> through pgorm's existing `TransactionAborted` behavior. Cause and rollback
> failures retain existing error composition. Native failures MUST NOT be
> automatically retried unless the available classification and transaction
> outcome meet pgorm's existing retry contract.

> [spec:pgorm:sem:turso.cleanup]
>
> Each native connection MUST serialize operations and cleanup that depend on
> its transaction state. Dropping an execution future, stream or transaction
> MUST arrange cleanup before subsequent work can use that connection. Native
> interrupt and statement cleanup MUST complete before successful reuse;
> unfinished work, failed cleanup or an unknown commit outcome MUST make the
> client unusable and cause pool disposal. Returning a connection to the pool
> alone is not evidence of cleanup. No later borrower may observe an abandoned
> transaction or operation.
>
> Tests MUST interleave cancellation/drop with later operations and pool
> acquisition, including nested savepoints, execution errors, partial streams
> and close. PostgreSQL's existing FIFO rollback-on-drop behavior is preserved;
> native cleanup need not imitate its network request channel.

## Pool and capabilities

> [spec:pgorm:req:turso.pool]
>
> With `turso` enabled, pgorm MUST expose
> `async connect_turso(path: impl AsRef<Path>) -> Result<DatabasePool, Error>`
> and `connect_turso_with(path, manager, build)`, an async fallible configuration
> variant taking the existing `ManagerConfig` and pool-builder customization.
> Opening the native database and establishing a
> usable connection MUST finish before this async constructor succeeds. These
> constructors MUST use the existing pgorm-pool connection factory,
> `ClientWrapper`, statement cache, hooks and transactions; the database handle
> is shared as `turso.lifecycle` requires. Public signature types MUST be
> available from pgorm's existing re-export surface.
>
> Native Fast recycling MUST reject closed or unclean clients. Verified
> recycling MUST perform a native health operation. PostgreSQL-specific Clean
> recycling and notification-based cache invalidation MUST fail explicitly if
> their required native operations are unsupported. Ordinary cache clear,
> eviction and connection-local typed cache reuse MUST continue to work.
> Unsupported manager options MUST be rejected while building the native
> pool, before it can be mistaken for a usable configured pool.

> [spec:pgorm:req:turso.capabilities]
>
> Backend identity MUST come from connection construction. The native support
> matrix MUST distinguish supported operations, driver integration gaps,
> native-engine gaps, known behavioral mismatches and unverified behavior at
> the pinned revision. It MUST cover COPY, notifications, cancellation,
> transaction modes, catalog/metadata operations, migrations and codegen as
> well as the ordinary query surface.
>
> An unsupported driver operation MUST fail at the layer that knows it cannot
> perform it, before causing unrelated effects. Backend errors from supported
> execution MUST be preserved. Capabilities MUST NOT be inferred from a server
> banner or the older pgwire frontend's limitations. No SQL-rewriting or
> general SQL-parsing layer is introduced in pgorm to simulate support, and no
> unsupported operation is a successful no-op.

## Validation and performance

> [spec:pgorm:req:turso.compatibility]
>
> Native live tests MUST use a fresh temporary database per test or an
> explicitly shared in-memory database within that test. They MUST NOT invoke
> PostgreSQL's CREATE/DROP DATABASE test setup. Equivalent supported operations
> MUST be checked against real PostgreSQL, including supported values, nulls,
> parameter typing, column metadata, row counts, statement reuse, transactions,
> errors, cancellation, bounded streaming, pooling and concurrent borrowers.
>
> The revisioned matrix MUST associate every supported entry and known
> mismatch with a reproducible test; a known failure is specific and visible,
> not a broad skipped suite. Both feature states MUST build and the existing
> PostgreSQL suite MUST remain exercised. An experimental CI lane MUST run the
> native suite. Completion of the initial native subset does not require every
> existing PostgreSQL test to pass against Turso.

> [spec:pgorm:req:turso.performance]
>
> The pre-integration PostgreSQL baseline and later comparisons MUST use the
> same release-build workloads, database/durability settings and concurrency,
> with warmup and setup excluded from timing. Workloads MUST include cached
> scalar queries, parameter-heavy operations, model reads, parameterized
> writes, incremental multi-row streams, transactions and concurrent pool
> acquisition/use. The report MUST record build features, revisions, runtime,
> machine and database settings, repeated latency distributions and throughput.
> Any unmeasured allocation, copy or memory metric MUST be identified as
> unmeasured rather than inferred from latency.
>
> Compare the PostgreSQL path before and after native integration with `turso`
> both disabled and enabled. Compare direct native API, pgorm-driver native
> execution and ORM execution using equivalent supported operations and
> settings. Repeat runs and their variance MUST accompany conclusions;
> repeatable material regressions require investigation and correction of
> avoidable dispatch, allocation, conversion or scheduling costs before
> delivery. Where the baseline establishes a quantitative acceptance bound,
> record and apply that bound. No unmeasured percentage or claim that removing
> TCP alone proves performance is an acceptance criterion. Streaming memory
> MUST remain bounded as the result count grows.

## Language bindings and documentation

> [spec:pgorm:req:turso.python]
>
> Python MUST expose `await connect_turso(path, **options)` returning the
> existing Pool, with the supported pool size, timeout, cache and hook options
> shared with its PostgreSQL pool. The optional native build feature MUST be
> recorded by package capabilities. Without it, the entry point MUST raise
> `UnsupportedCapabilityError` before opening anything; invalid or unsupported
> options retain the existing argument-error classifications. The path
> `:memory:` has the shared-pool lifetime defined in `turso.lifecycle`.
>
> Supported statements MUST use the same pgorm models, codecs, transactions,
> errors and result forms. Existing event-loop ownership, cancellation and
> close guarantees remain in force. Installed-package tests MUST cover
> feature availability, parameterized reads/writes, file reopen, pooled memory,
> transactions, streaming and unsupported-operation errors. Type stubs,
> lockfiles, source/wheel contents and notices MUST match the shipped optional
> dependency and feature set; ordinary builds MUST not enable it implicitly.

> [spec:pgorm:req:turso.javascript]
>
> JavaScript MUST expose `connectTurso(path, options)` returning a Promise of
> the existing Pool, with the supported pool size, timeout, cache and hook
> options shared with its PostgreSQL pool. The optional native build feature
> MUST be recorded in package feature metadata. Without it, the entry point MUST
> reject with the binding's construction-error classification before opening
> anything. Invalid or unsupported options retain existing argument-error
> classifications. `:memory:` has the lifetime defined in `turso.lifecycle`.
>
> Supported statements MUST use the same pgorm models, codecs, transactions,
> errors and result forms. Existing event-loop, AbortSignal, object lifetime
> and close guarantees remain in force. Installed-package tests MUST cover
> feature availability, parameterized reads/writes, file reopen, pooled memory,
> transactions, streaming and unsupported-operation errors on supported
> runtimes. TypeScript declarations, lockfiles, native package contents and
> notices MUST match the optional dependency and feature set; ordinary builds
> MUST not enable it implicitly.

> [spec:pgorm:req:turso.documentation]
>
> User documentation MUST identify the experimental feature/package choices,
> actual Rust/Python/JavaScript entry points, database lifetime, supported type
> and operation subset, typing requirements and explicit refusals. Examples
> MUST execute in the corresponding native test/package environments. The
> documentation MUST link the revisioned compatibility matrix and measured
> performance evidence, record engine provenance/licenses, and specify which
> compatibility, feature-isolation, package and performance checks to repeat
> when upgrading the native dependency. PostgreSQL compatibility claims MUST
> remain scoped to the evidence at the recorded revision.
