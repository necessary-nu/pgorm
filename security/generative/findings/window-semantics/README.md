# Nullable count and explicit first/last frames

Status: reproduced through the public Python module, an independent live
PostgreSQL reference and standalone Rust. Both halves are now **fixed** — see
the two closing sections. No weakened comparison was applied to either.

`count(score)` is documented as `COUNT(expr)` in `src/pipeline/funcs.rs`, with
`count_rows()` separately documented as `COUNT(*)`. Both Python and Rust emit
`COUNT(*)` for the former. With fixture scores `[10, 10, 30, NULL]`, the live
window returns 4 instead of the independent `COUNT(score)` result 3.

`Over.rows(1, 1)` explicitly selects the following row. `first(score)` and
`last(score)` omit this frame in the generated SQL, while ordinary window
aggregates retain it. For id 2, the next row's score is 30: both reference values
are 30, while both subject values are 10. For the last row the frame is empty;
the subject's `first` still returns 10 instead of NULL.

The preserved `known-*.program.json` files isolate each discrepancy, and the
matching reports retain typed observations, both fixture states and comparisons.
`identity.json` records their fresh run and the unchanged extension/source
identity. `native.rs` and `python.py` bypass the campaign interpreter and require
no database. Their assertions retain the advertised count/frame expectations:
the recorded failures are findings, not passing regressions.

```sh
target/generative-build/venv/bin/python security/generative/findings/window-semantics/python.py
cargo run --locked --manifest-path security/generative/findings/window-semantics/Cargo.toml --target-dir target
```

The Rust crate's initial setup mistakes (a missing `ExprOps` import and using
`aggregate` on an ungrouped `Pipeline`) are retained in `compile-setup*.log`.
They are harness errors. `native.log` records the corrected crate compiling,
printing both discrepancies and then failing its semantic assertion.

When this was found, the oracle's aggregate default frame was the whole
partition, and `first`/`last` without an authored frame used PostgreSQL's
default window frame (see the end of the next section for today's). Those rules
are distinct from dropping an explicitly requested frame. Early development
runs used the wrong aggregate default and remain separate from the preserved
corrected reports. The self-check expects these known programs to report
`defect`; this verifies detection and does not make them clean campaign cases.

## The dropped frame is fixed

The frame half was a translation defect on the subject side, not a disputed
reading of `start`/`end`: the reference's `boundary()` and `Over::rows`'
documentation agree exactly. prqlc's `std.sql.prql` annotates only the
aggregates `@{window_frame=true}`, and `sql/gen_expr.rs::translate_windowed`
emits a frame clause only for an annotated call, so `first`, `last`, `lag`,
`lead`, `rank`, `rank_dense` and `row_number` were handed the authored frame
and dropped it. pgorm then wrote the whole `OVER (...)` clause for those seven
itself ([spec:pgorm:sem:pipeline.window-frame]). The retained artifacts here
record the discrepancy as found; `known-first-last-frame.program.json` no
longer reproduces the frame half.

Since the fork's rebase onto upstream main, prqlc's #6435 annotates `first`
and `last` as well, so prqlc carries an authored frame to them itself and
pgorm writes only the other five. An ordered window that authors no frame now
gives them the whole window, `ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED
FOLLOWING`, as it gives an aggregate, and the reference does the same. Before,
both sides read them under PostgreSQL's default frame, which ends `last` at
the current row's last peer.

## The nullable count is fixed

The reference was right and PostgreSQL settles it: `COUNT(expr)` counts
non-null values and `COUNT(*)` counts rows, so with scores `[10, 10, 30, NULL]`
the two answers are 3 and 4 and the subject was giving the second to both
questions. The cause was not a disputed reading either. `prqlc/src/sql/
std.sql.prql` declares `let count = column -> s"COUNT(*)"`, discarding the
counted column, and that is the right rendering *of PRQL*: `std.prql` documents
`count` as counting the relation's items, nulls included, and prqlc's resolver
replaces the argument with a null before lowering. pgorm's surface is what
separates the two — `count_rows()` is PRQL's `count this` and keeps prqlc's
rendering, while `count(expr)` is now written out by pgorm with the counted
expression interpolated, carrying no clause inside an `aggregate`, the
window's own inside a `window`, and `OVER ()` elsewhere
([spec:pgorm:sem:pipeline.count-argument]). `count_distinct` was already
correct: prqlc renders it with its column.

The retained artifacts record the discrepancy as found;
`known-nullable-count.program.json` no longer reproduces it.
