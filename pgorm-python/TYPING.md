# Types and a runnable application

The installed package includes `py.typed`, native `.pyi` declarations and typed
Python resource/model wrappers. Use CPython 3.14. The public API is checked with
the latest mypy release in strict mode against an installed wheel.

[The ordinary application](examples/application.py) uses only `import pgorm`
and its `schema` and `pipeline` modules. It creates two uniquely named tables,
inserts and updates records, joins them, commits a transaction while rolling
back a nested savepoint, merges one table into the other and reads the rows
the MERGE returned, selects through a pipeline, streams results and deletes a
record. Its `finally` blocks remove its own tables. It needs a
PostgreSQL database on which the supplied account can create and drop tables.

With the wheel installed in your current Python environment, run it from the
repository root:

```sh
export PGORM_TEST_DSN="$DATABASE_URL"
python -I pgorm-python/examples/application.py
uv tool run --from mypy@latest mypy --strict \
  --python-executable "$(command -v python)" pgorm-python/examples/application.py
```

The output includes the two joined rows, the merged row with its
`merge_action` of `UPDATE`, streamed IDs `[1, 2]`, one deleted record and
transport `in-process`. The example needs no HTTP service or
security harness. See [schema](SCHEMA.md), [statements](STATEMENTS.md),
[transactions](TRANSACTIONS.md), [pipelines](PIPELINE.md) and
[stream ownership](RESULTS.md) for the corresponding API details.

## Return types and ownership

| Operation | Result after `await` |
| --- | --- |
| `pool.execute(query)`, `connection.execute(query)`, `transaction.execute(query)` | `int` affected rows |
| `query(query)` | `list[Record]` |
| `one(query)` | `Record`; raises `DecodeError` unless exactly one row exists |
| `optional(query)` | `Record \| None`; raises `DecodeError` if more than one row exists |
| `pool.stream(query)`, `connection.stream(query)` | `RowStream`, an `AsyncIterator[Record]` and async context manager |
| `copy_in(statement, source)` on a pool, connection or transaction | `int` rows inserted |
| `copy_out(statement)` on a pool, connection or transaction | `CopyOutStream`, an `AsyncIterator[bytes]` and async context manager |
| `copy_in_rows(statement, rows)` on a pool, connection or transaction | `int` rows inserted; `statement` is a `CopyFrom[Literal["binary"]]` |
| `copy_out_rows(statement)` on a pool, connection or transaction | `CopyRowStream`, an `AsyncIterator[tuple[object, ...]]` and async context manager |
| `pool.listener(capacity=1024)` | `Listener`, an `AsyncIterator[Notification]` and async context manager |
| `notify(channel, payload)` on a pool, connection or transaction | `None` |
| `pool.transaction()`, `connection.transaction()`, `transaction.transaction()` | `AbstractAsyncContextManager[Transaction]`; the `Transaction` is open for its `async with` block only |
| `Model.find().all(executor)` | `list[ModelRecord]` |
| `Model.find().optional(executor)` | `ModelRecord \| None` |
| `Model.insert(values)`, `Model.insert_many(rows)` | `ModelInsert`: `on_conflict`, `returning_upsert(s)`, `returning`, `execute`, `inspect` |
| `Model.update(values)` | `ModelUpdate`: `where`, `all_rows`, `returning_change(s)`, `returning`, `execute`, `inspect` |
| `Model.delete()` | `ModelDelete`: `where`, `all_rows`, `returning`, `execute`, `inspect` |
| `Entity.find().all(connection_or_transaction)` | `list[EntityModel]` |
| `Pipeline.all(executor)` | `list[Record]` |
| `Pipeline.optional(executor)` | `Record \| None` |

`Pool.connection()` and every `transaction()` return async context managers;
use `async with` directly. A `Transaction` is not one itself; its block's exit
decides its outcome. Queries and builder methods are synchronous and
immutable until an execution method is called. Native methods returning an
`Awaitable[T]` must be awaited just like the Python `async def` facade methods.
The stream must be closed or used with `async with` when iteration can stop
early. A transaction reserves its connection; a savepoint reserves its parent
transaction until it finishes. Transaction row streaming is not exposed.

A model write has only its own kind's methods, so mypy reports an insert's
`where` or an update's `on_conflict` as an `attr-defined` error.

A MERGE action carries the kind of row it takes as a type parameter:
`MergeAction.delete()` is a `MergeAction[Literal["target"]]`,
`insert_defaults()` a `MergeAction[Literal["source"]]` and `do_nothing()`
both. `when_matched` and `when_not_matched_by_source` take a `MergeUpdate` or
the first, `when_not_matched` a `MergeInsert` or the second, so mypy reports an
insert given to a matched arm as an `arg-type` error before the runtime's
`ConstructionError` would.

The stubs mark positional-only parameters and keyword-only options. An opaque
native result such as `Record`, `Compiled` or `Expr` is obtained from its
builder/operation, a DDL statement such as `CreateTable` from its function
(`create_table`), a conflict action from `Conflict`'s functions and a MERGE
action from `MergeAction`'s. Its stub constructor takes `Never` to reject
direct construction, matching the native runtime's `TypeError`.

## Dynamic columns and concrete application models

`Record` and runtime `ModelRecord` are mappings with dynamically named fields.
Their values are `Any`: a type checker cannot infer a SQL projection or a
runtime schema declaration. Optional record results still require checking for
`None`. Explicit `Value` instances preserve runtime integer widths, typed
NULLs, array identities and qualified enums; Python's `int` annotation does not
encode SQL widths or bounds. These remain runtime validation rules.

For concrete model properties and typed graph tuples, follow the optional
[compiled entity and code generation workflow](CODEGEN.md). Its final wheel
exports an application module such as `pgorm.app`, with properties including
`int`, `str`, `str | None` and graph slots such as `NoteModel | None`. Generated
write setters accept their reflected type or an explicit `Value`. Unknown
custom Rust representations and JSON retain `Any`. The generated module is
checked against its installed extension's version and registry metadata.

## Errors

Python typing does not declare checked exceptions. An argument of a type the
call does not take raises `TypeError`; a value that cannot become what pgorm
sends, or a builder state that cannot run, raises `ConstructionError`; a
listener capacity that is an `int` but not a positive one raises `ValueError`.
Unsupported installed capabilities raise `UnsupportedCapabilityError`, a
`ConstructionError`. Network/TLS setup uses `ConnectionError`, server errors
use `DatabaseError`, and failed decoding, a wrong row count from `one` or
`optional` among it, uses `DecodeError`. `DatabaseError` exposes optional `sqlstate`, `message`,
`severity` and the diagnostic fields `detail`, `hint`, `schema`, `table`,
`column`, `datatype` and `constraint`. Server errors populate these fields; a
`DatabaseError` raised without the server's answer leaves them as `None`. Use
`str(error)` for the exception text in either case.

A constraint violation raises a subclass of `DatabaseError` named for it:
`UniqueViolationError` (`23505`), `ForeignKeyViolationError` (`23503`),
`CheckViolationError` (`23514`), `NotNullViolationError` (`23502`) and
`ExclusionViolationError` (`23P01`, which a `WITHOUT OVERLAPS` key raises
too). A table's constraint is named in `constraint`, a `NOT NULL` column in
`column`, and a domain whose `CHECK` or `NOT NULL` refused the value in
`datatype`.

Closed resources, concurrent use of a reserved connection and use from another
event loop raise `LifecycleError`; configured deadlines raise pgorm's
`TimeoutError`. These errors derive from `PgormError`. Task cancellation remains
`asyncio.CancelledError` and should normally propagate. Rust panic exceptions
are not converted into input-validation errors. See [connections](README.md#connections)
and [transaction cleanup](TRANSACTIONS.md) for resource behavior on failure.

## Verify the installed package

With an existing database and a wheel installed in `target/python-check`:

```sh
target/python-dev/bin/python pgorm-python/checks/typing.py \
  --python target/python-check/bin/python
```

Alternatively, wrap that command with `tests/with_postgres.py` to provision a
disposable test database. The checker compares installed native signatures
with shipped stubs, verifies `py.typed`, runs strict mypy on the application,
checks each expected diagnostic in the invalid consumer, and executes the
application. It removes `PYTHONPATH`/`MYPYPATH`, uses isolated Python launches,
and records the installed native path and stub hashes in
`target/python-typing/summary.json`. The separate
[code generation checker](CODEGEN.md#verify-the-workflow) builds and checks the
concrete application types, including invalid assignments and optional slots.
