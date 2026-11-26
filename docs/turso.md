# Experimental native Turso

pgorm can open a Turso database file in-process through its existing connection
APIs. Enable the optional native build, then pass a path or `:memory:` where you
would pass a PostgreSQL connection string. PostgreSQL URLs and libpq settings
continue to select PostgreSQL.

This support is experimental and covers a tested subset of PostgreSQL
behavior. The [compatibility matrix](turso-compatibility.md) records supported
operations, explicit refusals and known differences at the pinned engine
revision. [The performance report](performance/turso-results.md) records latency,
allocations and streaming memory, with reproducible workloads and the limits
of the measurements.

## Rust

Enable the feature on the existing git dependency:

```toml
[dependencies]
pgorm = { git = "https://github.com/necessary-nu/pgorm", features = ["turso"] }
```

Parse the target into the existing `Config` and use the ordinary pool:

```rust,no_run
use pgorm::Connection;

async fn example() -> Result<(), Box<dyn std::error::Error>> {
    let pool = pgorm::connect("./app.db".parse()?);
    let connection = pool.get().await?;
    let row = connection.query_one("SELECT 1", &[]).await?;
    assert_eq!(row.get::<_, i32>(0), 1);
    Ok(())
}
```

`connect_with` and the builder variants retain their existing pool, cache,
timeout and hook options. Construction is synchronous and lazy: first
acquisition opens the database and reports opening or creation-hook failures.
Use `./` to disambiguate a filename resembling a libpq setting or URI.
Malformed recognized PostgreSQL strings never fall back to creating files.

Direct driver callers also use the existing `Config::connect` or
`pgorm_driver::connect`, receiving the same `Client` and lifecycle `Connection`
types. Spawn or drive that connection future. No local server is required.

`connect_read_only` over a file opens it read-only at the first acquisition:
its connections implement `Connection` alone, and the engine refuses every
write with SQLSTATE `25006`. The engine keeps one database per file for the
whole process, whatever access a later open asks for, so opening a file
read-only while this process holds it read-write, or the reverse, is refused
rather than handed the other access. `:memory:` cannot be read-only, and is
refused when the pool is built.

## Python, Node.js and Deno

Build the optional package described in the
[Python installation instructions](../pgorm-python/README.md#experimental-native-databases)
or [JavaScript distribution instructions](../pgorm-napi/DISTRIBUTION.md#experimental-native-turso).
Python enables `extension-module,turso` when building its wheel; the JavaScript
build and package commands accept `--turso`. These are build choices, not
runtime options that enable an ordinary binary.

| Language | Lazy pool | Open and validate before returning | Build availability |
| --- | --- | --- | --- |
| Python | `Pool("./app.db")` | `await connect("./app.db")` | `capabilities()["features"]` contains `"turso"` |
| JavaScript / Deno | `new Pool("./app.db")` | `await connect("./app.db")` | exported `features` contains `"turso"` |

Use the existing models, raw statements, transactions, row streams and close
methods. Python also accepts a `Path` for a filesystem target; use the string
`":memory:"` for memory. JavaScript supports `await using` for pool lifetime.
Without the native build, Python raises `UnsupportedCapabilityError` and
JavaScript raises `ConstructionError` before opening a file. Rust recognizes
the path but reports unavailable native support when connecting.

## Lifetime and typing

Connections within one pool share the native database, including `:memory:`.
Separately constructed memory pools are isolated. Repeated direct connects on
one retained driver `Config` share a database; cloning the config copies its
settings and starts a separate owner. A file clone still names the same file;
a memory clone starts empty when opened.

A file database may also create `app.db-log` and `app.db-wal`. Manage these
companion files as part of the database; copying or deleting only `app.db` is
not a complete backup or cleanup procedure.

Rust `DatabasePool::close` prevents new acquisitions and disposes idle
connections; checked-out connections remain usable until released. Python and
JavaScript pool close also closes lent connections and cancels their active
operations, transactions and streams. Native worker cleanup follows the driver
lifecycle; closing a pool does not promise that every native worker has exited.
Creation hooks must leave the client idle: an open transaction, closed client
or retained unfinished work refuses acquisition.

Native parameters need known types. ORM-built queries and binding values retain
their supported type information, including typed nulls. Direct driver code
uses `prepare_typed` or the typed query methods. Arbitrary erased `dyn ToSql`
parameters cannot rely on PostgreSQL's contextual inference. Unsupported types
remain unsupported when their value is null.

The initial codecs cover booleans, signed integer representations, text, bytes
and finite floats. Result metadata is established before reading rows. Native
compiler erasure can change identities or precision, including REAL, serial
aliases and unparameterized NUMERIC. Consult the
[scalar and metadata tables](turso-compatibility.md#scalar-codecs) before choosing
stored types; a successful cast does not establish PostgreSQL type fidelity.

Transactions, savepoints, prepared statements, cancellation and bounded streams
use the existing APIs. Unsupported operations fail explicitly. In particular,
Clean recycling, notification-based cache invalidation, native listeners and
streaming COPY are unavailable; migration prerequisites and other SQL features
also have documented engine gaps. Bindings refuse explicit TLS/network options
for file targets. Engine errors preserve their SQLSTATE; local refusals do not
invent one.

A failed batch can retain earlier completed statements. A conversion or
invalidation error after execution does not prove a write had no effects.
The cache can prepare a later operation afresh; it does not blindly replay the
failed write. The matrix gives the transaction and cleanup boundaries.

## Provenance and upgrades

The native dependencies are unmodified Turso `0.9.0-pre.6` sources pinned to
[`adf3b41f99ebb6b1cdd1b60ae08b483bc45686fa`](https://github.com/tursodatabase/turso/tree/adf3b41f99ebb6b1cdd1b60ae08b483bc45686fa):
`turso_pg` in `postgres/frontend` and `turso_core` in `core`, both declaring MIT.
The driver is a maintained fork of tokio-postgres; its separate upstream
revision and update procedure are in [UPSTREAM.md](../crates/pgorm-driver/UPSTREAM.md).

The Python wheel and JavaScript packages carry dependency inventories and
third-party notices for the optional graph, including bundled native sources.
Their [Python](../pgorm-python/licenses/README.md) and
[JavaScript](../pgorm-napi/licenses/README.md) supplement records preserve exact
license sources and hashes. The notice inventory is a superset of a default
build's linked dependencies.

When upgrading the engine:

1. Update both optional driver dependencies to the same exact revision, refresh
   the reachable committed lockfiles, and update provenance. Review changes to
   prepared metadata, schema invalidation, transaction state and cleanup APIs.
2. Run minimal builds with and without `turso`, the feature/fork graph checks,
   and the existing PostgreSQL suite in both feature states. Run native driver,
   ORM and paired compatibility tests; update specific mismatch assertions and
   matrix entries only after reproducing the new behavior.
3. Regenerate and verify both bindings' notices, then test installed default
   and native artifacts. Preserve source/wheel/tarball contents, feature
   metadata and type declarations alongside the binaries being tested.
4. After builds and tests finish, freeze the source snapshot and repeat the
   [performance workflow](performance/README.md#native-comparison-workflow).
   Compare repeated distributions under the same settings and investigate
   repeatable regressions before drawing conclusions.

The principal validation commands, run from the repository root with its
PostgreSQL test server configured, are:

```sh
cargo check -p pgorm-driver --lib --no-default-features
cargo check -p pgorm-driver --lib --no-default-features --features turso
cargo check -p pgorm --lib --no-default-features
cargo check -p pgorm --lib --no-default-features --features turso
cargo test -p pgorm --test turso_feature_tests --test fork_pin_tests
cargo test --workspace --lib --tests
cargo test --workspace --lib --tests --features pgorm/turso
cargo test -p pgorm --features turso --test turso_compatibility_tests
python3.14 pgorm-python/checks/notices.py --write
node pgorm-napi/checks/notices.js --write
python3.14 pgorm-python/checks/notices.py
node pgorm-napi/checks/notices.js
python3.14 pgorm-python/checks/turso.py
node pgorm-napi/checks/package.js all --partial
node pgorm-napi/checks/package.js all --partial --turso
```

The [experimental Rust CI lane](../.github/workflows/turso.yml) retains the
PostgreSQL suite in both feature states. Python's native package check builds
both wheel variants; JavaScript's package checks exercise Node and Deno.
`--partial` validates the current platform's packages, not every release
platform. Keep the ordinary binding PostgreSQL and type checks as well.
