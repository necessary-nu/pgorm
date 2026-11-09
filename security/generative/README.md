# Generated pgorm campaigns

This test package implements the `generative` WBS and
[generated-program specification](../../docs/spec/generative.md). Its target is
generated public Python builder compositions, independent PostgreSQL checks,
reproducible Python/Rust failures and a separate bounded Rust compile suite.
The Python module is supplied by `pgorm-python`; this package is test
infrastructure and is excluded from its distributions.

The [portable program format](PROGRAMS.md) defines typed operation references,
exact value tags, binder/transaction scopes and the versioned coverage matrix.

## Fixture checks

From the repository root:

```sh
PYTHONPATH=security/generative/src python3 -m unittest discover \
  -s security/generative/tests -p 'test_*.py' -v
PYTHONPATH=security/generative/src target/python-check/bin/python \
  security/generative/tests/live_fixtures.py
```

The live command requires Docker and an installed native `pgorm` package in the
selected interpreter. It creates its own pinned PostgreSQL container; it has
no external database URL option. Two subject/reference database pairs verify
baseline equivalence, independent state, reset, restricted privileges and
statement deadlines. Its evidence is in `target/generative-fixtures/`.

`Fixture` owns the container and anonymous data volume, with memory, CPU,
process and command limits. It publishes PostgreSQL only on loopback. Programs
receive connections for the restricted `campaign` role. Administrative access
stays in the fixture controller; credentials are excluded from its report and
the portable baseline definition. Cleanup verifies the ownership label and
removes the container and volume even after startup failure or cancellation.
Failure to establish cleanup fails the command.

`baseline.default()` supplies multiple tenants, NULLs, duplicate sort keys,
matched and unmatched joins, protected sentinels, schema collisions, quoted
identifiers and qualified enums, plus exact numeric, JSON, array and temporal
data. Subject and reference databases receive the same portable definition
through independent setup SQL. `Fixture.reset()` restores rows on both sides
before a new attempt, retaining enum identities for reusable connections.
Schema-changing programs require `rebuild=True`; close both pools before that
rebuild so PostgreSQL type metadata cannot outlive the replaced enum types.
State persists between operations within a program.

The live report records the pinned image and actual database image/version,
encoding, collation, timezone, search path, string mode, deadlines, restricted
role flags and baseline hash. Fixture success alone does not establish
generated-program coverage or the complete campaign's acceptance.

## Native executor

Build the private application extension with an interpreter that has Maturin
installed, then run probes in its installed environment:

```sh
PYTHONPATH=security/generative/src target/python-dev/bin/python -m pgorm_campaign.build
PYTHONPATH=security/generative/src target/generative-build/venv/bin/python \
  security/generative/tests/live_wire.py
PYTHONPATH=security/generative/src target/generative-build/venv/bin/python \
  security/generative/tests/live_executor.py
```

The first command records native source/lock hashes, revision, features, ABI,
toolchain, wheel/extension identity and build timing. Repeating it with the same
identity verifies and reuses the installed artifact. Native builds use a separate
Cargo target to avoid mixing application and standalone extension outputs.

The private Rust crate registers `campaign.Account`, `campaign.Note`, required,
optional and self-join graphs through seven sources, and pipeline `SourceList`
tuples through six. `campaign.Room` is keyed `(id, valid_at WITHOUT OVERLAPS)`
and `campaign.Stay` references a room over its own period, a temporal foreign
key checked at commit, and an account the server never checks; the
`campaign.StayRooms` and `campaign.StayGuests` graphs join by those two
relations. A program that uses them appends `rooms` and `stays` to its own
fixture, with `btree_gist` installed in the fixture schema for the room key's
scalar part, so the shared baseline is unchanged for every other program. Its Rust modules remain usable by standalone replay without
Python registration code. The Account projection includes scalar, enum, JSON,
decimal, UUID and temporal fields. The nullable-element `tags` array is exercised
through public records/runtime descriptors; it is not a field of this compiled
entity shape. Account's native `before_save` hook appends `|hook` on insert and
increments a supplied rank on update; Note uses the default hook behavior.

`Executor` accepts a validated `Program` and an owned `Fixture`. It retains the
same asyncio loop and bounded pools between programs, resolves earlier results
at their consuming effects, and creates private binder expressions inside the
actual public callbacks. Runtime code is prohibited from launching subprocesses.
Trace records preserve selected native paths, native value snapshots, tagged
rows, optional tuple members, stream state, errors and affected-row counts.
`executed` means observations were collected; an independent oracle must still
decide correctness. It is not a passing campaign verdict.

The 41 handwritten live probes exercise all 89 declared instructions and the
eight effects they were written for, plus timeout, cancellation, invalid-input,
inactive-dispatch and build prohibition checks. The three version-returning
write effects added since (`entity.change`, `entity.changes` and
`entity.upsert`: `UpdateOne::exec_returning_change`,
`UpdateMany::exec_returning_changes` and `Insert::exec_returning_upsert(s)`,
each row observed as its old and new versions, the old one absent for a row
an upsert inserted) are exercised by the generated active-family programs. They verify 125 exact scalar/null/array value snapshots and
pool reuse separately. Each executor run has a fresh evidence directory under
`target/generative-executor/`. This is executor validation, not the generated
full campaign or million-program stress proof.

Findings are retained under [findings](findings/). The distinct/append fix now
passes Python and standalone Rust reproduction plus independent live inspection
and eight-row fetch regressions. A chained set-operation program still loses its
expected grouping. New native count/window discrepancies are retained under
[window-semantics](findings/window-semantics/README.md): a nullable-column count
counts NULL rows, and first/last omit an explicitly authored row frame.
Full generation, complete oracle coverage, shrinking,
general Rust emission, compile campaigns and acceptance remain separate WBS work.

## Independent checks in development

```sh
PYTHONPATH=security/generative/src target/generative-build/venv/bin/python \
  security/generative/tests/live_oracles.py
```

`Checker` runs the public native executor and a separate Psycopg 3 reference
against the paired fixture databases. The reference interprets the operation
graph using its own SQL algebra, quoting, bound values and binary result decoder.
It observes final rows and schema definitions on both sides through Psycopg.
The native build preparation installs the oracle dependency separately;
dependency preparation does not invalidate the native compilation cache.

The current probes cover CRUD and result reuse, runtime and compiled models,
ActiveModel states/hooks, graph/source tuple shapes, cursors, pipeline stages,
windows and set operations, enum/table/index DDL, patterns, transactions and
streams. Typed scalar/array probes retain NULLs, enum identity, float bits,
decimal scale and temporal precision. Rows compare as multisets unless ordering
is explicitly asserted. An unordered partial stream must be a submultiset of
the reference rows with the required count; its closure/cancellation state has
an explicit invariant accounting for the cancellation race.

Literal reference values remain bound, with independent PostgreSQL type
inference for integers, finite floats, decimals, unknown strings/NULLs and
arrays. An interval literal carries its own cast, so the reference binds its
own exact spelling (`+1 mon -2 day +3000004 us`, every field signed so no
`IntervalStyle` can move a sign) cast to `interval`, and decodes `interval_send`'s
sixteen bytes itself rather than through a driver loader that would fold the
three fields into one duration. Literal temporal values follow the whole-second conversion explicitly
specified by `sql.render.value-literals+2`; bound temporal values retain their
fractional seconds, down to the microsecond the wire format admits and no
further — finer precision is rejected at the boundary rather than rounded, so a
library's rounding rule can never be mistaken for a pgorm defect. This
conversion happens in the reference semantics, never in result comparison. Ordinary and pipeline float literals have separate type
inference rules (for example, inline SQL `1` versus pipeline `1.0`).

Pipeline sort keys survive filtering, projection, take, joins and embedded
sources as private reference columns removed at the terminal. Group/distinct
reset output order. Ranked windows, NULL aggregates and empty frames have
dedicated live probes. A development probe wrongly assumed distinct retained
order; that reference error was corrected, and its original run was retained.

The template helper checks the existing `RawSQL` placeholder API. PostgreSQL
first infers parameter types from the original owned SQL; the independent
execution then binds values without borrowing the subject's substitution lexer.
Inspected SELECT/condition/expression SQL can be checked by a read-only probe
against independently computed results. Inspection amid writes or transactions
needs an oracle at the inspection point and currently reports incomplete.

Enum utility statements cannot bind labels directly. The reference creates a
uniquely named helper function in its own fixture schema, binds names/labels as
arguments, uses PostgreSQL's `format('%I', ...)` and `format('%L', ...)` inside
that function, and drops it before final state comparison. It does not require
temporary-table privileges or change the restricted execution role. Helper
cleanup failure makes the check incomplete.

The independent oracle kernel is implemented. Non-finite inline floats, an
uninstalled vector type, some invalid conversion cases and native-parity
policies report incomplete. Ordered pipeline observations require a surviving
explicit sort. Unsupported semantics cannot establish a pass.
Further coverage and adversarial oracle validation remain necessary before
the full generated campaign can be accepted.

Each live check has a fresh directory in `target/generative-oracles/`, retaining
programs, native observations, reference results, state, comparisons and build
identity. The preserved chained-set and count/window findings produce **defect**
in this oracle self-check; detecting it verifies the checker and does not make
that program a passing campaign case. Two early probe failures were test setup
errors: PostgreSQL needs explicit parameter types for `$1 / $2`, and an i8
builder value uses a SMALLINT carrier, requiring an intermediate INTEGER cast
when deliberately decoding PostgreSQL's internal `"char"`. Their original run
artifacts remain in the earlier fresh directories.

## Sensitivity controls and attribution

```sh
PYTHONPATH=security/generative/src target/generative-build/venv/bin/python \
  security/generative/tests/live_controls.py
```

The versioned catalog contains 23 controls across 22 mandatory comparison
categories. Each starts with a passing public pgorm execution and independent
reference. Deliberately incorrect SQL runs through a separate driver in the
owned subject database, after both databases reset to the program's fixture.
Other controls corrupt that fresh native run's observations. Control observations
carry their own dispatch evidence and cannot claim native execution.

Controls cover missing value/name escaping, broadened predicates, swapped binds,
wrong bound/decoded types, missing/duplicate/reordered rows, optional joins,
SQL/JSON NULL distinctions, arrays, enum identity, float bits, decimal scale,
temporal precision, an interval's month normalised into thirty days, affected counts, omitted writes/schema, committed rollbacks,
stream closure and exact error causes. A valid control requires exactly its
declared difference; arbitrary failures, inactive or unchanged mutations, missing
work and a failed native baseline fail the command. Aggregate validation
recomputes comparisons from retained evidence instead of trusting status labels.
The live check also verifies that the public wheel excludes these test modules.

Finding retention writes original programs, fixtures, observations, identities,
replay arguments and content hashes into a fresh directory. Attribution requires
completed standalone Rust evidence from the same native source identity and
rechecks both independent verdicts before comparing observed behavior. Native
reproduction locates a discrepancy below Python; reference semantics still need
review before blaming the library. Missing/inconsistent native evidence remains
unattributed and never removes the original Python failure. This machinery does
not patch production code or close findings. General Rust emission and full
Python/Rust execution parity remain the separate replay node's work.

## Input corpus

`corpus_builtin.builtin()` provides 319 repository-owned inputs: hostile text,
identifier lengths around PostgreSQL's 63-byte boundary, exact numeric limits,
IEEE float bits, typed NULLs, arrays (including empty/NULL/NULL elements), JSON,
Decimal, UUID, enum, temporal, interval, network, bytes and vector values. The
intervals mix signs across fields (`1 mon -2 days`), take each field to its
limits, hold a single microsecond either side of zero and both infinities, and
pair `1 mon` with `30 days` and `1 day` with `24 hours`: values PostgreSQL's `=`
equates and the campaign keeps apart.
`corpus_random.sample(seed, index)` produces deterministic typed values using a
versioned SHA-256 counter, independent of iteration order or worker partitioning.
Neither function imports pgorm, starts PostgreSQL, accesses the network or needs
sqlmap. A valid portable value does not imply support in every PostgreSQL context;
the grammar must choose compatible contexts or declare a rejection check.

`Input.node(author, role="value")` inserts a value node. The `identifier` role
inserts that text through a name node. No role can turn corpus text into a raw
query or an intentional expression. Empty names, NULs and byte lengths are
reported as traits, without assuming an API-specific acceptance/rejection rule.

The optional sqlmap importer reads a local archive matching the immutable revision
and SHA-256 in `sqlmap_archive.py`. It does not run the HTTP scanner:

```sh
PYTHONPATH=security/generative/src python3 -m pgorm_campaign.sqlmap_import \
  --archive target/sqlmap-cache/sqlmap.tar.gz \
  --output target/generative-corpus/my-import --seed 20260911 --variants 2
```

The output must be a new directory. It retains the upstream LICENSE,
THIRD-PARTY notices, original XML including notices, importer source, content
hashes, exact substitution contexts, transformation rules and a complete
template/boundary inventory. Imported upstream data and its license notices stay
together in this optional artifact; the public pgorm wheel excludes the campaign.
The importer source and data hashes make the recorded transformations reviewable.
`sqlmap_import.load(directory)` validates artifact hashes, input types, identities
and counts before returning data for generation.

At the pinned revision there are 365 request templates and 53 boundaries.
329 requests are supported; 36 dynamic UNION requests need scanner-selected
projection topology and are explicitly inventoried as unsupported. Scanner
vectors and response oracles are also inventoried as omitted. With the seed and
two variants above, import produces 21,070 contextual inputs containing 18,655
distinct values. **These are input counts, with zero executed programs.**
Clause/placement compatibility and boundary metadata are retained; all DBMS
families supply hostile strings, without claiming valid PostgreSQL attack syntax.
No URL decoding, tamper processing, inference or scanner request emulation occurs.

Verify the real pinned archive, deterministic re-import, per-input reconstruction,
complete unsupported inventory, exact notice retention and corruption rejection:

```sh
PYTHONPATH=security/generative/src python3 security/generative/tests/live_corpus.py \
  --archive target/sqlmap-cache/sqlmap.tar.gz \
  --output target/generative-corpus/verified-import
```

## Generated programs

`grammar.generate(seed, index, family=..., mode=...)` composes 15 families:
SELECT, CRUD/result reuse, pipelines, registered entities, ActiveModel writes,
graphs, cursors, runtime models, typed values, schema changes, SQL grouping,
pipeline grouping/windows, set operations, compiled source tuples and owned SQL
templates. All calls go through the public Python module and its native builders.
There is no HTTP transport or scanner in this path.

Recursive expressions carry schema types and nullability. Pipeline transitions
track available projections, source aliases and binder ownership. PRQL grouping
and partitioned windows remove their keys from the inner expression scope.
Pipeline identifiers are drawn without a double quote, because `into_sql`
refuses one rather than escaping it (`pipeline.errors+5`); a valid program
carrying one could never pass.
Runtime-model inserts use unaliased tables, as required by that public API.
Sequences reuse observed results, actually reach both conflict actions and close
nested commit/rollback scopes; their inserts sometimes leave nullable columns to
the table's default, which the final fixture comparison then has to agree on. Read-only transactions and ordered stream prefixes
are generated separately. The invalid-mode rejection family labels each
deliberate rejection with its exact expected cause: a SQLSTATE for a database
rejection, or the binding's documented refusal text for one pgorm makes before
anything is sent — a quoted pipeline identifier among them, so the refusal the
valid grammar steers around is itself checked, and an insert batch with no rows,
which the binding refuses rather than sending, whichever terminal asked for it. `refusals.py` holds that text
once; the independent reference reaches the same refusal from its own model of
the rule. Unexpected valid-program failures remain failures.

Recursive text uses NUL-free corpus entries up to 4 KiB and identifiers have a
smaller byte budget. The type family separately draws exact built-in and random
numeric, float, Decimal, JSON, array, enum, temporal and interval boundaries,
through bound and literal paths with explicit PostgreSQL types. A finite interval
is also projected beside a respelling PostgreSQL's comparison treats as equal (a
month moved to 30 days, or a day to 86,400,000,000 microseconds) and the server's
`=` on the two: both decode as the distinct triples they are, and both sides'
servers answer true. It pins the local-time worker
to UTC. It does not draw u64 or vector values, which have no installed
PostgreSQL type to round-trip through; their cells are the rejection family's,
which carries what fits int8 and checks the refusal of what does not, and has
every vector shape refused at the missing type (see `PROGRAMS.md`).
Observation-based coverage of every matrix context remains a full-profile
obligation; these samples do not establish it.

Fast tests check structural diversity after removing literal payloads, stable
generation, all catalog operations/effects across 2,250 generated programs under
128-node budgets, result dependencies and binder/source ownership. Generation
counts establish reachable productions, not executed database coverage. The live
command retains programs and recipes before execution, then native/reference
results, fixture evidence and the unchanged extension identity:

```sh
PYTHONPATH=security/generative/src target/generative-build/venv/bin/python \
  security/generative/tests/live_grammar.py --count 512 --seed 20260911
PYTHONPATH=security/generative/src target/generative-build/venv/bin/python \
  security/generative/tests/live_grammar.py --count 128 \
  --corpus target/generative-corpus/verified-20260911
```

`--corpus` verifies an existing offline import and records its manifest hash;
it neither downloads inputs nor runs sqlmap. `--family` selects a family for
focused verification, and `--program path/program.json` replays exact retained
instructions independently of later generator changes.

The seed `20260911` 512-program run `run-39373a66d2c1` completed with 476 passes,
32 exact expected rejections and four retained discrepancies, with no incomplete
cases. The 128-program imported-corpus run `run-37eb6fcba3f0` completed with 119
passes, eight expected rejections and one hidden-order discrepancy. Both use
the same installed extension and invoke zero builds. They correctly exit 1;
neither run is full matrix or million-program acceptance evidence.

Generated findings retain the original program, independent observations,
fixture, native identity and replay arguments with hashes. Open discrepancies
include [hidden sort columns](findings/pipeline-hidden-order/README.md),
[quoted output names](findings/pipeline-quoted-output/README.md),
[set composition](findings/set-precedence/generated-chain/README.md) and
[window frames](findings/window-semantics/generated-frame/README.md). A retained
[native panic](findings/pipeline-native-panic/README.md) is accounted as incomplete
execution while preserving cleanup and cancellation behavior. These findings
are separate from the fixed DISTINCT regression. The verifier exits unsuccessfully
for disagreements and incomplete cases; there is no passing allowlist.

## Shrinking

```sh
PYTHONPATH=security/generative/src target/generative-build/venv/bin/python \
  security/generative/tests/live_shrink.py \
  --program security/generative/findings/pipeline-hidden-order/program.json
```

`shrink.reduce(checker, program)` searches for a smaller program that still
fails the same way. It reduces effect sequences, transaction nesting, expression
and pipeline structure, bound values and the declared fixture, then re-validates
every candidate against the portable format before running it. An interval
shrinks toward zero one field at a time — all three zeroed, then each field
zeroed alone, then each brought to a single unit of its own sign — and never by
moving a month into days. Each candidate
goes through the same `Checker`, which restores its own declared baseline on
both databases first, so no attempt inherits the previous one's state.

A fixture reduction never drops a table another table's foreign key names, nor
rows from a table an enforced key points at. The referencing rows would point
nowhere and the baseline reset itself would fail, and a reset that fails
retires the fixture for the rest of the run rather than one candidate.

The predicate is frozen from a completed independent baseline run and is more
than the comparison verdict: it records the subject's own account of the failing
step — observation category, error class and SQLSTATE. A candidate that stops
failing, fails somewhere else, or reports incomplete for a malformed,
unsupported or fixture reason is recorded and discarded, never adopted. On the
retained hidden-order finding this matters concretely: a coarser predicate
accepted a reduction whose subject failed in PRQL compilation rather than with
PostgreSQL's `42703`, which is a different defect.

Budgets bound candidates, offered rewrites, passes, wall time and per-candidate
timeout, and all of them are recorded with the result. Interruption keeps the
original program and the best reproducer found so far. No result claims global
minimality, and the reports say so in as many words.

## Compile suite

```sh
PYTHONPATH=security/generative/src python3 -m pgorm_campaign.compile_main
```

A bounded Rust source-generation suite, separate from the runtime campaign and
reported separately from it. It covers the three `outside_runtime` entries in
`matrix.json` marked `compile-only` — entity derives and codegen output, Rust
borrow and brand rejection, fresh graph and source-tuple instantiations. An
installed binding fixes those shapes at build time, so no number of runtime
programs varies them.

Every case predicts its own outcome and is scored only against what the
compiler said about its own source. A positive must build. A negative must be
rejected by a named rustc error code inside its own line range — silence is
never a pass, and a diagnostic that lands on another case does not count for
this one. A `cargo` failure that produced no attributable diagnostic is a
toolchain failure, not a rejection, and marks every case in that batch untested
rather than passed.

Codegen cases visit `pgorm-codegen` first, through the detached driver in
`compile/`. A generation refusal — an extra-derive string that does not lex, DDL
the bridge does not carry — is the library holding its own boundary and is
counted apart from anything a compiler said; only what the generator emitted
reaches rustc.

Cases share a temporary crate when they are compatible: same verdict, phase,
kind and dependency set. Verdict is the hard constraint, since one exit status
cannot mean both "built" and "did not build". Crates are named by content
digest, because cargo keys freshness on package name and a fixed name sharing a
target directory reports a stale success.

The run writes `target/generative-compile/compile-report.json`, whose counts are
all labelled `compile_*` and which carries no runtime totals.

General Rust replay and dedicated CI remain separate unfinished WBS work. The
million-program stress demonstration and the acceptance assembly are built and
have been run; acceptance itself has not passed.

## Campaign profiles and the runner

```sh
PYTHONPATH=security/generative/src target/generative-build/venv/bin/python \
  security/generative/tests/live_campaign.py --profile smoke
```

`profiles.json` declares the versioned `smoke` and `full` profiles: coverage
obligations, generation and fixture limits, seed policy, worker count,
execution and shrink budgets, control requirements, and which of five run
classes each includes. The classes are counted in fields whose names carry the
distinction — `construction_only_programs_constructed`,
`live_database_programs_checked`, `invalid_input_programs_checked`,
`control_programs_checked` and the compile suite's own `compile_*` block. The
report contains no total across them, so a construction count cannot be read
as a count of independently checked database programs.

Every scheduled item ends with exactly one of pass, defect, expected-rejection,
invalid-control or incomplete. The command fails on a missing worker, a crash,
an unexpected panic, a deadline, an absent or malformed artifact, skipped work,
empty discovery or a cleanup failure — each a named fault, kept apart from the
verdicts because "we could not tell" is not a result. Aggregate success is
conjunctive: declared coverage satisfied, controls successful, and every
scheduled item accounted for.

Coverage is computed from observations, never from scheduled labels: a
constructed instruction with a recorded native path, a dispatched effect, a
value at the context its own instruction declares. The complete matrix is
evaluated on every run whatever the profile requires, so a smoke report states
its own gap rather than leaving it to be inferred.

Each run writes a fresh directory under `target/generative-campaign/`
recording source revision and dirty content identity, interpreter, pgorm, PyO3,
toolchain and dependency pins, extension hash, every artifact version,
PostgreSQL identity and settings, seeds, expected against executed work per
class, native call evidence, oracle decisions, timings and build counts —
including `extension_builds_during_campaign`, which a per-program build would
make non-zero. Credentials are scanned for and their presence fails the report.
A failing item retains its original program, a reduced reproducer within the
profile's shrink budget, the fixture definition, emitted Python and Rust
sources and concrete replay argument lists.

Smoke run `smoke-09b05f984b14` at seed 20260913 scheduled and recorded 208
items: 150 constructions (744 programs/second, zero database contact), 30 live
programs passing, six invalid-input programs producing their exact expected
rejections, and all 22 controls detecting their intended defect. 111 distinct
native API paths, 155 verified artifacts, zero builds, 18.4 seconds of runner
work inside a 22-second command. It satisfied its 20 declared obligations and
recorded 307 of 430 full-matrix obligations still outstanding: this is a
bounded smoke profile, not full coverage and not acceptance.

## Stress demonstration

```sh
PYTHONPATH=security/generative/src target/generative-build/venv/bin/python \
  -m pgorm_campaign.stress_main --target 1000000
```

Construction at scale through the installed native binding, sharded across
worker processes, deduplicated by program digest. It exists to show that the
generator and the constructor hold up over a million distinct programs without
per-program builds — and the build guard is the point: every shard constructs
inside an audit hook that refuses subprocesses, so the report can state
`per_program_builds: 0` and one unchanged native identity at entry and exit as
observations rather than intentions.

The report says what it is not, in the document itself. Construction is not
live coverage: no program here opens a database connection and none is decided
by an oracle, so the run neither replaces the runtime campaign nor proves any
ORM program safe. Distinctness is digest distinctness at the recorded
generation limits, not a claim that a million behaviours were covered. A
program that fails to construct is reported as a construction failure with its
cause and samples; it is never dropped to make a round number.

## Acceptance assembly

```sh
PYTHONPATH=security/generative/src target/generative-build/venv/bin/python \
  -m pgorm_campaign.acceptance_main \
  --campaign target/generative-campaign/<run>/campaign.json \
  --attempts security/generative/attempts.json
```

Five components are gathered and none is folded into another: the runtime
campaign, replay parity, the compile suite, the named regressions behind
resolved findings, and the stress demonstration. Each is read from the run that
produced it, keeping the vocabulary it was produced with, so a reader can tell a
constructor call from a compile invocation from an oracle decision. A component
that was never run is recorded as not run rather than defaulting to anything
that looks like a pass, and the command exits non-zero while acceptance is
incomplete — including when the only thing incomplete is an outstanding coverage
obligation.

Acceptance is not the union of things that passed. The retained finding
directories are checked against the registry in `acceptance.py` in both
directions: a finding added to the tree without an entry, or an entry whose
directory has gone, fails acceptance rather than quietly disappearing. Open
findings are listed by name, a runtime run's retained findings are listed by
item, diverging parity families are named, and stress construction failures stay
visible.

`attempts.json` records the profiles attempted, including any that produced no
usable verdict, so a report cannot read as having declined a profile it tried
and lost. Regressions are the one component this command can produce itself, at
the cost of building a second pgorm in a scratch worktree to show each named
test failing without its fix; `main` retains that evidence as `regressions.json`
and `--regressions` reads it back, the way every other component is already read
from a file.

Dated acceptance reports live in `acceptance/`. The current one is
[`acceptance/2026-09-14.md`](acceptance/2026-09-14.md): **acceptance has not
passed** — the full profile completed and failed with 33 defects, 4 incompletes
and 173 of 430 full-matrix obligations outstanding.
