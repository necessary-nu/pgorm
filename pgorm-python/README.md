# pgorm for Python

The `pgorm` Python package loads an optional PyO3 extension over pgorm's Rust
builders. Calls run in the Python process. PostgreSQL uses its native protocol;
there is no HTTP adapter or query dispatcher.

The package provides runtime SELECT/CRUD and schema builders, asyncio database
access, transactions, models, pipelines and compiled entity/graph registrations.
`pgorm.capabilities()` reports the operations and registrations present in the
installed build. The [specification](../docs/spec/python.md) defines the contract;
the [acceptance guide](ACCEPTANCE.md) maps it to installed-package tests and
independent Rust comparisons.

Use [explicit schema builders](SCHEMA.md) for tables, keys, indexes, types,
sequences, domains and views,
or to generate schema statements from a registered Rust entity.
Use [native transactions and savepoints](TRANSACTIONS.md), each open for one
`async with` block that commits or rolls it back, across builders and
registered models.
The [typed application example](TYPING.md) combines schema, CRUD, a join,
transactions, a pipeline and streaming in one runnable Python program.

## Build and install

From the repository root, using CPython 3.14:

```sh
uv venv target/python-dev
uv pip install --python target/python-dev/bin/python 'maturin>=1.15,<2'
CARGO_TARGET_DIR=target/python-codegen-seed \
  target/python-dev/bin/maturin build --manifest-path pgorm-python/Cargo.toml \
  --interpreter target/python-dev/bin/python --out target/python-dist --locked
uv pip install --python target/python-dev/bin/python target/python-dist/*.whl
target/python-dev/bin/python pgorm-python/tests/with_postgres.py \
  target/python-dev/bin/python -m unittest discover -s pgorm-python/tests
```

`maturin sdist --manifest-path pgorm-python/Cargo.toml --out target/python-dist`
builds the source distribution. Build and install commands do not publish to a
registry. `support.json` records the ABI and candidate platform matrix; release
artifacts require the installation checks on each claimed platform. Registry
name availability must be checked before publication.
See [distribution verification](DISTRIBUTION.md) for isolated wheel/source
builds, platform evidence, CI and the shipped dependency notices.

Python support has its own Cargo workspace, lockfile and build command. A normal
`cargo build --workspace` at the repository root does not load PyO3 or Python
configuration. CPython uses version-specific wheels; free-threaded builds and
subinterpreters are outside the supported matrix.

```python
import pgorm

print(pgorm.__version__)
print(pgorm.capabilities())
```

## Values

`Value(data, kind)` constructs an immutable native Rust value. Explicit kinds
retain integer widths, typed NULLs, temporal variants and qualified enum names;
`Value.array` and `Value.json` preserve array identity and JSON null. See the
[conversion table and policies](VALUES.md) for exact limits and examples.

```python
from pgorm import Value

value = Value(42, "i16")
assert value.value == 42
assert value.snapshot()["type"] == {"kind": "i16"}
assert Value.json(None) != Value.null("json")
```

## Expressions

Compose native expressions with `col`, `bind`, `literal`, `Condition` and
`call`. Operations return reusable, immutable Rust builder state. Inspection
returns Rust-built SQL and tagged parameters:

```python
from pgorm import col, literal

predicate = (col("active") == True) & (col("score") > literal(10))
compiled = predicate.inspect()
print(compiled.sql)
print([value.snapshot() for value in compiled.params])
```

See [expression construction and Rust API mappings](EXPRESSIONS.md) for
conditions, functions, SQL/JSON, literal/bound paths, casts and ownership
semantics.

## Statements

`Table`, `select`, `insert`, `update` and `delete` compose the Rust statement
builders over application names supplied at runtime. `inspect()` returns the
SQL and parameters produced by that builder state. See [runtime statement
examples](STATEMENTS.md) for joins, grouping, set operations, write guards,
conflict actions, RETURNING and the explicit `RawSQL` template API.

`await pool.execute(query)` returns affected rows. `query`, `one`
and `optional` return detached records; `stream` opens an async iterator.
See [execution, result types and stream ownership](RESULTS.md) for examples,
PostgreSQL decoding limits and cancellation behavior.

`CopyFrom` and `CopyTo` build COPY statements: `copy_in` streams bytes from a
sync or async source into a table, and `copy_out` streams a table's or a
query's data out as `bytes`; in the binary format `copy_in_rows` and
`copy_out_rows` carry rows of values instead. See [COPY](STATEMENTS.md#copy)
for the formats, chunking, abort and cancellation behaviour.

`pool.listener()` opens a dedicated connection that receives `LISTEN`
notifications as an async iterator, and `notify` on a pool, connection or
transaction sends one. See [notifications](RESULTS.md#notifications) for the
buffering, end and cancellation behaviour.

The [direct builder integration suite](DIRECT_BUILDERS.md) installs a wheel in
a fresh Python environment, runs 56 application query programs against
PostgreSQL, and compares their SQL and tagged parameters with independent Rust
builders. It provides the focused proof of direct Python access without HTTP.

[Runtime model descriptors](MODELS.md) declare table names, field mappings,
types, nullability and primary keys entirely in Python. They validate native
statement writes and return records using the declared field identities. A
[disjoint model](MODELS.md#disjoint-models) stores a value of one of several
kinds across a base model and a model per kind, its statements built by
pgorm's own `DisjointShape`.

Applications with Rust entities can also [compile their entity registrations
into an application wheel](ENTITIES.md). Python then uses their real typed
queries, models, ActiveValue states and write hooks through `pgorm.entity`.
Compiled [typed graphs and keyset cursors](GRAPHS.md) retain `SelectGraph`'s
required/optional join slots and model decoding through `pgorm.graph`.
The [application code generator](CODEGEN.md) creates the registration crate,
concrete Python model wrappers and matching stubs from compiled Rust metadata.

The [pipeline API](PIPELINE.md) composes native relational stages, including
grouping, windows, bound callbacks and set operations. Registered source tuples
decode actual Rust models, including absent sources in right and full joins.

## Connections

Construct and use resources inside one running asyncio loop. Database waiting
runs on the shared Tokio runtime. Connection checkout supports cancellation and
an acquisition timeout; one connection rejects concurrent operations.

```python
import asyncio
import os
import pgorm

async def main():
    async with pgorm.Pool(os.environ["DATABASE_URL"]) as pool:
        async with pool.connection() as connection:
            assert await connection.ping()

asyncio.run(main())
```

TLS verifies certificates and hostnames against the PEM certificates passed as
`ca` (a `str` or `bytes`), or read from the file `cafile` names, when one is
given — never both — and otherwise against the roots the platform's
certificate store trusts: on macOS the system roots and any certificate the
user or an administrator has marked trusted in Keychain, on Windows the root
certificate store, and on Linux and other Unix systems the CA bundle and
certificate directory OpenSSL would use, such as `/etc/ssl/certs`.
`SSL_CERT_FILE` and `SSL_CERT_DIR`, when set, replace the platform's store
rather than adding to it. The package carries no root list of its own, so a
pool trusts what the host trusts. The store is read when a verifying `Pool` is
constructed, and a store holding no usable root certificate (a minimal
container image without a CA bundle, or `SSL_CERT_FILE` naming a missing or
empty file) raises `ConstructionError` there, naming what was wrong; it never
yields a pool that trusts nothing, or one that skips verification. Pass
`ca` or `cafile`, or install the host's CA bundle. Set `tls="disable"` or include
`sslmode=disable` in the DSN for an explicitly plaintext connection. TLS
verification never falls back to plaintext. Pool sizing, connect/acquisition
deadlines, statement-cache bounds and verified/fast recycling are keyword
options on `Pool`. After DDL that a cached plan does not see, such as a
domain's changed default, `pool.clear_statement_caches()` makes every
connection prepare its statements again: an idle one before it is next lent,
and one checked out at the time once it is returned and lent again. That
reaches only this process's pool. A pool built with
`statement_cache_channel=pgorm.STATEMENT_CACHE_CHANNEL` also clears whenever a
notification arrives on that channel, as pgorm-migration's runner sends after
a migration from any process: it keeps one connection of its own listening
from its first acquisition until it closes, reconnects if that connection
drops, and clears once each time it listens again. The option is off by
default; any channel name of 1 to 63 bytes works, and a notification's
payload is ignored.

Closing a pool rejects waiters and cancels operations on its checked-out
connections. Closing a connection releases it. If an operation is cancelled
while its database state is uncertain, its connection is discarded. Cancellation
does not promise rollback of an already submitted write. Context managers close
their resources on normal and exceptional exit; pools, connections and streams
also have an explicit `close()`/`aclose()`, while a transaction ends only with
its block. Resources cannot migrate to another event loop.

Construction, capability, connection, database, decode, timeout and lifecycle
failures have distinct exception classes under `PgormError`. PostgreSQL server
errors retain `sqlstate`, message and structured diagnostics with connection
credentials redacted. Asyncio cancellation remains `asyncio.CancelledError`.
Internal Rust panics retain PyO3's identifiable panic exception and are not
classified as invalid application input.

The test wrapper provisions its own PostgreSQL server and CA certificate, runs
the supplied command with `PGORM_TEST_DSN` and `PGORM_TEST_CA`, and removes the
server afterward. It requires Docker and OpenSSL only for testing. Existing
fixtures can instead supply those two environment variables directly.
