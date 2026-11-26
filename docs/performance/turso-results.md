# Native Turso performance evidence

The retained native implementation preserves the shared row/type API with measurable overhead: it adds about 7–8 µs to a trivial prepared scalar read. The shipped driver scalar path remains at **11 allocations and 3,696 requested bytes per call**; ORM remains at **16 allocations and 5,769 bytes**. Streaming peak requested Rust heap storage stays bounded through one million consumed rows. PostgreSQL latency attribution is inconclusive on this shared host; the preserved binary itself varies substantially between repeats.

We rejected an optional batch-storage optimization despite its measured allocation savings: lower memory-mode throughput persisted in both run orders. The shipped implementation retains the original batch storage. The experiment and the unresolved timing evidence are preserved below.

The initial full comparison below records the implementation before the focused batch-allocation experiment. The complete before binaries remain preserved locally, and those raw measurements remain unchanged. The batching experiment is recorded separately below.

## Measurement conditions

The full comparison ran on 2026-11-25 from 16:07:00 to 16:14:57 UTC (477 seconds), at harness commit `418d5a739fcc634bc879b5bace0f1075daf31e3d`, following core commit `451ff607` and compatibility commit `7113156d`. All 27 processes completed successfully, with every measured result checked, stable source fingerprints and executable hashes, and no other active PostgreSQL client at each PostgreSQL process startup.

The build source fingerprint is `1bc9f034f0606044e72e1e8147bb09ee0f643670`. It includes the unchanged, unrelated untracked `tests/scope_security_tests.rs`; that file is not a benchmark dependency. The build record predates the harness commit because the same frozen working tree was built before commit. Runtime HEAD and compilation provenance are recorded separately.

The host has 18 logical CPUs. It was not idle: starting load averages were 13.77/39.68/51.75 and ending averages were 25.02/41.34/49.22. The start snapshot included an external integration executable, rustc, Tailscale, and Spotlight activity; the end snapshot included six busy rustc processes and Spotlight. These are boundary observations, not continuous utilization measurements. No external processes were stopped.

All binaries use Rust 1.99.0 (`b940084d7`, aarch64-apple-darwin). The preserved PostgreSQL executable is exactly the historical artifact (`6bddf829af0756e0cd9f689464ef2c2c35ab22b6`), not a rebuild. Current PostgreSQL feature-off/on builds retain the historical development features; native builds use ordinary defaults plus `turso`. Allocation instrumentation is a separate executable. Full commands, hashes, features and locked package identities are in [build.json](turso-before-2026-11-25/build.json); the process observations are in [window.json](turso-before-2026-11-25/window.json).

Each latency cell below is the median of three process medians, followed by their minimum–maximum, in microseconds. Each process has three rounds. PostgreSQL uses 100 warmups and 1,000 samples; native uses 500 warmups and 5,000 samples. Concurrent cases use four workers and four pool connections. Layers and storage modes rotate as documented in [the procedure](README.md#native-comparison-workflow).

## PostgreSQL comparison

| Workload | Preserved driver | Current feature off | Current feature on |
| --- | ---: | ---: | ---: |
| driver/scalar | 138.38 (71.12–173.83) | 69.21 (66.50–72.62) | 144.00 (71.33–217.38) |
| orm/scalar | 156.62 (75.92–178.92) | 70.25 (66.25–89.29) | 178.12 (74.42–211.46) |
| driver/params16 | 187.46 (74.71–192.00) | 72.38 (66.79–72.88) | 154.08 (81.17–212.00) |
| orm/params16 | 148.58 (83.71–177.88) | 76.25 (66.33–78.33) | 185.33 (73.96–207.46) |
| driver/model_lookup | 192.62 (81.83–199.58) | 73.42 (73.25–78.29) | 154.67 (89.25–213.58) |
| orm/model_lookup | 184.42 (141.67–189.46) | 75.29 (70.96–80.29) | 210.83 (71.88–217.62) |
| driver/update16 | 1146.92 (789.88–1189.54) | 637.71 (533.67–1793.38) | 1933.79 (624.42–2085.79) |
| orm/update16 | 1278.33 (805.08–1692.67) | 551.00 (527.08–2035.88) | 2251.33 (892.67–2417.38) |
| driver/stream | 207.04 (122.67–267.83) | 116.79 (107.46–189.17) | 252.29 (126.33–264.75) |
| orm/stream | 254.62 (136.88–296.17) | 111.62 (109.25–127.29) | 241.04 (171.46–292.54) |
| driver/transaction | 486.00 (230.62–567.29) | 208.00 (199.25–491.62) | 562.00 (220.08–578.46) |
| orm/transaction | 516.62 (236.04–547.42) | 233.33 (199.62–285.12) | 476.29 (273.21–602.33) |
| driver/concurrent_pool | 189.17 (110.08–189.67) | 109.50 (85.08–181.54) | 205.38 (120.33–215.00) |
| orm/concurrent_pool | 189.79 (107.12–194.79) | 107.58 (85.21–164.29) | 185.04 (123.38–231.88) |

The preserved driver scalar has a 40.8% coefficient of variation across process medians; feature-on has 50.6%. Some storage-sensitive cases vary still more. Feature-off often appears faster, but these data do not establish that it improved, that feature-on regressed, or that either has zero regression. The raw ratios are retained for inspection, not treated as causal estimates. A controlled-host repeat is needed to quantify small PostgreSQL dispatch costs.

## Native latency before batch optimization

| Storage | Workload | Direct engine | Driver | ORM |
| --- | --- | ---: | ---: | ---: |
| memory | scalar | 0.12 (0.08–0.12) | 7.71 (7.25–8.08) | 7.92 (7.42–9.96) |
| memory | params16 | 0.67 (0.46–0.71) | 8.04 (7.38–8.33) | 9.08 (7.83–10.54) |
| memory | model_lookup | 1.88 (1.21–2.17) | 11.71 (11.67–12.17) | 13.25 (11.71–13.46) |
| memory | update16 | 56.62 (38.92–70.46) | 56.75 (49.08–59.04) | 69.62 (48.38–137.46) |
| memory | stream | 106.00 (87.83–163.33) | 234.00 (117.71–263.17) | 223.71 (118.42–330.71) |
| memory | transaction | 5.75 (5.29–10.42) | 49.67 (21.67–89.17) | 39.46 (21.50–68.17) |
| memory | concurrent_pool | 0.33 (0.33–0.50) | 26.00 (12.50–30.00) | 22.62 (13.00–28.54) |
| file | scalar | 0.12 (0.08–0.12) | 8.08 (7.25–8.33) | 8.08 (7.54–9.71) |
| file | params16 | 0.54 (0.50–0.58) | 8.88 (7.42–9.71) | 8.79 (7.88–10.12) |
| file | model_lookup | 1.46 (1.25–1.54) | 12.83 (11.71–13.29) | 12.75 (10.62–14.17) |
| file | update16 | 87.25 (64.04–87.75) | 78.00 (72.33–86.17) | 78.12 (77.58–84.21) |
| file | stream | 98.67 (92.67–115.54) | 132.25 (117.79–148.50) | 126.38 (118.67–147.92) |
| file | transaction | 5.83 (5.54–7.25) | 24.25 (21.21–25.08) | 25.17 (21.67–28.50) |
| file | concurrent_pool | 0.38 (0.25–0.50) | 18.96 (13.54–21.96) | 21.71 (12.96–23.46) |

Direct concurrent reads retain one connection per OS thread and do not acquire a pool; driver and ORM cases include acquisition, cached preparation, and Fast recycling. The direct result is a lower bound, not isolated pool overhead. Direct/driver operations reuse prepared statements, while ORM operations include their normal value/SQL construction and cache lookup. Every model workload decodes the same three owned fields. Trivial scalar results are close to timer granularity; absolute microsecond differences are more useful than large scalar ratios.

File and memory modes both report SyncMode Full, page size 4096, cache size −2000, and MVCC enabled. Reports retain the actual passive-checkpoint option. MemoryIO and PlatformIO have different persistence behavior; checkpoint thresholds and power-loss guarantees were not measured. File mode is not asserted to have PostgreSQL-equivalent durability.

## Allocation and streaming evidence

Allocation cells are successful allocations/reallocations per operation, followed by requested allocated bytes per operation. These are medians of nine rounds from three separate profiling processes. Their timings are excluded from the latency comparison.

| Storage | Workload | Direct engine | Driver | ORM |
| --- | --- | ---: | ---: | ---: |
| memory | scalar | 0.0/0.0; 0 | 11.0/0.0; 3,696 | 16.0/0.0; 5,769 |
| memory | params16 | 0.0/0.0; 0 | 11.0/0.0; 4,176 | 16.0/2.0; 7,445 |
| memory | model_lookup | 22.0/0.0; 7,191 | 33.0/0.0; 11,007 | 77.0/0.0; 17,754 |
| memory | update16 | 32.0/2.0; 7,737 | 36.0/2.0; 8,409 | 98.0/6.0; 15,548 |
| memory | stream | 158.0/11.0; 294,592 | 427.0/11.0; 321,808 | 465.0/11.0; 328,043 |
| file | scalar | 0.0/0.0; 0 | 11.0/0.0; 3,696 | 16.0/0.0; 5,769 |
| file | params16 | 0.0/0.0; 0 | 11.0/0.0; 4,176 | 16.0/2.0; 7,445 |
| file | model_lookup | 22.0/0.0; 7,191 | 33.0/0.0; 11,007 | 77.0/0.0; 17,754 |
| file | update16 | 32.0/2.0; 7,650 | 36.0/2.0; 8,322 | 98.0/6.0; 15,461 |
| file | stream | 158.0/11.0; 294,592 | 427.0/11.0; 321,808 | 465.0/11.0; 328,043 |

The harness control records zero allocations. The scalar driver cost is stable at 11 allocations and 3,696 requested bytes; ORM is 16 and 5,769. Code inspection identifies per-request control/channel storage, native parameter values, owned row bytes/ranges, and an eager 32-row batch reservation as contributors. Aggregate counters do not attribute exact counts or bytes to individual sites. Async scheduling and the owned-row contract remain costs even if batch reservation is reduced.

The ORM 16-parameter case consistently records two reallocations. A separate,
unmeasured follow-up is to reserve the known parameter count before collecting
validated native type hints in `ValueSql::native_parameter_types`. Its current
fallible iterator collection may grow the vector incrementally. This is a
candidate to investigate, not established per-site attribution, a claimed
latency regression, or part of the batching change.

The streaming profile pauses driver/ORM consumers for 50 ms, then consumes every row without retaining the result set. The table shows median peak requested live bytes above the starting snapshot, with the range across three processes.

| Storage | Layer | 1,000 rows | 100,000 rows | 1,000,000 rows |
| --- | --- | ---: | ---: | ---: |
| memory | direct | 8,498 (8,498–8,498) | 8,499 (8,499–8,499) | 8,499 (8,499–8,499) |
| memory | driver | 26,594 (23,705–31,610) | 30,123 (28,203–30,275) | 30,395 (30,395–32,411) |
| memory | orm | 23,718 (23,717–25,734) | 29,694 (27,126–30,175) | 32,423 (30,046–32,423) |
| file | direct | 8,498 (8,498–8,498) | 8,499 (8,499–8,499) | 8,499 (8,499–8,499) |
| file | driver | 23,705 (23,705–25,722) | 30,315 (26,523–32,411) | 32,411 (32,411–32,411) |
| file | orm | 25,734 (25,734–26,198) | 30,175 (26,543–32,431) | 30,287 (30,047–30,407) |

Peak storage remains bounded in these workloads even though cumulative allocations grow with rows consumed. The direct engine profile also includes owned model decoding; the driver adds shared row storage before decoding that model. These counters cover the Rust global allocator across worker threads, not C allocator activity, fragmentation, stacks, memory mappings, RSS, OS caches, or byte copies. No copy-count claim is made.

All individual percentiles, elapsed throughput, allocation ranges and process variation remain in [summary.json](turso-before-2026-11-25/summary.json), with the full [generated table](turso-before-2026-11-25/summary.md) and individual raw reports alongside it.

## Focused batch-allocation experiment

Commit `2f118b501d43f8417aacb20718439d8a32ecdf31` retains the first row inline,
reserves the existing 32-row vector only when a second row arrives, and avoids
reserving an unused vector after sending a full batch. The row encoding,
channels, scheduling, cache keys and public API remain the same. Observable
row-count, byte-limit and pending-error regressions, the full 112-test native
suite, and strict driver Clippy passed before measurement.

The candidate source fingerprint is
`7eca24f3d3df8cd1387d7daa8c05f373c4e437df`. Native latency executable hashes are
`bfcb216c846553c194d7a286e079a120a0efa16f` before and
`9a4f2058d927a583be61f22430acc26c364c765c` after. Profiling executable hashes are
`f4102e89f57e96bea41ee48d22d373f0ff593d72` before and
`5b81f2172ce144de35cc6a2747a78a1d07dae197` after. Compiler, locked dependency
identities, features, profiles and recorded build environment match. The
PostgreSQL feature-off executable is byte-identical; PostgreSQL was not timed
again, so the earlier inconclusive PostgreSQL comparison remains scoped to
the implementation before this native-only change.

The first focused window ran 60 processes from 16:39:35 to 16:46:23 UTC,
including separate latency/allocation runs and growing-stream profiles. All
result and provenance checks passed. Load averages rose from
28.84/44.61/50.49 to 72.10/50.75/48.95; the ending process snapshot included an
external fixtures executable reported at approximately 667% CPU. Exact
conditions, runner SHA256 and commands are in its [window record](turso-batch-2026-11-25/window.json).

The allocation effect was identical in memory and file mode across all nine
rounds per variant:

| Operation | Before | After | Change |
| --- | ---: | ---: | ---: |
| Driver scalar: allocation calls | 11 | 11 | unchanged |
| Driver scalar: requested bytes | 3,696 | 1,712 | −1,984 (−53.7%) |
| ORM scalar: allocation calls | 16 | 16 | unchanged |
| ORM scalar: requested bytes | 5,769 | 3,785 | −1,984 (−34.4%) |
| 128-row stream: allocation calls | workload-dependent | one fewer | −1 |
| 128-row stream: requested bytes | workload-dependent | 2,048 fewer | −2,048 |
| 128-row stream: reallocations | 11 | 11 | unchanged |

Driver stream allocation medians changed from approximately 427.016 to
426.016 calls per operation; ORM changed from 465.016 to 464.016. Fractional
counts reflect totals divided by 5,000 operations. Across all growing
stream sizes and repeats, the maximum observed requested live bytes above
the starting point changed from 32,411 to 30,363 for the driver and from
32,431 to 30,383 for ORM, in both storage modes. Individual peaks still vary
with producer/consumer scheduling. These are heap-counter results, not RSS or
copy measurements; moving a row inline does not make its storage disappear.

The first window's timing schedule had an ordering flaw: reversing storage
order while including its index in the variant-order expression canceled the
intended variant reversal for each fixed storage mode. The original evidence
is preserved. Its candidate stream throughput was lower in most pairs, while
the unchanged direct-engine control was lower in all six pairs. Those results
do not justify declaring throughput unchanged or attributing the difference
to the batching code. The [complete first-window summary](turso-batch-2026-11-25/summary.json)
retains the unfavorable timing results as well as the allocation evidence.

## Balanced stream follow-up and decision

The corrected runner used `--stream-latency-only --repeats 4`, producing 16 processes with three rounds each. For each fixed storage mode, two pairs ran before-first and two ran after-first. Both original executables were reused without recompilation. All result, source and executable checks passed. The run lasted 140.48 seconds, from 16:49:49 to 16:52:10 UTC. Load averages changed from 39.62/46.86/47.67 to 17.51/35.05/42.74; it remained an uncontrolled shared host.

The corrected runner SHA256 is `14ef7b35154ff1ac3089c8cbcf8f86da604f1d397abe4f6338d4330b497bf960`. The [window record](turso-stream-followup-2026-11-25/window.json) preserves the exact command and observations; [summary.json](turso-stream-followup-2026-11-25/summary.json) and the individual reports retain per-round latency percentiles and elapsed throughput.

Throughput below is queries per second: the median of four process medians, followed by their minimum–maximum. Each query consumes 128 decoded model rows.

| Storage | Layer | Before | Candidate |
| --- | --- | ---: | ---: |
| memory | direct | 7,546 (6,463–9,178) | 7,250 (6,599–7,512) |
| memory | driver | 5,730 (4,969–7,499) | 5,028 (4,364–5,780) |
| memory | orm | 5,640 (4,166–7,264) | 4,982 (4,562–5,638) |
| file | direct | 8,476 (7,435–9,068) | 9,495 (8,313–9,975) |
| file | driver | 5,858 (5,664–5,868) | 6,274 (5,469–7,424) |
| file | orm | 5,598 (5,199–6,191) | 6,372 (5,172–7,078) |

The next table shows paired candidate/before throughput ratios. The last two columns divide each adapter ratio by the direct-engine ratio from the same process pair. This is a diagnostic for shared variation, not a causal correction or an isolated measurement of adapter cost.

| Storage | Repeat/order | Direct ratio | Driver ratio | Driver/direct ratio | ORM/direct ratio |
| --- | --- | ---: | ---: | ---: | ---: |
| memory | 1, before-first | 1.088 | 1.163 | 1.069 | 1.149 |
| memory | 2, after-first | 1.121 | 0.954 | 0.851 | 0.781 |
| memory | 3, before-first | 0.786 | 0.679 | 0.863 | 0.992 |
| memory | 4, after-first | 0.814 | 0.701 | 0.862 | 0.954 |
| file | 1, before-first | 1.118 | 1.095 | 0.979 | 0.826 |
| file | 2, after-first | 1.015 | 1.081 | 1.065 | 0.971 |
| file | 3, before-first | 1.148 | 0.932 | 0.812 | 1.034 |
| file | 4, after-first | 1.185 | 1.269 | 1.071 | 1.149 |

**Decision: reject the optional batching optimization.** Lower memory-mode driver throughput remained in three of four balanced pairs and in both orders, including relative to the direct control. File-mode results went in both directions and improved in aggregate. These observations do not establish a universal regression or precise causal penalty, but they leave enough throughput uncertainty to reject the change given pgorm’s performance priority.

Production retains the pre-experiment batch-storage implementation; the useful row-count, byte-limit and pending-error regression tests remain. The candidate’s 1,984-byte scalar saving, one-allocation stream saving and lower observed heap peaks are real experimental results, **not improvements in the shipped implementation**. Reconsidering the candidate requires a controlled-host throughput investigation. No further production optimization was folded into this experiment.

The restored `crates/pgorm-driver/src/native/query.rs` is byte-for-byte identical
to the measured before implementation at `418d5a73` (SHA256
`380652ddc447ddfdfd5af594688b0df14f6649d1453eca079a369ae28ab88200`). The final
runner only makes the binary-selection conditional easier to read after the
follow-up; raw windows retain the hashes of the runners actually executed.
