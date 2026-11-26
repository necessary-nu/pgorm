# Read-only connections

This chapter covers connections that may only read. Whether a connection may
write is part of its type: what reads takes `Connection`, what may write takes
`ConnectionMut`, and a pool whose connections may only read hands out
connections that implement `Connection` alone, opened so that the server
refuses a write too. The type is the first line, the session setting the
second, and the database role the boundary: SQL text the server is handed can
`SET` a session or a transaction back to `READ WRITE`, which no type sees.
Read-only connections are orthogonal to scopes (`scope.compile`): a scope
decides which rows a statement touches, this decides whether the connection
may write any.

## The traits

> [spec:pgorm:def:read-only.traits]
> `Connection` MUST cover what only reads: its six statement methods
> (`conn.pool.conn-trait`), and through them every read pgorm builds — the
> select, graph and selector terminals, counts, pagination, cursors, the
> pipeline's terminals and the loaders. `ConnectionMut: Connection` MUST cover
> what may change the database, its methods borrowing shared as `Connection`'s
> do: every write pgorm builds (`read-only.writes`), and `batch_execute`, a
> `Connection` method bounded by `Self: ConnectionMut` so that a script on a
> connection that may only read is the compile error `read-only.diagnostics`
> words rather than a method not found.
> `DatabaseConnection`, `&DatabaseConnection`, `DatabaseTransaction`,
> `HookConnection`, `DatabaseRef`, `&mut C` for every `C: ConnectionMut`, and
> the metrics wrappers around a connection that may write MUST implement both;
> `ReadOnlyConnection`, `ReadOnlyTransaction`, the `ReadOnly` and `Unchecked`
> views, and the metrics wrappers around a read-only connection MUST implement
> `Connection` alone. A function taking `&impl Connection` therefore cannot
> write through what it is given, whatever that is, by any statement whose
> writing its type shows. What no type shows — a `SELECT` calling a function
> that writes, a row lock a select builder adds, a `COPY` over a write's
> `RETURNING` — runs as a read; only a session the server holds read-only
> refuses it (`read-only.refusal`).

## What SQL text is known to do

> [spec:pgorm:def:read-only.kinds]
> `SqlText` MUST carry a `Kind`: `Reads` for a statement known to only read,
> `Writes` for one that may write. `str`, `String` and `ValueSql` — text
> known only at run time — MUST be `Writes`. A `sql!` or `prql!` statement's
> kind MUST be its classification (`read-only.classify`). The statements
> pgorm builds to read MUST be handed to a connection as `Reads`, through a
> crate-private text whose kind pgorm states, so no caller can state one.

> [spec:pgorm:def:read-only.classify]
> `pgorm_sql_check::classify_sql(sql)` MUST hold `sql` to the grammar as
> `check_sql` does and answer `Effect::Reads` exactly when every statement in
> it only reads: a `SELECT`, `VALUES` or `TABLE` holding no `INSERT`,
> `UPDATE`, `DELETE` or `MERGE` anywhere in its tree — a data-modifying `WITH`
> among them — no row-locking clause (`FOR UPDATE`, `FOR NO KEY UPDATE`,
> `FOR SHARE`, `FOR KEY SHARE`) at any depth, and no `INTO`; a `SHOW`; or an
> `EXPLAIN` that does not `ANALYZE`, whatever it explains, since it runs
> nothing. An `EXPLAIN` whose `ANALYZE` option is present and not `false`,
> `off` or `0` MUST be `Writes`, as MUST every other statement. The answer is
> the grammar's: a `SELECT` calling a function that writes is a read to it,
> and only a server running the statement read-only refuses it.

> [spec:pgorm:def:read-only.sql-macro]
> `sql!` MUST expand to a `&'static Sql<Reads>` or `&'static Sql<Writes>`
> by the classification of its literal, as a constant expression every token
> of which is spanned at the invocation, so that clippy's `disallowed_macros`
> finds the macro (`scope.raw`). `prql!` MUST classify the SQL it compiled
> the same way and expand to that `Sql` beside its `Values`. `Sql<K>` MUST be
> `SqlText` of kind `K`, dereference to `str`, and give its text in a
> constant through the `const fn as_str`. Its constructor is hidden and is
> for the macros alone: calling it states a classification nothing checked.

## Where text runs

> [spec:pgorm:req:read-only.text]
> Each `Connection` statement method MUST take its statement only where the
> statement's kind `RunsOn` the connection: `Reads` on every `Connection`,
> `Writes` on every `RunsText` connection, which is every `ConnectionMut` and
> the `Unchecked` view (`read-only.unchecked`). `RunsText` MUST be sealed.
> Run-time text on a connection that may only read, and on code generic over
> `Connection`, MUST therefore not compile.
>
> A raw statement taken away from a connection MUST keep its kind:
> `SelectorRaw<S, K>` carries it (`Writes` by default), and its constructors
> (`from_statement`, `into_tuple`, `with_columns`), `Select::from_raw_sql`,
> `FromQueryResult::find_by_statement` and `DecodeRaw`, whose `Kind` is the
> text's, return the selector of the text's kind. Its terminals and its
> paginator MUST run only on a connection the kind `RunsOn`. What pgorm
> builds itself MUST be `Reads` when it reads — a `Selector`'s, a cursor's, a
> paginator's and a pipeline's statements — and `Writes` when it writes.
> Text a program builds by `SelectStatement::build` or `Pipeline::into_sql`
> is text, and `Writes`.

> [spec:pgorm:req:read-only.writes]
> Every write pgorm builds MUST take a `ConnectionMut`: `Insert`'s, `Update`'s
> and `Delete`'s terminals, an `ActiveModel`'s `insert`, `update`, `save` and
> `delete` and their `_for` and `_in` forms, the `ActiveModelBehavior` hooks,
> which run inside them, a model's `delete` and `delete_for`, the write
> versions (`exec_with_versions` and the rest), a disjoint write's
> `exec_returning_model`, `notify`, and pgorm-migration's runner and its
> migrations. A write given a connection that may only read MUST be a compile
> error (`read-only.diagnostics`).

## The read-only pool

> [spec:pgorm:req:read-only.pool]
> `connect_read_only(config)`, `connect_read_only_with_builder(config,
> build)` and `connect_read_only_with(config, tls, manager, build)` MUST
> build a `ReadOnlyPool` through `connect_with`, as lazily as a
> `DatabasePool`, each returning `Result`: a native `:memory:` target is
> refused before anything opens (`read-only.turso`). On PostgreSQL each MUST
> append `-c default_transaction_read_only=on` to the configuration's startup
> `options`, after any the caller gave so that it is the one that holds, and
> as a startup option so that `DISCARD ALL` and `RESET ALL` return the
> session to it. `ReadOnlyPool::get` MUST lend a `ReadOnlyConnection`, which
> implements `Connection`, `Transact`, `CopyConnection` and
> `VerifyStatement`, and not `ConnectionMut`. The pool offers `tag`,
> `status`, `close`, `is_closed` and `clear_statement_caches` as a
> `DatabasePool` does. Nothing MUST turn a read-only connection into one that
> may write.

> [spec:pgorm:req:read-only.unchecked]
> `ReadOnlyConnection::unchecked`, `ReadOnlyTransaction::unchecked`, and
> those of the metrics wrappers around them, MUST return an `Unchecked`
> borrowing the connection: a `Connection` that is `RunsText`, so SQL text of
> any kind runs on it, and is not `ConnectionMut`, so pgorm's own writes stay
> compile errors. Only a connection or transaction the server holds
> read-only MUST have one, since there the server refuses a write
> (`read-only.refusal`); its constructor is crate-private.

> [spec:pgorm:req:read-only.transactions]
> `TransactionMode` MUST be the modes of a transaction that may write:
> `Default` and `ReadWrite { isolation }`. A transaction that may only read
> MUST be opened by `read_only_transaction(isolation, f)`, `START TRANSACTION
> [ISOLATION LEVEL <isolation>] READ ONLY`, or `snapshot(f)`, `START
> TRANSACTION ISOLATION LEVEL SERIALIZABLE, READ ONLY, DEFERRABLE`, on a
> `DatabaseConnection`, a `ReadOnlyConnection` and the metrics wrappers
> around either, its closure given a `ReadOnlyTransaction` (an instrumented
> one on the wrappers), which implements `Connection`, `Transact`,
> `CopyConnection` and `VerifyStatement`, and not `ConnectionMut`. Its
> transactions are savepoints of the same kind. `transaction` on a
> `ReadOnlyConnection` MUST open a `ReadOnlyTransaction` under the session's
> own read-only default. They commit, roll back and report as `transaction`
> does (`conn.tx.closure`).

> [spec:pgorm:req:read-only.downgrade]
> `ConnectionMut::read_only(&self)` MUST lend the connection as a
> `ReadOnly`, which implements `Connection` alone and runs every read, by its
> type: the server is not asked, so the session keeps whatever it had, and no
> `unchecked` text runs on it. Nothing MUST turn a `ReadOnly` back into the
> connection it borrows.

> [spec:pgorm:req:read-only.copy]
> COPY MUST be split as `Connection` is: `copy_out`, `copy_out_columns` and
> `copy_out_rows` read and run on every `CopyConnection`, and `copy_in`,
> `copy_in_columns` and `copy_in_rows` write and run only where the
> connection is a `CopyConnectionMut: CopyConnection` too, each bounded by
> `Self: CopyConnectionMut` so that a COPY FROM on any other connection is the
> compile error `read-only.diagnostics` words rather than a method not found.
> A `ReadOnlyConnection` and a `ReadOnlyTransaction`, and the metrics wrappers
> around them, MUST implement `CopyConnection` alone. A `COPY` whose source is
> a write's `RETURNING` list writes as it reads, which its statement's type
> does not show: on a read-only connection the server refuses it.

> [spec:pgorm:req:read-only.metrics]
> `InstrumentedPool<M, P>` MUST instrument any `ConnectionPool` — a
> `DatabasePool`, the default, or a `ReadOnlyPool` — and its connections MUST
> be `InstrumentedConnection<M, P::Connection>`, so an instrumented read-only
> connection, its transactions (`InstrumentedTransaction<'_, M,
> ReadOnlyTransaction<'_>>`) and their savepoints implement `Connection`,
> `Transact` and `CopyConnection` and not their `Mut` traits, report as the
> instrumented ones that may write do (`metric.layer.delegate`,
> `metric.layer.tx`), and have `unchecked`. `ConnectionPool` MUST be sealed.

## Refusals

> [spec:pgorm:req:read-only.refusal]
> The server's refusal of a write in a transaction that may only read,
> SQLSTATE `25006`, MUST be matchable: `Error::read_only_refusal` returns the
> `ReadOnlyRefusal` carrying the server's message for an `Error::Postgres`
> holding it, and `None` for every other error. On a read-only connection it
> is what reports what the types cannot see: a `SELECT` calling a function
> that writes, a builder's row-locking clause, a `COPY` over a write, and
> text run through `unchecked`.

> [spec:pgorm:req:read-only.diagnostics]
> A write on a connection that may only read MUST be a compile error worded
> by pgorm, through `#[diagnostic::on_unimplemented]` on the trait the write
> needs. `ConnectionMut`'s MUST read "`` `{connection}` may only read, so it
> cannot run this write ``", its label "this writes, which needs a
> `ConnectionMut`", and a note naming the connections that may write and
> those that may only read. `RunsOn`'s, for SQL text, MUST read "``
> `{connection}` may only read, and this SQL text is not known to read ``",
> its label "text written at run time, or a `sql!` statement that may write",
> and notes naming what `sql!` admits as a read and the routes text that may
> write takes; the `Writes` impl is `do_not_recommend`, so this message, not
> `RunsText`'s, is the one reported. `CopyConnectionMut`'s MUST read "``
> `{connection}` may only read, so it cannot run COPY FROM ``". Each message
> MUST be held by a compile-fail snapshot.

## Native Turso

> [spec:pgorm:req:read-only.turso]
> The pinned engine opens an existing file read-only, refusing each write on
> it with SQLSTATE `25006`, and refuses to open `:memory:` or a missing file
> that way. Its process-wide registry hands an open database back to a later
> open of the same file whatever flags the later open asked for. So
> `connect_read_only` and its `_with` forms over a native file target MUST
> open the file read-only at the first acquisition, and MUST refuse
> `:memory:` with the driver's unsupported-operation error before anything
> opens (`Config::turso_read_only`). An open whose database came back with
> the other access — a read-only open of a file this process has open
> read-write, or a read-write open of one it has open read-only — MUST be
> refused with that error rather than handed out. COPY stays refused on a
> native connection (`conn.copy`).
