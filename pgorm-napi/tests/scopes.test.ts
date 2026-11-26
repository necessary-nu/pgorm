// Declared models held to scopes, as Rust's scoped entities are, in either
// runtime: every statement on a scoped model starts from a `For` form given a
// context holding its scopes, which it is held to, and every statement built
// without one, or under a context missing a scope, is refused as it is built.
// `deno check` holds each `@ts-expect-error` to the types refusing what the
// builder refuses.

import assert from "node:assert/strict";
import { after, before, beforeEach, test } from "node:test";

import {
  column,
  Conflict,
  ConstructionError,
  DecodeError,
  every,
  literal,
  model,
  oneOf,
  pipeline as pl,
  Pool,
  scope,
} from "../lib/index.js";
import { scratchDatabase } from "./support.ts";

const CompanyId = scope("CompanyId");
const UserId = scope("UserId");

const Company = model("company", {
  schema: "napi_scopes",
  columns: { id: column("i64", { primaryKey: true, scope: CompanyId }), name: column("text") },
});
const Note = model("note", {
  schema: "napi_scopes",
  columns: {
    id: column("i64", { primaryKey: true, generated: "byDefault" }),
    companyId: column("i64", { name: "company_id", scope: CompanyId }),
    authorId: column("i64", { name: "author_id", scope: UserId }),
    body: column("text"),
  },
});
const Tag = model("tag", {
  schema: "napi_scopes",
  columns: { id: column("i64", { primaryKey: true }), noteId: column("i64", { name: "note_id" }), label: column("text") },
});
const Pin = model("pin", {
  columns: { id: column("i64", { primaryKey: true }), noteId: column("i64", { name: "note_id", nullable: true }) },
});
const companyNotes = Company.hasMany(Note, { from: "id", to: "companyId" });
const noteTags = Note.hasMany(Tag, { from: "id", to: "noteId" });
const tagNote = Tag.belongsTo(Note, { from: "noteId", to: "id" });
const labelled = Note.hasMany(Tag, { from: "id", to: "noteId", onCondition: (_note, tag) => tag.col("label").ne("") });

const ada = { company: CompanyId(1n), user: UserId(10n) };
const admin = { company: CompanyId(1n), users: every(UserId) };
const NOTE = `SELECT "note"."id", "note"."company_id", "note"."author_id", "note"."body" FROM "napi_scopes"."note"`;
const WHICH = "`note` is scoped by `CompanyId` and `UserId`, which";
const NO_CONTEXT =
  "`note` is scoped by `CompanyId` and `UserId`, which a statement taking no context does not hold: a statement on " +
  "`note` starts from a `For` form, given a context holding its scopes first: `findFor(cx)`, `insertFor(cx, values)`, " +
  "`updateFor(cx, values)`";

function refused(build: () => unknown, words: string) {
  assert.throws(build, (error: unknown) => error instanceof ConstructionError && error.message.includes(words));
}

// [spec:pgorm:req:bindings.scopes/test]    a read ANDs each scope its context holds one value of into its WHERE
// [spec:pgorm:req:napi.scopes/test]    a scope is scope(name), its value the scope called, a context an object of them
test("a read is held to its context", () => {
  assert.deepEqual(Note.scopes, ["CompanyId", "UserId"]);
  assert.equal(Note.findFor(ada).inspect().sql, `${NOTE} WHERE "note"."company_id" = $1 AND "note"."author_id" = $2`);
  assert.equal(Note.findFor(admin).inspect().sql, `${NOTE} WHERE "note"."company_id" = $1`);
  assert.equal(
    Company.findByKeyFor(CompanyId(1n), { id: 1n }).inspect().sql,
    `SELECT "company"."id", "company"."name" FROM "napi_scopes"."company" WHERE "company"."id" = $1 AND "company"."id" = $2`,
  );
  assert.equal(
    Note.as("n").selectFor(ada, "body").inspect().sql,
    `SELECT "n"."body" FROM "napi_scopes"."note" AS "n" WHERE "n"."company_id" = $1 AND "n"."author_id" = $2`,
  );
  assert.equal(Tag.findFor(ada).inspect().sql, Tag.find().inspect().sql);
});

// [spec:pgorm:req:bindings.scopes/test]    every form taking no context, and a context missing a scope, is refused as the statement is built
test("statements without their scopes are refused", () => {
  // @ts-expect-error: a scoped model's find() takes a context
  assert.throws(() => Note.find(), { message: NO_CONTEXT });
  // @ts-expect-error: a scoped model's select() takes a context
  refused(() => Note.select("body"), WHICH);
  // @ts-expect-error: a scoped model's findByKey() takes a context
  refused(() => Note.findByKey({ id: 1n }), WHICH);
  // @ts-expect-error: a scoped model's insert() takes a context
  refused(() => Note.insert({ companyId: 1n, authorId: 1n, body: "x" }), WHICH);
  // @ts-expect-error: a scoped model's insertMany() takes a context
  refused(() => Note.insertMany([]), WHICH);
  // @ts-expect-error: a scoped model's update() takes a context
  refused(() => Note.update({ body: "x" }), WHICH);
  // @ts-expect-error: a scoped model's delete() takes a context
  refused(() => Note.delete(), WHICH);
  // @ts-expect-error: a scoped model's graph() takes a context
  refused(() => Note.graph(), WHICH);
  // @ts-expect-error: a scoped model is a pipeline's source through fromFor
  refused(() => pl.from(Note), WHICH);
  refused(() => Tag.graph().joinOne(tagNote), WHICH);
  refused(() => tagNote.find({ noteId: 1n }), WHICH);
  refused(() => labelled.find({ id: 1n }), WHICH);
  refused(() => Tag.linked(tagNote, noteTags).findLinked(), WHICH);
  refused(() => Note.insertFor(ada, { body: "x" }).add({ body: "y" } as never), WHICH);
  // @ts-expect-error: the context holds no UserId
  assert.throws(() => Note.findFor(CompanyId(1n)), {
    message: "`note` is scoped by `CompanyId` and `UserId`, which `CompanyId` does not hold",
  });
  assert.throws(() => Note.findFor({ company: CompanyId(1n) } as never), {
    message: "`note` is scoped by `CompanyId` and `UserId`, which `{ company }` does not hold",
  });
  // @ts-expect-error: a scoped model joins by joinFor
  refused(() => Company.findFor(ada).join(companyNotes), "join it with a form taking the statement's context, such as `joinFor`");
  // @ts-expect-error: a scoped model joins by joinRevFor
  refused(() => Tag.findFor(admin).joinRev(noteTags), "such as `joinFor`");
  refused(() => Tag.update({ label: "x" }).from(Note), "such as `joinFor`");
  refused(() => Tag.delete().using(Note), "such as `joinFor`");
});

// [spec:pgorm:req:napi.scopes/test]    a context of another type, every(..) of no scope and a scope named by no type's name are TypeErrors
test("contexts are scope values, everys or objects of them", () => {
  for (const context of [1, null, "CompanyId", [CompanyId(1n)], { company: 1 }]) {
    // @ts-expect-error: no context
    assert.throws(() => Note.findFor(context), TypeError);
  }
  // @ts-expect-error: every takes a scope
  assert.throws(() => every("UserId"), TypeError);
  assert.throws(() => scope("not a name"), TypeError);
  refused(() => Note.findFor({ author: UserId(1n), editor: UserId(2n), company: CompanyId(1n) }), "holds `UserId` twice");
});

// [spec:pgorm:req:bindings.scopes/test]    a scope column is one value of its row: nullable, array and generated ones are refused, and one scope is one column
test("scope columns are declared as Rust declares them", () => {
  refused(() => column("i64", { scope: UserId, nullable: true }), "never NULL");
  refused(() => column("i64", { scope: UserId, array: true }), "not an array");
  refused(() => column("i64", { scope: UserId, generated: "always" }), "never generated");
  // @ts-expect-error: a column's scope is a scope
  assert.throws(() => column("i64", { scope: "UserId" }), TypeError);
  refused(
    () => model("pair", { columns: { author: column("i64", { scope: UserId }), editor: column("i64", { scope: UserId }) } }),
    "both hold `UserId`",
  );
  refused(() => oneOf("pinned", Pin, { note: ["noteId", Note] }), "is scoped by CompanyId, UserId");
});

// [spec:pgorm:req:bindings.scopes/test]    joins, graph slots and hops, and a relation's finds AND the joined table's scopes into its ON
test("joins hold the joined table's scopes", () => {
  assert.equal(
    Company.findFor(ada).joinFor(ada, companyNotes).inspect().sql,
    `SELECT "company"."id", "company"."name" FROM "napi_scopes"."company" ` +
      `INNER JOIN "napi_scopes"."note" ON "company"."id" = "note"."company_id" ` +
      `AND ("note"."company_id" = $1 AND "note"."author_id" = $2) WHERE "company"."id" = $3`,
  );
  const graph = Company.graphFor(admin).joinMaybe(companyNotes, { alias: "n" }).inspect().sql;
  assert.ok(graph.includes(`LEFT JOIN "napi_scopes"."note" AS "n" ON "company"."id" = "n"."company_id" AND "n"."company_id" = $1 `), graph);
  assert.ok(graph.includes(`WHERE "company"."id" = $2`), graph);
  assert.ok(Tag.findFor(ada).joinRevFor(ada, noteTags).inspect().sql.includes(`ON "note"."id" = "tag"."note_id" AND ("note"."company_id" = $1`));
  assert.equal(
    tagNote.findFor(ada, { noteId: 1n }).inspect().sql,
    `${NOTE} WHERE "note"."company_id" = $1 AND "note"."author_id" = $2 AND "note"."id" = $3`,
  );
  const linked = Company.linked(companyNotes, noteTags).findLinkedFor(ada).inspect().sql;
  assert.ok(linked.includes(`"r0"."company_id" = $1 AND "r0"."author_id" = $2`), linked);
  assert.ok(linked.includes(`"r1"."id" = $3`), linked);
});

// [spec:pgorm:req:bindings.scopes/test]    an insert fills the scope columns its row leaves out, and refuses another value or an unset column under every value
test("inserts fill and refuse their rows", () => {
  assert.equal(
    Note.insertFor(ada, { body: "x" }).inspect().sql,
    `INSERT INTO "napi_scopes"."note" ("company_id", "author_id", "body") VALUES ($1, $2, $3)`,
  );
  assert.deepEqual(Note.insertFor(ada, { body: "x", authorId: 10n }).inspect().values, Note.insertFor(ada, { body: "x" }).inspect().values);
  refused(() => Note.insertFor(ada, { body: "x", companyId: 2n }), "the row sets `note`.`company_id` to a value outside the context's `CompanyId`");
  refused(() => Note.insertFor(ada, { body: "x", companyId: literal(1) }), "to a value outside the context's `CompanyId`");
  refused(
    () => Note.insertFor(admin, { body: "x" }),
    "the context holds every `UserId`, so the row sets `note`.`author_id` itself, and it is not set",
  );
  refused(() => Note.insertManyFor(ada, [{ body: "x" }, { body: "y", authorId: 11n }]), "outside the context's `UserId`");
  refused(() => Note.insertFor(ada, { body: "x" }).addFor(admin, { body: "y" }), "the context holds every `UserId`");
  refused(
    () => Note.insertFor(ada, { body: "x" }).onConflict(Conflict.on("id").update("author_id")),
    "`note`.`author_id` is written when its row is inserted and never changed",
  );
  const batch = Note.insertManyFor(admin, [{ body: "x", authorId: 10n }]).addFor(ada, { body: "y" });
  assert.equal(batch.inspect().values.length, 6);
});

// [spec:pgorm:req:bindings.scopes/test]    an update and a delete are held to the scopes, and a scope column is never updated
test("updates and deletes hold the scopes", () => {
  assert.equal(
    Note.updateFor(ada, { body: "x" }).inspect().sql,
    `UPDATE "napi_scopes"."note" SET "body" = $1 WHERE "note"."company_id" = $2 AND "note"."author_id" = $3`,
  );
  assert.equal(
    Note.deleteFor(admin).where(Note.col("id").eq(2n)).inspect().sql,
    `DELETE FROM "napi_scopes"."note" WHERE "note"."company_id" = $1 AND "note"."id" = $2`,
  );
  // @ts-expect-error: a scope field is never updated
  assert.throws(() => Note.updateFor(admin, { authorId: 10n }), {
    message: "`note`.`author_id` is written when its row is inserted and never changed",
  });
});

// [spec:pgorm:req:bindings.scopes/test]    a pipeline fromFor filters by each scope value, binding it
test("pipelines filter by their context", () => {
  assert.equal(pl.fromFor(ada, Note).inspect().sql, "SELECT * FROM napi_scopes.note WHERE company_id = $1 AND author_id = $2");
  assert.equal(pl.fromFor(admin, Note).inspect().sql, "SELECT * FROM napi_scopes.note WHERE company_id = $1");
  assert.equal(pl.fromFor(ada, Tag).inspect().sql, pl.from(Tag).inspect().sql);
});

let database: { pool: Pool; drop(): Promise<void> } | undefined;

function pool(): Pool {
  if (!database) throw new Error("the scratch database was not created");
  return database.pool;
}

before(async () => {
  const scratch = await scratchDatabase("pgorm_napi_scopes");
  const created = new Pool(scratch.dsn, { maxSize: 2 });
  await created.execute("CREATE SCHEMA napi_scopes");
  await created.execute("CREATE TABLE napi_scopes.company (id int8 PRIMARY KEY, name text NOT NULL)");
  await created.execute(
    "CREATE TABLE napi_scopes.note (id int8 GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, " +
      "company_id int8 NOT NULL REFERENCES napi_scopes.company, author_id int8 NOT NULL, body text NOT NULL)",
  );
  await created.execute("CREATE TABLE napi_scopes.tag (id int8 PRIMARY KEY, note_id int8 NOT NULL, label text NOT NULL)");
  database = {
    pool: created,
    async drop() {
      await created.close();
      await scratch.drop();
    },
  };
});

beforeEach(async () => {
  await pool().execute("TRUNCATE napi_scopes.tag, napi_scopes.note, napi_scopes.company RESTART IDENTITY");
  for (const [id, name] of [[1n, "Acme"], [2n, "Globex"]] as const) {
    await Company.insertFor(CompanyId(id), { name }).execute(pool());
  }
  for (const [company, user, body] of [[1n, 10n, "Ada's plan"], [1n, 11n, "Bo's draft"], [2n, 20n, "Cy's memo"]] as const) {
    await Note.insertFor({ company: CompanyId(company), user: UserId(user) }, { body }).execute(pool());
  }
  await Tag.insertMany([{ id: 1n, noteId: 1n, label: "plan" }, { id: 2n, noteId: 2n, label: "draft" }]).execute(pool());
});

after(async () => {
  await database?.drop();
});

const bodies = (records: readonly { readonly body: string }[]) => records.map((record) => record.body).sort();

// [spec:pgorm:req:bindings.scopes/test]    reads, graphs, joins, finds and pipelines return only the rows the context holds
test("reads return the context's rows", async () => {
  assert.deepEqual(bodies(await Note.findFor(ada).all(pool())), ["Ada's plan"]);
  assert.deepEqual(bodies(await Note.findFor(admin).all(pool())), ["Ada's plan", "Bo's draft"]);
  await assert.rejects(Note.findByKeyFor(ada, { id: 3n }).one(pool()), DecodeError);
  assert.equal(await Note.findFor(admin).count(pool()), 2);
  const graph = await Company.graphFor(ada).joinMaybe(companyNotes).all(pool());
  assert.deepEqual(graph.map(([company, note]) => [company.name, note?.body ?? null]), [["Acme", "Ada's plan"]]);
  const joined = await Company.findFor(admin).joinFor(admin, companyNotes).all(pool());
  assert.deepEqual(joined.map((company) => company.name), ["Acme", "Acme"]);
  const tags = await Tag.findFor(ada).all(pool());
  const loaded = await tagNote.load(pool(), tags, { query: Note.findFor(ada) });
  assert.deepEqual(loaded.map((note) => note?.body ?? null), ["Ada's plan", null]);
  assert.deepEqual((await noteTags.findFor(ada, { id: 1n }).all(pool())).map((tag) => tag.label), ["plan"]);
  assert.deepEqual(bodies(await pl.fromFor(admin, Note).intoModel(Note).all(pool())), ["Ada's plan", "Bo's draft"]);
});

// [spec:pgorm:req:bindings.scopes/test]    writes fill the row, and an update or delete outside the scopes touches no row
test("writes reach only the context's rows", async () => {
  const written = await Note.insertFor(ada, { body: "Ada's second" }).returning().one(pool());
  assert.deepEqual([written.companyId, written.authorId], [1n, 10n]);
  assert.equal(await Note.updateFor(ada, { body: "x" }).where(Note.col("id").eq(2n)).execute(pool()), 0);
  assert.equal(await Note.updateFor(admin, { body: "reviewed" }).where(Note.col("id").eq(2n)).execute(pool()), 1);
  assert.equal(await Note.deleteFor(ada).where(Note.col("id").eq(2n)).execute(pool()), 0);
  const edit = Note.active({ id: 2n, body: "again" });
  await assert.rejects(edit.updateFor(ada, pool()), DecodeError);
  assert.equal((await edit.updateFor(admin, pool())).body, "again");
  const created = await Note.active({ body: "hooked" }).insertFor(ada, pool());
  assert.equal(created.authorId, 10n);
  const globex = { company: CompanyId(2n), users: every(UserId) };
  assert.equal(await Note.active({ id: created.id }).deleteFor(globex, pool()), 0);
  assert.equal(await Note.active({ id: created.id }).deleteFor(ada, pool()), 1);
  // @ts-expect-error: a scoped model's active insert takes a context
  await assert.rejects(Note.active({ body: "x" }).insert(pool()), ConstructionError);
  assert.deepEqual(bodies(await Note.findFor(admin).all(pool())), ["Ada's plan", "Ada's second", "again"]);
});
