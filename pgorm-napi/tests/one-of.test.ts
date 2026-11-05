// One-of references declared over runtime models, in either runtime: the
// declaration and its refusals, the CHECK pgorm's shape writes, and a
// reference read, set and loaded on a live server.

import assert from "node:assert/strict";
import { after, before, beforeEach, test } from "node:test";

import {
  AlterTable,
  CheckViolationError,
  column,
  ConstructionError,
  createOneOf,
  DecodeError,
  model,
  oneOf,
  Pool,
} from "../lib/index.js";
import { mistyped, scratchDatabase } from "./support.ts";

const SCHEMA = "one_of";

const post = model("post", {
  schema: SCHEMA,
  columns: { id: column("i32", { primaryKey: true }), title: column("text") },
});
const photo = model("photo", {
  schema: SCHEMA,
  columns: { id: column("i64", { primaryKey: true }), url: column("text") },
});
const comment = model("comment", {
  schema: SCHEMA,
  columns: {
    id: column("i32", { primaryKey: true }),
    body: column("text"),
    post: column("i32", { name: "post_id", nullable: true }),
    photo: column("i64", { name: "photo_id", nullable: true }),
  },
});

function commentable(options: { optional?: boolean } = {}) {
  return oneOf("commentable", comment, { post: ["post", post], photo: ["photo", photo] }, options);
}

function refused(pattern?: RegExp): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof ConstructionError, `expected a ConstructionError, got ${error}`);
    if (pattern) assert.match(error.message, pattern);
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

// [spec:pgorm:req:bindings.one-of+1/test]    a reference declared over models sends nothing, describes itself under the declared keys, and writes the CHECK pgorm's shape writes: at most one target where it is optional, exactly one otherwise
test("a declaration describes itself and its CHECK", () => {
  const optional = commentable({ optional: true });
  assert.deepStrictEqual(optional.describe(), {
    name: "commentable",
    model: "comment",
    targets: [
      { value: "post", field: "post", target: "post" },
      { value: "photo", field: "photo", target: "photo" },
    ],
    optional: true,
  });
  const check = createOneOf(optional);
  assert.ok(check instanceof AlterTable);
  assert.equal(
    check.inspect().sql,
    `ALTER TABLE "${SCHEMA}"."comment" ADD CONSTRAINT "ck-comment-commentable" CHECK (num_nonnulls("post_id", "photo_id") <= 1)`,
  );
  assert.match(createOneOf(commentable()).inspect().sql, /= 1\)$/);
});

// [spec:pgorm:req:bindings.one-of+1/test]    what the derive refuses is a ConstructionError thrown as the reference is declared: a field the model lacks, one not nullable, one field for two targets, one of another kind than its target's key, a target keyed by more than one field, one target model twice, no target and a name past 59 bytes; an argument of the wrong type is a TypeError
test("a declaration refuses what the derive refuses", () => {
  const pair = model("pair", {
    schema: SCHEMA,
    columns: { a: column("i32", { primaryKey: true }), b: column("i32", { primaryKey: true }) },
  });
  const declare = (targets: Record<string, unknown>) => oneOf("commentable", comment, targets as never);
  assert.throws(() => declare({ post: ["missing", post] }), refused(/no field "missing"/));
  assert.throws(() => declare({ post: ["id", post] }), refused(/is not nullable/));
  assert.throws(() => declare({ post: ["post", post], photo: ["post", photo] }), refused(/holds the keys/));
  assert.throws(() => declare({ photo: ["post", photo] }), refused(/the key of/));
  assert.throws(() => declare({ pair: ["post", pair] }), refused(/keyed by 2 columns/));
  assert.throws(() => declare({ post: ["post", post], again: ["photo", post] }), refused(/is the target of/));
  assert.throws(() => declare({}), refused(/names no target/));
  assert.throws(() => oneOf("n".repeat(60), comment, { post: ["post", post] }), refused());
  assert.throws(() => oneOf(1 as never, comment, { post: ["post", post] }), TypeError);
  assert.throws(() => oneOf("c", "comment" as never, { post: ["post", post] }), TypeError);
  assert.throws(() => declare({ post: ["post"] }), mistyped(/\[field, model\]/));
  assert.throws(() => oneOf("c", comment, { post: ["post", post] }, { optional: 1 } as never), mistyped(/is a boolean/));
  assert.throws(() => oneOf("c", comment, { post: ["post", post] }, { optinal: true } as never), mistyped(/no one-of option/));
  assert.throws(() => createOneOf(comment as never), TypeError);
});

// [spec:pgorm:req:bindings.one-of+1/test]    set gives the fields a write sets, its target's field to the key and every other to null, all null for no target only where the reference is optional; an unknown target, a target without its key and no target where one is required are ConstructionErrors
test("set gives the fields a write sets", () => {
  const optional = commentable({ optional: true });
  assert.deepStrictEqual(optional.set("photo", 2n), { post: null, photo: 2n });
  assert.deepStrictEqual(optional.set(null), { post: null, photo: null });
  assert.throws(() => commentable().set(null), refused(/is not optional/));
  assert.throws(() => optional.set("video" as never, 1), refused(/`video` is no target/));
  assert.throws(() => optional.set("post"), refused(/holds its key/));
  assert.throws(() => optional.set(1 as never, 2), TypeError);
});

let database: { pool: Pool; drop(): Promise<void> } | undefined;

function pool(): Pool {
  if (!database) throw new Error("the scratch database was not created");
  return database.pool;
}

before(async () => {
  const scratch = await scratchDatabase("pgorm_napi_one_of");
  const opened = new Pool(scratch.dsn, { maxSize: 2 });
  database = {
    pool: opened,
    async drop() {
      await opened.close();
      await scratch.drop();
    },
  };
});

beforeEach(async () => {
  for (const sql of [
    `DROP SCHEMA IF EXISTS ${SCHEMA} CASCADE`,
    `CREATE SCHEMA ${SCHEMA}`,
    `CREATE TABLE ${SCHEMA}.post (id integer PRIMARY KEY, title text NOT NULL)`,
    `CREATE TABLE ${SCHEMA}.photo (id bigint PRIMARY KEY, url text NOT NULL)`,
    `CREATE TABLE ${SCHEMA}.comment (id integer PRIMARY KEY, body text NOT NULL, ` +
    `post_id integer REFERENCES ${SCHEMA}.post (id), photo_id bigint REFERENCES ${SCHEMA}.photo (id))`,
  ]) await pool().execute(sql);
  await pool().execute(createOneOf(commentable({ optional: true })));
  for (const at of [1, 2]) {
    await post.insert({ id: at, title: `post ${at}` }).execute(pool());
    await photo.insert({ id: BigInt(at), url: `${at}.png` }).execute(pool());
  }
});

after(async () => {
  await database?.drop();
});

const reference = commentable({ optional: true });

async function commentOn(id: number, body: string, value: "post" | "photo" | null, key: unknown = null) {
  return await comment.insert({ id, body, ...reference.set(value, key) } as never).returning().one(pool());
}

// [spec:pgorm:req:bindings.one-of+1/test]    the CHECK the shape writes holds every row on the server: a row referencing two targets is refused, one referencing none is taken where the reference is optional
test("the CHECK holds every row", async () => {
  await commentOn(1, "loose", null);
  await assert.rejects(pool().execute(`INSERT INTO ${SCHEMA}.comment VALUES (2, 'both', 1, 1)`), (error: unknown) => {
    assert.ok(error instanceof CheckViolationError, `expected a CheckViolationError, got ${error}`);
    assert.equal(error.constraint, "ck-comment-commentable");
    return true;
  });
});

// [spec:pgorm:req:bindings.one-of+1/test]    each reference written through set reads back as its tagged key, null for no target; a row setting two fields is a DecodeError naming the table, its key and the fields
test("set and key round trip", async () => {
  const written = [await commentOn(1, "a", "photo", 2n), await commentOn(2, "b", "post", 1), await commentOn(3, "c", null)];
  assert.deepStrictEqual(written.map((record) => reference.key(record)), [["photo", 2n], ["post", 1], null]);
  const rewritten = await comment.update(reference.set("post", 2) as never).where(comment.col("id").eq(1)).returning().one(pool());
  assert.deepStrictEqual(reference.key(rewritten), ["post", 2]);
  assert.equal(rewritten.photo, null);
  await pool().execute(`ALTER TABLE ${SCHEMA}.comment DROP CONSTRAINT "ck-comment-commentable"`);
  await pool().execute(`INSERT INTO ${SCHEMA}.comment VALUES (9, 'both', 1, 1)`);
  const both = await comment.findByKey({ id: 9 }).one(pool());
  assert.throws(
    () => reference.key(both),
    undecodable(/the `one_of.comment` row whose key is \(`id` = 9\) references more than one target, setting `post_id`, `photo_id`/),
  );
});

// [spec:pgorm:req:bindings.one-of+1/test]    load gives each record its target's record, tagged, in order, null for no target, reading each target's table once; a key its table does not hold is a DecodeError naming the table, the row's key, the target's key and the field
test("load reads each target", async () => {
  for (const [id, value, key] of [[1, "photo", 2n], [2, "post", 1], [3, null, null], [4, "post", 2], [5, "photo", 1n]] as const) {
    await commentOn(id, String(id), value, key);
  }
  const records = await comment.find().orderBy(comment.col("id").asc()).all(pool());
  assert.deepStrictEqual(await reference.load(pool(), records), [
    ["photo", { id: 2n, url: "2.png" }],
    ["post", { id: 1, title: "post 1" }],
    null,
    ["post", { id: 2, title: "post 2" }],
    ["photo", { id: 1n, url: "1.png" }],
  ]);
  assert.deepStrictEqual(await reference.load(pool(), []), []);
  await pool().execute(`ALTER TABLE ${SCHEMA}.comment DROP CONSTRAINT comment_post_id_fkey`);
  await pool().execute(`INSERT INTO ${SCHEMA}.comment VALUES (7, 'dangling', 9, NULL)`);
  const dangling = await comment.findByKey({ id: 7 }).one(pool());
  await assert.rejects(
    reference.load(pool(), [dangling]),
    undecodable(/references the `one_of.post` row of key 9 through `post_id`, and `one_of.post` holds no such row/),
  );
  await assert.rejects(reference.load(pool(), records[0] as never), TypeError);
});
