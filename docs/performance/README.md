# Driver performance measurements

`driver_baseline` compares the direct `pgorm-driver` path with pgorm on the same
PostgreSQL server. It measures an existing connection separately from concurrent
pool acquisition. The separate native harness compares the embedded engine,
`pgorm-driver`, and pgorm using matched SQL and database settings.

[The recorded Turso results](turso-results.md) include the retained implementation,
the rejected batching experiment, raw evidence, and measurement limitations.
[The recorded Drizzle comparison](drizzle-results.md) measures pgorm's JavaScript
binding against Drizzle ORM and the raw Node.js drivers; its harness is
described [below](#javascript-pgorm-napi-against-drizzle-and-the-nodejs-drivers).

Run from the repository root, with the normal test server available:

```sh
cargo bench --bench driver_baseline -- --output target/driver-baseline.json
```

The harness reads `DATABASE_URL`, then `.env.local` and `.env` for values absent
from the environment, as the live test suite does. This is a server URL without
a database component. It creates a database with `pgorm-test-support`'s unique
run-scoped name and drops that database when the run finishes, including when a
measurement returns an error. It records server settings but does not change
them. The report contains no connection password or connection URL.

The release profile is required. Defaults are 100 warmup operations and 1,000
measured operations per workload, three rounds, four concurrent workers, four
pool connections, and 128 rows per streamed result. Options are `--warmup`,
`--samples`, `--rounds`, `--workers`, `--pool-size`, `--rows` (1–1,024),
`--source-root`, `--output`, and `--raw-samples true`. Cargo enables the root
crate's development features for this target; the actual feature values are
recorded in the report. Warmup must be positive: these workloads measure warm
connections and cached statements, not first-use setup.

The paired workloads are:

| Workload | Direct driver | pgorm |
| --- | --- | --- |
| `scalar` | Explicitly prepared scalar query | Same query through the connection's statement cache |
| `params16` | Prepared query with 16 `int8` parameters | Same cached query and parameters |
| `model_lookup` | Prepared primary-key lookup, decoding all three fields into the model | Entity lookup, SQL construction and derived model decoding |
| `update16` | Prepared update with 16 parameters | Same cached update and parameters |
| `stream` | Incremental rows, decoding all three fields | Ordered entity stream with model decoding |
| `transaction` | Begin, scalar query, commit | Closure transaction with the same query and pgorm's commit checks |
| `concurrent_pool` | `pgorm-pool` acquisition, cached prepare, query, release | `DatabasePool` acquisition, cached query, release |

Setup, preparation and warmup are excluded from the timed operations. A round
runs direct-driver workloads first, then pgorm; the following round reverses
that order. The concurrent workloads synchronize workers after warmup and
before timing starts. Each measured result is checked so an error or wrong
result cannot be reported as a faster successful query. Updates preserve the
fixture values across rounds. The report records p50, p95, p99, minimum and
maximum latency, elapsed throughput, and optionally every latency sample.

Run on an otherwise idle machine and server. A scoped database prevents name
collisions; it does not isolate CPU, storage, PostgreSQL buffers or checkpoint
activity. The report records other active server sessions at startup, not a
guarantee that the server stayed idle. Compare repeated runs with the same
compiler, feature set, runtime threads, server settings, transport, workloads
and concurrency. Judge changes against the observed variability instead of
using a percentage threshold from a different machine.

These runs do not instrument allocations, byte copies or memory residency.
`allocation_measurements` is explicitly `null`; latency improvements are not
evidence that allocations fell. Use separate profiling runs for those claims.
The source fingerprint hashes the not-ignored Rust sources and Cargo manifests
and lockfiles, including untracked inputs. Start/end fingerprints expose source
changes during a run; HEAD and working-tree status are recorded separately.
Compiler, flags and source metadata are inspected at run time, not embedded
compilation provenance. The executable's Git blob hash identifies the measured
binary. Preserve the report with its build command and source snapshot for a
comparison; a runtime compiler version alone does not prove which compiler
built that binary. Transport and the fixed four Tokio worker threads are also
recorded.

## Before native Turso dispatch

[postgres-before-turso.json](postgres-before-turso.json) records the vendored
driver at `14707a0eab0ad1daaf0f43065b5192a766a17e7c`, with this benchmark and its
manifest entry still in the working tree. No native backend dispatch had been
added. The recorded source fingerprint stayed unchanged during the measurement.

The run used an Apple M5 Pro, PostgreSQL 19.0 over local TCP, and the defaults
above. PostgreSQL reported `fsync`, `full_page_writes` and `synchronous_commit`
enabled. No other PostgreSQL client was active when the run started. The exact
command was:

```sh
cargo bench --bench driver_baseline --offline --target-dir /tmp/pgorm-perf-target -- --output docs/performance/postgres-before-turso.json
```

These are the medians of the three rounds' p50 latencies, in microseconds:

| Workload | Driver | pgorm |
| --- | ---: | ---: |
| Scalar | 96.00 | 94.58 |
| 16 parameters | 99.29 | 98.75 |
| Model lookup | 99.29 | 102.17 |
| 16-parameter update | 880.12 | 750.29 |
| 128-row stream | 158.12 | 161.17 |
| Transaction | 278.08 | 284.62 |
| Concurrent pool | 112.54 | 112.25 |

The pool runs reached approximately 34,100 operations per second on both paths.
The update p50 varied from 860 to 1,030 microseconds for the driver and 737 to
954 for pgorm; these storage-sensitive results do not establish a stable
performance ranking. This snapshot is a comparison baseline for subsequent
driver changes, not a universal throughput claim or a bound on acceptable
regression. The JSON retains each round's percentiles and throughput so that
the variation remains visible.

## Native comparison workflow

`benches/native` is a separate Cargo workspace so direct access to `turso_pg`
does not add engine dependencies to ordinary PostgreSQL builds. Its lockfile
uses the main workspace's dependency versions, including the exact Turso
revision. The default executable has no allocation instrumentation. Building
with `alloc-profile` replaces the global Rust allocator with a counting wrapper;
those timings must not be used to rank performance.

Native runs enable pgorm's ordinary default features plus `turso`; optional
`tests-cfg` and `metrics-fingerprint` are disabled. The PostgreSQL comparison
keeps the historical benchmark's development features, including fingerprinting,
in both current builds. These feature sets are recorded separately: comparisons
within each backend hold them constant.

The native harness runs the seven workloads above at all three layers. Model
lookup includes the ORM's one-row limit, and streams bind the same row-limit
parameter. The 16-parameter sum explicitly casts its final result to `BIGINT`:
the pinned native compiler does not resolve that arithmetic expression's result
type without the cast. Direct and driver operations reuse prepared statements; ORM
operations include SQL/value construction, cached preparation, and model
decoding as applicable. Every layer decodes all three model fields into the
same owned model. Writes alternate two equal-length strings so each iteration
changes the row while preserving its shape and numeric values.

Native defaults are 500 warmup operations, 5,000 samples, three rounds, four
workers, four pool connections, and 128 rows per stream. Layers rotate between
rounds. File and memory modes use separate temporary databases for each layer,
with identical schema and fixture data. Direct concurrent reads use one
retained connection per OS thread; driver and ORM concurrent reads include
pool acquisition, cached preparation, and Fast recycling. The direct case is
a lower bound without a pool, not a measurement of pool overhead alone.

Reports read sync mode, page size, cache size, MVCC state, and the passive
checkpoint option from native public getters. The I/O backend is identified
from the pinned `Database::io_for_path` implementation: `MemoryIO` or
`PlatformIO`, with no custom VFS. Checkpoint thresholds are not measured.
An engine-reported `SyncMode::Full` describes a setting; these benchmarks do
not verify power-loss guarantees or PostgreSQL-equivalent durability. Compare
adapter layers within one storage mode, not file-versus-memory or cross-engine
latency as though persistence guarantees were equal.

Builds and measurements are separate commands. With the source tree frozen,
create immutable binaries and their build record:

```sh
python3 docs/performance/run.py build \
  --artifacts /tmp/pgorm-performance-artifacts \
  --target /tmp/pgorm-native-perf-build
```

This builds current PostgreSQL feature-off and feature-on harnesses, native
latency and allocation harnesses, and copies the preserved pre-integration
PostgreSQL executable. It verifies that executable against the original report
and requires the original compiler version. The default reference path is the
one used for the recorded baseline; `--reference-binary` can name a preserved
copy. If that binary is unavailable, this historical comparison cannot be
reproduced by silently substituting a rebuild with current dependencies.

After every other build and database test has finished, run the comparison:

```sh
python3 docs/performance/run.py run \
  --artifacts /tmp/pgorm-performance-artifacts \
  --output /tmp/pgorm-performance-results
python3 docs/performance/run.py summarize \
  --input /tmp/pgorm-performance-results \
  --output /tmp/pgorm-performance-results/summary.json
```

The runner rotates the preserved, feature-off, and feature-on PostgreSQL
executables across three repeated processes. It also repeats native file and
memory runs, allocation profiles, and growing-stream memory profiles. Use
`--suite postgres|native|alloc|memory` to select one part. PostgreSQL retains its
original warmup, sample counts, SQL, and concurrency. Source fingerprints and
executable hashes must remain unchanged between build and measurement.

Allocation profiles report allocation and reallocation calls, requested bytes,
peak live requested bytes above the starting point, and retained-byte changes.
They include native worker threads and measurement coordination. Sample
storage is preallocated and report construction is outside the counters.
These are Rust heap measurements: C allocator activity, allocator fragmentation,
stacks, memory mappings, operating-system caches, and allocations bypassing the
Rust global allocator are outside the boundary. Byte copies and RSS remain
unmeasured.

The separate stream-memory mode reads 1,000, 100,000, and 1,000,000 rows. Driver
and ORM consumers initially pause for 50 ms to exercise producer backpressure,
then consume and check every row without retaining the result set. Compare
peak live storage across increasing result counts and account for the direct
engine measurement before attributing growth to the adapter.

The summary records distributions across rounds and process medians, including
ranges, standard deviations, and coefficients of variation. Allocation-build
timings are excluded from its latency table. Final conclusions require the
recorded run results and investigation of repeatable regressions; the presence
of this harness alone establishes no performance result.

For a focused native batching comparison, preserve the complete before artifact
directory and build the candidate into a different directory using the same
target cache. Then run:

```sh
python3 docs/performance/run.py build \
  --artifacts /tmp/pgorm-performance-artifacts-after \
  --target /tmp/pgorm-native-perf-build
python3 docs/performance/run.py compare \
  --before /tmp/pgorm-performance-artifacts \
  --after /tmp/pgorm-performance-artifacts-after \
  --output /tmp/pgorm-performance-batch-results
python3 docs/performance/run.py summarize \
  --input /tmp/pgorm-performance-batch-results \
  --output /tmp/pgorm-performance-batch-results/summary.json
```

This interleaves before/after scalar and 128-row stream measurements in both
storage modes, with separate latency and allocation executables. Three process
repeats each contain three rounds. Growing-stream memory profiles also repeat
three times per storage mode and variant. The command verifies matching compiler,
dependency identities, feature sets and profiles, preserves both build records,
records the runner SHA256 and start/end host conditions, and checks every result.
Runtime source inspection in the old executable describes the candidate checkout;
its separate build record identifies the actual compiled source. This focused
comparison does not rerun PostgreSQL workloads.

To isolate stream throughput, add `--stream-latency-only --repeats 4` to the
comparison command and choose a fresh output directory. This runs 16 processes,
retaining the direct engine control and alternating variant order for each
fixed storage mode: two repeats start with before and two start with after.
Python-only scheduling changes do not require recompiling the preserved
executables; each window records the exact runner SHA256 used.

## JavaScript: pgorm-napi against Drizzle and the Node.js drivers

`benches/drizzle` is an npm package of its own that measures pgorm's
JavaScript binding, `@necessary/pgorm` (a `file:` dependency on
`pgorm-napi`), against Drizzle ORM and the raw `pg` (node-postgres) and
`postgres` (postgres.js) drivers on `driver_baseline`'s workloads. Every side
is TypeScript, run by one Node.js process through type stripping, so each
side's own JavaScript and native code is what is measured: pgorm's addon is
part of pgorm's cost as each driver is part of Drizzle's. Rust is not one of
the sides.

Build the release addon, install the harness's locked dependencies, check its
types, and run it from its directory:

```sh
node pgorm-napi/scripts/build.mjs --release
cd benches/drizzle
npm ci
npx tsc --noEmit
node --expose-gc src/main.ts --require-idle true --label recorded
node src/main.ts summarize ../../target/drizzle-bench/<stamp>.json
```

The harness refuses an addon that is not the `addon` profile's build (the
release profile with fat LTO, as `build.mjs --release` writes it), matched by
its bytes against `target/addon/`; `--allow-unverified-addon true` runs one
anyway and records which profile, if any, it matched. `--require-idle true`
refuses to start, saying why, while a Time Machine backup runs, while the
one-minute load average is at or above `--idle-load` (4), or while any `cargo`
or `rustc` process runs. `--expose-gc` lets the harness collect garbage
between workloads, so one workload's garbage is not collected during the
next one's timed operations; without it the report says no collection ran.

The connection string is `PGORM_TEST_DSN`, or `DATABASE_URL` from the
environment, `.env.local` or `.env`, as pgorm-napi's live suite reads it: the
harness imports `scratchDatabase` from `pgorm-napi/tests/support.ts`, so its
database has the run-scoped name and the sweep of crashed runs' leftovers that
suite has, and it is dropped when the run ends, a failed check included.
Reports go to `target/drizzle-bench/<stamp>.json` with a Markdown summary
beside it, or to `--output`. They hold no password or connection string.

Defaults follow `driver_baseline`: 100 warmup and 1,000 measured operations
per workload, three rounds, four workers over a pool of four, 128 rows. The
options are `--warmup`, `--samples`, `--rounds`, `--workers`, `--pool-size`,
`--rows`, `--attribution-rows` (1,128,1024), `--cursor-batch` (100),
`--sides` and `--workloads` (comma lists, every one by default),
`--attribution` (true), `--raw-samples` (false), `--label`, and `--output`.

The sides are pgorm's raw statement path (`pgorm/raw`: SQL text and values on
a `Connection`) and its model path (`pgorm/orm`: `model`, `findByKey`,
`find().orderBy().limit()`, `update().where()`, as the JavaScript guide reads
and writes rows); Drizzle over node-postgres and over postgres.js, each with
its `.prepare()` (statements built once with `sql.placeholder`) and without
it; and each raw driver prepared (node-postgres's named query config,
postgres.js's default `prepare: true`) and unprepared (plain text;
`prepare: false`). pgorm prepares and caches every statement on its
connection and has no unprepared mode. After its workloads, each side's
session reads `pg_prepared_statements`, so the report records what each side
actually holds prepared on the server.

The workloads are `scalar`, `params16`, `model_lookup`, `update16` and
`transaction` as in `driver_baseline`; `rows_all` and `rows_stream` (the 128
rows of `bench_row` into objects, all at once or pulled one at a time: pgorm's
`stream()`, `pg-query-stream`, postgres.js's `cursor()`); `wide_all` (128 rows
of an int8, numeric, timestamptz, uuid, text, boolean and jsonb table); and
`concurrent_pool`, each side's pool-level query from concurrent async loops.
A side with no statement of its own for a workload is reported as n/a with
the reason: Drizzle has no PostgreSQL stream, Drizzle's raw `db.execute` has
no `.prepare()`, and pgorm's model API adds nothing to raw SQL for a scalar.
Within a round the sides run each workload one after another, in an order
rotated every round, so a workload's sides are measured close together in
time. Each round then runs the attribution: `rows_all`, `wide_all` and
`rows_stream` at each attribution row count; for pgorm, the native call alone
(the addon's result arrays, before any row object is made) and the row
objects alone (built from a result fetched once); one column of each type of
the wide table at a time; text 32 B, 1 KiB and 16 KiB wide; and each side's
pool query with one operation in flight, with pgorm's pool also measured with
`recycle: "fast"` beside its default `"verified"`.

Every result is checked, warmup included, after the clock stops, so neither
an error nor a wrong answer is a fast query: values are normalised only for
the check (an int8 compared as text whether a side read a string or a bigint,
a numeric without trailing zeros, an instant as Unix milliseconds), and each
side keeps its own defaults, whose conversions are part of what it costs.
The update writes row 17's own values back, so every round reads the fixture
it started with. Sequential throughput is samples over the sum of their
latencies; concurrent throughput is wall-clock, as `driver_baseline` reports
it.

Each report records the Node.js version and flags, every resolved package
version, the addon's profile and SHA-256, SHA-256 fingerprints of the
harness's sources and of pgorm-napi's JavaScript at the start and end, git
HEAD and status, the OS and CPU, the load averages and Time Machine state at
the start and end of every round, the `cargo`, `rustc` and `node` processes
running at the start, the PostgreSQL version, the settings `driver_baseline`
records plus `plan_cache_mode` and `jit`, and the other server sessions at the
start. Like the Rust harnesses, it measures latency only: no allocations,
copies or memory.
