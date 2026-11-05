# Metrics Layer

`src/metric.rs` provides an opt-in instrumentation layer. The core types
(`DatabasePool`, `DatabaseConnection`, `DatabaseTransaction`) carry no metrics
hooks at all; instrumentation exists only in wrapper types the application
chooses to construct, so unused metrics cost nothing.

## Collector contract

> [spec:pgorm:def:metric.layer+2]
> The metrics layer consists of a `MetricsCollector` trait (`Clone + Send +
> Sync + 'static`) and three wrappers parameterized by it:
> `InstrumentedPool<M>` (wrapping `DatabasePool`), `InstrumentedConnection<M>`
> (wrapping `DatabaseConnection`), and `InstrumentedTransaction<'a, M>`
> (wrapping `DatabaseTransaction<'a>`). Each wrapper exposes `inner()` and
> `metrics()` accessors to reach the wrapped value and the collector; the pool
> wrapper also forwards `tag()`, `status()` and `clear_statement_caches()`
> (`conn.pool.statement-cache.clear`). Two further public types are
> the argument the query hooks take rather than machinery of their own:
> `QueryContext<'a>` and `QueryFingerprint`, both defined by
> `metric.fingerprint`.

> [spec:pgorm:def:metric.layer.collector+3]
> `MetricsCollector` defines seven async hook points:
> `record_query_success(query, duration, rows)`,
> `record_query_error(query, duration, error)`,
> `record_connection_acquired(duration)`,
> `record_connection_error(duration, error)`,
> `record_transaction_begin(duration)`,
> `record_transaction_commit(duration)`, and
> `record_transaction_rollback(duration)`.
>
> The two query hooks take a `QueryContext<'_>` rather than a bare operation
> name; the operation is one of its fields, so identifying a statement more
> precisely is a question of what the context carries rather than of the
> trait's arity. The trait keeps no default methods, so every implementor
> writes all seven; a hook it does not care about is an empty body.
>
> Seven is the count, and a transaction round trip that fails does not make it
> eight. Each of the three — `begin`, `commit`, `rollback` — reports its failure
> through `record_query_error` under its own operation name (`metric.layer.tx`),
> because that hook already carries an operation, a duration and the `Error`,
> which is everything there is to say. Adding a `record_commit_failure` would
> break every existing implementor — there are no defaults to fall back on — to
> express what the existing hook expresses, and would then owe the other two
> the same treatment.
>
> The hooks' meanings are therefore stated on the trait methods themselves,
> which is where an implementor writing a counter reads them, rather than only
> on the wrapper methods that happen to call them.
> `record_transaction_rollback` in particular means *the transaction ended
> without committing* — by a closure that returned `Err` or by a failed
> `COMMIT` alike — and says so in its own documentation;
> `record_transaction_commit` is success only.
>
> Two implementations ship in-tree: `NoOpMetrics`, whose hooks are all empty
> bodies, and `LoggingMetrics`, which emits `tracing` events — `debug` for
> query success, connection acquired, transaction begin, and commit; `warn`
> for query errors and rollbacks; `error` for connection failures. Its two
> query messages end with ` [<fingerprint>]` when the statement has one, and
> carry no suffix at all when it does not.

## Query identity

> [spec:pgorm:req:metric.fingerprint+3]
> `QueryContext` is what a query hook is told about the statement it reports
> on: `operation()` — one of the seven `ConnectionTrait` method names,
> `"copy_in"` / `"copy_out"` for a COPY (`metric.layer.copy`), or `"begin"` /
> `"commit"` / `"rollback"` for a failed transaction round trip —,
> `sql()` (the
> statement text, where it survives per `conn.sql-text`), and `fingerprint()`.
> It borrows for the duration of the call, so a collector that keeps anything
> copies it out. A `QueryFingerprint` is libpg_query's constants-normalized
> parse-tree hash: `Display` renders its canonical 16-character zero-padded
> hex, `value()` hands back the same number as a `u64`, and it is the notion of
> query identity `pg_stat_statements` aggregates by. Statements differing only
> in their literals or their whitespace share one; statements differing in
> shape do not.
>
> The fingerprint MUST follow PostgreSQL 18's query-ID rules, libpg_query
> 18's default, so that it groups statements the way the 18 server's own
> query IDs do: a schema qualifier is not part of a relation's identity, and
> an aliased relation is identified by its alias rather than its name. Those
> rules changed between releases, so a fingerprint is stable across a
> parser upgrade only where they did not: with libpg_query 18.1.0, the
> fingerprints of statements with an aliased or schema-qualified relation,
> of every statement with `RETURNING`, and of `BEGIN` with options all moved
> from what libpg_query 17 gave them. A collector that stores fingerprints
> across that upgrade sees those statements under new identities.
>
> `fingerprint()` MUST NOT fail the query path, and so returns an `Option`
> whose `None` covers three cases without distinguishing them: the
> off-by-default `metrics-fingerprint` feature is not enabled, so no
> statement is parsed for its identity; the statement carries no text to
> parse; or libpg_query rejected
> the text it was given. The last is not an error — raw SQL the server accepts
> may still be text this parser will not reduce to a tree. Such a statement
> executes normally and is reported normally; only its identity is missing. An
> empty statement, by contrast, is an empty parse rather than a rejected one,
> and does have a fingerprint.
>
> Fingerprints are computed on demand rather than when the context is built, so
> a collector that never asks pays only for the two borrowed fields.
> Computing one is a parse, so answers are memoized process-wide by statement
> text — a rejection memoized as readily as a fingerprint — in an
> `RwLock<HashMap>` bounded at 1024 distinct texts. At the bound the memo stops
> admitting new entries rather than evicting: an application's statement shapes
> are a fixed set well under it, and the texts that would overrun it are the
> per-call ones (an `IN` list whose arity follows the input, a generated
> script), which are better re-parsed than retained forever.
>
> Without the feature no statement is parsed for metrics — libpg_query is
> linked either way, for the paginator and the macros, so the feature adds no
> dependency — and the public shape is
> unchanged: both types still exist, the hooks still take a context, and
> `fingerprint()` is simply always `None`. A collector therefore compiles
> against either build, and enabling the feature changes no API — only an
> answer.

## Delegation contract

> [spec:pgorm:req:metric.layer.delegate+5]
> Instrumented wrappers MUST delegate every operation to the wrapped type and
> return its result unchanged — wrapping preserves `ConnectionTrait`
> semantics, adding only timing and collector calls around each awaited
> operation. Both `InstrumentedConnection` and `InstrumentedTransaction`
> implement `ConnectionTrait`, and so does the `&mut InstrumentedTransaction`
> a transaction closure is given, through the `&mut` forwarding of
> `conn.pool.conn-trait`: inside an instrumented closure `find().all(&txn)`
> and a helper taking `&C` are passed `&txn`, as inside a plain one, and run
> on the instrumented handle. On success they report
> `record_query_success` with a `QueryContext` (`metric.fingerprint`) naming
> the operation (`"execute"`,
> `"execute_raw"`, `"query_one"`, `"query_opt"`, `"query_all"`,
> `"query_raw"`, `"batch_execute"`) and carrying the statement's own text —
> the `&str`/`&String` argument, or `sql` for `batch_execute` — and a row
> count — the affected-row count for `execute`/`execute_raw`, `1` for
> `query_one`, `1`/`0` for `query_opt` `Some`/`None`, `rows.len()` for
> `query_all`, `None` for `query_raw` per `exec.stream.decode`, and `None`
> for `batch_execute`, which yields no rows at all — and on
> failure they report `record_query_error` with the same
> context before propagating the `Error`. `InstrumentedPool::get`
> times pool acquisition, reporting `record_connection_acquired` on success
> (and returning an `InstrumentedConnection` sharing a clone of the
> collector) or `record_connection_error` on failure.

## COPY instrumentation

> [spec:pgorm:req:metric.layer.copy]
> `InstrumentedConnection` and `InstrumentedTransaction` MUST implement
> `CopyTrait` (`conn.copy`) by delegating to the wrapped connection or
> transaction and returning its sink or stream unchanged, timing the start
> and reporting it as `metric.layer.delegate` reports a statement: on success
> `record_query_success` under the operation `"copy_in"` or `"copy_out"`, with
> the statement's rendered text as `sql()` and no row count, and on failure
> `record_query_error` under the same context. The row count is `None` for the
> reason a stream's is (`exec.stream.decode`): what the hook times is the
> start, and the rows have not moved yet. The trait's return types are the
> plain `CopyInSink` and `CopyOutStream`, so what happens after the start — the
> count `finish` returns, a row the server refuses there, the bytes a stream
> yields — reaches the caller and not the collector, as a stream's rows and
> its mid-stream failures do.

## Transaction instrumentation

> [spec:pgorm:sem:metric.layer.tx+4]
> `InstrumentedConnection` and `InstrumentedTransaction` are `TransactionTrait`
> implementors (`conn.tx`) whose closure is given an
> `InstrumentedTransaction<'_, M>` sharing a clone of the collector, so the
> statements, COPYs and savepoints inside an instrumented transaction are
> instrumented with no wrapping by the caller. `InstrumentedTransaction` has no
> public constructor: it exists only as that handle, and its `inner()` reaches
> the plain `DatabaseTransaction` beneath, whose statements bypass the
> collector.
>
> Opening a transaction — `BEGIN` on the connection, a savepoint inside a
> transaction — is timed and reports `record_transaction_begin` on success; a
> failed opening is reported through `record_query_error` under the operation
> `"begin"` — a context with no statement text, since none was sent — not a
> dedicated hook.
>
> A closure that returns `Ok` is followed by a timed `COMMIT` reporting
> `record_transaction_commit` on success. A failed commit MUST report both
> `record_query_error` — under the operation `"commit"`, a context with no
> statement text — and `record_transaction_rollback`, in that order. A closure
> that returns `Err` is followed by an awaited `ROLLBACK`, which reports
> `record_transaction_rollback` on either outcome — the transaction is
> discarded whether or not the round trip succeeds — and a failed round trip
> additionally through `record_query_error` under the operation `"rollback"`,
> likewise with no statement text.
>
> The two ends thus report the same pair on failure, which is the point:
> PostgreSQL aborts a transaction whose `COMMIT` fails, so the transaction
> ended without committing in both cases and the rollback hook means exactly
> that (`metric.layer.collector`). Reporting only the rollback hook would leave
> the commit error itself unreported — visible to the caller through the
> returned `TransactionError::Connection` and to nobody else — while a
> collector's `rollbacks_total` silently absorbed a failure with no matching
> entry in its error stream.
>
> The failure is not reported through a hook of its own. See
> `metric.layer.collector` for why the count stays at seven.
>
> A transaction whose closure's future is dropped before it finishes records
> no ending at all — not even a rollback. This is a limit, not a policy: every
> collector hook is `async` while `Drop::drop` is synchronous, so no hook is
> reachable from a drop. The drop defers entirely to the inner
> `DatabaseTransaction`'s (`conn.tx.guard`): a tracing warning plus a
> fire-and-forget `ROLLBACK`.
