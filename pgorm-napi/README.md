# pgorm-napi

pgorm from Node.js and Deno. The npm package `@necessary/pgorm` is pgorm, the
PostgreSQL ORM, through a Node-API addon built with
[Neon](https://neon-bindings.com) and the ES module that loads it: pgorm's
pool, its query builder and pipeline, its models, transactions, `COPY` and
`LISTEN`, each operation a Promise. It needs a runtime with `Temporal` as a
global: Node.js 26 or later, or Deno 2.9.5 or later.

**[The guide](guide/README.md)** teaches the package from a first query to
every feature it has. It is the JavaScript dialect of pgorm's Rust guide,
chapter for chapter, and every example in it runs in both runtimes.

## Installing

Nothing is published to npm yet; until it is, build the addon from a checkout:

```sh
node pgorm-napi/scripts/build.mjs            # a debug build
node pgorm-napi/scripts/build.mjs --release  # the release build the packages ship
```

and import `pgorm-napi/lib/index.js`. Once published, `npm install
@necessary/pgorm` installs the package with the prebuilt addon for the
platform, and a Deno project imports `npm:@necessary/pgorm` and runs with
`--allow-ffi --allow-read`. [Getting started](guide/01-getting-started.md)
covers both runtimes and Deno's permissions, and
[DISTRIBUTION.md](DISTRIBUTION.md) the packages and their platforms.

## A first query

```js
import { connect } from "@necessary/pgorm";

await using pool = await connect("postgres://postgres@localhost/bakery?sslmode=disable");
const row = await pool.one("SELECT name, price FROM cake WHERE id = $1", [1]);
console.log(row.name, row.price.toString()); // Chocolate Fudge 4.50
```

Every value is bound as a parameter, never pasted into the SQL. A pool
verifies TLS unless the connection string says `sslmode=disable`.

## Tests

The package's suite, written against `node:test` and `node:assert`, runs
unchanged in both runtimes against a live server. It reads `PGORM_TEST_DSN`,
or else the server `DATABASE_URL` names (from the environment or the
checkout's `.env.local` or `.env`, as the Rust suite does):

```sh
cd pgorm-napi
node --test "tests/*.test.ts"
deno test --allow-ffi --allow-read --allow-env --allow-run tests/
deno check
```

- Each live file makes a database of its own, named for the runtime and
  process, and drops it when it ends.
- `tests/guide.test.ts` runs every example of the guide, each in a scratch
  copy of the bakery, from the modules it writes under
  `tests/guide-examples/`. They are committed so that `deno check`
  type-checks every example; after editing a chapter, run the file with
  `PGORM_GUIDE_WRITE=1` to write them again, and `PGORM_GUIDE_CHAPTERS`, a
  comma-separated list of chapter ids, runs only those chapters.
  `tests/guide-structure.test.ts` holds the guide's chapters to the Rust
  guide's.
- `tests/exit.test.ts` runs each of `tests/fixtures/` in a fresh process of
  the runtime under test, which is why Deno's suite needs `--allow-run`, and
  holds it to exiting by itself. `tests/runtime.test.ts` drives a panic on
  the runtime and Neon's drop queue through two exports only debug builds
  carry, skipped against a release build.
- `tests/statements-parity.test.ts` and `tests/models-parity.test.ts` hold
  each statement family's cases under `tests/parity/` to the golden files
  beside them, which the addon's Rust unit tests build the same statements
  to with pgorm-query and pgorm directly. `tests/model-types.test.ts` is held
  by `deno check`: it type-checks only while the declarations type a model's
  records, writes, keys, graphs and cursors from its columns.
- `deno check` also fails an import that would fail to link: `deno.json`'s
  `checkJs` type-checks every JavaScript module's imports, and
  `tests/declarations.ts`, which neither runner collects, holds the values
  `lib/index.d.ts` declares to the ones `lib/pgorm.js` exports. Deno resolves
  none of `lib/index.js`'s own imports, since its types are the declarations,
  so that module is only `export * from "./pgorm.js"`.
- The suite connects in plaintext unless its connection string names an
  `sslmode`; with `PGORM_TEST_CA` naming a PEM CA whose certificate for
  `localhost` the server presents, it also holds each TLS mode against that
  server, as CI does, and with `PGORM_TEST_CLIENT_CERT`,
  `PGORM_TEST_CLIENT_KEY` and `PGORM_TEST_CERT_USER` a client certificate and
  the role the server lets in by it alone. `pgorm-python/tests/with_postgres.py`
  runs a command against a disposable server set up so, exporting all four. `deno.json` keeps Deno on its global npm cache, so type checking
  `node:` imports needs no `node_modules`.
- `tests/package/` is the installed packages' smoke suite, which runs only in
  the projects `checks/package.js` installs the packed packages into
  ([DISTRIBUTION.md](DISTRIBUTION.md#build-and-verify)); `deno test` leaves
  it out here, while `deno check` holds it to the declarations through the
  package's own name.
- `node pgorm-napi/checks/entities.js` builds the repository's application
  fixture, which registers Rust entities, and runs its suite and the guide's
  registered-entity examples against it in both runtimes; `node
  pgorm-napi/checks/codegen.js` runs the registration generator over its
  fixture ([Generating registrations](guide/25-generating-registrations.md)).

The rules the package follows are [docs/spec/napi.md](../docs/spec/napi.md)
and, with pgorm-python, [docs/spec/bindings.md](../docs/spec/bindings.md).
