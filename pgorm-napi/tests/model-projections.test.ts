// A declared model's query clauses, projections, partial models and cursor
// secondary orderings, against a live server in either runtime: each still
// decodes typed rows, and a row that breaks a declaration is a DecodeError.

import assert from "node:assert/strict";
import { after, before, beforeEach, test } from "node:test";

import {
  call,
  column,
  Condition,
  ConstructionError,
  DatabaseError,
  DecodeError,
  fromExpr,
  partialModel,
  type Pool,
  select,
  skip,
} from "../lib/index.js";
import { Account, author, modelDatabase, mood, Post, posts, Tag, truncate } from "./model-fixtures.ts";

let database: { pool: Pool; drop(): Promise<void> } | undefined;

function pool(): Pool {
  if (!database) throw new Error("the scratch database was not created");
  return database.pool;
}

before(async () => {
  database = await modelDatabase("pgorm_napi_projections");
});

beforeEach(async () => {
  await truncate(pool());
  await Account.insertMany([
    { name: "Ann", displayName: "A" },
    { name: "Bob", displayName: null },
    { name: "Cy", displayName: null },
  ]).execute(pool());
  await Post.insertMany([
    { id: 10, authorId: 1, editorId: null, title: "a1" },
    { id: 11, authorId: 1, editorId: null, title: "a2" },
    { id: 12, authorId: 2, editorId: null, title: "b1" },
    { id: 13, authorId: 1, editorId: null, title: "a3" },
  ]).execute(pool());
});

after(async () => {
  await database?.drop();
});

const postsCounted = () => call("count", Post.col("id"));

function refused(pattern: RegExp): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof ConstructionError, `expected a ConstructionError, got ${error}`);
    assert.match(error.message, pattern);
    return true;
  };
}

// [spec:pgorm:req:bindings.distinct-on/test]
test("the statement builder's DISTINCT ON takes columns", () => {
  const query = select(Post.col("authorId"), Post.col("title")).from(Post.table);
  assert.equal(
    query.distinctOn(Post.col("authorId")).orderBy(Post.col("authorId").asc()).inspect().sql,
    `SELECT DISTINCT ON ("post"."author_id") "post"."author_id", "post"."title" FROM "app"."post" ORDER BY "post"."author_id" ASC`,
  );
  assert.equal(
    query.distinctOn(Post.col("authorId")).distinct().inspect().sql,
    `SELECT DISTINCT "post"."author_id", "post"."title" FROM "app"."post"`,
  );
  assert.throws(() => query.distinctOn(Post.col("authorId").add(1)), refused(/takes columns/));
  // @ts-expect-error: the builder's DISTINCT ON takes a non-empty list.
  assert.throws(() => query.distinctOn(), refused(/at least one column/));
});

// [spec:pgorm:req:bindings.condition-option/test]
test("addOption adds a condition that is there, and leaves the condition as it is otherwise", () => {
  const base = Condition.all(Post.col("authorId").eq(1));
  assert.equal(base.addOption(null).inspect().sql, base.inspect().sql);
  assert.equal(base.addOption(undefined).inspect().sql, base.inspect().sql);
  assert.equal(base.addOption(Post.col("title").eq("a")).inspect().sql, base.add(Post.col("title").eq("a")).inspect().sql);
  // @ts-expect-error: an option is a predicate or nothing.
  assert.throws(() => base.addOption(1), TypeError);
});

// [spec:pgorm:req:bindings.model-projections/test]
test("a projection refuses what it cannot read", () => {
  const empty = Account.find().selectOnly();
  assert.throws(() => empty.intoTuple(), refused(/at least one item/));
  assert.throws(() => empty.inspect(), refused(/at least one item/));
  assert.throws(() => empty.column(Account.col("name")).columnAs(Account.col("id"), "name"), refused(/named "name"/));
  assert.throws(
    () => empty.column(Account.col("displayName")).exprAs(call("upper", Account.col("name")), "display_name", column("text")),
    refused(/read the column "display_name"/),
  );
  // @ts-expect-error: a projection's column is a model's column, not any expression.
  assert.throws(() => empty.column(call("upper", Account.col("name"))), TypeError);
  // @ts-expect-error: an expression's declaration is a column().
  assert.throws(() => empty.exprAs(call("count", Account.col("id")), "n", "i64"), TypeError);
  assert.throws(
    () => empty.exprAs(call("count", Account.col("id")), "n", column("i64", { primaryKey: true })),
    refused(/how it decodes/),
  );
  const named = empty.column(Account.col("name"));
  // @ts-expect-error: the projection has no item "id".
  assert.throws(() => named.intoValues("id"), refused(/no item "id"/));
  assert.throws(() => named.intoValues("name", "name"), refused(/twice/));
  // @ts-expect-error: intoModel takes a function of the row.
  assert.throws(() => named.intoModel(1), TypeError);
  // A field projects under its column's name, as the model's query does, and is keyed by field.
  assert.equal(
    empty.column(Account.col("displayName")).inspect().sql,
    `SELECT "account"."display_name" FROM "app"."account"`,
  );
});

// [spec:pgorm:req:bindings.partial-models/test]
test("a partial model declares its fields", () => {
  // @ts-expect-error: a partial model's field is a model column, a fromExpr() or a skip().
  assert.throws(() => partialModel(Account, { id: "id" }), TypeError);
  assert.throws(() => partialModel(Account, { cached: skip(0) }), refused(/at least one field/));
  assert.throws(() => fromExpr(Account.col("name"), column("text", { name: "x" })), refused(/how it decodes/));
  const line = partialModel(Account, { id: Account.col("id") });
  assert.throws(() => Post.find().intoPartialModel(line), refused(/reads account, and the query post/));
  assert.equal(line.model, Account);
});

// [spec:pgorm:req:bindings.model-clauses/test]
test("a model query's clauses refuse what the builder refuses", () => {
  // @ts-expect-error: a lock's strength is one of the four.
  assert.throws(() => Account.find().lock("exclusive"), TypeError);
  // @ts-expect-error: a model query's DISTINCT ON takes a non-empty list too.
  assert.throws(() => Account.find().distinctOn(), refused(/at least one column/));
  assert.throws(() => Account.find().distinctOn(Account.col("id").add(1)), refused(/takes columns/));
  assert.throws(() => Tag.find().joinRev(author), refused(/ends at account, which the query does not read/));
  // @ts-expect-error: a model join's kind is inner, left, right or full.
  assert.throws(() => Post.find().join(author, { kind: "cross" }), TypeError);
});

// [spec:pgorm:req:bindings.cursor-secondary-order/test]
test("a secondary ordering names a table the cursor's query reads", () => {
  const cursor = Post.find().cursor("authorId");
  assert.throws(() => cursor.addSecondaryOrderBy(Account, "id"), refused(/reads no table named account/));
  // @ts-expect-error: the model has no field "nope".
  assert.throws(() => cursor.addSecondaryOrderBy(Post, "nope"), refused(/no field "nope"/));
  const ordered = cursor.addSecondaryOrderBy(Post, "id");
  assert.throws(() => ordered.afterWith(1, 2, 3), refused(/1 or 2 key column/));
  // @ts-expect-error: after takes the key alone.
  assert.throws(() => ordered.after(1, 2), refused(/does not match 1 key column/));
});

// [spec:pgorm:req:bindings.model-clauses/test]
test("clauses keep the model's records", async () => {
  const distinct = Account.select("id").join(posts).distinct().orderBy(Account.col("id").asc());
  assert.deepStrictEqual((await distinct.all(pool())).map((row) => row.id), [1n, 2n]);
  const latest = Post.select("authorId", "title")
    .distinctOn(Post.col("authorId"))
    .orderBy(Post.col("authorId").asc(), Post.col("id").desc());
  assert.deepStrictEqual(await latest.all(pool()), [{ authorId: 1n, title: "a3" }, { authorId: 2n, title: "b1" }]);
  const busy = Account.select("name").join(posts).groupBy(Account.col("id")).having(postsCounted().gt(1));
  assert.deepStrictEqual(await busy.all(pool()), [{ name: "Ann" }]);
  assert.equal(await busy.count(pool()), 1);
});

// [spec:pgorm:req:bindings.model-clauses/test]
test("a locked query reads in a transaction, and another worker passes over its rows", async () => {
  const queue = Post.select("id").orderBy(Post.col("id").asc()).limit(1).lock("update", { wait: "skipLocked" });
  assert.match(queue.inspect().sql, /LIMIT \$1 FOR UPDATE SKIP LOCKED$/);
  await pool().transaction(async (tx) => {
    assert.deepStrictEqual(await queue.one(tx), { id: 10n });
    await pool().transaction(async (other) => {
      assert.deepStrictEqual(await queue.one(other), { id: 11n });
    });
    const shared = Post.select("id").where(Post.col("id").eq(10)).lock("share", { wait: "nowait" });
    await assert.rejects(pool().transaction(async (other) => await shared.one(other)), DatabaseError);
  });
});

// [spec:pgorm:req:bindings.model-clauses/test]
test("right and reverse joins", async () => {
  const tallies = Post.find()
    .join(author, { kind: "right" })
    .selectOnly()
    .column(Account.col("name"))
    .exprAs(postsCounted(), "posts", column("i64"))
    .groupBy(Account.col("name"))
    .orderBy(Account.col("name").asc());
  const rows: [string, bigint][] = await tallies.intoTuple().all(pool());
  assert.deepStrictEqual(rows, [["Ann", 3n], ["Bob", 1n], ["Cy", 0n]]);
  // Cy has no post, so the right join's row holds NULL in each post field.
  await assert.rejects(Post.find().join(author, { kind: "right" }).all(pool()), DecodeError);
  const writers = Account.select("name").joinRev(author).where(Post.col("title").eq("b1"));
  assert.equal(
    writers.inspect().sql,
    `SELECT "account"."name" FROM "app"."account" INNER JOIN "app"."post" ON "post"."author_id" = "account"."id" WHERE "post"."title" = $1`,
  );
  assert.deepStrictEqual(await writers.all(pool()), [{ name: "Bob" }]);
});

// [spec:pgorm:req:bindings.model-projections/test]
test("projections decode typed rows", async () => {
  const byAuthor = Post.find()
    .join(author)
    .selectOnly()
    .columnAs(Account.col("name"), "name")
    .exprAs(postsCounted(), "posts", column("i64"))
    .exprAs(call("max", Post.col("title")), "last", column("text", { nullable: true }))
    .groupBy(Account.col("name"))
    .orderBy(Account.col("name").asc());
  const tuples: [string, bigint, string | null][] = await byAuthor.intoTuple().all(pool());
  assert.deepStrictEqual(tuples, [["Ann", 3n, "a3"], ["Bob", 1n, "b1"]]);
  const values: [bigint, string][] = await byAuthor.intoValues("posts", "name").all(pool());
  assert.deepStrictEqual(values, [[3n, "Ann"], [1n, "Bob"]]);
  const objects: { name: string; posts: bigint; last: string | null }[] = await byAuthor.intoModel().all(pool());
  assert.deepStrictEqual(objects, [{ name: "Ann", posts: 3n, last: "a3" }, { name: "Bob", posts: 1n, last: "b1" }]);
  const made = await byAuthor.intoModel((row) => `${row.name}:${row.posts}`).all(pool());
  assert.deepStrictEqual(made, ["Ann:3", "Bob:1"]);

  const names = Account.select("name", "displayName").orderBy(Account.col("id").asc());
  const pairs: [string, string | null][] = await names.intoTuple().all(pool());
  assert.deepStrictEqual(pairs, [["Ann", "A"], ["Bob", null], ["Cy", null]]);
  assert.equal(await names.intoTuple().count(pool()), 3);
  assert.deepStrictEqual(await names.intoValues("name").paginate(2).fetchPage(pool(), 1), [["Cy"]]);
  const streamed: [string][] = [];
  for await (const row of names.intoValues("name").stream(pool())) streamed.push(row);
  assert.deepStrictEqual(streamed, [["Ann"], ["Bob"], ["Cy"]]);
  assert.deepStrictEqual(await names.intoValues("name").cursor("id").after(1n).first(1).all(pool()), [["Bob"]]);
  await assert.rejects(names.intoModel().optional(pool()), DecodeError);
});

// [spec:pgorm:req:bindings.model-projections/test]
test("a row that breaks its declaration is a DecodeError", async () => {
  const counted = Post.find().selectOnly().exprAs(postsCounted(), "posts", column("i32"));
  await assert.rejects(counted.intoTuple().one(pool()), DecodeError);
  const nothing = Post.find().where(Post.col("id").eq(99)).selectOnly();
  await assert.rejects(nothing.exprAs(call("max", Post.col("title")), "last", column("text")).intoTuple().one(pool()), DecodeError);
  const nullable = nothing.exprAs(call("max", Post.col("title")), "last", column("text", { nullable: true }));
  assert.deepStrictEqual(await nullable.intoTuple().one(pool()), [null]);
  await Account.update({ mood: "glad" }).where(Account.col("id").eq(1)).execute(pool());
  const moods = Account.find()
    .selectOnly()
    .exprAs(Account.col("mood"), "mood", column(mood, { nullable: true, values: ["calm"] }));
  await assert.rejects(moods.intoTuple().all(pool()), DecodeError);
});

// [spec:pgorm:req:bindings.partial-models/test]
test("a partial model selects and decodes its fields", async () => {
  const line = partialModel(Account, {
    id: Account.col("id"),
    shout: fromExpr(call("upper", Account.col("name")), column("text")),
    display: Account.col("displayName"),
    cached: skip("none"),
  });
  const query = Account.find().where(Account.col("id").lte(2)).orderBy(Account.col("id").asc());
  const read = query.intoPartialModel(line);
  assert.equal(
    read.inspect().sql,
    `SELECT "account"."id", UPPER("account"."name") AS "shout", "account"."display_name" AS "display" FROM "app"."account" WHERE "account"."id" <= $1 ORDER BY "account"."id" ASC`,
  );
  const rows: { id: bigint; shout: string; display: string | null; cached: string }[] = await read.all(pool());
  assert.deepStrictEqual(rows, [
    { id: 1n, shout: "ANN", display: "A", cached: "none" },
    { id: 2n, shout: "BOB", display: null, cached: "none" },
  ]);
  assert.deepStrictEqual(Object.keys(rows[0]!), ["id", "shout", "display", "cached"]);
  const strict = partialModel(Account, { shout: fromExpr(call("upper", Account.col("displayName")), column("text")) });
  await assert.rejects(query.intoPartialModel(strict).all(pool()), DecodeError);
});

// [spec:pgorm:req:bindings.cursor-secondary-order/test]
test("a secondary ordering breaks ties after the cursor's key", async () => {
  const byAuthor = Post.find().cursor("authorId").addSecondaryOrderBy(Post, "id");
  assert.deepStrictEqual((await byAuthor.first(2).all(pool())).map((row) => row.id), [10n, 11n]);
  // `after` takes the key alone and skips the rest of author 1's posts.
  assert.deepStrictEqual((await byAuthor.after(1).all(pool())).map((row) => row.id), [12n]);
  // `afterWith` takes the whole ordering and resumes inside the run.
  assert.deepStrictEqual((await byAuthor.afterWith(1, 11).first(2).all(pool())).map((row) => row.id), [13n, 12n]);
  assert.deepStrictEqual((await byAuthor.beforeWith(1, 13).last(1).all(pool())).map((row) => row.id), [11n]);
  const joined = Post.find()
    .join(author)
    .cursor("title")
    .addSecondaryOrderBy(Account, "name")
    .addSecondaryOrderBy(Post, "id");
  assert.equal(
    joined.afterWith("a1", "Ann", 10).first(1).inspect().sql,
    `SELECT "post"."id", "post"."author_id", "post"."editor_id", "post"."title" FROM "app"."post" INNER JOIN "app"."account" ON "post"."author_id" = "account"."id" WHERE ("post"."title" = $1 AND "account"."name" = $2 AND "post"."id" > $3) OR ("post"."title" = $4 AND "account"."name" > $5) OR "post"."title" > $6 ORDER BY "post"."title" ASC, "account"."name" ASC, "post"."id" ASC LIMIT $7`,
  );
  assert.deepStrictEqual((await joined.afterWith("a1", "Ann", 10).all(pool())).map((row) => row.id), [11n, 13n, 12n]);
  const graph = Post.graph().joinOne(author).cursor("title").addSecondaryOrderBy(Account, "name");
  assert.equal((await graph.first(1).all(pool()))[0]!.length, 2);
});
