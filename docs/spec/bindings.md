# Language bindings

pgorm has two language bindings. pgorm-python is a Python package over a
native PyO3 extension. pgorm-napi is a JavaScript module over a native
Node-API addon, which Node.js and Deno both load. This chapter states what
both bindings offer. It is written once, and both bindings' code and tests
answer to it.

[python.md](python.md) and [napi.md](napi.md) keep only what differs by
language: how names are spelled, how asynchronous work is awaited and closed,
which native types carry values, typing, packaging and runtimes. Where the two
bindings made different choices that the operator has not yet settled, each
binding's current behaviour stays in its own chapter until it is.

A shared rule writes a name once, in a neutral spelling: snake_case for
functions, methods and options, PascalCase for classes. Each binding spells
the name by its own naming rule (`python.naming`, `napi.naming`), which also
lists the names that differ beyond that rule. A closed choice, such as a join
kind, is named in plain words; each binding spells its members by its naming
rule. A Python keyword argument and a JavaScript options object are the same
option.

## Package boundary

> [spec:pgorm:def:bindings.api]
> A language binding is a package for one language, backed by a native
> extension over pgorm's Rust query construction, execution and decoding APIs.
> It offers runtime builders for statements, expressions, schema and
> pipelines; models an application declares over its own tables; and
> registrations, through which a Rust application exposes its compiled
> entities and graph shapes. A program composes operations and values at run
> time, within what the installed build offers.

> [spec:pgorm:req:bindings.optional]
> Each binding MUST live in an opt-in companion crate. Ordinary pgorm builds,
> default workspace builds and Rust library users MUST NOT need the binding's
> language, its runtimes, its native-extension framework or its linker
> configuration. Building a binding MUST be explicit. Its public artifacts
> MUST exclude test fixtures, deliberately vulnerable controls and
> campaign-only dependencies.
>
> Each companion crate is its own Cargo workspace, which keeps its
> configuration out of ordinary Rust builds. Cargo reads a `patch` table only
> from a workspace's root, so a revision pgorm pins MUST reach the companion
> crate through pgorm's own dependency declarations, which travel with pgorm
> into every workspace that depends on it, and never through a `patch` table.
> The crate's committed lockfile MUST resolve each pinned dependency to the
> pinned revision. A divergence is silent: the binding would link the unpinned
> crate and behave differently from the library every pgorm test exercises,
> and for a compiler dependency it would write SQL the Rust API does not. So a
> divergence MUST fail a check in this repository.

> [spec:pgorm:req:bindings.delegation]
> Each operation a binding exposes MUST invoke its corresponding Rust API in
> the language's process, through the native extension. Building and running
> a query MUST NOT need an HTTP adapter, an RPC service or a subprocess that
> dispatches queries. A program composes builder operations directly; choosing
> a fixed query handler and handing it inputs does not meet this rule.
>
> A binding MUST NOT implement a second SQL renderer, escaping algorithm,
> placeholder substitution or query optimizer. Every convenience lowers into a
> documented Rust operation. A dynamic statement path MUST NOT be presented as
> an `EntityTrait`, ActiveModel or typed graph path. A query composed at run
> time MUST NOT invoke rustc, Cargo, a packaging tool or a code-generation
> build.

> [spec:pgorm:req:bindings.postgres-target]
> Each binding MUST carry pgorm's `pg-19` cargo feature, forwarding it to
> pgorm (`pg-19 = ["pgorm/pg-19"]`) and to nothing else, so that a build of
> the binding targets the PostgreSQL release a build of pgorm does
> (`sql.target`): 18 by default, 19 with the feature. Each binding's module
> MUST export `POSTGRES_TARGET`, that release as an integer, read from
> `pgorm_query::POSTGRES_TARGET` and never computed again. Where the two
> releases answer one statement differently, a binding's live tests hold the
> answer of the release its build targets.

> [spec:pgorm:req:bindings.surface+2]
> The two bindings MUST offer the same public operations, each spelled by its
> binding's naming rule. `docs/spec/bindings-surface.json` declares the shared
> surface once, in neutral spelling: each module's functions, classes and
> constants, and each class's methods. Beside it, the file lists:
>
> - the names one binding offers alone, each with the language reason it has
>   no counterpart;
> - the shared names one binding still lacks, each with the plan node that
>   adds it;
> - the differences that wait on the operator's decision.
>
> Each binding's own test suite MUST read its live module — its exports, its
> classes and their methods — and fail when the module offers a name the file
> does not list for that binding, when it lacks a shared name that is not
> listed as a gap, or when a name listed as a gap is present.
>
> The file also declares, under `descriptions`, the keys of the data a
> model's, a registered entity's, a registered graph's and a source tuple's
> `describe()` report (`bindings.models`, `bindings.entities`,
> `bindings.entity-graphs`, `bindings.pipeline-sources`), nested objects and
> the items of lists included, in the same neutral spelling. Each binding's
> surface test MUST compare a live `describe()` of each with that
> declaration, recursively, and fail on a key missing, a key the declaration
> does not list, or a declared key no live description reached; a module
> that registers no entity, graph or source tuple is held to it by the
> application module its checks build. A class's other data fields are
> outside this check; the rules that state them are tested by each binding's
> own suites.

## Runtime and errors

> [spec:pgorm:req:bindings.runtime]
> Each binding MUST run pgorm's asynchronous work on one multi-threaded Tokio
> runtime per process, built on first use and reused by every pool and every
> operation after it. A runtime, a pool or a native build MUST NOT be created
> per call or per query. The language's own thread MUST NOT block waiting on
> the runtime: an operation that does I/O is awaited in the language's own
> asynchronous form (`python.runtime`, `napi.promises`).

> [spec:pgorm:req:bindings.errors+6]
> Each binding MUST report a failure as an instance of its own error classes,
> all derived from one base class of the binding and from the language's own
> base error:
>
> - `DatabaseError` when PostgreSQL reports an error, carrying its SQLSTATE as
>   `sqlstate`, its severity and message, and the diagnostic fields `detail`,
>   `hint`, `schema`, `table`, `column`, `datatype` and `constraint`, each text
>   or null;
> - for a constraint violation pgorm's `SqlError` classifies
>   (`error.model.sql-class`), the subclass of `DatabaseError` named for it:
>   `UniqueViolationError` (`23505`), `ForeignKeyViolationError` (`23503`),
>   `CheckViolationError` (`23514`), `NotNullViolationError` (`23502`) or
>   `ExclusionViolationError` (`23P01`), with the same fields, so the names the
>   server reported (a domain's in `datatype`) are read as from any other
>   `DatabaseError`;
> - `ConnectionError` when the server cannot be reached or the connection
>   breaks;
> - `ConstructionError` for an argument whose value cannot become what pgorm
>   sends; one its parameter's type refuses as the driver encodes it is
>   reported with pgorm's `BindRefusal` text (`error.model.bind-refusal`),
>   `parameter $1 cannot be bound:` followed by pgorm's reason, the same text
>   in each binding, and a statement given another number of values than its
>   placeholders take with pgorm's `ParameterCountRefusal` text
>   (`error.model.parameter-count`), the same text in each binding;
> - `DecodeError` for a result that cannot become the native value asked for,
>   a wrong row count among them (`bindings.row-count`); one the driver cannot
>   read is reported with pgorm's `DecodeRefusal` text
>   (`error.model.decode-refusal`), what it was reading followed by the
>   decoder's reason, the same text in each binding;
> - `LifecycleError` for a pool, connection, transaction, stream or listener
>   used after it closed, or while another operation holds it;
> - `TimeoutError` when no connection becomes free within a pool's acquire
>   timeout;
> - `InternalError` when pgorm or the binding fails in a way no input should
>   cause, a panic in its work on the runtime above all.
>
> An argument a call cannot use is refused before anything is sent, with the
> class `bindings.argument-errors` names. A cancelled operation is reported in
> the language's own way (`python.cancellation`, `napi.cancellation`). An error
> from PostgreSQL anywhere in a failure's cause chain makes the failure a
> `DatabaseError`, so a refusal during connection carries its SQLSTATE too.
> Credentials MUST NOT appear in an error, a nested cause, a representation
> or a default diagnostic log. Invalid input MUST NOT end the process, be
> swallowed, or become an ordinary result.
> A panic the binding catches, every one in work on the runtime, MUST stay an
> identifiable `InternalError`. One the language's own boundary catches
> before the binding can, PyO3's in a synchronous Python call
> (`python.runtime`), keeps that boundary's exception. Either way it MUST NOT leave
> an operation pending, and MUST NOT be classified as an expected rejection of
> input.

> [spec:pgorm:req:bindings.argument-errors+1]
> An argument of a type the call does not take MUST be refused with the
> language's `TypeError`, before anything is sent; a `ConstructionError`
> (`bindings.errors`) is left to an argument whose value cannot become what
> pgorm sends. Where a rule refuses an argument's other values with a
> `ConstructionError`, an argument of another type is a `TypeError` all the
> same. A `TypeError` is:
>
> - text that is not the language's string — an identifier, a type's,
>   function's, field's or qualifier's name, a JSON path, a LIKE pattern or
>   escape, a channel, a payload, a connection string, a COPY option — a flag
>   that is not a boolean, a count, size, offset or sampling argument that is
>   not a number (Python's `int` where a whole one is taken), a list that is
>   not the language's list, a pair of the wrong length, and a value kind that
>   is no kind name, `TypeName`, `CreatedRange` or `CreatedMultirange`;
> - a value outside a closed choice, whose type is the choice: a class whose
>   members are the choice, or a string that is one of its spellings, the
>   union of their literals (`Literal[..]` in Python's typing) — a join kind,
>   a lock strength, a frame's mode, start, end or exclusion, a NULLS
>   placement, an SQL/JSON behaviour, a generated or identity mode, a
>   deferrability, an enforcement, a foreign key's action, a behavior, a
>   constraint's change, a transaction's mode or isolation level, a COPY
>   format or log verbosity, the TLS mode, the recycling method, a sequence's
>   type, an index entry's order, a view's check option and the rest;
> - an option the call does not know — a keyword argument in Python, a key of
>   the options object in JavaScript — so a misspelt option is never left out;
> - a builder, or a plain value, where the call takes a builder of another
>   class: a `Select`; an expression where only an expression goes (a
>   projection, a grouping item, a RETURNING item, a table function's call, a
>   CHECK, a side of AND and OR); an expression or a `Condition` where a
>   predicate goes; an ordering; a `Window` or `ExtendedWindow`, a WINDOW
>   entry and OVER's window among them; a frame; a `Table` or FROM item; a
>   WITH clause; a MERGE action; a conflict action; a `ColumnDef`; a
>   `DataType`; a `TypeName` where a type is named; a model's column
>   declaration; and a receiver of another class, a handle only the binding
>   makes among them;
> - a builder no terminal runs given to one, a COPY given to any terminal but
>   `copy_in` and `copy_out`, and a COPY of another direction or format given
>   to `copy_in`, `copy_out`, `copy_in_rows` or `copy_out_rows`;
> - a statement in a form the binding's raw rule does not take (`python.raw`,
>   `napi.raw`): Python's SQL text outside a `RawSQL`, and JavaScript's
>   parameter list beside a built statement; a COPY source that is text or no
>   iterable, a chunk it yields that is not bytes, rows that are text or
>   bytes, and a row that is no sequence; and a binder function that returns
>   an awaitable or anything but an operand or a list of them.
>
> A value has no wrong type. A parameter, an operand, a `Value`'s data, the
> data the binding's value classes are made from, a model field's value, a
> COPY row's field and a JSON document may each be any value of the
> language, and one that cannot become what pgorm sends is a
> `ConstructionError`: a builder that is no expression given as an operand,
> and `any(..)` and `all(..)` where no operator takes one
> (`bindings.quantifiers`), among them. So is a name
> pgorm has nothing for — a function, a value kind, a built-in type, a
> pipeline cast, a registration, a field or a column — an identifier outside
> 1–63 bytes, a count or size out of range, an integer outside `bigint`, an
> empty list where one is needed, an expression where a table function takes
> a call, a pending statement or a `ColumnDef` given to a terminal
> (`bindings.schema`, `bindings.merge`), and a statement PostgreSQL would
> refuse.
>
> A listener's capacity is the one argument a value of the right type can
> take out of range with nothing pgorm sends to refuse it: a capacity of the
> type the language counts in that is not a positive whole number is the
> language's own class for a value out of range, Python's `ValueError` and
> JavaScript's `RangeError`.

## Values

> [spec:pgorm:req:bindings.values]
> Every kind of pgorm `Value` MUST cross into and out of each language as the
> one native type its binding declares for that kind (`python.values`,
> `napi.values`), and SQL NULL as the language's null, both ways. The kinds
> are named alike in both bindings: `bool`, `i8`, `i16`, `i32`, `i64`, `u32`,
> `u64`, `f32`, `f64`, `text`, `char`, `bytes`, `json`, `decimal`, `uuid`,
> `date`, `time`, `datetime`, `datetime_utc`, `interval`, `ipnetwork`,
> `mac_address`, `vector`, PostgreSQL's six built-in range types and their
> multiranges by type name (`int4range` … `tstzmultirange`), an enum by its
> `TypeName`, and a range type a schema created by its `CreatedRange` or
> `CreatedMultirange` (`bindings.created-ranges`).
>
> A value MUST reach the language exactly or not at all. A value with no exact
> native form MUST be a `DecodeError`, never a rounded value, a string, a null
> or a missing column: a `numeric` outside pgorm's range of a 96-bit
> coefficient and 28 fractional digits, `NaN` or infinite; a date or timestamp
> outside the years pgorm holds, or infinite; the time `24:00`; a `timetz`; an
> array of more than one dimension, or one not starting at 1; and a type no
> kind covers. A domain decodes as the type it is built over.
>
> In the other direction a value MUST become the pgorm value it declares, or
> be refused with a `ConstructionError`: an integer outside its kind's range,
> a float its kind cannot hold without change, a decimal outside pgorm's range,
> a value of an unsupported type, an ambiguous value with no declared kind,
> and statement text holding NUL. A value MUST NOT silently become text, wrap,
> truncate or become NULL. A decimal MUST NOT pass through a float.
>
> Parameters are bound, never interpolated, through pgorm's `ValueHolder`,
> which writes each against the type PostgreSQL inferred for its placeholder
> and refuses a value that type cannot receive. Its coercions — an integer to
> any numeric type, a float to `float4` rounding as PostgreSQL's own cast
> does — are pgorm's, and a refusal names the parameter and the codec's
> reason.
>
> An interval is the binding's `Interval`: PostgreSQL's three independently
> signed fields, months, days and microseconds, each an exact integer within
> its width (`sql.value.interval`). It is immutable, is inferred as the
> `interval` kind and decodes from an `interval` column. A range or
> multirange value is the binding's `Range` or `Multirange`; a side with no
> bound is the language's null, and the empty range is `Range.empty()`. A
> range's kind is never inferred: a range operand or parameter needs the
> `Value` that declares its kind.

> [spec:pgorm:req:bindings.value-tags+1]
> Each binding's `Value` MUST declare what a plain native value cannot: the
> kind of a value, so integer widths, enum types and temporal variants stay
> distinct; SQL NULL of a kind, `Value.null(kind)`; a JSON document,
> `Value.json(data)`, so JSON's `null` stays apart from SQL NULL; and an array
> of a named element kind, `Value.array(kind, items)`, which keeps its kind
> when it is empty or NULL. `Value(data, kind)` takes a kind by its name, a
> `TypeName` for an enum, or a `CreatedRange` or `CreatedMultirange`.
> Construction does no I/O and MUST convert at once, refusing data the kind
> cannot hold exactly with a `ConstructionError`. A `Value` is immutable, can
> be bound any number of times, and the plain value it hands out is owned
> independently. Its type name (`type_name`, `typeName`) MUST be the
> schema-qualified `TypeName` the server names its type by — an enum's, or a
> created range's or multirange's — an array's being its element kind's, and
> none for any other kind; its element kind is the array's alone.
>
> A result read tagged MUST give each column as a `Value` carrying its kind:
> an integer's width, an enum's schema-qualified type, an array's element kind,
> and SQL NULL apart from JSON's null.

> [spec:pgorm:req:bindings.created-ranges+5]
> A created range's kind MUST carry the type's name, quoted and
> schema-qualifiable, and its subtype, one of the twelve pgorm's
> `RangeSubtype` admits, which converts each bound with that kind's limits
> (`sql.value.created-range`). Its value is the text form a
> `CreatedRange` newtype converts into, bound as text for the statement
> to cast, `CAST($1::text AS name)`, as `Expr::as_range` writes it. A result
> column of one MUST decode, through its subtype's binary codec, to a `Range`
> whose kind is named as the column's type is, whatever its subtype, so the
> value writes back to its column. A column a registration or a schema
> describes by its `ColumnType` MUST take the kind pgorm's codec reads that
> column type as (`CreatedRangeCodec::of_column`), the same in both bindings,
> and a kind MUST declare the subtype's column type that codec gives
> (`subtype_column_type`). A `char(n)` subtype is text: PostgreSQL drops a
> range subtype's modifier, so a range type created over `char`, `char(n)`
> or `bpchar` ranges over `bpchar`, whose values are text of any length. A
> domain subtype, which PostgreSQL takes, holding each bound to the domain's
> constraints, is the kind of the type it is over, through any number of
> domains, as a server's subtype type is read (`CreatedRangeCodec::subtype_kind`).
>
> A created multirange's kind is a `CreatedMultirange`, and its value is
> bound the same way. The driver reports a query's result column of one with
> no subtype, so reading that column MUST be a `DecodeError`, leaving it to be
> read cast to text. A binary COPY's column of one is described with its
> subtype, which pgorm completes from the catalogue (`conn.copy.rows`), so
> `copy_out_rows` MUST decode it through the subtype's codec to a
> `Multirange`, and `copy_in_rows` MUST write one back from a `Value` of its
> kind or from its text form.
>
> An array of either is a value kind of its own, both ways and in both
> bindings. `Value.array(kind, items)` of a `CreatedRange` or
> `CreatedMultirange` MUST convert each item as a value of the kind is
> converted — data of the kind, its text form, or a `Value` of that kind and
> no other — and hold the array of their texts, the empty array, a `NULL`
> element and an SQL NULL array included, its element kind kept. Its value
> is that array of texts: bound bare it is read by the subtype's parsing as
> a scalar's text is, and written in an expression it is cast to the type's
> array through `Expr::as_range`, `CAST($1::text[] AS name[])`, the array a
> column of the type's array reads (`sql.value.created-range`). A result
> column of an array of a created range MUST decode element by element
> through the subtype's binary codec to that kind, each element named as the
> column's member type is; a query's array of a created multirange, whose
> member the driver reports with no subtype, MUST be a `DecodeError` as a
> column of one is. A binary COPY's array column is described with its
> member completed, so `copy_out_rows` MUST decode an array of either the
> same way and `copy_in_rows` MUST write one back from a `Value` of its kind
> or from an array of texts. A declared model's column of either kind with
> `array` MUST take and give back an array of the kind's values, written
> through the same cast and an array of multiranges projected cast to
> `text[]`, each text read as the declared kind. A registered entity's column
> of one takes a list of the kind's values, written as the array of their
> texts that its `save_as` casts to the type's array — the array a quantified
> comparison with a column of the type writes — and reads each element back
> as the kind's value; a `Value` of another declared kind is refused with a
> `ConstructionError` there as at any declared column.

> [spec:pgorm:req:bindings.rows]
> A result's rows MUST be decoded on the runtime and reach the language one
> record per row, each keyed by column name in column order and each value
> the one `bindings.values` declares. Two columns of one name MUST be a
> `DecodeError`, not one key for both. A statement that returns no rows
> yields an empty list.

## Connections

A pool holds the connections; a `Connection` is one of them checked out; a
`Transaction` is a transaction or a savepoint on one; and a stream pulls a
statement's rows over one. Each is a native handle that is released by being
closed, and a transaction by the block or callback it was opened for ending.

> [spec:pgorm:req:bindings.connections+3]
> A `Pool` MUST be built from a connection string and options, which are the
> arguments of Rust's `connect_with` (`conn.pool`) a program in the binding's
> language can give: TLS (`bindings.tls`); the `Config`'s connect timeout; the
> `PoolBuilder`'s maximum size, its `wait_timeout`, `create_timeout` and
> `recycle_timeout`, each off unless given, its `queue_mode`, `fifo` unless
> `lifo` is given, and its hooks (`bindings.pool-hooks`); and the
> `ManagerConfig`'s recycling method, its statement-cache size, its
> statement-cache channel and its `tag`. Beside them is the binding's own
> acquire timeout, which bounds the whole of an acquisition. The recycling
> method is `recycle`'s `verified`, the default, `fast` or `clean`, or
> `recycle_sql`, SQL of the program's own, which is `RecyclingMethod::Custom`
> and is a `TypeError` beside `recycle`. A pool's `tag` names it, and the
> pool reports it, the `default-N` pgorm gives an untagged pool among them;
> an empty one is refused. Both bindings read these options into one Rust
> crate, pgorm-bindings, which decides every one of them, refusals included,
> and builds the pool through `connect_with`, so a pool means the same from
> either language. Building one sends nothing, and a string or an option
> value the pool cannot use is a `ConstructionError`; `connect` returns a
> pool whose server answered.
>
> A pool lends a `Connection` with `acquire`, and runs a statement of its own
> on a connection lent to it alone. Acquiring past the acquire timeout MUST
> fail with a `TimeoutError`, and so MUST one the wait or create timeout
> ends, its message naming which. A check of a returned connection that
> outlasts the recycle timeout discards that connection, as deadpool does,
> and the pool lends another or opens one. A connection's `discard()` MUST
> close it as pgorm's `DatabaseConnection::discard` does, never returning it
> to its pool, so the pool opens a fresh one in its place: one whose session
> state the program changed and cannot put back. A connection an operation,
> transaction or stream holds is discarded once that ends, as a close
> discards one. A connection runs one operation at a time: a
> second one, or any while a transaction or stream holds it, MUST be refused
> with a `LifecycleError` rather than queued or raced, as is any use after
> close. `status()` reports the pool's maximum size, its size, the
> connections available and the callers waiting.
>
> `clear_statement_caches()` MUST mark every connection's cached statements
> stale, as `conn.pool.statement-cache.clear` describes. It is synchronous,
> sends nothing, returns nothing, and is refused by no state of the pool. The
> `statement_cache_channel` option, absent by default, names a channel the
> pool listens on, clearing on every notification there as
> `conn.pool.statement-cache.listen` describes; an empty name or one longer
> than 63 bytes is a `ConstructionError`, and closing the pool closes the
> listening connection. `STATEMENT_CACHE_CHANNEL`, which each binding
> exports, is the channel pgorm-migration's runner notifies.
>
> Each pool, connection and stream MUST be closable explicitly, and in the
> language's scoped form (`python.lifecycle`, `napi.lifecycle`). What each
> holds MUST be released by that close, never left to garbage collection:
> closing a connection returns it to its pool at once, and closing a pool
> refuses new work, interrupts what runs and finishes once every connection
> is released. Connections live on the runtime and hold nothing the
> language's event loop waits on.

> [spec:pgorm:req:bindings.tls]
> A pool's TLS MUST be configured by the options `tls`, a mode named as
> libpq's `sslmode` names it; `ca`, PEM certificates a verifying mode trusts
> in place of the platform's store; and `cert` and `key`, a PEM client
> certificate chain and its private key, shown to a server that asks for
> one. The modes are:
>
> - `verify-full`: the server's certificate MUST chain to a trusted
>   certificate and name the host connected to;
> - `verify-ca`: it MUST chain to a certificate `ca` gives, whatever host it
>   names. Without `ca` the mode is refused, since the platform's store
>   trusts certificates issued to anyone;
> - `require`: TLS always, with no certificate verified — only the
>   handshake's signatures, which prove the server holds the key of the
>   certificate it sent;
> - `prefer`: TLS when the server offers it, plaintext when it does not,
>   nothing verified;
> - `disable`: plaintext.
>
> With no `tls` option the connection string's `sslmode` decides: `disable`
> is plaintext, and `prefer`, `require` or none is `verify-full`. So a mode
> that verifies less than `verify-full` is the program's explicit option and
> never a default a string carried in. libpq's `allow`, plaintext first and
> TLS only if the server refuses it, is not offered: tokio-postgres
> negotiates TLS before anything else, so there is nothing for it to fall
> back from, and a server's impostor would always be given the plaintext.
> What the pool would not do as asked MUST be a `ConstructionError` raised
> before anything is sent: `disable` beside `ca`, `cert`, `key` or the
> string's `sslmode=require`; `prefer` beside `sslmode=require`; `ca` with a
> mode that verifies nothing; `cert` without `key`, or `key` without `cert`;
> PEM that holds no certificate, or no private key rustls can use; a
> platform store with no usable root. A verifying mode MUST NOT fall back to
> plaintext. With TLS, `channel_binding=require` in the string binds SCRAM
> authentication to the server's certificate. A listener opened with
> `connect_listener` (`bindings.listen`) takes the same options.

> [spec:pgorm:req:bindings.pool-hooks]
> A pool's `post_create`, `pre_recycle` and `post_recycle` options MUST each
> take a function of the language's, which pgorm runs as the `PoolBuilder`
> hook of its name: on each connection as it opens, before the pool lends
> it; on a returned connection before the pool checks it for reuse; and
> after. The function runs on the language's thread, on the event loop the
> pool belongs to, never on pgorm's runtime, and is given a
> `HookConnection`: the connection the hook is about, whose `execute`,
> `query`, `one` and `optional` run as a connection's do, through pgorm's
> `HookConnection` (`conn.pool.hook-connection`), and which reports the
> connection's `age` and `recycle_count`. What it returns MAY be awaited, as
> a coroutine or a promise is, and the hook waits for it. A `HookConnection`
> used once its hook has returned MUST be refused with a `LifecycleError`.
>
> A function that raises, throws or rejects refuses the connection, as a
> Rust hook's `HookError` does. A `post_create` refusal MUST fail the
> acquisition with a `ConnectionError` naming the hook and carrying the
> error's class and message; a `pre_recycle` or `post_recycle` refusal
> discards the connection, the pool lending another or opening one. A
> function that never finishes holds its acquisition until a timeout ends
> it, and a hook function that is no function is a `TypeError`.

> [spec:pgorm:req:bindings.multi-pool]
> `MultiPool(dsn, pools, options)` MUST build one pool per entry of `pools`
> through pgorm's `connect_multi_with` (`conn.pool.multi`): each tagged with
> its entry's key and shaped by its entry's options — the maximum size, the
> acquire, wait, create and recycle timeouts, the queue mode and the hooks,
> which Rust's per-pool `PoolBuilder` closure sets — every pool sharing the
> connection string, TLS and the `ManagerConfig`'s options, which `options`
> gives. The shared options take no `tag`, so one is refused as any option
> the call does not take is (`bindings.argument-errors`); an empty key and a
> `pools` of no entry MUST be a `ConstructionError` before anything is built. Each pool is a `Pool`, as one
> built alone is. `get(tag)` gives the pool with a tag, or null; `items()`
> every pool with its tag, in the tags' order; `status()` each pool's status
> by tag; and `close()` closes every pool. Building one sends nothing, and
> its scoped form checks a connection of each. Which pool a statement runs
> on is the program's choice: nothing balances or falls back across them.

> [spec:pgorm:req:bindings.results+2]
> A pool, a connection and a transaction MUST run a statement through pgorm's
> cached `ConnectionTrait` path with the same four terminals. `execute` gives
> the affected-row count as an exact integer; `query` gives every row; `one`
> gives exactly one row; and `optional` gives at most one, or null. A
> statement is a built statement (`bindings.statements`), a pipeline
> (`bindings.pipeline`), or raw SQL with its values (`python.raw`,
> `napi.raw`). Any other row count is a `DecodeError` (`bindings.row-count`).
> A database or decode error MUST NOT become an empty result, a null or a
> missing row. Decoding uses the documented Rust decode path.

> [spec:pgorm:req:bindings.row-count]
> `one` finding no row or several, `optional` finding several, and the `one`
> and `optional` of model, entity, graph and pipeline queries and of a
> pipeline's selected sources finding a count they do not take MUST be a
> `DecodeError`, the result being no record of the kind asked for: decided
> after the query, never in place of a database or decode failure, and never
> a `DatabaseError`, which PostgreSQL's errors alone are. A registered
> entity's write that reads back no row, pgorm's `RecordNotFound`, is a
> `DecodeError` too.

> [spec:pgorm:req:bindings.streams+1]
> `stream` on a pool or a connection MUST give an async iterator over a
> statement's rows through pgorm's `query_raw`, each pull giving one row, so
> that the driver's bounded buffer and the socket hold the server back while
> nothing pulls. A second pull while one is pending is refused. The stream holds its
> connection until its last row, which frees it, or until it is closed or
> abandoned early, which discards the connection if rows remained. A pool's
> stream holds a connection of its own; a connection's stream reserves that
> connection, which refuses other work meanwhile. A decode error ends the
> stream as an error, never as its end, and a database error keeps its
> SQLSTATE and the rows before it.
>
> `stream` on a transaction or a savepoint MUST give the same iterator through
> pgorm's `query_raw` on the `DatabaseTransaction`, opened by the closure
> transaction's task (`bindings.transactions`) and reserving the transaction
> until its last row: the transaction refuses other work meanwhile with a
> `LifecycleError`, and its scope ending while the stream is open, or the
> stream closed or abandoned with rows remaining, discards the transaction
> with its connection, nothing committed. A model's, a graph's and a disjoint
> read's `stream` take a transaction as they take a connection.

> [spec:pgorm:req:bindings.transactions+2]
> Transactions MUST open only through the language's scoped form on a pool or
> a connection, and savepoints only through the same form on a transaction
> (`python.transactions`, `napi.transactions`), as Rust's open only around a
> closure (`conn.tx.closure`). No method begins, commits, rolls back or closes
> one. A transaction takes pgorm's `TransactionMode`: an access mode and an
> isolation level, the combinations PostgreSQL acts on. A savepoint takes
> neither.
>
> Each transaction and savepoint MUST run inside pgorm's closure transaction,
> a task on the runtime serving its commands from inside the closure. A scope
> that ends normally returns `Ok` from the closure, committing or releasing;
> one that ends with an error returns `Err`, rolling back. So pgorm issues the
> `COMMIT` or `ROLLBACK`, and a connection given up mid-command drops the
> closure's future rather than await a rollback on it. A `COMMIT` that fails
> ends the transaction, discards its connection and is reported with its
> database error. A scope that ends normally after a statement in it failed,
> its error caught inside the scope, cannot commit: PostgreSQL has aborted
> the transaction, and pgorm rolls it back, or rolls a savepoint back to,
> and reports `Error::TransactionAborted` (`conn.tx.aborted`). The scope's
> end then raises a `DatabaseError` whose `sqlstate` is `25P02` and whose
> message is pgorm's account of the rollback, the same text in each binding,
> and keeps its connection, which is in a known state. A `ROLLBACK` that
> fails, which pgorm reports as `TransactionError::RollbackFailed`, discards
> the connection and is reported with both errors (`bindings.rollback-failed`).
> An abandoned scope whose `ROLLBACK` fails discards the connection too.
>
> pgorm's rule that a transaction borrows its parent exclusively MUST hold:
> while a transaction is open its connection refuses other work; while a
> savepoint is open its transaction refuses statements and other savepoints;
> and a statement started while another runs on the same transaction is
> refused. Each refusal is a `LifecycleError`, never queued or raced. A closed
> or foreign transaction handle fails explicitly. Nothing is retried but what
> `transaction_with_retry` runs again (`bindings.transaction-retry`): neither a
> write nor any other failed transaction.

> [spec:pgorm:req:bindings.transaction-retry]
> `transaction_with_retry(work, retries, mode, isolation)` on a pool and a
> connection MUST run `work`, a function given each attempt's `Transaction`,
> through pgorm's `DatabaseConnection::transaction_with_retry`, each attempt a
> frame of the closure transaction (`bindings.transactions`) on one connection,
> a pool's lent for the whole. An attempt whose `work` fails with an error
> pgorm calls retryable, or whose `COMMIT` fails so, is rolled back and run
> again from `BEGIN`, up to `retries` more times, a whole number of a `u32`;
> past the last, or for any other failure, the transaction ends as a scope
> does, its failure reported as the scoped form reports it. The binding
> decides nothing itself: whether an attempt runs again is pgorm's answer,
> given whether the error `work` raised is retryable. `retries` of another type
> is a `TypeError`, and one out of range a `ConstructionError`.
>
> Every error of the binding MUST say whether it is retryable, as pgorm's
> `Error::is_retryable` answers for the error it came from: a serialization
> failure (`40001`) or a deadlock (`40P01`). Python spells it `is_retryable`, a
> `bool` attribute, and JavaScript `isRetryable`, a getter; an error the
> binding or the program made is not retryable.

> [spec:pgorm:req:bindings.rollback-failed]
> A transaction whose scope failed and whose `ROLLBACK` then failed MUST be
> reported with both errors, as Rust's `TransactionError::RollbackFailed` and
> `Error::RollbackFailed` carry them: Python raises `RollbackFailedError`, a
> `PgormError` whose `cause` is the block's exception and `rollback` the
> rollback's error, chained from the cause; JavaScript rejects with a
> `SuppressedError` whose `error` is the rollback's and `suppressed` what the
> callback threw (`napi.transactions`). A cancellation is never replaced this
> way. An error pgorm itself reports as `Error::RollbackFailed` is reported
> the same way.

> [spec:pgorm:req:bindings.cancellation]
> Acquiring a connection, running a statement, opening a transaction or a
> savepoint, and pulling a stream's rows MUST each be cancellable in the
> language's own way (`python.cancellation`, `napi.cancellation`). A cancelled
> operation's outcome is unknown — a write may or may not have happened — so
> the connection it ran on MUST be discarded rather than returned to its pool
> as healthy, and a transaction it ran in ends with nothing committed. A
> cancellation MUST NOT be reported as success, nor promise the rollback of a
> write whose outcome is unknown. An operation cancelled before it started
> sends nothing. Closing a pool or a connection MUST likewise interrupt what
> runs on it, idle transactions and streams included, without waiting for
> them, and leave no background work using a closed handle.

> [spec:pgorm:req:bindings.copy]
> `copy_from(table, format)` and `copy_to(source, format)` MUST build
> pgorm-query's `CopyFromStatement` and `CopyToStatement` (`sql.ast.copy`) in
> the format named — text, CSV or binary — chosen when the statement is
> built. A COPY TO reads a table, the rows of a `Select`, or an INSERT,
> UPDATE, DELETE or MERGE through its RETURNING list. A write with no
> RETURNING list MUST be refused with a `ConstructionError` when the COPY is
> built, as pgorm-query refuses to make it a source. A `Select` source is
> final once the COPY reads it, so its tie limit and WINDOW clause MUST be
> checked then by `check_tie_limit` and `check_windows`. Each option is the
> pgorm-query method of its name, offered where pgorm-query's typed statement
> has it: an option the statement's format or direction does not take MUST be
> refused with a `ConstructionError` as it is set, never written for the
> server to refuse, and a one-character option takes a one-character string.
> COPY takes no parameters, so `inspect()` gives pgorm-query's rendering,
> every value in it a literal, and no values, and a statement terminal MUST
> refuse a COPY.
>
> `copy_in(statement, source)` and `copy_out(statement)` MUST run a COPY
> through pgorm's `CopyTrait` (`conn.copy`) on a pool, a connection and a
> transaction or savepoint. A pool's COPY takes a connection of its own; a
> connection's or transaction's holds it, refusing other work with a
> `LifecycleError` until the COPY ends. `copy_in` takes bytes, or a
> synchronous or asynchronous iterable of bytes; text is refused, never
> encoded implicitly. It pulls the source only as the COPY takes data,
> through a bounded queue, so a large source streams rather than accumulates,
> and gives the rows written as an exact integer. A source that fails, or
> yields anything but bytes, MUST abandon the COPY so that nothing is
> written, and report that failure unchanged: outside a transaction the
> connection stays usable; inside one the transaction is aborted until it is
> rolled back. A row the server refuses is its `DatabaseError` once the data
> is sent. A COPY FROM the server refuses before any data moves leaves its
> connection out of step, so that connection is discarded and a transaction
> on it ends (`conn.copy`).
>
> `copy_out` gives, once the server accepts the statement, an async iterator
> of byte chunks with bounded buffering, holding the server back while
> nothing reads, and refusing a second pull while one is pending. Its end
> frees what it holds. Closing it before its end discards the connection, and
> any transaction on it, which the server would otherwise go on sending to. A
> failure the server reports part way leaves the connection as a failed
> statement does. A cancelled COPY follows `bindings.cancellation`: before it
> finishes it is abandoned, and when its outcome is unknown its connection or
> transaction is discarded.

> [spec:pgorm:req:bindings.copy-rows+2]
> `copy_in_rows(statement, rows)` and `copy_out_rows(statement)` MUST run a
> binary COPY as rows of values through `conn.copy.rows`, on a pool, a
> connection and a transaction or savepoint, held and released as `copy_in`
> and `copy_out` are (`bindings.copy`). The statement MUST be a COPY in the
> binary format, which each binding's types state; any other is refused. The
> columns are the ones pgorm describes before the COPY starts. A description
> the server refuses is its `DatabaseError`, and leaves the connection or
> transaction as a failed statement does.
>
> `copy_in_columns(statement)` and `copy_out_columns(statement)` MUST give
> that description without starting the COPY, through pgorm's
> `CopyTrait::copy_in_columns` and `copy_out_columns`, on a pool, a
> connection and a transaction or savepoint, for a COPY in any format: a list
> of `CopyColumn`s in the COPY's order. A `CopyColumn` reports its `name`,
> its `type` as a `TypeName` schema-qualified as the server names it, and,
> for `money`, the `money_places` its currency is written to. A program
> builds one of a built-in type by its name, as `pg_type` spells it or an
> element's followed by `[]`; any other type is a `ConstructionError` that
> says to describe the COPY instead, and `money` needs `money_places`, which
> no other type takes.
>
> `copy_in_rows(statement)` with no rows MUST give a `CopyRowSink`, pgorm's
> own, holding the COPY until it ends: `columns` are the described ones;
> `send(row)` writes one row, and `send_model(record)` a model's record, each
> of the COPY's columns taken from the record's field whose column it is, a
> column no field reads being a `ConstructionError`. A row the columns refuse
> is a `ConstructionError` before anything of it is written, and the sink
> goes on taking rows. `finish()` completes the COPY and gives the rows
> written; closing the sink, or leaving its scoped form, without `finish`
> abandons the COPY, so nothing is written.
>
> `rows` is a synchronous or asynchronous iterable of rows. Text, bytes and
> anything else are refused before anything is sent. Each row is a sequence
> of its columns' values in order; a row that is no sequence abandons the
> COPY. A value is read as a parameter is — null as NULL, a plain value by
> its inferred kind, a `Value` by its own — and written against its column's
> type as a parameter is bound. A row of another width, and a value its
> column's type does not take, MUST be a `ConstructionError` naming the
> columns or the column, abandoning the COPY as a failing source does, so
> nothing is written. Rows are pulled as the COPY takes them, a batch at a
> time, and the call gives the rows written.
>
> `copy_out_rows` gives a `CopyRowStream`: an async iterator of rows, each a
> sequence of values decoded by its column's type as a result column of that
> type is (`bindings.values`), fetched as they arrive with bounded buffering,
> one pull at a time, a `money` field read as the decimal `conn.copy.rows`
> reads it as, which no query's result column of `money` is. Its `columns`
> names the columns. A field that does not decode is a `DecodeError` naming
> the column and closes the stream, discarding its connection. Its end and its closing behave as `copy_out`'s
> stream does. `models(model)` MUST read the stream's rows instead as the
> model's records, each field found by its column among the COPY's and held
> to its kind as a model's terminals hold it (`bindings.model-records`).
>
> `CopyRowEncoder(columns)` and `CopyRowDecoder(columns)` MUST be pgorm's,
> over `CopyColumn`s, for a program that moves the bytes itself. The
> encoder gives the format's `header()` and `trailer()` and each row's
> `encode(row)` as bytes, a row read and checked as `send` reads and checks
> one. The decoder takes the data with `push(chunk)`, in chunks split
> anywhere, gives each row with `next_row()` once all of it has arrived,
> null until then, its fields decoded as `copy_out_rows` decodes them, and
> `finish()` refuses data that ends part way through a row or without the
> trailer, each refusal a `DecodeError`.

> [spec:pgorm:req:bindings.listen+3]
> `listener(capacity)` on a pool MUST open a `Listener` through pgorm's
> `DatabasePool::listener` (`conn.listen`): a connection of its own, opened
> with the pool's configuration and TLS, never one the pool lends, holding at
> most `capacity` notifications unread — a positive whole number, 1024 by
> default, anything else refused before anything is sent. The listener
> belongs to its pool: closing the pool closes it. `connect_listener(dsn,
> options)` MUST open one through pgorm's `connect_listener`, belonging to no
> pool: its connection string, connect timeout and TLS (`bindings.tls`) read
> as a pool's are and its `capacity` as `listener`'s, and nothing but its own
> close, or its iteration abandoned, closes it.
> Closing the listener, or abandoning its iteration early, closes its
> connection. It holds nothing the event loop waits on unless a pull is in
> flight.
>
> `listen(channel)`, `unlisten(channel)` and `unlisten_all()` MUST send
> pgorm's `LISTEN`, `UNLISTEN` and `UNLISTEN *`, the channel an exact name,
> and MUST be usable while another task iterates the listener. A channel
> longer than 63 bytes MUST be a `ConstructionError` before anything is sent,
> and one that is not text is refused. A cancelled `listen`, `unlisten` or
> `unlisten_all` MUST close the listener, since which channels the server
> listens on is then unknown.
>
> The listener MUST be an async iterator of `Notification`s carrying
> `channel`, `payload` and `process_id`, pulled one at a time in the order the
> server sent them; a second pull while one is pending is a `LifecycleError`.
> A cancelled pull leaves the listener open and loses no notification: the one
> it waited for is the next pull's. Nothing reconnects a listener. When its
> connection fails or is terminated, iteration MUST yield the notifications
> already received first and then fail with that failure, a `DatabaseError`
> carrying the server's SQLSTATE where it sent one. When a notification
> arrives while `capacity` wait unread, pgorm closes the connection, and the
> pull past the waiting ones MUST fail with a `ConnectionError`. Either closes
> the listener, and iteration then ends. A pull pending when its pool closes
> fails with a `LifecycleError`, and one pending when the listener itself
> closes ends the iteration, as a pull made after it closed does. `listen`
> and `unlisten` on a closed listener are a `LifecycleError`.
>
> `notify(channel, payload)` on a pool, a connection and a transaction MUST
> run pgorm's `notify`, `SELECT pg_notify($1, $2)` with both bound, as a
> statement runs there. So in a transaction it is delivered at commit and
> never after a rollback, and a cancelled one discards the connection as
> `bindings.cancellation` requires. A channel or payload that is not text is
> refused. The server's bounds — a payload of 8000 bytes or more, an
> empty channel, one longer than 63 bytes — are its `DatabaseError`, SQLSTATE
> `22023`.

> [spec:pgorm:req:bindings.metrics]
> `InstrumentedPool(pool, metrics)` MUST wrap `pool` in pgorm's
> `InstrumentedPool` (`metric.layer`), reporting to `metrics`, a collector the
> program writes: an object with the seven hooks of `metric.layer.collector`,
> `record_query_success(query, duration, rows)`,
> `record_query_error(query, duration, error)`,
> `record_connection_acquired(duration)`,
> `record_connection_error(duration, error)`,
> `record_transaction_begin(duration)`, `record_transaction_commit(duration)`
> and `record_transaction_rollback(duration)`. A collector lacking one, a pool
> that is no `Pool` and a pool already instrumented are each a `TypeError`.
> The instrumented pool is a `Pool` over the same connections: it shares the
> wrapped pool's size, its status and its closing, so closing either closes
> both, and dropping it closes neither. `inner` is the wrapped pool, whose work
> is not reported, and `metrics` the collector.
>
> What runs through an instrumented pool — on it, on a connection it lends, or
> in a transaction or savepoint on one; a model's, a registered entity's and a
> pipeline's terminals, a stream, a COPY's start and a notification included —
> MUST run on pgorm's `InstrumentedConnection` and `InstrumentedTransaction`, a
> transaction in a mode through `InstrumentedConnection::transaction_with`, so
> that what is timed and which hook hears it are `metric.layer.delegate`'s,
> `metric.layer.copy`'s and `metric.layer.tx`'s and never the binding's own.
> Each acquisition is reported by pgorm's `InstrumentedPool::get`; one the
> pool's acquire timeout cuts short is reported to nobody, as in Rust a `get`
> a caller's timeout drops is not. A listener is not reported
> (`bindings.listen`). A pool that is not instrumented runs on pgorm's plain
> types and calls no collector code at all.
>
> A query hook's `query` is a `QueryContext` carrying the `operation`, `sql`
> and `fingerprint` of `metric.fingerprint`, JavaScript spelling an operation
> as it spells a closed choice; `QueryContext(operation, sql)` makes one. Its
> `fingerprint` is a `QueryFingerprint` — its string the 16 hex digits, its
> `value` the same number — or null, computed by pgorm's
> `QueryContext::fingerprint` when it is read. Both bindings build pgorm with
> `metrics-fingerprint`, so only a context with no statement text, or text
> libpg_query refuses, has none. `duration` is a number in the unit the
> binding's timeouts take, seconds in Python and milliseconds in JavaScript;
> `rows` a whole number or null; and `error` the error the caller is given, not
> raised.
>
> A hook MUST NOT run on pgorm's runtime. The runtime hands it to the
> language's thread as it hands over an operation's result — through the
> pool's event loop in Python, the addon's channel in JavaScript — and awaits
> it: a plain hook until it returns, one that returns an awaitable or a promise
> until that settles. So, as in Rust, the statement a hook reports on finishes
> only once its collector has been told, and one connection's reports arrive in
> order. A hook that cannot be handed over, the language's thread gone, is
> skipped. A hook that raises or rejects MUST NOT change the outcome of the
> statement it reports on: its error goes where the language reports an error
> nothing awaits — the event loop's exception handler in Python, a process
> warning named `MetricsWarning`, the error its `cause`, in JavaScript — and
> the statement goes on.
>
> `NoOpMetrics` is a collector whose every hook does nothing. A collector
> extending it overrides the hooks it wants, and a hook left as `NoOpMetrics`
> wrote it is never handed over. `LoggingMetrics` is the counterpart of
> pgorm's: it writes each report in its words and at its levels — `debug` for
> a successful query, an acquisition, a begin and a commit; a warning for a
> failed query and a rollback; an error for a failed acquisition — to a
> logger, Python's `logging.getLogger("pgorm.metrics")` or JavaScript's
> `console` unless the program gives another.

## Raw SQL

> [spec:pgorm:req:bindings.batch-execute]
> `batch_execute(script)` on a pool, a connection and a transaction MUST run
> `script`, `;`-separated statements and no parameters, through pgorm's
> `ConnectionTrait::batch_execute`, the simple-query protocol, preparing
> nothing; a pool's on a connection of its own. A script is text, a `TypeError`
> otherwise, and what the server refuses is its `DatabaseError`.

> [spec:pgorm:req:bindings.sql-check]
> `sql(text)` MUST hold `text` to PostgreSQL 18's grammar through
> `pgorm::sql_check::check_sql` (`macros.runtime`), a script statement by
> statement, and return it unchanged: the run-time counterpart of Rust's
> compile-time `sql!`, called where a module defines its statements. A
> rejection is a `ConstructionError` whose message is the rejection's text:
> the parser's message and the line it stopped on with a caret under the token.
> The check is the grammar's alone. Text that is not a string is a `TypeError`.

> [spec:pgorm:req:bindings.prql-text]
> `prql(text, values)` MUST compile `text`, PRQL, through
> `pgorm::sql_check::compile_prql`, hold the SQL to the grammar, refuse a gap
> in its placeholders or `$0`, and hold the number of `values` to its distinct
> placeholders, giving the SQL and the values in placeholder order as the
> binding's raw statement holds them, Python's `RawSQL` and JavaScript's pair
> of `sql` and `values` (`python.raw`, `napi.raw`): the counterpart of Rust's
> `prql!`. Each refusal is a
> `ConstructionError` with the text pgorm's refusal has, prqlc's own diagnostic
> for PRQL it rejects, `take $1` among it.

> [spec:pgorm:req:bindings.raw-models]
> A declared model's `find_by_statement(statement)` MUST read a raw
> statement's rows into the model's records as Rust's
> `FromQueryResult::find_by_statement` reads them: each field from the result
> column of its SQL name, held to its declared kind as the model's own reads
> hold it (`bindings.model-records`), columns no field reads left unread, and a
> field whose column is absent a `DecodeError`. It gives a `SelectorRaw` whose
> `all`, `one`, `optional` and `stream` run the statement as written, adding no
> `LIMIT`, with `paginate` and `count` (`bindings.raw-pages`).
> `from_statement(statement)` gives a `SelectorRaw` whose rows are the
> binding's plain rows, as Rust's `SelectorRaw::from_statement` holds a
> statement. A registered entity's query's `from_raw_sql(statement)` reads them
> through Rust's `Select::from_raw_sql`, the entity's own `FromQueryResult`.
> Python takes the statement as a `RawSQL` and JavaScript as SQL text and its
> parameters, each binding's raw statement (`python.raw`, `napi.raw`).

> [spec:pgorm:req:bindings.verify]
> `verify(statement, model)` on a pool, a connection and a transaction MUST
> check a declared model against the result columns the server describes for
> the statement through `VerifyStatement::describe`, without running it: each
> field's column, by its SQL name, MUST be among them, and of a type whose
> values decode as the field's declared kind, the test a row's decode applies.
> A field that fails is a `DecodeError` naming the model's table, the column
> and either the columns the statement returns or the kind and the PostgreSQL
> type, as Rust's `VerifyError` names them; a statement the server refuses to
> prepare is its `DatabaseError`. The statement's values are neither bound nor
> checked.

> [spec:pgorm:req:bindings.raw-pages]
> `paginate(page_size)` and `count(db)` on a `SelectorRaw` MUST page and
> count the statement as Rust's `SelectorRaw::paginate` and
> `count` do, through `pgorm::raw_select`: the statement read as a subquery,
> its values renumbered into the page's or the count's, and refused with a
> `ConstructionError` as `paginate` is called, or as `count` runs, unless it is
> one row-returning `SELECT` whose markers and values pair
> (`exec.paginator.raw`). The pages hold what the selector's rows are, plain
> rows or the model's records; the `Paginator` is the one a model's query
> gives (`bindings.pagination`).

> [spec:pgorm:req:bindings.refusal-fields]
> A value the driver refused as it encoded a parameter MUST be reported as a
> `ConstructionError` carrying pgorm's `BindRefusal` as fields, `parameter`,
> the one-based `$N`, and `reason`, the codec's refusal or null; and a result
> the driver could not read as a `DecodeError` carrying `DecodeRefusal`'s
> `reading` and `reason`. Any other `ConstructionError` or `DecodeError` holds
> null in each. The message is the refusal's text (`bindings.errors`).

## Statements

Each binding builds pgorm-query's statements and expressions as builder
objects whose methods return new builders, and runs a built statement through
the same terminals that run raw SQL.

> [spec:pgorm:req:bindings.statements]
> Each binding MUST build pgorm-query's statements and expressions through
> builder objects that each own the pgorm-query builder state they stand for,
> held by the native extension. A builder MUST be immutable: each method
> returns a new builder, made from a copy of its receiver's state by the
> pgorm-query method of the same meaning. A builder reused, or extended in two
> directions, leaves itself and each extension unchanged, and reusing one
> expression in two queries mutates neither nor leaks parameters between
> them. A builder is made only by the binding's own constructors. An argument
> a method cannot use MUST be refused as the method is called, so a statement
> that cannot be built never exists to be run. A binding owns its Rust state;
> it MUST NOT extend a borrowed lifetime or bypass a Rust typestate with an
> unchecked cast.
>
> `inspect()` MUST give the SQL and the bound values, in placeholder order and
> each a `Value` carrying its kind, that running the builder sends: a
> statement as it runs, an expression as the one item of a `SELECT`, and a
> condition as the `WHERE` of `SELECT TRUE`. Running and inspecting use the
> same builder state. The terminals (`bindings.results`) take a built
> statement where they take raw SQL and build it with the function `inspect()`
> uses, binding its values as any parameter is bound. An expression or a
> condition is no statement to run. A statement binding more than 65,535
> values MUST be refused with a `ConstructionError` before anything is sent.

> [spec:pgorm:req:bindings.identifiers+3]
> Identifiers, qualified type names, values, typed structural choices and raw
> SQL MUST each have their own constructors, so no one is taken for another.
> Every identifier — of a column, a table, a schema, an alias, a collation, a
> common table expression or a type — MUST be text of 1 to 63 UTF-8 bytes
> without NUL, minted with `Name::runtime`, so it is quoted wherever
> pgorm-query writes it and a dot in it is part of the name; any other text is
> a `ConstructionError`. `Name::runtime` is Rust's one route from text to a
> name, and every name a binding is given is computed at run time, so the
> binding mints it where an identifier position reads the text: there is no
> identifier class of a binding's own, and an identifier is the language's
> string in both. Schema, table and column names are runtime inputs;
> no fixture's names are required. Accepted text MUST reach the Rust API
> unchanged, quotes, percent signs, backslashes and Unicode included. A
> language or PostgreSQL limit on a representation MUST be an explicit error,
> never a silent normalization.

> [spec:pgorm:req:bindings.select+4]
> `select` MUST build pgorm-query's `SelectStatement` through the Rust
> builder's own methods:
>
> - a projection of expressions, aliased expressions and windowed calls, `*`
>   until one is given;
> - FROM items: a `Table`, schema-qualified and aliased, or a `FromItem`,
>   which is always aliased; a table's `tablesample`, by the Bernoulli or
>   system method with a bound percentage and an optional bound seed
>   (`sql.ast.select.table-sample`); a table function, built as
>   pgorm-query's `TableFunction` is (`sql.ast.select.table-function`): the
>   class constructed over one set-returning call, or `rows_from(call)`,
>   `ROWS FROM` over it; `and(call)`, a further call read beside the others,
>   which is `ROWS FROM` whichever began it; `with_ordinality()`;
>   `columns(names..)`, the column names its alias gives, replacing any
>   given before; `column_definition(name, type)`, a column of the `record`
>   the call added last returns, lowered into pgorm-query's
>   `column_definition` (`sql.ast.select.column-definitions`); and
>   `alias(name)`, which finishes it as the `FromItem` the FROM list and the
>   joins take. Each call is one `call` makes, another expression a
>   `ConstructionError`; and `JSON_TABLE` (`bindings.sql-json`);
> - inner, left, right and full joins on a predicate, and cross joins;
> - WHERE and HAVING, each call ANDed to what is there;
> - GROUP BY over expressions and the grouping elements, each made by the
>   static factory of `GroupingElement` named for the pgorm-query constructor
>   that builds it: `set(exprs..)`, a grouping set, one expression rendering
>   bare and none `()`; `empty()`, the empty set; `rollup(exprs..)` and
>   `cube(exprs..)`; and `sets(first, rest..)`, GROUPING SETS, which takes its
>   first element so it is never empty, an expression among its elements
>   being the set of itself. `GroupingElement` is exported for annotations and
>   `isinstance` (`instanceof`) and cannot be called, a call being a
>   `TypeError`. And `group_by_distinct` (`sql.ast.select.grouping`,
>   `sql.ast.select.group-distinct`);
> - ORDER BY with a direction and NULLS FIRST or LAST;
> - LIMIT and OFFSET, each a non-negative integer within PostgreSQL's `bigint`,
>   or null to remove one, any other integer a `ConstructionError`; and
>   `limit_with_ties`, the same count in LIMIT's slot;
> - DISTINCT;
> - UNION, INTERSECT and EXCEPT, each with its ALL form;
> - a WINDOW clause (`bindings.windows`).
>
> `limit_with_ties` is refused with a `ConstructionError` by pgorm-query's
> `check_tie_limit` (`sql.ast.select.tie-limit`) when the statement has no
> ORDER BY or locks SKIP LOCKED. The order may be added after the limit, so
> the method cannot refuse it: `inspect()` and every run do. A method reading
> the statement into another — a subquery, a set operation's arm, an
> INSERT's rows, a common table expression's body, a view or a COPY — refuses
> it as it is called, since nothing can order it after.
>
> A set operation reads the other `Select` into this one as a parenthesised
> arm, its tie limit checked as it is read; anything but a `Select` is
> refused. The arms are written in call order after the receiver's HAVING
> and WINDOW clauses, and combine in call order, as Rust's do:
> `a.union(b).intersect(c)` is `(a UNION b) INTERSECT c`, everything before
> an INTERSECT that follows a UNION or EXCEPT parenthesised so PostgreSQL's
> precedence cannot regroup it (`sql.ast.select.union`). The receiver's ORDER
> BY, LIMIT and OFFSET, whenever they were set, order and bound the combined
> rows, and its WITH clause precedes every arm.

> [spec:pgorm:req:bindings.from-subqueries]
> A `Select` MUST become a FROM item under an alias, `as(alias)`, read as a
> subquery wherever a FROM item is read — the FROM list, a join, a MERGE's
> source — its tie limit and windows checked as it is read. A join MUST take
> a lateral option, which reads its item `LATERAL`, so a subquery joined
> lateral can read the columns of the items before it.

> [spec:pgorm:req:bindings.row-locks]
> A `Select` MUST take a row lock: one of the four strengths — FOR UPDATE, FOR
> NO KEY UPDATE, FOR SHARE and FOR KEY SHARE — OF the FROM items named, each
> by the name it answers to in the statement, and with NOWAIT or SKIP LOCKED,
> through pgorm-query's own lock builder.

> [spec:pgorm:req:bindings.distinct-on]
> A `Select` MUST take `distinct_on(columns..)`, `SELECT DISTINCT ON (..)`,
> through pgorm-query's `distinct_on`, which takes columns: each argument is a
> column expression, and another expression, or no column at all, is a
> `ConstructionError`. `distinct()` and `distinct_on` replace each other, the
> later call winning, as pgorm-query's do.

> [spec:pgorm:req:bindings.with+1]
> A WITH clause MUST hold common table expressions in call order, each named.
> A body is a `Select`, or a MERGE with a RETURNING list. A SELECT and a MERGE
> take a WITH clause.

> [spec:pgorm:req:bindings.cte-options]
> A common table expression MUST take an optional column list and an optional
> materialization, MATERIALIZED or NOT MATERIALIZED. `WITH RECURSIVE` MUST
> hold exactly one, whose body is a `Select`, with SEARCH (breadth or depth
> first, by an expression, into a column) and CYCLE (by an expression, into a
> mark column, using a path column); a second one is a `ConstructionError`. A
> MERGE's WITH clause is never recursive, as PostgreSQL refuses it, so a
> recursive one given a MERGE is a `ConstructionError`.

> [spec:pgorm:req:bindings.write-ctes]
> INSERT, UPDATE and DELETE MUST each take a WITH clause (`bindings.with`),
> and each, with a RETURNING list, MUST be a common table expression's body,
> whose rows are those its RETURNING list yields.

> [spec:pgorm:req:bindings.writes+2]
> `insert`, `update` and `delete` MUST build pgorm-query's INSERT, UPDATE and
> DELETE. An INSERT names its distinct columns once, before any row, a second
> `columns` call being a `ConstructionError`. Each `values` call adds one row
> of exactly as many operands, by pgorm-query's arity check, and
> `default_values` writes one row of defaults, mixed with neither.
>
> `on_conflict` takes only a completed action, which `Conflict`'s factories
> begin: `Conflict.do_nothing()`, answering any conflict; or an arbiter,
> `Conflict.on(columns..)`, a non-empty list of columns with an optional
> partial-index predicate, `where`, followed by `do_nothing()` or a non-empty
> update. An update is begun and extended by `update(columns..)`, one column
> or more taken from EXCLUDED, and `set(column, value)`, a column set to an
> expression, and takes an optional condition, `where`; an empty list of
> targets or of columns is a `ConstructionError`. `Conflict`,
> `ConflictTarget` and `ConflictUpdate` are exported for annotations and
> `isinstance` (`instanceof`) and cannot be called, a call being a
> `TypeError`.
>
> An UPDATE has at least one assignment, each column once. An INSERT with no
> row, an UPDATE with no assignment, and an UPDATE or DELETE with neither a
> WHERE nor an explicit `all_rows()` MUST be refused with a
> `ConstructionError` when it is inspected or run, or read into a WITH clause
> (`bindings.write-ctes`). `returning` gives the
> RETURNING list, every column when it is empty, and reads a written row's old
> and new versions through `ReturningRow`'s old and new rows, renamed with
> the `old_as` and `new_as` options.

> [spec:pgorm:req:bindings.write-sources]
> An INSERT MUST take its rows from a `Select` of as many columns as it names,
> through `select`, by pgorm-query's arity check and mixed with neither
> `values` nor `default_values`, the `Select` checked as a subquery is as it
> is read (`bindings.select`); and it MUST take `OVERRIDING SYSTEM VALUE` or
> `OVERRIDING USER VALUE`. An UPDATE MUST read further FROM items through
> `from`, and a DELETE through `using`, each a `Table` or a FROM item.

> [spec:pgorm:req:bindings.conflict-arbiters]
> A conflict arbiter's non-empty list MUST take index expressions beside
> columns, its partial-index predicate covering both, and an arbiter MUST
> instead name a constraint, `on_constraint(name)`, which takes no predicate:
> a predicate given one is a `ConstructionError`.

> [spec:pgorm:req:bindings.merge+2]
> `merge(target, source, on)` MUST follow pgorm-query's MERGE typestate. Its
> source is a table. It gives a `PendingMerge`, which has no `inspect()` and
> which every terminal refuses with a `ConstructionError`, because PostgreSQL
> refuses a MERGE with no WHEN arm; its first arm gives the `Merge`
> statement. `MergeAction`'s factories make an arm's action. `when_matched`
> and `when_not_matched_by_source` take an action on a target row —
> `MergeAction.update(column, value)`, `delete()` or `do_nothing()` — and
> `when_not_matched` an action on a source row —
> `MergeAction.insert(column, value)`, `insert_defaults()` or `do_nothing()`.
> An update and an insert take their first column with its value, so neither
> is ever empty, and `set(column, value)` adds another; an insert takes
> OVERRIDING. An action given to an arm whose row it cannot take MUST be a
> `ConstructionError`. `MergeAction`, `MergeUpdate` and `MergeInsert` are
> exported for annotations and `isinstance` (`instanceof`) and cannot be
> called, a call being a `TypeError`. Each arm takes an optional condition.
> Within a kind of row the unconditional arm renders after the conditional
> ones, and a later one replaces it, as pgorm-query orders them. `returning`
> reads a row's versions as a write's does; `returning_action()` puts
> `merge_action()` first in the list, its only form; and `only()` writes
> `ONLY` before the target. A MERGE reads a common table expression and, with
> a RETURNING list, is a common table expression's body (`bindings.with`).

> [spec:pgorm:req:bindings.merge-sources]
> A MERGE's source MUST be, beside a table, any FROM item: a subquery
> (`bindings.from-subqueries`), a table function, a sampled table or
> `JSON_TABLE`.

> [spec:pgorm:req:bindings.windows]
> `over` MUST attach a window only to what pgorm-query's `WindowFunction`
> admits — a function call, or the JSON aggregates `json_arrayagg` and
> `json_objectagg` — and anything else is a `ConstructionError`. The call
> under it is a projection item, named with `as`. Its window is a `Window`,
> or an `ExtendedWindow(base)` built on an entry of the statement's WINDOW
> clause, written inline, or the name of an entry the clause declares.
>
> `window(name, window)` MUST append an entry to the WINDOW clause, which
> holds every entry in call order, as `SelectStatement::window` appends them:
> a `Window`, or an `ExtendedWindow(base)`, which builds pgorm-query's
> `ExtendedWindow` on an earlier entry with an ORDER BY and a frame and has no
> `partition_by` (`sql.ast.window-statement`); anything else is refused. A
> clause PostgreSQL refuses — a name declared twice, or an extended window
> whose base is no earlier entry, has a frame, or already orders its rows when
> it adds an ORDER BY — a call over a name the clause does not declare, and a
> call over an inline extended window whose base no entry declares, has a
> frame, or already orders its rows when it adds an ORDER BY, are refused by
> pgorm-query's `check_windows` with a `ConstructionError`, where the tie
> limit is checked: by `inspect()` and every run, and by a method reading the
> statement into another as it is called.
>
> A `Window` builds pgorm-query's `WindowStatement` with PARTITION BY, ORDER
> BY and a frame. A frame MUST follow pgorm-query's frame typestate, which
> types a start by the side of the current row it lies on: a start from
> ROWS, RANGE or GROUPS offers only the ends that may follow it, so no frame
> can be built whose end lies on an earlier side than its start, that starts
> at `UNBOUNDED FOLLOWING` or ends at `UNBOUNDED PRECEDING`; a following start
> does not stand alone as a frame; and an exclusion belongs to a frame. Two
> offsets on one side are not compared, as PostgreSQL's grammar does not
> compare them: `ROWS BETWEEN 1 PRECEDING AND 3 PRECEDING` builds and holds
> no rows. `window_function` builds PostgreSQL's general-purpose window
> functions at the argument counts each takes, usable only through `over`,
> which the server requires of them; another name or count is a
> `ConstructionError`.

> [spec:pgorm:req:bindings.sql-json]
> SQL/JSON's query functions `json_exists`, `json_value` and `json_query`, its
> constructors `json_object`, `json_array`, `json_array_query`,
> `json_objectagg`, `json_arrayagg`, `json_parse` (`JSON(..)`), `json_scalar`
> and `json_serialize`, `format_json`, and `is_json` and `is_not_json` MUST
> lower into pgorm-query's SQL/JSON builders, each clause applied through the
> builder's own method. A path is text pgorm-query binds as text cast to
> `jsonpath`; PASSING takes variable names, each an identifier, mapped to
> values. Behaviours are typed per function, so a choice PostgreSQL refuses
> for a function cannot be passed: `json_exists` takes true, false, unknown or
> error; `json_value` null, error or a DEFAULT value; `json_query` those, an
> empty array or an empty object, and one shaping of with wrapper, with
> conditional wrapper or omit quotes. A choice the Rust builder cannot
> express MUST be an error rather than be dropped. A DEFAULT value is written
> as an escaped literal, because PostgreSQL refuses a parameter there, and one
> that carries an enum's or a created range's cast is refused. RETURNING
> takes a `DataType` or a built-in type's name, and `json_value` refuses
> `json` and `jsonb`. `json_arrayagg`'s ordering takes no NULLS placement, and
> one that asks for it is refused rather than dropped.
>
> `json_table(context, path, columns, alias)` MUST build a FROM item through
> `Func::json_table`: its columns a non-empty list of ordinality, value,
> query, exists and nested columns, each taking only the clauses its kind of
> column admits; its alias required, so an empty column list or an unnamed
> item cannot be built; its paths written as escaped literals and its PASSING
> values bound. Its ON ERROR behaviour is error or empty.

## Expressions

> [spec:pgorm:req:bindings.operands]
> A value an operand is given that is not an expression MUST be bound, in
> statements and pipelines alike, never written into the SQL: in a statement
> as `bindings.expressions` binds it, and in a pipeline inferred or declared
> with `Value` as a parameter is, an interval included, and minted as a
> placeholder by the binder of the stage that takes it, a stage given no
> binder function taking its binder form (`bindings.pipeline-binder`) when one
> of its operands binds. A window's partition and ordering take no value.
>
> Both bindings MUST offer `literal(value)` in statements and in pipelines,
> the one way a value reaches the SQL text, never by concatenation: a
> statement's through pgorm-query's own rendering of a constant
> (`SimpleExpr::Constant`), the value read as a parameter is read and cast as
> its kind needs, an enum's label to its type and a created range's text to
> its range; a pipeline's as pgorm's pipeline writes a literal
> (`bindings.pipeline-expressions`). The literal path and the bound path stay
> apart: neither replaces the other.

> [spec:pgorm:req:bindings.expressions+4]
> An expression MUST lower into pgorm-query's own builders. `col` is a column,
> qualified by a table and its schema. An operand that is not an expression is
> a value bound as a parameter, never interpolated: inferred as a parameter
> is, or declared with `Value`, an enum's label written cast to its type and a
> created range's text cast to its range through `Expr::as_range`. `bind`
> makes a value an expression. An `Interval` binds as pgorm's interval value
> (`sql.value.interval`), its fields kept apart. A null operand, which has no
> kind to bind, MUST be a `ConstructionError` naming the explicit form.
>
> Comparison and arithmetic, `||` (`concat`), `IS [NOT] DISTINCT FROM`, AND,
> OR, NOT, `IS [NOT] NULL`, `[NOT] BETWEEN [SYMMETRIC]` and `[NOT] IN` a list —
> an empty one included, which no value is in — or a `Select`'s rows MUST each
> be the pgorm-query method of that meaning, and `Condition.all` and
> `Condition.any` pgorm-query's `Condition`, an empty one true and false.
> Condition grouping and empty conditions mean what the named Rust API means.
>
> `like`, `ilike`, `similar_to` and their negations take a pattern, bound, as
> pgorm-query's take an `IntoLikeExpr` (`sql.ast.expr.similar-to`): text,
> escaped by the default backslash, or a `LikeExpr`, pgorm-query's
> `LikeExpr`, the class constructed over the pattern's text. Its
> `escape(character)` escapes with that one character instead, any other
> text, the empty string and NUL among it, a `ConstructionError`; and its
> `no_escape()` turns escaping off. `LikeExpr.literal(text)`,
> `LikeExpr.containing(text)`, `LikeExpr.starting_with(text)` and
> `LikeExpr.ending_with(text)` (`startingWith` and `endingWith` in
> JavaScript, static methods of the class there as in Python) build one over
> literal text through pgorm-query's constructors of those names, its `%`,
> `_` and escape character escaped, so a user's search text matches itself.
> A `LikeExpr` is no expression, and nothing but these six methods reads
> one. `regex` (`~`), `not_regex`
> (`!~`), `iregex` (`~*`) and `not_iregex` (`!~*`) match a POSIX regular
> expression. `matches` (`@@`) is `BinOper::Matches`, full-text search or a
> `jsonpath` predicate. `path_exists` (`@?`) takes a `jsonpath` as text,
> bound as pgorm-query's `json_path_exists` binds it, as text cast to
> `jsonpath`; any other value is a `ConstructionError`. `starts_with`,
> `ends_with` and `contains_text` read their argument as text through
> PostgreSQL's `starts_with`, `right` and `strpos`, so `%` and `_` match
> themselves. The other operand of each binary operator is bound unless it is
> an expression.
>
> `call` reaches only the functions pgorm-query has a constructor for, at the
> argument counts each takes — `uuidv7` with or without its shift among them
> — and another name or count is a `ConstructionError`: the name selects a
> constructor and never reaches SQL. `generate_series`, with or without its
> step, and `unnest` are the set-returning ones a table function reads, and
> `json_to_record`, `jsonb_to_record`, `json_to_recordset` and
> `jsonb_to_recordset` the record-returning ones it reads through a column
> definition list. `tuple` is a row of expressions, never empty.
>
> A cast names its type by name or `TypeName`. `at(index)` is pgorm-query's
> `Expr::index` (`a[index]`) and `slice(lower, upper)` its `Subscript::Slice`
> (`a[lower:upper]`), a null bound leaving that end open and each bound bound
> unless it is an expression. `exists(select)` is `Expr::exists`;
> `scalar(select)` is the scalar subquery `(select)`; and `array(select)` is
> `Expr::array`, `ARRAY(select)`: the subquery's one column as one array
> value, empty when it yields no row, which casts and compares as a value
> (`sql.ast.expr`), the server refusing a subquery of more than one column
> (`42601`). Each subquery is checked as any embedded subquery is.
> `domain_value()` is `Expr::domain_value()`, the keyword `VALUE` a domain's
> CHECK tests. `asc()` and `desc()` give an ordering, with NULLS FIRST or
> LAST, and `as(alias)` an aliased projection item.

> [spec:pgorm:req:bindings.condition-option]
> A `Condition` MUST take `add_option(condition)`, pgorm-query's
> `Condition::add_option`: the condition with the expression or `Condition`
> added, or the condition as it is when the argument is the language's
> absent value (`None`, `null` or `undefined`), so a search whose fields are
> optional adds each without a branch around the builder.

> [spec:pgorm:req:bindings.quantifiers+4]
> `any(source)` and `all(source)` MUST be `ANY (..)` and `ALL (..)`:
> pgorm-query's `Expr::any` / `Expr::all` over a `Select`'s rows, and
> `Func::any` / `Func::all` over an array, an expression or a value bound as
> one array parameter. Each is a `Quantified`, no expression, and MUST be
> taken as the right operand of each operator PostgreSQL reads one after:
> every operator its grammar takes before `ANY` or `ALL` whose result is a
> boolean, which the server requires of one:
>
> - the comparisons `eq`, `ne`, `lt`, `lte`, `gt` and `gte`;
> - `like`, `not_like`, `ilike` and `not_ilike`, the quantifier ranging over
>   patterns, which carry no escape, since the grammar reads no `ESCAPE`
>   after one;
> - `regex`, `not_regex`, `iregex` and `not_iregex`;
> - `contains`, `contained_by` and `overlaps`;
> - `strictly_left_of`, `strictly_right_of`, `not_extend_right_of`,
>   `not_extend_left_of` and `adjacent_to`;
> - `matches` and `path_exists`, the latter's quantifier ranging over
>   `jsonpath` values: PostgreSQL reads a bound array there as `jsonpath[]`,
>   which pgorm sends no text as, so an array of text is a
>   `ConstructionError` as the statement runs, and one cast to `jsonpath[]`
>   is sent.
>
> A list no element of which names the list's kind — an empty one, or one of
> ranges, whose kind no bound names — is typed by the registered column it is
> compared with, as that column's `compare_any` / `compare_all`
> (`bindings.entity-reads`), or by the declared model column, as its
> declared kind (`bindings.model-columns`): `= ANY` of an empty list is
> false and `<> ALL` true. Nothing else can type it, so beside an
> expression, a column neither a registration nor a declaration types, or
> anything but such a column it MUST be refused with the
> `ConstructionError` its kind's inference met, as the quantifier is used
> rather than as it is built.
>
> Each is the pgorm-query operator of its meaning with the quantifier as its
> right operand. The grammar reads none after `similar_to`, `not_similar_to`,
> `is_distinct_from`, `is_not_distinct_from`, `between` or a membership test,
> and the server refuses one after an operator whose result is no boolean
> (`42809`), arithmetic and `concat`. Those operators, every other operand
> position, a membership list and `bind` MUST refuse it with a
> `ConstructionError`.

> [spec:pgorm:req:bindings.case]
> `case_when(condition, result)` MUST build a searched CASE and
> `case_of(operand)` a simple CASE through pgorm-query's CASE builder
> (`sql.ast.case`). A CASE exists as an expression only once it has an arm:
> `case_of(operand)` gives an operand that takes `when(value, result)` before
> it is one, and every CASE takes further `when` arms and one `else` result.
> Each value and result is bound unless it is an expression, and a CASE is an
> expression wherever an expression goes.

> [spec:pgorm:req:bindings.ranges]
> `contains` (`@>`), `contained_by` (`<@`) and `overlaps` (`&&`) MUST compare
> ranges, multiranges and arrays through pgorm-query's operators of that
> meaning, and `strictly_left_of` (`<<`), `strictly_right_of` (`>>`),
> `not_extend_right_of` (`&<`), `not_extend_left_of` (`&>`) and
> `adjacent_to` (`-|-`) MUST place ranges and multiranges through them. A
> range or multirange operand is bound as the `Value` that declares its kind —
> a built-in range or multirange kind, or a `CreatedRange` or
> `CreatedMultirange`, whose text is cast to its type through
> `Expr::as_range` — and a `Range` or `Multirange` given without one MUST be a
> `ConstructionError`, a range's kind never being inferred.

> [spec:pgorm:req:bindings.collations]
> `collate(name, schema)` on an expression MUST put it under a collation
> through Rust's `Expr::collate` (`sql.ast.expr.collate`), written
> `(expr COLLATE "name")`, the schema first when one is given. It gives an
> expression, so the collated value goes on to be compared, and `asc()` or
> `desc()` over it writes the clause into the sort key, which is where
> PostgreSQL's grammar reads an ORDER BY's collation.
>
> `collate(name, schema)` on a `ColumnDef` MUST give the column its one
> collation through Rust's `ColumnDef::collate` (`sql.ddl.column-def`),
> written `COLLATE` after the type, a later call replacing an earlier, since
> PostgreSQL refuses a column with two. Creating a table or adding a column
> writes it where the column is declared, and modifying a column writes it on
> the column's retype, `ALTER COLUMN .. TYPE .. COLLATE`, the only place
> PostgreSQL changes a column's collation; every comparison and sort over the
> column then uses it. A domain's base takes one the same way
> (`bindings.schema-domains`).
>
> The name and the schema are each an identifier (`bindings.identifiers`),
> and one that is not MUST be refused with a `ConstructionError` before
> anything is built. What only the server knows it refuses by SQLSTATE, as a
> `DatabaseError`: a collation that does not exist (`42704`) and a collation
> on a type without one (`42804`).

> [spec:pgorm:req:bindings.named-functions]
> `func(name, ...arguments)` MUST call any function by name through
> pgorm-query's `Func::named`, each argument bound unless it is an expression.
> The name is an identifier (`bindings.identifiers`), written bare when it is a
> lowercase name PostgreSQL does not restrict and quoted otherwise, so it names
> a function and never reaches SQL as syntax; a function that does not exist
> is the server's `42883`. A call `func` makes is a function call a table
> function reads, so any set-returning function is a source. `call` reaches,
> beside the functions `bindings.expressions` names, `grouping` with one
> argument or more, pgorm-query's `Func::grouping`, and the full-text search
> functions `to_tsvector`, `to_tsquery`, `plainto_tsquery`, `phraseto_tsquery`
> and `websearch_to_tsquery` of one argument and `ts_rank` and `ts_rank_cd` of
> two.

> [spec:pgorm:req:bindings.template]
> `template` MUST write a piece of SQL the builders have no method for, as
> Rust's `Expr::template` does, its text held to program text as far as the
> language holds it: Python takes a t-string, `template(t"..")`, and
> JavaScript a tagged template literal, `` template`..` ``, the text of either
> written in the program and each interpolation a value. The pieces are SQL,
> and each interpolation goes between them through pgorm-query's
> `SqlTemplate::from_pieces` (`sql.render.template-pieces`): an expression
> embedded as it is, any other value bound. A plain string is a `TypeError`, so
> text built at run time becomes a template only by being written into the
> language's own template construct. A piece that writes a `$N` marker, a value
> that lands in a quoted region, and in Python an interpolation with a
> conversion or a format, are `ConstructionError`s.

> [spec:pgorm:req:bindings.json-operators]
> Every expression MUST have the `jsonb` accessors and key tests through
> pgorm-query's methods of those meanings: `get_json_field` (`->`) and
> `cast_json_field` (`->>`), whose key or index is bound unless it is an
> expression; `get_json_path` (`#>`) and `cast_json_path` (`#>>`), whose path is
> a list of keys bound as one `text[]`; `has_json_key` (`?`) of one key; and
> `has_any_json_keys` (`?|`) and `has_all_json_keys` (`?&`) of a list of keys.
> A key that is not text, and a path or list that is not a list of text, are
> `TypeError`s.

## Models

A model is data the application declares: a table and its columns, lowered
into the statement builders. It claims no Rust entity. Its reads and writes
decode into records of its fields, and its relations, graphs, cursors and
pages follow pgorm's own `RelationDef`, `SelectGraph`, `Cursor` and
`Paginator`.

> [spec:pgorm:req:bindings.models+6]
> `model(name, columns, schema, unique_keys, comment, rename_all)` MUST
> declare a model of a table, schema-qualified when a schema is given, from a
> mapping of fields, each a `Column(kind, options)`, the table's unique keys
> (`bindings.model-unique-keys`), its comment and the case style its columns
> are named in (`bindings.schema-from-model`). The kind is one every result
> decodes as — any value kind but `u64` and `char`, which no column decodes
> as — or a schema-qualified `TypeName`, `CreatedRange` or
> `CreatedMultirange`, as a decoded row names each type. The options are
> `name` (the SQL column, the field's own name, or the field written in the
> `rename_all` style, by default), `nullable`, `array`, `primary_key`,
> `default` (filled by a default the declaration does not state),
> `generated` (always or by default) and an enum's `values`, and the table's
> DDL options `bindings.schema-from-model` gives: `column_type`,
> `default_value`, `default_expr`, `generated_stored`, `generated_virtual`,
> `auto_increment`, `indexed`, `comment` and `without_overlaps`; the casts
> `bindings.model-casts` gives, `select_as`, `save_as`, `select_with` and
> `save_with`; and the conversions `bindings.model-conversions` gives,
> `decode` and `encode`. A column with
> a generation expression is generated always, as no write sets it; an insert
> may leave out a column that is nullable, generated, drawn from the serial
> family or filled by a default, stated or not.
>
> Any other value MUST be a `ConstructionError` raised as the model is
> declared: a key column that is nullable, a column filled two ways (a
> default, stated or not, an identity, a generation expression, the serial
> family), labels on a column that is no enum, two fields of
> one column, a field named twice, an enum or created range without its
> schema, an identifier pgorm would not take, and what
> `bindings.schema-from-model` refuses of the DDL options. Declaring a model MUST send
> nothing, run no DDL and claim no Rust entity: no derive, `EntityTrait`,
> implementation or ActiveModel hook is behind it. A model and its columns
> never change; `as(alias)` gives another model reading the table under an
> alias. Its columns are qualified as pgorm qualifies an entity's, by the
> table's alias or else its name, never its schema.
>
> A `Column` is a declaration and has no method: the model converts a value
> through the field's column where it takes one, and `Value(data, kind)`
> (`bindings.value-tags`) is the conversion a program calls. `describe()`
> MUST report the model as data, under these keys and no other, each spelled
> by the binding's naming rule: `schema`, `table` and `alias`, its table's;
> `primary_key`, its key fields; `unique_keys`, its unique keys
> (`bindings.model-unique-keys`); and `fields`, mapping each field to its SQL
> `column`, its `kind`, `nullable`, `primary_key`, `default`, `generated`
> (always or by default, or null) and an enum's `values` (null for any other
> column). A kind is spelled as a decoded column's is compared
> (`bindings.model-records`): a scalar kind's name; `enum` and the enum's
> type; `range` or `multirange`, a created range's type, `of` and its
> subtype's kind; and an array's element's spelling followed by `[]` — `i64`,
> `text[]`, `enum "app"."mood"`, `range "app"."floatrange" of f64` — a type
> written as its schema and name, each double-quoted with its own quotes
> doubled, joined by a dot, or as its quoted name alone.

> [spec:pgorm:req:bindings.model-unique-keys+2]
> A model's `unique_keys` MUST declare its table's unique keys, in order, as
> an entity's `unique(..)` declares them
> (`[spec:pgorm:req:entity.traits.entity-name]`): each key an object of
> `fields`, one field or a non-empty list of them in key order, and the
> options `name`, the constraint's name, which PostgreSQL chooses when none
> is given, and `nulls_not_distinct`, whether nulls are equal under the key.
> A key may be of one field, a model's column having no `unique` of its own.
> A key naming no field, a field the model lacks or one field twice, which
> PostgreSQL refuses (`42701`), and a name pgorm would not take as an
> identifier MUST be a `ConstructionError` raised as the model is declared,
> and a key that is not an object, or names an option it does not know, a
> `TypeError` (`bindings.argument-errors`). Declaring them sends nothing,
> and `as(alias)` keeps them. `describe()` reports each, in order, as its
> `name`, null when none was given, its `fields` as a list and
> `nulls_not_distinct`.
>
> The tables `create_disjoint` writes from models (`bindings.disjoint`) MUST
> each write its model's keys after its primary key, each a `UNIQUE`
> constraint over its fields' SQL columns, as pgorm's `DisjointTable` writes
> the keys `with_unique_key` gives it, which refuses also what a model cannot
> declare: a `VIRTUAL` generated column, which PostgreSQL cannot key
> (`0A000`). A key named as one PostgreSQL would refuse beside the shape's
> other names MUST be a `ConstructionError` raised as the disjoint model is
> declared, the shape's refusal naming both, before anything is sent: as
> another key of its table or its composite primary key, `pk-{table}`
> (`42P07`); as a name the disjoint schema adds (`[spec:pgorm:sem:disjoint.schema]`),
> the base's `UNIQUE` key or another table's composite primary key in its
> schema (`42P07`), or the base's foreign key to a kind's table or a kind
> table's to its base row on its own table (`42710`); as a name PostgreSQL
> gives the shape's tables itself, an unnamed one-column primary key's
> `{table}_pkey` or an unnamed unique key's `{table}_{columns}_key` in its
> schema (`42P07`), or a not-null constraint's `{table}_{column}_not_null`
> on its own table (`42710`), each derived as `disjoint.schema` says; or as
> another table's unique key in its schema (`42P07`). A relation of the
> schema outside the shape is the server's to refuse.

> [spec:pgorm:req:bindings.model-columns+2]
> `col(field)` MUST be a model field's column, qualified as the model's
> columns are. Its comparisons `eq`, `ne`, `lt`, `lte`, `gt` and `gte`, and
> `is_in` and `is_not_in` over a list, MUST convert each value through the
> field's declared kind, an enum's label bound cast to its type as pgorm's
> column comparisons cast through `save_as`. `any(..)` / `all(..)` over a
> list of values MUST range over the values each converted the same way,
> bound as one array of the declared kind, an enum's cast to its array type,
> as pgorm's `eq_any` binds through `save_as`, so an empty list is typed by
> the column too. An expression is taken as written, and so is `any(..)` /
> `all(..)` over a `Select`, over an expression, or beside an array column,
> whose elements are arrays; a `Value` only of the field's own kind; and null
> never, which `is_null()` tests, in a quantifier's list as beside a
> comparison.

> [spec:pgorm:req:bindings.column-operators+3]
> A model's column and a registered entity's column MUST each be an
> expression: every expression method applies to it, reading the column as
> written, and it goes wherever an expression goes. A model column's other
> operators that take a value — `between` and `not_between`,
> `is_distinct_from` and `is_not_distinct_from`, `contains`, `contained_by`
> and `overlaps`, and `strictly_left_of`, `strictly_right_of`,
> `not_extend_right_of`, `not_extend_left_of` and `adjacent_to` — MUST
> convert their values as the column's comparisons do
> (`bindings.model-columns`). A registered column's `between` and
> `not_between`, `SYMMETRIC` or not, its `is_distinct_from` and
> `is_not_distinct_from`, and its `contains`, `contained_by`, `overlaps`
> and range positions MUST be its `ColumnTrait`'s methods of the same
> meaning (`includes` for `contains`): each value converted as its
> comparisons convert one (`bindings.entity-reads`) and compared under the
> same casts, so a bound compared with a `money` column is cast to `money`,
> a range compared with a created range's column is its text cast to the
> type, and a column whose domain is over an enum is tested as the enum,
> where a bound left bare would reach no operator (`42883`) or bind as a type
> it is not. An element `contains` tests is no value of the column's kind,
> and is given as an expression. `any(..)` / `all(..)` over a list beside a
> range operator is the trait's `compare_any` / `compare_all`, as beside a
> comparison. A bound or value that is an expression takes the whole test as
> written, and null is refused. Every other method takes its operands as
> `bindings.expressions` does — a pattern among them, which no column type
> casts — and a registered column's comparisons are those
> `bindings.entity-reads` names. Neither column describes itself: a
> model column's declaration is read through its model's `describe()`
> (`bindings.models`), and a registered column's through its registration's
> (`bindings.entities`).

> [spec:pgorm:req:bindings.model-records+1]
> A model's terminals MUST decode each row into a record keyed by field, in
> declaration order, its values those `bindings.values` declares. Each field
> MUST be found by the result column its statement projected it under, and the
> result's columns MUST be exactly those. Each column's kind MUST be the kind
> its field declares, compared as the binding spells a decoded column's kind
> — a scalar's name, an enum's schema-qualified type, a created range's type
> and subtype, an array's element — so an `int8` read into an `i32` field, or
> another schema's enum, is a `DecodeError` rather than a value of the wrong
> type. A NULL in a field that is not nullable, and a label an enum field does
> not list, MUST be a `DecodeError`, never a value.
>
> A created multirange's field is read from its text, since the driver
> reports a column of one with no subtype (`bindings.created-ranges`): every
> statement a model builds MUST project it cast to `text`, as pgorm's
> `ColumnTrait::select_as` reads an entity's, its result column is held to
> `text`, and its text MUST be read as a `Multirange` of the declared
> subtype, text that subtype does not read being a `DecodeError`.

> [spec:pgorm:req:bindings.model-reads+1]
> `find()` MUST select every field, `select(fields)` the fields it names, and
> `find_by_key(key)` the row of a primary key, through pgorm-query's
> `SelectStatement`. `where` ANDs a condition; `order_by`, `limit` and
> `offset` are the builder's; and `join(relation)` joins a relation's far
> end, from the query's model or a table it joined, without reading its
> columns. `key(values)` MUST match every primary-key field and no other. The
> terminals `all`, `one`, `optional` and `count` run on a pool, a connection
> or a transaction — `one` exactly one row and `optional` at most one, any
> other count an error — and `count` counts as `bindings.pagination` does. A
> model's rows stream from a pool, a connection or a transaction as records,
> through the same decode (`bindings.streams`).

> [spec:pgorm:req:bindings.model-writes+3]
> `insert(values)` MUST give a `ModelInsert` and `insert_many(rows)` a
> `ModelInsertMany`, an insert typed by its row count as Rust's
> `Insert<A, OneRow>` and `Insert<A, ManyRows>` are; `update(values)` a
> `ModelUpdate` and `delete()` a `ModelDelete`, building pgorm-query's
> INSERT, UPDATE and DELETE of a model's table. A field left
> out of `values` MUST stay out of the statement — an insert takes the
> table's default, an update leaves the column as it is — and a field set to
> null MUST be written as SQL NULL, which only a nullable field takes. Each
> value converts through its field's declared kind as a comparison's does, and
> the fields set are written in declaration order whatever order the values
> name them in.
>
> Each write MUST have only its own statement's methods. All of them have
> `returning`, `execute` and `inspect`. An insert also has `on_conflict`,
> `add(values)` and `add_many(rows)`, and its own upsert terminal:
> `returning_upsert` on a `ModelInsert` and `returning_upserts` on a
> `ModelInsertMany`, neither the other's. `add` and `add_many` give the
> `ModelInsertMany` holding its rows and then theirs, its conflict clause
> kept, as Rust's `add` and `add_many` make a batch of any insert. An update
> has `where`, `all_rows()` and the change terminals, and a delete `where`
> and `all_rows()`. A method of another kind is one the write lacks, so a
> type checker refuses the call before anything runs.
>
> An unknown field, an undefined value, a field generated always, an insert
> leaving out a field neither nullable, defaulted nor generated, an update
> setting nothing, and rows of one insert setting different fields MUST be
> `ConstructionError`s, and an insert into an aliased model is refused. An
> insert of no rows sends nothing: `execute` gives 0 and the returning
> terminals no rows. An UPDATE or DELETE MUST have a `where` or `all_rows()`
> before it runs, as `bindings.writes` requires. An insert takes
> `on_conflict` as an INSERT does, an insert of no rows included, as Rust's
> `Insert::many` of none takes it: it still sends nothing, and the rows
> `add` and `add_many` give it later are written under the clause.
>
> `returning(fields)` reads the fields written as records.
> `returning_change` and `returning_changes` on an update, and
> `returning_upsert` and `returning_upserts` on an insert, MUST read each
> written row's two versions as pgorm's `exec_returning_change(s)` and
> `exec_returning_upsert(s)` do: every field of both versions in a
> `RETURNING WITH (OLD AS pgorm_old, NEW AS pgorm_new)` list, the old under
> `o_` and the new under `n_`. A change holds the old and the new record. An
> upsert is inserted, holding the new record, where the old version is NULL in
> every column, and updated, holding both, otherwise. A row an insert's
> conflict clause did not write is null from `returning_upsert` and left out
> of `returning_upserts`.

> [spec:pgorm:req:bindings.model-try-inserts]
> A model insert's `on_conflict_do_nothing()` MUST give the try-insert pgorm's
> `Insert::on_conflict_do_nothing` makes, its statement taking `ON CONFLICT`
> on the primary key's columns `DO NOTHING`, or `ON CONFLICT DO NOTHING` for a
> model with no primary key, and `on_empty_do_nothing()` the try-insert
> `Insert::on_empty_do_nothing` makes, its statement unchanged: a
> `ModelTryInsert` from a `ModelInsert`, a `ModelTryInsertMany` from a
> `ModelInsertMany`, typed by row count as `TryInsert<A, OneRow>` and
> `TryInsert<A, ManyRows>` are. Each keeps `on_conflict`, `add`, `add_many`
> (giving a `ModelTryInsertMany`) and `inspect`, and answers as pgorm's
> `TryInsertResult` does, `inserted` with what it wrote, `conflicted` or
> `empty`: `execute` with the rows written, `conflicted` where a conflict
> clause wrote none; and `returning(fields)`, whose `one` (one row) answers
> with the row's record and whose `all` (a batch) with every written row's,
> `conflicted` where a conflict clause held every row back, and with no
> conflict clause a missing row a `DecodeError`. An insert of no rows, and a
> row that sets no field, MUST be `empty` and send nothing, as `TryInsert`
> answers an all-`NotSet` model. An insert of no rows MUST take a conflict
> clause, as Rust's does, and still send nothing.

> [spec:pgorm:req:bindings.model-write-sources]
> A model update's `from(source)` MUST add a FROM item to its UPDATE, as
> pgorm's `Update::many(..).from(..)` does, and a model delete's
> `using(source)` a USING item to its DELETE, as `Delete::many(..).using(..)`
> does: another model's table, as that model names it, or a `Table` or FROM
> item; anything else is an argument of the wrong type. A write that reads
> another table MUST qualify its RETURNING list's columns by its own table's
> name, so a column of the same name in the other table cannot make them
> ambiguous.

> [spec:pgorm:req:bindings.model-hooks]
> A model MUST take its hooks as functions of the program's own, the dialect of
> Rust's `ActiveModelBehavior`: a `hooks` option of `before_save(values,
> insert)`, `after_save(record, insert)`, `before_delete(values)` and
> `after_delete(values)`, each optional and any other name a `TypeError`, an
> aliased model keeping its model's; `describe()` reports the names of the
> hooks it has under `hooks`. A hook may return an awaitable (a promise in
> JavaScript), which is awaited. The hooks run only around a model's record
> writes, which mirror `ActiveModelTrait`: `active(values)` gives a
> `ModelActive` holding the fields set, whose
>
> - `insert(db)` runs `before_save(values, true)`, inserts the values it gives
>   and reads the row back, then gives `after_save(record, true)`'s record;
> - `update(db)` runs `before_save(values, false)`, updates the row the
>   primary key of the values it gives names, setting their other fields, and
>   reads it back, a key no row has being a `DecodeError` as Rust's
>   `RecordNotUpdated` is an error, then gives `after_save(record, false)`'s
>   record;
> - `delete(db)` runs `before_delete(values)`, deletes the row the primary key
>   of the values it gives names, then runs `after_delete(values)`, giving the
>   rows deleted.
>
> A `before_*` hook that raises or rejects refuses the write: nothing is sent,
> and its error is the caller's unchanged; one that gives no mapping (no
> object in JavaScript) is a `TypeError`. An `after_*` hook's error is the
> write's outcome, the row already written, as Rust's is. An update or delete
> of a model without a primary key, or of values not holding it, is a
> `ConstructionError`. The statement builders — `insert`, `insert_many`,
> `update` and `delete` on the model, their returning and try-insert
> terminals, an update `FROM` and a delete `USING` — run no hook, as Rust's
> `Insert`, `Update` and `Delete` run none.

> [spec:pgorm:req:bindings.relations+3]
> A model's `belongs_to`, `has_one` and `has_many` MUST make a relation to
> another model, pairing fields of each — one, or non-empty lists of equal
> length — in order, and taking pgorm's `on_condition` and `condition_type`:
> `on_condition` a function given the two ends, each the model answering to
> the name the join gives its table (aliased where that is not its own),
> returning a condition; `condition_type` `"all"`, the default, or `"any"`. A
> `belongs_to` relation is the side holding the foreign key, and takes its
> `on_delete` and `on_update` actions — `no_action`, `restrict`, `cascade`,
> `set_null` or `set_default`, spelled in the binding's case — and its
> `fk_name`, as an entity's `belongs_to` does, which
> `bindings.schema-from-model` writes; the relation reports each, null when
> none was given. `has_one` and `has_many` take neither, one given being a
> `TypeError`, as an action outside the set is. A join along it MUST be the condition pgorm's `join_condition` writes for a
> relation: each pair of fields equal and the pairs ANDed, then the
> `on_condition`'s condition, the two composed by AND under `"all"` and by OR
> under `"any"`, between the tables as the join names them. A join starts
> from the first table of the relation's model the query or graph reads, or,
> for the relation `from_alias(alias)` gives, as pgorm's
> `RelationDef::from_alias` does, from the table answering to `alias`; a
> table of another model under that name, or no table, is a
> `ConstructionError`.
>
> `find(row)` MUST read the far end's rows whose paired fields equal the
> row's, and none when one of the row's is NULL; a relation with an
> `on_condition` joins its starting model back, under its own name unless
> the far end has it, and keys it by that model's primary key, as pgorm's
> `find_related` does. `load(db, rows, query)` MUST read the far end of every
> row in one query, as pgorm's loader reads it: `query`, a query of the far
> end's model (its every row when none is given), keeps its conditions,
> ordering, limit and joins, and the starting model is joined back under
> `pgorm_loader_src` as one required source projecting the relation's key
> fields, filtered to the rows' distinct keys, so the relation is read
> whole. Each returned row is filed under the key read back from that source;
> a key no row's key equals is a `DecodeError`. A `has_many` gives a list of
> records per row, in the query's order; a `belongs_to` or `has_one` a record
> per row, null where nothing matched or the row's key holds a NULL, its
> repeats — which joining the starting model back makes — collapsed by the
> far end's primary key, and two distinct rows for one key a `DecodeError`. A
> query of another model is a `ConstructionError`.

> [spec:pgorm:req:bindings.related]
> A model's `related(relation, via)` MUST make the path a Rust `Related`
> implementation with a `via` declares: `via` a relation from the model to a
> junction, and `relation` one from the junction to the far end reaching one
> row. A `via` starting at another model, a `relation` not starting where
> `via` reaches, a `relation` to many rows, and either relation given a
> `from_alias` are `ConstructionError`s. Its `find(row)` MUST be pgorm's
> `find_related` through the junction: the far end's query with the junction
> and the starting model joined back by INNER JOINs, each under its own name
> unless the query has it, keyed by the starting model's primary key, and
> none for a NULL key field. Its `load(db, rows, query)` MUST be pgorm's
> `load_many_via`: the query re-rooted as `bindings.relations` loads, the
> junction joined back from it LEFT as a hop and the starting model joined
> back from the junction as the required source, giving a list of records
> per row in the query's order. A graph's `related_one(path)` and
> `related_maybe(path)` MUST fold a path in as pgorm's `related_one` and
> `related_maybe` do: a relation is `join_one` and `join_maybe`; a related
> path joins its junction as a hop, INNER under `related_one` and LEFT under
> `related_maybe`, and its far end as the slot.

> [spec:pgorm:req:bindings.linked]
> A model's `linked(relations)` MUST make the chain a Rust `Linked` type
> lists: one or more relations, the first starting at the model and each
> other starting where the one before it reaches, any other a
> `ConstructionError`. `find_linked()` MUST read the far end's rows as
> pgorm's `Linked::find_linked` does, walking the chain backwards from the far
> end and joining each relation's starting table by an INNER JOIN under
> `r{i}`, `r0` the last relation's, so a chain may pass through one table more
> than once; `find(row)` narrows it to the row's chain, keyed by the starting
> model's primary key on the last hop's table, as `ModelTrait::find_linked`
> keys it, and reads none for a NULL key field. `hop(i)` MUST be the model the
> query reads at hop `i`, answering to `r{i}`, as `LinkedAlias::hop` names
> it, and `last_hop()` the starting model at the last hop, as
> `last_hop_alias` names it; a hop the chain lacks is a `ConstructionError`.

> [spec:pgorm:req:bindings.graphs+2]
> A model's `graph()` MUST read its rows with the rows its relations reach,
> as pgorm's `SelectGraph` does, its SQL the SQL `SelectGraph` writes for the
> same tables, relations and aliases. The slot kind MUST be the join type and
> the decode shape: `join_one` an INNER JOIN decoded as a record,
> `join_maybe` a LEFT JOIN decoded as a record or null, and `via` a LEFT JOIN
> of a table no record reads. A relation is joined from the first table of
> its model the graph reads, the decoded source its `from` option names, or
> the table its `from_alias` names (`bindings.relations`), a hop's included,
> the two together a `ConstructionError`; a slot or hop answers to its
> table's name or the alias it is given; and a second table answering to a
> name already read MUST be a `ConstructionError`.
> `join_maybe_filtered(relation, condition)` MUST be `join_maybe` with
> `condition` — a function given the two ends as the join names them,
> returning a condition — ANDed into the join's ON behind the relation's
> own, as pgorm's `join_maybe_filtered` adds it, so it narrows which rows
> match and a root none matches keeps its row.
>
> Every decoded source is projected under its own prefix, `s0_` for the root
> and `s{i}_` for the i-th slot, each name composed as pgorm's
> `result_column_name` composes it — under 63 bytes as it stands, otherwise
> bounded with its FNV-1a hash. A graph with no slot decodes as the root's
> record, and a graph with slots as a tuple of the root's and each slot's
> records, an optional slot null exactly where every column it reads is NULL,
> as pgorm's absence witness reads it. A present slot that does not decode
> MUST be a `DecodeError`, never an absent one. `col(source, field)`
> qualifies a field as a decoded source is named. `all_grouped` reads a graph
> of one slot as each root with its slot's records, ordering by the root's
> primary key behind the graph's ordering and grouping by the decoded root's
> key, as pgorm's `all_grouped` does.
>
> `limit` and `offset` cut a graph's read to a window of rows of the join,
> not roots, and leave the graph sliced whatever value they were given, as
> pgorm's `SelectGraph<E, S, Sliced>` is (`query.graph.slice`). A sliced
> graph MUST refuse the reads that decide the row range themselves,
> `all_grouped`, `count`, `cursor`, `cursor_on` and `paginate`, and `inspect`
> of the `count` or `all_grouped` terminal, with a `ConstructionError` naming
> the read and why, the same text in each binding: a window can end part-way
> through a root's rows, a count would ignore it, and a page or a cursor's
> window would replace it. Each binding's type checker refuses them too: the
> graph carries whether it is sliced as a boolean type parameter, `false`
> until `limit` or `offset` returns it `true`, and those methods take only an
> unsliced graph.

> [spec:pgorm:req:bindings.cursors+1]
> `cursor(fields)` on a model query or a graph MUST page by keyset as pgorm's
> `Cursor` does: ordered by its fields, then — on a graph — by the root's
> primary key and each slot's, in declaration order, as tiebreaks, a root key
> field already ordered by not repeated; that ordering replaces the query's.
> A graph's `cursor_on(source, fields)` MUST order by fields of slot
> `source`, the first slot `1`, as pgorm's `cursor_by_on` does: that slot's
> primary key, a field already ordered by not repeated, then the root's, then
> every other slot's in declaration order, are its tiebreaks; `0` and a
> source the graph lacks are `ConstructionError`s. `after` and
> `before` take a boundary of the order fields' values, and `after_with` and
> `before_with` of the order fields' or the whole key's, each converted
> through its field's declared kind and any other arity a
> `ConstructionError`. A boundary of n values MUST be the row-value
> comparison written out as n disjuncts, the k-th holding the first k-1
> columns equal and comparing the k-th: greater past an ascending cursor's
> `after`, less short of its `before`, the reverse when it descends.
> `first(n)` and `last(n)` limit the window, a `last` window read in the
> reversed order and given in the cursor's own, and each replaces the other;
> `asc()` and `desc()` choose the direction.

> [spec:pgorm:req:bindings.pagination]
> `paginate(page_size)` on a model query or a graph MUST read page `n` as
> the query with a limit of `page_size` and an offset of `page_size × n`,
> replacing its own, as pgorm's `Paginator` does; a page size below 1, and an
> offset past what the language holds exactly, are refused. `num_items` and a
> query's `count` MUST count the query's rows with its limit, offset and
> ordering dropped, `SELECT COUNT(*) AS "num_items" FROM (..) AS
> "sub_query"`, and give an exact integer; `num_pages` rounds the pages up;
> and `pages` yields each page from the first until one is empty.

> [spec:pgorm:req:bindings.model-clauses]
> A model query, and a projection of one (`bindings.model-projections`), MUST
> take the clauses of pgorm's entity `Select` beside `where`, `order_by`,
> `limit` and `offset`, each a new query decoding as the one it was called on:
> `distinct()`; `distinct_on(columns..)`, of the queried model or a table it
> joined (`bindings.distinct-on`); `group_by(items..)`, expressions and
> grouping elements as the statement builder's `group_by` takes them, and
> `having(condition)`, ANDed as `where` is; and `lock(strength, wait, of)`,
> the statement builder's row lock (`bindings.row-locks`), which is Rust's
> `lock`, `lock_shared`, `lock_exclusive` and `lock_with_behavior`. A model
> query's records stay the model's: a grouped or distinct query decodes each
> row as its model declares it.
>
> `join(relation)` MUST take an inner, left, right or full join, Rust's
> `join(JoinType, RelationDef)`; a row a right or full join reads with no
> match holds NULL in the queried model's columns, which a field not
> nullable refuses as `bindings.model-records` says. `join_rev(relation)`
> MUST join the table the relation starts at from its far end, which the
> query's model or a table it joined is, on the relation's condition as it
> stands: Rust's `join_rev`, so a relation declared on one model joins from
> the other. A relation whose start (for `join`) or end (for `join_rev`) is
> no table the query reads is a `ConstructionError`.

> [spec:pgorm:req:bindings.model-projections]
> A model query MUST become a projection, Rust's `select_only` and the
> projections the entity `Select` takes: `select_only()` with no items;
> `column(column)`, `columns(columns..)`, `column_as(column, name)` and
> `expr_as(expr, name, declaration)` each add items, called on a model query
> after its fields. A column is a model's column, the queried one's or a
> joined table's, and its item is read under its field's name and decoded as
> its field declares; `column_as` names it, aliased as Rust's `column_as`
> aliases. An expression's item is decoded as its declaration, a `Column` of
> a kind, nullability, array and values and nothing else, says — the type an
> aggregate or a function returns, which Rust states in the type it decodes
> into — and any other option is a `ConstructionError`. Each item is
> projected as a model's field is, a created multirange's cast to `text`
> (`bindings.model-records`). Two items of one name, or of one result
> column, and a projection of no items are `ConstructionError`s.
>
> A projection reads rows once `into_tuple()`, `into_values(names..)` or
> `into_model(..)` says what each becomes — the items' values in order, the
> named items' values in the order named (Rust's `into_values`), or an
> object keyed by item name, made by the binding's own form of a
> `FromQueryResult` target — and a model query takes the three directly, as
> a projection of its fields. Each is a selector, Rust's `Selector`, with
> `all`, `one`, `optional`, `stream`, `count`, `cursor` (by the queried
> model's fields) and `paginate`, as `bindings.model-reads`,
> `bindings.cursors` and `bindings.pagination` state them. A selector MUST
> decode each row as a model's terminals do: the result's columns exactly
> the items', each of its item's declared kind, and a NULL an item is not
> declared nullable for, or a label an enum item does not list, a
> `DecodeError`.

> [spec:pgorm:req:bindings.partial-models]
> A partial model MUST declare a read shape over a model, as Rust's
> `PartialModel` derive does: each field one of the model's columns (Rust's
> `from_col`, a field named for its column being Rust's plain field), a
> `FromExpr` of an expression and the declaration it decodes by (Rust's
> `from_expr`, its declaration as an `expr_as` item's), or a `Skip` holding a
> value (Rust's `skip`), and at least one field not skipped.
> `into_partial_model(shape)` on a model query MUST select what the shape
> declares as Rust's `PartialModelTrait::select_cols` does, the query's other
> clauses kept — a column under the field's name, aliased where that is not
> the column's own; an expression aliased to its field; a skipped field
> nothing — and decode each row into the shape's fields in declaration
> order, a skipped field holding its value, as a selector decodes. A shape
> over another table than the query's is a `ConstructionError`.

> [spec:pgorm:req:bindings.cursor-secondary-order+1]
> A cursor MUST take `add_secondary_order_by(model, field)`, Rust's
> `Cursor::add_secondary_order_by(table, column)`: an ordering by the field of
> the queried model, a table the query joined or a graph's source, the model
> named as the query names its table, its alias included, appended after the
> cursor's key and the entries before it, its boundary values converted
> through the field's declared kind and compared through its column's casts.
> A first argument that is not a model is one of the wrong type, and a model
> the query reads under no such name a `ConstructionError`. `after` and
> `before` still take the key alone, so
> they resume past every row sharing the boundary's key; `after_with` and
> `before_with` take the key or the whole ordering, so a window that ended
> inside a run of one key resumes inside it (`bindings.cursors`).

## Registered entities

A Rust application that already has pgorm entities registers them with a
binding and builds one native module carrying the binding's whole API and its
registrations. The program reaches each registration by name, and every
operation on one is the entity's own Rust API. A binding cannot instantiate a
Rust generic at run time, only use the ones compiled in.

> [spec:pgorm:req:bindings.entities+6]
> Each binding MUST let a downstream application crate register its own
> entities, `SelectGraph` shapes and pipeline source tuples, for any entity
> whose model converts into its ActiveModel, and build one native module from
> them; the binding's own module registers none. The public package MUST NOT
> be limited to this repository's test fixtures. A name or a Rust entity
> registered twice, and a graph or source tuple whose entities are not
> registered, MUST be refused as they are registered. A binding MUST NOT
> fabricate a generic instantiation at run time.
>
> An entity whose names are computed at run time is registered by instance
> too: `Registry::entity_in(name, entity)` registers that entity value, and
> every statement the registration builds — its reads, writes, version
> writes, schema and pipeline source — names the instance's table, as Rust's
> `_in` constructors do (`[spec:pgorm:req:entity.traits.entity-name]`), where
> `entity::<E>(name)` registers `E::default()`. A binding cannot construct a
> Rust value at run time, so an application registers each instance it
> serves, a tenant's under a name of its own, and a program picks the tenant
> by the name it reads. One Rust type may therefore be registered more than
> once, once for each table its instances name; the same type and table twice
> is the duplicate refused above. A graph or source tuple over the type reads
> the metadata of its first registration.
>
> `entity(name)` and `graph(name)` give one registration, and a name the
> module does not register is a `ConstructionError`; `entities()`, `graphs()`
> and `pipeline.source_tuples()` list the names the module registers of each
> kind, in order. `describe()` reports a registration as data, under these
> keys and no other, each spelled by the binding's naming rule, as is a value
> naming a choice: its `name`, and its table's `schema` and `table`;
> `columns`, each column's SQL `name`, the `json_key` its Rust field
> serializes under, its `sql_type`, the `rust_type` its model reads it as,
> `nullable`, `primary_key`, the `kind` its values bind and read as, spelled
> as a model's is (`bindings.models`) and null for a type that takes a
> `Value` — a `money` column's being `decimal`, as the entity reads it cast
> to `numeric` and writes it cast to `money`
> (`[spec:pgorm:sem:entity.traits.column.enum-cast]`) — and an enum's
> `values` (null for any other column);
> `primary_key`, and `primary_key_without_overlaps`, whether that key ends
> `WITHOUT OVERLAPS`; `unique_keys`, the table's unique keys over two columns
> or more as the entity's `EntityName::unique_keys` declares them
> (`[spec:pgorm:req:entity.traits.entity-name]`), each its `name`, null where
> PostgreSQL names the constraint, its `columns` by their SQL names in key
> order and `nulls_not_distinct`; `relations`, each with its `name`, `type` (has one or
> has many), the `from` and `to` tables, each its `schema` and `table`, its
> `columns` as pairs, its `PERIOD` pair as `period`, `condition_type` and
> `on_condition` (`bindings.relation-conditions`), `enforcement` and
> `deferrability`; and `rust`, the Rust `entity`, `model`, `active_model`
> and `column` types behind it. A
> registered entity's records MUST keep the Rust model each was read as.
> Every operation runs as a statement does (`bindings.registered-pools`) —
> refused while another holds the connection, cancellable, the connection
> discarded when cancellation leaves its state unknown — and a failure is the
> error `bindings.errors` classifies.

> [spec:pgorm:req:bindings.relation-conditions]
> A registration's `describe()` MUST report, for each relation, its condition
> type, all or any, and whether an `on_condition` adds a condition of its own,
> the Rust closure reported as present or absent only: both change what a
> join by the relation means.

> [spec:pgorm:req:bindings.registered-pools]
> Every operation on a registration — an entity query's terminals, an
> ActiveModel's writes, the version terminals, a registered graph's
> terminals and a source selection's terminals — MUST run on a pool as well
> as on a connection or a transaction. On a pool it runs on a connection lent
> to it alone, as a pool's statement does.

> [spec:pgorm:req:bindings.entity-reads+4]
> `find()` MUST be the entity's `E::find()`, and `where`, `order_by`,
> `limit` and `offset` the `Select<E>` methods of the same meaning. `all`,
> `one` and `optional` MUST be `Select::all`, `one` and `one_opt`, `one` and
> `optional` narrowing the row limit to one as Rust does, and `one` finding
> no row an error. `inspect()` gives the SQL and values a terminal sends.
>
> `count` MUST be `PaginatorTrait::count`, and `paginate(page_size)`
> `PaginatorTrait::paginate`: a paginator holding no database and no current
> page, whose `fetch_page`, `num_items`, `num_pages` and
> `num_items_and_pages` are `Paginator`'s, whose `pages` yields each page
> from the first until one is empty, and whose `inspect` gives a page's
> statement or the count's; a page size below 1 is refused. `stream` MUST
> read the rows one at a time from a pool's connection of its own or a
> connection, each decoded by `Select<E>`'s own selector as it arrives, as
> `Select::stream` decodes them. `cursor(columns)` MUST be
> `Select::cursor_by` over the columns in order: `before` and `after` take
> one value per column, each converted to its column's declared kind and
> cast as a comparison's value is, any other count refused; `first`, `last`,
> `asc` and `desc` are the cursor's own; and `inspect()` gives
> `Cursor::statement`. `join(relation, kind)` MUST be `QuerySelect::join` of
> the `RelationDef` its relation gives: `relation(name)` on a registration
> names a variant of its entity's `Relation` enum, a name it does not declare
> refused, and the joined rows stay the entity's records. `select(columns)`
> MUST be `select_only` and `columns` over the columns named, each read
> through its `select_as`: its rows are records of those columns as a
> statement's are, and its `one` and `optional` narrow the row limit to one.
>
> `col(name)` is the column as the entity's `ColumnTrait` names it, and its
> comparisons MUST be that trait's methods: a value converted to the column's
> declared kind and written through the column's `save_as` — an enum's label
> cast to its type, and a column whose domain is over an enum compared as the
> enum, the column and the label each cast to it. `any(..)` / `all(..)` over
> a list of values MUST be the trait's `compare_any` / `compare_all`, the
> values converted to the column's declared kind as one array and cast as
> `eq_any`'s is, so a `money` column compares with a list of decimals; over a
> `Select`, over an expression, or beside a column of no scalar kind it is
> compared as written, as is an expression, and null is refused. Its
> `is_in` and `is_not_in` MUST be the trait's `is_in` / `is_not_in` over a
> list of values, each converted as a compared value is, and its
> `in_subquery` / `not_in_subquery` over a `Select`, so a column whose domain
> is over an enum is a member as the enum and the subquery's projection is
> cast beside it. A list holding an expression is a member as written.

> [spec:pgorm:req:bindings.entity-writes+1]
> `active()` MUST be the entity's `ActiveModelBehavior::new`, its defaults
> included, and turning a record into an ActiveModel the real
> `IntoActiveModel` of the model behind the record. An ActiveModel MUST
> report each column as not set, set or unchanged, and `set`, `not_set` and
> `reset` give a new one, a value converted to the column's declared kind or
> refused. Omitted, explicit NULL and assigned values stay distinct. Its
> `insert`, `update` and `delete` MUST be `ActiveModelTrait`'s, the
> application's `ActiveModelBehavior` hooks running around them, and a hook's
> refusal a `ConstructionError`. Setting a value on a record is
> `ModelTrait::set` on a copy, writing nothing, and a record's tagged value
> is the column's value with the kind the entity declares. A record, an
> ActiveModel or a column of another registration, given where this entity's
> is expected, is an argument of the wrong type and MUST be refused with a
> `TypeError` (`bindings.argument-errors`).

> [spec:pgorm:req:bindings.entity-active-changes]
> A registered entity's ActiveModel MUST have `set_if_not_equals(column,
> value)`, a new ActiveModel with the column set unless it holds that value
> unchanged, `ActiveModelTrait::set_if_not_equals` reached by column name, its
> value converted as `set` converts one; and `is_changed()`,
> `ActiveModelTrait::is_changed`, which says whether any column is set, so a
> model read and given back its own values writes nothing.

> [spec:pgorm:req:bindings.entity-versions+3]
> The write terminals that read a row's two versions MUST be the statement
> terminals they are in Rust, so no `ActiveModelBehavior` hook runs around
> them: an update's `returning_change` is `UpdateOne::exec_returning_change`,
> and `update_many()`'s `returning_changes` is
> `UpdateMany::exec_returning_changes`. An insert MUST be typed by its row
> count, as Rust's `Insert<A, OneRow>` and `Insert<A, ManyRows>` are:
> `insert(active)` gives an `EntityInsert`, `Insert::one`'s, whose terminal is
> `returning_upsert`, `Insert::exec_returning_upsert`; and
> `insert_many(actives)` gives an `EntityInsertMany`, `Insert::many`'s, whose
> terminal is `returning_upserts`, `Insert::exec_returning_upserts`. Each
> takes `on_conflict` and has no terminal of the other's. A
> change holds two records, old and new. An upsert is inserted, holding the
> new record, or updated, holding both. A row a conflict clause held back is
> null from `returning_upsert` and left out of `returning_upserts`.
> `update_many().set` writes a value through the column's `save_as` and takes
> an expression as written. An `update_many()` MUST have `where` or
> `all_rows()` before it runs, a `ConstructionError` otherwise, as a
> statement's UPDATE does (`bindings.writes`).

> [spec:pgorm:req:bindings.entity-inserts]
> An entity's insert (`bindings.entity-versions`) MUST have the rest of
> Rust's insert terminals of its row count, statement terminals as the
> upserts are, so no `ActiveModelBehavior` hook runs around them. Both kinds
> have `execute`, `Insert::exec`, giving the rows written. An `EntityInsert`
> has `returning_pk` and `returning_model`, `exec_returning_pk` and
> `exec_returning_model`, and an `EntityInsertMany` has `returning_pks` and
> `returning_models`, `exec_returning_pks` and `exec_returning_models`, each
> row's in the order the database wrote them. A key is as Rust's
> `PrimaryKey::ValueType` holds it: the value of a key of one column, and the
> values of a composite key's in its columns' order. A row a conflict clause
> held back is a `DecodeError` from `returning_pk` and `returning_model`, and
> left out of `returning_pks` and `returning_models`; a batch of none sends
> nothing. Both kinds have `add(active)` and `add_many(actives)`, which give
> the `EntityInsertMany` of its ActiveModels and then theirs, its conflict
> clause kept, as Rust's `add` and `add_many` make a batch of any insert,
> and refuse an ActiveModel of another registration as `bindings.entity-writes`
> does. The ActiveModels of one insert MUST set the same columns: a batch
> whose ActiveModels set different ones is a `ConstructionError` as it is
> given, where Rust's `Insert` refuses it before anything is sent.

> [spec:pgorm:req:bindings.entity-graphs+4]
> `graph(name).find(aliases)` MUST build the registered `SelectGraph` with the
> application's factory, its declared entity tuple, join slot kinds and
> decoding terminals, each joined slot under the alias given for it, `g1`,
> `g2`, .. by default; an alias count other than the slots', an alias
> repeated, or one naming the root's table is refused. Its rows MUST be
> decoded by `GraphRow`: a graph with no slot as the root's record, otherwise
> a tuple of each source's record, an optional slot null where it matched
> nothing, and a present slot that does not decode a `DecodeError`. Required
> rows, optional rows, absent joined sources and decode errors stay distinct.
> `col(source, column)` qualifies a column of a decoded source as the query
> names it; `where` and `order_by` are `SelectGraph`'s; `all` and `optional`
> its terminals, `SelectGraph::all` and `one_opt`. An entity combination or
> arity the module does not register is refused, never lowered to a different
> API.
>
> `cursor(columns)` MUST be `SelectGraph::cursor_by` on one to six root
> columns, and `cursor_on(source, columns)` `SelectGraph::cursor_by_on` on
> one to six columns of slot `source`, the first slot being 1: `before` and
> `after` bound the order columns, and `before_with` and `after_with` the
> order columns or the whole key — the order columns, the ordered source's
> other key columns, then each other source's in source order, the root's
> first — each value converted to its column's declared kind and any other
> count refused. `first`, `last`, `asc` and `desc` are the cursor's own, and
> `inspect()` gives `Cursor::statement`. `stream` MUST read the rows one at a
> time as `bindings.entity-reads` streams an entity's, each decoded by
> `GraphRow` as `SelectGraph::stream` decodes it; `all_grouped` MUST be
> `SelectGraph::all_grouped`, which only a graph of one `Opt` slot has, any
> other shape refused; and `count` and `paginate` are `PaginatorTrait`'s, a
> page boundary falling between rows of the join.
>
> `describe()` MUST report a registered graph as data, under these keys and
> no other, each spelled by the binding's naming rule: its `name`;
> `rust_shape`, the Rust `SelectGraph` type its factory builds; and
> `sources`, the root and then each slot in order, each with its `index`, the
> `entity` it decodes as and its `slot`, `root` or the slot's kind, `Req` or
> `Opt`.

> [spec:pgorm:req:bindings.pipeline-sources+3]
> A registered entity MUST be a pipeline source wherever a relation is, read
> through its own `IntoSource`, so its table and schema are the ones its Rust
> declaration names, and `pipeline.col(table, entity.col(name))` MUST lower
> through the registration's own `Named::col`, so the column compares as the
> entity types it. A module registers source tuples of one to six entity
> types, each entity registered first. `pipeline.sources(name)` gives one, and
> another name is a `ConstructionError`.
>
> `select_sources(selection, qualifiers)` MUST be pgorm's `select_sources` as
> the pipeline's last stage, each source projected under the qualifier given
> for it or its table's name, a count of qualifiers other than the sources'
> refused. Its `all`, `one` and `optional` are `SelectedSources`'s `all`,
> `one` and `one_opt`, `one` and `optional` taking one row. Each row is a
> tuple of the sources' records, decoded through each entity's absence
> witness, a source a row does not carry being null. A pipeline reshaped
> before the selection is the `ConstructionError` pgorm's refusal names,
> before anything is sent.
>
> A source tuple's `describe()` MUST report it as data, under these keys and
> no other, each spelled by the binding's naming rule: its `name`;
> `rust_shape`, the Rust tuple of entity types it registers; and `entities`,
> the name of each source's entity registration, in order.

> [spec:pgorm:req:bindings.codegen]
> Each binding MUST offer a documented workflow that generates, from a
> description of a downstream application's entities, graph shapes and
> source tuples, a crate registering exactly what is described and a module of
> typed wrappers over it (`python.codegen`, `napi.codegen-types`). Generation
> MUST preserve qualified names, column mappings, enum types and optional
> decode shapes, and MUST refuse, before anything is written, a description
> whose Rust paths are not paths to items of the crate, whose names repeat
> within a kind or collide, whose entity is registered twice, or whose source
> tuple lists an entity it does not describe. The output is the same bytes
> each time from the same build. Compilation happens when the definitions or
> the shapes change, not when a program builds another query over the
> installed types. The generated module MUST check, as it loads, that the
> native module and every registration it names describe as they did when it
> was written, and refuse to load otherwise.

> [spec:pgorm:req:bindings.codegen-layout]
> The module each binding's codegen writes (`bindings.codegen`), and its
> declarations, MUST read as a person writes the language, laid out as the
> language's usual formatter lays it out, so nothing needs to run over it
> once it is written, as pgorm-codegen's Rust needs no `rustfmt`
> (`[spec:pgorm:req:codegen.entity.files]`). Python is black's layout:
> strings double-quoted; a bracket's contents on its line where they fit in
> 88 columns, on a line of their own where those fit, and otherwise one to a
> line, each followed by a comma; a conditional expression too long for its
> line split before its `if` and its `else`; a long union a member to a
> line inside parentheses; two blank lines around each top-level class and
> function and one between a class's methods, the stub in black's `--pyi`
> style; and the imports the module uses and no others, grouped and sorted
> as isort sorts them. The compatibility data the module checks as it loads
> is a literal laid out the same way, not one line. TypeScript and
> JavaScript are deno fmt's and prettier's layout: a bracket's items on its
> line where they fit in 80 columns and otherwise one to a line, each
> followed by a comma; an object type a member to a line where it does not
> fit; a long union a member to a line, each led by its `|`; and each doc
> comment wrapped to the width.

> [spec:pgorm:req:bindings.codegen-disjoint]
> The codegen workflow (`bindings.codegen`) MUST carry an application's
> `#[derive(Disjoint)]` enums beside its entities: the description names
> each by its Rust path, its registration name and its export, a name or a
> Rust path repeated, and an export colliding with another, refused before
> anything is written as an entity's are; the generated crate registers each
> after the entities (`bindings.disjoint`); and generation refuses, before it
> writes the module, an enum whose base or kind entity the description does
> not export. The typed wrapper of a registration MUST type its rows as a
> union over its kinds, one alternative per kind holding the base's record,
> the kind's value as a literal and the kind's record, or null for a kind
> without a table, each record the generated type of that entity, so a
> checker narrows a row's record by its kind; a kinded enum's `insert`,
> `update` and `change_kind` take the base's and each kind's generated
> ActiveModel and give the written row as a read does, and an unkinded enum's
> wrapper offers no write and no schema, as `bindings.disjoint-unkinded`
> types it. The generated module's load-time check MUST compare every
> registered enum's `describe()` with the one it was written from, as it
> compares each entity's.

## Disjoint models

> [spec:pgorm:req:bindings.disjoint+2]
> `disjoint(name, base, kinds, options)` MUST declare a disjoint model over a
> base model (`bindings.models`), where `kinds` maps each kind's value, in
> order, to the model of its table or to null for a kind with none. The
> options are named as `[spec:pgorm:def:disjoint.declare]`'s attributes are
> and default as they do: `kind`, the base's kind field, `kind` by default;
> and the kind type's `enum_name`, `{name}_kind` by default, and
> `schema_name`, none by default. A model's enum column names its schema
> (`bindings.models`), so a base's kind field is of the kind type only when
> `schema_name` names that schema. A downstream crate's `#[derive(Disjoint)]`
> enum MUST be registrable as an entity is (`bindings.entities`), its base
> and kind entities registered before it, and `disjoint_entity(name)` and
> `disjoint_entities()` give one registration and list them. Declaring or
> registering sends nothing. What the Rust declaration refuses — no kinds, a
> model given for two kinds, a kind model whose key differs from the base's,
> a base whose kind field is missing, nullable, generated or of another
> type, a kind model whose key the database generates or fills — MUST be a
> `ConstructionError` raised as the model is declared or the enum
> registered, as is a base or kind model read under an alias, since a
> disjoint statement names each table bare.
>
> Either is pgorm's `DisjointShape`: described from the models, each column
> of the type its kind is written with (`ArrayType::column_type`) and cast as
> a column of that type is, or `DisjointShape::of` the registered enum. Every
> statement is the shape's — the read and its narrowing by key and kind, its
> cursor and pages, the writes and the schema — so reads, writes and the
> schema MUST be Rust's (`[spec:pgorm:sem:disjoint.read]`,
> `[spec:pgorm:sem:disjoint.write]`, `[spec:pgorm:sem:disjoint.schema]`):
> the same statements, the same refusals, the same errors as
> `bindings.errors` classifies them, a refusal before anything is sent of a
> value that cannot become what pgorm sends being a `ConstructionError`, and
> a row the constraints do not admit and a write that reads back no row
> (`RecordNotFound`) a `DecodeError`. An ActiveModel of another entity given
> for the base or a kind is an argument of the wrong type, a `TypeError`, as
> `bindings.entity-writes` refuses one. A read gives
> each row as a triple — a tuple in Python, an array in JavaScript — of the
> base's record, the kind's value, and the kind's record or null: a model's
> records, an enum column the read projects as its text read back as its
> enum, or the registered entities' own. Its terminals are `all`, `one` and
> `stream`, beside `count`, `paginate` and `cursor`. `insert`, `update` and
> `change_kind` take the base's values and a kind with its values — a
> model's as fields, a registration's as its entities' ActiveModels — and
> their one terminal, `returning_model`, gives the written row as a read
> does. `create_disjoint(model)` gives the shape's statements as DDL: a
> model's `generated` column is written as an identity, and a model column
> whose default the declaration does not state (`default`) has no `DEFAULT`
> to write, so a table holding one is refused with a `ConstructionError`.
> `describe()` MUST report a disjoint model or registration under these keys
> and no other, each spelled by the binding's naming rule: `name`; `base`,
> the base model's table name or the base entity's registration name;
> `kind_field`; `kind_type`, the kind type's `schema` and `name`; and
> `kinds`, each with its `value` and its `model`, the kind model's table name
> or the kind entity's registration name, or null.

> [spec:pgorm:req:bindings.disjoint-unkinded]
> `disjoint(name, base, kinds, {unkinded: true})` MUST declare an unkinded
> disjoint model (`[spec:pgorm:sem:disjoint.unkinded]`) over a base model
> with no kind field, `kinds` mapping each kind's value, in order, to the
> model of its table or to null for the kind of a base row with a row in no
> kind's table. It is pgorm's `DisjointShape::unkinded`, and what that
> refuses — no kinds, a value given twice, one model for two kinds or for a
> kind and the base, a kind model keyed by other types than the base, a
> second kind with no model, no kind with one — MUST be a
> `ConstructionError` raised as the model is declared, as are `kind`,
> `enum_name` and `schema_name` given beside `unkinded`, there being no kind
> field and no kind type to name. A downstream crate's `#[derive(Disjoint)]`
> enum declared `unkinded` MUST register as a kinded one does
> (`bindings.disjoint`), its base and kind entities registered before it,
> through the same registration, which takes any `Disjoint`.
>
> Either reads as `[spec:pgorm:sem:disjoint.unkinded]` reads: every statement
> is the shape's, and each row is the triple `bindings.disjoint` gives, its
> kind the one whose row is present — a declared kind's value as `kinds`
> gives it, a registered one's its variant's name in snake_case, as
> `DisjointShape::kinds` names it. A base row with rows in two or more kinds'
> tables, and one with a row in none where every kind has a model, MUST be a
> `DecodeError` naming the base, the key and the kinds, never a row read as
> one of its kinds or dropped. Nothing is written through one: each binding's
> types MUST offer no `insert`, `update` or `change_kind` and no
> `create_disjoint` for an unkinded model, so that a type checker refuses
> each call as rustc refuses it, and a call made regardless, on a model or a
> registration, is refused before anything is sent with a
> `ConstructionError` naming the base as read-only, the shape's refusal. Its `describe()` has `bindings.disjoint`'s keys, its
> `kind_field` null and its `kind_type`'s `schema` and `name` null.

> [spec:pgorm:req:bindings.disjoint-evolve]
> Each binding MUST write a change to a kinded disjoint model's or
> registration's kinds as pgorm's `DisjointShape` writes it
> (`[spec:pgorm:sem:disjoint.evolve]`), from the declaration as it will be,
> through five functions beside `create_disjoint`, each taking the disjoint
> model or registration and a kind's value as `kinds` gives it:
> `add_disjoint_kind` (`DisjointShape::add_kind`),
> `add_disjoint_kind_recreating` (`add_kind_recreating`),
> `remove_disjoint_kind` (`remove_kind`), `add_disjoint_kind_table`
> (`add_kind_table`) and `remove_disjoint_kind_table` (`remove_kind_table`),
> the last also taking the table dropped, named as `bindings.schema` names a
> table. Each gives a `DisjointChange`, whose `steps` are, in the order they
> run, each a `DisjointStep` of its `statements`, the binding's DDL
> statements in the order they run in one transaction committed before the
> next step, and its `gap`: null, or the `DisjointGap` naming the `kind` and
> the `table`, a `Table` schema-qualified as the kind's table is, in which
> the program writes a row for every base row of that kind before the next
> step runs. They are data, Python's frozen classes of those names and
> JavaScript's frozen plain objects, as each binding's other results are
> (`python.records`, `napi.records`), and calling a function sends nothing.
>
> What the shape refuses — what `create_disjoint` refuses, a kind added or
> given a table that the declaration does not declare or declares without
> one, a kind removed that it still declares, and a table dropped from a
> kind it declares with one — MUST be a `ConstructionError` raised as the
> function is called, and a value that is not text, or a first argument
> that is no disjoint model or registration, a `TypeError`. An unkinded
> model or registration has no kind type to change: each binding's types
> MUST take none, as they take none for `create_disjoint`, and a call made
> regardless is refused with the `ConstructionError` naming the base as
> read-only (`bindings.disjoint-unkinded`).

> [spec:pgorm:req:bindings.migration-file]
> Each binding MUST freeze generated DDL as pgorm freezes it
> (`[spec:pgorm:def:migration.frozen]`), the text and the file written by
> pgorm's `MigrationFile` alone: `migration_file(name, statements,
> commits_after)` freezes the binding's DDL statements, `commits_after`
> false unless given; `migration_files(change, name)` freezes a
> `DisjointChange` a kind-change function of `bindings.disjoint-evolve`
> returned as `DisjointChange::migration_files` does
> (`[spec:pgorm:sem:disjoint.evolve.frozen]`), a `TypeError` for anything
> else, a change built by hand included, since only one of those carries the
> change pgorm wrote. Each gives `MigrationFile`s, instances made by those
> functions alone, each reading its `name`, its `text` and whether it
> `commits_after`, and `freeze(directory)` writing `<name>.sql` where no file
> of that name is and answering whether it did, never rewriting one. A name
> that is no plain file stem MUST be a `ConstructionError`, and any other
> failure to write the file the language's own I/O error. A binding has no
> migration runner: each file is one migration for a tool that runs a
> directory of SQL files, each in its own transaction, or for
> pgorm-migration's `sql_migration!`.

> [spec:pgorm:req:bindings.one-of+1]
> Each binding MUST read, set and load a reference to exactly one of several
> targets (`[spec:pgorm:def:disjoint.one-of]`) and write its `CHECK` through
> pgorm's `OneOfShape`, the reference as data, as `DisjointShape` is a
> disjoint entity's (`bindings.disjoint`): described from models, or
> `OneOfShape::of` a downstream crate's `#[derive(OneOf)]` enum. Every
> statement and every refusal is the shape's, so a binding's reads, loads
> and `CHECK` are Rust's.
>
> `one_of(name, model, targets, options)` MUST declare one over a model,
> `targets` mapping each target's value, in order, to the field holding its
> key and the target's model, keyed by one field, and the option `optional`
> declaring that a row may reference none, as a unit variant does. A
> downstream crate's enum MUST be registrable as an entity is
> (`bindings.entities`), its entity and its targets' entities registered
> before it, and `one_of_entity(name)` and `one_of_entities()` give one
> registration and list them, its values its variants' names in snake_case.
> What the derive refuses — a field the model lacks, one that is not
> nullable, a field named twice, one whose kind is not its target's key's, a
> target keyed by more than one field, one target model given twice, no
> target, and a name past 59 bytes — MUST be a `ConstructionError` raised as
> the reference is declared or registered, and declaring or registering one
> sends nothing.
>
> `key(record)` MUST read a record's reference as a tagged key — Python's
> tuple `(value, key)`, JavaScript's array `[value, key]` — or null for a row
> referencing none where the reference is optional; a row setting two of its
> fields, or none where it is not optional, is a `DecodeError` naming the
> table, the row's key and the fields, `OneOf::key`'s refusal. A setting
> MUST be the shape's: the value's field set to the key and every other
> field to null, or every field null for null where the reference is
> optional. A model's reference's `set(value, key)` gives those fields and
> their values, which its `insert` and `update` take beside their other
> values, and a registration's `set(active, value, key)` gives its entity's
> ActiveModel with them set; a value that names no target, a target without
> its key, and null where the reference is not optional are each a
> `ConstructionError`. `load(db, records)` MUST read each target's table
> once, for the keys the records hold for it, and give each record its
> target's record, tagged as a key is, in the records' order; a key its
> target's table does not hold is a `DecodeError` naming the table, the
> row's key, the target's key and the field. `create_one_of(one_of)` gives
> the `CHECK` `Schema::create_one_of` writes, an `AlterTable`. `describe()`
> MUST report a declaration or registration under these keys and no other,
> each spelled by the binding's naming rule: `name`; `model`, the model's
> table name or the entity's registration name; `targets`, each with its
> `value`, its `field` and its `target`, the target model's table name or
> the target entity's registration name; and `optional`.

## Pipelines

Each binding composes pgorm's PRQL-shaped pipeline, `pgorm::pipeline`:
relation-to-relation stages over sources, compiled through prqlc to
PostgreSQL SQL, with runtime values entering through a binder whose
placeholders belong to the stage that minted them.

> [spec:pgorm:req:bindings.pipeline+1]
> The binding's `pipeline` module MUST compose `pgorm::pipeline::Pipeline`,
> each object owning the pgorm state it stands for and each stage returning a
> new pipeline from a copy: `from(source)` and the stages `filter`, `derive`,
> `select`, `group` followed by `aggregate`, `window`, `sort`, `take(n)`,
> `take_range(start, end)`, `join(source, on, kind)`, `append`, `intersect`,
> `remove` and `distinct`. A source is a `Table`, schema-qualified and read
> under its alias when it has one, another `Pipeline` embedded whole with its
> bound values, a declared model (`bindings.pipeline-models`), a registered
> entity (`bindings.pipeline-sources`), or `source(relation).named(name)`,
> which reads a relation under a name of its own. A grouping is no pipeline until it
> is aggregated, so `group` gives a `Grouped` whose only way back is
> `aggregate`, and which no terminal runs. A row count is an integer, never an
> expression, as PRQL refuses a bound LIMIT.
>
> A `Pipeline` is a statement for the terminals (`bindings.results`),
> compiled through `Pipeline::into_sql` as `inspect()` compiles it, its values
> bound as any statement's are, `one` and `optional` keeping their
> cardinality. What pgorm's compile step judges — a name it cannot write as
> one, a reserved alias, a relation or column prqlc cannot resolve — MUST be a
> `ConstructionError` from `inspect()` or the terminal, before anything is
> sent.

> [spec:pgorm:req:bindings.pipeline-terminals]
> A `Pipeline`'s own terminals MUST be pgorm's: `all`, `one` and `optional`
> are `Pipeline::all`, `one` and `one_opt`, each row a record as the
> statement terminals decode one (`bindings.results`). `one` and `optional`
> append a `take 1` stage before compiling, as Rust's do, so at most one row
> leaves the server, `one` finding none a `DecodeError` (`bindings.row-count`)
> and `optional` null. Each runs on a pool, a connection or a transaction, on
> a pool on a connection lent to it alone, as a pool's statement does.
> `inspect(terminal)` gives the SQL and values `all`, `one` or `optional`
> sends, `all`'s by default. Given to the statement terminals instead, a
> pipeline runs as Rust runs the `(String, Values)` its `into_sql` gives,
> through `ConnectionTrait`: as compiled, `one` and `optional` keeping their
> cardinality (`bindings.pipeline`).

> [spec:pgorm:req:bindings.pipeline-models]
> A declared model MUST be a pipeline source wherever a relation is, read as
> its table, schema-qualified, under the model's alias when it has one, as a
> `Table` is read. `col(model, field)` MUST be the field's column, qualified
> by the model's alias or its table's name and written under the column's
> SQL name, typed by its declaration through `pgorm::pipeline::col_typed`
> (`pipeline.described`), so it compares as an entity column of its kind. A
> value compared with it — in a comparison, `coalesce` or `in_array`, and not
> a pipeline expression, which is taken as written — MUST convert through the
> field's declaration as the model column's own comparisons convert one
> (`bindings.model-columns`), null and a label the declaration does not list
> being `ConstructionError`s before anything is sent, and is bound as the
> value it holds, the column's cast typing it: an enum's label cast to its
> enum, a created range's text to its type. Only the column itself converts;
> what an operator makes of it is an expression of its own. A field the model
> does not declare is a `ConstructionError`.
>
> `select_sources(models)` — one model or more, each restating a relation the
> pipeline read: a model for one read under its table's name, the aliased
> model for one read under its alias — MUST be pgorm's `select_described` as
> the pipeline's last stage, each model's fields projected in declaration
> order, the i-th under the `s{i}_` prefix, each read as the model's own
> statements read it (`bindings.model-records`). `one` and `optional` take one
> row before the projection, as `bindings.pipeline-terminals`'s do, and each
> row MUST be a tuple with one record per model, decoded as a graph's
> optional slot is (`bindings.graphs`): null exactly where every column the
> model reads is NULL, as a right or full join leaves a side absent, and a
> present side that does not decode a `DecodeError`. A pipeline reshaped
> before the selection is the `ConstructionError` pgorm's `ReshapedSources`
> names, before anything is sent. Registered entities select as
> `bindings.pipeline-sources` says, and the two do not mix in one selection.
>
> `into_model(target)` and `into_tuple()` MUST stage a read of the pipeline's
> rows, as Rust's `into_model::<M>` and `into_tuple::<T>` do: a declared
> model's records, each field read from the result column of its SQL name,
> the result's columns exactly the model's and each held to its declaration
> (`bindings.model-records`); a function's results, the function called with
> each row's columns by name, as keyword arguments in Python and as one
> object in JavaScript; and a tuple of each row's values in column order,
> read by position, so a result naming one column twice is read. Rust's
> `all::<M>()` is `into_model(M).all(db)`. Each of the three reads, `select_sources` over
> models, `into_model` and `into_tuple`, MUST give a `PipelineSelector`,
> whose `all`, `one` and `optional` run on a pool, a connection or a
> transaction with `bindings.pipeline-terminals`'s cardinality, and whose
> `inspect(terminal)` gives the SQL and values each sends.

> [spec:pgorm:req:bindings.pipeline-expressions+3]
> Pipeline expressions MUST be objects of their own, lowered into
> `pgorm::pipeline::Expr` only as a stage takes them, never mixed with the
> statement builders' expressions: `col(table, column)`, qualified as prqlc
> requires; `alias(name)`, a name a stage introduces, read back unqualified;
> `this(name)` and `that(name)`, a join's two sides; the operators `eq` ..
> `lte`, and, or, not, negation, `add` .. `rem`, `coalesce`, `is_null`,
> `is_not_null`, `in_array`, `cast` over pgorm's closed `CastType` set, `as`,
> `asc` and `desc`; `compares_as(type)`, a name or a `TypeName`, lowered
> through `ExprOps::compares_as`, the type a column no registration types
> compares as — the enum a domain is over, `money` — the column and each
> value compared with it cast to it (`pipeline.surface`); `case(arms,
> otherwise)`;
> and the aggregates and window functions pgorm's pipeline has, each at the
> argument count it takes. `over()` builds a window's partition, ordering
> and ROWS or RANGE frame, and takes no value, bound or not, as pgorm's `Over`
> takes none.
>
> `literal(value)` — null, a boolean, an integer within `bigint`, a finite
> float, or text — is written into the SQL as pgorm's pipeline writes a
> literal. A value bound through a stage's binder (`bindings.pipeline-binder`)
> is a placeholder, inferred or declared with `Value` as a parameter is, an
> interval included. A value with no kind and a value whose kind needs a cast
> pgorm's pipeline cannot write (an enum's or a created range's) MUST be a
> `ConstructionError` naming the explicit form. A plain operand is bound by
> the stage that takes it (`bindings.operands`).

> [spec:pgorm:req:bindings.pipeline-binder]
> Each expression-taking stage MUST have a binder form — `filter_with`,
> `derive_with`, `select_with`, `group_with`, `aggregate_with`,
> `window_with`, `sort_with` and `join_with` — that calls its function once,
> synchronously, with a `Binder`, whose `bind(value)` mints one placeholder
> for one value, reusable within the stage. A placeholder is branded with the
> scope of the call that minted it, as pgorm's binder brands it with a
> lifetime, and the binding's ownership MUST NOT erase that discipline. Once
> the function has returned or failed, the binder MUST refuse to bind, and an
> expression carrying its placeholder MUST be refused, each with a
> `LifecycleError`, as any handle used after it closed is. So MUST an
> expression that combines two scopes' placeholders, and one given to a stage
> other than the one its function returns it to — a plain stage, another
> call's or another pipeline's — or to a window's partition or ordering.
>
> The function MUST return its result directly, never through the language's
> asynchronous form: an operand for a filter or a join condition, and an
> expression or a list of operands for a list-taking stage; anything else
> there is refused. A list of more than 32 is a `ConstructionError`:
> pgorm's list-taking `_with` stages take a fixed-size array, which the
> binding dispatches up to that bound, where a stage that binds nothing takes
> a list of any length.

## Schema

> [spec:pgorm:req:bindings.schema]
> Each binding MUST build DDL through pgorm-query's DDL builders, each object
> owning the builder state it stands for, immutable as a statement builder is:
> a method gives a new builder from a copy, and an argument it cannot use is
> refused as it is given, an option the builder does not know among them, so
> that a misspelt option is never left out of a statement. Constructing one
> does no I/O, and nothing runs DDL but a terminal: declaring a model or
> importing a module MUST NOT execute DDL.
>
> Every complete DDL statement is the binding's DDL class, a statement the
> terminals take; only the binding's DDL constructors make one. A schema is a
> database schema alone (`bindings.schema-schemas`), as it is in pgorm-query
> (`sql.ddl`). PostgreSQL takes no parameters in DDL, so a DDL statement MUST
> render through pgorm-query's own rendering, which writes every value it
> carries — a DEFAULT, a CHECK's operands, an enum label, a comment, an
> extension's version, a view's query — as an escaped literal, and MUST run
> through the terminals as that SQL text with no values beside it; `inspect()`
> gives the same SQL and an empty value list. No name or value is
> concatenated into SQL by the binding: every identifier is minted with
> `Name::runtime` and quoted where pgorm-query writes it. A table or sequence
> is named by text or by a `Table`, whose schema qualifies it and which MUST
> NOT carry an alias; a type by text or a `TypeName`, keeping its schema
> qualification, quoting and type identity.
>
> pgorm-query's typestates hold: an `ALTER` that names its object and no
> action yet MUST have no `inspect()`, every terminal MUST refuse it with a
> `ConstructionError`, and its first action gives the statement. A
> `ColumnDef` is no statement, and a terminal refuses it likewise.

> [spec:pgorm:req:bindings.schema-api+3]
> Each binding MUST spell the DDL `bindings.schema` describes as these
> functions and the builders they give, every complete statement of the `DDL`
> class or a class extending it, so a statement the terminals take; no class
> is named `SchemaStatement`. A statement is made by its factory, never by its
> class: `DDL`, each builder, each pending `ALTER` and `EntitySchema` are
> exported for annotations and `isinstance` (`instanceof`) and cannot be
> called, a call being a `TypeError`, as Python's `Expr` is made only by
> `col()` and `bind()`. Python's stubs give each such class an `__init__`
> taking `Never`, and JavaScript's declarations a private or protected
> constructor. A `ColumnDef` and a `DataType` are parts of a statement, not
> statements, and are made by calling their classes. The factories are:
>
> - `create_table(table)` gives a `CreateTable` with `column`, `primary_key`,
>   `unique`, `foreign_key`, `check` and `if_not_exists`, and
>   `ColumnDef(name, type)` a column, its type left out for a `modify_column`
>   that does not retype it;
> - `alter_table(table)` gives a `PendingAlterTable`, whose first action gives
>   an `AlterTable` that takes more; `drop_table`, `rename_table`,
>   `rename_column`, `rename_constraint` and `truncate_table` give
>   statements;
> - `create_index(table, entry)`, with the option `name`, gives a
>   `CreateIndex` with `column`, `unique`, `nulls_not_distinct`,
>   `if_not_exists`, `using`, `include` and `where`, an entry being a name or
>   an expression with the options `order` and `operator_class`, which
>   JavaScript gives as one object `{ on, order, operatorClass }`;
>   `drop_index(table, name)` gives a statement;
> - `create_type(name)` gives a `CreateType` with `as_enum`, `values`,
>   `as_composite`, `attribute` and `as_range`; `alter_type(name)` a
>   `PendingAlterType` with `add_value`, `rename_to`, `rename_value`,
>   `rename_attribute` (a `RenameAttribute`) and the composite's
>   `add_attribute`, `drop_attribute` and `alter_attribute` (an
>   `AlterComposite`); and `drop_type(names)`;
> - `create_sequence(name)` gives a `CreateSequence` and
>   `alter_sequence(name)` a `PendingAlterSequence`, whose first clause gives
>   an `AlterSequence`; `options` and an identity take the sequence options
>   as options, and `owned_by` with no table is `OWNED BY NONE`;
>   `drop_sequence`, `rename_sequence`, `create_extension(name)`,
>   `drop_extension(name)`, `comment_on_table` and `comment_on_column` give
>   statements;
> - `create_view(name, query)` gives a `CreateView` with `or_replace`,
>   `columns`, `check_option`, `security_barrier` and `security_invoker`;
>   `create_recursive_view(name, columns, query)` a `CreateRecursiveView`
>   with the same methods but `check_option`;
>   `create_materialized_view(name, query)` and `create_table_as(name,
>   query)` builders with `if_not_exists`, `columns` and `with_no_data`; and
>   `refresh_materialized_view(name)`, `drop_view(names)` and
>   `drop_materialized_view(names)` statements;
> - `create_schema(name)`, `drop_schema(names)` and `rename_schema(name,
>   new_name)` give statements;
> - `create_domain(name, base)` gives a `CreateDomain`; `alter_domain(name)` a
>   `PendingAlterDomain` whose actions `set_default`, `drop_default`,
>   `set_not_null`, `drop_not_null`, `add_check`, `drop_constraint`,
>   `rename_constraint`, `validate_constraint` and `rename_to` each give a
>   statement; and `drop_domain(names)`;
> - `from_entity(entity)` gives an `EntitySchema` of the table, enum, index
>   and comment statements (`bindings.schema-from-entity`), its enums each a
>   `CreateType`, and `from_model(model, relations)` one of a declared
>   model's (`bindings.schema-from-model`).
>
> Each statement's further options are those the rules below give it, as
> options. A list of names is one name or a non-empty list, an empty list a
> `ConstructionError`; a behavior is `"cascade"` or `"restrict"`; a collation
> a name or `{ name, schema }`; and every other closed choice a string
> spelled in the binding's case, `"not_enforced"` in Python and
> `"notEnforced"` in JavaScript. Python's capability manifest lists each
> function and constructor as a `schema.` operation needing no registration
> but `from_entity`'s (`python.capabilities`), `from_model` among them.

> [spec:pgorm:req:bindings.schema-from-entity]
> The schema of a registered entity MUST be generated by pgorm's own entity
> schema path, `Schema::create_table_from_entity` with its enum, index and
> comment statements, and offered as DDL statements. When that path refuses
> the entity's table, as it refuses a foreign key PostgreSQL cannot build
> (`schema.from-entity`), the binding MUST raise a `ConstructionError`
> carrying the Rust message, and MUST NOT give a schema that leaves the key
> out.

> [spec:pgorm:req:bindings.schema-from-model]
> The schema of a declared model MUST be written by pgorm's own `Schema`
> from the `TableShape` the model describes
> (`[spec:pgorm:sem:schema.from-shape]`), the statements
> `create_table_from_entity` and its siblings write for an entity of the same
> declaration, and offered as an `EntitySchema`: its `CREATE TABLE`, the
> `CREATE TYPE` of each enum its columns hold, an index for each `indexed`
> column and its comments. The table is the model's, unaliased (an aliased
> model is a `ConstructionError`), with a column per field in declaration
> order under its SQL name, of the kind's own type
> (`ArrayType::column_type`, an enum of the labels it lists, a created range
> over its subtype), `NOT NULL` unless it is nullable; its primary key, of
> its key fields in declaration order; each unique key the model declares;
> its comment; and a foreign key for each `belongs_to` relation in
> `relations`, starting at the model's table, with the actions and name the
> relation gives (`bindings.relations`), written as `RelationDef` converts
> (`[spec:pgorm:req:entity.relation.fk]`). A relation of another kind, one
> starting at another table, and a column filled by a `default` the
> declaration does not state, which has no `DEFAULT` to write, MUST be a
> `ConstructionError` carrying the Rust message.
>
> A column's DDL options are Rust's field attributes of the same names:
> `column_type` its type instead of the kind's, any type a `ColumnDef` takes
> but an array, since an array column's is its elements' (Rust's
> `column_type`); `default_value` a value converted through the kind,
> written as pgorm-query's escaped literal, and `default_expr` an
> expression, its `DEFAULT`; `generated_stored` and `generated_virtual` an
> expression it is `GENERATED ALWAYS AS (..)`, `STORED` or `VIRTUAL`;
> `generated` without one an identity, always or by default; `auto_increment`
> a one-column `i16`, `i32` or `i64` key drawn from the serial family, as a
> Rust entity's one-column integer key is unless it says otherwise; `indexed`
> an index of its own, `idx-{table}-{column}`; `comment` its `COMMENT ON
> COLUMN`; and `without_overlaps` a range or multirange key column that is
> the key's period, written `WITHOUT OVERLAPS` after the rest. A model's
> `comment` is its `COMMENT ON TABLE`, and `rename_all` names each field's
> column, where the column names none, by the case style Rust's
> `rename_all` takes (`camelCase`, `PascalCase`, `kebab-case`,
> `snake_case`, `SCREAMING_SNAKE_CASE`, `SCREAMING-KEBAB-CASE`,
> `lowercase`, `UPPERCASE`, `title_case`, `mixed_case`), converted by the
> derive's own conversion, pgorm's `CaseStyle`; any other style is a
> `TypeError`. The tables of a disjoint model (`bindings.disjoint`) and a
> one-of model (`bindings.one-of`) are described from their models' columns
> by the same lowering, so a column's `column_type` replaces the type its
> kind is written with there too, and `create_disjoint` writes its stated
> default, generation expression or identity; that schema writes no index or
> comment, which `from_model` of the kind's model gives.
>
> What the options cannot say MUST be a `ConstructionError` raised as the
> model is declared, before anything is sent: a `column_type` on an enum or
> created range column, whose type is its kind's, or naming an array; a
> `default_value` of NULL or not of the column's kind; a generation
> expression on a column generated by default; `auto_increment` on a column
> that is no `i16`, `i32` or `i64` primary key, or in a key of more than one
> column; `without_overlaps` on a column that is no range or multirange key
> column, on a key field but the last, or in a key of one column, which
> PostgreSQL refuses; and whatever pgorm's `TableShape` refuses, its unique
> keys over a `VIRTUAL` generated column among them. A `default_expr`,
> `generated_stored` or `generated_virtual` that is no expression, and a
> `comment` that is no string, are `TypeError`s.

> [spec:pgorm:req:bindings.model-casts]
> A declared model's column MUST be read, written and compared through
> pgorm's own casts for the type it declares, as an entity's column is:
> where it declares a `column_type`, a `select_as` or a `save_as`, the model
> asks pgorm's `TableColumn` of that type (`[spec:pgorm:sem:schema.shape-casts]`)
> for each cast, and a column declaring none of them is read, written and
> compared as it stands, as before. So a `money` column of the `decimal`
> kind is read cast to `numeric` and each value written and compared cast to
> `money`; an enum column whose `column_type` is a domain over its enum is
> read as its text, read back as the enum's label, written cast to the
> domain, and compared with both sides cast to the enum, which PostgreSQL
> resolves no operator for otherwise (`42883`); and a domain over any other
> type is written and read as its base. `select_as` and `save_as` name the
> type a read and a write cast to — a built-in type's name, a `TypeName`, or
> a `DataType`, whose length or precision the cast writes as the type's
> modifiers (`bit(7)`, through pgorm-query's `ColumnType::cast_type`) — and
> a comparison's value takes the `save_as` cast too. `select_with` and
> `save_with` are functions of the binding's own from an expression to an
> expression: the column read through the one, and each written or compared
> value, bound, through the other. A `column_type` on a created range's
> column, a `column_type` on an enum's that is no domain over that enum,
> both `select_as` and `select_with`, both `save_as` and `save_with`, and a
> type no cast names (an interval restricted to fields, an unlimited `bit`)
> MUST be `ConstructionError`s as the model is declared, and a `select_with`
> or `save_with` that is no function, or returns no expression, a
> `TypeError`. Every projection a model builds — its queries', projections',
> graphs' and RETURNING lists' — reads a field through the same cast, and a
> keyset cursor's boundaries compare through it.
>
> A `DataType` also names a domain, `DataType.domain(name, base)`, and an
> interval restricted to fields, `DataType("interval", fields=..)`, the
> fields spelled in the binding's case (`hour_to_minute`, `hourToMinute`) and
> read by pgorm-query's `PgInterval`, a `precision` only where the last
> field is seconds (`PgInterval::with_precision`), any other a
> `ConstructionError`. A disjoint or one-of model's tables take their
> columns' type casts natively (`bindings.disjoint`); the four cast options
> are a declared model's own reads and writes.

> [spec:pgorm:req:bindings.model-conversions]
> A declared model's column MUST take a `decode` and an `encode`, functions
> of the program's own, as a Rust field of a `ValueType` or `JsonValueType`
> is read and written as the program's type: each value a record reads for
> the field is the field's decoded value passed through `decode`, checked as
> the row is read — a function that throws or raises makes the read a
> `DecodeError`, never a record holding the value — and each value a write,
> a comparison or a cursor boundary takes is passed through `encode` before
> it is converted through the kind, a `Value` and an expression taken as
> written. A NULL is never passed to either. An enum stored as numbers or as
> text, Rust's `rs_type` and `db_type`, is a column of the kind it is stored
> as with a `decode` and `encode` between the stored value and the program's
> enum, and Rust's `display` is the program's own spelling of it. A `decode`
> or `encode` that is no function MUST be a `TypeError` as the column is
> declared.

> [spec:pgorm:req:bindings.schema-tables]
> Creating a table MUST build pgorm-query's `TableCreateStatement`: columns,
> each a `ColumnDef` with a type; `if_not_exists`; the table's one primary key,
> a later one replacing it; and any number of unique keys, foreign keys and
> CHECK constraints. A key is a column or a non-empty list of them, with a
> name, INCLUDEd columns, a deferrability and PostgreSQL 18's
> `without_overlaps` column, which pgorm-query writes last as `"c" WITHOUT
> OVERLAPS`; `nulls_not_distinct` is the unique key's alone. A foreign key
> pairs its columns in order with as many referenced columns, anything else
> refused, and takes a name, ON DELETE and ON UPDATE actions, a
> deferrability, an enforcement and PostgreSQL 18's PERIOD pair, written last
> on both sides. A CHECK is an expression with a name, an enforcement and NO
> INHERIT. Deferrability is not deferrable, deferrable initially immediate or
> deferrable initially deferred, and enforcement enforced or not enforced;
> neither is written unless it is given.
>
> A `ColumnDef(name, type)` takes a `DataType`, a built-in type's name, a
> `TypeName` or a created range, and the clauses pgorm-query writes in the
> order they are added: `not_null`, the column's one NOT NULL constraint,
> with a name and NO INHERIT; `null`; `default`, an expression or a value;
> `check`; `generated(expression, kind)`, whose kind, stored or virtual, MUST
> be named, PostgreSQL 17 refusing a generated column without one and 18
> reading it as virtual; `identity(always or by default, options)`, its
> sequence's options those `bindings.schema-sequences` gives;
> `auto_increment`, the serial family; and `collate`
> (`bindings.collations`). A column with no type is refused where a table is
> created or a column added. Dropping tables (one or more, `if_exists`, a
> behavior), renaming a table, whose new name is bare, renaming a column or a
> constraint, and truncating MUST build the statements pgorm-query has for
> each.

> [spec:pgorm:req:bindings.schema-alter]
> Altering a table MUST build pgorm-query's `TableAlterStatement`, which
> holds one or more actions, each the pgorm-query action of its name:
> `add_column` (with `if_not_exists`), `modify_column`, `drop_column`,
> `add_primary_key`, `add_unique`, `add_foreign_key`, `add_check`,
> `add_not_null` (PostgreSQL 18's table-level NOT NULL, with a name and NO
> INHERIT), `drop_constraint` of any kind by name (with `if_exists` and a
> behavior), `validate_constraint`, `alter_constraint` (inherit, no inherit,
> enforced or not enforced), `set_expression` and `drop_expression` (with
> `if_exists`). NOT VALID belongs to the three actions PostgreSQL takes it on
> — adding a foreign key, a CHECK and a NOT NULL — and leaves the rows
> already there unchecked until `validate_constraint` checks them, while new
> rows are held to the constraint at once.
>
> `modify_column` writes each aspect its column carries as pgorm-query writes
> it: a retype with its collation, SET or DROP NOT NULL, a named NOT NULL
> added, SET DEFAULT, a CHECK added, an identity added. An aspect no such
> action writes MUST be refused rather than dropped: a generated expression
> (which `set_expression` and `drop_expression` change), the serial family,
> and a collation without the type it is given with. So MUST a column that
> changes nothing.

> [spec:pgorm:req:bindings.schema-indexes]
> Creating an index MUST build pgorm-query's `IndexCreateStatement` over its
> first entry, and further entries append. An entry is a column's name or an
> expression, with an optional order, ascending or descending, and an
> optional operator class. The index takes `unique`; `nulls_not_distinct`,
> which PostgreSQL defines for a unique index alone, and which so makes the
> index unique rather than be written for nothing; `if_not_exists`; `using`,
> an access method by identifier; `include`; and `where`, a partial index's
> predicate ANDed to one already there. CONCURRENTLY is not offered,
> PostgreSQL refusing it in a transaction. Dropping an index drops it from its
> table's schema, with `if_exists`.

> [spec:pgorm:req:bindings.schema-types]
> Creating a type MUST build pgorm-query's `TypeCreateStatement`, a shell
> type until a kind is chosen, its kind one slot as pgorm-query holds it: an
> enum with its labels, appended; a composite with its attributes, each a
> name, a type and an optional collation, appended; or a range with its
> subtype and its optional subtype operator class, collation, subtype
> difference function and multirange type name. An enum label is data,
> written as a literal, and MUST be at most 63 bytes without NUL, as
> PostgreSQL stores it; the empty label is one.
>
> Altering a type MUST give one change: adding a value before or after
> another, never both; renaming the type, whose new name is bare; renaming a
> value; renaming an attribute, with a behavior; or a composite's adding,
> dropping (with `if_exists`) and altering of attributes, which take more of
> their kind and a behavior, cascade carrying the changes into typed tables.
> Dropping types drops one or more, with `if_exists` and a behavior.

> [spec:pgorm:req:bindings.schema-sequences]
> A sequence's options MUST be one vocabulary for a standalone sequence and an
> identity column, pgorm-query's `SequenceOptions`: `increment_by`,
> `min_value`, `max_value`, `start_with`, `cache` and `cycle`, each number an
> integer within `bigint`, a bound's null its NO form, and any other option
> refused. Creating a sequence takes `if_not_exists`, a type (smallint,
> integer or bigint), options, which merge at least one option into those set,
> and OWNED BY a table's column, or NONE. Altering a sequence gives its
> statement with its first clause, and takes the same clauses, `restart` with
> an optional value, and `if_exists`. Dropping and renaming a sequence build
> pgorm-query's statements. Creating an extension, with `if_not_exists`, a
> schema, a version and CASCADE, and dropping one, with `if_exists` and a
> behavior, build pgorm-query's extension statements, the version a literal.
> Commenting on a table and on a column build its COMMENT ON, the text a
> literal pgorm-query escapes.

> [spec:pgorm:req:bindings.schema-views+2]
> Views, materialized views and tables created from a query MUST be built
> through pgorm-query's builders for them (`sql.ddl.view`,
> `sql.ddl.materialized-view`, `sql.ddl.create-table-as`), each named as a
> table is, under `bindings.schema`. Each body is a `Select` the binding built,
> handed to the Rust statement as the `SelectStatement` it owns, and nothing
> else is taken. It renders through pgorm-query's own rendering with every
> value written as an escaped literal, because a definition is stored as SQL
> text and binds nothing, so `inspect()` gives an empty value list. The body
> is read as a method reads a statement into another (`bindings.select`):
> nothing can order it or declare its windows once it is a definition, so a
> tie limit `check_tie_limit` refuses, and a WINDOW clause or a reference to
> one that `check_windows` refuses, MUST be a `ConstructionError` as the
> statement is built, never sent to the server.
>
> - A view takes OR REPLACE, a column list, a check option (local or
>   cascaded), `security_barrier` and `security_invoker`.
> - A recursive view names one column or a non-empty list of them, an empty
>   list being refused, since PostgreSQL refuses a recursive view without
>   columns. It takes the same options except the check option, which the
>   grammar refuses on a recursive view. Its query reads the view through a
>   set operation joining a non-recursive term to an arm reading it by its
>   bare name, the name of the `WITH RECURSIVE` query PostgreSQL defines the
>   view as; a schema-qualified name looks for a relation that does not exist
>   yet (`42P01`).
> - A materialized view and a table created from a query take
>   `if_not_exists`, a column list and WITH NO DATA. Neither has OR REPLACE,
>   which PostgreSQL 18 has for neither, and a view has no IF NOT EXISTS,
>   which PostgreSQL does not have for one.
> - Refreshing a materialized view takes at most one of CONCURRENTLY and
>   WITH NO DATA: PostgreSQL refuses them together, pgorm-query holds them in
>   one slot, and asking for both MUST be a `ConstructionError` rather than
>   one of them dropped.
> - Dropping a view or a materialized view takes one or more relations, with
>   `if_exists` and a behavior.
>
> What the Rust types cannot hold MUST be refused with a `ConstructionError`
> as the statement is built, never dropped: a recursive view without columns,
> and a refresh both concurrent and with no data. What only the server knows
> it refuses, by SQLSTATE, as a `DatabaseError`: a check option on a view that
> cannot be updated (`0A000`), a row a check option refuses (`44000`), an OR
> REPLACE that moves or drops a column (`42P16`), a materialized view read
> before it is populated (`55000`), a concurrent refresh without a unique
> index (`55000`) or of an unpopulated view (`0A000`), and a drop of the other
> kind (`42809`).

> [spec:pgorm:req:bindings.schema-schemas]
> The statements over a schema MUST be built through pgorm-query's `Schema`
> (`sql.ddl.schema`), under `bindings.schema`, each schema named by one
> identifier, quoted, and none schema-qualified, since schemas do not nest; a
> `Table` is refused. `create_schema(name)` takes `if_not_exists`.
> `drop_schema` drops one or more schemas in order, the list never empty, with
> `if_exists` and a behavior, cascade or restrict. `rename_schema(name,
> new_name)` has no `if_exists`, PostgreSQL's grammar having none for it.
> `inspect()` gives an empty parameter list. A name that is not an identifier
> MUST be a `ConstructionError` as the statement is built. What only the
> server knows it refuses by SQLSTATE, as a `DatabaseError`: a schema that
> exists (`42P06`), a `pg_` name (`42939`), a missing schema (`3F000`), and a
> drop under RESTRICT, said or unsaid, of a schema that holds anything
> (`2BP01`).

> [spec:pgorm:req:bindings.schema-domains+2]
> The statements over a domain MUST be built through pgorm-query's `Domain`
> (`sql.ddl.domain`), under `bindings.schema`, each domain named as a type is,
> by text or a `TypeName` carrying its schema, and every other name an
> identifier. `create_domain(name, base)` takes its base as a column takes its
> type — a `DataType`, a built-in type's name, a `TypeName` (another domain's
> among them) or a created range — and is a builder: `collate`; `default`, an
> expression or a value written as a literal; `not_null` and `null`, which
> share one slot, the later call winning; and `check(condition, name)`,
> appended in the order added. A check takes no NOT VALID at CREATE DOMAIN,
> PostgreSQL having none there. `domain_value()` is the `VALUE` a check's
> condition tests.
>
> Each ALTER DOMAIN action gives a whole statement over the domain it names,
> PostgreSQL taking one action per ALTER DOMAIN and no IF EXISTS: setting and
> dropping the default; setting and dropping NOT NULL; adding a check, with a
> name and NOT VALID; dropping a constraint, with `if_exists` and a behavior;
> renaming and validating a constraint; and renaming the domain, whose new
> name is bare. `drop_domain` drops one or more domains in order, the list
> never empty, with `if_exists` and a behavior. `inspect()` gives an empty
> parameter list, a default's and a check's values inlined.
>
> Text that is not an identifier and an unknown base type MUST be refused as
> the statement is built, the first with a `ConstructionError`. A registered
> entity's column whose type is a domain (`ColumnType::Domain`) reports, binds
> and reads as the kind of the type it is over, an enum's labels included, so
> its records and the types generated for it are typed by its base. What only
> the server knows it refuses by SQLSTATE, as a `DatabaseError`: a domain or
> constraint name already taken (`42710`); a missing domain or constraint
> (`42704`); a value a check refuses (`23514`) and a null under NOT NULL
> (`23502`), whether written to a column of the domain or met among the values
> already there by adding a check, setting NOT NULL or validating a
> constraint; a collation on a base type without one (`42804`); and a drop
> under RESTRICT, said or unsaid, of a domain a column is typed with
> (`2BP01`).
