# Python API

This chapter specifies what is particular to pgorm's Python binding, the
optional `pgorm-python` companion crate built with PyO3. What both bindings
offer is specified once in [bindings.md](bindings.md); this chapter keeps
Python's spelling, its asynchronous forms, its native types, its typing and
packaging, and the choices still waiting on the operator. The WBS root is
`python`; the governing decision is `[dec:pgorm:python-api]`.

The Python surface serves application developers and the generated campaign in
[generative.md](generative.md). The campaign consumes the public bindings;
fixture entities, vulnerable controls and scanner dependencies remain separate.

The integration milestone is `python.direct-builders`: an installed Python
module composes and executes real pgorm builders in process against
PostgreSQL. Neither this milestone nor full package acceptance depends on an
HTTP adapter, an external sqlmap scan or `sqlmap.acceptance`.

## Package and spelling

> [spec:pgorm:def:python.api+1]
> The Python binding (`bindings.api`) is the package `pgorm`, backed by a
> native PyO3 extension module. Statements, expressions, values and
> connections are in `pgorm`, DDL in `pgorm.schema`, the pipeline in
> `pgorm.pipeline`, and the code generator in `pgorm.codegen`. Concrete
> compiled Rust entity and graph types reach it through the extension's
> registration mechanism.

> [spec:pgorm:req:python.naming+4]
> Python MUST spell a shared rule's neutral name as written: functions,
> methods and keyword arguments in snake_case, classes in PascalCase, and an
> option as a keyword argument. A name that is a Python keyword takes a
> trailing underscore: `from_`, `as_`, `with_`, `except_`, `and_`, `else_`. A
> closed choice is a class whose members are PascalCase class attributes
> (`Join.Inner`, `Direction.Asc`, `FrameType.Rows`, `ReturningRow.Old`). A
> class is constructed by calling it, where JavaScript calls a factory
> function (`napi.naming`), unless a shared rule makes it by a factory alone
> (`bindings.select`, `bindings.writes`, `bindings.merge`,
> `bindings.schema-api`); `select`,
> `insert`, `update` and `delete` are the classes `Select`, `Insert`,
> `Update` and `Delete` under those names.
>
> Python's own operators spell the expression operators: `==`, `!=`, `<`,
> `<=`, `>` and `>=` beside `eq` .. `gte`; `+`, `-`, `*`, `/` and `%` for
> `add`, `sub`, `mul`, `div` and `mod` (`rem` in a pipeline); `&`, `|` and `~`
> for `and`, `or` and `not`; and unary `-` for a pipeline's negation.
>
> `any_` and `all_` take an underscore, and `tuple_expr` its suffix, so as not
> to shadow Python's builtins.

> [spec:pgorm:req:python.package]
> The package MUST declare its Rust crate, Python distribution, import module,
> pgorm compatibility and supported Python/platform matrix in version-controlled
> metadata. Maturin builds MUST produce an installable wheel and source
> distribution. The installed module MUST expose package, pgorm, binding and
> enabled-feature identities. A clean environment MUST be able to import and
> use the wheel without a Rust toolchain; source builds may require one.
> Distribution-name availability and the chosen ABI policy MUST be established
> before release; this specification does not assert a reserved registry name.

> [spec:pgorm:req:python.capabilities+2]
> A versioned capability manifest, `capabilities()`, MUST enumerate supported
> operations, value types, result forms, feature requirements, corresponding
> Rust APIs and compiled entity, graph and source-tuple registrations, keeping
> runtime DDL construction apart from schema generation for a registered
> entity. `require_capability(name)` and every operation outside the manifest
> MUST fail explicitly with `UnsupportedCapabilityError` before execution, as
> does a model column of a kind the native record decoder does not offer. An
> operation outside the manifest is a value the build has nothing for, so
> `UnsupportedCapabilityError` is a `ConstructionError` (`bindings.errors`),
> and a shared rule's `ConstructionError` holds of it.

## Values

> [spec:pgorm:req:python.values+7]
> The kinds `bindings.values` names MUST cross as these Python types: `bool`;
> `int` for every integer kind, held to its kind's range; `float` for `f32`
> and `f64`, signed zero and the non-finite values included; `str` for `text`
> and `char`; `bytes`; `decimal.Decimal`, never through a float; `uuid.UUID`;
> JSON as the Python values `json` reads; `datetime.date`, `datetime.time`, a
> naive `datetime.datetime` for `datetime` and an aware one in UTC for
> `datetime_utc`, each to the microsecond; `pgorm.Interval`; `str` for an
> enum's label, `ipnetwork` and `mac_address`; and `list` for an array.
>
> A plain Python value is inferred as the one kind its type leaves: `bool` as
> `bool` — never the integer path — `int` as `i64`, `float` as `f64`, `str` as
> `text`, `bytes`, a naive `datetime` as `datetime`, `date`, `time`,
> `Decimal`, `UUID` and `Interval` as their kinds, a `dict` as `json`, and a
> `list` as an array of the one kind its items other than `None` infer as,
> each `None` an SQL NULL element. An empty or all-`None` list, a list of
> several kinds or of lists, a `tuple`, which reads as a row, an aware
> `datetime`, `None` and anything else MUST raise `ConstructionError` asking
> for an explicit kind; integer overflow is an explicit error. A temporal
> value keeps its declared timezone policy and precision or is refused.
>
> A range is `pgorm.Range(lower, upper, bounds="[)")` or
> `pgorm.Range.empty()`, and a multirange `pgorm.Multirange(ranges)`:
> immutable native classes holding Python bound values, `None` on a side being
> no bound, so that side's bracket is always `(` or `)`. An interval is
> `pgorm.Interval(months, days, microseconds)`, immutable and hashable, each
> field an exact `int` within its width and with its own sign; a
> `datetime.timedelta` is not one, having no months and keeping only its days
> signed.

> [spec:pgorm:req:python.value-tags+1]
> `Value.snapshot()` MUST serialize a value for the generated campaign
> (`generative.format`), preserving its kind, its tags and special values: SQL
> NULL apart from JSON null, typed NULLs and empty arrays, integer widths, an
> enum's type, a temporal variant, non-finite floats and signed zero. Each
> integer, an interval's three fields included, is written as decimal text; a
> range as `{"empty": true}` or its two bounds' own payloads (`null` for no
> bound) and its brackets; and a created range with its name, schema and
> subtype beside its text.

> [spec:pgorm:req:python.records]
> A statement's rows MUST be `Record`s: read-only mappings from column name to
> value, in column order, whose `fields` describe each column (`Field`: its
> name, index, table OID, column number, type OID and type name), and whose
> `tagged(name)` gives one column as a `Value` carrying its kind. A model's
> rows are `ModelRecord` mappings keyed by field, with `tagged(name)` and the
> record's model. A registered entity's rows are `EntityModel`s keyed by SQL
> column name, whose `into_active()`, `with_value(column, value)` and
> `tagged(column)` are the entity operations on the record. A change is a
> `Change` of `old` and `new`, an upsert an `Upserted` that is `Inserted` or
> `Updated`, and a notification a `Notification`.

## Expressions and statements

> [spec:pgorm:req:python.expressions+7]
> `Expr` has no `__getitem__`, which would make it iterable to Python;
> `at(index)` and `slice(lower, upper)` subscript instead. `Expr`'s native
> methods are one `#[pymethods]` block per operator family, each in its own
> module (PyO3's `multiple-pymethods`), and its stub inherits one stub-only
> mixin per family, each in its own `.pyi`: the mixins exist for the type
> checker alone, so the runtime class has no base but `object` and defines
> every method itself, and the signature test checks each mixin's methods
> against it. A model's and a registered entity's column give their plain
> expression through `expr`.

> [spec:pgorm:req:python.truth-testing+5]
> Every class the package defines MUST raise `ConstructionError` on Python
> truth testing — `bool()`, `if`, `not`, `and`, `or` — so `if statement:` or
> `statement or fallback` fails where it is written instead of reading the
> object as true: a statement, an expression, `Compiled`, every part either
> is built from (a table, a FROM item, a join kind, a WITH clause, a window
> or frame, a conflict or MERGE action, a sort direction or NULL ordering, an
> SQL/JSON clause, a pipeline and its sources), a schema object, a
> registered entity's or graph's queries and writes, with the views
> generated code wraps them in, every stream — `RowStream`,
> `CopyOutStream`, `CopyRowStream` and `Listener` — whose truth would say
> nothing of whether rows remain, so `if rows:` fails as well, and each
> `Notification`. Exempt are values (`Value`, the range types, `Interval`
> and `TypeName`, compared with `==` as data), rows, models and
> what a write answers, pools, connections and transactions and the private
> native handles under them and under the streams, registrations and model
> declarations, the metrics layer's collectors and what a query hook is told
> (`QueryContext`, and `QueryFingerprint`, compared with `==` as data,
> so `if query.fingerprint:` tests for one; `bindings.metrics`), and
> exceptions. A test MUST enumerate every class the
> package's modules define, so a class that neither refuses nor is exempt for
> a stated reason fails it.

> [spec:pgorm:req:python.raw]
> Raw SQL MUST be an explicit application API with a declared SQL template and
> separate values, retaining pgorm's execution semantics. An owned-template
> substitution API, if exposed, MUST invoke the Rust lexer/substitution path.
> Neither API may be an implicit fallback for unsupported builder operations.
> Campaign-generated protected queries MUST obey their stricter raw-slot rules.

> [spec:pgorm:req:python.pipeline+1]
> `pgorm.pipeline.null()` MUST be `literal(None)` under a name of its own; a
> pipeline's own terminals are both bindings' (`bindings.pipeline-terminals`).

## Execution and lifecycle

> [spec:pgorm:req:python.runtime+1]
> Database I/O MUST have an asyncio-compatible awaitable interface on the
> shared runtime (`bindings.runtime`). Native waiting MUST NOT block the
> Python event loop or hold Python interpreter access unnecessarily. Objects
> MUST define interpreter, thread and event-loop ownership: a pool belongs to
> the event loop running when it was built, and use from another loop, or
> other unsupported cross-owner use, MUST raise `LifecycleError` rather than
> deadlock or panic.

> [spec:pgorm:req:python.lifecycle+3]
> `Pool(dsn, *, tls, ca, cafile, max_size, connect_timeout, acquire_timeout,
> statement_cache_size, recycle, statement_cache_channel)` builds a pool,
> `ca` the PEM certificates to trust as a `str` or `bytes`, `cafile` naming a
> PEM file to read them from, never both, and the timeouts given in seconds;
> `await connect(dsn, **options)` gives one whose server answered. A pool, a
> connection and every stream MUST be closable by `await close()` (`aclose()`
> on a stream or listener) and MUST be asynchronous context managers that
> close on exit; `pool.connection()` lends a connection for an `async with`
> block and `await pool.acquire()` lends one to close explicitly.

> [spec:pgorm:req:python.transactions+3]
> A transaction MUST be an `async with` block: `pool.transaction()` and
> `connection.transaction()` take the keyword arguments `mode` and
> `isolation`, and `transaction.transaction()` opens a nested savepoint, which
> takes neither. A `Transaction` is not itself a context manager. A block that
> exits normally commits, or releases its savepoint; an exceptional or
> cancelled block rolls back, or discards the connection if its state cannot
> be recovered, and its exception propagates. A failed `COMMIT` MUST be raised
> from the block's exit with its database error. A failed `ROLLBACK` MUST be
> raised with its database error in place of the block's exception, carrying
> that as its `__context__`, unless the block's exception is a cancellation or
> an interpreter exit, which propagates unreplaced.

> [spec:pgorm:req:python.cancellation+1]
> Cancellation is asyncio's: a cancelled task, a timeout, an abandoned
> iterator and pool shutdown MUST release what they own and behave as
> `bindings.cancellation` requires, raising `asyncio.CancelledError`, which
> the package re-exports as `pgorm.CancelledError`. Tests MUST show that
> cancellation and shutdown finish within declared budgets and leave no
> background work using a closed interpreter or transaction. A COPY cancelled
> before it finishes aborts; one cancelled while finishing, when its outcome
> is unknown, discards its connection or transaction.

## Direct builder integration

> [spec:pgorm:req:python.direct-builders]
> An integration suite MUST install a built wheel in a clean Python environment
> and import its declared public module. Using only public bindings, it MUST
> compose SELECT with joins and nested predicates, INSERT, UPDATE and DELETE
> over application tables whose names are supplied at runtime. Within one
> loaded extension build it MUST vary both builder structure and values, inspect
> SQL and tagged parameters, execute against PostgreSQL and check returned rows
> and affected counts against the fixture's expected state. Equivalent Rust
> builder operations MUST establish SQL/parameter parity for these cases.
> The installed suite MUST pass without an HTTP adapter, scanner or query
> dispatcher service and without invoking a compiler or package build per
> query. It MUST exercise both literal and bound value paths. Fixture creation
> MAY use separate test setup; the queries under test MUST use the builders.
> The suite MUST have a documented command and produce failing exit status
> for construction, parity, execution or result mismatches. Broader package
> acceptance MUST retain this suite as required evidence.

## Downstream development and release

> [spec:pgorm:req:python.codegen+3]
> The Python code generator (`bindings.codegen`) MUST create concrete Python
> wrappers and matching type information for downstream Rust entities and
> selected graph shapes, and generated modules MUST declare their
> compatibility with the binding's registry and pgorm build. A field's Python
> type follows its reflected Rust type: each Rust value type `pgorm.Value`
> carries maps to the Python type its values decode as — an `Interval` field
> to `pgorm.Interval` whatever the column's field qualifier, bound under the
> `interval` kind, and an array of them to `list[pgorm.Interval]` — and a
> type outside them to `Any`. A field over a column whose type is a domain
> types and binds as the domain's base does, the domain's name riding in the
> Rust entity's column definition: a domain over an integer is an `int` bound
> as `i32`, one over an enum the enum's `str`, one over an array a `list`.

> [spec:pgorm:req:python.typing+2]
> Public Python APIs MUST ship typing information and py.typed, with callable
> signatures, awaitable/iterator results, optional values and error behavior
> documented. A COPY statement's format is a type parameter,
> `CopyFrom[Literal["binary"]]` or `CopyTo[Literal["binary"]]` for the row
> COPY. So is the kind of row a MERGE action takes: `MergeAction.delete()` is
> a `MergeAction[Literal["target"]]`, `insert_defaults()` a
> `MergeAction[Literal["source"]]` and `do_nothing()` both, and an arm on a
> target row takes a `MergeUpdate` or the first, one on a source row a
> `MergeInsert` or the second, so a checker refuses an action an arm cannot
> take (`bindings.merge`). Static-checker examples and runtime signature tests MUST agree with
> the installed extension. Documentation MUST show an ordinary schema, CRUD,
> joins, a transaction, a pipeline, streaming and the optional compiled
> entity workflow, without requiring the security harness.

> [spec:pgorm:req:python.distribution]
> CI MUST build and install wheels and a source distribution in clean
> environments for the declared support matrix, exercising import, native
> loading, a real database operation, TLS configuration and typing artifacts.
> Public dependencies, licenses and linked native components MUST be included
> in package metadata/notices. Platform, Python ABI and free-threading support
> MUST be claimed only for tested combinations. Artifacts MUST be ready for
> review without uploading to package registries as part of ordinary tests.

> [spec:pgorm:req:python.acceptance+1]
> Initial acceptance MUST include the direct-builder integration suite and
> public-API tests against PostgreSQL for an application schema outside the
> harness fixtures, lossless conversion and boundary rejection cases,
> runtime/lifecycle/cancellation tests, compiled
> entity/graph registration tests and install/type-check tests of built
> artifacts. Rust/Python parity tests MUST cover every claimed API family and
> execution path. Default Rust builds MUST still work without Python tooling.
> nspec MUST track Rust bindings, Python sources and their tests; documentation
> alone MUST NOT count as implementation or verification evidence.

## Design references

- [PyO3 class restrictions](https://pyo3.rs/main/class#restrictions) explain why
  Rust generic entity types need concrete registrations.
- [PyO3 parallelism](https://pyo3.rs/main/parallelism) describes interpreter
  attachment when native work runs independently.
- [Maturin](https://www.maturin.rs/) documents native Python package builds.

Dependency versions and support matrices are implementation deliverables;
these references do not select moving upstream revisions as release pins.
