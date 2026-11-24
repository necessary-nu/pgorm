# sqlmap verification

The Rust runner drives the upstream scanner against a loopback HTTP adapter
and a disposable PostgreSQL container. The case manifest names the actual
pgorm API exercised by each protected route and its vulnerable control.

## Run locally

Use Docker, Rust (CI uses the stable toolchain), a C compiler, and
any release of the Python series `pins.json` names (3.14). The runner refuses
an interpreter outside that series and records the release it used in the
report's `interpreter`. It verifies the upstream archive digest, uses the
pinned PostgreSQL image, and accepts no external scan target.

```sh
cargo test --locked --manifest-path security/sqlmap/adapter/Cargo.toml --tests
cargo build --locked --manifest-path security/sqlmap/adapter/Cargo.toml --bins
security/sqlmap/adapter/target/debug/sqlmap-harness --profile smoke
```

Initial full acceptance also requires the direct security regressions against
the same disposable server. Generate the root workspace lockfile first if it
does not exist; preserve it with the report for dependency reproduction.

```sh
cargo generate-lockfile
security/sqlmap/adapter/target/debug/sqlmap-harness \
  --profile full --direct-regressions --artifacts target/sqlmap/full-run
```

Artifact directories must be new. `--python PATH` selects the interpreter.
`--case ID` runs a named subset; its result cannot stand for a complete
profile. `--baseline-only` is a diagnostic and always exits nonzero.
The runner also runs and records the direct regressions. Python is needed only
to execute the pinned scanner itself; the harness around it is entirely Rust.
`sqlmap-verdict <profile> <artifact directory>` turns a finished run's
`run/report.json` into the `ci-status.json` and `ci-summary.txt` CI publishes.

Every setting in `profiles.json` reaches the run. `dbms`, `level`, `risk`,
`concurrency`, `retries`, `http_timeout_seconds`, `time_sec`,
`union_columns`, `union_type_fuzzing` and `verbosity` become scanner
options; `postgres_statement_timeout_seconds` and
`postgres_lock_timeout_seconds` are set on the disposable server; and
`case_timeout_seconds` bounds each scan. Each case's `field` in `cases.json`
is the parameter under test: the scanner's `-p`, the only query parameter
the adapter route reads, and the only injection point a finding may name.
The runner refuses a profile with a missing or unrecognised field, or with
timeouts that do not strictly increase from the timed sleep to the scan
deadline. The other scanner options are the same for every profile: batch
mode declining sqlmap's offer to test past the declared level and risk, a
flushed session with fresh queries, no proxy, no colour, and per-case output
paths.

To reproduce a report, check out its recorded source revision, restore any
recorded source changes and workspace lockfile, use the accompanying
`pins.json`, `cases.json` and `profiles.json` and the Python release the
report's `interpreter` records, then run the recorded profile and subset into
a fresh artifact directory. `command.json` files retain individual scanner
invocations; their loopback addresses belong to the old fixture and must be
replaced by a newly provisioned adapter.

## CI and evidence

`.github/workflows/sqlmap.yml` runs smoke on relevant pull requests, and full
and identifiers weekly or through **Run workflow**. All three call
`sqlmap-run.yml`, which uses the same local runner and profiles, and each has
its own check name. Fast runner tests do not need Docker or a live scanner.
CI installs the latest patch of the Python series in `pins.json`. A new push
to a pull request cancels its superseded smoke run, but every scheduled or
dispatched run has its own concurrency group, so dispatching the workflow
again never cancels a full or identifiers scan already under way.

The identifiers profile joined the schedule on its measured cost. Its 28
scheduled pairs took 13.5 minutes of scanning in each of two measurement runs,
and no single scan took longer than 45 seconds against its 300-second deadline.
That is cheap enough to run every week beside the full profile. It is not
cheap enough to add to every pull request, whose smoke gate scans three pairs.

The scan deadlines are 30 minutes for smoke, 60 for identifiers and 300 for
full, inside 45/90/330-minute job limits. SIGTERM gives the runner time to stop its process
groups, clean up its fixture, and retain an incomplete report. Hitting a
deadline fails the job. Scanner work stays outside the pre-commit checks.

Jobs use hosted runners, read-only repository permissions, no repository
secrets, and no persistent checkout credentials. Fork pull requests use the
same smoke job after any required repository approval. A pending approval,
disabled workflow, setup failure, skipped check or unavailable Docker is
**not a pass**. `ci-status.json` records `not-run` when setup never reaches
the runner. Integration requires a successful smoke check for the proposed
revision; if repository policy prevents a fork run, a maintainer must run
that revision on a trusted repository branch before integrating it. This
workflow does not change branch-protection settings.

Artifacts are uploaded even on failure and retained for 14 days. They include
the report, profile/manifest/pins, scanner logs and JSON results, request
evidence, database identity/settings, source and binary identities, cleanup
status, runtime information and the workspace dependency lock. Copy acceptance
evidence to durable storage before the CI retention window expires.

A pass covers only the named, pinned manifest/profile. All scheduled scans
must complete, all required vulnerable controls must be detected, every
protected case must pass, and cleanup must succeed. Scanner silence is not
proof that every possible ORM query is injection-free. At an identifier
position it is evidence only for a pair whose control fires at the profile's
level, and most identifier pairs have no such control at level 3. See
[Identifier positions](#identifier-positions-the-render-oracle).

The [2026-09-09 acceptance attempt](acceptance/2026-09-09.md) completed all
420 scans and passed the direct regressions, but failed acceptance because
105 required controls were not detected. Its case matrix records the remaining
work; it is not a passing acceptance report.

## Declared inapplicability

[CONTEXTS.md](CONTEXTS.md) derives, from the pinned scanner's own payload and
boundary XML, which case/technique pairs no faithful control can ever reach.
Each case in `cases.json` carries an `inapplicable` map keyed by technique:

```json
"Q": {
  "reason": "<why no faithful control exists at this injection point>",
  "evidence": {
    "kind": "no-boundary",          /* or no-attachable-position */
    "payload": "inline_query.xml",  /* must define the exempted technique */
    "where": [3],                   /* qualifying <where> at this level/risk */
    "clause": [1, 2, 3, 8],         /* qualifying <clause> at this level/risk */
    "boundary": "<the boundary the context would need, and why it is absent>",
    "contexts": "4",                /* the CONTEXTS.md section that derives it */
    "ceiling": 5                    /* optional: the level whose boundary reaches it */
  }
}
```

`kind` names which argument the reviewer is checking. `no-boundary` means the
catalogue holds no boundary that can pair with those `where` values and still
escape this context — the inline-query case, where exactly one of the 53
boundaries serves `where=3` and it carries an empty prefix and suffix.
`no-attachable-position` means a boundary does pair, but the control's grammar
offers no slot the technique's vector can attach to.

`ceiling` marks an exemption that holds only below a scanner level, because a
boundary at that level was measured to reach the context. A profile below the
ceiling treats the pair as inapplicable. A profile at or above it schedules the
pair, and its control must then fire like any other. A declaration without a
ceiling applies in every profile that schedules its case. On the eleven
identifier cases, which the level-5 identifiers profile schedules, such a
declaration's evidence names the level-5 boundaries that were tried and cites
`where` values that qualify at level 5 / risk 3.

Both runners refuse a declaration that omits a reason or any evidence field,
carries a surplus field, cites a payload file that does not define the
technique, gives a ceiling that is not a scanner level from 2 to 5, or names a
technique the case does not declare. Exempted pairs leave the scheduled
work, appear under `inapplicable` in the report and CI summary with their
evidence, and are never counted as passes.

A profile run never scans an exempted pair, so it cannot notice a declaration
going stale. Re-measure them explicitly:

```sh
security/sqlmap/adapter/target/debug/sqlmap-harness \
  --profile full --remeasure-exemptions --artifacts target/sqlmap/exemptions
```

This scans each declared pair's control, alone, with its exempted technique
under the profile's settings; `--case ID` narrows it. A pair ends `held`
(complete scan, no detection), `falsified` (the scanner detected the
technique, so the pair was never inapplicable; it is listed in
`falsified_exemptions`) or `incomplete`. The run passes only if every pair
held. It is evidence about the declarations, never a profile pass:
`sqlmap-verdict` reports such a report as incomplete. CI does not run it, as
the full profile's 93 declarations would add a second long scan to the
schedule.

In the full profile, 93 of 210 pairs are declared inapplicable (Q 35, U 17,
E 12, T 12, B 11, S 6), leaving 117 scheduled. Twenty of those 93 carry
`ceiling` 5 (E 8, T 8, B 4). The identifiers profile schedules them alongside
the eight identifier pairs that already fire at level 3: 28 of its 66 pairs,
with 38 declared inapplicable. At level 3, `schema`, `function`, `column`,
`group`, `pipeline-projection` and `stored-identifier` have no scheduled
technique. At level 5, only `schema` and `function` have none. Only CONTEXTS.md
sections 7a, 7d and 9 are exempted. The section-7c control-shape cells are
reshaped to fire at level 3 — `insert` (INSERT … SELECT … WHERE),
`update-value` (value in the WHERE), `cast` (CAST target inside a WHERE) and
`enum` (an enum column filtered on the cast) now reach every scheduled
technique.

Double-quoted identifier contexts are where the exemptions concentrate, because
every boundary that closes a `"` below level 5 appends a comparison between two
invented double-quoted identifiers. A technique whose tests carry no `<comment>`
at level 3 — error-based and time-based — can only use that suffix, so it never
produces a statement PostgreSQL will resolve, whatever the surrounding SQL.
Level 5 adds two boundaries that change this, and CONTEXTS.md section 9
records what each reaches. The clause-8 `"="[ORIGINAL]"` boundary turns a
projected or sorted identifier into a self-comparison the payload can join.
The clause-9 `" WHERE [RANDNUM]=[RANDNUM]` boundary opens a WHERE clause after a
FROM or alias tail.

## Identifier positions: the render oracle

The identifiers profile scans the eleven identifier cases at sqlmap's highest
strength, level 5 and risk 3. Measured twice
([2026-10-05](acceptance/2026-10-05-identifiers.md)), 28 of their 66 controls
fire there, and both runs agreed on every pair. For those 28, a clean protected
route is the same kind of evidence it is anywhere else in this suite. The other
38 are beyond this instrument at any level:

- `schema` and `function`, all six techniques;
- boolean, UNION and stacked queries at a leading projected identifier
  (`column`, `group`, `pipeline-projection`, `stored-identifier`);
- everything but stacked queries in the CREATE TABLE column definition
  (`enum-ddl`);
- UNION after ORDER BY (`order`);
- inline queries in all eleven.

Those positions are judged by a separate, structural instrument: the
identifier render oracle (`tests/identifier_oracle/`, run by
`tests/identifier_oracle_tests.rs` and specified as
`[spec:pgorm:req:security.ident-oracle+19]` in
[`docs/spec/ident-oracle.md`](../../docs/spec/ident-oracle.md)). It covers:

- the 38 identifier pairs no sqlmap boundary reaches;
- the identifier positions behind the 28 that sqlmap does reach;
- every other public API that renders a caller-supplied name.

Scanner silence says nothing about the first group. The oracle does
not probe them with payloads. It renders each registered name position with a
hostile-name corpus, parses the statement with pgsql-syntax, and requires the
name to come back as exactly the identifiers the position should produce,
with the rest of the parse tree unchanged. A live leg runs the nastiest names
against a real server.

That instrument, not this one, is where identifier-injection coverage is
claimed. The identifiers profile shows which identifier contexts a scanner can
exploit. The oracle shows that pgorm never emits a name that leaves its
identifier. It has found defects this suite could not reach:

- keyword names read as grammar at type and function positions;
- pipeline names beginning with `$`, which become placeholders or dollar
  quotes;
- a pipeline cast type written verbatim.

Each is filed as a plan node and pinned in the oracle until fixed.
