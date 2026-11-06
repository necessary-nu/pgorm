# Connections, Pooling, and Transactions

pgorm's connection layer is a thin composition over a vendored deadpool-postgres
fork (`pgorm-pool`) and `tokio-postgres`. `src/database/` exposes the pool and
connection handles plus the `ConnectionTrait` / `TransactionTrait` surface;
`pgorm-pool/src/` owns connection lifecycle, recycling, and statement caching.

## Pool construction and access

> [spec:pgorm:req:conn.pool+4]
> `connect_with(config, tls, manager, build)` is the general pool constructor,
> and the only one: every other entry point delegates to it. It takes the
> `tokio_postgres::Config`, a TLS connector bounded exactly as
> `pgorm_pool::Manager::from_config` bounds its own (`T: MakeTlsConnect<Socket>
> + Clone + Sync + Send + 'static`, plus `Send + Sync` on `T::Stream` and
> `T::TlsConnect` and `Send` on the connect future), the whole `ManagerConfig`,
> and a `FnOnce(PoolBuilder) -> PoolBuilder`. It builds the `Manager` from the
> first three, applies `build` to the resulting `PoolBuilder`, and returns
> `Result<DatabasePool, Error>`.
>
> It exists because `DatabasePool` wraps a crate-private `pgorm_pool::Pool` and
> has no other constructor, which made two things unreachable rather than merely
> inconvenient. `Pool` is TLS-erased (its `Manager` holds the connector as an
> `Arc<dyn Connect>`), so a TLS pool was
> always buildable inside `pgorm-pool` and never assignable to a `DatabasePool`
> — leaving every managed PostgreSQL that requires TLS unusable through pgorm.
> And the `Manager` was fully built before any caller hook could run, so
> `RecyclingMethod::Verified`/`Clean`/`Custom` and
> `StatementCacheSize::Bounded`/`Disabled` had no reachable setter at all:
> `ManagerConfig` is private once the `Manager` holds it, and `PoolBuilder`
> reaches only the pool. There is deliberately no `From<pgorm_pool::Pool> for
> DatabasePool` beside it — construction stays funnelled through one function,
> so a `DatabasePool` always holds a pool pgorm shaped.
>
> `ManagerConfig`, `RecyclingMethod`, `StatementCacheSize` and `PoolBuilder`,
> together with `NoTls`, `Socket`, `MakeTlsConnect` and `TlsConnect`, MUST be
> re-exported from `pgorm`. Every one of them is named in `connect_with`'s
> signature or its bounds, and calling it should not require depending on
> `pgorm-pool` or `tokio-postgres` directly.
>
> `connect(config: tokio_postgres::Config) -> DatabasePool` MUST delegate to
> `connect_with` with `NoTls`, `RecyclingMethod::Fast`, no tag, the default
> `StatementCacheSize`, and an identity builder closure — leaving the default
> deadpool pool configuration. Its signature is infallible by design, not by
> omission: `config` shapes the `Manager` and no caller input reaches the pool
> builder, so the only way the build can fail is if pgorm's own defaults are
> invalid. That is an internal invariant and MUST panic.
>
> `connect_with_builder(config, build)` MUST delegate with that same `NoTls` and
> those same manager defaults, applying the caller's closure to the
> `PoolBuilder` before building, and MUST return `Result<DatabasePool, Error>`.
> Because the closure is caller input, a builder shaped into an unbuildable pool
> — deadpool rejects timeouts configured without a runtime — MUST surface as
> `Error::Custom` carrying the builder's message, never as a panic. That is the
> rule for every fallible entry point here: `connect_with` itself, and
> `connect_multi_with_builder` (`conn.pool.multi`), fail the same way.

> [spec:pgorm:req:conn.driver-types]
> A crate depending on pgorm alone MUST be able to name every type of the
> crates pgorm is built on — tokio-postgres, deadpool, `bytes` and `futures` —
> that using pgorm makes it write. Each MUST be re-exported from pgorm's root
> under its own name, but for the driver's error, which is `PostgresError`
> because pgorm's own is `Error`. That is every such type pgorm's public
> signatures take or return:
>
> - `Row` and `RowStream`, which the `ConnectionTrait` statement methods
>   return;
> - `PostgresError`, which `Error::Postgres` carries and `BindRefusal::of` and
>   `DecodeRefusal::of` take;
> - `PoolError`, which `Error::Pool` carries, and `Status`, which
>   `DatabasePool::status` and `InstrumentedPool::status` return;
> - `BytesMut`, which `CopyRowEncoder` writes into, beside the `Bytes` of
>   `conn.copy`;
> - the `Stream` and `Sink` traits that the COPY sink and stream, the row
>   streams and `Notifications` implement;
>
> and every type a caller writes to use one of those, or one `conn.pool`
> re-exports:
>
> - `SqlState`, the code `PostgresError::code` returns and is compared
>   against, and `DbError`, the server's error `PostgresError::as_db_error`
>   returns;
> - `TimeoutType` and `HookError`, which `PoolError`'s variants carry;
> - `Runtime`, `Timeouts`, `QueueMode`, `PoolConfig` and `Hook`, which
>   `PoolBuilder`'s setters take: a pool timeout needs a runtime to time it;
> - `SslMode`, `SslNegotiation`, `TargetSessionAttrs`, `ChannelBinding`,
>   `LoadBalanceHosts` and `Host`, which `Config`'s setters take and its
>   getters return.
>
> Without them a program that sets a pool timeout, matches a SQLSTATE outside
> `SqlError`'s classes, or writes a helper taking a `Row` has to add the
> dependency pgorm already carries, at a version it has to keep in step by
> hand.

> [spec:pgorm:sem:conn.pool.get+1]
> `DatabasePool::get()` asynchronously acquires a pooled connection and wraps
> it as `DatabaseConnection`; acquisition failure surfaces as
> `Error::Pool(pgorm_pool::PoolError)` via `From`. `DatabasePool::status()`
> returns the live `deadpool::Status` (size, available, waiting) and
> `DatabasePool::tag()` returns the pool's tag as `Arc<String>`. An untagged
> pool receives a generated default tag of the form `default-{n}` from a
> process-wide monotonic counter in `pgorm-pool`.

> [spec:pgorm:req:conn.pool.no-conn-trait]
> `DatabasePool` MUST NOT implement `ConnectionTrait` (nor `Deref` to the inner
> pool). Executing a statement always requires explicitly acquiring a
> `DatabaseConnection` via `DatabasePool::get()` first. This is deliberate:
> implicitly checking out a fresh connection per statement hides pool churn and
> breaks transactional reasoning, so the pool-as-connection pattern inherited
> from SeaORM was removed.

> [spec:pgorm:sem:conn.pool.multi+4]
> `connect_multi_with(config, tls, manager, builders)` builds one pool per
> entry of a `BTreeMap<String, F>`, `F: FnOnce(PoolBuilder) -> PoolBuilder`,
> each through `connect_with` (`conn.pool`): each pool clones the shared
> `tokio_postgres::Config` and the TLS connector, which is bounded as
> `connect_with`'s is, takes `manager`'s recycling method, statement cache
> and statement-cache channel with its `tag` replaced by the entry's key, and
> is shaped by the entry's builder. It is the only route to several pools
> over TLS, or recycling with anything but `Fast`. `connect_multi_with_builder(config,
> builders)` delegates to it with `NoTls` and the manager defaults `connect`
> uses, so each of its pools recycles with `RecyclingMethod::Fast`; it keeps
> its `BTreeMap<String, Box<dyn Fn(PoolBuilder) -> PoolBuilder>>`. Either
> returns a `Result<DatabaseMultiPool, Error>`, the pools keyed by tag. Every
> builder is caller input, so construction is fallible on the same terms as
> `connect_with_builder` (`conn.pool`): the first entry whose builder cannot
> produce a pool aborts the whole construction with `Error::Custom` and no
> pool is returned.
>
> `DatabaseMultiPool` is built by these two alone; its map is crate-private. It offers `get(tag: &str) -> Option<&DatabasePool>`,
> `iter()` over `(&str, &DatabasePool)` in tag order, `status()` returning
> each pool's `Status` keyed by tag, and `close()`, which closes every pool
> (`conn.pool.lifecycle`). Selection is explicit by tag only — there is no
> round-robin, load balancing, or fallback across pools.
>
> The type once had no constructor at all: `connect_multi_with_builder`
> returned the bare map, and `DatabaseMultiPool`, public with its accessors,
> could not be obtained outside the crate.

> [spec:pgorm:req:conn.pool.config-forwarding+1]
> `pgorm_pool::Config` is the deserializable connection configuration — the
> shape `PG__HOST`, `PG__PASSWORD`, `PG__SSL_MODE` and their siblings land in.
> Every field it declares MUST reach the `tokio_postgres::Config` that
> `get_pg_config` builds, or else MUST NOT be declared. A field the struct
> accepts and the connection never sees is a setting the caller wrote and the
> database was never told about, and nothing anywhere reports the gap.
>
> Three fields were declared, given `From` conversions to their tokio-postgres
> counterparts, and then never applied: `target_session_attrs`,
> `channel_binding`, and `load_balance_hosts`. The conversions were dead code,
> and the settings were dropped in every spelling but one — a value embedded in
> `url` survived, because `tokio_postgres::Config::from_str` parses it — so the
> same setting was honoured or ignored depending on which way it was written.
>
> Two of the three lose more than configuration. `channel_binding: Require`
> dropped downgrades the connection to whatever binding the server will settle
> for, which is the opposite of what `Require` asks and is invisible.
> `target_session_attrs: ReadWrite` dropped lets a connection land on a hot
> standby, where the first write fails with SQLSTATE `25006` at run time — this
> setting exists precisely to move that failure to connect time.
>
> `ssl_mode` was already forwarded and stays so. The rule is over the whole
> struct, not over the three that happened to be missing.
>
> The four enums the struct mirrors from tokio-postgres — `SslMode`,
> `TargetSessionAttrs`, `ChannelBinding` and `LoadBalanceHosts` — are
> `#[non_exhaustive]` as tokio-postgres's own are, and so is `ConfigError`,
> what `get_pg_config` refuses a configuration with: each can grow
> (`sql.surface`).

> [spec:pgorm:sem:conn.pool.config-redaction]
> `Debug` on a pool configuration type MUST NOT reveal a credential.
> `pgorm_pool::Config` carries two. `password` is one, and the crate's own
> documentation tells the reader to fill it from `PG__PASSWORD`; `url` is the
> other, since a connection string's `userinfo` holds the same secret in a
> different spelling. A derived `Debug` printed both verbatim, so a single
> `tracing::info!(?cfg)` at startup put the database password in the logs.
>
> The impl is therefore hand-written. `password` prints as `_` when set and
> `None` when not — `tokio_postgres::Config`'s own redaction spelling, and the
> reason pgorm's `DatabasePool` Debug chain was already safe. `url` prints with
> its `userinfo` replaced by `_`, keeping the host, port, database and options
> after it: those are what make the value worth printing, and only the part
> before the `@` is secret. A `url` in libpq keyword/value form has no `@` to
> cut at, and its quoting is not worth re-implementing to find the boundary, so
> one that mentions `password` at all is withheld whole as `_`.
>
> `Serialize` is deliberately unchanged. Round-tripping the configuration is
> what it is for, and a serializer that dropped the password would emit a config
> that cannot connect; the two traits answer different questions, and only one
> of them answers into a log.

## Connection lifecycle in pgorm-pool

> [spec:pgorm:sem:conn.pool.lifecycle+3]
> `Manager::create` connects via `tokio_postgres`, spawns the connection
> driver future onto a tokio task, and hands out a `ClientWrapper` owning the
> client, the task's `JoinHandle`, and a fresh per-connection
> `StatementCache`; the cache is registered with the manager's
> `StatementCaches` registry, current with every clear asked of it so far.
> Before it connects, the first `create` of a pool whose `ManagerConfig`
> names a `statement_cache_channel` starts that pool's statement-cache
> listener (`conn.pool.statement-cache.listen`), and dropping the `Manager`
> stops it. Dropping a `ClientWrapper` aborts its connection task.
> `Manager::recycle` rejects clients whose `is_closed()` is true; otherwise it
> clears the connection's statement cache if a clear has been asked for since
> the cache was last cleared (`conn.pool.statement-cache.clear`), then runs
> the recycling method's query, if any. `detach` unregisters the connection's
> statement cache.

> [spec:pgorm:sem:conn.pool.recycle+1]
> `RecyclingMethod` selects the health check run on reuse: `Fast` (the
> default, and what `connect` uses) performs no query — only the `is_closed()`
> check; `Verified` executes an empty `simple_query` round trip; `Clean` runs
> a `DISCARD ALL`-like statement sequence (`CLOSE ALL; SET SESSION
> AUTHORIZATION DEFAULT; RESET ALL; UNLISTEN *; SELECT
> pg_advisory_unlock_all(); DISCARD TEMP; DISCARD SEQUENCES;`) that
> intentionally omits `DEALLOCATE ALL`/`DISCARD PLANS` so cached prepared
> statements survive recycling; `Custom(sql)` runs caller-provided SQL.
> `RecyclingMethod` is `#[non_exhaustive]`, so a further health check is not a
> breaking change (`sql.surface`).

> [spec:pgorm:sem:conn.pool.statement-cache+3]
> Each `ClientWrapper` carries an `Arc<StatementCache>` keyed by `(query text,
> parameter types)`. `prepare_cached` / `prepare_typed_cached` return the
> cached `tokio_postgres::Statement` on hit and prepare-then-insert on miss.
> `Transaction`s, nested transactions, and savepoints created through the
> wrapper share the owning client's cache. The manager-level `StatementCaches`
> holds weak references to every live cache and supports `clear()` and
> `remove(query, types)` across all pooled connections, at once and whether
> or not a connection is checked out, and `clear_before_reuse()`, which clears
> each cache before its connection is next lent
> (`conn.pool.statement-cache.clear`).
>
> The cache is on the ordinary execution path, not beside it, and it is the
> whole of that path: each of `ConnectionTrait`'s six statement methods
> resolves its SQL through the cache (`conn.pool.conn-trait`), with no
> unresolved alternative beside it, so one text is parsed once per connection
> rather than once per call. What still bypasses it is `tokio_postgres`'s own
> statement surface — `Client::query`, `Transaction::execute` and their
> siblings prepare an unnamed statement per call and consult nothing — which is
> what pgorm-pool's `Client` exposes by `Deref`, what its `Transaction`
> forwards, and what a caller reaching past `ConnectionTrait` gets.
>
> A cached statement outlives the call that prepared it. That is the point, and
> three things follow from it: the cache is capacity-bounded
> (`conn.pool.statement-cache.bound`), because the key space is not; a cached
> plan can be invalidated by DDL under it
> (`conn.pool.statement-cache.invalidate`); and DDL can change what a plan
> was built from without invalidating it, which a pool-wide clear answers
> (`conn.pool.statement-cache.clear`).

> [spec:pgorm:req:conn.pool.statement-cache.bound+1]
> A `StatementCache` MUST be bounded. Its key space is the SQL text, and one
> logical query spreads across many texts — an `IN` list rendered with a
> placeholder per element is a different text at every arity, 25 of them for one
> measured query — while every entry is a live server-side prepared statement
> that `Fast` and `Clean` recycling both deliberately keep
> (`conn.pool.recycle`). Unbounded, the cache would hold statements the
> connection will never run again for as long as the connection lives.
>
> `ManagerConfig::statement_cache: StatementCacheSize` carries the bound, per
> connection: `Bounded(NonZeroUsize)`, defaulting to 256, or `Disabled`. There
> is deliberately no unbounded variant — that is the growth the type exists to
> stop, and a caller wanting an effectively unlimited cache says so with a large
> `Bounded` — and no zero bound, which would be `Disabled` spelled a second way.
> The default is an order of magnitude above the worst measured spread of a
> single query and keeps even a large pool's server-side statement count in the
> low thousands.
>
> Inserting into a full cache MUST evict an existing entry before inserting, and
> MUST NOT panic or refuse. The victim is whichever entry the map yields first,
> not the least recently used: recency would have to be written on every lookup,
> making the read lock a cache hit takes today an exclusive one, and the bound
> exists to stop growth rather than to maximise the hit rate. Re-inserting a key
> the cache already holds replaces it and evicts nothing.
>
> `Disabled` stores and looks up nothing, so `prepare_cached` is
> `prepare_typed` itself: every call parses its own statement and closes it when
> the last handle drops, which is the behaviour that predates the routing of
> `conn.pool.conn-trait`. It is the opt-out for a caller who would rather pay
> the parse than reason about a plan cached across DDL.
>
> Evicting a statement — like disabling the cache, and like recycling with SQL
> that deallocates — drops pgorm's last handle to it only once the caller has
> dropped the rows it produced, because a `tokio_postgres::Row` holds the
> statement it was decoded against. The `Close` reaches the server when that
> happens, not when the entry leaves the map.
>
> pgorm's `connect`, `connect_with_builder` and `connect_multi_with_builder`
> (`conn.pool`, `conn.pool.multi`) always take the default. The knob is reached
> through `connect_with`, the general entry point those three delegate to, which
> takes the whole `ManagerConfig`.

> [spec:pgorm:req:conn.pool.statement-cache.invalidate+4]
> Reusing a prepared statement admits one failure that preparing afresh does
> not: PostgreSQL raises SQLSTATE `0A000` — *cached plan must not change result
> type* — when a statement's plan is revalidated and the result it would now
> produce no longer matches the description the client was given. DDL under a
> live cache entry is how that happens: `SELECT *` over a table that gained a
> column, or a projection whose column changed type. Three neighbouring cases
> were probed and are NOT hazards. PostgreSQL re-plans two of them itself:
> changing `search_path` so the same text resolves to a different table, and
> dropping and recreating the table a cached statement names. The third is a
> statement first prepared inside a transaction that then rolls back — the cache
> is the connection's, shared with every transaction on it (`conn.tx`), so this
> would leave an entry naming a statement the rollback had discarded. It does
> not: a statement parsed over the extended protocol is not undone by
> `ROLLBACK`, and the cached entry keeps working.
>
> When a statement resolved through the cache fails with `0A000`, pgorm MUST
> evict that key. Whether it then retries depends on whether a transaction is
> open, because `0A000` is an error and an error aborts an open transaction.
>
> Outside a transaction, pgorm MUST re-prepare the evicted key exactly once and
> execute again. A second `0A000` MUST reach the caller as the `Error::Postgres`
> it is: the recovery is a single retry, never a loop, because a plan that is
> stale twice running is not a plan going stale.
>
> Inside a transaction, pgorm MUST NOT retry, and MUST surface the `0A000`
> unchanged. The transaction is already aborted by the time the error arrives,
> so the re-prepare fails on its own account with SQLSTATE `25P02`, *current
> transaction is aborted*, and it is that error the caller would be handed: the
> one fact they can act on, displaced by a consequence of pgorm's own retry.
> More generally, no recovery pgorm attempts may substitute its own failure for
> the failure it was recovering from. The eviction still happens, so the
> caller's next attempt prepares afresh, and the recovery is theirs to run at
> the boundary they own — retry the transaction (`conn.tx.retry`), or roll back
> to a savepoint opened as a nested transaction (`conn.tx`) and re-run inside
> the still-live outer one.
>
> Retrying under an implicit savepoint is rejected rather than merely
> unimplemented. It is the only shape recovery inside a transaction could take,
> since an abort cannot be undone after the fact and the savepoint would
> therefore have to be taken before every cached execution rather than on
> failure — two extra round trips on every in-transaction statement, to hide a
> DDL race, spent by machinery whose purpose (`conn.pool.statement-cache`) is to
> remove round trips. It would also duplicate a facility callers already reach
> explicitly: a nested transaction is a savepoint (`conn.tx`), so whoever wants a
> statement fenced can fence it, and whoever does not is not charged for it.
>
> Which of the two applies is decided by the type, not by asking the server.
> `ConnectionTrait` on `DatabaseConnection` and `&DatabaseConnection` is the
> retrying path, and on `DatabaseTransaction` — with `&mut DatabaseTransaction`
> forwarding to it (`conn.pool.conn-trait`) — the evict-only one. No third case
> hides between them: a transaction's closure borrows the connection
> exclusively (`conn.tx`), so while the transaction is open the borrow checker
> forbids reaching the connection's impls at all.
>
> `execute_raw` and `query_raw` never retry, transaction or no transaction. They
> take an `IntoIterator` consumed by the first attempt, which is not `Clone` and
> cannot be held across the retry without a `Send` bound `ConnectionTrait` does
> not carry, so they evict the rejected key and return the error — which leaves
> the next call to re-prepare, making the recovery one call later rather than
> absent. The four methods whose parameters are a reusable `&[&(dyn ToSql +
> Sync)]` slice are the ones that *can* retry, and outside a transaction they
> do. Eviction itself has no exception: every statement `ConnectionTrait`
> accepts is text (`conn.sql-text`), so every one is cache-resolved and every
> one can be prepared again from what the caller passed.
>
> `0A000` is also PostgreSQL's generic *feature not supported*, and nothing but
> the message text — which is localized — separates the two. A statement
> rejected on its own merits is therefore treated the same way: outside a
> transaction it is retried once and fails identically, costing one round trip
> on a call that was already failing; inside one it costs an eviction, so the
> next call parses the text again. That is preferred to matching on prose, and
> it is also why `0A000` is NOT added to the retryable SQLSTATEs of
> `conn.tx.retry` — `transaction_with_retry` would replay whole transactions
> against a permanently unsupported feature. A caller who knows their statements
> reach no such feature can classify it as retryable themselves, since
> `RetryableError` is implemented on the closure's own error type.
>
> One case is out of reach of this rule rather than absent from it. A recycling
> method whose SQL deallocates — `Custom("DISCARD ALL")`, which is exactly what
> `Clean` avoids (`conn.pool.recycle`) — drops every server-side statement while
> the cache keeps naming them, and the next use fails with SQLSTATE `26000`,
> which is not retried. `connect` and its two sibling shorthands recycle with
> `Fast` (`conn.pool`), so they cannot reach it; a caller who passes such a
> method to `connect_with` should pair it with `StatementCacheSize::Disabled`.

> [spec:pgorm:req:conn.pool.statement-cache.clear+1]
> `DatabasePool::clear_statement_caches()` MUST drop every pooled
> connection's cached prepared statements, so that each statement is prepared
> again the next time a connection runs it. It is for DDL that changes what a
> plan was built from without PostgreSQL re-planning it — a domain's changed
> default is one (`sql.ddl.domain`) — and it spares the one `0A000` failure
> per connection that a changed result type otherwise costs
> (`conn.pool.statement-cache.invalidate`).
>
> It MUST NOT clear a cache in place. It advances a generation held by the
> pool's `StatementCaches` (`StatementCaches::clear_before_reuse` in
> pgorm-pool), and each connection records the generation its cache was last
> cleared at — the current one when the connection is created, its cache then
> being empty. `Manager::recycle` (`conn.pool.lifecycle`), which runs while
> the pool holds a connection idle and before it lends it, clears the cache of
> a connection whose generation is behind and records the current one. An idle
> connection is therefore cleared before it is next lent, and a connection
> checked out when the call is made keeps its statements until it is returned
> and is cleared before it is lent again: a caller holding one across the DDL
> returns it and takes another to see the change. Clearing in place, as
> `StatementCaches::clear` does, races the owner of a checked-out connection,
> because a statement whose preparation is in flight lands in the cache after
> the clear has passed it, planned against the schema the DDL replaced. An
> idle connection has nothing in flight.
>
> The server-side statements MUST be deallocated, not only forgotten: a cached
> entry is a live prepared statement on the server, and one the client drops
> without closing stays there until the connection closes. Dropping a
> `tokio_postgres::Statement`'s last handle sends its `Close`, so clearing the
> map is the deallocation, deferred only for a statement a caller's `Row`
> still holds (`conn.pool.statement-cache.bound`), and the `Close` is queued
> ahead of the connection's next request.
>
> The call sends nothing, cannot fail, and does not wait. Generations only
> advance, so a connection behind by several clears is cleared once, and one
> created after the call is not cleared at all. `InstrumentedPool` forwards it
> to the pool it wraps (`metric.layer`); the Python and JavaScript pools expose
> it as `clear_statement_caches()` and `clearStatementCaches()`
> (`bindings.connections`); and pgorm-migration's `up`
> calls it once a batch that applied a migration commits (`migration.up`).
>
> It reaches only the pool it is called on, and so only the process holding
> that pool. A pool in another process on the same database clears when a
> notification arrives on the channel it listens on, if it was built to
> (`conn.pool.statement-cache.listen`), and `up` sends one as its batch
> commits.

> [spec:pgorm:req:conn.pool.statement-cache.listen+1]
> `ManagerConfig::statement_cache_channel: Option<String>` carries a clear
> between processes. A pool given a channel keeps one connection of its own
> `LISTEN`ing on it (`conn.listen`) and, whenever a notification arrives
> there, advances its `StatementCaches` generation exactly as
> `clear_before_reuse` does (`conn.pool.statement-cache.clear`). The payload
> is ignored: any notification on the channel clears, so a sender agrees with
> its listeners on the name alone. The setting MUST be off by default,
> `None`, because it costs each pool that takes it one idle connection, and
> `connect`, `connect_with_builder` and `connect_multi_with_builder` leave it
> off (`conn.pool`); `connect_with` is how it is set. `STATEMENT_CACHE_CHANNEL`,
> `pgorm_statement_caches`, defined in pgorm-pool and re-exported by pgorm, is
> the channel pgorm-migration's runner notifies unless told otherwise
> (`migration.up`). A notification reaches only sessions connected to the
> database it was sent in, so one name serves every database on a server
> without a clear crossing between them.
>
> The listening connection is opened through `Manager::connect_raw`, with the
> pool's configuration and TLS connector, as `DatabasePool::listener`'s is: it
> is not taken from the pool, counts against no size, has no statement cache,
> and is never lent or recycled. The pool's first `Manager::create` starts it,
> just before that connection opens (`conn.pool.lifecycle`), rather than the
> pool's construction: a pool is built in a synchronous call that may run
> outside any runtime, as the Python and JavaScript pools' are, and a pool
> that has opened no connection has prepared nothing to clear. Its task holds
> the pool's connector, configuration and generation, and nothing that keeps
> the pool alive.
>
> Unlike a caller's `Listener`, which reports a lost connection and never
> reconnects, this one MUST survive losing its connection, since nobody reads
> its errors. When the connection fails, is terminated, or cannot be opened,
> it opens another after a wait of 100 ms that doubles with each attempt that
> fails to `LISTEN`, up to 10 s, and returns to 100 ms once a `LISTEN` takes
> effect. Each time its `LISTEN` takes effect, the first time included, it
> MUST clear once: a notification sent while no session of the pool listened
> reached none, and a connection the pool opened before the first `LISTEN`
> may have prepared statements before a notification it could not hear. Each
> loss is logged through `tracing` as a warning, and each clear it makes at
> debug level. A hot standby keeps it from settling: it refuses `LISTEN`,
> and receives no notification anyway, so the listener retries there at its
> longest wait.
>
> Its session MUST NOT idle out. Before its `LISTEN` it sends `SET
> idle_session_timeout = 0`, which reaches that session alone and which any
> role may send, the setting's context being `user`; a server, database or
> role whose timeout ends idle sessions would otherwise end this one each
> period, and each reconnection would clear the pool once more. A periodic
> no-op would also keep it alive, at a round trip each period and a period
> that must undercut a timeout it does not know. A `SET` the server refuses
> is logged as a warning and the `LISTEN` goes ahead without it. The
> `LISTEN` is sent alone and last, so it is the statement
> `pg_stat_activity` shows for the session. What ends an idle connection
> outside the server, a pooler's or a network middlebox's timeout, still
> ends this one; TCP keepalives in the pool's `Config` reach the listener
> as they reach the pool's own connections.
>
> `DatabasePool::close` MUST stop the listener, closing its connection,
> through `Manager::stop_statement_cache_listener`, which also keeps a pool
> that has not yet connected from starting one. `pgorm_pool::Pool::close` does
> not reach the manager, so a pool closed through pgorm-pool alone listens
> until it is dropped. Dropping a pool's last handle drops its `Manager`,
> whose drop MUST stop the listener as well, so a dropped pool leaks neither
> the task nor its connection.
>
> `connect_with` MUST refuse a channel no notification could reach with
> `Error::Custom`, before anything connects: an empty name, which `LISTEN`
> refuses, and one longer than 63 bytes, which `LISTEN` truncates with only a
> notice while `pg_notify` refuses it whole. A `Manager` built through
> pgorm-pool alone with such a name logs an error and listens on nothing.
>
> The clear is the deferred one `conn.pool.statement-cache.clear` describes:
> an idle connection is cleared before it is next lent, and one checked out
> across it — a Rust `DatabaseConnection`, or a Python or JavaScript
> `Connection` held for the application's life — keeps its statements until
> it is returned. The metrics layer does not see it (`metric.layer`): the
> listener runs inside pgorm-pool, below the `InstrumentedPool` that wraps the
> built pool, and runs no statement a caller wrote, while `MetricsCollector`'s
> seven hooks report statements, transactions and acquisitions and have no
> defaults, so a hook for this would be a change every collector had to make.
> The Python and JavaScript pools take the setting as
> `statement_cache_channel` and `statementCacheChannel`, and export
> `STATEMENT_CACHE_CHANNEL` (`bindings.connections`).

## Statement execution surface

> [spec:pgorm:def:conn.sql-text+2]
> `SqlText` answers `fn sql_text(&self) -> &str` for a statement, and is what
> `ConnectionTrait` means by one. It MUST be sealed against implementations
> outside pgorm, and MUST be implemented for exactly `str` and `String` — a
> statement is its SQL text, in whichever of the two spellings the caller holds
> it, and there is deliberately no third form.
>
> What the seal excludes is tokio-postgres's prepared `Statement`.
> `ToStatement`, which bounds tokio-postgres's own statement surface, admits
> `str`, `String` and `Statement` alike; but a `Statement` names a statement
> that exists on the single connection it was prepared against, so running one
> on any other connection type-checks and then fails on the wire with SQLSTATE
> `26000`, *prepared statement does not exist*. Under a pool, which connection
> a call runs on is not the caller's to choose, so a bound that accepts a
> `Statement` is a bound that accepts a statement bound to a connection nobody
> named. Removing the type from the bound removes the call rather than
> diagnosing it (`[dec:pgorm:invalid-states-unrepresentable]`), and costs
> nothing pgorm could have used: a `Statement` retains its server-side name,
> parameter types and result columns but not the text it was prepared from, so
> it is exactly the statement pgorm can neither cache nor fingerprint.
>
> Because a statement is nothing but text, `sql_text` is total — no `Option`,
> and no absent case for its readers to carry. There are two:
> `conn.pool.conn-trait` keys the statement cache on it, and
> `metric.fingerprint` identifies the query it reports on by it.
>
> The narrowing is `ConnectionTrait`'s alone. `pgorm_pool`'s `GenericClient`
> (`conn.pool.generic-client`) and the `tokio_postgres` surface that pgorm-pool's
> `Client` exposes by `Deref` and its `Transaction` forwards keep
> `ToStatement`: those are
> tokio-postgres's own contract, reached past `ConnectionTrait` with a
> particular connection already in hand, and there a prepared `Statement` is
> the point rather than the hazard.

> [spec:pgorm:def:conn.pool.hook-connection]
> `HookConnection<'a>` is the connection a pool `Hook` is given, as a
> `ConnectionTrait` (`conn.pool.conn-trait`): `HookConnection::new(client)`
> borrows the pooled client deadpool passes the hook for the hook's length.
> Its statements resolve through that connection's statement cache, evicted
> and retried once as a `DatabaseConnection`'s are
> (`conn.pool.statement-cache.invalidate`), so pgorm's statements — an
> entity's `find`, a pipeline's terminal — run in a hook, and a statement a
> hook prepares is the one the connection's later borrowers find cached. It
> opens no transaction and offers nothing beyond the trait. The client's
> own tokio-postgres methods stay what a hook calls when it names none of
> pgorm's: the type is a wrapper, and implementing the trait on the client
> itself would turn the hook's `client.query_one` into pgorm's, changing the
> error it returns. It is how the bindings run a hook's statements
> (`bindings.pool-hooks`).

> [spec:pgorm:def:conn.pool.conn-trait+10]
> `ConnectionTrait` is the uniform statement-execution surface over
> connections and transactions. It defines seven async methods. Six are
> generic over `T: ?Sized + SqlText + Sync` — the statement is SQL text and
> nothing else (`conn.sql-text`), which is why `ToStatement` is named nowhere
> in this trait and a `Statement` prepared on some other connection does not
> typecheck — with parameter binding
> (no string interpolation): `execute(stmt, params) -> u64` (affected-row
> count), `execute_raw(stmt, params)` taking an `ExactSizeIterator` of
> `BorrowToSql` values instead of a `&[&dyn ToSql]` slice, `query_one ->
> Row` (errors if not exactly one row), `query_opt -> Option<Row>`,
> `query_all -> Vec<Row>`, and `query_raw(stmt, params) -> RowStream`,
> which takes the same `BorrowToSql` iterator as `execute_raw` and returns
> the unbuffered row stream of `exec.stream`. Errors map to
> `Error::Postgres`.
>
> All six resolve the statement through the connection's `StatementCache`
> (`conn.pool.statement-cache`) before executing it, so a text executed twice on
> one connection is parsed once: the text is looked up, and prepared and
> inserted on a miss. There is no second route, because there is no statement
> without text to take one. What the cache returns is prepared on, and used
> only on, the connection the call is running on, so no prepared statement
> crosses connections at all. A rejected cached plan is evicted under
> `conn.pool.statement-cache.invalidate`, and retried there only outside a
> transaction: inside one the rejection has already aborted the transaction, so
> the original error is what the caller gets.
>
> The seventh, `batch_execute(sql: &str) -> ()`, is neither generic nor
> parameterized: it sends `sql` through the simple-query protocol, so the
> string MAY hold several `;`-separated statements, and it returns no rows.
> It is the only method that accepts a multi-statement string — the other six
> go through the extended protocol, where a prepared statement carries exactly
> one command and PostgreSQL rejects a second with *cannot insert multiple
> commands into a prepared statement*. Execution stops at the first statement
> that fails. Nothing is prepared, so the statement cache
> (`conn.pool.statement-cache`) is bypassed by construction rather than by
> policy. It exists for DDL, migration, and fixture surfaces, where a script
> is the unit of work; since values can reach it only by interpolation, `sql`
> must be built from trusted input.
>
> It is implemented for `DatabaseConnection`, `&DatabaseConnection`,
> `DatabaseTransaction` and `HookConnection` (`conn.pool.hook-connection`),
> each resolving through the cache its underlying `pgorm-pool` client or
> transaction owns and then delegating to it — they share one cache per
> physical connection, since a transaction's is its client's (`conn.tx`) and
> a hook's connection is the client itself — with `batch_execute` going through
> `GenericClient::batch_execute` (`conn.pool.generic-client`), which is
> otherwise unreachable from pgorm because the wrapper types' inner fields are
> crate-private, and a hook's connection's through its client's own.
>
> One further impl carries no execution of its own: `&mut C`, for every
> implementor `C`, forwards every method through the exclusive borrow. It
> exists because a transaction closure is given its handle as `&mut` — `&mut
> DatabaseTransaction<'_>` on a connection or a transaction, `&mut
> InstrumentedTransaction<'_, M>` on the metrics wrappers (`metric.layer.tx`)
> — and a helper generic over `C: ConnectionTrait` taking `&C` cannot be
> passed one without it: `&mut T` is its own type, distinct from `T`, and the
> coercion to `&T` does not fire into an inference variable, so the call site
> would otherwise have to reborrow as `&*txn`. It is one impl over every
> implementor rather than one per handle type so that no closure's handle can
> be left out, as the instrumented one once was. Being a distinct type is also
> why it cannot overlap the impl for `C` itself.

> [spec:pgorm:def:conn.pool.generic-client]
> `pgorm_pool::GenericClient` is the sealed trait (`Sync` + a private
> `Sealed` supertrait) unifying `Client` — the deadpool `Object` wrapping a
> `ClientWrapper` — and `Transaction<'_>` behind one statement surface. It
> is a 1:1 copy of `tokio_postgres::GenericClient` as of tokio-postgres
> 0.7.7 with two deliberate changes: the `client()` accessor is removed,
> and `prepare_cached` / `prepare_typed_cached` are added. Sealing limits
> implementors to exactly those two types.
>
> Its methods — `execute`, `execute_raw`, `query`, `query_one`,
> `query_opt`, `query_raw`, `prepare`, `prepare_typed`, `prepare_cached`,
> `prepare_typed_cached`, `transaction(&mut self)`, `batch_execute` —
> delegate directly to the corresponding `tokio_postgres::Client` /
> `tokio_postgres::Transaction` method, except the cached prepares and
> `transaction`, which route through the pool wrapper types
> (`ClientWrapper`, pgorm-pool's `Transaction`) so both levels share the
> per-connection `StatementCache` (`conn.pool.statement-cache`) and nested
> transactions go through the wrapper's savepoint logic.

> [spec:pgorm:req:conn.pool.savepoint-name+3]
> `pgorm_pool::Transaction::savepoint(name)` MUST quote `name` as a
> PostgreSQL identifier — every `"` in it doubled, the whole wrapped in
> `"` — before handing it to `tokio_postgres`.
>
> `SAVEPOINT` takes no parameters, so unlike every other value pgorm
> sends (`conn.pool.conn-trait`) the name can only travel as SQL text.
> `tokio_postgres` interpolates it into `SAVEPOINT {name}` and runs that
> through `batch_execute` — the simple query protocol, which accepts
> several statements in one message — so an unquoted runtime name is
> executable SQL: `x"; DROP TABLE t; --` would be a statement, not a
> savepoint. It also *stores* the name and reuses it for `RELEASE {name}`
> and `ROLLBACK TO {name}`, so quoting once before delegation is what
> covers all three.
>
> The quoting MUST be total, because the signature has no refusal to
> return that pgorm can build: its error type is `tokio_postgres::Error`,
> which has no public constructor. Totality is available: doubling is
> PostgreSQL's own spelling for an embedded delimiter, and the delimiter
> is the only character a quoted identifier gives meaning to. Bounding
> the parameter to `&'static str` instead would answer the injection but
> would also delete the runtime-name capability this inherited
> deadpool-postgres API has, which callers may be using legitimately.
>
> Two consequences are deliberate. A name keeps its case and its
> punctuation rather than folding to lower case — that is what quoting an
> identifier means, and within one handle it is invisible, since the
> release and the rollback quote it identically. And a name holding a NUL
> byte is an `Err`: PostgreSQL carries no identifier containing one under
> any quoting at all — the same closed judgment
> `[spec:pgorm:req:pipeline.errors]` makes one layer up — and the wire
> encoder refuses an embedded NUL in the query message before a byte is
> sent, so the failure arrives client-side with the transaction
> untouched. It is refused, never stripped: a name silently altered to be
> sendable is a savepoint the caller did not ask for.
>
> Nested transactions opened through `Transaction::transaction` and
> `GenericClient::transaction` (`conn.pool.generic-client`) are unaffected
> — `tokio_postgres` names those itself, `sp_{depth}`, from no caller
> input.
>
> The quoted method MUST be the only route to a savepoint name. pgorm-pool's
> `Transaction` therefore does not `Deref` to `tokio_postgres::Transaction`,
> whose own `savepoint` interpolates the name unquoted: through a `Deref`,
> `tokio_postgres::Transaction::savepoint(&mut *txn, name)` compiled and
> skipped the quoting. The wrapper forwards the statement methods its
> callers use instead — `execute`, `execute_raw`, `query`, `query_one`,
> `query_opt`, `query_raw`, `batch_execute`, `copy_in` and `copy_out`, each a
> call with the caller's own statement — and holds the inner transaction
> privately. `copy_in` adds the guard `conn.copy` requires of a COPY FROM
> refused at its start.

## Transactions

> [spec:pgorm:req:conn.tx+3]
> A transaction MUST be opened only by running a closure inside one:
> `TransactionTrait::transaction(f)`, and on a `DatabaseConnection` the
> inherent `transaction_with(mode, f)` and `transaction_with_retry`
> (`conn.tx.closure`, `conn.tx.retry`). No public method returns a transaction
> handle, commits one or rolls one back. The closure is given
> `&mut DatabaseTransaction<'_>`, borrowing the parent exclusively, so no other
> statement can run on the connection while the transaction is open, and what
> the closure returns ends it. On a `DatabaseConnection` the transaction is
> `BEGIN` on the pooled client; on a `DatabaseTransaction` it is a nested
> transaction, which `tokio_postgres` implements as a savepoint. Both
> transaction levels share the connection's statement cache.
>
> `TransactionTrait` is sealed, and its handle is the receiver's kind:
> `DatabaseTransaction` on a connection or a transaction, the
> `InstrumentedTransaction` of `metric.layer.tx` on the metrics wrappers, each a
> `TransactionTrait` itself. Opening and ending a transaction are methods of
> crate-private traits that take a token no other crate can construct, so even
> code generic over `TransactionTrait`, which has those methods in reach
> through its bound, cannot call them. The handle type is declared on a trait
> of its own, apart from the one that opens it: a trait that declares a
> lifetime-generic type and returns it from `&mut self` must bound it by
> `Self: 't`, which a closure generic over every `'t` cannot satisfy for a
> parent that is itself borrowed. Opening is generic over the borrow alone,
> never in an implementation that relates the handle's lifetime to the
> borrow's: a spawned task's `Send` check erases every lifetime in the future
> it holds and needs each implementation it meets to hold for any of them.
>
> `DatabaseConnection::transaction_with(mode: TransactionMode, f)` is the
> configured counterpart, opening the transaction through `pgorm_pool`'s
> `TransactionBuilder`, which emits `START TRANSACTION` followed by the clauses
> the mode selects. `TransactionMode` MUST be a closed `Debug + Clone + Copy +
> Default` enum of exactly four variants, each one a combination PostgreSQL
> acts on:
>
> - `Default` — the `Default::default()` variant — appends no clause. The bare
>   `START TRANSACTION` inherits `default_transaction_isolation`,
>   `default_transaction_read_only`, and `default_transaction_deferrable`, so it
>   opens the same transaction `transaction` does.
> - `ReadWrite { isolation: Option<IsolationLevel> }` appends `ISOLATION LEVEL
>   <level>` when `isolation` is `Some`, then `READ WRITE`. That clause is
>   emitted unconditionally rather than elided as the usual server default, so
>   the mode overrides a session running under `SET
>   default_transaction_read_only = on` instead of inheriting it.
> - `ReadOnly { isolation: Option<IsolationLevel> }` appends `ISOLATION LEVEL
>   <level>` when `isolation` is `Some`, then `READ ONLY`. A write inside such a
>   transaction is rejected by the server with SQLSTATE `25006`
>   (`read_only_sql_transaction`).
> - `DeferrableSnapshot` appends `ISOLATION LEVEL SERIALIZABLE, READ ONLY,
>   DEFERRABLE`, and takes no parameter. Opening it may block until the snapshot
>   is safe, after which the transaction cannot be aborted by a serialization
>   failure.
>
> `DeferrableSnapshot` MUST be the only shape that reaches `DEFERRABLE`, and it
> MUST carry the isolation level and access mode itself rather than accepting
> either from the caller. PostgreSQL honours `DEFERRABLE` only when the
> transaction is both `SERIALIZABLE` and `READ ONLY`; in every other
> combination the server parses the clause and ignores it. Deferrability as an
> independent flag would therefore be constructible, accepted, and a silent
> no-op — an invalid state the type MUST NOT be able to represent. For the same
> reason no variant offers `NOT DEFERRABLE`: it is the server default, and
> selecting it says nothing.
>
> `IsolationLevel` is `tokio_postgres::IsolationLevel`, re-exported from
> `pgorm`.
>
> `transaction_with` is an inherent method on `DatabaseConnection`,
> deliberately NOT on `TransactionTrait`: the trait's other implementors open
> savepoints, and `SAVEPOINT` accepts no isolation level, access mode, or
> deferrability. Putting the configured form on the trait would require the
> savepoint implementation to ignore or reject its argument, so it is offered
> only where PostgreSQL honours it.

> [spec:pgorm:sem:conn.tx.guard+4]
> `DatabaseTransaction` wraps `Option<pgorm_pool::Transaction>` as the handle a
> transaction closure is given, and a transaction has three ends. Two are
> awaited and chosen by the closure (`conn.tx.closure`): `COMMIT` after `Ok`,
> `ROLLBACK` after `Err`, each consuming the handle, `take()`-ing the inner
> transaction and awaiting the server's answer, mapping failure to
> `Error::Postgres`, and a `COMMIT` an aborted transaction could not make to
> `Error::TransactionAborted` (`conn.tx.aborted`); neither is public. Because both take the `Option`, the
> `Drop` impl's `tracing::warn!("Transaction dropped without committing!")`
> fires only on the third: the closure's future dropped before it finished —
> a timeout, a cancelled task, a `select!` that took another branch — or a
> panic unwinding through it.
>
> The two rollbacks are not equivalent. The closure's awaits the round trip.
> The dropped handle's is fire-and-forget: `tokio_postgres::Transaction::drop`
> synchronously enqueues a raw `ROLLBACK` (`ROLLBACK TO <savepoint>` when
> nested) onto the connection's unbounded request channel and discards both the
> response stream and any send error, so the rollback's outcome is
> unobservable and the warning is the only trace of it.
>
> The queued rollback is nevertheless correctly ordered against subsequent work
> on the same connection. The enqueue happens inside `Drop`, and the exclusive
> `&mut` borrow the transaction holds on the client (`conn.tx`) is not released
> until that drop completes, so no later statement can reach the channel first;
> the connection driver task drains the channel in FIFO order. That task is
> owned by `ClientWrapper` and aborted only when the wrapper is dropped
> (`conn.pool.lifecycle`) — pool recycling borrows the wrapper rather than
> replacing it — so a `ROLLBACK` queued as the handle drops is still flushed
> after the connection returns to the pool and is handed to the next caller.

> [spec:pgorm:sem:conn.tx.closure+6]
> `TransactionTrait::transaction(f)` and `DatabaseConnection::transaction_with(opts,
> f)` run a closure inside a transaction, taking `F: AsyncFnOnce(&mut
> Self::Transaction<'_>) -> Result<T, E>` — `&mut DatabaseTransaction<'_>` on a
> connection or a transaction — a native `AsyncFn*` bound, not a boxed future.
> `transaction` is one default body on the trait, the same for every
> implementor, opening through the sealed means of `conn.tx`;
> `transaction_with` opens with `opts`; the rest of the cycle is shared.
> `Ok(value)` is followed by `COMMIT`, and `value` is returned only once the
> `COMMIT` succeeded and committed: a transaction that a statement failing
> inside it had aborted, the error caught by the closure rather than
> returned, is rolled back instead, and that is an error
> (`conn.tx.aborted`). `Err(e)` is followed by an *awaited* `ROLLBACK`, so the
> transaction is over by the time the caller is resumed — one of the awaited
> ends of `conn.tx.guard`, not its fire-and-forget drop, which is why the
> closure's rollback does not inherit that end's unobservable outcome. A
> future dropped before the closure finishes takes the drop.
>
> The bound is higher-ranked over the transaction's lifetime, and on a
> concrete receiver every closure form — these two and `transaction_with_retry`
> (`conn.tx.retry`) — MUST return a future that is `Send` whenever the
> closure's is, so the call runs inside `tokio::spawn` or a handler a web
> framework requires to be `Send`, a savepoint's closure nested in it
> included. Naming the receiver's lifetime instead (`F: AsyncFnOnce(&mut
> DatabaseTransaction<'s>)` with `&'s mut self`) compiles at a direct `.await`
> but fails a spawned task's `Send` check, which then needs the closure general
> over every lifetime: "implementation of `AsyncFnOnce` is not general
> enough".
>
> The future `TransactionTrait::transaction` returns promises no `Send`.
> Promising it would mean bounding an async closure's future, which stable
> Rust cannot state: the bound needs the unstable `async_fn_traits`, and a
> helper trait with a `Send` associated future is not general enough for an
> async closure. Without the promise the compiler sees through the future a
> call on a concrete type returns, so that call is `Send` whenever its
> closure's future is; code generic over `C: TransactionTrait` gets a future it
> does not know to be `Send`.
>
> The handle the closure receives is a `ConnectionTrait` implementor in its own
> right (`conn.pool.conn-trait`), so `&txn` is what a helper generic over `&C`
> is passed, inside a closure exactly as outside one; no `&*txn` reborrow is
> required at the call site.
>
> `E` stays the caller's. Where a closure body's tail is a bare `Ok(value)` and
> every fallible call reaches `?`, nothing pins `E` and the caller annotates —
> `Ok::<_, Error>(value)`. There is deliberately no fixed-`Error` sibling of
> these methods to spare that annotation: it would be a second name per entry
> point differing only in a type parameter, and it would have to either return
> the same `TransactionError<Error>` — an inference alias, nothing more — or
> flatten to `Error` and erase the distinction `TransactionError` exists to
> draw.
>
> Failures are wrapped in `TransactionError<E>`, whose variants keep the kinds
> of failure apart: `Connection(Error)` for a failing `BEGIN` or `COMMIT` —
> the transaction machinery, an aborted transaction's `COMMIT` among them — `Transaction(E)` for the closure's own error,
> rolled back, and `RollbackFailed { cause: E, rollback: Error }` for the
> closure's error followed by a `ROLLBACK` that failed too. It is `Debug`
> unconditionally, `Display` where `E: Display`, and `std::error::Error` with a
> `source()` where `E: Error + 'static`. There is deliberately no
> `From<Error> for TransactionError<E>`: with `E = Error`, the common case, it
> would file every closure-side error under `Connection` and erase the
> distinction the enum exists to draw. It is `#[non_exhaustive]`, since kinds
> of failure grow (`sql.surface`), and a caller's `match` ends in an arm for a
> kind it does not know, which it treats as it treats `RollbackFailed` below:
> neither how the transaction ended nor the state of its connection is known.
>
> When the closure returned an error AND the `ROLLBACK` then fails, the caller
> MUST be told both. The closure's error stays the cause — `RollbackFailed`'s
> `cause` and its `source()`, so the answer to "why did this transaction
> fail?" is not replaced with a symptom — and the rollback's failure travels
> beside it rather than only to a log, because after it neither pgorm nor the
> caller knows how the transaction ended or what state its connection is in.
> A caller that would hand the connection on, a pool's or a binding's, needs
> that to discard it (`DatabaseConnection::discard`) rather than lend it
> again.

> [spec:pgorm:sem:conn.tx.compose]
> A savepoint's failure MUST reach the closure it is nested in through `?`,
> as a statement's does. The savepoint's `transaction` returns a
> `TransactionError<E>` while the enclosing closure returns its own error, so
> pgorm implements `From<TransactionError<E>> for Error` for every
> `E: Into<Error>`: `Connection(error)` becomes `error`, `Transaction(error)`
> becomes `error.into()`, and `RollbackFailed { cause, rollback }` becomes
> `Error::RollbackFailed { cause, rollback }`, both boxed (`error.model`).
> A closure whose error is a type of the caller's own gets `?` by implementing
> the same conversion into it.
>
> The conversion MUST NOT drop a failed rollback for its cause. A savepoint
> whose `ROLLBACK TO SAVEPOINT` failed has left the enclosing transaction in
> a state nobody knows, and the enclosing closure, its caller, and a
> `transaction_with_retry` deciding whether to replay all have to see that
> it did; reporting only the cause would describe a savepoint that rolled
> back cleanly. `Error::RollbackFailed` keeps the cause as its `source()`, as
> `TransactionError::RollbackFailed` does, and is not retryable
> (`conn.tx.retry`) whatever its cause: a failed rollback is not a refusal
> the server asks the client to replay, and a replay that succeeded would
> bury it. Flattening `Connection` and `Transaction` loses nothing the
> enclosing transaction acts on: either way the savepoint is over and its
> error is the enclosing closure's to return or handle.
>
> The same conversion applies to a transaction opened on a connection, in a
> function whose error is `Error`. Flattening there is the caller's choice,
> made by writing `?`; the `TransactionError` it flattens is still what
> `transaction` returns. `Error::RollbackFailed` then means what
> `TransactionError::RollbackFailed` means: the connection's state is
> unknown, and it is to be discarded.

> [spec:pgorm:sem:conn.tx.aborted]
> A transaction whose closure returned `Ok` MUST NOT be reported as committed
> when PostgreSQL rolled it back. A statement that fails inside a transaction
> aborts it, and PostgreSQL refuses every later statement with SQLSTATE `25P02`
> (`in_failed_sql_transaction`) until the transaction ends. A closure that
> catches the failed statement's error and returns `Ok` all the same leads to a
> `COMMIT`, which PostgreSQL answers with the `ROLLBACK` command tag and no
> error, and tokio-postgres reads no command tag. So `pgorm_pool`'s
> `Transaction::commit` asks first. It sends `SELECT; COMMIT` as one simple
> query: the empty `SELECT` is refused in an aborted transaction, and its
> refusal skips the `COMMIT` sent with it. A failure that came before the
> `SELECT` completed is the transaction's abort: the transaction is rolled
> back, the `ROLLBACK` awaited, and the commit fails with
> `CommitError::Aborted { refusal, rollback }`. A failure after it is the
> `COMMIT`'s own, which ends the transaction: `CommitError::Failed`. Either way
> a transaction the server has ended is given up without the `ROLLBACK` its
> drop would queue. A savepoint is asked a round trip ahead of its `RELEASE`,
> with a `SELECT` alone: an aborted savepoint refuses the `RELEASE` too, but
> the `RELEASE` consumes the handle a `ROLLBACK TO` would be sent through. On
> the refusal the savepoint is rolled back to, so the transaction it is nested
> in carries on. A transaction's `COMMIT` therefore costs no round trip more
> than before, a savepoint's `RELEASE` one, and the empty `SELECT` appears in
> the server's statement log and statistics beside them.
>
> pgorm reports an aborted transaction as
> `TransactionError::Connection(Error::TransactionAborted(refusal))`: under
> `Connection` because its `COMMIT` is what failed, holding the server's
> refusal as its `source()`. Its connection is in a known state, idle or back
> inside the enclosing transaction, so it need not be discarded. It is not
> retryable (`conn.tx.retry`): the closure caught the statement that aborted
> it and chose not to return the error, and a replay would run the same
> closure again. When the rollback fails as well, the commit's error is
> `Error::RollbackFailed { cause: TransactionAborted, rollback }`, with
> `RollbackFailed`'s meaning: the connection's state is unknown. Through `?`
> a savepoint's `TransactionAborted` reaches the closure around it as itself,
> as `conn.tx.compose` flattens `Connection`.

> [spec:pgorm:sem:conn.tx.retry+3]
> `DatabaseConnection::transaction_with_retry(opts, max_retries, f)` is
> `transaction_with` plus replay: it retries the whole begin/run/commit cycle —
> not the failing statement — while the failure is retryable, for at most `1 +
> max_retries` attempts. Each attempt reborrows the connection for a fresh
> transaction opened as `transaction_with` opens one, under the same
> higher-ranked bound as `conn.tx.closure`
> but callable once per attempt: `F: AsyncFnMut(&mut DatabaseTransaction<'_>)
> -> Result<T, E>`. A failed attempt is rolled back exactly as `conn.tx.closure`
> specifies before the next one begins, and an attempt whose `ROLLBACK` fails
> ends the retrying with `RollbackFailed`: a connection in an unknown state is
> not one to replay a transaction on.
>
> Retryable means SQLSTATE `40001` (`serialization_failure`) or `40P01`
> (`deadlock_detected`) — the transaction-rollback errors PostgreSQL raises
> expecting the client to replay — as decided by `Error::is_retryable()`, which
> is `false` for every non-`Error::Postgres` variant and for any `Postgres`
> error carrying no `DbError`. Anything else returns immediately, on the first
> attempt, with its variant intact.
>
> Both failure sites are classified, because a serialization failure can surface
> either mid-transaction (a statement raises it, reaching the helper as the
> closure's `E`) or at `COMMIT` (reaching it as an `Error`). Classifying the
> closure's error requires seeing inside an otherwise opaque `E`, so
> `transaction_with_retry` bounds `E: RetryableError` — a single-method trait
> (`is_retryable(&self) -> bool`) implemented for `Error` and implementable for
> a domain error type. `transaction` and `transaction_with` carry no such bound.
>
> `F` is `AsyncFnMut` rather than `AsyncFnOnce` because it is called once per
> attempt, which makes replayability the caller's obligation: work the closure
> does inside the transaction is undone by the rollback, but any effect outside
> it happens again on every attempt.

## COPY

> [spec:pgorm:req:conn.copy+2]
> `CopyTrait` is COPY's data flow on a connection: `copy_in(&mut self,
> &CopyFromStatement<F>)` starts `COPY … FROM STDIN` and returns a
> `CopyInSink<'_>`, and `copy_out(&mut self, &CopyToStatement<F>)` starts `COPY
> … TO STDOUT` and returns a `CopyOutStream<'_>`, over the typed statements of
> `sql.ast.copy`. It MUST be implemented for `DatabaseConnection` and
> `DatabaseTransaction`, savepoints included, and for the metrics wrappers
> (`metric.layer.copy`). A COPY's rows travel as `CopyData` messages beside the
> statement, which no `ConnectionTrait` method sends or returns (`sql.scope`).
>
> The methods take only the typed statements. Text that is not a COPY would
> reach the driver's `copy_in` as a statement that runs to completion before
> the driver notices that no data flow opened — a `DELETE` passed to it would
> delete and then fail — so text is not accepted at all.
>
> Both MUST borrow the connection or transaction exclusively for as long as
> the sink or stream they return is alive. A COPY in progress holds the
> connection's request queue: a statement sent while a COPY FROM is open waits
> behind data the caller has not yet sent, and one sent while a COPY TO is
> unread waits behind data the caller has not yet read, so within one task
> either never completes; a transaction committed while its COPY is open waits
> the same way. The exclusive borrow makes each a compile error, and is why
> `CopyTrait` is a trait of its own rather than two more `ConnectionTrait`
> methods, which borrow shared and which `&DatabaseConnection` implements.
>
> `copy_out` MUST run `CopyToStatement::check_source` before anything is
> sent, so a `SELECT` source whose tie limit or WINDOW clause the grammar
> refuses (`sql.ast.copy`) is the `Error::QueryBuilder` naming it, as on every
> path that sends a `SELECT` (`exec.tie-limit`), and the connection is not
> touched.
>
> The statement is sent as its one rendering (`sql.render.copy`) and is
> prepared afresh, never through the statement cache
> (`conn.pool.statement-cache`): the text of a COPY over a query carries that
> query's values, so caching it would fill the bounded cache with statements
> run once, and one prepare per COPY is nothing beside the transfer it starts.
>
> `CopyInSink` implements `Sink<Bytes>` with pgorm's `Error`, and
> `finish(self)` returns the number of rows the COPY inserted; `Bytes` is
> re-exported from `pgorm`. A chunk is the format's own encoding, split
> anywhere. The sink MUST send a chunk longer than a mebibyte as several
> `CopyData` messages: the driver sends every chunk past four kilobytes as a
> single message, and PostgreSQL ends the session on a message past 1 GiB. The
> sink applies backpressure, so a source written through it streams rather
> than accumulates. A row the server refuses — a unique violation (`23505`), a
> field that does not parse — is the `Error::Postgres` that `finish` returns,
> after the rest of the data has been sent, since the protocol carries no error
> back to a client that is still sending; inside a transaction the failure
> aborts the transaction, as any failed statement does. Dropping the sink
> unfinished MUST abort the COPY: the driver sends `CopyFail` and no row is
> written; outside a transaction the connection is left as it was, and inside
> one the transaction is aborted. `Sink::close` finishes as `finish` does but
> discards the count. Cancelling `finish` once the data is sent leaves the
> outcome unknown: outside a transaction the rows may have been committed.
>
> `CopyOutStream` implements `Stream<Item = Result<Bytes, Error>>`. Each item
> is one `CopyData` message, which the server sends per row in the text
> formats without the protocol promising that boundary. Rows arrive as the
> stream is polled, so an unread stream holds the server back. Dropping it
> early does not stop the server: the connection reads and discards the rest
> before its next statement runs.
>
> A refusal at the start is the error `copy_in` or `copy_out` returns: an
> undefined table or column, a column list naming a generated column, an
> option the server refuses (`sql.ast.copy`), a transaction already aborted. A
> COPY FROM refused there MUST close its
> connection. tokio-postgres sends `Bind`, `Execute` and `Sync` together and a
> second `Sync` when the sink ends. The server ignores the first while it is
> in copy-in mode, so a COPY that starts is answered by one `ReadyForQuery`;
> a COPY refused before copy-in mode answers both, and the driver hands the
> second answer to the next request, which reads it as its own and leaves its
> own to the request after it, so that every later response on the connection
> answers the wrong request. Nothing in the driver's public API sends a
> message the server leaves unanswered, so the extra answer cannot be
> absorbed. pgorm-pool aborts the connection's driver task instead, through an
> abort handle `ClientWrapper` and every transaction opened from it carry:
> every later call on the connection fails as closed, the pool's recycling
> discards it (`conn.pool.recycle`), and a transaction open on it ends without
> committing. A `copy_in` dropped before it completes closes the connection
> too, since whether the server refused is then unknown. The statement is
> prepared before the start, as a request of its own, so one the server
> cannot parse is refused without closing anything. A COPY TO is one request
> answered by one `ReadyForQuery` whether or not it starts, and a refused one
> leaves the connection as it was.
>
> A generated column, stored or virtual, takes no data, and PostgreSQL refuses
> a column list naming one with `42P10` (*column is a generated column*) when
> the COPY starts, not when its text is prepared: a COPY FROM whose list names
> one therefore closes its connection, as every refusal there does. pgorm does
> not look first. Knowing which columns are generated takes a catalogue read
> (`pg_attribute.attgenerated`), which every COPY would pay for a mistake the
> caller can avoid by listing only the columns it sends. A COPY with no column
> list leaves the generated columns out in both directions, the server
> computing them as the rows go in; a COPY TO reads one only through a
> `SELECT` source naming it, since `CopyToStatement` takes no column list.

> [spec:pgorm:req:conn.copy.statement]
> A `COPY` MUST run only through `CopyTrait` (`conn.copy`). The statement
> methods of `ConnectionTrait` — `execute`, `execute_raw`, `query_one`,
> `query_opt`, `query_all`, `query_raw` and `batch_execute`, on a
> connection, a transaction and the metrics wrappers — run what they are
> given as a statement, and a `COPY`'s data flows beside it: the driver
> meets its `CopyOutResponse` or `CopyInResponse` as a protocol failure
> after the server has started it, and a `COPY … FROM STDIN` is left waiting
> for data nothing sends. Each MUST refuse SQL text holding a statement whose
> first token is the `COPY` keyword with `Error::CopyStatement` before
> anything is sent, the statement cache untouched. The text is first asked
> whether it spells `copy` at all, in any case, which a statement pgorm
> composes never does unless a name or a value it holds does, and only one
> that does is scanned by libpg_query's lexer (`pg_query::scan_tokens`), so a
> comment before the keyword is skipped and a `copy` in a string, a quoted
> identifier or a comment is no statement. `batch_execute`'s text is a
> simple query of any number of statements, and a `COPY` anywhere among them
> refuses the whole text, none of it sent. Text the lexer refuses is sent,
> for the server to refuse. The bindings report `Error::CopyStatement` as a
> `ConstructionError` naming their own `copy_in` and `copy_out`
> (`bindings.copy`).

> [spec:pgorm:req:conn.copy.rows+5]
> Binary COPY carries rows of pgorm `Value`s, each field in its value's
> binary form, the form a parameter is bound in and a result column decoded
> from. The format sends no column types — a header, then each row as its
> field count and each field as its length, `-1` for NULL, and its type's
> binary send form, then a trailer — so a row's columns are learned from the
> server, which knows them exactly. `CopyTrait::copy_in_columns` and
> `copy_out_columns` MUST describe a statement's columns, in order, each a
> `CopyColumn` holding its name and its `Type`, by preparing a statement and
> running nothing: for a COPY FROM, a `SELECT` of its column list from its
> table, or with no list `SELECT *`, less the columns the catalog's
> `attgenerated` marks generated, which COPY leaves out in both directions;
> for a COPY TO, its table read the same way, or its source statement, a
> write included, which is prepared and not run. A COPY TO's source is checked
> first, as `copy_out` checks it. The server describes a domain column by the
> type the domain is over, whose form its values travel in, and checks the
> domain's constraint as it reads them. The driver describes a multirange
> type a schema created as a simple type, with no subtype to read or write
> its binary form by (`sql.value.created-range`), so each description
> completes it from the catalogue: `pg_range` names the subtype of the range
> the multirange was created beside (`rngmultitypid`), the driver resolves
> that subtype, a domain included, by preparing a statement whose parameter
> is declared with it, and the column's `Type` is the multirange's name, OID
> and schema over a `Kind::Multirange` of that subtype. An array's member
> and a domain's base are completed the same way; any other type is the
> driver's, and a simple type `pg_range` holds no multirange for costs the
> description one catalogue read. A `money` travels as an `int8` count of its
> currency's minor units, whose places only the session's `lc_monetary`
> knows, so a description with a column holding `money` — the type, an array
> of it, a domain over either — asks the server for them
> (`scale(CAST(CAST(0 AS money) AS numeric))`) and each such `CopyColumn`
> carries them (`money_places`); `CopyColumn::with_money_places` gives them
> to a column known without asking. A table or column the server refuses
> is refused by the description, before any COPY starts, and leaves the
> connection as it was. A generated column in a COPY FROM's list is not: a
> `SELECT` reads one, so the description names it like any other column, and
> the COPY's start is what refuses it, closing the connection (`conn.copy`).
> The statements' parts are read through pgorm-query's
> getters (`sql.ast.copy.parts`).
>
> `copy_in_rows(&CopyFromStatement<CopyBinary>)` and
> `copy_out_rows(&CopyToStatement<CopyBinary>)` are provided by `CopyTrait`
> over the descriptions and `copy_in` and `copy_out`, so they take only a
> binary statement, borrow as those do, and a COPY refused at its start
> behaves as `conn.copy` says. `CopyRowSink::send` writes a row, one value for
> each column in order. A row of another width, and a value its column's type
> does not take — the check `ValueHolder` makes of a parameter
> (`exec.cursor.binding-accepts`) — MUST be refused with `Error::Type` naming
> the COPY's columns or the column, with nothing of the row written, and the
> sink goes on taking rows. That check takes a range or multirange type a
> schema created from the `String` of its text form
> (`exec.cursor.binding-range`), so the rows a COPY TO reads from such a
> column write back into it. A `money` column, an array of one and a domain
> over either take what a `numeric` parameter takes — a `Decimal`, an
> integer as whole units — its amount multiplied out to the currency's minor
> units and rounded half away from zero, as PostgreSQL's cast of a `numeric`
> to `money` rounds, an amount past `int8`'s range refused as a value its
> column cannot take. `send_model` writes a model's value for each of
> the COPY's columns by name, refusing a
> column its entity lacks. `finish` writes the trailer and returns the row
> count; a row the server refuses is reported there, and a sink dropped
> unfinished aborts the COPY, as for bytes.
>
> `CopyRowStream` is a `Stream` of rows, each a `Vec<Value>` whose fields are
> decoded by their column's type through the driver's codec for it, the one
> pgorm's `TryGetable` uses, into the variant that type reads as: `"char"` as
> `TinyInt`, `oid` as `Unsigned`, the text types and an enum's label as
> `String`, a built-in range or multirange as `Range` or `Multirange`, an
> array as an `Array` of its element's variant, and every other type pgorm
> binds as its own variant. A range type a schema created reads as the
> `String` of its text form, the `Value` the newtype naming it converts into
> (`sql.value.created-range`): the driver reports it as a range over its
> subtype, so the range is read through the `FromSql` of the `RangeSubtype`
> that subtype reads as — `int2`, `int4`, `int8`, `float4`, `float8`,
> `numeric`, `text`, `varchar`, `bpchar`, `date`, `time`, `timestamp`,
> `timestamptz`, and `uuid` under `with-uuid`, a domain over one read as the
> one — and written as that type's `Display` writes it, so a model's newtype
> field reads through `models`, and an array of one reads as an `Array` of
> those `String`s. A `numeric` field reads as `ExactDecimal` reads one, a
> `Decimal` holding it exactly, its scale included, and an array as an
> `Array` of those: rust_decimal's own decoder rounds a value with more than
> 28 fractional digits, or with a coefficient past 96 bits whose integer
> part a `Decimal` still holds (`7.9228162514264337593543950336`, and the
> trailing zero of `12345678901234567890123456789.0`), and such a field is
> refused, as are `NaN`, `Infinity` and `-Infinity` and an integer part past
> 96 bits, which no `Decimal` holds at all. A `money` field, its column's places known, reads as the
> `Decimal` of its minor units at those places, the amount PostgreSQL's cast
> to `numeric` gives, and an array of them as an `Array` of `Decimal`s, so
> what a COPY TO reads writes back through a COPY FROM and a model's
> `Decimal` field over a `money` column reads through `models`. A multirange
> type a schema created, its kind completed by the description, reads the
> same way, as the `String` of its text form
> (`{[1,3),[5,8)}`), each range read through that `FromSql` and the
> multirange written as `Multirange<T>`'s `Display` writes it, and an array
> of one as an `Array` of those. A type no `Value` holds — a geometric type,
> and a range or multirange type a schema created over any other subtype —
> and a field that does not decode are an `Error::Type` naming the column.
> Data that is not the format — no signature, a header flag the format does
> not define, a row whose field count
> is not the COPY's, data ending before the trailer or going on after it — is
> an `Error::Query`. Either ends the stream as its last item. `models::<E>()`
> reads each row into a model of `E` through its `ActiveModel`, setting each
> column the entity has from the field of its name: a column the entity lacks
> is left unread, and a model column the COPY lacks is `Error::AttrNotSet`.
>
> `CopyRowEncoder` and `CopyRowDecoder` are the format's two sides over a
> list of columns, for a caller that moves the bytes itself, as the bindings
> do with their own values. The encoder writes the header, a row of any
> `ToSql` values checked as `send` checks one, leaving its output as it was
> when it refuses, and the trailer. The decoder takes the data in chunks
> split anywhere and yields each row's fields, or their values, once the
> whole row has arrived. The metrics wrappers delegate the descriptions
> unreported: what they report is the COPY's start (`metric.layer.copy`).

## LISTEN and NOTIFY
## LISTEN and NOTIFY

> [spec:pgorm:req:conn.listen]
> A notification is delivered to the session that ran `LISTEN`, as an
> asynchronous message outside any statement's result, so no
> `ConnectionTrait` method returns one (`sql.scope`), and a pooled connection
> cannot receive one at all: pgorm-pool's task drives it and discards such
> messages (`conn.pool.lifecycle`), and the pool lends it to another caller
> once it is returned. A listener is therefore a connection of its own.
> `connect_listener(config, tls, capacity)` opens one with a
> `tokio_postgres::Config` and a TLS connector bounded as `connect_with`'s is
> (`conn.pool`), and `DatabasePool::listener(capacity)` opens one with the
> pool's configuration and connector through `pgorm_pool::Manager::connect_raw`,
> which connects as the pool does and hands back the connection's I/O half
> undriven, as a `RawConnection`. That connection is not taken from the pool,
> counts against no size, has no statement cache and is never recycled. Each
> returns a pair, a `Listener` and its `Notifications`.
>
> `Listener` is the connection's `LISTEN` set: `listen(channel)`,
> `unlisten(channel)` and `unlisten_all()`, each taking `&self` and sending
> `LISTEN`, `UNLISTEN` or `UNLISTEN *`. Cloning it is cheap and every clone
> changes the same set, so one task can listen while another reads. A channel
> MUST be sent as a quoted identifier, every `"` in it doubled and the whole
> wrapped in `"`, so that a name is exact: its case and punctuation are kept,
> `Jobs` and `jobs` are two channels, and the name is the text `pg_notify`
> takes. Neither statement takes a parameter, so this is the only form the
> name can travel in, and the quoting is total, as a savepoint name's is
> (`conn.pool.savepoint-name`). A name longer than 63 bytes MUST be refused
> with `ListenError::ChannelTooLong` before anything is sent: PostgreSQL
> truncates a longer identifier with only a notice and would listen on the
> truncated name, which nothing can send to, since `pg_notify` refuses a name
> that long (`22023`). An empty name is the server's refusal (`42601`).
>
> `Listener` MUST NOT implement `ConnectionTrait`. Notifications arrive on its
> connection whenever the server sends them; a statement there would wait
> behind them, and a transaction there would hold back their delivery until it
> ended. The metrics layer does not instrument it (`metric.layer`): it is none
> of the wrapped types, and what it sends is not a statement a caller wrote.
>
> `Notifications` implements `Stream<Item = Result<Notification, Error>>`,
> `Notification` being tokio-postgres's — the channel, the payload and the
> sending session's process id — re-exported from `pgorm`. A task spawned when
> the listener connects drives the connection and queues each notification as
> it arrives, so `listen` and `unlisten` complete whether or not the stream is
> read. The stream yields them in the order the server sent them, which is
> PostgreSQL's: across transactions in commit order, within one in sending
> order, with a notification that repeats an earlier one's channel and payload
> in the same transaction folded into it, and a rolled-back transaction's never
> sent. Once `listen` returns, a notification on the channel from a
> transaction that commits afterwards is delivered; once `unlisten` returns,
> none is, though one already received is still yielded. The stream holds the
> connection open while it lives, and dropping it closes the connection, after
> which every `Listener` method fails.
>
> The stream MUST end only after an error, and MUST NOT reconnect. A
> notification sent while no session listens reaches none, so a reconnection
> would hide a gap the caller has to know about, to re-read whatever the
> notifications were tracking. When the connection fails or is terminated —
> `pg_terminate_backend` ends it with SQLSTATE `57P01` — the stream yields every
> notification already received, then the `Error::Postgres` it failed with, and
> ends.
>
> The queue MUST be bounded by `capacity`, a `NonZeroUsize`
> (`Listener::DEFAULT_CAPACITY` is 1024). When `capacity` notifications are
> waiting unread and another arrives, the listener MUST close its connection,
> and the stream yields those waiting, then `ListenError::Overflow { capacity
> }`, and ends. Waiting for the reader instead would mean leaving the socket
> unread, which holds the listener's own `LISTEN` and `UNLISTEN` replies behind
> the unread notifications and stops its session consuming PostgreSQL's
> notification queue. That queue is shared by every database in the cluster,
> and once it fills every `NOTIFY` fails at commit; closing gives up the
> session's place in it at once.
>
> `notify(db, channel, payload)` sends a notification from any
> `ConnectionTrait` implementor as `SELECT pg_notify($1, $2)`, both values
> bound. `NOTIFY channel, 'payload'` takes no parameter, so its payload would
> have to be escaped into the statement, and each payload would be a statement
> text of its own in the bounded statement cache
> (`conn.pool.statement-cache.bound`); through `pg_notify` there is one text,
> prepared once per connection. Being a `ConnectionTrait` call, it runs inside
> a transaction when given one, delivering at commit, and through an
> instrumented connection or transaction it is reported as the `execute` it is
> (`metric.layer.delegate`). The bounds are the server's: a payload of 8000
> bytes or more, an empty channel and one longer than 63 bytes are each refused
> with SQLSTATE `22023`.
