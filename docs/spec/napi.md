# JavaScript API (Node-API)

This chapter specifies what is particular to pgorm's JavaScript binding, the
optional `pgorm-napi` companion crate: a native addon built with the Neon
framework against Node-API, which both Node.js and Deno load. What both
bindings offer is specified once in [bindings.md](bindings.md); this chapter
keeps JavaScript's spelling, its Promises and disposal, its native types, its
TypeScript declarations, its runtimes and packaging, and the choices still
waiting on the operator. The WBS root is `napi`.

## Package and spelling

> [spec:pgorm:def:napi.api+1]
> The JavaScript API is an ES module, `pgorm-napi/lib/index.js`, backed by a
> native Node-API addon over pgorm's Rust query construction, execution and
> decoding APIs. The module loads the addon, hands it the error and value
> classes it rejects with and builds values from, exports them, and is what an
> application imports; the addon is not imported directly.

> [spec:pgorm:req:napi.naming+4]
> JavaScript MUST spell a shared rule's neutral name in camelCase — functions,
> methods and option keys — and a class as written. A closed choice is a
> string literal in camelCase (`"inner"`, `"noKeyUpdate"`, `"skipLocked"`,
> `"byDefault"`), and the options of a call are the keys of an options object,
> its last argument. A class Python constructs by calling it, JavaScript
> constructs through a factory function named for it in camelCase (`select`,
> `insert`, `update`, `merge`, `column`, `model`, `copyFrom`, `copyTo`,
> `tableFunction`, `jsonDefault`), its constructor private where the factory
> alone makes one. The DDL factories, `createTable`, `alterTable` and the
> rest, are both bindings' (`bindings.schema-api`). The pipeline is the
> module's `pipeline` namespace.
>
> A reserved word is spelled otherwise: `deleteFrom` for `delete`, and
> `thisColumn`, `thatColumn` and `caseWhen` for a pipeline's `this`, `that`
> and `case`. The grouping elements differ from the shared shape by choice
> and wait on the operator (`bindings.surface`): the functions `groupingSet`,
> `rollup`, `cube` and `groupingSets`.

## Targeting and loading

> [spec:pgorm:req:napi.loading+1]
> The addon MUST target Node-API version 6 — the lowest that carries
> per-instance data, which the shared channel, instance-local values and the
> dropped-handle queue rest on — and MUST NOT link against a particular
> runtime's executable: its Node-API symbols are resolved from the host process
> when it loads, so the one build serves Node.js and Deno.
>
> A checkout's build is copied to `lib/pgorm_napi.node`, beside the module, and
> kept out of version control. `lib/native.js`, which `lib/index.js` imports,
> loads the addon through a CommonJS `require` made with `node:module`'s
> `createRequire`, the one loader both runtimes give a `.node` file: a library
> beside the module — a checkout's own build, or an application's
> (`napi.application`) — before anything else, and otherwise the
> addon of the installed platform package (`napi.platform-loading`).
> Node.js needs no flag to load it. Deno
> MUST be granted `--allow-ffi`, to open a native library, and `--allow-read`,
> to resolve its path; `--allow-net` is not needed and does not confine the
> addon. Native code runs outside Deno's permission checks, so granting
> `--allow-ffi` to a program that loads the addon trusts it with everything
> the process can do, the network included.

## Runtime and threading

> [spec:pgorm:req:napi.runtime+1]
> The shared runtime (`bindings.runtime`) MUST be built when the first
> instance of the addon loads and shared by every instance after it — the
> main thread's and each worker thread's. The runtime is never shut down: its
> idle threads hold nothing a JavaScript event loop waits on, the process's
> exit ends them, and shutting it down with an instance would cancel the work
> of the instances still running.

> [spec:pgorm:req:napi.promises]
> Every operation that does I/O MUST return a Promise and MUST NOT throw
> synchronously: an argument that cannot be used rejects the Promise like any
> other failure. The work runs on the runtime, and its outcome MUST settle the
> Promise on the JavaScript thread of the instance that started it, through
> that instance's shared channel. Each operation in flight MUST hold a
> reference that keeps its event loop alive, and MUST release it once its
> settlement is queued, so that concurrent operations each settle with their
> own outcome and a loop with nothing in flight is free to finish. An
> operation MUST NOT leave its promise to Neon's drop queue, which rejects a
> promise whose `Deferred` was dropped unsettled: the queue holds no event
> loop open, and Node.js finishes a loop with its deliveries still pending
> where Deno delivers them first, so a promise left to it may never settle.

> [spec:pgorm:req:napi.errors+3]
> Failures MUST reject with instances of the module's error classes
> (`bindings.errors`), all derived from `PgormError` and `Error`, a panic on
> the runtime included, which MUST reject its promise rather than leave it
> pending. A failure is captured as data on the runtime thread and becomes an
> error object on the JavaScript thread, built by a factory the module
> registers with its instance; an instance with none registered rejects with
> an `Error` of the same name and fields.

## Values

Every kind of value pgorm's `Value` holds crosses into and out of JavaScript
with one declared JavaScript type, `interval` among them. Precision is the
first rule: a value reaches JavaScript exactly or not at all.

| Kind | PostgreSQL | JavaScript |
| --- | --- | --- |
| `bool` | `boolean` | `boolean` |
| `i8`, `i16`, `i32`, `u32` | `"char"`, `int2`, `int4`, `oid` | `number` |
| `i64`, `u64` | `int8` | `bigint` |
| `f32`, `f64` | `float4`, `float8` | `number` |
| `text`, `char` | `text`, `varchar`, `bpchar`, `name` | `string` |
| `bytes` | `bytea` | `Uint8Array` |
| `json` | `json`, `jsonb` | `null`, boolean, number, `bigint`, string, array, plain object |
| `decimal` | `numeric` | the module's `Decimal` |
| `uuid` | `uuid` | the module's `Uuid` |
| `date`, `time`, `datetime`, `datetime_utc` | `date`, `time`, `timestamp`, `timestamptz` | `Temporal.PlainDate`, `PlainTime`, `PlainDateTime`, `Instant` |
| `interval` | `interval` | the module's `Interval` |
| `ipnetwork`, `mac_address` | `inet`/`cidr`, `macaddr` | `string`, six-byte `Uint8Array` |
| `vector` | pgvector's `vector` | `Float32Array` |
| a built-in range or multirange | `int4range` … `tstzmultirange` | the module's `Range`, `Multirange` |
| an enum | its type | `string` |
| a created range | its type | `Range`, its kind a `CreatedRange` |
| a created multirange | its type, in a binary COPY | `Multirange`, its kind a `CreatedMultirange` |

> [spec:pgorm:req:napi.values+1]
> Every kind `bindings.values` names, and SQL NULL as `null`, MUST cross as
> the JavaScript type the table above declares for it, exactly: `int8`
> decodes as a `bigint`, never a number; `numeric` as a `Decimal`, whose text
> keeps the value's scale; `float4` widened exactly; a JSON number exactly, an
> integer literal past `Number.MAX_SAFE_INTEGER` as a `bigint` and a number
> serde_json would read as another refused; and a date or timestamp within the
> years -9999 to 9999 pgorm holds.
>
> In the other direction a number past 2^53 - 1 declared as an integer, an
> `f32` that narrowing would change, a string holding a lone surrogate, a
> `Decimal` in exponent notation, and a `Decimal` made from a number, which
> may already be rounded, MUST be refused with a `ConstructionError`. A
> `Decimal` MUST NOT become a number implicitly.

> [spec:pgorm:req:napi.temporal+1]
> Dates and times MUST be Temporal's: `date` a `Temporal.PlainDate`, `time` a
> `Temporal.PlainTime`, `timestamp` a `Temporal.PlainDateTime` and
> `timestamptz` a `Temporal.Instant`, a `PlainDate` or `PlainDateTime` in
> another calendar binding as the same day. A `timestamptz` is an instant:
> PostgreSQL keeps no zone with it and sends it in binary as microseconds from
> 2000-01-01 UTC, so the instant read MUST NOT depend on the session's
> `TimeZone`, which governs only its text rendering and the reading of text
> without an offset. A `ZonedDateTime` would invent a zone on the way out and
> drop one on the way in, so it is refused as a parameter, as a `Date` is.
> A `PlainDateTime` MUST NOT bind to `timestamptz`, nor an `Instant` to
> `timestamp`, in an array or a range bound included: pgorm writes both as
> microseconds and would read either as UTC, where PostgreSQL's own cast
> between the two reads the wall clock in the session's zone.
>
> PostgreSQL keeps microseconds and Temporal nanoseconds. Reading MUST be
> exact, and a value written with a nonzero digit below the microsecond MUST
> be a `ConstructionError`, never truncated or rounded by the binding.
>
> `Temporal.Duration` requires every field to share one sign and so cannot
> hold `1 mon -2 days`, which is why an interval is the module's `Interval`
> (`bindings.values`). `Interval.from` takes a `Duration`, years and months
> becoming months, weeks and days days and the rest microseconds, and a
> `Duration` parameter binds as one; `toDuration` gives one back where the
> signs agree and throws a `RangeError` where they do not.

> [spec:pgorm:req:napi.inference]
> A plain JavaScript parameter MUST be bound as the one kind its type leaves:
> `null` as SQL NULL of no declared kind; a boolean as `bool`; a number as
> `i64` when it is a safe integer other than negative zero, so that it binds
> to an integer column exactly, and as `f64` otherwise; a `bigint` as `i64`; a
> string as `text`; a `Uint8Array` as `bytes`; a `Decimal`, `Uuid`,
> `Interval`, `Temporal.Duration` or Temporal date, time, date-time or
> instant as its kind; a plain object as `json`; and an array as an array of
> the one kind its non-null items infer as, numbers being `i64` items only
> when every one is a safe integer. `undefined`, an empty or all-null array,
> a range, an array of mixed kinds or nested arrays, and any other object
> MUST be refused with a `ConstructionError` that names the explicit form,
> never guessed at or turned into a string.

> [spec:pgorm:req:napi.records]
> A statement's rows MUST reach JavaScript as plain objects, one per row,
> keyed by column name in column order, each key an own data property, so
> that a column named `__proto__` is a property rather than the row's
> prototype. `{ tagged: true }` among a terminal's options gives every column
> as a `Value` carrying its kind (`bindings.value-tags`). A model's records are
> plain objects keyed by field. A registered entity's records are frozen plain
> objects keyed by SQL column name, behind each of which the module keeps the
> Rust model it was read as; the entity operations on a record are the
> entity's functions over it, `intoActive(record)`, `withValue(record,
> column, value)` and `tagged(record, column)`. A change is `{ old, new }`,
> an upsert `{ kind: "inserted", new }` or `{ kind: "updated", old, new }`, a
> notification a frozen `{ channel, payload, processId }`, and a pool's status
> a plain object.

## Connections

> [spec:pgorm:req:napi.lifecycle+2]
> `new Pool(connectionString, options)` builds a pool, and `connect` resolves
> with one whose server answered. The options are `tls` (`"verify-full"` or
> `"disable"`), `ca`, PEM certificates as a string or `Uint8Array`,
> `maxSize`, `connectTimeout` and `acquireTimeout` in milliseconds,
> `statementCacheSize`, `recycle` (`"verified"` or `"fast"`) and
> `statementCacheChannel`. Each pool, connection, stream and listener MUST be
> closable by `close()` and by `await using`. A stream is an async iterator
> whose `next()` pulls one row and whose `return()`, which a `for await` loop
> leaving early calls, closes it as `close()` does. What each holds MUST be
> released by that close, never left to garbage collection or Neon's drop
> queue. A handle collected unclosed releases only what is idle — an idle
> transaction rolled back, an idle connection returned to its pool, a
> listener's connection closed — never cutting short work still running on
> it, and a transaction keeps the connection or transaction it borrows
> reachable. A pull made after any iterator has closed or ended resolves as
> done. A process that ends holding open handles still exits (`napi.exit`).

> [spec:pgorm:req:napi.raw]
> `execute`, `query`, `one`, `optional` and `stream` on a pool, a connection
> and a transaction MUST take SQL text and an array of parameters, each bound,
> never interpolated, through the path a built statement takes
> (`bindings.results`), and a built statement with its options second.

> [spec:pgorm:req:napi.transactions+3]
> A transaction MUST open only through `transaction(fn, options)` on a pool
> or a connection, and a savepoint only through `transaction(fn)` on a
> transaction; a `Transaction` is not disposable. JavaScript sends the
> closure transaction's task (`bindings.transactions`) owned commands. When
> `fn` resolves the closure returns `Ok`, and when it throws `Err`.
> `transaction(fn)` resolves with what `fn` resolved with and rejects with
> what it threw, or with the failed commit's error. A failed `ROLLBACK`
> rejects with a `SuppressedError` whose `error` is the rollback's database
> error and whose `suppressed` is what `fn` threw. A callback that settles
> with a savepoint still open fails, and commits nothing. The options `mode`
> and `isolation` select pgorm's `TransactionMode`.

> [spec:pgorm:req:napi.cancellation+2]
> Acquiring a connection, running a statement, opening a transaction or a
> savepoint, pulling a stream's rows, a COPY, a listener's operations and
> `notify` MUST each take an `AbortSignal` as the `signal` option. An aborted
> operation rejects with the signal's reason, and behaves as
> `bindings.cancellation` requires.
>
> An operation is aborted when its signal fires before its promise settles,
> even where the addon finished the work first, its result queued behind the
> abort: the promise MUST reject with the signal's reason all the same, after
> releasing what the result holds. A statement's or a ping's result is
> discarded; a connection it acquired goes back to its pool; a COPY TO it
> opened is closed; a statement, a COPY or a job it ran in a transaction, and
> a transaction or savepoint it began, end the whole transaction with nothing
> committed and its connection discarded, as an abort the addon saw ends it; a
> stream whose pull it was closes; a listener it opened closes, as one whose
> `listen`, `unlisten` or `unlisten_all` it was does; and a notification a
> pull received is kept for the listener's next pull, so none is lost.

## Pipelines

> [spec:pgorm:req:napi.pipeline-table-names]
> A pipeline's source (`bindings.pipeline`) MAY also be a table's name as a
> string, read as the `Table` of that name.

## Registered entities

> [spec:pgorm:req:napi.application+2]
> A downstream application crate registers its entities (`bindings.entities`)
> in a `Registry` — `entity::<E>(name)`, `graph::<E, S, _>(name, factory)`
> and `sources::<T>(name)` — and builds its module through
> `install(cx, registry)`, called from the application's own
> `#[neon::main]` with the binding's `standalone-module` feature off; the
> binding's own module is that feature's, with an empty registry. A name
> outside 1–255 bytes is refused as it is registered.
>
> An application crate outside the binding's workspace MUST build the module:
> it depends on pgorm-napi without default features, on the same checkout's
> pgorm and on Neon, registers its entities and graphs, and exports a
> `#[neon::main]` calling `install`. The binding's ES module, copied beside
> the application's library as `lib/pgorm_napi.node`, is then the module an
> application imports. `checks/entities.js` MUST build the repository's
> application fixture — its own entities, hooks and graphs, none of the
> binding's test tables — run its Rust tests, materialize the module beside
> its library, and run its JavaScript suite against a live server under both
> `node --test` and `deno test`, failing on any failure; CI runs it.

## Generated TypeScript

An application's registrations are typed by a module generated from its
built library (`bindings.codegen`).

> [spec:pgorm:req:napi.codegen+1]
> `pgorm-napi/codegen` MUST generate an application's module from a build
> description naming its entity crate and the entities, graphs and source
> tuples to register, each under a registration name and a TypeScript
> export, refusing a description whose exports repeat or are not public
> JavaScript identifiers. `scaffold` MUST write, at a destination that does
> not exist, the crate installing what is described from its own
> `#[neon::main]`, beside a copy of the binding's ES module; `build` places
> the crate's library as that module's addon; `emit` MUST write the
> TypeScript module and its declarations from what the built library
> describes. The generated module refuses to load with a
> `ConstructionError` when the addon's version or a registration it names no
> longer describes as it did.

> [spec:pgorm:req:napi.codegen-types]
> The generated declarations MUST type each entity's records as their reads
> give them — keyed by SQL column name, each column the type its kind decodes
> as, `| null` where it is nullable, an enum's labels as a union of string
> literals, an array's items `| null` unless the entity's Rust field is a
> `Vec` of a type that is no `Option`, and a column of a type the binding has
> no kind for as `PlainValue` — and what each column takes, so that column
> comparisons, ActiveModel `set`s, records and the version terminals are typed
> by the entity. A graph's row MUST be its root's record when it has no slot
> and otherwise a tuple of every source's record, an `Opt` slot's `| null`;
> a source tuple's row a tuple of each source's record `| null`. `deno check`
> MUST hold a consumer of the generated module to these types, its
> `@ts-expect-error` lines included.

## Clean exit

> [spec:pgorm:req:napi.exit]
> Loading the addon MUST NOT keep a process alive, nor MUST anything it leaves
> behind once its operations have settled: a process that has finished its
> work exits on its own, in Node.js and in Deno, with its ordinary status and
> nothing written to stderr. An operation still in flight when a script's last
> statement runs MUST keep the process up until it settles. A process that
> exits explicitly while operations are running MUST neither wait for them nor
> crash, and an outcome that arrives after its instance has been torn down — the
> process exiting, its worker terminated — MUST be dropped without a panic or a
> write to a closed environment. The shared channel and the drop queue MUST
> NOT hold a process open once nothing is in flight, whether or not the drop
> queue has deliveries pending. An unhandled rejection MUST end the
> process the way the runtime ends one for any other unhandled rejection.

## Types and tests

> [spec:pgorm:req:napi.typing+1]
> The module MUST ship TypeScript declarations for every export, error classes
> and their fields included, beside it as `lib/index.d.ts` and named by the
> module's `@ts-self-types` directive so Deno finds them as Node.js tooling
> does. `deno check` MUST type-check the declarations, the module and the test
> suite, which uses the public API through them. The declarations MUST type a
> model from its declaration alone: `RowOf`, a record's fields and their
> types, a nullable column's `| null` and an array column's items, an enum's
> listed values as a union of their labels; `InsertOf`, requiring every field
> neither nullable, defaulted nor generated and refusing a field generated
> always; `UpdateOf`; and `KeyOf`. A COPY statement's format is a type
> parameter, `CopyFrom<"binary">` or `CopyTo<"binary">` for the row COPY.

> [spec:pgorm:req:napi.runtimes+1]
> The binding targets runtimes that carry `Temporal` as a global, which its
> date and time values are: Node.js 26 or later, and Deno 2.9.5 or later, the
> oldest release tested. Loading the module in a runtime without `Temporal`
> MUST throw an error naming the requirement rather than fail later on a
> value. Node.js 26 strips TypeScript types without a flag, so the suite and
> its fixtures run under plain `node`.
>
> One test suite MUST run unchanged under `node --test` and under `deno test`,
> written against `node:test` and `node:assert`, which both runners provide,
> with every runtime difference — how a fixture process is started, and the
> Deno permissions it is granted — kept in one support module. The exit
> behaviour `napi.exit` requires MUST be tested in fresh processes of
> the runtime under test, each held to exiting by itself before a deadline. CI
> MUST run the suite in both runtimes, Node.js 26 and Deno, against PostgreSQL
> 18 on Linux and macOS.

## Distribution

pgorm-napi reaches applications as npm packages with prebuilt addons, the
layout Node-API modules use: a main package of JavaScript, `@necessary/pgorm`,
and a package per platform holding that platform's native code, which a
package manager selects by the platform it runs on. [DISTRIBUTION.md](../../pgorm-napi/DISTRIBUTION.md)
is the user-facing account of it.

> [spec:pgorm:req:napi.packages+1]
> pgorm-napi MUST be distributed as npm packages released together at one
> version, the crate's, in the `@necessary` scope: a main package,
> `@necessary/pgorm`, holding the ES module, its declarations,
> `DISTRIBUTION.md`, `support.json` and pgorm's licences, and no native code;
> and one package per release platform holding that platform's release addon
> as `pgorm_napi.node`, pgorm's licences and the Rust dependency notices
> `napi.notices` requires. A platform package is named for the main package
> and its platform, `@necessary/pgorm-<os>-<cpu>` with `-gnu` or `-musl` added
> on Linux, in Node.js's `process.platform` and `process.arch` vocabulary, and
> declares the `os`, `cpu` and, on Linux, `libc` a package manager selects it
> by. Every package's manifest MUST declare public access, which npm otherwise
> withholds from a scoped package. The main package MUST list exactly the
> release platforms' packages as optional dependencies, each at the main
> package's own version exactly — the one exact requirement the packages
> carry, because a module and the addon built with it are one release. The
> checkout's own `package.json` carries the main package's name and stays
> private: the packages are made by the packaging check, never by packing the
> checkout.

> [spec:pgorm:req:napi.platform-loading]
> Without a library beside it, the module MUST load the addon from the
> platform package for the running process: `process.platform` and
> `process.arch`, and on Linux the C library the runtime links, read from
> Node.js's diagnostic report or, under Deno, whose report needs
> `--allow-sys`, from `Deno.build.env`. Loading MUST throw an Error naming the
> platform and the main package's version rather than fail later: when the
> platform's package is not installed, saying so and how to install it; when
> the main package offers no package for the platform, naming the platforms it
> does; and when the installed platform package's version, or its addon's
> `version`, differs from the main package's, naming both.

> [spec:pgorm:req:napi.release-builds+1]
> A platform package MUST hold a stripped release build of the addon, built
> with the crate's `addon` profile: Cargo's release profile with fat LTO, one
> codegen unit and the symbol table stripped. Its panics MUST still unwind,
> so that a panic reaches JavaScript as an exception rather than ending the
> host process, and the crate's own `release` profile stays as Cargo has it.
> The packaging check MUST refuse an addon carrying the debug-only probe
> exports, both by loading it on the platform that built it and by finding
> their names in its bytes, which any runner can check for any platform's
> binary; MUST refuse one carrying symbols its loading does not need (an ELF
> symbol table, or Mach-O local symbols), read from its bytes the same way;
> and MUST refuse one whose `version` is not the package version.

> [spec:pgorm:req:napi.notices]
> Each platform package MUST carry an inventory and the notice texts of the
> committed lockfile's whole Cargo graph — build and platform-conditional
> dependencies included, so a superset of what one addon links — and of the C
> components bundled through it, generated from Cargo's package metadata and
> the notice files each package ships. A text a crate archive omits MUST be
> recorded beside the generator with its upstream source and hash. Packing
> MUST refuse notices that no longer match the lockfile, and the lockfile MUST
> pass `cargo deny` under the repository's policy.

> [spec:pgorm:req:napi.support]
> `pgorm-napi/support.json` MUST list every platform the packages are named
> for, whether a release builds it, and the combinations the packaging check
> has passed on. Only release platforms are offered as optional dependencies,
> so the main package never names a package no release publishes, and
> documentation MUST NOT describe a platform as tested before the packaging
> check has passed on it.

> [spec:pgorm:req:napi.distribution]
> CI MUST build the release addon on each release platform's own runner, pack
> the full set of packages once, and on each release platform install that set
> from a registry serving only its tarballs into fresh projects — `npm install`
> for Node.js, and `npm:` in a `nodeModulesDir: "auto"` project for Deno — then
> run a smoke suite against PostgreSQL 18 in both runtimes: connecting, a bound
> query, a built statement, a model, a committed and a rolled-back
> transaction, `Temporal`, `bigint` and `Decimal` round trips, and a process
> exiting on its own. It MUST hold the loader to the refusals
> `napi.platform-loading` requires in both runtimes, and retain the
> tarballs for review. CI MUST NOT publish to a registry.
