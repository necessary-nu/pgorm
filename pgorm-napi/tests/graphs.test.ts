// Relations, graphs, keyset cursors and pages over the models JavaScript
// declares, against a live server in either runtime: what each slot kind
// decodes, how a relation's far end is found and loaded, and how a cursor
// and a paginator walk the rows.

import assert from "node:assert/strict";
import { after, before, beforeEach, test } from "node:test";

import { column, ConstructionError, DecodeError, model, type Pool } from "../lib/index.js";
import {
  Account,
  author,
  editor,
  modelDatabase,
  Post,
  posts,
  PostTag,
  postTags,
  Tag,
  tagOf,
  truncate,
} from "./model-fixtures.ts";

let database: { pool: Pool; drop(): Promise<void> } | undefined;

function pool(): Pool {
  if (!database) throw new Error("the scratch database was not created");
  return database.pool;
}

before(async () => {
  database = await modelDatabase("pgorm_napi_graphs");
});

beforeEach(async () => {
  await truncate(pool());
  await Account.insertMany([{ name: "Ann" }, { name: "Bob" }, { name: "Cy" }]).execute(pool());
  await Post.insertMany([
    { id: 10, authorId: 1, editorId: 2, title: "a1" },
    { id: 11, authorId: 1, editorId: null, title: "a2" },
    { id: 12, authorId: 2, editorId: 999, title: "b1" },
  ]).execute(pool());
  await Tag.insertMany([{ id: 1, label: "red" }, { id: 2, label: "blue" }]).execute(pool());
  await PostTag.insertMany([{ postId: 10, tagId: 1 }, { postId: 10, tagId: 2 }, { postId: 12, tagId: 2 }]).execute(pool());
});

after(async () => {
  await database?.drop();
});

const byAccount = Account.col("id").asc();
const byPost = Post.col("id").asc();

function refused(pattern: RegExp): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof ConstructionError, `expected a ConstructionError, got ${error}`);
    assert.match(error.message, pattern);
    return true;
  };
}

// [spec:pgorm:req:bindings.relations+1/test]
test("a relation finds the rows at its far end, and none for a NULL key", async () => {
  const [ann] = await Account.find().orderBy(byAccount).limit(1).all(pool());
  assert.ok(ann);
  assert.deepStrictEqual((await posts.find(ann).orderBy(byPost).all(pool())).map((post) => post.title), ["a1", "a2"]);
  const a2 = await Post.findByKey({ id: 11 }).one(pool());
  assert.equal((await author.find(a2).one(pool())).name, "Ann");
  assert.deepStrictEqual(await editor.find(a2).all(pool()), []);
  assert.equal(posts.cardinality, "many");
  assert.equal(author.cardinality, "one");
  assert.deepStrictEqual([author.kind, author.fromFields, author.toFields], ["belongsTo", ["authorId"], ["id"]]);
  assert.equal(author.from, Post);
  assert.equal(author.to, Account);
});

// [spec:pgorm:req:bindings.relations+1/test]
test("load reads every row's far end in one query, a list for hasMany and a record or null otherwise", async () => {
  const accounts = await Account.find().orderBy(byAccount).all(pool());
  const loaded = await posts.load(pool(), accounts);
  assert.deepStrictEqual(loaded.map((list) => list.map((post) => post.title)), [["a1", "a2"], ["b1"], []]);
  const authored = await Post.find().orderBy(byPost).all(pool());
  assert.deepStrictEqual((await author.load(pool(), authored)).map((account) => account?.name), ["Ann", "Ann", "Bob"]);
  // A key nothing at the far end matches reads as null, as pgorm's load_one reads it.
  assert.deepStrictEqual((await editor.load(pool(), authored)).map((account) => account?.name ?? null), ["Bob", null, null]);
  assert.deepStrictEqual(await posts.load(pool(), []), []);
  // [spec:pgorm:req:bindings.argument-errors+1/test]    an option load does not know is refused, not left out
  await assert.rejects(posts.load(pool(), accounts, { sginal: undefined } as never), TypeError);
  const twice = Account.hasOne(Post, { from: "id", to: "authorId" });
  await assert.rejects(
    twice.load(pool(), accounts),
    (error: unknown) => error instanceof DecodeError && /2 post rows have the key/.test(error.message),
  );
});

// [spec:pgorm:req:bindings.relations+1/test]
test("a relation pairs as many fields at each end, each a field of its model", () => {
  assert.throws(() => Post.belongsTo(Account, { from: ["authorId", "id"], to: "id" } as never), refused(/as many fields/));
  assert.throws(() => Post.belongsTo(Account, { from: "author", to: "id" } as never), refused(/no field "author"/));
  assert.throws(() => Post.belongsTo(Account, { from: [], to: [] } as never), refused(/at least one field/));
  assert.throws(() => Post.belongsTo(Account, { from: 7, to: "id" } as never), TypeError);
  assert.throws(() => Post.belongsTo(Account, { from: [7], to: ["id"] } as never), TypeError);
  assert.throws(() => Post.belongsTo({} as never, { from: "authorId", to: "id" }), TypeError);
});

// [spec:pgorm:req:bindings.graphs+2/test]
test("joinOne drops a root its relation misses, joinMaybe keeps it with null", async () => {
  const required = await Post.graph().joinOne(editor).orderBy(byPost).all(pool());
  assert.deepStrictEqual(required.map(([post, account]) => [post.title, account.name]), [["a1", "Bob"]]);
  const optional = await Post.graph().joinMaybe(editor).orderBy(byPost).all(pool());
  assert.deepStrictEqual(optional.map(([post, account]) => [post.title, account?.name ?? null]), [
    ["a1", "Bob"],
    ["a2", null],
    ["b1", null],
  ]);
  const roots = await Account.graph().orderBy(byAccount).all(pool());
  assert.deepStrictEqual(roots.map((account) => account.name), ["Ann", "Bob", "Cy"]);
  assert.equal((await Post.graph().joinOne(author).where(Post.col("id").eq(12)).one(pool()))[1].name, "Bob");
  assert.equal(await Post.graph().joinOne(author).where(Post.col("id").eq(99)).optional(pool()), null);
});

// [spec:pgorm:req:bindings.graphs+2/test]
test("a graph reads through a junction it never decodes, and names a second table of one model by its alias", async () => {
  const tagged = await Post.graph().via(postTags).joinOne(tagOf).orderBy(byPost, Tag.col("id").asc()).all(pool());
  assert.deepStrictEqual(tagged.map(([post, tag]) => [post.id, tag.label]), [[10n, "red"], [10n, "blue"], [12n, "blue"]]);
  const both = await Post.graph().joinOne(author).joinMaybe(editor, { alias: "ed" }).orderBy(byPost).all(pool());
  assert.deepStrictEqual(both.map(([post, by, ed]) => [post.title, by.name, ed?.name ?? null]), [
    ["a1", "Ann", "Bob"],
    ["a2", "Ann", null],
    ["b1", "Bob", null],
  ]);
  const graph = Post.graph().joinOne(author).joinMaybe(editor, { alias: "ed" });
  assert.deepStrictEqual(
    (await graph.where(graph.col(2, "name").eq("Bob")).all(pool())).map(([post]) => post.title),
    ["a1"],
  );
  assert.throws(() => Post.graph().joinOne(author).joinMaybe(editor), refused(/already reads a table called "account"/));
  assert.throws(() => Account.graph().joinOne(tagOf), refused(/starts at post_tag, which the graph does not read/));
  assert.throws(() => Post.graph().joinOne(author).joinOne(posts, { alias: "p", from: 0 }), refused(/source 0 is post/));
  assert.throws(() => Post.graph().col(3 as never, "id"), refused(/no source 3/));
});

// [spec:pgorm:req:bindings.graphs+2/test]
test("a present slot that does not decode is a DecodeError, never an absent slot", async () => {
  await pool().execute("UPDATE app.account SET mood = 'wild' WHERE id = 1");
  await assert.rejects(
    Post.graph().joinMaybe(author).all(pool()),
    (error: unknown) => error instanceof DecodeError && /"wild" is not one of mood's values/.test(error.message),
  );
});

// [spec:pgorm:req:bindings.graphs+2/test]
test("allGrouped gathers each root's slot records, ordered by the graph and then the root's key", async () => {
  const grouped = await Account.graph().joinMaybe(posts).orderBy(Account.col("name").desc()).allGrouped(pool());
  assert.deepStrictEqual(grouped.map(([account, list]) => [account.name, list.map((post) => post.title)]), [
    ["Cy", []],
    ["Bob", ["b1"]],
    ["Ann", ["a1", "a2"]],
  ]);
  await assert.rejects(Account.graph().allGrouped(pool() as never), refused(/exactly one slot/));
  const keyless = model("account", { schema: "app", columns: { name: column("text") } });
  await assert.rejects(
    keyless.graph().joinMaybe(keyless.hasMany(Post, { from: "name", to: "title" })).allGrouped(pool()),
    refused(/no.* primary key|none of/),
  );
});

// [spec:pgorm:req:bindings.graphs+2/test]
test("a sliced graph refuses the reads that decide its rows themselves", async () => {
  const graph = Account.graph().joinMaybe(posts).orderBy(byAccount, byPost);
  for (const sliced of [graph.limit(2), graph.offset(1), graph.limit(null), graph.limit(2).where(Account.col("id").gt(0))]) {
    // @ts-expect-error a sliced graph has no allGrouped
    await assert.rejects(sliced.allGrouped(pool()), refused(/a graph cut by limit or offset has no allGrouped/));
    // @ts-expect-error a sliced graph has no count
    await assert.rejects(sliced.count(pool()), refused(/has no count/));
    // @ts-expect-error a sliced graph has no cursor
    assert.throws(() => sliced.cursor("name"), refused(/has no cursor/));
    // @ts-expect-error a sliced graph has no cursorOn
    assert.throws(() => sliced.cursorOn(1, "title"), refused(/has no cursorOn/));
    // @ts-expect-error a sliced graph has no paginate
    assert.throws(() => sliced.paginate(2), refused(/has no paginate/));
    assert.throws(() => sliced.inspect("count"), refused(/has no count/));
    assert.throws(() => sliced.inspect("allGrouped"), refused(/has no allGrouped/));
  }
  const rows = await graph.limit(2).all(pool());
  assert.deepStrictEqual(rows.map(([account, post]) => [account.name, post?.title]), [["Ann", "a1"], ["Ann", "a2"]]);
});

// [spec:pgorm:req:bindings.cursors+1/test]
test("a cursor walks a model's rows by keyset, forwards and backwards", async () => {
  await Account.insertMany([{ name: "Ann" }, { name: "Dee" }]).execute(pool());
  const cursor = Account.find().cursor("name", "id");
  const pages = [];
  let page = await cursor.first(2).all(pool());
  while (page.length > 0) {
    pages.push(page.map((account) => `${account.name}${account.id}`));
    const last = page[page.length - 1]!;
    page = await cursor.after(last.name, last.id).first(2).all(pool());
  }
  assert.deepStrictEqual(pages, [["Ann1", "Ann4"], ["Bob2", "Cy3"], ["Dee5"]]);
  assert.deepStrictEqual((await cursor.last(2).all(pool())).map((account) => account.name), ["Cy", "Dee"]);
  assert.deepStrictEqual((await cursor.desc().first(2).all(pool())).map((account) => account.name), ["Dee", "Cy"]);
  assert.deepStrictEqual((await cursor.before("Bob", 2n).all(pool())).map((account) => account.id), [1n, 4n]);
  assert.deepStrictEqual((await cursor.desc().before("Cy", 3n).all(pool())).map((account) => account.id), [5n]);
  assert.deepStrictEqual(await cursor.first(0).all(pool()), []);
  // @ts-expect-error: the boundary is a name and an id
  assert.throws(() => cursor.after("Ann"), refused(/boundary of 1 values does not match 2/));
  assert.throws(() => cursor.after("Ann", "x" as never), ConstructionError);
  assert.throws(() => cursor.first(-1), refused(/non-negative integer/));
  // @ts-expect-error: a cursor orders by a field
  assert.throws(() => Account.find().cursor(), refused(/at least one field/));
});

// [spec:pgorm:req:bindings.cursors+1/test]
test("a graph's cursor tiebreaks on each slot's key, and afterWith resumes inside a run of one root", async () => {
  const cursor = Account.graph().joinOne(posts).cursor("name");
  const first = await cursor.first(1).all(pool());
  assert.deepStrictEqual(first.map(([account, post]) => [account.name, post.title]), [["Ann", "a1"]]);
  const [account, post] = first[0]!;
  const resumed = await cursor.afterWith(account.name, account.id, post.id).first(5).all(pool());
  assert.deepStrictEqual(resumed.map(([, row]) => row.title), ["a2", "b1"]);
  const skipped = await cursor.after(account.name).first(5).all(pool());
  assert.deepStrictEqual(skipped.map(([, row]) => row.title), ["b1"]);
  assert.match(cursor.inspect().sql, /ORDER BY "account"\."name" ASC, "account"\."id" ASC, "post"\."id" ASC/);
  assert.throws(() => cursor.afterWith("Ann", 1n), refused(/1 or 3 key column/));
});

// [spec:pgorm:req:bindings.pagination/test]
test("a paginator reads pages by limit and offset and counts the rows it pages", async () => {
  const paginator = Account.find().orderBy(byAccount).limit(1).paginate(2);
  assert.equal(paginator.pageSize, 2);
  assert.deepStrictEqual(await paginator.numItemsAndPages(pool()), { items: 3, pages: 2 });
  assert.equal(await paginator.numPages(pool()), 2);
  assert.deepStrictEqual((await paginator.fetchPage(pool(), 1)).map((row) => row.name), ["Cy"]);
  assert.deepStrictEqual(await paginator.fetchPage(pool(), 5), []);
  const pages = [];
  for await (const page of paginator.pages(pool())) pages.push(page.map((row) => row.name));
  assert.deepStrictEqual(pages, [["Ann", "Bob"], ["Cy"]]);
  const graphPages = Account.graph().joinOne(posts).orderBy(byAccount, byPost).paginate(2);
  assert.equal(await graphPages.numItems(pool()), 3);
  assert.deepStrictEqual((await graphPages.fetchPage(pool(), 1)).map(([, row]) => row.title), ["b1"]);
  assert.equal(await Account.graph().joinOne(posts).count(pool()), 3);
  assert.throws(() => Account.find().paginate(0), refused(/at least 1/));
  assert.throws(() => paginator.inspect(Number.MAX_SAFE_INTEGER), refused(/past the offsets/));
  assert.throws(() => paginator.inspect(-1), refused(/non-negative integer/));
});

// [spec:pgorm:req:bindings.model-reads/test]
test("a model query joins a relation to filter on its far end without reading it", async () => {
  const titles = await Post.find().join(author).where(Account.col("name").eq("Bob")).all(pool());
  assert.deepStrictEqual(titles.map((post) => post.title), ["b1"]);
  const edited = await Post.find().join(editor, { kind: "left", alias: "ed" }).where(Account.as("ed").col("name").isNull())
    .orderBy(byPost).all(pool());
  assert.deepStrictEqual(edited.map((post) => post.title), ["a2", "b1"]);
  const tagged = await Post.select("title").join(postTags).join(tagOf).where(Tag.col("label").eq("red")).all(pool());
  assert.deepStrictEqual(tagged, [{ title: "a1" }]);
  assert.throws(() => Account.find().join(tagOf), refused(/starts at post_tag, which the query does not read/));
});
