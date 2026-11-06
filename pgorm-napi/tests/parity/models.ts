// The model family's parity cases: statements the models JavaScript declares
// build, held to `models.json`, which the Rust test `models::parity` builds
// with pgorm's own entities — `find`, `insert`, `update_many`,
// `delete_many`, `SelectGraph` — or, where pgorm composes a statement inside
// a terminal, with pgorm-query as that terminal does.
// [spec:pgorm:req:bindings.graphs+1/test]

import {
  call,
  column,
  type Compiled,
  Conflict,
  fromExpr,
  model,
  partialModel,
  skip,
  TypeName,
  Value,
} from "../../lib/index.js";

/** A column name long enough that a graph's prefixed alias is bounded. */
export const LONG = "column_with_a_name_long_enough_to_need_bounding_in_a_graph_xx";

const Account = model("account", {
  schema: "app",
  columns: {
    id: column("i64", { primaryKey: true, generated: "byDefault" }),
    name: column("text"),
    displayName: column("text", { name: "display_name", nullable: true }),
  },
});
const Post = model("post", {
  schema: "app",
  columns: {
    id: column("i64", { primaryKey: true }),
    authorId: column("i64", { name: "author_id" }),
    title: column("text"),
  },
});
const Tag = model("tag", {
  schema: "app",
  columns: { id: column("i32", { primaryKey: true }), label: column("text") },
});
const PostTag = model("post_tag", {
  schema: "app",
  columns: {
    postId: column("i64", { name: "post_id", primaryKey: true }),
    tagId: column("i32", { name: "tag_id", primaryKey: true }),
  },
});
const Version = model("version", {
  schema: "app",
  columns: { doc: column("i32", { primaryKey: true }), rev: column("i32", { primaryKey: true }), body: column("text") },
});
const Ticket = model("ticket", {
  columns: { id: column("i64", { primaryKey: true, generated: "byDefault" }), note: column("text", { nullable: true }) },
});
const Long = model("long_names", { columns: { id: column("i32", { primaryKey: true }), [LONG]: column("text") } });
const Moody = model("moody", {
  schema: "app",
  columns: { id: column("i32", { primaryKey: true }), mood: column(new TypeName("mood", { schema: "app" })) },
});

const author = Post.belongsTo(Account, { from: "authorId", to: "id" });
const posts = Account.hasMany(Post, { from: "id", to: "authorId" });
const postTags = Post.hasMany(PostTag, { from: "id", to: "postId" });
const tagOf = PostTag.belongsTo(Tag, { from: "tagId", to: "id" });

const postCount = () => call("count", Post.col("id"));
const AccountLine = partialModel(Account, {
  id: Account.col("id"),
  shout: fromExpr(call("upper", Account.col("name")), column("text")),
  display: Account.col("displayName"),
  note: skip(""),
});

export const cases: Record<string, () => Compiled> = {
  "find": () => Account.find().inspect(),
  "find-filtered": () =>
    Account.find().where(Account.col("id").gte(10)).orderBy(Account.col("name").desc()).limit(5).offset(10).inspect(),
  "select-fields": () => Account.select("id", "name").inspect(),
  "find-by-key": () => Account.findByKey({ id: 7 }).inspect(),
  "find-by-composite-key": () => Version.findByKey({ doc: 1, rev: 2 }).inspect(),
  "find-joined": () => Post.find().join(author).where(Account.col("name").eq("Ann")).inspect(),
  "enum-comparison": () => Moody.find().where(Moody.col("mood").eq("calm")).inspect(),
  "insert": () => Account.insert({ name: "Ann", displayName: null }).inspect(),
  "insert-many": () => Account.insertMany([{ name: "Ann" }, { name: "Bob" }]).inspect(),
  "insert-defaults": () => Ticket.insert({}).inspect(),
  "insert-returning": () => Account.insert({ displayName: "A", name: "Ann" }).returning("id", "name").inspect(),
  "update": () => Account.update({ name: "x" }).where(Account.key({ id: 1 })).inspect(),
  "delete": () => Account.delete().where(Account.col("name").eq("x")).inspect(),
  "update-changes": () => Account.update({ name: "x" }).where(Account.col("id").gte(2)).inspect("returningChanges"),
  "insert-upserts": () =>
    Account.insert({ id: 1, name: "Ann" }).onConflict(Conflict.on("id").update("name")).inspect("returningUpsert"),
  "relation-find": () => posts.find({ id: 3n }).inspect(),
  "graph": () => Post.graph().joinOne(author).inspect(),
  "graph-maybe-alias": () => Account.graph().joinMaybe(posts, { alias: "p" }).inspect(),
  "graph-via": () => Post.graph().via(postTags).joinMaybe(tagOf).inspect(),
  "graph-filtered": () =>
    Post.graph().joinOne(author).where(Post.col("title").eq("x")).orderBy(Post.col("id").asc()).inspect(),
  "graph-long-name": () => Long.graph().inspect(),
  "graph-grouped": () => Account.graph().joinMaybe(posts).orderBy(Account.col("name").desc()).inspect("allGrouped"),
  "cursor-after-first": () => Account.find().cursor("id").after(5).first(10).inspect(),
  "cursor-composite-last-desc": () => Account.find().cursor("name", "id").before("m", 4).last(3).desc().inspect(),
  "graph-cursor-with": () => Post.graph().joinMaybe(author).cursor("title").afterWith("t", 1, 2).first(2).inspect(),
  "page": () => Account.find().orderBy(Account.col("id").asc()).paginate(10).inspect(2),
  "count": () =>
    Account.find().where(Account.col("name").ne("x")).orderBy(Account.col("id").asc()).limit(5).inspect("count"),
  "find-distinct": () => Account.find().distinct().inspect(),
  "find-distinct-on": () =>
    Account.find().distinctOn(Account.col("name")).orderBy(Account.col("name").asc(), Account.col("id").asc()).inspect(),
  "find-grouped": () => Account.find().groupBy(Account.col("id")).having(Account.col("name").ne("x")).inspect(),
  "find-locked": () => Account.find().lock("update", { wait: "skipLocked" }).inspect(),
  "find-lock-shared": () => Account.find().lock("share").inspect(),
  "find-right-joined": () => Post.find().join(author, { kind: "right" }).inspect(),
  "find-joined-rev": () => Account.find().joinRev(author).where(Post.col("title").eq("x")).inspect(),
  "projection-columns": () =>
    Account.find().selectOnly().column(Account.col("id")).columnAs(Account.col("name"), "label").intoTuple().inspect(),
  "projection-grouped": () =>
    Post.find()
      .selectOnly()
      .column(Post.col("authorId"))
      .exprAs(postCount(), "posts", column("i64"))
      .groupBy(Post.col("authorId"))
      .having(postCount().gt(new Value(1n, "i64")))
      .intoTuple()
      .inspect(),
  "projection-joined": () =>
    Post.find()
      .join(author)
      .selectOnly()
      .column(Post.col("title"))
      .columnAs(Account.col("name"), "author")
      .intoTuple()
      .inspect(),
  "partial-model": () => Account.find().where(Account.col("id").eq(1)).intoPartialModel(AccountLine).inspect(),
  "cursor-secondary-after": () =>
    Account.find().cursor("name").addSecondaryOrderBy(Account, "id").after("m").first(3).inspect(),
  "cursor-secondary-after-with": () =>
    Account.find().cursor("name").addSecondaryOrderBy(Account, "id").afterWith("m", 4).first(3).inspect(),
};
