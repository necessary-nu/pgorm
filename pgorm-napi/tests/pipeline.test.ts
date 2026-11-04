// pgorm's pipeline composed from JavaScript against a live server, in either
// runtime: its stages and the rows they give, the two routes a value takes
// into the SQL, the binder's scope, and what is refused before anything is
// sent.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import {
  col as statementCol,
  ConstructionError,
  DatabaseError,
  Decimal,
  DecodeError,
  Interval,
  LifecycleError,
  pipeline as pl,
  Pool,
  Table,
  TypeName,
  Value,
} from "../lib/index.js";
import { mistyped, scratchDatabase } from "./support.ts";

let database: { dsn: string; drop(): Promise<void> } | undefined;
let shared: Pool | undefined;

function pool(): Pool {
  if (!shared) throw new Error("the scratch database was not created");
  return shared;
}

const items = new Table("items", { schema: "app" });
const base = pl.from(items);
const id = pl.col("items", "id");
const category = pl.col("items", "category");
const amount = pl.col("items", "amount");
const total = pl.alias("total");

before(async () => {
  database = await scratchDatabase("pgorm_napi_pipeline");
  shared = new Pool(database.dsn, { maxSize: 4 });
  for (
    const sql of [
      "CREATE SCHEMA app",
      "CREATE TABLE app.items (id int8 PRIMARY KEY, category text, amount int4)",
      `INSERT INTO app.items VALUES (1, 'fruit', 5), (2, 'fruit', 7), (3, 'veg', 2), (4, 'veg', 9),
        (5, 'O''Brien; -- \\', 4), (6, NULL, 1)`,
      "CREATE TYPE app.mood AS ENUM ('sad', 'calm', 'glad')",
      "CREATE DOMAIN app.cheerful AS app.mood",
      "CREATE TABLE app.lines (id int8 PRIMARY KEY, mood app.cheerful NOT NULL)",
      "INSERT INTO app.lines VALUES (1, 'calm'), (2, 'glad'), (3, 'calm'), (4, 'glad')",
      "CREATE TABLE app.tills (id int8 PRIMARY KEY, takings money NOT NULL)",
      "INSERT INTO app.tills VALUES (1, 12.35), (2, 3)",
    ]
  ) {
    await pool().execute(sql);
  }
});

after(async () => {
  await shared?.close();
  await database?.drop();
});

function refused(kind: typeof ConstructionError | typeof LifecycleError, pattern: RegExp): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof kind, `expected a ${kind.name}, got ${error}`);
    assert.match(error.message, pattern);
    return true;
  };
}

const lifecycle = (pattern: RegExp) => refused(LifecycleError, pattern);
const construction = (pattern: RegExp) => refused(ConstructionError, pattern);

// [spec:pgorm:req:bindings.pipeline/test]
test("a pipeline's stages run as any statement does, through each terminal", async () => {
  const totals = base.filter(category.isNotNull()).group(category).aggregate(pl.sum(amount).as(total))
    .filter(total.gt(5)).sort(total.desc());
  assert.deepStrictEqual(await pool().query(totals), [{ category: "fruit", total: 12n }, { category: "veg", total: 11n }]);
  assert.deepStrictEqual(await pool().one(totals.take(1)), { category: "fruit", total: 12n });
  assert.equal(await pool().optional(totals.filter(total.gt(100))), null);
  await assert.rejects(pool().one(totals), (error) => error instanceof DecodeError);
  const streamed: unknown[] = [];
  for await (const row of pool().stream(base.select(id).sort(id).takeRange(2, 3))) streamed.push(row.id);
  assert.deepStrictEqual(streamed, [2n, 3n]);
  const doubled = base.derive(amount.mul(2).as("doubled")).filter(pl.alias("doubled").gte(14)).select(id);
  assert.deepStrictEqual((await pool().query(doubled)).map((row) => row.id), [2n, 4n]);
  assert.equal(await pool().transaction((tx) => tx.execute(base.select(id))), 6);
});

// [spec:pgorm:req:bindings.pipeline-expressions+3/test]
// [spec:pgorm:req:bindings.operands/test]
test("a value an operand is given is bound; only literal() writes one into the SQL", async () => {
  const awkward = "O'Brien; -- \\";
  const bound = base.filter(category.eq(awkward)).select(id);
  const written = base.filter(category.eq(pl.literal(awkward))).select(id);
  assert.equal(bound.inspect().sql, "SELECT id FROM app.items WHERE category = $1");
  assert.deepStrictEqual(bound.inspect().values.map((value) => [value.kind, value.value]), [["text", awkward]]);
  assert.match(written.inspect().sql, /WHERE category = E'O''Brien; -- \\\\'$/);
  assert.deepStrictEqual(written.inspect().values, []);
  for (const query of [bound, written]) assert.deepStrictEqual(await pool().query(query), [{ id: 5n }]);
  const declared = base.filter(id.eq(new Value(3, "i16"))).select(amount);
  assert.equal(declared.inspect().values[0]?.kind, "i16");
  assert.deepStrictEqual(await pool().query(declared), [{ amount: 2 }]);
  const listed = base.filter(category.inArray(["veg", pl.literal("fruit")]).and(amount.lt(6))).select(id).sort(id);
  assert.deepStrictEqual((await pool().query(listed)).map((row) => row.id), [1n, 3n]);
  const sized = base.select(id, pl.caseWhen([[amount.gt(6), "big"]], pl.literal("small")).as("size")).sort(id).take(2);
  assert.deepStrictEqual(await pool().query(sized), [{ id: 1n, size: "small" }, { id: 2n, size: "big" }]);
  const cast = base.filter(id.eq(1)).select(amount.cast("text").as("spelled"), amount.coalesce(0).neg().as("negated"));
  assert.deepStrictEqual(await pool().query(cast), [{ spelled: "5", negated: -5 }]);
});

// [spec:pgorm:req:bindings.pipeline-expressions+3/test]    a column whose domain is over an enum,
// read with no registration, compares as the schema-qualified enum comparesAs names, bound, listed,
// coalesced, through an alias and in max, where each left bare is refused with 42883
test("a column whose domain is over an enum compares as the enum it is named as", async () => {
  const lines = pl.from(new Table("lines", { schema: "app" }));
  const mood = pl.col("lines", "mood");
  const lineId = pl.col("lines", "id");
  const typed = mood.comparesAs(new TypeName("mood", { schema: "app" }));
  const ids = async (pipeline: pl.Pipeline) => (await pool().query(pipeline.select(lineId).sort(lineId))).map((row) => row.id);
  assert.equal(
    lines.filter(typed.eq("glad")).inspect().sql,
    "SELECT * FROM app.lines WHERE CAST(mood AS app.mood) = CAST($1::text AS app.mood)",
  );
  assert.deepStrictEqual(await ids(lines.filter(typed.eq("glad"))), [2n, 4n]);
  assert.deepStrictEqual(await ids(lines.filter(typed.inArray(["calm", pl.literal("sad")]))), [1n, 3n]);
  assert.deepStrictEqual(await ids(lines.filter(typed.coalesce("glad").lt("glad"))), [1n, 3n]);
  const m = pl.alias("m");
  const named = m.comparesAs("mood");
  assert.equal(
    lines.derive(mood.as(m)).filter(named.eq(pl.literal("glad"))).inspect().sql,
    "WITH table_0 AS (SELECT *, mood AS m FROM app.lines) SELECT * FROM table_0 WHERE CAST(m AS mood) = CAST('glad' AS mood)",
  );
  const highest = await pool().one(lines.group().aggregate(pl.max(typed).cast("text").as("highest")));
  assert.deepStrictEqual(highest, { highest: "glad" });
  const undefinedOperator = (error: unknown) => {
    assert.ok(error instanceof DatabaseError, `expected a DatabaseError, got ${error}`);
    assert.equal(error.sqlstate, "42883");
    return true;
  };
  await assert.rejects(ids(lines.filter(mood.eq("glad"))), undefinedOperator);
  await assert.rejects(pool().query(lines.derive(mood.as(m)).filter(m.eq("glad"))), undefinedOperator);
  assert.throws(() => mood.comparesAs(5 as unknown as string), mistyped(/a TypeName/));
});

// [spec:pgorm:req:bindings.pipeline-expressions+3/test]    a money column read with no registration
// compares with a Decimal, bound or listed, as the money comparesAs names, where left bare the
// Decimal cannot be bound to it
test("a money column compares with a Decimal as the money it is named as", async () => {
  const tills = pl.from(new Table("tills", { schema: "app" }));
  const tillId = pl.col("tills", "id");
  const takings = pl.col("tills", "takings").comparesAs("money");
  const ids = async (pipeline: pl.Pipeline) => (await pool().query(pipeline.select(tillId).sort(tillId))).map((row) => row.id);
  assert.equal(
    tills.filter(takings.gt(new Decimal("5"))).inspect().sql,
    "SELECT * FROM app.tills WHERE CAST(takings AS money) > CAST($1::numeric AS money)",
  );
  assert.deepStrictEqual(await ids(tills.filter(takings.gt(new Decimal("5")))), [1n]);
  assert.deepStrictEqual(await ids(tills.filterWith((binder) => takings.lte(binder.bind(new Decimal("3"))))), [2n]);
  assert.deepStrictEqual(await ids(tills.filter(takings.inArray([new Decimal("3"), new Decimal("12.35")]))), [1n, 2n]);
  await assert.rejects(ids(tills.filter(pl.col("tills", "takings").gt(new Decimal("5")))), ConstructionError);
});

// [spec:pgorm:req:bindings.pipeline-expressions+3/test]
test("an interval binds in a pipeline as any value does", async () => {
  const span = new Interval({ months: 1, days: -2, microseconds: 3_000_004n });
  const selected = base.filter(id.eq(1)).selectWith((binder) => [
    binder.bind(span).cast("interval").as("span"),
    binder.bind(Temporal.Duration.from({ days: 1, hours: 2 })).cast("interval").as("duration"),
  ]);
  assert.deepStrictEqual(selected.inspect().values.map((value) => value.kind), ["i64", "interval", "interval"]);
  assert.deepStrictEqual(await pool().query(selected), [
    { span, duration: new Interval({ days: 1, microseconds: 7_200_000_000n }) },
  ]);
});

// [spec:pgorm:req:bindings.pipeline/test]
test("a window computes over its partition, ordering and frame", async () => {
  const position = pl.alias("position");
  const ranked = base.filter(category.isNotNull()).window(
    pl.over().by(category).sortBy(amount.desc()),
    pl.rowNumber().as(position),
  ).filter(position.lte(1)).select(category, amount).sort(amount);
  assert.deepStrictEqual(await pool().query(ranked), [
    { category: "O'Brien; -- \\", amount: 4 },
    { category: "fruit", amount: 7 },
    { category: "veg", amount: 9 },
  ]);
  const running = base.window(pl.over().sortBy(id).rows(-1, 0), pl.sum(amount).as("pair")).select(id, pl.alias("pair"))
    .take(3);
  assert.deepStrictEqual(await pool().query(running), [{ id: 1n, pair: 5n }, { id: 2n, pair: 12n }, { id: 3n, pair: 9n }]);
  const shifted = base.window(pl.over().sortBy(id), pl.lag(1, amount).as("before")).filter(id.lte(2)).select(
    pl.alias("before"),
  );
  assert.deepStrictEqual(await pool().query(shifted), [{ before: null }, { before: 5 }]);
});

// [spec:pgorm:req:bindings.pipeline/test]
test("joins read named sources and embedded pipelines, whose values travel with them", async () => {
  const peer = base.join(pl.source(items).named("peer"), category.eq(pl.col("peer", "category")).and(
    id.lt(pl.col("peer", "id")),
  )).select(id, pl.col("peer", "id").as("peer_id")).sort(id);
  assert.deepStrictEqual(await pool().query(peer), [{ id: 1n, peer_id: 2n }, { id: 3n, peer_id: 4n }]);
  const heavy = base.filterWith((binder) => amount.gt(binder.bind(new Value(4, "i32")))).select(id, category);
  const joined = base.joinWith(pl.source(heavy).named("heavy"), (binder) =>
    id.eq(pl.col("heavy", "id")).and(category.ne(binder.bind("veg"))), { kind: "inner" }).select(id).sort(id);
  assert.deepStrictEqual(joined.inspect().values.map((value) => value.value), ["veg", 4]);
  assert.deepStrictEqual((await pool().query(joined)).map((row) => row.id), [1n, 2n]);
  const outer = pl.from(heavy).filter(pl.alias("id").gt(1)).select(pl.alias("id")).sort(pl.alias("id"));
  assert.deepStrictEqual((await pool().query(outer)).map((row) => row.id), [2n, 4n]);
  const left = base.join(new Table("items", { schema: "app", alias: "other" }), id.eq(pl.col("other", "id").add(10)), {
    kind: "left",
  }).select(id, pl.col("other", "id").as("other_id")).sort(id).take(1);
  assert.deepStrictEqual(await pool().query(left), [{ id: 1n, other_id: null }]);
});

// [spec:pgorm:req:bindings.pipeline/test]
test("set operations append, intersect and remove rows, and distinct keeps one of each", async () => {
  const low = base.filter(amount.lt(6)).select(id);
  const fruit = base.filter(category.eq("fruit")).select(id);
  const ids = async (query: pl.Pipeline) => (await pool().query(query)).map((row) => row.id).sort();
  assert.deepStrictEqual(await ids(low.append(fruit)), [1n, 1n, 2n, 3n, 5n, 6n]);
  assert.deepStrictEqual(await ids(low.intersect(fruit)), [1n]);
  assert.deepStrictEqual(await ids(low.remove(fruit)), [3n, 5n, 6n]);
  assert.deepStrictEqual(
    (await pool().query(base.select(category).distinct())).map((row) => row.category).sort(),
    ["O'Brien; -- \\", "fruit", "veg", null].sort(),
  );
});

// [spec:pgorm:req:bindings.pipeline-binder/test]
test("a binder binds while its function runs, one placeholder per bind, reusable in its stage", async () => {
  const both = base.filterWith((binder) => {
    const pivot = binder.bind(new Value(3, "i64"));
    return id.gte(pivot).and(amount.gt(pivot));
  }).select(id).sort(id);
  assert.equal(both.inspect().sql, "SELECT id FROM app.items WHERE id >= $1 AND amount > $1 ORDER BY id");
  assert.deepStrictEqual((await pool().query(both)).map((row) => row.id), [4n, 5n]);
  const listed = base.selectWith((binder) => [id, binder.bind("tag").as("tag")]).filter(id.eq(6));
  assert.deepStrictEqual(await pool().query(listed), [{ id: 6n, tag: "tag" }]);
  const many = base.selectWith((binder) => Array.from({ length: 32 }, (_, at) => binder.bind(at).as(`c${at}`)));
  assert.equal(many.inspect().values.length, 32);
  assert.throws(
    () => base.selectWith((binder) => Array.from({ length: 33 }, (_, at) => binder.bind(at).as(`c${at}`))),
    construction(/at most 32/),
  );
  assert.equal(base.select(...Array.from({ length: 40 }, (_, at) => pl.literal(at).as(`c${at}`))).inspect().values.length, 0);
  const grouped = base.groupWith((binder) => [category.coalesce(binder.bind("none"))]).aggregateWith((binder) => [
    pl.countRows().add(binder.bind(100)).as("n"),
  ]);
  assert.deepStrictEqual(
    (await pool().query(grouped)).map((row) => Object.values(row)).sort(),
    [["O'Brien; -- \\", 101n], ["fruit", 102n], ["none", 101n], ["veg", 102n]].sort(),
  );
});

// [spec:pgorm:req:bindings.pipeline-binder/test]
test("a placeholder used outside the stage its binder's function returns it to is a LifecycleError", () => {
  let saved: pl.Binder | undefined;
  let placeholder: pl.PipelineExpr | undefined;
  const kept = base.filterWith((binder) => {
    saved = binder;
    placeholder = binder.bind(2);
    return id.gt(placeholder);
  });
  assert.equal(kept.inspect().values.length, 1);
  assert.ok(saved && placeholder);
  const [binder, value] = [saved, placeholder];
  assert.throws(() => binder.bind(3), lifecycle(/only while its function runs/));
  assert.throws(() => value.add(1), lifecycle(/function has returned/));
  assert.throws(() => base.filter(value), lifecycle(/not a plain stage/));
  assert.throws(() => base.filterWith(() => value), lifecycle(/another function's stage/));
  assert.throws(() => pl.from(new Table("other")).sortWith(() => [value]), lifecycle(/another pipeline's/));
  base.filterWith((outer) => {
    const mine = outer.bind(1);
    assert.throws(() => base.filterWith((inner) => id.gt(mine).and(id.lt(inner.bind(3)))), lifecycle(/two binders/));
    assert.throws(() => base.filterWith(() => mine), lifecycle(/another function's stage/));
    assert.throws(() => pl.over().by(mine), lifecycle(/a window/));
    assert.throws(() => base.window(pl.over(), mine.as("x")), lifecycle(/not a plain stage/));
    return id.gt(mine);
  });
  let thrown: pl.Binder | undefined;
  assert.throws(() =>
    base.filterWith((binder) => {
      thrown = binder;
      throw new RangeError("the function's own failure");
    }), RangeError);
  assert.throws(() => thrown?.bind(1), lifecycle(/only while its function runs/));
});

// [spec:pgorm:req:bindings.pipeline-binder/test]
// [spec:pgorm:req:bindings.argument-errors+1/test]
test("shapes a pipeline cannot take are refused as they are built", () => {
  assert.throws(() => base.filterWith((async () => id.gt(1)) as never), TypeError);
  assert.throws(() => base.deriveWith((() => "column") as never), TypeError);
  assert.throws(() => base.filterWith(42 as never), TypeError);
  assert.throws(() => base.filter(null as never), construction(/null has no kind/));
  assert.throws(() => base.filter(category.eq(new Value("calm", new TypeName("mood")))), construction(/enum/));
  assert.throws(() => base.filter(statementCol("id") as never), construction(/expected a pipeline expression/));
  assert.throws(() => pl.literal(Number.NaN), construction(/a literal number is finite/));
  assert.throws(() => pl.literal({} as never), mistyped(/a literal is/));
  assert.throws(() => pl.literal(2n ** 63n), construction(/within PostgreSQL's bigint/));
  assert.throws(() => amount.cast("money" as never), construction(/casts to one of/));
  assert.throws(() => amount.cast(7 as never), mistyped(/casts to one of/));
  assert.throws(() => pl.over().by(amount.add(1)), construction(/take no value/));
  assert.throws(() => base.take(1.5), construction(/row count/));
  assert.throws(() => pl.from({} as never), mistyped(/a pipeline reads/));
  assert.throws(() => base.join(items, id, { kind: "outer" as never }), mistyped(/join's kind/));
  assert.throws(() => pl.col("", "id"), construction(/1–63/));
  assert.throws(() => pl.col("items", 7 as never), mistyped(/an identifier is a string/));
  assert.throws(() => pl.lag(1.5, amount), construction(/offset/));
  assert.throws(() => pl.lag("1" as never, amount), mistyped(/offset is a number/));
});

// [spec:pgorm:req:bindings.pipeline/test]
test("what pgorm's compile step judges is a ConstructionError before anything is sent", async () => {
  await assert.rejects(pool().query(base.select(pl.literal(1).as("sum"))), construction(/PRQL built-in/));
  assert.throws(() => base.select(pl.col("items", 'a"b')).inspect(), construction(/a"b/));
  const grouped = base.group(category);
  assert.equal("inspect" in grouped, false);
  await assert.rejects(pool().query(grouped as never), construction(/needs aggregate/));
  await assert.rejects(pool().query(pl.over() as never), mistyped(/window is not a statement/));
  await assert.rejects(pool().execute(id as never), mistyped(/pipeline expression is not a statement/));
});
