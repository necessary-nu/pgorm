# Driver performance measurements

`driver_baseline` compares the direct `pgorm-driver` path with pgorm on the same
PostgreSQL server. It measures an existing connection separately from concurrent
pool acquisition. The separate native harness compares the embedded engine,
`pgorm-driver`, and pgorm using matched SQL and database settings.

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
