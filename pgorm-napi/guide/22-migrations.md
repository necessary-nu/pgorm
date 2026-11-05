# Migrations

A database's schema changes as the program does. A *migration* is one such
change, written once and applied to every database the program runs against:
a developer's, a test server's, production's. In Rust, `pgorm-migration` runs
them, and records each one it applies in a table of its own, the *ledger*, so
that each database receives each migration exactly once, in the order the
program lists them. This chapter is the JavaScript dialect of the [Rust
guide's chapter](../../docs/guide/21-migrations.md) of the same name.

**Not available in JavaScript yet:** a migration runner. Rust's
`pgorm-migration` has a migration type, `MigrationTrait`, whose `up` changes
the schema inside the transaction it is handed; a migrator, `MigratorTrait`,
listing every migration in order; `up`, which applies the pending ones in one
transaction under an advisory lock, so two deploys queue rather than collide,
committing partway after a migration that answers `commits_after`;
migrations written as SQL files and checked against PostgreSQL's grammar
when the crate compiles (`sql_migration!`); a ledger, `pgorm_migrations`,
that a migrator can rename or place in a schema of its own, and that takes
over SeaORM's `seaql_migrations`; questions about what has run
(`status`, `get_pending_migrations`, `get_applied_migrations`); checksums
that catch a migration edited after it ran; and, once a batch commits, a
clear of the statement caches of its own pool and a notification clearing
every other pool's. JavaScript has none of these.

What JavaScript has is the rest of what a migration is made of: DDL builders,
transactions that make a change all or nothing, migration files frozen from
generated DDL, which a runner of SQL files runs, and pools that clear their
statement caches when a migration elsewhere tells them to. The sections below
show each, and how a JavaScript program sits beside a Rust runner.

## Running the Rust runner beside a JavaScript program

A program's migrations can be a Rust crate of their own, whichever language
the program is written in. A starter crate lives in pgorm's repository at
`pgorm-migration/template/migration/`; its binary reads `DATABASE_URL` and
runs `up`, `up <n>` or `status`, and a deploy runs it before it starts the
JavaScript program:

```sh
DATABASE_URL=postgres://localhost/bakery cargo run -p migration -- up
DATABASE_URL=postgres://localhost/bakery cargo run -p migration -- status
```

The [Rust guide's chapter](../../docs/guide/21-migrations.md) writes such a
crate. Two things then connect it to the JavaScript side: the ledger it
writes, which JavaScript can read, and the notification it sends, which
JavaScript's pools can hear.

## Changing the schema in one transaction

PostgreSQL's DDL is transactional: a `CREATE TABLE`, an `ALTER TABLE`, a
`CREATE INDEX` run inside a transaction take effect at its commit or not at
all. That is what makes a migration all or nothing, and a JavaScript
transaction gives it the same way. Here the second statement cannot succeed,
since the table already exists, so the first is rolled back with it:

```ts
import assert from "node:assert/strict";
import { ColumnDef, createIndex, createTable, Table } from "@necessary/pgorm";

const delivery = new Table("delivery");
const create = createTable(delivery)
  .column(new ColumnDef("id", "integer").identity("always"))
  .column(new ColumnDef("order_id", "integer").notNull())
  .primaryKey("id");

await assert.rejects(
  pool.transaction(async (tx) => {
    await tx.execute(createIndex("cake", { on: "name" }, { name: "cake_name_index" }));
    await tx.execute(create);
    await tx.execute(create);
  }),
  /already exists/,
);

const { indexes } = await pool.one(
  "SELECT count(*) AS indexes FROM pg_indexes WHERE indexname = 'cake_name_index'",
);
assert.equal(indexes, 0n);
```

Some changes PostgreSQL will not let one transaction finish. A value that
`ALTER TYPE .. ADD VALUE` adds to an enum cannot be used until the
transaction that added it has committed, which is why Rust's runner commits
partway after a migration that answers `commits_after`. In JavaScript the
same change is two transactions, the second run once the first commits:

```ts
import assert from "node:assert/strict";
import { alterType } from "@necessary/pgorm";

await pool.transaction(async (tx) => {
  await tx.execute(alterType(flavor).addValue("almond"));
});
await pool.transaction(async (tx) => {
  await tx.execute(
    "INSERT INTO cake (name, flavor, price, gluten_free, bakery_id) VALUES ('Almond Cake', 'almond', 3.60, true, 2)",
  );
});

const { flavor: added } = await pool.one("SELECT flavor::text AS flavor FROM cake WHERE name = 'Almond Cake'");
assert.equal(added, "almond");
```

[Schema generation](14-schema.md) covers the DDL builders, and
[Disjoint enums](15-disjoint.md) a change to a disjoint model's kinds, whose
steps each run in a transaction of their own in the same way.

## The ledger

Rust's runner keeps its ledger in a table named `pgorm_migrations`, one row
per applied migration: its `version`, the migration's name; `applied_at`, the
Unix time in seconds it was applied; and `checksum`, what the migration
reported as its checksum, or `NULL`. Nothing in pgorm-migration deletes a
row. A JavaScript program can declare a model of it and read it like any
table, such as from a readiness probe that waits for the migration a release
needs. Reading changes nothing, so the probe needs no right to change the
schema:

```ts
import assert from "node:assert/strict";
import { column, model } from "@necessary/pgorm";

// What pgorm-migration's runner leaves behind, written here by hand.
await pool.execute(
  `CREATE TABLE "pgorm_migrations" ( "version" varchar NOT NULL, "applied_at" bigint NOT NULL, "checksum" varchar, PRIMARY KEY ("version") )`,
);
await pool.execute(
  `INSERT INTO "pgorm_migrations" VALUES ('m20260101_000001_create_bakery', 1767225600, NULL)`,
);

const Ledger = model("pgorm_migrations", {
  columns: {
    version: column("text", { primaryKey: true }),
    appliedAt: column("i64", { name: "applied_at" }),
    checksum: column("text", { nullable: true }),
  },
});

const needed = "m20260101_000001_create_bakery";
const applied = await Ledger.findByKey({ version: needed }).optional(pool);
assert.equal(applied?.appliedAt, 1767225600n);
assert.equal(applied?.checksum, null);
```

A migrator that renames its ledger or places it in a schema of its own names
that table instead, and the model says so with its name and `schema`.

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
the declarations have moved on. The files are what Rust's pgorm-migration
lists with `sql_migration!`, or what any runner of SQL files runs:

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
`-- pgorm: commits-after`, which Rust's runner reads as `commits_after()`, so
its batch commits once the file has run, and the file's `commitsAfter` says
so:

```ts
import assert from "node:assert/strict";
import { alterType, migrationFile } from "@necessary/pgorm";

const almond = migrationFile("m20260401_000001_add_almond", [alterType(flavor).addValue("almond")], {
  commitsAfter: true,
});
assert.equal(almond.commitsAfter, true);
assert.equal(almond.text, `-- pgorm: commits-after\nALTER TYPE "public"."flavor" ADD VALUE 'almond';\n`);
```

## Statement caches

pgorm prepares each statement once per pooled connection and reuses it. Most
schema changes make PostgreSQL plan the statement again, but some do not: an
insert prepared before a domain's default changed keeps writing the old
default. So once a run that applied a migration commits, Rust's runner clears
the statement caches of the pool it was given, and sends a notification on
`STATEMENT_CACHE_CHANNEL`, `pgorm_statement_caches`, from inside the run's
transaction, so it is delivered exactly when the run commits. A JavaScript
pool built with `statementCacheChannel: STATEMENT_CACHE_CHANNEL` hears it and
clears its own caches, so a JavaScript program needs nothing more when a Rust
runner migrates its database.

A JavaScript program that changes such things itself does both halves by
hand: it clears its own pool, and notifies the others from inside the
transaction that makes the change, so the notification is delivered when it
commits:

```ts
import assert from "node:assert/strict";
import { STATEMENT_CACHE_CHANNEL } from "@necessary/pgorm";

await using listener = await pool.listener();
await listener.listen(STATEMENT_CACHE_CHANNEL);

await pool.transaction(async (tx) => {
  await tx.execute("CREATE DOMAIN ribbon AS text DEFAULT 'red'");
  await tx.execute("ALTER DOMAIN ribbon SET DEFAULT 'gold'");
  await tx.notify(STATEMENT_CACHE_CHANNEL, "");
});
pool.clearStatementCaches();

// Every pool listening on the channel, in any process, hears it at the commit.
const heard = await listener.next({ signal: AbortSignal.timeout(5000) });
assert.ok(!heard.done);
assert.equal(heard.value.channel, "pgorm_statement_caches");
```

[Statement caches and DDL](19-statement-caches.md) covers both: what a
cleared cache does, and how a pool listens.
