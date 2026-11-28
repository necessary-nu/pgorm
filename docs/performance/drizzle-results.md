# pgorm-napi against Drizzle ORM and the Node.js drivers

**These results are provisional.** The machine never went idle by the
harness's gate (one-minute load under 4, no `cargo` or `rustc`, no Time
Machine backup) during the hour the harness waited for it. The run reported
here then ran without the gate at one-minute loads of 6.8 to 17. A second
run, started a few minutes later when the gate did pass at a load of 3.9,
saw the load climb to 109 during its first round. Both reports are kept. The
comparisons below hold in both runs' quieter rounds, but the absolute
latencies, and any difference under about 10%, should be confirmed on an
idle machine.

On single statements, pgorm's JavaScript binding is within about 20% of the
raw drivers' prepared paths and close to Drizzle's prepared statements: 76 µs
for a scalar against 62–64 µs, and 72 µs for a raw primary-key lookup (84 µs
through a model) against 66 µs for `pg` and 67 µs for Drizzle's prepared
lookup over `pg`. It falls behind as soon as a result carries rows. Reading
128 rows into objects takes 1.55× `pg`'s time and 1.82× postgres.js's. Reading
128 rows of seven mixed types takes 3.2× and 4.0×, and streaming 128 rows
takes 4.5× and 5.1×. Its pool is 1.9× slower than `pg`'s under four
concurrent workers at the binding's default `recycle: "verified"`, and on par
with `recycle: "fast"`.

The attribution runs locate those costs:

- (a) The stream's promise per row is confirmed, and it is the largest of
  them: 9.9 µs per streamed row, against 0.63 µs per row for the same rows
  read all at once.
- (b) `Object.fromEntries` is confirmed and minor: 0.10 µs per three-column
  row, 14% of a 1,024-row read.
- (c) The `make` call per Decimal, Uuid and Temporal value is confirmed:
  +100 to +260 ns per value over a bigint. JSON costs +2.7 µs per value, the
  largest per-value cost found.
- (d) The double string copy is not measurable: pgorm's cost per byte equals
  `pg`'s.

The native crossing itself also costs 0.17 µs per value before any of
these: more per row than `pg`'s whole path.

## Conditions

The run reported here, [drizzle-results.json](drizzle-results.json), started
at 2026-11-27 22:13:34 UTC and took 6.6 minutes. Every one of its 498
summaries, 222 workload and 276 attribution, finished, and all 627,000
checked operations passed, warmups included. The repeat that passed the idle
gate at its start is [drizzle-results-gated.json](drizzle-results-gated.json):
22:21:19–22:34:52 UTC, its JSON labelled `recorded` because the gate passed.
Its rounds ran at loads of 3.9 → 109, 109 → 18 and 18 → 36. Its first round
agrees with the run reported here, and its later rounds are up to twice as slow on
every side.

| | |
| --- | --- |
| Host | Apple M5 Pro, 18 logical CPUs, 64 GiB, macOS 27.2 (Darwin 27.2.0, arm64) |
| Runtime | Node.js 26.11.1 (V8 14.6.202.34, libuv 1.52.1, Node-API 10), run as `node --expose-gc src/main.ts`, garbage collected between workloads |
| pgorm | `@necessary/pgorm` 0.2.0 at `c200012ca727047936cf3deb2335e0b1ab00b1df`, the addon built by `node pgorm-napi/scripts/build.mjs --release` (the `addon` profile: release, fat LTO, one codegen unit), SHA-256 `f382299a…20208` |
| Other packages | drizzle-orm 0.45.4, pg 8.23.1 (pg-pool 3.14.0, pg-protocol 1.16.1, pg-types 2.2.0), pg-query-stream 4.17.1 (pg-cursor 2.22.1), postgres 4.1.1, TypeScript 7.0.2 |
| Server | PostgreSQL 19.0 in the `pgorm-test-19` Docker container, over TCP to 127.0.0.1; `fsync`, `full_page_writes` and `synchronous_commit` on, `shared_buffers` 16384 (128 MB), `wal_level` replica, `max_connections` 100, `plan_cache_mode` auto, `jit` off; no other session on the server at the start |
| Load (1/5/15 min) | round 0: 15.79/27.26/33.11 → 8.76/20.27/29.46; round 1: → 16.98/18.29/27.11; round 2: → 6.78/13.93/24.22. No Time Machine backup at any round boundary. One idle `cargo` process at the start, no `rustc` |
| Shape | 100 warmup and 1,000 measured operations per workload, three rounds, 128 rows, four workers over a pool of four, stream fetches of 100 rows for `pg-query-stream` and postgres.js's cursor |

The working tree was `c200012ca` with the harness, this document's README
section and the plan's claim uncommitted. The harness's sources and
pgorm-napi's JavaScript hashed the same at the start and the end of the run
(`harness_sha256` `822a911c…0d747a`). The committed harness differs from the
measured one only by doc comments deleted afterwards, so it hashes
differently, `8d07911a…bad2ba`.

## Sides

Every side is TypeScript run by the same process, and each sends what its
API writes. The reports record the SQL each one renders, under `sides[].sql`.

| Side | What it is |
| --- | --- |
| `pgorm/raw` | SQL text and values on a pgorm `Connection` (`query`, `one`, `execute`, `transaction`, `stream`); its pool's `one` for `concurrent_pool` |
| `pgorm/orm` | A declared `model`: `findByKey(..).one`, `find().orderBy(..).limit(n).all` / `.stream`, `update(..).where(key)` with the sum written as a `template` |
| `drizzle-pg/*`, `drizzle-postgres/*` | Drizzle over a node-postgres `Client` / `Pool`, or over a postgres.js client, with `bigint` columns in `mode: "bigint"`; `prepared` builds each statement once with `sql.placeholder` and `.prepare(name)`; `unprepared` builds it per call; raw statements are `` db.execute(sql`..`) `` |
| `pg/prepared`, `pg/unprepared` | node-postgres with a named query config, or plain text and values; `pg-query-stream` for the stream |
| `postgres/prepared`, `postgres/unprepared` | postgres.js tagged templates on a client of `prepare: true` (its default), or `prepare: false`; `.cursor(100)` for the stream |

Not applicable:

- Drizzle has no row stream for PostgreSQL, so `rows_stream` is n/a for every
  Drizzle side.
- Drizzle's `db.execute(sql)` has no `.prepare()`, and its select builder
  needs a `FROM`. `scalar`, `params16`, `transaction` and `concurrent_pool`
  therefore run on the unprepared Drizzle sides alone.
- pgorm's model API adds nothing to raw SQL for those four workloads, so
  `pgorm/orm` does not run them.

### What is prepared on the server

Each side's own session read `pg_prepared_statements` after its workloads.
pgorm prepares and caches every statement on its connection itself (128 per
connection by default) and has no unprepared mode, so `pgorm/raw` and
`pgorm/orm` are prepared sides by construction.

| Side | Statements | Names, with generic/custom plans |
| --- | ---: | --- |
| pgorm/raw | 6 | `s11` (1095/5), `s3` (2195/5), `s4` (1095/5), `s5` (1095/5), `s7` (1095/5), `s9` (2195/5) |
| pgorm/orm | 4 | `s10` (2195/5), `s12` (1095/5), `s6` (1095/5), `s8` (1095/5) |
| drizzle-pg/prepared | 4 | `drizzle_lookup` (1095/5), `drizzle_rows` (1095/5), `drizzle_update16` (1095/5), `drizzle_wide` (1095/5) |
| drizzle-postgres/prepared | 4 | `jr31u0po1` (1095/5), `jr31u0po2` (1095/5), `jr31u0po3` (1095/5), `jr31u0po4` (1095/5) |
| drizzle-pg/unprepared | 0 |  |
| drizzle-postgres/unprepared | 0 |  |
| pg/prepared | 6 | `lookup` (1095/5), `params16` (1095/5), `rows` (1095/5), `scalar` (2195/5), `update16` (1095/5), `wide` (1095/5) |
| pg/unprepared | 0 |  |
| postgres/prepared | 6 | `ucxzq64l1` (2195/5), `ucxzq64l2` (1095/5), `ucxzq64l3` (1095/5), `ucxzq64l4` (1095/5), `ucxzq64l5` (2195/5), `ucxzq64l6` (1095/5) |
| postgres/unprepared | 0 |  |

Every prepared side held one named statement per distinct statement text.
Each statement was planned as a custom plan five times and then generically:
1,095 generic and 5 custom over 1,100 executions, or 2,195 and 5 for the
scalar, which the transaction workload runs too. The prepared sides are
therefore the same thing on the server: `pgorm/raw` held as many statements
as `pg/prepared` and `postgres/prepared`, and `pgorm/orm` as many as the
prepared Drizzle sides. The unprepared sides held none: each execution
parsed and planned an unnamed statement. `pg/prepared`'s stream is unnamed
too, because `pg-query-stream` (over `pg-cursor`) always parses an unnamed
statement.

### Values

Each side returns its own defaults, and the checks normalise them. The
conversions are part of what each side costs:

- pg and postgres.js read `int8` and `numeric` as strings and `timestamptz` as
  a `Date`.
- pgorm reads `bigint`, its `Decimal`, `Temporal.Instant` and its `Uuid`.
- Drizzle reads `bigint` (the columns are declared `mode: "bigint"`), `numeric`
  as a string and `timestamptz` as a `Date`.

## Results

Each latency is the median of the three rounds' percentile, in microseconds.
The p50 range is the spread across rounds. Each ratio divides a side's p50
by the raw driver's prepared p50. Sequential throughput is samples over the
sum of their latencies, and `concurrent_pool` throughput is wall-clock across
the four workers.

### `scalar`

| Side | p50 µs | p95 µs | p99 µs | min µs | max µs | p50 range µs | ops/s | p50 ÷ pg/prepared | p50 ÷ postgres/prepared |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pgorm/raw | 76.2 | 111.5 | 123.3 | 58.5 | 1648.7 | 71.3–173.1 | 12203 | 1.19 | 1.22 |
| drizzle-pg/unprepared | 77.7 | 102.1 | 140.7 | 61.0 | 1951.9 | 73.3–288.5 | 12381 | 1.21 | 1.25 |
| drizzle-postgres/unprepared | 75.4 | 125.8 | 172.8 | 58.5 | 622.1 | 70.3–185.4 | 12187 | 1.18 | 1.21 |
| pg/prepared | 64.0 | 84.8 | 164.1 | 51.8 | 420.6 | 60.8–107.0 | 14761 | 1.00 | 1.03 |
| pg/unprepared | 74.8 | 150.2 | 314.3 | 57.2 | 536.8 | 65.2–137.0 | 11443 | 1.17 | 1.20 |
| postgres/prepared | 62.3 | 73.8 | 83.5 | 50.6 | 425.8 | 62.0–134.7 | 15751 | 0.97 | 1.00 |
| postgres/unprepared | 70.8 | 92.2 | 128.1 | 56.5 | 9041.3 | 69.1–161.2 | 13554 | 1.11 | 1.14 |

### `params16`

| Side | p50 µs | p95 µs | p99 µs | min µs | max µs | p50 range µs | ops/s | p50 ÷ pg/prepared | p50 ÷ postgres/prepared |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pgorm/raw | 74.1 | 91.8 | 105.3 | 59.6 | 245.4 | 72.0–83.6 | 13147 | 0.89 | 1.08 |
| drizzle-pg/unprepared | 129.0 | 252.7 | 352.7 | 85.1 | 626.5 | 97.9–179.1 | 6821 | 1.55 | 1.88 |
| drizzle-postgres/unprepared | 111.0 | 154.1 | 246.1 | 83.0 | 523.0 | 95.4–143.1 | 8518 | 1.34 | 1.62 |
| pg/prepared | 83.0 | 132.5 | 171.0 | 54.4 | 336.2 | 64.7–84.7 | 11165 | 1.00 | 1.21 |
| pg/unprepared | 87.5 | 103.1 | 120.0 | 68.0 | 300.5 | 78.1–90.9 | 11218 | 1.05 | 1.28 |
| postgres/prepared | 68.6 | 87.6 | 97.3 | 53.2 | 401.5 | 63.6–75.7 | 13875 | 0.83 | 1.00 |
| postgres/unprepared | 79.9 | 92.3 | 104.1 | 66.0 | 253.2 | 76.3–80.3 | 12326 | 0.96 | 1.17 |

### `model_lookup`

| Side | p50 µs | p95 µs | p99 µs | min µs | max µs | p50 range µs | ops/s | p50 ÷ pg/prepared | p50 ÷ postgres/prepared |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pgorm/raw | 71.8 | 85.6 | 95.5 | 58.5 | 267.3 | 71.0–73.1 | 13635 | 1.09 | 1.14 |
| pgorm/orm | 84.4 | 101.3 | 121.8 | 68.8 | 463.9 | 81.5–84.9 | 11426 | 1.28 | 1.34 |
| drizzle-pg/prepared | 67.4 | 82.0 | 92.8 | 54.4 | 3451.7 | 66.8–67.8 | 14557 | 1.02 | 1.07 |
| drizzle-postgres/prepared | 65.8 | 82.6 | 95.8 | 54.5 | 392.5 | 64.9–66.8 | 14763 | 1.00 | 1.05 |
| drizzle-pg/unprepared | 95.5 | 116.9 | 136.9 | 80.0 | 446.9 | 92.7–96.4 | 10311 | 1.44 | 1.52 |
| drizzle-postgres/unprepared | 149.5 | 178.7 | 200.3 | 126.8 | 3140.0 | 148.9–160.5 | 6545 | 2.26 | 2.38 |
| pg/prepared | 66.1 | 76.2 | 87.6 | 54.2 | 330.2 | 64.3–68.0 | 14930 | 1.00 | 1.05 |
| pg/unprepared | 75.0 | 88.3 | 100.3 | 61.3 | 228.0 | 73.8–75.3 | 13132 | 1.13 | 1.19 |
| postgres/prepared | 62.8 | 74.3 | 81.7 | 51.7 | 621.5 | 62.2–70.8 | 15657 | 0.95 | 1.00 |
| postgres/unprepared | 76.0 | 100.3 | 123.4 | 60.9 | 343.3 | 74.0–82.6 | 12556 | 1.15 | 1.21 |

### `update16`

| Side | p50 µs | p95 µs | p99 µs | min µs | max µs | p50 range µs | ops/s | p50 ÷ pg/prepared | p50 ÷ postgres/prepared |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pgorm/raw | 481.0 | 745.7 | 1015.4 | 410.5 | 8285.7 | 463.0–504.8 | 1917 | 0.98 | 0.97 |
| pgorm/orm | 519.5 | 728.8 | 1176.5 | 436.7 | 8197.0 | 484.8–526.0 | 1837 | 1.05 | 1.05 |
| drizzle-pg/prepared | 488.4 | 739.3 | 1355.0 | 406.2 | 6070.3 | 454.8–539.5 | 1960 | 0.99 | 0.99 |
| drizzle-postgres/prepared | 482.9 | 672.4 | 937.8 | 407.3 | 14637.8 | 464.6–487.5 | 1928 | 0.98 | 0.98 |
| drizzle-pg/unprepared | 551.0 | 702.9 | 874.8 | 468.5 | 4886.3 | 544.0–559.5 | 1741 | 1.12 | 1.11 |
| drizzle-postgres/unprepared | 637.2 | 862.8 | 1130.3 | 533.9 | 12426.1 | 610.3–641.1 | 1485 | 1.29 | 1.29 |
| pg/prepared | 493.3 | 648.8 | 806.5 | 422.1 | 6356.0 | 489.5–509.8 | 1937 | 1.00 | 1.00 |
| pg/unprepared | 509.2 | 654.8 | 948.3 | 423.8 | 4776.8 | 506.8–542.6 | 1862 | 1.03 | 1.03 |
| postgres/prepared | 494.9 | 729.4 | 1574.6 | 399.6 | 15111.1 | 473.8–504.7 | 1825 | 1.00 | 1.00 |
| postgres/unprepared | 548.3 | 753.4 | 1206.1 | 472.0 | 4767.8 | 543.0–597.3 | 1678 | 1.11 | 1.11 |

### `transaction`

| Side | p50 µs | p95 µs | p99 µs | min µs | max µs | p50 range µs | ops/s | p50 ÷ pg/prepared | p50 ÷ postgres/prepared |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pgorm/raw | 216.7 | 391.8 | 632.5 | 175.2 | 1098.7 | 215.9–241.9 | 4196 | 1.20 | 1.16 |
| drizzle-pg/unprepared | 195.4 | 228.5 | 287.8 | 168.8 | 980.8 | 193.4–227.4 | 5002 | 1.08 | 1.05 |
| drizzle-postgres/unprepared | 192.7 | 223.6 | 266.3 | 162.3 | 1147.5 | 192.0–218.2 | 5053 | 1.07 | 1.03 |
| pg/prepared | 180.6 | 218.5 | 264.5 | 153.1 | 7925.7 | 179.6–203.3 | 5385 | 1.00 | 0.97 |
| pg/unprepared | 184.6 | 222.9 | 322.4 | 159.5 | 932.9 | 182.2–240.1 | 5311 | 1.02 | 0.99 |
| postgres/prepared | 186.7 | 215.7 | 251.5 | 156.4 | 959.9 | 184.0–240.8 | 5265 | 1.03 | 1.00 |
| postgres/unprepared | 195.7 | 256.1 | 327.0 | 164.2 | 678.7 | 184.1–226.6 | 4896 | 1.08 | 1.05 |

### `rows_all`

| Side | p50 µs | p95 µs | p99 µs | min µs | max µs | p50 range µs | ops/s | p50 ÷ pg/prepared | p50 ÷ postgres/prepared |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pgorm/raw | 170.8 | 270.6 | 390.5 | 147.9 | 693.7 | 169.9–183.3 | 5352 | 1.55 | 1.82 |
| pgorm/orm | 185.2 | 214.5 | 404.0 | 160.9 | 932.0 | 181.1–207.2 | 5203 | 1.68 | 1.98 |
| drizzle-pg/prepared | 138.7 | 510.4 | 987.5 | 116.8 | 3472.7 | 138.0–147.3 | 5125 | 1.26 | 1.48 |
| drizzle-postgres/prepared | 127.3 | 146.2 | 178.9 | 105.5 | 682.0 | 118.6–127.4 | 7681 | 1.16 | 1.36 |
| drizzle-pg/unprepared | 166.8 | 246.0 | 659.0 | 140.2 | 1512.5 | 158.3–172.5 | 5407 | 1.51 | 1.78 |
| drizzle-postgres/unprepared | 151.8 | 242.4 | 353.1 | 134.0 | 766.6 | 149.1–159.9 | 5796 | 1.38 | 1.62 |
| pg/prepared | 110.2 | 134.0 | 225.4 | 94.8 | 1630.6 | 103.4–111.2 | 8506 | 1.00 | 1.18 |
| pg/unprepared | 128.6 | 212.2 | 350.3 | 105.0 | 1597.8 | 119.3–128.6 | 6857 | 1.17 | 1.37 |
| postgres/prepared | 93.8 | 110.4 | 126.3 | 80.2 | 790.3 | 93.7–95.1 | 10364 | 0.85 | 1.00 |
| postgres/unprepared | 107.8 | 127.6 | 143.1 | 93.1 | 527.7 | 106.4–127.9 | 9072 | 0.98 | 1.15 |

### `rows_stream`

| Side | p50 µs | p95 µs | p99 µs | min µs | max µs | p50 range µs | ops/s | p50 ÷ pg/prepared | p50 ÷ postgres/prepared |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pgorm/raw | 1394.1 | 2016.0 | 2967.6 | 1206.1 | 10099.0 | 1373.2–1865.7 | 670 | 4.53 | 5.10 |
| pgorm/orm | 1421.5 | 1675.8 | 2051.1 | 1205.9 | 6523.4 | 1375.4–1858.5 | 685 | 4.62 | 5.20 |
| pg/prepared | 307.6 | 475.3 | 744.8 | 259.5 | 1933.2 | 293.3–330.2 | 2983 | 1.00 | 1.13 |
| pg/unprepared | 320.4 | 383.8 | 476.5 | 263.8 | 2649.0 | 288.4–322.5 | 3036 | 1.04 | 1.17 |
| postgres/prepared | 273.3 | 516.3 | 709.8 | 184.3 | 1357.0 | 209.7–397.4 | 3220 | 0.89 | 1.00 |
| postgres/unprepared | 306.1 | 479.9 | 634.7 | 250.8 | 1326.0 | 283.8–333.5 | 3030 | 0.99 | 1.12 |

### `wide_all`

| Side | p50 µs | p95 µs | p99 µs | min µs | max µs | p50 range µs | ops/s | p50 ÷ pg/prepared | p50 ÷ postgres/prepared |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pgorm/raw | 683.6 | 910.1 | 1071.5 | 581.3 | 10489.5 | 628.5–701.0 | 1390 | 3.17 | 4.04 |
| pgorm/orm | 682.3 | 766.8 | 856.3 | 633.5 | 1487.4 | 673.0–723.5 | 1440 | 3.17 | 4.03 |
| drizzle-pg/prepared | 244.3 | 353.4 | 898.1 | 215.4 | 2905.9 | 243.3–247.3 | 3705 | 1.13 | 1.44 |
| drizzle-postgres/prepared | 227.6 | 267.8 | 352.8 | 196.5 | 1450.3 | 225.2–243.4 | 4296 | 1.06 | 1.34 |
| drizzle-pg/unprepared | 279.9 | 395.1 | 790.0 | 250.2 | 1388.9 | 278.9–302.9 | 3314 | 1.30 | 1.65 |
| drizzle-postgres/unprepared | 261.3 | 312.0 | 404.6 | 235.7 | 1116.7 | 259.5–292.4 | 3709 | 1.21 | 1.54 |
| pg/prepared | 215.3 | 301.0 | 931.2 | 192.0 | 3204.5 | 213.6–220.1 | 4170 | 1.00 | 1.27 |
| pg/unprepared | 235.7 | 342.0 | 1045.0 | 202.4 | 3720.9 | 227.6–242.3 | 3702 | 1.09 | 1.39 |
| postgres/prepared | 169.4 | 217.9 | 259.0 | 147.5 | 6411.0 | 167.4–187.2 | 5707 | 0.79 | 1.00 |
| postgres/unprepared | 193.3 | 240.0 | 360.7 | 157.2 | 3376.6 | 179.8–195.7 | 4933 | 0.90 | 1.14 |

### `concurrent_pool`

| Side | p50 µs | p95 µs | p99 µs | min µs | max µs | p50 range µs | ops/s | p50 ÷ pg/prepared | p50 ÷ postgres/prepared |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| pgorm/raw | 187.7 | 286.8 | 394.4 | 115.9 | 1260.3 | 172.2–206.5 | 19991 | 1.87 | 1.72 |
| drizzle-pg/unprepared | 108.5 | 187.8 | 245.1 | 60.6 | 747.9 | 99.7–117.7 | 32364 | 1.08 | 1.00 |
| drizzle-postgres/unprepared | 101.0 | 168.7 | 239.5 | 56.4 | 526.5 | 99.9–116.5 | 36581 | 1.01 | 0.93 |
| pg/prepared | 100.2 | 164.9 | 211.1 | 52.8 | 424.0 | 91.7–105.3 | 36909 | 1.00 | 0.92 |
| pg/unprepared | 103.8 | 177.6 | 257.5 | 58.1 | 955.6 | 97.6–114.5 | 35367 | 1.04 | 0.95 |
| postgres/prepared | 109.0 | 229.9 | 399.7 | 53.4 | 696.4 | 96.5–116.4 | 32627 | 1.09 | 1.00 |
| postgres/unprepared | 99.6 | 163.3 | 218.9 | 55.8 | 823.1 | 94.0–135.5 | 37375 | 0.99 | 0.91 |

## Attribution

Each attribution workload runs every round on `pgorm/raw`, `pgorm/orm`,
`pg/prepared` and `postgres/prepared`. For pgorm it also times the addon's
own call, through the module's internals (`native.connectionRun`,
`native.streamNext`), which returns the result as arrays before any row
object is made. It also times the row objects alone, built with the module's
own `row()` from a result fetched once. A plain loop that assigns each key
is a reference point only, not an equivalent: it does not define
`__proto__` as an own property, which is why the module uses
`Object.fromEntries`. The reads by column type and by text width are raw
statements, so `pgorm/orm` has no cells in those tables.

### Rows: per call and per row

Per row is the slope between the smallest and largest row counts, (p50 at the largest − p50 at the smallest) ÷ the rows between them; per value divides it by the columns.

| Side | Workload | p50 µs at each count | per row µs | per value µs |
| --- | --- | --- | ---: | ---: |
| pgorm/raw | rows_all | 1: 73.3, 128: 169.5, 1024: 712.8 | 0.625 | 0.208 |
| pgorm/orm | rows_all | 1: 86.8, 128: 184.6, 1024: 742.0 | 0.640 | 0.213 |
| pg/prepared | rows_all | 1: 68.9, 128: 110.7, 1024: 354.3 | 0.279 | 0.093 |
| postgres/prepared | rows_all | 1: 65.9, 128: 91.3, 1024: 255.9 | 0.186 | 0.062 |
| pgorm/raw | wide_all | 1: 88.5, 128: 634.6, 1024: 4406.8 | 4.221 | 0.603 |
| pgorm/orm | wide_all | 1: 107.3, 128: 669.7, 1024: 4399.5 | 4.196 | 0.599 |
| pg/prepared | wide_all | 1: 77.9, 128: 225.0, 1024: 1079.2 | 0.979 | 0.140 |
| postgres/prepared | wide_all | 1: 72.9, 128: 169.9, 1024: 739.2 | 0.651 | 0.093 |
| pgorm/raw | rows_stream | 1: 115.3, 128: 1537.8, 1024: 10245.8 | 9.903 | 3.301 |
| pgorm/orm | rows_stream | 1: 120.2, 128: 1394.5, 1024: 10542.7 | 10.188 | 3.396 |
| pg/prepared | rows_stream | 1: 187.3, 128: 285.4, 1024: 1163.8 | 0.955 | 0.318 |
| postgres/prepared | rows_stream | 1: 126.6, 128: 209.8, 1024: 922.0 | 0.777 | 0.259 |
| pgorm/raw | native_rows_all | 1: 73.8, 128: 153.5, 1024: 595.2 | 0.510 | 0.170 |
| pgorm/raw | native_wide_all | 1: 85.9, 128: 597.6, 1024: 4181.9 | 4.004 | 0.572 |
| pgorm/raw | native_rows_stream | 1: 107.0, 128: 1383.1, 1024: 10550.1 | 10.208 | 3.403 |
| pgorm/raw | row_objects | 1: 0.3, 128: 12.5, 1024: 97.2 | 0.095 | 0.032 |
| pgorm/raw | wide_row_objects | 1: 0.4, 128: 28.2, 1024: 224.0 | 0.219 | 0.031 |
| pgorm/raw | loop_objects | 1: 0.3, 128: 3.6, 1024: 27.8 | 0.027 | 0.009 |
| pgorm/raw | wide_loop_objects | 1: 0.3, 128: 6.9, 1024: 54.5 | 0.053 | 0.008 |

### Pools

Each side's pool-level query with one operation in flight (`@1worker`), and the pool configurations a side offers beside its default.

| Side | Workload | p50 µs | p95 µs | ops/s |
| --- | --- | ---: | ---: | ---: |
| pgorm/raw | pool_query@1worker | 122.7 | 157.9 | 7801 |
| pg/prepared | pool_query@1worker | 65.0 | 77.3 | 14849 |
| postgres/prepared | pool_query@1worker | 61.0 | 69.5 | 16167 |
| pgorm/raw | pool_query[recycle_fast]@1worker | 69.9 | 82.0 | 14093 |
| pgorm/raw | concurrent_pool[recycle_fast] | 106.0 | 152.0 | 36219 |

### One column of each type

p50 of reading one column of bench_wide, and the same per row.

| Column | pgorm/raw p50 µs | per row ns | pgorm/orm p50 µs | per row ns | pg/prepared p50 µs | per row ns | postgres/prepared p50 µs | per row ns |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| id | 357.4 | 349 |  |  | 231.1 | 226 | 173.7 | 170 |
| amount | 621.5 | 607 |  |  | 300.9 | 294 | 192.6 | 188 |
| at | 538.1 | 525 |  |  | 415.4 | 406 | 234.1 | 229 |
| uid | 463.6 | 453 |  |  | 232.2 | 227 | 215.5 | 210 |
| label | 415.8 | 406 |  |  | 210.5 | 206 | 188.1 | 184 |
| flag | 369.8 | 361 |  |  | 223.8 | 219 | 163.8 | 160 |
| doc | 3093.8 | 3021 |  |  | 501.8 | 490 | 467.6 | 457 |

### Text of growing width

p50 of reading 256 rows of text of each width; per byte is the slope from the narrowest to the widest.

| Width | pgorm/raw p50 µs | pgorm/orm p50 µs | pg/prepared p50 µs | postgres/prepared p50 µs |
| --- | ---: | ---: | ---: | ---: |
| 32 B | 227.8 |  | 156.6 | 127.8 |
| 1024 B | 439.0 |  | 261.8 | 255.0 |
| 16384 B | 3592.8 |  | 3520.7 | 2966.2 |
| per byte ns | 0.804 |  | 0.804 | 0.678 |

### (a) The stream's promise per row: confirmed, and the largest cost

A streamed row costs 9.9 µs, against 0.63 µs for a row read all at once and
0.96 µs and 0.78 µs for a row of `pg`'s and postgres.js's streams. The
addon's `streamNext` alone costs 10.2 µs per row, so the cost is the native
pull, not `RowStream.next`'s JavaScript: the two are within this run's
noise of each other.

Each pull is one call into the addon. The call spawns a task on pgorm's
Tokio runtime, takes the next row, and settles a promise through the
instance's Neon channel. That is a task, two cross-thread wake-ups and an
event-loop turn per row. The drivers fetch 100 rows per round trip and hand
them out from memory.

At 128 rows the stream takes 1,394 µs where `rows_all` takes 171 µs.

### (b) `Object.fromEntries` per row: confirmed, minor

Building row objects with `row()` costs 0.095 µs per three-column row and
0.219 µs per seven-column row, 3.5–4.1× the assigning loop's 0.027 µs and
0.053 µs. At 1,024 rows that is 97 µs of `rows_all`'s 713 µs (14%) and
224 µs of `wide_all`'s 4,407 µs (5%). `rows_all` less the native call alone
is 118 µs at 1,024 rows: the row objects and the JavaScript around the call.

Without any row objects at all, the native call alone (0.51 µs per row)
still costs more per row than `pg`'s complete path (0.28 µs).

### (c) A `make` call per Decimal, Uuid and Temporal value: confirmed

Reading one column of 1,024 rows isolates each type's per-value cost. A
bigint column costs pgorm 349 ns per row, against 226 ns for `pg` and 170 ns
for postgres.js. Over that bigint column, each type adds per value:

| Type | pgorm | `pg` | postgres.js |
| --- | ---: | ---: | ---: |
| `numeric` (a `Decimal`) | +258 ns | +68 ns (a string) | +18 ns (a string) |
| `uuid` (a `Uuid`) | +104 ns | +1 ns (a string) | +40 ns (a string) |
| `timestamptz` (a `Temporal.Instant`) | +176 ns | +180 ns (a `Date`) | +59 ns (a `Date`) |

These differences hold each type's Rust decoding and its call into
JavaScript together. For `Decimal` and `Uuid`, where the drivers return
strings, the cost is pgorm's alone. For `timestamptz` it matches `pg`'s own
`Date` parsing.

### (d) Strings copied twice: not measurable here

Reading 256 rows of text 32 B, 1 KiB and 16 KiB wide gives a cost per byte
of 0.804 ns for pgorm, 0.804 ns for `pg` and 0.678 ns for postgres.js. The
gated repeat gives 1.215 ns and 1.262 ns for pgorm and `pg`. The extra copy
does not show above what every side pays per byte: the server, the socket
and UTF-8 decoding.

What text costs pgorm is per value, not per byte. A 15-byte label adds
57 ns per value over a bigint, where `pg`'s adds nothing.

### Other costs

- **JSON**, the largest per-value cost. A small `jsonb` document adds
  2.67 µs per value over a bigint, against 0.26 µs for `pg` and 0.29 µs for
  postgres.js. That is 63% of a wide row's 4.2 µs. Each document is decoded
  into a `serde_json::Value` in Rust, written back to text, and parsed again
  by `JSON.parse` with a reviver (for integers past 2^53) on the JavaScript
  thread.
- **The pool's recycling check.** The binding's pools default to
  `recycle: "verified"`, an empty query on every acquire. Rust's `connect()`
  defaults to Fast.
  - With one operation in flight, a pool-level query costs 123 µs, against
    70 µs with `recycle: "fast"`, 65 µs for `pg` and 61 µs for postgres.js.
  - Under four workers, `concurrent_pool` drops from 188 µs and 20,000 ops/s
    to 106 µs and 36,200 ops/s with `"fast"`, which is `pg`'s 36,900.
  - Since this run, both bindings' pools default to `"fast"`, as Rust's
    `connect()` does; `"verified"` remains an option.
- **The crossing per value.** With no type conversion at all (`id`), pgorm
  spends 120–180 ns per row more than the drivers. The addon's call alone
  costs 0.17 µs per value of `bench_row`, against `pg`'s complete 0.09 µs.
  The addon decodes every row into a vector of values, then builds a
  JavaScript array per row and sets each value into it, one Node-API call
  per value.
- **A statement per call.**
  - pgorm's scalar takes 12 µs longer than `pg`'s prepared one (76 µs against
    64 µs), and a transaction 36 µs longer (217 µs against 181 µs). A
    transaction makes three calls into the addon: BEGIN, the statement, and
    the commit, whose `SELECT; COMMIT` check is one round trip.
  - The model path builds and renders its statement through the addon on
    every call. That costs 13 µs over the raw path for a lookup and for a
    one-row read.
  - Drizzle's unprepared builder costs 28 µs over its prepared statement on
    `pg`.
- **postgres.js and string parameters.** Unprepared Drizzle over postgres.js
  takes 150 µs for a lookup, against 76 µs for unprepared raw postgres.js.
  postgres.js 4.1.1 describes an unnamed statement before each run when a
  parameter is a string, and Drizzle's `mode: "bigint"` binds an int8 as a
  string. A probe against the same server measured `SELECT ${v}::BIGINT`
  with `prepare: false` at 136.8 µs for `"17"` and 82.2 µs for `17`. With
  `prepare: true` both took 68 µs.

## Limitations

- The run is provisional: see the first paragraph.
- The server is a Docker container on the same machine, reached through
  Docker's port forwarding, so every round trip includes it.
- The concurrent workers are async loops on one JavaScript thread. The
  drivers do all their work on it, while pgorm's runs on its own Tokio
  runtime as well. CPU time is not measured, so a latency here says nothing
  about the cores each side used.
- The streams differ by design. The drivers fetch 100 rows per round trip;
  pgorm sends one query and pulls each row across the addon.
- As in the Rust harnesses, latency is the only measurement: no allocations,
  copies or memory.
