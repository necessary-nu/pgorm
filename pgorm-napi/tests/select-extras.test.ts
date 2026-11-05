// SELECT's second tier against a live server, in either runtime: the tie
// limit and its refusals, GROUP BY DISTINCT over grouping elements, SIMILAR
// TO, set-returning functions read as relations, and TABLESAMPLE.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import {
  bind,
  call,
  col,
  ConstructionError,
  DataType,
  DatabaseError,
  exists,
  type Expr,
  GroupingElement,
  LikeExpr,
  likeExpr,
  Pool,
  type Select,
  select,
  Table,
  TableFunction,
  tableFunction,
  Value,
  With,
} from "../lib/index.js";
import { mistyped, scratchDatabase } from "./support.ts";

let database: { dsn: string; drop(): Promise<void> } | undefined;
let shared: Pool | undefined;

function pool(): Pool {
  if (!shared) throw new Error("the scratch database was not created");
  return shared;
}

const score = new Table("score");
const sale = new Table("sale");
const word = new Table("word");
const bag = new Table("bag");
const draw = new Table("draw");

before(async () => {
  database = await scratchDatabase("pgorm_napi_select_extras");
  shared = new Pool(database.dsn, { maxSize: 4 });
  for (
    const sql of [
      "CREATE TABLE score (id int8 PRIMARY KEY, player text NOT NULL, score int4 NOT NULL)",
      "INSERT INTO score VALUES (1, 'ann', 10), (2, 'bea', 9), (3, 'cal', 9), (4, 'dan', 8)",
      "CREATE TABLE sale (a int4 NOT NULL, b int4 NOT NULL)",
      "INSERT INTO sale VALUES (1, 2), (1, 3)",
      "CREATE TABLE word (w text NOT NULL)",
      "INSERT INTO word VALUES ('abc'), ('abd'), ('xyz'), ('a%c')",
      "CREATE TABLE bag (id int8 PRIMARY KEY, tags text[] NOT NULL)",
      "INSERT INTO bag VALUES (1, '{x,y}'), (2, '{z}')",
      "CREATE TABLE draw AS SELECT g::int8 AS id FROM generate_series(1, 1000) AS g",
    ]
  ) {
    await pool().execute(sql);
  }
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

function sqlstate(code: string): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof DatabaseError, `expected a DatabaseError, got ${error}`);
    assert.equal(error.sqlstate, code);
    return true;
  };
}

const field = (rows: Record<string, unknown>[], key: string) => rows.map((row) => row[key]);

/** A bound integer typed `int8`, as a set-returning function's overloads need. */
const int8 = (value: number) => bind(value).cast("int8");

const ranked = select(score.col("player"), score.col("score")).from(score);
const byScore = score.col("score").desc();
const unordered = ranked.limitWithTies(2);

// [spec:pgorm:req:bindings.select+4/test]
// [spec:pgorm:req:sql.ast.select.tie-limit+2/test]
test("a tie limit keeps every row tied with the last one counted, in LIMIT's slot", async () => {
  const tied = ranked.orderBy(byScore).limitWithTies(2);
  assert.deepStrictEqual(field(await pool().query(tied), "score"), [10, 9, 9]);
  assert.deepStrictEqual(field(await pool().query(ranked.orderBy(byScore).limit(2)), "score"), [10, 9]);
  assert.deepStrictEqual(field(await pool().query(tied.limit(2)), "score"), [10, 9]);
  assert.equal((await pool().query(tied.limit(null))).length, 4);
  assert.deepStrictEqual(field(await pool().query(unordered.orderBy(byScore)), "score"), [10, 9, 9]);
  assert.deepStrictEqual(field(await pool().query(tied.offset(1)), "score"), [9, 9]);
  const waiting = tied.lock("update", { wait: "nowait" });
  assert.deepStrictEqual(field(await pool().query(waiting), "score"), [10, 9, 9]);
  for (const count of [-1, 1.5]) {
    assert.throws(() => ranked.limitWithTies(count as never), refused(/row count/));
  }
  for (const count of [null, "2"]) {
    assert.throws(() => ranked.limitWithTies(count as never), mistyped(/row count/));
  }
});

// [spec:pgorm:req:bindings.select+4/test]
// [spec:pgorm:req:sql.ast.select.tie-limit+2/test]
test("a tie limit with no ORDER BY, or beside SKIP LOCKED, is refused by inspect() and every run before it is sent", async () => {
  const skipping = ranked.orderBy(byScore).limitWithTies(2).lock("update", { wait: "skipLocked" });
  for (const [statement, reason] of [[unordered, /needs an ORDER BY/], [skipping, /SKIP LOCKED/]] as const) {
    assert.throws(() => statement.inspect(), refused(reason));
    for (
      const run of [
        () => pool().execute(statement),
        () => pool().query(statement),
        () => pool().one(statement),
        () => pool().optional(statement),
        () => pool().stream(statement).next(),
      ]
    ) {
      await assert.rejects(run(), refused(reason));
    }
    await using connection = await pool().acquire();
    await assert.rejects(connection.query(statement), refused(reason));
    await assert.rejects(connection.stream(statement).next(), refused(reason));
    await connection.transaction(async (transaction) => {
      for (
        const run of [
          () => transaction.execute(statement),
          () => transaction.query(statement),
          () => transaction.one(statement),
          () => transaction.optional(statement),
        ]
      ) {
        await assert.rejects(run(), refused(reason));
      }
      // A statement the server had refused would have aborted the transaction.
      assert.equal((await transaction.query(ranked.orderBy(byScore).limitWithTies(2))).length, 3);
    });
  }
});

// [spec:pgorm:req:bindings.select+4/test]
// [spec:pgorm:req:sql.ast.select.tie-limit+2/test]
test("a refused tie limit cannot be read into another statement, where nothing could still order it", async () => {
  for (
    const embed of [
      () => exists(unordered),
      () => unordered.as("top"),
      () => ranked.union(unordered),
      () => new With("top", unordered),
      () => score.col("player").isIn(unordered.select(score.col("player"))),
    ]
  ) {
    assert.throws(embed, refused(/needs an ORDER BY/));
  }
  const top = ranked.orderBy(byScore).limitWithTies(2).as("top");
  assert.deepStrictEqual(field(await pool().query(select(top.col("player")).from(top)), "player").sort(), [
    "ann",
    "bea",
    "cal",
  ]);
});

// [spec:pgorm:req:bindings.select+4/test]
// [spec:pgorm:req:sql.ast.select.group-distinct/test]
// [spec:pgorm:def:sql.ast.select.grouping/test]
test("GROUP BY DISTINCT groups each grouping set its elements reach once", async () => {
  const grouped = (select: Select) =>
    select.orderBy(col("a").asc({ nulls: "last" }), col("b").asc({ nulls: "last" }));
  const totals = select(col("a"), col("b"), call("count", col("a")).as("n")).from(sale);
  const repeated = totals.groupBy(GroupingElement.rollup(col("a"), col("b")), GroupingElement.rollup(col("a")));
  assert.equal((await pool().query(repeated)).length, 8);
  assert.deepStrictEqual(await pool().query(grouped(repeated.groupByDistinct())), [
    { a: 1, b: 2, n: 1n },
    { a: 1, b: 3, n: 1n },
    { a: 1, b: null, n: 2n },
    { a: null, b: null, n: 2n },
  ]);
  assert.equal((await pool().query(totals.groupBy(GroupingElement.cube(col("a"), col("b"))))).length, 6);
  const sets = select(col("a"), call("count", col("a")).as("n")).from(sale)
    .groupBy(GroupingElement.sets(col("a"), GroupingElement.empty())).orderBy(col("a").asc({ nulls: "last" }));
  assert.deepStrictEqual(await pool().query(sets), [{ a: 1, n: 2n }, { a: null, n: 2n }]);
  const plain = totals.groupBy(col("a"), col("b"));
  assert.deepStrictEqual(await pool().query(grouped(plain.groupByDistinct())), await pool().query(grouped(plain)));
  for (
    const [build, reason] of [
      [() => totals.groupBy(1 as never), /grouping item is an expression or a grouping element/],
      [() => GroupingElement.sets(score as never), /grouping item is an expression or a grouping element, not a Table/],
      [() => GroupingElement.rollup("a" as never), /expected an expression/],
      [() => (GroupingElement.sets as () => unknown)(), /grouping item is an expression or a grouping element/],
      [() => new (GroupingElement as unknown as new () => unknown)(), /made by GroupingElement\.set\(\)/],
    ] as const
  ) {
    assert.throws(build, mistyped(reason));
  }
});

// [spec:pgorm:req:bindings.expressions+4/test]
// [spec:pgorm:req:sql.ast.expr.similar-to+3/test]
test("SIMILAR TO matches a whole string against its pattern, the pattern bound and its escape honoured", async () => {
  const words = async (predicate: Expr) =>
    field(await pool().query(select(word.col("w")).from(word).where(predicate)), "w").sort();
  assert.deepStrictEqual(await words(word.col("w").similarTo("a(b|x)%")), ["abc", "abd"]);
  assert.deepStrictEqual(await words(word.col("w").notSimilarTo("a(b|x)%")), ["a%c", "xyz"]);
  assert.deepStrictEqual(await words(word.col("w").similarTo("a%c")), ["a%c", "abc"]);
  assert.deepStrictEqual(await words(word.col("w").similarTo("a\\%c")), ["a%c"]);
  assert.deepStrictEqual(await words(word.col("w").similarTo(likeExpr("a!%c").escape("!"))), ["a%c"]);
  assert.deepStrictEqual(await words(word.col("w").notSimilarTo(likeExpr("a!%c").escape("!"))), ["abc", "abd", "xyz"]);
  assert.deepStrictEqual(await words(word.col("w").similarTo("ab")), []);
  const { sql, values } = word.col("w").similarTo("x' OR '1'='1").inspect();
  assert.equal(sql, 'SELECT "word"."w" SIMILAR TO $1');
  assert.deepStrictEqual(values.map((value) => value.value), ["x' OR '1'='1"]);
  assert.throws(() => likeExpr("a").escape("!!"), refused(/one character/));
  assert.throws(() => word.col("w").notSimilarTo(1 as never), mistyped(/pattern is a string/));
  assert.throws(() => likeExpr("a").escape(1 as never), mistyped(/escape is a string/));
});

// [spec:pgorm:req:bindings.expressions+4/test]
test("a pattern built from literal text matches the text and nothing its % or _ would widen it to", async () => {
  const text = (value: string) => bind(value).cast("text");
  const answers = await pool().query(
    select(
      text("save 50%_off now").like(LikeExpr.containing("50%_off")).as("inside"),
      text("save 50xyoff now").like(LikeExpr.containing("50%_off")).as("widened"),
      text("50%").like(LikeExpr.literal("50%")).as("exact"),
      text("A!B%c").ilike(LikeExpr.startingWith("a!b%").escape("!")).as("chosen"),
      text("x_y").like(LikeExpr.endingWith("_y")).as("tail"),
    ),
  );
  assert.deepStrictEqual(
    ["inside", "widened", "exact", "chosen", "tail"].map((name) => field(answers, name)[0]),
    [true, false, true, true, true],
  );
  assert.throws(() => LikeExpr.containing(1 as never), mistyped(/text is a string/));
});

// [spec:pgorm:req:bindings.select+4/test]
// [spec:pgorm:req:bindings.expressions+4/test]
// [spec:pgorm:req:sql.ast.select.table-function+2/test]
test("unnest WITH ORDINALITY numbers each element, reading a column of the item before it", async () => {
  const letters = tableFunction(call("unnest", bind(["x", "y", "z"]).cast("text", { array: true })))
    .withOrdinality()
    .columns("tag", "position")
    .alias("t");
  assert.deepStrictEqual(await pool().query(select().from(letters).orderBy(letters.col("position").asc())), [
    { tag: "x", position: 1n },
    { tag: "y", position: 2n },
    { tag: "z", position: 3n },
  ]);
  const tags = tableFunction(call("unnest", bag.col("tags"))).withOrdinality().columns("tag", "n").alias("t");
  const rows = await pool().query(
    select(bag.col("id"), tags.col("tag"), tags.col("n")).from(bag).crossJoin(tags)
      .orderBy(bag.col("id").asc(), tags.col("n").asc()),
  );
  assert.deepStrictEqual(rows, [{ id: 1n, tag: "x", n: 1n }, { id: 1n, tag: "y", n: 2n }, { id: 2n, tag: "z", n: 1n }]);
});

// [spec:pgorm:req:bindings.select+4/test]
// [spec:pgorm:req:bindings.expressions+4/test]
// [spec:pgorm:req:sql.ast.select.table-function+2/test]
test("ROWS FROM reads its calls side by side, padding the shorter with NULL", async () => {
  const zipped = tableFunction(call("generate_series", int8(1), int8(2)))
    .and(call("unnest", bind(["a", "b", "c"]).cast("text", { array: true })))
    .withOrdinality()
    .columns("x", "y", "n")
    .alias("r");
  assert.deepStrictEqual(await pool().query(select().from(zipped).orderBy(zipped.col("n").asc())), [
    { x: 1n, y: "a", n: 1n },
    { x: 2n, y: "b", n: 2n },
    { x: null, y: "c", n: 3n },
  ]);
  const stepped = TableFunction.rowsFrom(call("generate_series", int8(10), int8(1), int8(-3))).alias("s");
  assert.deepStrictEqual(field(await pool().query(select().from(stepped)), "s"), [10n, 7n, 4n, 1n]);
  const series = tableFunction(call("generate_series", int8(1), int8(3))).alias("g");
  assert.equal(
    select().from(series).inspect().sql,
    'SELECT * FROM GENERATE_SERIES(CAST($1::int8 AS int8), CAST($2::int8 AS int8)) AS "g"',
  );
  for (
    const [build, reason] of [
      [() => tableFunction(col("x")), /reads a function call, not an expression/],
      [() => TableFunction.rowsFrom(col("x")), /reads a function call, not an expression/],
      [() => tableFunction(call("generate_series", int8(1), int8(2))).and(col("x")), /reads a function call, not an expression/],
      [() => tableFunction(call("generate_series", int8(1), int8(2))).alias(""), /identifier/],
      [() => tableFunction(call("generate_series", int8(1), int8(2))).columns("a", ""), /identifier/],
      [() => call("generate_series", 1), /no "generate_series" taking 1 argument/],
      [() => call("unnest"), /no "unnest" taking 0 argument/],
    ] as const
  ) {
    assert.throws(build, refused(reason));
  }
  assert.throws(() => tableFunction(call("generate_series", int8(1), int8(2))).alias(undefined as never), mistyped(/identifier/));
  assert.throws(() => tableFunction(1 as never), mistyped(/reads a function call/));
  assert.throws(() => tableFunction(series as never), mistyped(/reads a function call, not a FROM item/));
  assert.throws(() => new (TableFunction as unknown as new () => unknown)(), mistyped(/made by tableFunction\(\)/));
  assert.throws(() => call(7 as never), mistyped(/a function's name is a string/));
});

// [spec:pgorm:req:bindings.select+4/test]
// [spec:pgorm:req:bindings.expressions+4/test]
// [spec:pgorm:req:sql.ast.select.column-definitions/test]
test("a call returning record reads through its column definitions, alone or inside ROWS FROM", async () => {
  const one = tableFunction(call("json_to_record", new Value({ a: 7, tags: ["x", "y"] }, "json")))
    .columnDefinition("a", "integer")
    .columnDefinition("tags", new DataType("text").array())
    .columnDefinition("missing", "text")
    .alias("r");
  const query = select(one.star()).from(one);
  assert.equal(
    query.inspect().sql,
    'SELECT "r".* FROM JSON_TO_RECORD($1) AS "r"("a" integer, "tags" text[], "missing" text)',
  );
  assert.deepStrictEqual(await pool().query(query), [{ a: 7, tags: ["x", "y"], missing: null }]);

  const zipped = tableFunction(call("jsonb_to_recordset", new Value([{ b: "x" }, { b: "y" }], "json")))
    .columnDefinition("b", "text")
    .and(call("generate_series", int8(10), int8(12)))
    .withOrdinality()
    .columns("b", "s", "n")
    .alias("z");
  const rows = await pool().query(select(zipped.star()).from(zipped).orderBy(zipped.col("n").asc()));
  assert.deepStrictEqual(rows, [
    { b: "x", s: 10n, n: 1n },
    { b: "y", s: 11n, n: 2n },
    { b: null, s: 12n, n: 3n },
  ]);
  const numbered = tableFunction(call("json_to_record", new Value({ a: 1 }, "json")))
    .columnDefinition("a", "integer")
    .withOrdinality()
    .alias("o");
  assert.equal(
    select().from(numbered).inspect().sql,
    'SELECT * FROM ROWS FROM (JSON_TO_RECORD($1) AS ("a" integer)) WITH ORDINALITY AS "o"',
  );
  assert.deepStrictEqual(await pool().query(select().from(numbered)), [{ a: 1, ordinality: 1n }]);

  const bare = tableFunction(call("json_to_record", new Value({ a: 1 }, "json"))).alias("b");
  await assert.rejects(pool().query(select().from(bare)), sqlstate("42601"));
  const typed = tableFunction(call("generate_series", int8(1), int8(2))).columnDefinition("a", "bigint").alias("g");
  await assert.rejects(pool().query(select().from(typed)), sqlstate("42601"));
  for (
    const [build, reason] of [
      [() => tableFunction(call("json_to_record", col("d"))).columnDefinition("", "text"), /identifier/],
      [() => tableFunction(call("json_to_record", col("d"))).columnDefinition("a", "no such type" as never), /no built-in type/],
      [() => call("json_to_record"), /no "json_to_record" taking 0 argument/],
    ] as const
  ) {
    assert.throws(build, refused(reason));
  }
  assert.throws(
    () => tableFunction(call("json_to_record", col("d"))).columnDefinition("a", 1 as never),
    mistyped(/a kind is a kind name/),
  );
});

// [spec:pgorm:req:bindings.select+4/test]
// [spec:pgorm:req:sql.ast.select.table-sample/test]
test("TABLESAMPLE draws a bound share of a table's rows, the same sample again for the same seed", async () => {
  const ids = async (sample: ReturnType<Table["tablesample"]>) =>
    field(await pool().query(select(sample.col("id")).from(sample).orderBy(sample.col("id").asc())), "id");
  assert.equal((await ids(draw.tablesample("bernoulli", 100))).length, 1000);
  assert.equal((await ids(draw.tablesample("system", 100))).length, 1000);
  assert.deepStrictEqual(await ids(draw.tablesample("bernoulli", 0)), []);
  const seeded = draw.as("d").tablesample("bernoulli", 50, { repeatable: 7 });
  const first = await ids(seeded);
  assert.ok(first.length > 0 && first.length < 1000, `${first.length} of 1000 drawn`);
  assert.deepStrictEqual(await ids(seeded), first);
  const bound = select().from(seeded).inspect().values;
  assert.deepStrictEqual(bound.map((value) => [value.kind, value.value]), [["f64", 50], ["f64", 7]]);
  await assert.rejects(pool().query(select().from(draw.tablesample("bernoulli", 150))), sqlstate("2202H"));
  assert.throws(() => draw.tablesample("random" as never, 10), mistyped(/bernoulli/));
  assert.throws(() => draw.tablesample("system", "10" as never), mistyped(/percentage is a number/));
  assert.throws(() => draw.tablesample("system", 10, { repeatable: "1" as never }), mistyped(/seed is a number/));
});
