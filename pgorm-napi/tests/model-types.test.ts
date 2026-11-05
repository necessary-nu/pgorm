// What the declarations make of a model's declaration, held by `deno check`:
// each assignment below type-checks only if a record, an insert, an update,
// a key, a graph's tuple, a cursor's boundary and a column's operands are
// typed from the columns alone, and each `@ts-expect-error` only if the types
// refuse what the binding refuses at run time. The tests themselves build the
// same statements, so the functions stay in step with the module they type.

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  col,
  column,
  ConstructionError,
  CreatedMultirange,
  CreatedRange,
  type Entity,
  type EntityTypes,
  type Expr,
  type InsertOf,
  type KeyOf,
  model,
  Multirange,
  type Pool,
  Range,
  type RowOf,
  TypeName,
  type UpdateOf,
} from "../lib/index.js";

const Account = model("account", {
  schema: "app",
  columns: {
    id: column("i64", { primaryKey: true, generated: "byDefault" }),
    name: column("text"),
    displayName: column("text", { name: "display_name", nullable: true }),
    mood: column(new TypeName("mood", { schema: "app" }), { values: ["calm", "glad"], nullable: true }),
    tags: column("text", { array: true }),
    span: column(new CreatedRange("floatrange", "f64", { schema: "app" }), { nullable: true }),
    created: column("datetime_utc", { default: true }),
    seq: column("i32", { generated: "always" }),
  },
});
const Post = model("post", {
  columns: { id: column("i64", { primaryKey: true }), authorId: column("i64", { name: "author_id" }), title: column("text") },
});
const author = Post.belongsTo(Account, { from: "authorId", to: "id" });
const posts = Account.hasMany(Post, { from: "id", to: "authorId" });

/** Never called: its body is what `deno check` holds the declarations to. */
async function typed(pool: Pool): Promise<unknown[]> {
  const record = null as unknown as RowOf<typeof Account>;
  const id: bigint = record.id;
  const mood: "calm" | "glad" | null = record.mood;
  const tags: (string | null)[] = record.tags;
  const span: Range<number> | null = record.span;
  const created: Temporal.Instant = record.created;
  const inserted: InsertOf<typeof Account> = { name: "x", tags: [], id: 5 };
  // @ts-expect-error: name is neither nullable, defaulted nor generated
  const missing: InsertOf<typeof Account> = { tags: [] };
  // @ts-expect-error: seq is generated always
  const generated: InsertOf<typeof Account> = { name: "x", tags: [], seq: 1 };
  // @ts-expect-error: wild is not one of mood's values
  const wild: UpdateOf<typeof Account> = { mood: "wild" };
  const key: KeyOf<typeof Account> = { id: 1 };
  // @ts-expect-error: no such field
  Post.belongsTo(Account, { from: "author", to: "id" });
  const picked = await Account.select("id", "name").one(pool);
  // @ts-expect-error: mood was not selected
  picked.mood;
  const [post, by, other] = (await Post.graph().joinOne(author).joinMaybe(posts, { alias: "p", from: 1 }).all(pool))[0]!;
  const title: string = post.title;
  const name: string = by.name;
  const maybe: string | undefined = other?.title;
  const grouped: string = (await Account.graph().joinMaybe(posts).allGrouped(pool))[0]![1][0]!.title;
  // @ts-expect-error: a slot's own fields only
  Post.graph().joinOne(author).col(1, "title");
  // @ts-expect-error: the boundary is an id and a name
  Account.find().cursor("id", "name").after(1n);
  const loaded: string | undefined = (await posts.load(pool, [record]))[0]![0]?.title;
  const owner: string | undefined = (await author.load(pool, [post]))[0]?.name;
  const change = await Account.update({ name: "z" }).where(Account.key({ id: 1n })).returningChange(pool);
  const before: string = change.old.name;
  const upserted = await Account.insert(inserted).returningUpsert(pool);
  const was: string | undefined = upserted?.kind === "updated" ? upserted.old.name : undefined;
  return [id, mood, tags, span, created, missing, generated, wild, key, title, name, maybe, grouped, loaded, owner, before, was];
}

// [spec:pgorm:req:napi.typing+1/test]
test("the declarations type a model's records, writes, keys, graphs and cursors from its columns", () => {
  assert.equal(typeof typed, "function");
  assert.match(Account.select("id", "name").inspect().sql, /SELECT "account"\."id", "account"\."name"/);
  assert.match(Post.graph().joinOne(author).joinMaybe(posts, { alias: "p", from: 1 }).inspect().sql, /"p"\."title" AS "s2_title"/);
  assert.equal(Account.find().cursor("id", "name").after(1n, "x").inspect().values.length, 3);
});

const Booking = model("booking", {
  schema: "app",
  columns: {
    id: column("i32", { primaryKey: true }),
    seats: column("int4range"),
    free: column("int4multirange", { nullable: true }),
    span: column(new CreatedRange("floatrange", "f64", { schema: "app" })),
    open: column(new CreatedMultirange("floatmultirange", "f64", { schema: "app" })),
  },
});
const seats = new Range(1, 4);
const span = new Range(1.5, 2.5);
const open = new Multirange([span]);

/** A registered entity's types, as a generated module declares a range column's. */
type Booked = EntityTypes<{ id: number; seats: Range<number> }, { id: number; seats: Range<number> }>;

/** Never called: its body is what `deno check` holds a column's operands to. */
function operands(registered: Entity<Booked>): Expr[] {
  return [
    Booking.col("seats").eq(seats),
    Booking.col("seats").isIn([seats, new Range(5, 8)]),
    Booking.col("seats").between(seats, new Range(5, 8)),
    Booking.col("free").contains(new Multirange([seats])),
    Booking.col("span").adjacentTo(span),
    Booking.col("span").isDistinctFrom(span),
    Booking.col("open").overlaps(open),
    Booking.col("open").eq(open),
    registered.col("seats").eq(seats),
    registered.col("seats").isNotIn([seats]),
    // @ts-expect-error: a scalar column takes no Range
    Booking.col("id").eq(seats),
    // @ts-expect-error: nor does its containment
    Booking.col("id").contains(open),
    // @ts-expect-error: a range column takes a Range, not a Multirange
    Booking.col("span").eq(open),
    // @ts-expect-error: a registered scalar column takes no Range
    registered.col("id").eq(seats),
    // @ts-expect-error: a registered column's containment binds as an expression's does, which infers no range's kind
    registered.col("seats").contains(seats),
    // @ts-expect-error: a free expression infers no range's kind
    col("seats").eq(seats),
  ];
}

// [spec:pgorm:req:napi.typing+1/test]
// [spec:pgorm:req:bindings.column-operators+2/test]
test("the declarations take a range or multirange column's Range or Multirange as its kind converts it", () => {
  assert.equal(typeof operands, "function");
  assert.equal(Booking.col("seats").isIn([seats]).inspect().values[0]?.kind, "int4range");
  assert.equal(Booking.col("free").contains(new Multirange([seats])).inspect().values[0]?.kind, "int4multirange");
  assert.match(Booking.col("span").adjacentTo(span).inspect().sql, /"span" -\|- CAST\(\$1::text AS app\.floatrange\)$/);
  assert.match(Booking.col("open").eq(open).inspect().sql, /"open" = CAST\(\$1::text AS app\.floatmultirange\)$/);
  assert.throws(() => Booking.col("id").eq(seats as never), ConstructionError);
  assert.throws(() => Booking.col("id").contains(open as never), ConstructionError);
  assert.throws(() => Booking.col("span").eq(open as never), ConstructionError);
  assert.throws(() => col("seats").eq(seats as never), ConstructionError);
});
