// Declared models following relations as pgorm's entities do, against a live
// server in either runtime: a relation's own condition, a path through a
// junction, a chain of relations, loads of a filtered and ordered query, a
// relation starting from an alias, a join narrowed in its ON, and cursors
// over a slot's fields.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { column, ConstructionError, DecodeError, model, Pool } from "../lib/index.js";
import { scratchDatabase } from "./support.ts";

const Account = model("account", {
  schema: "rel",
  columns: { id: column("i64", { primaryKey: true }), name: column("text"), active: column("bool") },
});
const Post = model("post", {
  schema: "rel",
  columns: {
    id: column("i64", { primaryKey: true }),
    authorId: column("i64", { name: "author_id" }),
    editorId: column("i64", { name: "editor_id", nullable: true }),
    title: column("text"),
    published: column("bool"),
  },
});
const Tag = model("tag", { schema: "rel", columns: { id: column("i32", { primaryKey: true }), label: column("text") } });
const PostTag = model("post_tag", {
  schema: "rel",
  columns: {
    postId: column("i64", { name: "post_id", primaryKey: true }),
    tagId: column("i32", { name: "tag_id", primaryKey: true }),
  },
});

const posts = Account.hasMany(Post, { from: "id", to: "authorId" });
const author = Post.belongsTo(Account, { from: "authorId", to: "id" });
const editor = Post.belongsTo(Account, { from: "editorId", to: "id" });
const postTags = Post.hasMany(PostTag, { from: "id", to: "postId" });
const tagOf = PostTag.belongsTo(Tag, { from: "tagId", to: "id" });
const tagPostTags = Tag.hasMany(PostTag, { from: "id", to: "tagId" });
const postTagPost = PostTag.belongsTo(Post, { from: "postId", to: "id" });
const published = Account.hasMany(Post, { from: "id", to: "authorId", onCondition: (_account, post) => post.col("published").eq(true) });
const activeAuthor = Post.belongsTo(Account, { from: "authorId", to: "id", onCondition: (_post, account) => account.col("active").eq(true) });
const tags = Post.related(tagOf, { via: postTags });

const SCHEMA = [
  "CREATE SCHEMA rel",
  "CREATE TABLE rel.account (id int8 PRIMARY KEY, name text NOT NULL, active bool NOT NULL)",
  `CREATE TABLE rel.post (
     id int8 PRIMARY KEY, author_id int8 NOT NULL REFERENCES rel.account,
     editor_id int8, title text NOT NULL, published bool NOT NULL
   )`,
  "CREATE TABLE rel.tag (id int4 PRIMARY KEY, label text NOT NULL)",
  "CREATE TABLE rel.post_tag (post_id int8 REFERENCES rel.post, tag_id int4 REFERENCES rel.tag, PRIMARY KEY (post_id, tag_id))",
];

let database: { pool: Pool; drop(): Promise<void> } | undefined;

function pool(): Pool {
  if (!database) throw new Error("the scratch database was not created");
  return database.pool;
}

before(async () => {
  const scratch = await scratchDatabase("pgorm_napi_relations");
  const created = new Pool(scratch.dsn, { maxSize: 4 });
  for (const sql of SCHEMA) await created.execute(sql);
  await Account.insertMany([
    { id: 1n, name: "Ann", active: true },
    { id: 2n, name: "Bob", active: false },
    { id: 3n, name: "Cy", active: true },
  ]).execute(created);
  await Post.insertMany([
    { id: 10n, authorId: 1n, editorId: 2n, title: "a1", published: true },
    { id: 11n, authorId: 1n, editorId: null, title: "a2", published: false },
    { id: 12n, authorId: 2n, editorId: 999n, title: "b1", published: true },
    { id: 13n, authorId: 1n, editorId: null, title: "a3", published: true },
  ]).execute(created);
  await Tag.insertMany([{ id: 1, label: "red" }, { id: 2, label: "blue" }]).execute(created);
  await PostTag.insertMany([{ postId: 10n, tagId: 1 }, { postId: 10n, tagId: 2 }, { postId: 12n, tagId: 2 }]).execute(created);
  database = {
    pool: created,
    async drop() {
      await created.close();
      await scratch.drop();
    },
  };
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

const titles = (rows: readonly { title: string }[]) => rows.map((row) => row.title);
const labels = (rows: readonly { label: string }[]) => rows.map((row) => row.label);

// [spec:pgorm:req:bindings.relations+2/test]    onCondition joins with the fields' equality, ANDed or ORed
test("a relation's own condition is part of every join of it, given the ends as the join names them", () => {
  assert.equal(
    Account.find().join(published).inspect().sql,
    `SELECT "account"."id", "account"."name", "account"."active" FROM "rel"."account" INNER JOIN "rel"."post" ON "account"."id" = "post"."author_id" AND "post"."published" = $1`,
  );
  const either = Account.hasMany(Post, {
    from: "id",
    to: "authorId",
    conditionType: "any",
    onCondition: (_account, post) => post.col("title").eq("b1"),
  });
  assert.match(Account.find().join(either).inspect().sql, /ON "account"\."id" = "post"\."author_id" OR "post"\."title" = \$1/);
  const aliased = Post.graph().joinOne(activeAuthor, { alias: "writer" }).inspect().sql;
  assert.match(aliased, /"post"\."author_id" = "writer"\."id" AND "writer"\."active" = \$1/);
  assert.equal(published.conditionType, "all");
  assert.equal(typeof published.onCondition, "function");
  assert.throws(() => Account.hasMany(Post, { from: "id", to: "authorId", onCondition: "published" as never }), TypeError);
  assert.throws(() => Account.hasMany(Post, { from: "id", to: "authorId", conditionType: "either" as never }), TypeError);
  const broken = Account.hasMany(Post, { from: "id", to: "authorId", onCondition: () => "yes" as never });
  assert.throws(() => Account.find().join(broken), /onCondition gives an expression/);
});

// [spec:pgorm:req:bindings.related/test]    find_related's SQL through the junction, and what a path refuses
test("a related path joins through its junction, as find_related writes it", () => {
  assert.equal(
    tags.find({ id: 10n }).inspect().sql,
    `SELECT "tag"."id", "tag"."label" FROM "rel"."tag" INNER JOIN "rel"."post_tag" ON "post_tag"."tag_id" = "tag"."id" INNER JOIN "rel"."post" ON "post"."id" = "post_tag"."post_id" WHERE "post"."id" = $1`,
  );
  assert.equal(tags.from, Post);
  assert.equal(tags.to, Tag);
  assert.equal(tags.via, postTags);
  assert.equal(tags.relation, tagOf);
  assert.throws(() => Tag.related(tagOf, { via: postTags } as never), refused(/via relation starts at post/));
  assert.throws(() => Post.related(author as never, { via: postTags }), refused(/where the via relation reaches post_tag/));
  assert.throws(
    // @ts-expect-error: a junction's relation to the far end reaches one row
    () => Post.related(PostTag.hasMany(Tag, { from: "tagId", to: "id" }), { via: postTags }),
    refused(/reaches one row/),
  );
  assert.match(
    Post.graph().relatedMaybe(tags).inspect().sql,
    /LEFT JOIN "rel"\."post_tag" ON "post"\."id" = "post_tag"\."post_id" LEFT JOIN "rel"\."tag" ON "post_tag"\."tag_id" = "tag"\."id"/,
  );
  assert.match(
    Post.graph().relatedOne(tags).inspect().sql,
    /INNER JOIN "rel"\."post_tag" ON "post"\."id" = "post_tag"\."post_id" INNER JOIN "rel"\."tag" ON "post_tag"\."tag_id" = "tag"\."id"/,
  );
  assert.equal(Post.graph().relatedOne(author).inspect().sql, Post.graph().joinOne(author).inspect().sql);
});

// [spec:pgorm:req:bindings.linked/test]    each hop under r{i}, walked from the far end, keyed by the start's key
test("a linked chain joins every hop under an alias of its own", () => {
  const chain = Account.linked(posts, postTags, tagOf);
  assert.equal(
    chain.find({ id: 1n }).inspect().sql,
    `SELECT "tag"."id", "tag"."label" FROM "rel"."tag" INNER JOIN "rel"."post_tag" AS "r0" ON "r0"."tag_id" = "tag"."id" INNER JOIN "rel"."post" AS "r1" ON "r1"."id" = "r0"."post_id" INNER JOIN "rel"."account" AS "r2" ON "r2"."id" = "r1"."author_id" WHERE "r2"."id" = $1`,
  );
  assert.deepStrictEqual([0, 1, 2].map((index) => (chain.hop(index) as { alias: string | null }).alias), ["r0", "r1", "r2"]);
  assert.equal(chain.lastHop().name, "account");
  assert.equal(chain.from, Account);
  assert.equal(chain.to, Tag);
  assert.throws(() => chain.hop(3), refused(/no hop 3/));
  assert.throws(() => Account.linked(posts, tagOf as never), refused(/relation 1 starts at post_tag, where the one before it reaches post/));
  assert.throws(() => Account.linked(postTags as never), refused(/first relation starts at post/));
  assert.throws(() => (Account.linked as (...relations: unknown[]) => unknown)(), ConstructionError);
});

// [spec:pgorm:req:bindings.relations+2/test]    fromAlias starts a join from the table answering to the alias
test("fromAlias names the table a join starts from", () => {
  const shared = Post.find()
    .join(postTags, { alias: "mine" })
    .join(tagOf.fromAlias("mine"))
    .join(tagPostTags, { alias: "theirs" })
    .join(postTagPost.fromAlias("theirs"), { alias: "other" });
  assert.match(shared.inspect().sql, /INNER JOIN "rel"\."post" AS "other" ON "theirs"\."post_id" = "other"\."id"/);
  assert.throws(() => Post.find().join(tagOf.fromAlias("nowhere")), refused(/reads no table called "nowhere"/));
  assert.throws(() => Post.find().join(tagOf.fromAlias("post")), refused(/"post" is post, where the relation starts at post_tag/));
  assert.throws(() => Post.graph().via(postTags, { alias: "mine" }).joinOne(tagOf.fromAlias("mine"), { from: 0 }), refused(/takes no from/));
});

// [spec:pgorm:req:bindings.relations+2/test]    find and load read a relation whole, its onCondition included
test("a relation's condition reads both ends in find and load", async () => {
  const accounts = await Account.find().orderBy(byAccount).all(pool());
  const ann = accounts[0]!;
  assert.deepStrictEqual(titles(await published.find(ann).orderBy(byPost).all(pool())), ["a1", "a3"]);
  assert.match(published.find(ann).inspect().sql, /WHERE "account"\."id" = \$2/);
  const loaded = await published.load(pool(), accounts, { query: Post.find().orderBy(byPost) });
  assert.deepStrictEqual(loaded.map(titles), [["a1", "a3"], ["b1"], []]);
  const all = await Post.find().orderBy(byPost).all(pool());
  assert.deepStrictEqual((await activeAuthor.load(pool(), all)).map((row) => row?.name ?? null), ["Ann", "Ann", null, "Ann"]);
  const either = Account.hasMany(Post, {
    from: "id",
    to: "authorId",
    conditionType: "any",
    onCondition: (_account, post) => post.col("title").eq("b1"),
  });
  const files = await either.load(pool(), accounts, { query: Post.find().orderBy(byPost) });
  assert.deepStrictEqual(files.map(titles), [["a1", "a2", "b1", "a3"], ["b1"], ["b1"]]);
});

// [spec:pgorm:req:bindings.relations+2/test]    a load takes a query of the far end, filtered and ordered
test("load reads a query of the far end, null where it matched nothing", async () => {
  const accounts = await Account.find().orderBy(byAccount).all(pool());
  const all = await Post.find().orderBy(byPost).all(pool());
  const newest = Post.find().where(Post.col("published").eq(true)).orderBy(Post.col("id").desc());
  assert.deepStrictEqual((await posts.load(pool(), accounts, { query: newest })).map(titles), [["a3", "a1"], ["b1"], []]);
  const partial = await posts.load(pool(), accounts, { query: Post.select("title").orderBy(byPost) });
  assert.deepStrictEqual(partial[1], [{ title: "b1" }]);
  const bobs = await author.load(pool(), all, { query: Account.find().where(Account.col("name").eq("Bob")) });
  assert.deepStrictEqual(bobs.map((row) => row?.name ?? null), [null, null, "Bob", null]);
  assert.deepStrictEqual((await editor.load(pool(), all)).map((row) => row?.name ?? null), ["Bob", null, null, null]);
  const twice = Account.hasOne(Post, { from: "id", to: "authorId" });
  await assert.rejects(
    twice.load(pool(), accounts),
    (error: unknown) => error instanceof DecodeError && /3 post rows have the key/.test(error.message),
  );
  await assert.rejects(posts.load(pool(), accounts, { query: Tag.find() as never }), refused(/load reads post, and the query reads tag/));
  await assert.rejects(posts.load(pool(), accounts, { query: Post as never }), TypeError);
  assert.deepStrictEqual(await posts.load(pool(), [], { query: newest }), []);
});

// [spec:pgorm:req:bindings.related/test]
test("a related path finds and loads through its junction", async () => {
  const all = await Post.find().orderBy(byPost).all(pool());
  assert.deepStrictEqual(labels(await tags.find(all[0]!).orderBy(Tag.col("label").asc()).all(pool())), ["blue", "red"]);
  const loaded = await tags.load(pool(), all, { query: Tag.find().orderBy(Tag.col("label").desc()) });
  assert.deepStrictEqual(loaded.map(labels), [["red", "blue"], [], ["blue"], []]);
  const rows = await Post.graph().relatedMaybe(tags).orderBy(byPost, Tag.col("id").asc()).all(pool());
  assert.deepStrictEqual(rows.map(([post, tag]) => [post.title, tag?.label ?? null]), [
    ["a1", "red"],
    ["a1", "blue"],
    ["a2", null],
    ["b1", "blue"],
    ["a3", null],
  ]);
  const required = await Post.graph().relatedOne(tags).orderBy(byPost, Tag.col("id").asc()).all(pool());
  assert.deepStrictEqual(required.map(([post, tag]) => [post.title, tag.label]), [["a1", "red"], ["a1", "blue"], ["b1", "blue"]]);
});

// [spec:pgorm:req:bindings.linked/test]
test("a linked chain reads along its hops, through one table twice", async () => {
  const chain = Account.linked(posts, postTags, tagOf);
  assert.deepStrictEqual(labels(await chain.find({ id: 1n }).orderBy(Tag.col("id").asc()).all(pool())), ["red", "blue"]);
  const hop = chain.hop(1) as typeof Post;
  assert.equal((await chain.find({ id: 1n }).where(hop.col("title").eq("a1")).all(pool())).length, 2);
  assert.deepStrictEqual(await chain.find({ id: null } as never).all(pool()), []);
  const neighbours = Tag.linked(tagPostTags, postTagPost, postTags, tagOf);
  assert.deepStrictEqual(labels(await neighbours.find({ id: 1 }).orderBy(Tag.col("id").asc()).all(pool())), ["red", "blue"]);
  assert.deepStrictEqual(labels(await chain.findLinked().all(pool())).sort(), ["blue", "blue", "red"]);
});

// [spec:pgorm:req:bindings.graphs+2/test]    a hop under an alias, the next relation from it, and a filtered slot
test("a graph joins from an aliased hop, and narrows a slot in its ON", async () => {
  const graph = Post.graph()
    .via(postTags)
    .via(tagOf)
    .via(tagPostTags, { alias: "shared" })
    .joinMaybe(postTagPost.fromAlias("shared"), { alias: "sibling" });
  const siblings = await graph
    .where(graph.col(1, "id").ne(graph.col(0, "id")))
    .orderBy(byPost, graph.col(1, "id").asc())
    .allGrouped(pool());
  assert.deepStrictEqual(siblings.map(([post, others]) => [post.title, titles(others)]), [["a1", ["b1"]], ["b1", ["a1"]]]);
  const filtered = await Account.graph()
    .joinMaybeFiltered(posts, (_account, post) => post.col("published").eq(true))
    .orderBy(byAccount, byPost)
    .allGrouped(pool());
  assert.deepStrictEqual(filtered.map(([account, rows]) => [account.name, titles(rows)]), [
    ["Ann", ["a1", "a3"]],
    ["Bob", ["b1"]],
    ["Cy", []],
  ]);
  assert.match(
    Account.graph().joinMaybeFiltered(posts, (_account, post) => post.col("published").eq(true)).inspect().sql,
    /LEFT JOIN "rel"\."post" ON "account"\."id" = "post"\."author_id" AND "post"\."published" = \$1/,
  );
  assert.throws(() => Account.graph().joinMaybeFiltered(posts, Post.col("published").eq(true) as never), TypeError);
});

// [spec:pgorm:req:bindings.cursors+2/test]    cursorOn orders by a slot; its key and tiebreaks
test("cursorOn pages a graph by a slot's field, the slot's key and then the root's breaking ties", async () => {
  const cursor = Post.graph().joinOne(author).cursorOn(1, "name");
  assert.ok(cursor.first(2).inspect().sql.endsWith(`ORDER BY "account"."name" ASC, "account"."id" ASC, "post"."id" ASC LIMIT $1`));
  const first = await cursor.first(2).all(pool());
  assert.deepStrictEqual(first.map(([post, account]) => [post.title, account.name]), [["a1", "Ann"], ["a2", "Ann"]]);
  assert.deepStrictEqual((await cursor.after("Ann").all(pool())).map(([post]) => post.title), ["b1"]);
  assert.deepStrictEqual((await cursor.afterWith("Ann", 1n, 11n).all(pool())).map(([post]) => post.title), ["a3", "b1"]);
  // @ts-expect-error: cursorOn orders by a slot, the first slot 1
  assert.throws(() => Post.graph().joinOne(author).cursorOn(0, "title"), refused(/cursor orders by the root's/));
  // @ts-expect-error: the graph has one slot
  assert.throws(() => Post.graph().joinOne(author).cursorOn(2, "name"), refused(/no source 2/));
});
