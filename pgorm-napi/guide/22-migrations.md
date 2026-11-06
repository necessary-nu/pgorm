# Migrations

A database's schema changes as the program does. A *migration* is one such
change, written once and applied to every database the program runs against:
a developer's, a test server's, production's. The migration runner,
pgorm-migration's, records each migration it applies in a table of its own,
the *ledger*, so that each database receives each migration exactly once, in
the order the program lists them. A JavaScript program runs its migrations
with that runner: `migrations(list)` builds pgorm-migration's migrator at
run time, and the ledger, the lock, the checksums and everything else this
chapter describes are the Rust runner's own. This chapter is the JavaScript
dialect of the [Rust guide's chapter](../../docs/guide/21-migrations.md) of
the same name.

Migrations run forward only. A migration has an `up` and nothing else: there
is no `down`, and no call that rolls back, refreshes or resets a database. A
mistake is corrected by the next migration.

## Adding the crate

In Rust, pgorm-migration is a crate of its own. The package carries it:
`migrations`, `SqlMigration` and `sqlMigration` are exported beside the rest,
and there is nothing more to install.

## Writing a migration

A migration written in JavaScript is an object with a `name` and an
`up(tx)` returning a promise. The runner awaits it with the transaction the
migration runs in, an ordinary `Transaction`: statements, the query
terminals, a savepoint ([Transactions](08-transactions.md)) and a COPY
([COPY](17-copy.md)) all run on it. Only the runner commits or rolls it back,
and once `up` settles the handle is closed. Its `parent` is `null`, since no
connection of the program's opened it.

A *migrator* lists every migration the program owns, in the order they
apply. `migrations(list)` builds one, and its `up(pool)` applies the
migrations the database has not had yet:

```ts
import assert from "node:assert/strict";
import { ColumnDef, createTable, type MigrationTrait, migrations, Table } from "@necessary/pgorm";

const createDelivery: MigrationTrait = {
  name: "m20260201_000001_create_delivery",
  async up(tx) {
    await tx.execute(
      createTable(new Table("delivery"))
        .column(new ColumnDef("id", "integer").autoIncrement().notNull())
        .column(new ColumnDef("order_id", "integer").notNull())
        .column(new ColumnDef("address", "text").notNull())
        .primaryKey("id"),
    );
  },
};
const migrator = migrations([createDelivery]);

await migrator.up(pool);
await pool.execute("INSERT INTO delivery (order_id, address) VALUES (1, 'Storgata 1, Oslo')");

// Running it again applies nothing: the ledger says the table exists.
await migrator.up(pool);
const applied = await migrator.getAppliedMigrations(pool);
assert.deepEqual(applied.map((migration) => migration.name), ["m20260201_000001_create_delivery"]);
```

`up` takes the pool, and checks connections out of it as the run needs
them. Its second argument bounds the run: `null`, the default, applies
everything pending, and a number `n` at most `n` migrations.

## Naming migrations

A migration's name is its identity in the ledger, so it must be unique and
must never change. Name each one for when it was written and what it does,
such as `m20260201_000001_create_delivery`; a migration written as SQL takes
the name of its file. The runner refuses a list that names one migration
twice, before it touches the database, with a `ConstructionError`. Renaming
a migration renames it: the old name is still in the ledger with no
migration to match, and the runner reports it as missing rather than
applying the new name a second time.

```ts
import assert from "node:assert/strict";
import { ConstructionError, type MigrationTrait, migrations } from "@necessary/pgorm";

const noop = (name: string): MigrationTrait => ({ name, async up() {} });

await assert.rejects(
  migrations([noop("m20260201_000001_noop"), noop("m20260201_000001_noop")]).up(pool),
  (error) => error instanceof ConstructionError && /Duplicate migration name\(s\): 'm20260201_000001_noop'/.test(error.message),
);

await migrations([noop("m20260201_000001_noop")]).up(pool);
await assert.rejects(
  migrations([noop("m20260201_000001_renamed")]).getMigrationWithStatus(pool),
  (error) => error instanceof ConstructionError && /'m20260201_000001_noop' is missing/.test(error.message),
);
```

## Migrations written as SQL

A migration can be a `.sql` file instead. `SqlMigration.read(path)` reads
one as Rust's `sql_migration!` does: named for the file's stem, its text run
whole in the batch's transaction, and its checksum the SHA-256 digest of
that text, so an edit made after it ran is caught, as the next sections
show. `sqlMigration(name, sql)` makes one from text the program holds. The
file is read when the program runs, so its grammar is checked by the server
as the migration runs, where `sql_migration!` checks it as the crate
compiles. The guide keeps its frozen migrations in
[`guide/migrations/`](migrations):

<!-- example: empty -->
```ts
import assert from "node:assert/strict";
import { migrations, SqlMigration, sqlMigration } from "@necessary/pgorm";

const bakery = SqlMigration.read("guide/migrations/m20260101_000001_create_bakery.sql");
assert.equal(bakery.name, "m20260101_000001_create_bakery");
assert.match(bakery.sql, /^CREATE TABLE "bakery" \(/);
assert.equal(bakery.checksum.length, 64);

const index = sqlMigration(
  "m20260102_000001_index_bakery_city",
  "CREATE INDEX bakery_city_index ON bakery (city);",
);
await migrations([bakery, index]).up(pool);
const { found } = await pool.one(
  "SELECT count(*) AS found FROM pg_indexes WHERE indexname = 'bakery_city_index'",
);
assert.equal(found, 1n);
```

## One transaction, one runner at a time

`up` runs every pending migration in one transaction, with each migration's
ledger row, and commits once at the end. If any migration fails, the whole
run rolls back: the schema and the ledger are as they were before it
started. A JavaScript migration's rejection is what `up` rejects with.

Before it reads the ledger, the run takes a PostgreSQL advisory lock keyed
by the ledger's name. Two deploys that start together therefore queue
rather than collide: the second waits until the first commits, then finds
nothing left to apply.

```ts
import assert from "node:assert/strict";
import { DatabaseError, type MigrationTrait, migrations } from "@necessary/pgorm";

const createNote: MigrationTrait = {
  name: "m20260201_000001_create_customer_note",
  async up(tx) {
    await tx.execute("CREATE TABLE customer_note (id serial PRIMARY KEY, note text NOT NULL)");
  },
};
// A migration that cannot succeed: the bakery has a customer table.
const createCustomerAgain: MigrationTrait = {
  name: "m20260201_000002_create_customer_again",
  async up(tx) {
    await tx.execute("CREATE TABLE customer (id serial PRIMARY KEY)");
  },
};

// The second migration fails, so the first is rolled back with it.
await assert.rejects(
  migrations([createNote, createCustomerAgain]).up(pool),
  (error) => error instanceof DatabaseError && error.sqlstate === "42P07",
);
const { tables } = await pool.one("SELECT count(*) AS tables FROM pg_tables WHERE tablename = 'customer_note'");
assert.equal(tables, 0n);

// Two runners at once: one applies the migration, the other waits and finds
// nothing to do.
const migrator = migrations([createNote]);
await Promise.all([migrator.up(pool), migrator.up(pool)]);
assert.equal((await migrator.getAppliedMigrations(pool)).length, 1);
```

### Committing partway

Some changes PostgreSQL will not let one transaction finish. A value that
`ALTER TYPE .. ADD VALUE` adds to an enum cannot be used until the
transaction that added it has committed. A migration making such a change
answers `true` from `commitsAfter`, a property beside its `name`. `up` then
commits the run so far once that migration and its ledger row are written,
and runs the rest in a new transaction, which takes the lock again. A SQL
file answers `true` when its first line is `-- pgorm: commits-after`.

Such a run is no longer all or nothing. A failure after the commit rolls
back only the transaction it happens in; the migrations committed before it
stay applied, and the next `up` resumes after them.

```ts
import assert from "node:assert/strict";
import { alterType, type MigrationTrait, migrations } from "@necessary/pgorm";

const addAlmond: MigrationTrait = {
  name: "m20260401_000001_add_almond",
  commitsAfter: true,
  async up(tx) {
    await tx.execute(alterType(flavor).addValue("almond"));
  },
};
const addAlmondCake: MigrationTrait = {
  name: "m20260401_000002_add_almond_cake",
  async up(tx) {
    await tx.execute(
      "INSERT INTO cake (name, flavor, price, gluten_free, bakery_id) VALUES ('Almond Cake', 'almond', 3.60, true, 2)",
    );
  },
};

await migrations([addAlmond, addAlmondCake]).up(pool);
const { added } = await pool.one("SELECT flavor::text AS added FROM cake WHERE name = 'Almond Cake'");
assert.equal(added, "almond");
```

[Disjoint models](15-disjoint.md) produce changes of this kind: a change to
the kinds is a list of steps, one migration each, every step but the last
answering `true`. Frozen into migration files, each such step opens with the
line `-- pgorm: commits-after` ([Generated DDL in a
migration](#generated-ddl-in-a-migration)).

## The ledger

The ledger is a table named `pgorm_migrations`. It has one row per applied
migration: its `version`, the migration's name; `applied_at`, the Unix time
in seconds it was applied; and `checksum`, what the migration reported as
its checksum, or `NULL`. `up` creates the table the first time it runs.
Nothing in pgorm-migration deletes a row.

By default the ledger sits wherever `search_path` puts an unqualified name,
usually beside the application's tables in `public`. A migrator can place it
in a schema of its own with `migrationTableSchema`, which keeps it apart
from triggers, policies and grants that apply to every application table,
and rename it with `migrationTableName`:

```ts
import assert from "node:assert/strict";
import { migrations } from "@necessary/pgorm";

const migrator = migrations(
  [
    {
      name: "m20260201_000001_create_delivery",
      async up(tx) {
        await tx.execute("CREATE TABLE delivery (id serial PRIMARY KEY, address text NOT NULL)");
      },
    },
  ],
  { migrationTableSchema: "ops", migrationTableName: "bakery_migrations" },
);

await migrator.up(pool);
const row = await pool.one("SELECT version, checksum FROM ops.bakery_migrations");
assert.equal(row.version, "m20260201_000001_create_delivery");
assert.equal(row.checksum, null);
```

`up` creates the schema if it is missing, and names the table in it
everywhere, so a table of the same name elsewhere on `search_path` is never
taken for it. The runner does not move a ledger that already exists: a
ledger left in its old place reads as absent from the new one, and every
migration would run again. Move it with `ALTER TABLE .. SET SCHEMA` before
deploying a release that names a new place.

### A ledger from SeaORM

A database last migrated by SeaORM keeps its ledger under the name
`seaql_migrations`, with the same `version` and `applied_at` columns. The
first `up` renames that table to `pgorm_migrations` in place, so nothing
runs twice. This applies to the default name with no schema; a migrator that
names or places its own ledger takes the old table over by hand.

```ts
import assert from "node:assert/strict";
import { migrations } from "@necessary/pgorm";

// What SeaORM left behind: the table, and the ledger row saying it exists.
await pool.transaction(async (tx) => {
  await tx.execute("CREATE TABLE delivery (id serial PRIMARY KEY)");
  await tx.execute("CREATE TABLE seaql_migrations (version varchar PRIMARY KEY, applied_at bigint NOT NULL)");
  await tx.execute("INSERT INTO seaql_migrations VALUES ('m20220101_000001_create_delivery', 1640995200)");
});

// The migration is not run again, which would fail: the table exists.
await migrations([
  {
    name: "m20220101_000001_create_delivery",
    async up(tx) {
      await tx.execute("CREATE TABLE delivery (id serial PRIMARY KEY)");
    },
  },
]).up(pool);
const { applied } = await pool.one("SELECT applied_at AS applied FROM pgorm_migrations");
assert.equal(applied, 1640995200n);
```

## Asking what has run

`status` logs each migration's name and whether it is `Pending` or
`Applied` through `logger.info`, `console` unless a logger is given.
`getMigrationWithStatus`, `getPendingMigrations` and `getAppliedMigrations`
answer the same question as values: each a list of `{ name, status }`, its
status `"pending"` or `"applied"`, in the migrator's order. Each takes a
pool, a connection or a transaction and only reads: against a database never
migrated, every migration is pending and no ledger is created to say so. A
readiness probe or a CI step can ask with a role that has no right to change
the schema.

```ts
import assert from "node:assert/strict";
import { type MigrationTrait, migrations } from "@necessary/pgorm";

const create = (name: string, table: string): MigrationTrait => ({
  name,
  async up(tx) {
    await tx.execute(`CREATE TABLE ${table} (id serial PRIMARY KEY)`);
  },
});
const migrator = migrations([
  create("m20260201_000001_create_delivery", "delivery"),
  create("m20260201_000002_create_voucher", "voucher"),
]);

assert.equal((await migrator.getPendingMigrations(pool)).length, 2);

// Apply one migration of the two.
await migrator.up(pool, 1);

assert.deepEqual(await migrator.getMigrationWithStatus(pool), [
  { name: "m20260201_000001_create_delivery", status: "applied" },
  { name: "m20260201_000002_create_voucher", status: "pending" },
]);

const logged: string[] = [];
await migrator.status(pool, { logger: { info: (line) => logged.push(line) } });
assert.equal(logged.at(-1), "Migration 'm20260201_000002_create_voucher'... Pending");
```

## Catching an edited migration

The ledger matches migrations by name, so a migration edited after it ran
would go unnoticed: its name still matches, and a database built afresh
would quietly differ from the ones built before. A migration that reports a
`checksum` closes that gap. `up` stores the checksum in the migration's
ledger row, and every later `status` and `up` compares it with what the
migration reports now, failing with a `ConstructionError` that names the
migration when the two differ.

The checksum is any text the author controls, such as a hash of the
migration's SQL or a version bumped by hand, given as a `checksum`
property. It is off by default: a migration that reports none is never
checked, nor is a row written before it reported one. A `SqlMigration`
always reports one.

```ts
import assert from "node:assert/strict";
import { ConstructionError, type MigrationTrait, migrations } from "@necessary/pgorm";

const createDelivery = (revision: string): MigrationTrait => ({
  name: "m20260201_000001_create_delivery",
  checksum: revision,
  async up(tx) {
    await tx.execute("CREATE TABLE delivery (id serial PRIMARY KEY)");
  },
});

await migrations([createDelivery("1")]).up(pool);

// The same migration, edited after it ran.
await assert.rejects(
  migrations([createDelivery("2")]).up(pool),
  (error) =>
    error instanceof ConstructionError &&
    error.message.includes("'m20260201_000001_create_delivery' was applied with checksum '1'"),
);
```

## Generated DDL in a migration

A statement built from a declaration describes the table as the declaration
has it now, while a migration describes a change made once. A migration that
built its DDL as it ran would follow the declaration as it changed: once the
declaration gained a column, a database built from nothing would get the
column from the first migration and then again from the migration that added
it, and fail.

So build the DDL once, when the change is made, and freeze it into a
migration written as SQL. [`migrationFile(name, statements, { commitsAfter
})`](../lib/disjoint.d.ts) holds DDL statements as a `.sql` file's text, each
ending `;` on a line of its own, and `freeze(directory)` writes `<name>.sql`
into the migrations directory if no such file is there, answering whether it
wrote one. A file already there is never rewritten: it is the migration
databases have run, its checksum in their ledgers. So the code that freezes
each change can live in a test or a small script and run again whenever a
change is added, writing the new file and keeping the old ones, however far
the declarations have moved on. `SqlMigration.readDir` reads the files back
as migrations ([A migrator built at run time](#a-migrator-built-at-run-time)),
and Rust's pgorm-migration lists the same files with `sql_migration!`:

```ts
import assert from "node:assert/strict";
import { ColumnDef, createTable, migrationFile, Table } from "@necessary/pgorm";

const create = createTable(new Table("bakery"))
  .column(new ColumnDef("id", "integer").autoIncrement().notNull())
  .column(new ColumnDef("name", "varchar").notNull())
  .column(new ColumnDef("city", "varchar").notNull())
  .primaryKey("id");

// Frozen when the bakery table was added; freezing again writes nothing.
const file = migrationFile("m20260101_000001_create_bakery", [create]);
assert.equal(
  file.text,
  `CREATE TABLE "bakery" ( "id" serial NOT NULL, "name" varchar NOT NULL, "city" varchar NOT NULL, PRIMARY KEY ("id") );\n`,
);
assert.equal(file.freeze("guide/migrations"), false);
```

The file is the same text Rust's `MigrationFile` freezes from the bakery's
entity, so either language's frozen history runs under either runner.

A change PostgreSQL will not let one transaction finish is frozen into
several files, a [disjoint model](15-disjoint.md)'s new kind into one per
step. `migrationFile(.., { commitsAfter: true })` opens the file with the line
`-- pgorm: commits-after`, which the runner reads as the migration's
`commitsAfter`, so its batch commits once the file has run, and the file's
`commitsAfter` says so:

```ts
import assert from "node:assert/strict";
import { alterType, migrationFile } from "@necessary/pgorm";

const almond = migrationFile("m20260401_000001_add_almond", [alterType(flavor).addValue("almond")], {
  commitsAfter: true,
});
assert.equal(almond.commitsAfter, true);
assert.equal(almond.text, `-- pgorm: commits-after\nALTER TYPE "public"."flavor" ADD VALUE 'almond';\n`);
```

## A migrator built at run time

Every migrator in JavaScript is built as the program runs, so its list can
be read from disk. `SqlMigration.readDir(directory)` reads every `.sql` file
in a directory, in the order of their names, each as `read` reads one. So a
program replays the files it has frozen without naming each, with its own
JavaScript migrations listed among them where they belong. Here the guide's
frozen history, the bakery and the payments with their voucher kind, is
replayed on an empty database, then a migration written in JavaScript pays
by voucher:

<!-- example: empty -->
```ts
import assert from "node:assert/strict";
import { type MigrationTrait, migrations, SqlMigration } from "@necessary/pgorm";

const frozen = SqlMigration.readDir("guide/migrations");
assert.deepEqual(frozen.map((file) => file.name), [
  "m20260101_000001_create_bakery",
  "m20260201_000001_create_payment",
  "m20260301_000001_add_voucher_1",
  "m20260301_000001_add_voucher_2",
]);
assert.deepEqual(frozen.map((file) => file.commitsAfter), [false, false, true, false]);

const firstVoucher: MigrationTrait = {
  name: "m20260301_000002_first_voucher",
  async up(tx) {
    // The base row and its kind's row reference each other, so the keys are
    // checked once both are written.
    await tx.execute("SET CONSTRAINTS ALL DEFERRED");
    await tx.execute("INSERT INTO payment (id, kind, order_id, amount) VALUES (1, 'voucher', 1, 9.50)");
    await tx.execute("INSERT INTO voucher_payment (payment_id, code) VALUES (1, 'SPRING')");
  },
};
const migrator = migrations([...frozen, firstVoucher]);

await migrator.up(pool);
assert.equal((await migrator.getAppliedMigrations(pool)).length, 5);
const { checksum } = await pool.one(
  "SELECT checksum FROM pgorm_migrations WHERE version = 'm20260201_000001_create_payment'",
);
assert.equal(checksum, frozen[1]!.checksum);
const { code } = await pool.one("SELECT code FROM voucher_payment JOIN payment ON payment.voucher_id = payment_id");
assert.equal(code, "SPRING");
```

## Statement caches

pgorm prepares each statement once per pooled connection and reuses it. Most
schema changes make PostgreSQL plan the statement again, but some do not: an
insert prepared before a domain's default changed keeps writing the old
default ([Statement caches and DDL](19-statement-caches.md)). So once a run
that applied a migration commits, `up` clears the statement caches of the
pool it was given:

<!-- example: empty -->
```ts
import assert from "node:assert/strict";
import { migrations, Pool } from "@necessary/pgorm";

// One connection, so every statement runs on the same session.
await using shop = new Pool(url, { maxSize: 1 });
await shop.execute("CREATE DOMAIN ribbon AS text DEFAULT 'red'");
await shop.execute("CREATE TABLE box (id serial PRIMARY KEY, contents text, ribbon ribbon)");
const pack = "INSERT INTO box (contents) VALUES ('truffles') RETURNING ribbon";
assert.equal((await shop.one(pack)).ribbon, "red");

// The insert prepared before the change would still write the old default,
// but the run clears the pool's caches as it commits.
await migrations([
  {
    name: "m20260201_000001_gold_ribbon",
    async up(tx) {
      await tx.execute("ALTER DOMAIN ribbon SET DEFAULT 'gold'");
    },
  },
]).up(shop);
assert.equal((await shop.one(pack)).ribbon, "gold");
```

`up` also sends a notification on `statementCacheChannel`,
`STATEMENT_CACHE_CHANNEL` (`pgorm_statement_caches`) unless the migrator is
given another, from inside the run's transaction, so it is delivered exactly
when the run commits. A pool built with `statementCacheChannel:
STATEMENT_CACHE_CHANNEL` listens there and clears its caches on every
notification, so another process's pool, a Rust program's or a JavaScript
one's, hears this one migrate the database:

```ts
import assert from "node:assert/strict";
import { migrations, STATEMENT_CACHE_CHANNEL } from "@necessary/pgorm";

await using listener = await pool.listener();
await listener.listen(STATEMENT_CACHE_CHANNEL);

await migrations([
  {
    name: "m20260201_000001_create_ribbon",
    async up(tx) {
      await tx.execute("CREATE DOMAIN ribbon AS text DEFAULT 'red'");
    },
  },
]).up(pool);

// Every pool listening on the channel, in any process, hears it at the commit.
const heard = await listener.next({ signal: AbortSignal.timeout(5000) });
assert.ok(!heard.done);
assert.equal(heard.value.channel, "pgorm_statement_caches");
```

A migrator built with `statementCacheChannel: null` sends nothing, and a
pool built without the option does not listen.
[Statement caches and DDL](19-statement-caches.md) covers both in full.

## A migration crate

There is no command-line tool. A JavaScript program runs its migrations
itself: as a step of each deploy, a small script that builds the migrator
and calls `up`, or at startup, before it serves. The advisory lock makes that
safe when several replicas start at once.

A program whose migrations are written in Rust keeps them in a crate of
their own instead, with a small binary that runs them. A starter crate lives
in pgorm's repository at `pgorm-migration/template/migration/`; its binary
reads `DATABASE_URL` and runs `up`, `up <n>` or `status`:

```sh
DATABASE_URL=postgres://localhost/bakery cargo run -p migration -- up
DATABASE_URL=postgres://localhost/bakery cargo run -p migration -- status
```

Both runners keep the same ledger, so a database can be migrated by either,
and a migration a JavaScript program froze into a file runs in the Rust crate
through `sql_migration!` with the checksum it has here.
