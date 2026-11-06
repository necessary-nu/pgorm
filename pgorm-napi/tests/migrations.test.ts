// pgorm-migration's runner, run from JavaScript in either runtime: SQL files
// read as the program runs and migrations written in JavaScript, applied
// through Rust's `Migrations`.

import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, before, test } from "node:test";

import {
  alterType,
  ConstructionError,
  createSchema,
  createType,
  DatabaseError,
  LifecycleError,
  type MigrationTrait,
  migrationFile,
  migrations,
  Pool,
  SqlMigration,
  sqlMigration,
  type Transaction,
  TypeName,
} from "../lib/index.js";
import { scratchDatabase } from "./support.ts";

const SQL = new URL("../../pgorm-migration/tests/sql/", import.meta.url);
const NOTE = new URL("m20260101_000001_create_note.sql", SQL);
// The digest pgorm-migration's own tests hold NOTE's text to.
const NOTE_DIGEST = "9ae0994f5653f1500a920aee8881629d9aafeead971627b2f54bc0ace1b1bb17";

let pool: Pool;
let server: string;
let drop: () => Promise<void>;

before(async () => {
  const scratch = await scratchDatabase("pgorm_napi_migrations");
  server = scratch.dsn;
  pool = new Pool(server, { maxSize: 3 });
  drop = async () => {
    await pool.close();
    await scratch.drop();
  };
});

after(async () => {
  await drop();
});

let schemas = 0;
/** A schema of the test's own, and the one its ledger lives in. */
function scope(): { schema: string; ops: string } {
  schemas += 1;
  const schema = `migrations_${schemas}`;
  return { schema, ops: `${schema}_ops` };
}

class Written implements MigrationTrait {
  runs = 0;
  readonly name: string;
  readonly sql: string[];
  readonly checksum: string | null;

  constructor(name: string, sql: string[], checksum: string | null = null) {
    this.name = name;
    this.sql = sql;
    this.checksum = checksum;
  }

  async up(tx: Transaction): Promise<void> {
    this.runs += 1;
    for (const statement of this.sql) await tx.execute(statement);
  }
}

async function scalar(sql: string): Promise<unknown> {
  return (await pool.one(sql)).value;
}

// [spec:pgorm:req:bindings.migrations/test]    a file reads as pgorm-migration reads it: name, digest, directive
test("a SQL file reads as the runner reads it", () => {
  const note = SqlMigration.read(NOTE);
  assert.equal(note.name, "m20260101_000001_create_note");
  assert.equal(note.checksum, NOTE_DIGEST);
  assert.equal(note.commitsAfter, false);
  assert.match(note.sql, /CREATE TABLE note \(/);

  const frozen = SqlMigration.readDir(new URL("frozen/", SQL));
  assert.deepEqual(frozen.map((file) => file.name), [
    "m20240401_000001_create_fleet",
    "m20240401_000002_add_van_1",
    "m20240401_000002_add_van_2",
  ]);
  assert.deepEqual(frozen.map((file) => file.commitsAfter), [false, true, false]);
  assert.equal(sqlMigration(note.name, note.sql).checksum, NOTE_DIGEST);

  const directory = mkdtempSync(join(tmpdir(), "pgorm-napi-migrations-"));
  try {
    mkdirSync(join(directory, "nested.sql"));
    writeFileSync(join(directory, "m1.sql"), note.sql.replaceAll("\n", "\r\n"));
    writeFileSync(join(directory, "notes.txt"), "not a migration");
    const [crlf, ...rest] = SqlMigration.readDir(directory);
    assert.equal(rest.length, 0);
    assert.equal(crlf?.checksum, NOTE_DIGEST);
    assert.throws(() => SqlMigration.read(join(directory, "missing.sql")), (error) => !(error instanceof ConstructionError) && error instanceof Error);
  } finally {
    rmSync(directory, { recursive: true });
  }
  assert.throws(() => SqlMigration.read("/"), ConstructionError);
});

// [spec:pgorm:req:bindings.migrations/test]    a migration is a SqlMigration or an object with a name and an up
test("a migrator refuses what is no migration", () => {
  // @ts-expect-error: a number is no migration
  assert.throws(() => migrations([1]), TypeError);
  // @ts-expect-error: a migration's name is a string
  assert.throws(() => migrations([{ name: 1, async up() {} }]), TypeError);
  // @ts-expect-error: a migration has an up
  assert.throws(() => migrations([{ name: "m1" }]), TypeError);
  // @ts-expect-error: a checksum is a string
  assert.throws(() => migrations([{ name: "m1", async up() {}, checksum: 1 }]), TypeError);
  // @ts-expect-error: no such option
  assert.throws(() => migrations([], { tableName: "ledger" }), TypeError);
  // @ts-expect-error: the constructor is private
  assert.throws(() => new SqlMigration(), TypeError);
});

// [spec:pgorm:req:bindings.migrations/test]    files and JavaScript migrations run in order, once, through the runner
test("a frozen history and JavaScript migrations replay", async () => {
  const { schema, ops } = scope();
  const mood = new TypeName("mood", { schema });
  const directory = mkdtempSync(join(tmpdir(), "pgorm-napi-migrations-"));
  const frozen = [
    migrationFile("m20260101_000001_create_note", [
      createSchema(schema),
      createType(mood).asEnum().values(["calm"]),
    ]),
    migrationFile("m20260102_000001_add_glad", [alterType(mood).addValue("glad")], { commitsAfter: true }),
  ];
  let files: SqlMigration[];
  try {
    for (const file of frozen) assert.equal(file.freeze(directory), true);
    files = SqlMigration.readDir(directory);
  } finally {
    rmSync(directory, { recursive: true });
  }
  assert.deepEqual(files.map((file) => file.sql), frozen.map((file) => file.text));
  const seed = new Written(
    "m20260103_000001_create_feeling",
    [`CREATE TABLE "${schema}".feeling (mood "${schema}".mood DEFAULT 'glad')`, `INSERT INTO "${schema}".feeling DEFAULT VALUES`],
    "1",
  );
  const migrator = migrations([...files, seed], { migrationTableSchema: ops, migrationTableName: "ledger" });
  const pending = await migrator.getPendingMigrations(pool);
  assert.deepEqual(pending, [
    { name: "m20260101_000001_create_note", status: "pending" },
    { name: "m20260102_000001_add_glad", status: "pending" },
    { name: "m20260103_000001_create_feeling", status: "pending" },
  ]);

  await migrator.up(pool);
  await migrator.up(pool);
  assert.equal(seed.runs, 1);
  assert.deepEqual(
    await migrator.getAppliedMigrations(pool),
    pending.map(({ name }) => ({ name, status: "applied" })),
  );
  assert.deepEqual(await migrator.getPendingMigrations(pool), []);
  assert.equal(await scalar(`SELECT count(*) AS value FROM "${ops}".ledger`), 3n);
  assert.equal(await scalar(`SELECT checksum AS value FROM "${ops}".ledger WHERE version = '${seed.name}'`), "1");
  assert.equal(
    await scalar(`SELECT checksum AS value FROM "${ops}".ledger WHERE version = '${files[0]!.name}'`),
    files[0]!.checksum,
  );
  assert.equal(await scalar(`SELECT mood::text AS value FROM "${schema}".feeling`), "glad");

  const edited = new Written(seed.name, seed.sql, "2");
  await assert.rejects(
    migrations([...files, edited], { migrationTableSchema: ops, migrationTableName: "ledger" }).up(pool),
    (error) => error instanceof ConstructionError && /was applied with checksum '1'/.test(error.message),
  );
  assert.equal(edited.runs, 0);

  const logged: string[] = [];
  await migrator.status(pool, { logger: { info: (message) => logged.push(message) } });
  assert.deepEqual(logged, [
    "Checking migration status",
    "Migration 'm20260101_000001_create_note'... Applied",
    "Migration 'm20260102_000001_add_glad'... Applied",
    "Migration 'm20260103_000001_create_feeling'... Applied",
  ]);
});

// [spec:pgorm:req:bindings.migrations/test]    a migration's rejection rolls the batch back and is what up rejects with
test("a rejecting migration rolls the batch back", async () => {
  const { schema, ops } = scope();
  const refusal = new Error("no");
  const first = new Written("m20260101_000001_create", [`CREATE SCHEMA "${schema}"`]);
  const rejecting = {
    name: "m20260101_000002_reject",
    async up(): Promise<void> {
      throw refusal;
    },
  };
  const migrator = migrations([first, rejecting], { migrationTableSchema: ops });
  await assert.rejects(migrator.up(pool), (error) => error === refusal);
  assert.equal(await scalar(`SELECT count(*) AS value FROM pg_namespace WHERE nspname IN ('${schema}', '${ops}')`), 0n);

  const thrown = { name: "m20260101_000002_throw", up: () => { throw "plain"; } };
  await assert.rejects(migrations([first, thrown as unknown as MigrationTrait]).up(pool), (error) => error === "plain");

  await assert.rejects(migrations([first, new Written(first.name, [])]).up(pool), ConstructionError);
});

// [spec:pgorm:req:bindings.migrations/test]    up's transaction takes savepoints and statements, commitsAfter commits, and closes after
test("the lent transaction is a transaction", async () => {
  const { schema, ops } = scope();
  const held: Transaction[] = [];
  const tolerant: MigrationTrait = {
    name: "m20260101_000001_tolerant",
    commitsAfter: true,
    async up(tx) {
      held.push(tx);
      assert.equal(tx.parent, null);
      await assert.rejects(
        tx.transaction(async (savepoint) => {
          await savepoint.execute("CREATE EXTENSION pgorm_no_such_extension");
        }),
        DatabaseError,
      );
      await tx.execute(createSchema(schema));
      await tx.execute(createType(new TypeName("mood", { schema })).asEnum().values(["calm"]));
      await tx.execute(alterType(new TypeName("mood", { schema })).addValue("glad"));
    },
  };
  const uses = new Written("m20260101_000002_uses", [`CREATE TABLE "${schema}".feeling (mood "${schema}".mood DEFAULT 'glad')`]);
  await migrations([tolerant, uses], { migrationTableSchema: ops }).up(pool);
  assert.equal(
    await scalar(`SELECT count(*) AS value FROM pg_tables WHERE schemaname = '${schema}' AND tablename = 'feeling'`),
    1n,
  );
  assert.equal(held[0]!.closed, true);
  await assert.rejects(held[0]!.execute("SELECT 1"), LifecycleError);
});

// [spec:pgorm:req:bindings.migrations/test]    the runner clears the pool's statement caches after a batch
test("up clears the pool's statement caches", async () => {
  const { schema, ops } = scope();
  await using single = new Pool(server, { maxSize: 1 });
  await single.execute(createSchema(schema));
  await single.execute(`CREATE DOMAIN "${schema}".ribbon AS text DEFAULT 'red'`);
  await single.execute(`CREATE TABLE "${schema}".box (id serial, ribbon "${schema}".ribbon)`);
  const pack = `INSERT INTO "${schema}".box DEFAULT VALUES RETURNING ribbon`;
  assert.equal((await single.one(pack)).ribbon, "red");
  const gold = new Written("m20260101_000001_gold", [`ALTER DOMAIN "${schema}".ribbon SET DEFAULT 'gold'`]);
  await migrations([gold], { migrationTableSchema: ops, statementCacheChannel: null }).up(single);
  assert.equal((await single.one(pack)).ribbon, "gold");
});

test("the types refuse what the runner refuses", () => {
  const migrator = migrations([]);
  // @ts-expect-error: migrations run on a Pool
  void migrator.up(null).catch(() => {});
  // @ts-expect-error: steps is a number
  void migrator.up(pool, "1").catch(() => {});
  // @ts-expect-error: a directory is a path
  assert.throws(() => SqlMigration.readDir(1), TypeError);
});
