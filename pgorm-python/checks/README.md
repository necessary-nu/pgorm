# pgorm-python's checks

This is maintainer documentation: how pgorm-python is built and verified, and
what each check proves. Users of the package want [the Python
guide](../guide/README.md) instead.

Every check verifies an installed wheel in a fresh environment, never the
source tree's Python files. Each writes its evidence under `target/`, starting
with a `summary.json` that says `"passed": false` until the whole check has
passed, so a failed rerun never leaves an old passing status behind.
`--output PATH` chooses another evidence directory.

## The database wrappers

The live checks need a PostgreSQL 18 server with TLS. A wrapper provides a
disposable one, runs the command given to it with `PGORM_TEST_DSN` and
`PGORM_TEST_CA` set, and removes the server afterwards:

- `tests/with_postgres.py` starts a Docker container (`postgres:18.6-bookworm`,
  pinned by digest) with a test CA of its own. It waits up to 180 seconds for
  the container, or `PGORM_TEST_READY_SECONDS`.
- `tests/with_local_postgres.py` starts a cluster from the PostgreSQL 18
  binaries in `PGORM_TEST_PG_BIN`, for hosts without Docker, such as CI.

An existing server can supply the two variables directly; it must allow the
checks to create and drop their schemas, tables and databases. Neither wrapper
is a dependency of the package.

```sh
uv venv target/python-dev
uv pip install --python target/python-dev/bin/python 'maturin>=1.15,<2'
target/python-dev/bin/python pgorm-python/tests/with_postgres.py \
  target/python-dev/bin/python pgorm-python/checks/acceptance.py
```

A build for PostgreSQL 19 (`--features pg-19`) is tested against a 19 server
supplied through `PGORM_TEST_DSN`; the wrappers provision 18, and CI builds
and tests 18 only until 19's general release.

## The checks

| Check | What it verifies |
| --- | --- |
| `unit.py` | pgorm-python's Rust unit tests, against an interpreter whose `LIBDIR` holds a libpython (its docstring says why). |
| `typing.py` | The installed stubs match the native signatures; mypy (latest release, `--strict`) accepts the package, `examples/application.py` and every guide example, and refuses `tests/public_types_invalid.py`'s programs with the expected codes; the application runs. |
| `entities.py` | Builds the application wheel of `tests/application-binding`, runs its Rust parity tests, installs it and runs the registered-entity, graph, source and disjoint tests, with `execution_oracle` repeating the operations in Rust. Its wheel and report go to `target/python-entities`. |
| `codegen.py` | Builds a seed wheel, scaffolds `tests/codegen-entities`, builds the probe wheel, emits twice to check determinism, builds and installs the application wheel, runs `generated_application.py`, and type-checks valid and invalid consumers. Evidence goes to `target/python-codegen`. |
| `guide.py` | Builds the guide's registration of the bakery (the registrations chapter's `application.json` over `pgorm-guide-support`) the way `codegen.py` builds its application, then runs and type-checks every example the guide fences ```python registered. |
| `direct_builders.py` | The direct-builder integration suite, below. |
| `distribution.py` | Builds a wheel and a source archive through PEP 517, installs the wheel and runs the whole installed suite, `typing.py` and the native link report; then builds a second wheel from the archive and repeats it all. Evidence goes to `target/python-distribution`. |
| `notices.py` | Verifies, or with `--write` regenerates, `DEPENDENCIES.json` and `THIRD_PARTY_NOTICES.txt`, below. |
| `acceptance.py` | Runs all of the above in order, and checks that the default Rust workspace builds without Python tooling. Below. |

The installed suite itself is `python -m unittest discover -s
pgorm-python/tests`. It includes `test_guide.py`, which holds the guide's
chapter list to the Rust guide's and runs every standalone guide example in a
scratch database of its own; `PGORM_GUIDE_CHAPTERS=<id>,..` narrows it to some
chapters. `python pgorm-python/guide/examples.py run <id> ..` runs chapters'
examples directly, with `PGORM_TEST_DSN` set.

## Distribution builds

The distribution check requires Rust, a C compiler, libclang, CPython 3.14
and `uv`. Maturin (`>=1.15,<2`) is installed into temporary PEP 517 build
environments from the package's build requirements. It uses the committed
Cargo lockfile and separate Cargo target directories for the standalone and
source-archive builds; Cargo's dependency caches may be reused. Its tests
include real database operations, verified TLS, hostname and CA rejection,
cancellation and resource cleanup. The database wrappers' test CA is created
per run and is in no platform store, so the tests check both directions: the
test server is refused under the platform's store and accepted through
`cafile` and through `SSL_CERT_FILE`, and an empty, missing or root-less store
fails construction. `target/python-distribution/` keeps both wheels, the
source archive, their hashes, the type-check evidence and the extension's
dynamic library dependencies (`otool -L` on macOS, `ldd` on Linux). Nothing
here uploads to a package registry.

## CI and supported combinations

The [Python workflow](../../.github/workflows/python.yml) runs the
distribution check on macOS arm64 (the `macos-15` runner, PostgreSQL 18 from
Homebrew) and Linux x86-64 (Ubuntu 24.04, `postgresql-18` from the PostgreSQL
project's apt repository, since the Ubuntu archive stops at 16), with the
latest CPython 3.14 patch release, and keeps each job's packages and evidence
as review artifacts, failed jobs' partial evidence included. A third job runs
`entities.py` and `guide.py` on Linux and keeps their wheels and reports. The
workflow runs on pushes to `main`, on pull requests that touch Rust, Cargo or
`pgorm-python` files, and weekly, so a change elsewhere that breaks an
application binding is caught.

Configuring a CI job does not establish a passing result. A combination joins
`support.json`'s tested matrix only after its distribution check has passed.
Review the exact interpreter, OS, hashes and dynamic dependencies in that run
before using or publishing its artifacts. Registry publication is a separate
action and is not part of the workflow. The locally verified combination is
macOS 26.5.1 on arm64 with CPython 3.14.4; the configured macOS 15 and Linux
jobs remain candidates until their own runs pass.

## Dependency notices

`pgorm/DEPENDENCIES.json` inventories the entire locked Cargo graph, build
dependencies and platform-conditional packages included, so it is a superset
of any one wheel's; `pgorm/THIRD_PARTY_NOTICES.txt` holds the full notice
texts. After a dependency update, run:

```sh
python pgorm-python/checks/notices.py --write
python pgorm-python/checks/notices.py
```

The generator reads exact Cargo package metadata and the notices they
include. `licenses/supplemental.json` pins source URLs and hashes for upstream
notice files some crate archives omit; missing or changed evidence fails
verification, so review those inputs when updating the dependencies they
cover. The generated files and supplemental texts are committed, so installed
applications need no network access to read their notices. The
[supply-chain workflow](../../.github/workflows/supply-chain.yml) audits the
lockfile with `cargo deny` against the repository's `deny.toml`, weekly and on
pull requests that touch a manifest or lockfile.

## The direct-builder suite

`direct_builders.py` verifies the binding's founding claim: install `pgorm`,
compose its real Rust builders in Python, and execute them against PostgreSQL
in the same process, with no HTTP, query dispatcher or per-query compilation.

```sh
target/python-dev/bin/python pgorm-python/tests/with_postgres.py \
  target/python-dev/bin/python pgorm-python/checks/direct_builders.py
```

Optional `PGORM_DIRECT_SCHEMA`, `PGORM_DIRECT_ACCOUNTS` and
`PGORM_DIRECT_EVENTS` choose the application names; the schema must not
already exist. The defaults are fresh names with quotes, Unicode, percent
signs and backslashes. The suite drops only the schema it created.

The runner first compiles the independent Rust parity executable and builds
one wheel. It installs the wheel into a new temporary environment, with user
site packages and `PYTHONPATH` disabled. Query execution runs with only that
environment's executable directory on `PATH`, and a Python audit hook rejects
process launches during the query phase.

Four programs combine two structural variants with bound and literal value
paths. Each runs fourteen query shapes, giving 56 exact SQL and parameter
comparisons:

| Query shape | Database checks |
| --- | --- |
| INSERT application accounts | Three affected rows; runtime qualified table |
| INSERT application events | Three affected rows; changing quoted/Unicode values |
| SELECT with a left join and nested predicates | Expected records; absent joined fields keep typed NULLs |
| UPDATE with RETURNING | Exact updated record and visits count |
| DELETE | One affected row, then zero on repeating the same builder |
| SELECT with no matching row | `optional` returns None |
| MERGE of events with accounts, every kind of arm, RETURNING `merge_action()` and both row versions | Four rows: a deleted unmatched event, two inserts and a conditional update, each with its old and new points |
| SELECT final state | Expected surviving accounts and updated values |
| COPY FROM csv, with a column list, `HEADER` and `DELIMITER` | Two rows; the injection-shaped term holds the delimiter and Python's `csv` quotes it |
| COPY FROM text, with `NULL` | One row whose account is NULL |
| COPY TO csv over a SELECT, with `HEADER` and `FORCE_QUOTE` | The exact CSV of the three copied rows |
| COPY TO binary over a DELETE's RETURNING | The three copied rows, deleted |
| COPY FROM binary of those bytes | The three rows written back |
| COPY TO text over the events table, with `NULL` | Every event, backslashes escaped and the NULL account spelled `-` |

Structural variants change predicates and a limit inside the same loaded
extension. Values include quote-, semicolon- and comment-like text, and every
query under test uses public builders; RawSQL is confined to fixture DDL and
resetting state. Literal values stay SQL literals, while the typed `limit(2)`
still binds the u64 parameter Rust's limit API binds. COPY binds nothing, so a
bound value in a COPY's SELECT or DELETE is written inline, and the two value
paths render the same COPY.

`tests/direct_builders.rs` independently constructs equivalent `pgorm_query`
statements from the run's inputs and compares their SQL and tagged parameters
with the Python report; only the stable value snapshot encoder is shared. The
runner then changes one SQL string and one parameter list in copies of the
report and requires the verifier to fail for both. `target/python-direct-builders`
keeps the installed wheel, `queries.json` (every input, SQL, tagged parameters
and database outcome, without credentials) and `summary.json` (status, the 56
comparisons, the negative checks and the wheel's and report's hashes).

## Acceptance

`acceptance.py` is the complete check. Run it from the repository root with
CPython 3.14, Rust, a C compiler, libclang, OpenSSL, `uv` and the
repository's `nplan`, through either wrapper. Standalone builds use their own
Cargo target directory because application extensions have a different module
entry point, and every package is installed in a fresh environment.
`target/python-acceptance/summary.json` reports success only once every phase
has passed, with hashes of each phase's report; the directory keeps the
artifacts, recorded query results and each phase's log.

| API or contract | Evidence |
| --- | --- |
| No HTTP; direct SELECT/CRUD | `direct_builders.py`, above. |
| Values and expressions | Rust tests compare the Python wrappers with Rust value variants and expression constructors. Installed `test_values.py` and `test_expressions.py` cover exact conversion, typed NULL, enum identity, precision rejection and owned builder state. |
| Statements and raw SQL | Native statement tests compare SQL and parameter order for SELECT, INSERT, UPDATE, DELETE, conflicts and raw templates. Installed statement and direct-builder tests exercise their construction and execution. |
| Runtime models and schema | Installed model tests cover mapped fields, defaults, CRUD, composite keys and decoding constraints; `test_models_parity.py` holds every model statement to the SQL pgorm's own entities, `SelectGraph`, `Cursor` and `Paginator` build for the same tables. Rust `tests/schema.rs` and `tests/schema_objects.rs` compare explicit DDL, and the application's `schema_parity.rs` compares generated statements with real entity traits. |
| Registered entities, graphs and cursors | The application wheel runs real entity queries, ActiveModel writes and hooks. Native parity tests compare typed query and graph SQL and ActiveValue states. Registered graph tests cover source arities 1–7, and `graph_oracle` compares eight cursor programs with independent Rust builders. |
| Pipelines and registered sources | `tests/pipeline.rs` compares Python pipeline SQL and parameters with independent Rust pipelines. Installed tests exercise stages, bound callbacks, scope rejection and source tuples of arity 1–6, absent joined models included. |
| Connection and transaction execution | `registered_execution.py` records runtime builders, compiled queries, bound raw SQL, entity reads and writes, graph reads and cursors, plain pipelines and typed source terminals through connection and transaction objects. `execution_oracle` repeats them in Rust and compares outcomes, including hooks, DDL, commit, rollback, a nested savepoint, read-only mode and connection streaming; a deliberately changed outcome must be rejected. |
| Results, runtime and lifecycle | Installed tests exercise pool and connection execution, result cardinality, type decoding, TLS verification and rejection, errors, checkout timeout, cancellation, event-loop ownership, transaction reservation and bounded streaming cleanup. Native lifecycle tests verify the Rust pool close and discard behaviour under the Python lifecycle policy. |
| The guide | `test_guide.py` runs every standalone example and `guide.py` every registered one; `typing.py` and `guide.py` type-check them. |
| Code generation and typing | `codegen.py` builds and installs generated bindings, checks deterministic output, runs registered entities and graphs and checks their stubs. The distribution check compares installed runtime signatures with the stubs, type-checks the package and the example, and requires the invalid programs to fail. |
| Distribution and optional Rust integration | `distribution.py` builds and tests both installations. The acceptance runner checks that the default Rust dependency graph excludes PyO3, then builds the workspace with Python tooling replaced by failing probes that record which package's build ran them. A workspace package's record fails the check; a dependency's is a probe the build survived without Python, such as libc's build script running `emcc -dumpversion` where emcc is Homebrew's Python program, and is listed in the summary. |
| Specification tracking | `nplan spec validate` checks the references under the binding's Rust code, Python sources and tests. |

The execution comparison uses PostgreSQL's actual results, produced separately
by Rust and Python; the SQL and parameter comparisons cover construction
choices that could produce the same rows, and the negative comparisons make
sure an oracle cannot silently accept a mismatched report.

The complete check last passed locally on 2026-10-08 with CPython 3.14.4 and
the GIL, macOS 26.5.1 arm64 and PostgreSQL 18.6 (the Docker wrapper's
`postgres:18.6-bookworm`), under macOS's default temporary directory (reached
through the `/var` → `/private/var` symlink). The run included 34 native binding
tests, eight application Rust tests, 32 direct-builder query programs, the
registered application checks and 156 standalone Python tests on each of two
installations (the direct wheel and the source-derived wheel). Both passed
strict package and example typing, the installed signature checks and 13
expected invalid typing cases; the generated application passed its typing
and four expected invalid cases; the default Rust workspace build and nspec
validation passed. That record predates the guide phase. It establishes the
tested combination recorded in [support.json](../support.json); the macOS 15
and Linux CI jobs still need their own passing runs.
