// Models JavaScript declares, against a live server in either runtime: their
// declarations, the records their reads decode, the writes that tell a field
// left out from one set to NULL, the two versions a write returns, and the
// refusals that keep a declaration, a value or a result from being misread.

import assert from "node:assert/strict";
import { after, before, beforeEach, test } from "node:test";

import {
  all,
  any,
  column,
  Conflict,
  ConstructionError,
  CreatedMultirange,
  CreatedRange,
  DecodeError,
  model,
  ModelInsert,
  ModelInsertMany,
  Multirange,
  type Pool,
  Range,
  TypeName,
  Value,
} from "../lib/index.js";
import { Account, modelDatabase, truncate, Version } from "./model-fixtures.ts";
import { mistyped } from "./support.ts";

let database: { pool: Pool; drop(): Promise<void> } | undefined;

function pool(): Pool {
  if (!database) throw new Error("the scratch database was not created");
  return database.pool;
}

before(async () => {
  database = await modelDatabase("pgorm_napi_models");
});

beforeEach(async () => {
  await truncate(pool());
});

after(async () => {
  await database?.drop();
});

function refused(pattern: RegExp): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof ConstructionError, `expected a ConstructionError, got ${error}`);
    assert.match(error.message, pattern);
    return true;
  };
}

function undecodable(pattern: RegExp): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof DecodeError, `expected a DecodeError, got ${error}`);
    assert.match(error.message, pattern);
    return true;
  };
}

async function seed(): Promise<void> {
  await Account.insertMany([
    { name: "Ann", mood: "calm", tags: ["a"] },
    { name: "Bob", mood: null, tags: ["b", null] },
    { name: "Cy", mood: "glad", tags: [] },
  ]).execute(pool());
}

const byId = Account.col("id").asc();

// [spec:pgorm:req:bindings.models+5/test]
test("a declaration is data: it sends nothing, and describes its table and fields", () => {
  const unconnected = model("nowhere", { schema: "absent", columns: { id: column("i32", { primaryKey: true }) } });
  assert.equal(unconnected.find().inspect().sql, 'SELECT "nowhere"."id" FROM "absent"."nowhere"');
  assert.deepStrictEqual(Account.primaryKey, ["id"]);
  assert.equal(Account.columns.displayName.name, "display_name");
  const described = Account.describe();
  assert.equal(described.table, "account");
  assert.equal(described.schema, "app");
  assert.deepStrictEqual(described.fields.mood, {
    column: "mood",
    kind: 'enum "app"."mood"',
    nullable: true,
    primaryKey: false,
    default: false,
    generated: null,
    values: ["calm", "glad", "sour"],
  });
  assert.equal(described.fields.tags?.kind, "text[]");
  assert.equal(described.fields.span?.kind, 'range "app"."floatrange" of f64');
  assert.ok(Object.isFrozen(Account));
  assert.ok(Object.isFrozen(Account.columns.name));
  const aliased = Account.as("a");
  assert.equal(aliased.alias, "a");
  assert.equal(aliased.col("name").inspect().sql, 'SELECT "a"."name"');
  assert.equal(Account.alias, null);
});

// [spec:pgorm:req:bindings.models+5/test]
// [spec:pgorm:req:bindings.argument-errors+1/test]
test("a declaration that cannot be used is refused as it is made", () => {
  const unknown = { nullabel: true } as unknown as Record<string, never>;
  assert.throws(() => column("text", unknown), mistyped(/"nullabel" is no column option/));
  assert.throws(() => column("text", { nullable: "yes" as never }), mistyped(/nullable is a boolean/));
  assert.throws(() => column("text", { name: 7 as never }), mistyped(/name is a string/));
  assert.throws(() => column("i32", { generated: "sometimes" as never }), mistyped(/"always" or "byDefault"/));
  assert.throws(() => column(7 as never), mistyped(/a column's kind is/));
  assert.throws(() => column(new TypeName("mood", { schema: "app" }), { values: "a" as never }), mistyped(/a list of strings/));
  assert.throws(() => column(new TypeName("mood", { schema: "app" }), { values: [] }), refused(/at least one value/));
  assert.throws(() => column("u64" as "i64"), refused(/never decodes as u64/));
  assert.throws(() => column(new TypeName("mood")), refused(/names its type's schema/));
  assert.throws(() => column("i32", { primaryKey: true, nullable: true }), refused(/primary-key column is not nullable/));
  assert.throws(() => column("i32", { generated: "always", default: true }), refused(/declare one or the other/));
  assert.throws(() => column("text", { values: ["a"] }), refused(/only an enum column lists its values/));
  assert.throws(() => column(new TypeName("mood", { schema: "app" }), { values: ["a", "a"] }), refused(/each value once/));
  assert.throws(
    () => model("t", { columns: { a: column("text"), b: column("text", { name: "a" }) } }),
    refused(/two fields of a model name one column/),
  );
  assert.throws(() => model("t", { columns: {} }), refused(/at least one column/));
  assert.throws(() => model("t", { columns: { a: "text" as never } }), mistyped(/no column\(\) declaration/));
  assert.throws(() => model("t", { columns: { a: column("text") }, table: "x" } as never), mistyped(/"table" is no model option/));
  assert.throws(() => model("", { columns: { a: column("text") } }), ConstructionError);
  assert.throws(() => model("t", { columns: { a: column("text", { name: "x".repeat(64) }) } }), ConstructionError);
});

// [spec:pgorm:req:bindings.model-unique-keys+2/test]    a model declares its unique keys in order, one field or several, named or not and NULLS NOT DISTINCT or not, describes each and keeps them under an alias, sending nothing
test("a model declares and describes its unique keys", () => {
  const columns = {
    id: column("i32", { primaryKey: true }),
    handle: column("text", { name: "user handle" }),
    realm: column("text", { nullable: true }),
  };
  const members = model("members", {
    columns,
    uniqueKeys: [
      { fields: ["realm", "handle"], name: "members_realm_handle", nullsNotDistinct: true },
      { fields: "handle" },
    ],
  });
  const expected = [
    { name: "members_realm_handle", fields: ["realm", "handle"], nullsNotDistinct: true },
    { name: null, fields: ["handle"], nullsNotDistinct: false },
  ];
  assert.deepStrictEqual(members.describe().uniqueKeys, expected);
  assert.deepStrictEqual(members.as("m").describe().uniqueKeys, expected);
  assert.deepStrictEqual(model("members", { columns }).describe().uniqueKeys, []);
});

// [spec:pgorm:req:bindings.model-unique-keys+2/test]    a unique key naming no field, a field the model lacks or a field twice, or a name that is no identifier, is a ConstructionError as the model is declared; a key of another type, an unknown option and a mistyped field, name or flag a TypeError
test("a unique key PostgreSQL or the derive would refuse is refused as the model is declared", () => {
  const columns = { id: column("i32", { primaryKey: true }), handle: column("text") };
  const declare = (...keys: unknown[]) => model("members", { columns, uniqueKeys: keys as never });
  assert.throws(() => declare({ fields: [] }), refused(/at least one field/));
  assert.throws(() => declare({}), refused(/at least one field/));
  assert.throws(() => declare({ fields: ["handle", "missing"] }), refused(/"missing", which is no field/));
  assert.throws(() => declare({ fields: ["handle", "id", "handle"] }), refused(/"handle" twice/));
  assert.throws(() => declare({ fields: ["handle"], name: "" }), refused(/identifier/));
  assert.throws(() => declare({ fields: ["handle"], name: "k".repeat(64) }), refused(/identifier/));
  assert.throws(() => model("members", { columns, uniqueKeys: { fields: "handle" } as never }), mistyped(/an array of keys/));
  assert.throws(() => declare(["handle", "id"]), mistyped(/a unique key is an object/));
  assert.throws(() => declare({ fields: ["handle"], nullsDistinct: false }), mistyped(/"nullsDistinct" is no unique key option/));
  assert.throws(() => declare({ fields: 7 }), mistyped(/a field or an array/));
  assert.throws(() => declare({ fields: [7] }), mistyped(/named by a string/));
  assert.throws(() => declare({ fields: ["handle"], name: 7 }), mistyped(/name is a string/));
  assert.throws(() => declare({ fields: ["handle"], nullsNotDistinct: 1 }), mistyped(/is a boolean/));
});

// [spec:pgorm:req:bindings.model-writes+2/test]
// [spec:pgorm:req:bindings.model-records+1/test]
test("an insert writes the fields it sets, the rest taking the table's defaults", async () => {
  const ann = await Account.insert({ mood: "calm", name: "Ann", span: new Range(1.5, 2.5) }).returning().one(pool());
  assert.deepStrictEqual(Object.keys(ann), ["id", "name", "displayName", "mood", "tags", "visits", "created", "span", "seq"]);
  assert.equal(ann.id, 1n);
  assert.equal(ann.displayName, null);
  assert.equal(ann.mood, "calm");
  assert.deepStrictEqual(ann.tags, []);
  assert.equal(ann.visits, 0);
  assert.ok(ann.created instanceof Temporal.Instant);
  assert.ok(ann.span instanceof Range);
  assert.equal(String(ann.span), "[1.5,2.5)");
  assert.equal(ann.seq, 1);
  assert.deepStrictEqual(await Account.insert({ name: "Bob" }).returning("id", "name").one(pool()), { id: 2n, name: "Bob" });
});

// [spec:pgorm:req:bindings.model-writes+2/test]
test("a field left out stays as it is, and one set to null is written as NULL", async () => {
  await Account.insert({ name: "Ann", displayName: "A", mood: "calm" }).execute(pool());
  const ann = Account.findByKey({ id: 1n });
  assert.equal(await Account.update({ mood: "glad" }).where(Account.key({ id: 1n })).execute(pool()), 1);
  assert.deepStrictEqual(await Account.select("displayName", "mood").where(Account.col("id").eq(1)).one(pool()), {
    displayName: "A",
    mood: "glad",
  });
  await Account.update({ displayName: null, visits: Account.col("visits").add(2) }).where(Account.key({ id: 1n })).execute(
    pool(),
  );
  const changed = await ann.one(pool());
  assert.equal(changed.displayName, null);
  assert.equal(changed.mood, "glad");
  assert.equal(changed.visits, 2);
  assert.equal(changed.seq, 3);
});

// [spec:pgorm:req:bindings.model-writes+2/test]
test("a write that cannot be built is refused before anything is sent", async () => {
  const write = (values: unknown) => () => Account.insert(values as never);
  assert.throws(write({ displayName: "x" }), refused(/name is required/));
  assert.throws(write({ name: "x", seq: 1 }), refused(/seq is generated always/));
  assert.throws(write({ name: "x", nickname: "y" }), refused(/no field "nickname"/));
  assert.throws(write({ name: undefined }), refused(/name is undefined/));
  assert.throws(write({ name: null }), refused(/name is not nullable/));
  assert.throws(write({ name: 5 }), ConstructionError);
  assert.throws(write({ name: "x", visits: new Value(1, "i64") }), refused(/visits is i32, and the Value given is i64/));
  assert.throws(write({ name: "x", mood: "wild" }), refused(/"wild" is not one of mood's values/));
  assert.throws(write({ name: "x", tags: "a" }), refused(/tags is an array column/));
  assert.throws(() => Account.update({}), refused(/sets at least one field/));
  assert.throws(() => Account.update({ seq: 2 } as never), refused(/seq is generated always/));
  assert.throws(() => Account.insertMany([{ name: "a" }, { name: "b", mood: "calm" }]), refused(/sets the same fields/));
  assert.throws(() => Account.as("a").insert({ name: "x" }), refused(/unaliased/));
  await assert.rejects(Account.update({ name: "x" }).execute(pool()), refused(/WHERE|where/));
  await assert.rejects(Account.delete().execute(pool()), refused(/WHERE|where/));
  assert.equal(await Account.find().count(pool()), 0);
});

// [spec:pgorm:req:bindings.model-writes+2/test]
test("insertMany writes rows that set the same fields, and a batch of none sends nothing", async () => {
  const rows = await Account.insertMany([{ name: "Ann" }, { name: "Bob" }]).returning("id").all(pool());
  assert.deepStrictEqual(rows, [{ id: 1n }, { id: 2n }]);
  const none = Account.insertMany([]);
  assert.equal(await none.execute(pool()), 0);
  assert.deepStrictEqual(await none.returning().all(pool()), []);
  assert.equal(await none.returning().optional(pool()), null);
  await assert.rejects(none.returning().one(pool()), DecodeError);
  assert.deepStrictEqual(await none.returningUpserts(pool()), []);
  assert.throws(() => none.inspect(), refused(/no rows writes nothing/));
  assert.equal(await Account.find().count(pool()), 2);
});

// [spec:pgorm:req:bindings.model-writes+2/test]
test("an insert is typed by its row count, and add or addMany makes a batch of its rows and theirs", async () => {
  const one = Account.insert({ name: "Ann" });
  const many = Account.insertMany([{ name: "Bob" }]);
  assert.ok(one instanceof ModelInsert && !(one instanceof ModelInsertMany));
  assert.ok(many instanceof ModelInsertMany && !(many instanceof ModelInsert));
  assert.equal("returningUpserts" in one, false);
  assert.equal("returningUpsert" in many, false);
  assert.throws(() => one.inspect("returningUpserts" as never), TypeError);
  assert.throws(() => many.inspect("returningUpsert" as never), TypeError);
  const added = one.add({ name: "Cy" });
  assert.ok(added instanceof ModelInsertMany);
  assert.equal(added.inspect().sql, Account.insertMany([{ name: "Ann" }, { name: "Cy" }]).inspect().sql);
  assert.equal(many.addMany([]).inspect().sql, many.inspect().sql);
  const fromNone = Account.insertMany([]).add({ name: "Dee" });
  assert.equal(fromNone.inspect().sql, Account.insertMany([{ name: "Dee" }]).inspect().sql);
  assert.throws(() => one.add({ name: "x", mood: "calm" }), refused(/sets the same fields/));
  assert.throws(() => one.addMany({ name: "x" } as never), TypeError);
  assert.throws(() => Account.insert({ name: "x" }).add({ name: "x", nickname: "y" } as never), refused(/no field "nickname"/));
  const renamed = Conflict.on("id").update("name");
  const kept = Account.insert({ id: 1, name: "Ann" }).onConflict(renamed).add({ id: 2, name: "Bob" });
  assert.match(kept.inspect().sql, /ON CONFLICT \("id"\) DO UPDATE/);
  assert.deepStrictEqual((await kept.returningUpserts(pool())).map((row) => [row.kind, row.new.name]), [
    ["inserted", "Ann"],
    ["inserted", "Bob"],
  ]);
  const again = Account.insert({ id: 1, name: "Anne" }).onConflict(renamed).addMany([{ id: 3, name: "Cy" }]);
  assert.deepStrictEqual((await again.returningUpserts(pool())).map((row) => [row.kind, row.new.name]), [
    ["updated", "Anne"],
    ["inserted", "Cy"],
  ]);
  assert.equal(await Account.insert({ id: 10, name: "Eve" }).add({ id: 11, name: "Fay" }).execute(pool()), 2);
  assert.equal(await Account.find().count(pool()), 5);
});

// [spec:pgorm:req:bindings.model-reads/test]
test("find, select and findByKey read records, filtered, ordered and windowed", async () => {
  await seed();
  const names = (rows: { name: string }[]) => rows.map((row) => row.name);
  assert.deepStrictEqual(names(await Account.find().orderBy(byId).all(pool())), ["Ann", "Bob", "Cy"]);
  assert.deepStrictEqual(await Account.select("name").where(Account.col("tags").contains(["b"])).all(pool()), [{ name: "Bob" }]);
  assert.deepStrictEqual(
    names(await Account.find().where(Account.col("id").gte(2)).orderBy(Account.col("name").desc()).limit(1).all(pool())),
    ["Cy"],
  );
  assert.deepStrictEqual(names(await Account.find().orderBy(byId).offset(1).limit(null).all(pool())), ["Bob", "Cy"]);
  assert.equal((await Account.findByKey({ id: 2 }).one(pool())).name, "Bob");
  assert.equal(await Account.findByKey({ id: 9 }).optional(pool()), null);
  await assert.rejects(Account.find().one(pool()), DecodeError);
  await assert.rejects(Account.find().optional(pool()), DecodeError);
  assert.equal(await Account.find().where(Account.col("mood").isNotNull()).count(pool()), 2);
  assert.equal(await Account.find().limit(1).count(pool()), 3);
});

// [spec:pgorm:req:bindings.model-columns+1/test]
// [spec:pgorm:req:bindings.column-operators+3/test]
test("a column compares through its declared kind, and refuses what it cannot bind", async () => {
  await seed();
  const glad = await Account.select("name").where(Account.col("mood").eq("glad")).all(pool());
  assert.deepStrictEqual(glad, [{ name: "Cy" }]);
  assert.match(Account.col("mood").eq("glad").inspect().sql, /CAST\(\$1::text AS app\.mood\)/);
  assert.match(
    Account.col("span").adjacentTo(new Value(new Range(2.5, 3.0), new CreatedRange("floatrange", "f64", { schema: "app" })))
      .inspect().sql,
    /"span" -\|- CAST\(\$1::text AS app\.floatrange\)$/,
  );
  assert.equal(Account.col("id").eq(1).inspect().values[0]?.kind, "i64");
  assert.deepStrictEqual(
    await Account.select("name").where(Account.col("mood").isIn(["calm", "glad"])).orderBy(byId).all(pool()),
    [{ name: "Ann" }, { name: "Cy" }],
  );
  assert.deepStrictEqual(
    await Account.select("name").where(Account.col("name").eq(any(["Ann", "Cy"]))).orderBy(byId).all(pool()),
    [{ name: "Ann" }, { name: "Cy" }],
  );
  assert.deepStrictEqual(
    await Account.select("name").where(Account.col("name").ne(all(["Ann", "Cy"]))).orderBy(byId).all(pool()),
    [{ name: "Bob" }],
  );
  assert.throws(() => Account.col("id").isDistinctFrom(any([1]) as never), refused(/ANY and ALL follow a comparison/));
  assert.deepStrictEqual(await Account.select("name").where(Account.col("id").between(2, 3)).orderBy(byId).all(pool()), [
    { name: "Bob" },
    { name: "Cy" },
  ]);
  assert.throws(() => Account.col("mood").eq(null as never), refused(/compared with null is never true/));
  assert.throws(() => Account.col("mood").eq("wild" as never), refused(/not one of mood's values/));
  assert.throws(() => Account.col("id").eq("1" as never), ConstructionError);
  assert.throws(() => Account.col("id").eq(new Value(1, "i32")), refused(/id is i64, and the Value given is i32/));
  assert.throws(() => Account.col("nickname" as never), refused(/no field "nickname"/));
  assert.deepStrictEqual(
    await Account.select("name").where(Account.col("name").eq(Account.col("name"))).orderBy(byId).limit(1).all(pool()),
    [{ name: "Ann" }],
  );
});

// [spec:pgorm:req:bindings.model-reads/test]
test("a key names every key field and no other, a composite one included", async () => {
  await Version.insertMany([{ doc: 1, rev: 1, body: "one" }, { doc: 1, rev: 2, body: "two" }]).execute(pool());
  assert.equal((await Version.findByKey({ doc: 1, rev: 2 }).one(pool())).body, "two");
  assert.throws(() => Version.key({ doc: 1 } as never), mistyped(/gives doc, rev, and nothing else/));
  assert.throws(() => Version.key({ doc: 1, rev: 1, body: "x" } as never), mistyped(/gives doc, rev, and nothing else/));
  assert.throws(() => Version.key({ doc: 1, rev: null } as never), refused(/compared with null/));
  const keyless = model("keyless", { columns: { a: column("text") } });
  assert.throws(() => keyless.key({} as never), refused(/declares no primary key/));
});

// [spec:pgorm:req:bindings.model-records+1/test]
test("a result that breaks the declaration is a DecodeError, never a value", async () => {
  await seed();
  await pool().execute("INSERT INTO app.account (name, mood) VALUES ('Dee', 'wild')");
  const narrow = model("account", { schema: "app", columns: { id: column("i32", { primaryKey: true }) } });
  await assert.rejects(narrow.find().all(pool()), undecodable(/column "id" is i64, and id declares i32/));
  const strict = model("account", { schema: "app", columns: { id: column("i64"), mood: column(new TypeName("mood", { schema: "app" })) } });
  await assert.rejects(
    strict.find().where(strict.col("id").eq(2)).all(pool()),
    undecodable(/mood is not nullable, and the row holds NULL/),
  );
  const elsewhere = model("account", {
    schema: "app",
    columns: { id: column("i64"), mood: column(new TypeName("mood", { schema: "public" }), { nullable: true }) },
  });
  await assert.rejects(elsewhere.find().all(pool()), undecodable(/is enum "app"."mood", and mood declares enum "public"."mood"/));
  await assert.rejects(Account.find().all(pool()), undecodable(/"wild" is not one of mood's values/));
  const partial = model("account", { schema: "app", columns: { id: column("i64"), name: column("text") } });
  assert.equal((await partial.find().all(pool())).length, 4);
});

// [spec:pgorm:req:bindings.models+5/test]
// [spec:pgorm:req:bindings.model-records+1/test]
// [spec:pgorm:req:bindings.created-ranges+5/test]
test("a created multirange field is projected as its text and read as the multirange of its declared subtype", async () => {
  await pool().execute("CREATE TABLE app.slot (id int4 PRIMARY KEY, open app.floatmultirange)");
  const kind = new CreatedMultirange("floatmultirange", "f64", { schema: "app" });
  const Slot = model("slot", {
    schema: "app",
    columns: { id: column("i32", { primaryKey: true }), open: column(kind, { nullable: true }) },
  });
  assert.equal(Slot.find().inspect().sql, 'SELECT "slot"."id", CAST("slot"."open" AS text) AS "open" FROM "app"."slot"');
  const hours = new Multirange([new Range(9, 12.5), new Range(13.5, 17)]);
  assert.deepStrictEqual(await Slot.insert({ id: 1, open: hours }).returning().one(pool()), { id: 1, open: hours });
  assert.deepStrictEqual(await Slot.insert({ id: 2, open: null }).returning("open").one(pool()), { open: null });
  assert.equal((await Slot.findByKey({ id: 2 }).one(pool())).open, null);
  assert.deepStrictEqual(await Slot.findByKey({ id: 1 }).one(pool()), { id: 1, open: hours });
  const filtered = Slot.find().where(Slot.col("open").eq(hours));
  assert.deepStrictEqual(await filtered.all(pool()), [{ id: 1, open: hours }]);
  // The text is read through the declared subtype, so text that subtype does
  // not read is a DecodeError, never a value of another type.
  const wrong = model("slot", {
    schema: "app",
    columns: {
      id: column("i32", { primaryKey: true }),
      open: column(new CreatedMultirange("floatmultirange", "i64", { schema: "app" }), { nullable: true }),
    },
  });
  await assert.rejects(wrong.findByKey({ id: 1 }).one(pool()), undecodable(/is no value of floatmultirange/));
  const later = new Multirange([new Range(18, null)]);
  const change = await Slot.update({ open: later }).where(Slot.key({ id: 1 })).returningChange(pool());
  assert.deepStrictEqual([change.old.open, change.new.open], [hours, later]);
  assert.deepStrictEqual(await Slot.graph().where(Slot.col("id").eq(1)).all(pool()), [{ id: 1, open: later }]);
});

// [spec:pgorm:req:bindings.created-ranges+5/test]    a declared column of an array of a created range or
// multirange writes the array of its values' texts cast to the type's array, and reads it back, a
// multirange's projected as text[]
test("an array field of a created range or multirange writes and reads back as an array of its kind", async () => {
  await pool().execute("CREATE TABLE app.shelf (id int4 PRIMARY KEY, spans app.floatrange[], week app.floatmultirange[])");
  const spans = new CreatedRange("floatrange", "f64", { schema: "app" });
  const week = new CreatedMultirange("floatmultirange", "f64", { schema: "app" });
  const Shelf = model("shelf", {
    schema: "app",
    columns: {
      id: column("i32", { primaryKey: true }),
      spans: column(spans, { array: true, nullable: true }),
      week: column(week, { array: true, nullable: true }),
    },
  });
  assert.equal(
    Shelf.find().inspect().sql,
    'SELECT "shelf"."id", "shelf"."spans", CAST("shelf"."week" AS text[]) AS "week" FROM "app"."shelf"',
  );
  const stocked = [new Range(1.5, null), null, Range.empty<number>()];
  const hours = [new Multirange([new Range(9, 12.5)]), null, new Multirange<number>([])];
  assert.deepStrictEqual(await Shelf.insert({ id: 1, spans: stocked, week: hours }).returning().one(pool()), {
    id: 1,
    spans: stocked,
    week: hours,
  });
  await Shelf.insert({ id: 2, spans: [], week: null }).execute(pool());
  assert.deepStrictEqual(await Shelf.find().where(Shelf.col("spans").eq([])).all(pool()), [{ id: 2, spans: [], week: null }]);
  assert.deepStrictEqual((await Shelf.find().where(Shelf.col("spans").eq(stocked)).one(pool())).id, 1);
  assert.throws(() => Shelf.insert({ id: 3, spans: [new Range(1n, 2n)] as never }), ConstructionError);
});

// [spec:pgorm:req:bindings.model-records+1/test]
test("records stream from a pool or a connection, and a transaction has no stream", async () => {
  await seed();
  const streamed = [];
  for await (const record of Account.select("id", "tags").orderBy(byId).stream(pool())) streamed.push(record);
  assert.deepStrictEqual(streamed, [{ id: 1n, tags: ["a"] }, { id: 2n, tags: ["b", null] }, { id: 3n, tags: [] }]);
  await pool().connection(async (connection) => {
    const first = [];
    for await (const record of Account.select("name").orderBy(byId).stream(connection)) {
      first.push(record);
      break;
    }
    assert.deepStrictEqual(first, [{ name: "Ann" }]);
  });
  await pool().transaction(async (transaction) => {
    assert.throws(() => Account.find().stream(transaction as never), TypeError);
    assert.equal(await Account.find().count(transaction), 3);
  });
  await pool().execute("UPDATE app.account SET mood = 'wild' WHERE id = 3");
  const stream = Account.find().orderBy(byId).stream(pool());
  await assert.rejects(
    (async () => {
      for await (const _ of stream) { /* the third row breaks the declaration */ }
    })(),
    undecodable(/"wild" is not one of mood's values/),
  );
});

// [spec:pgorm:req:bindings.model-writes+2/test]
test("an update returns each row it wrote before and after", async () => {
  await seed();
  const change = await Account.update({ name: "Anne", visits: 5 }).where(Account.key({ id: 1n })).returningChange(pool());
  assert.equal(change.old.name, "Ann");
  assert.equal(change.new.name, "Anne");
  assert.equal(change.old.seq, 1);
  assert.equal(change.new.seq, 6);
  assert.ok(Object.isFrozen(change));
  const changes = await Account.update({ mood: "sour" }).where(Account.col("id").gte(2)).returningChanges(pool());
  assert.deepStrictEqual(changes.map(({ old, new: now }) => [old.mood, now.mood]), [[null, "sour"], ["glad", "sour"]]);
  await assert.rejects(
    Account.update({ name: "x" }).where(Account.key({ id: 99n })).returningChange(pool()),
    undecodable(/exactly one row|expected/),
  );
  assert.deepStrictEqual(await Account.update({ name: "x" }).where(Account.key({ id: 99n })).returningChanges(pool()), []);
});

// [spec:pgorm:req:bindings.model-writes+2/test]
test("an upsert tells a row it inserted from one it updated, and one its conflict clause held back", async () => {
  await Account.insert({ name: "Ann" }).execute(pool());
  const renamed = Conflict.on("id").update("name");
  const updated = await Account.insert({ id: 1, name: "Anne" }).onConflict(renamed).returningUpsert(pool());
  assert.ok(updated?.kind === "updated");
  assert.equal(updated.old.name, "Ann");
  assert.equal(updated.new.name, "Anne");
  const inserted = await Account.insert({ id: 7, name: "Gil" }).onConflict(renamed).returningUpsert(pool());
  assert.ok(inserted?.kind === "inserted");
  assert.equal(inserted.new.name, "Gil");
  assert.equal("old" in inserted, false);
  assert.equal(await Account.insert({ id: 7, name: "Hal" }).onConflict(Conflict.doNothing()).returningUpsert(pool()), null);
  const many = await Account.insertMany([{ id: 1, name: "Ann" }, { id: 8, name: "Ivy" }])
    .onConflict(Conflict.on("id").update("name").where(Account.col("name").ne("Anne")))
    .returningUpserts(pool());
  assert.deepStrictEqual(many.map((row) => [row.kind, row.new.name]), [["inserted", "Ivy"]]);
  assert.throws(() => Account.insert({ name: "x" }).onConflict(Conflict.on("id") as never), TypeError);
});

// [spec:pgorm:req:bindings.model-writes+2/test]
test("a delete removes the rows its condition matches and returns what it removed", async () => {
  await seed();
  assert.deepStrictEqual(await Account.delete().where(Account.col("name").eq("Bob")).returning("name").all(pool()), [
    { name: "Bob" },
  ]);
  assert.equal(await Account.delete().allRows().execute(pool()), 2);
});

// [spec:pgorm:req:bindings.model-writes+2/test]
// [spec:pgorm:req:bindings.transactions/test]
test("a model's writes run in a transaction and roll back with it", async () => {
  await assert.rejects(
    pool().transaction(async (transaction) => {
      await Account.insert({ name: "Ann" }).execute(transaction);
      assert.equal(await Account.find().count(transaction), 1);
      throw new Error("roll back");
    }),
    /roll back/,
  );
  assert.equal(await Account.find().count(pool()), 0);
});
