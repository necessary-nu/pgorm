# Driver performance measurements

`driver_baseline` compares the direct `pgorm-driver` path with pgorm on the same
PostgreSQL server. It measures an existing connection separately from concurrent
pool acquisition. The native Turso comparison can use the same workload and
report format once its connection path exists.

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
