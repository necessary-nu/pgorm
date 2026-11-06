// A registered entity's and graph's reads beyond all, one and optional:
// counts, pages, keyset cursors, streams, joins, projections and grouping,
// through the application's own native module.

import assert from "node:assert/strict";
import { after, before, beforeEach, test } from "node:test";

import { ConstructionError, entity, type EntityRecord, graph, Pool } from "../../../lib/index.js";
import { scratchDatabase } from "../../support.ts";

let database: { dsn: string; drop(): Promise<void> } | undefined;
let shared: Pool | undefined;

function pool(): Pool {
  if (!shared) throw new Error("the scratch database was not created");
  return shared;
}

const Account = entity("app.Account");
const Note = entity("app.Note");

before(async () => {
  database = await scratchDatabase("pgorm_napi_reads");
  shared = new Pool(database.dsn, { maxSize: 2 });
  for (
    const sql of [
      "CREATE SCHEMA napi_entities",
      "CREATE TYPE napi_entities.mood AS ENUM ('calm', 'busy')",
      `CREATE TABLE napi_entities.accounts (
         id int4 PRIMARY KEY, "display name" text NOT NULL, note text, version int4 NOT NULL,
         mood napi_entities.mood NOT NULL
       )`,
      "CREATE TABLE napi_entities.notes (id int4 PRIMARY KEY, account_id int4 NOT NULL, body text NOT NULL)",
    ]
  ) {
    await pool().execute(sql);
  }
});

beforeEach(async () => {
  await pool().execute("TRUNCATE napi_entities.accounts, napi_entities.notes");
  await pool().execute(
    `INSERT INTO napi_entities.accounts VALUES (1, 'Ann', NULL, 1, 'calm'), (2, 'Bob', NULL, 1, 'busy'),
       (3, 'Cy', NULL, 1, 'calm'), (4, 'Di', NULL, 1, 'busy'), (5, 'Ed', NULL, 1, 'calm')`,
  );
  await pool().execute("INSERT INTO napi_entities.notes VALUES (11, 1, 'first'), (12, 1, 'second'), (21, 2, 'third')");
});

after(async () => {
  await shared?.close();
  await database?.drop();
});

function refused(pattern: RegExp): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof ConstructionError, `expected a ConstructionError, got ${error}`);
    assert.match(error.message, pattern);
    return true;
  };
}

const calm = () => Account.find().where(Account.col("mood").eq("calm")).orderBy(Account.col("id").asc());

type Pair = [EntityRecord, EntityRecord | null];
const ids = (rows: unknown[]) => (rows as Pair[]).map(([root, note]) => [root.id, note?.id ?? null]);

// [spec:pgorm:req:bindings.entity-reads+4/test]
test("a registered query counts and pages as Rust's Paginator does", async () => {
  const query = calm();
  assert.equal(await query.count(pool()), 3);
  assert.match(query.inspect("count").sql, /^SELECT COUNT\(\*\) AS num_items FROM \(SELECT "accounts"\."id"/);
  const pages = query.paginate(2);
  assert.deepStrictEqual(await pages.numItemsAndPages(pool()), { items: 3, pages: 2 });
  assert.equal(await pages.numItems(pool()), 3);
  assert.equal(await pages.numPages(pool()), 2);
  assert.deepStrictEqual((await pages.fetchPage(pool(), 1)).map((account) => account.id), [5]);
  assert.deepStrictEqual(await pages.fetchPage(pool(), 2), []);
  assert.match(pages.inspect(1).sql, /ORDER BY "accounts"\."id" ASC LIMIT \$2 OFFSET \$3$/);
  const seen: unknown[][] = [];
  for await (const page of pages.pages(pool())) seen.push(page.map((account) => account.id));
  assert.deepStrictEqual(seen, [[1, 3], [5]]);
  assert.throws(() => query.paginate(0), refused(/page size is at least 1/));
});

// [spec:pgorm:req:bindings.entity-reads+4/test]
test("a registered query's cursor orders by its columns and casts each boundary", async () => {
  const cursor = Account.find().cursor("mood", "id");
  assert.deepStrictEqual((await cursor.first(2).all(pool())).map((account) => account.id), [1, 3]);
  const following = cursor.after("calm", 3).first(3);
  assert.deepStrictEqual((await following.all(pool())).map((account) => account.id), [5, 2, 4]);
  const { sql } = following.inspect();
  assert.match(sql, /\("accounts"\."mood" = CAST\(\$1::text AS napi_entities\.mood\) AND "accounts"\."id" > \$2\) OR/);
  assert.match(sql, /ORDER BY "accounts"\."mood" ASC, "accounts"\."id" ASC LIMIT \$4$/);
  assert.deepStrictEqual((await cursor.before("busy", 4).last(2).all(pool())).map((account) => account.id), [5, 2]);
  assert.deepStrictEqual((await cursor.desc().first(1).all(pool())).map((account) => account.id), [4]);
  assert.throws(() => cursor.after("calm"), refused(/does not match 2 key column/));
  assert.throws(() => Account.find().cursor(), refused(/at least one column/));
  assert.throws(() => Account.find().cursor("missing"), refused(/no column "missing"/));
});

// [spec:pgorm:req:bindings.entity-reads+4/test]
test("a registered query streams its records, each decoded as it arrives", async () => {
  const read: unknown[] = [];
  for await (const account of calm().stream(pool())) read.push(account.id);
  assert.deepStrictEqual(read, [1, 3, 5]);
  await pool().connection(async (connection) => {
    const names: unknown[] = [];
    for await (const account of calm().limit(1).stream(connection)) names.push(account["display name"]);
    assert.deepStrictEqual(names, ["Ann"]);
  });
});

// [spec:pgorm:req:bindings.entity-reads+4/test]
test("a registered query joins a relation its entity declares", async () => {
  const notes = Account.relation("Note");
  assert.deepStrictEqual([notes.entityName, notes.name], ["app.Account", "Note"]);
  const withSecond = Account.find().join(notes).where(Note.col("body").eq("second"));
  assert.match(withSecond.inspect().sql, /INNER JOIN "napi_entities"\."notes" ON "accounts"\."id" = "notes"\."account_id"/);
  assert.deepStrictEqual((await withSecond.all(pool())).map((account) => account.id), [1]);
  assert.match(Account.find().join(notes, { kind: "left" }).inspect().sql, /LEFT JOIN/);
  assert.throws(() => Account.relation("Missing"), refused(/declares no relation Missing/));
});

// [spec:pgorm:req:bindings.entity-reads+4/test]
test("a registered query's selection reads some of its columns as plain records", async () => {
  const names = calm().select("display name", "mood");
  assert.deepStrictEqual(names.columns, ["display name", "mood"]);
  assert.match(names.inspect().sql, /^SELECT "accounts"\."display name", CAST\("accounts"\."mood" AS text\) FROM/);
  assert.deepStrictEqual((await names.all(pool())).slice(0, 2), [
    { "display name": "Ann", mood: "calm" },
    { "display name": "Cy", mood: "calm" },
  ]);
  assert.equal((await names.one(pool()))["display name"], "Ann");
  assert.match(names.inspect("one").sql, /LIMIT \$2$/);
  assert.equal(await Account.find().where(Account.col("id").eq(99)).select("id").optional(pool()), null);
  const streamed: unknown[] = [];
  for await (const row of names.stream(pool())) streamed.push(row["display name"]);
  assert.deepStrictEqual(streamed, ["Ann", "Cy", "Ed"]);
  assert.throws(() => calm().select(), refused(/at least one column/));
  assert.throws(() => calm().select("missing"), refused(/no column "missing"/));
});

const notes = () => {
  const query = graph("app.AccountNotes").find({ aliases: ["n"] });
  return query.orderBy(query.col(0, "id").asc(), query.col(1, "id").asc());
};

// [spec:pgorm:req:bindings.entity-graphs+4/test]
test("a registered graph groups, counts and pages its rows", async () => {
  const grouped = await notes().allGrouped(pool());
  assert.deepStrictEqual(grouped.map(([root, children]) => [root.id, children.map((note) => note.id)]), [
    [1, [11, 12]],
    [2, [21]],
    [3, []],
    [4, []],
    [5, []],
  ]);
  await assert.rejects(graph("app.RequiredNotes").find().allGrouped(pool()), refused(/one Opt slot/));
  assert.equal(await notes().count(pool()), 6);
  const pages = notes().paginate(4);
  assert.deepStrictEqual(await pages.numItemsAndPages(pool()), { items: 6, pages: 2 });
  assert.deepStrictEqual(ids(await pages.fetchPage(pool(), 1)), [[4, null], [5, null]]);
});

// [spec:pgorm:req:bindings.entity-graphs+4/test]
test("a registered graph streams its rows, each decoded as it arrives", async () => {
  const read: unknown[] = [];
  for await (const row of notes().stream(pool())) read.push(row);
  assert.deepStrictEqual(ids(read), [[1, 11], [1, 12], [2, 21], [3, null], [4, null], [5, null]]);
});

// [spec:pgorm:req:bindings.entity-graphs+4/test]
test("a registered graph's cursor orders by a slot's columns or by several of the root's", async () => {
  const onBody = graph("app.AccountNotes").find({ aliases: ["n"] }).cursorOn(1, "body");
  assert.match(onBody.first(2).inspect().sql, /ORDER BY "n"\."body" ASC, "n"\."id" ASC, "accounts"\."id" ASC LIMIT \$1$/);
  const firstTwo = await onBody.first(2).all(pool()) as Pair[];
  assert.deepStrictEqual(firstTwo.map(([, note]) => note?.body), ["first", "second"]);
  const past = await onBody.afterWith("first", 11, 1).first(1).all(pool()) as Pair[];
  assert.deepStrictEqual(past.map(([, note]) => note?.body), ["second"]);
  const composite = graph("app.AccountNotes").find({ aliases: ["n"] }).cursor("mood", "id");
  assert.deepStrictEqual(ids(await composite.first(3).all(pool())), [[1, 11], [1, 12], [3, null]]);
  assert.deepStrictEqual(ids(await composite.afterWith("calm", 1, 11).first(1).all(pool())), [[1, 12]]);
  assert.throws(() => graph("app.AccountNotes").find().cursorOn(2, "body"), refused(/no source 2/));
  assert.throws(() => composite.after("calm"), refused(/does not match 2 key column/));
});
