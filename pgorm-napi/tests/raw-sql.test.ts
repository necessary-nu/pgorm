// SQL text beyond its terminals — scripts, the grammar and PRQL checks, a
// transaction's stream, rows decoded into declared models, verification,
// pages over a written SELECT — and the retrying transaction, the refusal
// fields, functions by name, templates and the jsonb operators, against a
// live server in either runtime.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import {
  call,
  col,
  column,
  ConstructionError,
  DatabaseError,
  DecodeError,
  func,
  GroupingElement,
  LifecycleError,
  literal,
  model,
  PgormError,
  Pool,
  fromStatement,
  prql,
  select,
  sql,
  Table,
  tableFunction,
  template,
  type Transaction,
} from "../lib/index.js";
import { scratchDatabase } from "./support.ts";

let database: { dsn: string; drop(): Promise<void> } | undefined;
let shared: Pool | undefined;

function pool(): Pool {
  if (!shared) throw new Error("the scratch database was not created");
  return shared;
}

const Item = model("raw_items", {
  columns: {
    id: column("i32", { primaryKey: true }),
    label: column("text", { name: "name" }),
    price: column("decimal"),
  },
});

before(async () => {
  database = await scratchDatabase("pgorm_napi_raw_sql");
  shared = new Pool(database.dsn, { maxSize: 4 });
});

after(async () => {
  await shared?.close();
  await database?.drop();
});

async function fresh(): Promise<void> {
  await pool().batchExecute(
    "DROP TABLE IF EXISTS raw_items;" +
      "CREATE TABLE raw_items (id integer PRIMARY KEY, name text NOT NULL, price numeric NOT NULL);" +
      "INSERT INTO raw_items VALUES (1, 'one', 1.50), (2, 'two', 2.50), (3, 'three', 3.50)",
  );
}

// [spec:pgorm:req:bindings.batch-execute/test]
test("a script runs unprepared on a pool, a connection and a transaction", async () => {
  await fresh();
  await pool().connection(async (connection) => {
    await connection.batchExecute("UPDATE raw_items SET price = price + 1; UPDATE raw_items SET price = price * 2");
    await connection.transaction(async (tx) => {
      await tx.batchExecute("DELETE FROM raw_items WHERE id = 3; DELETE FROM raw_items WHERE id = 2");
      const { total } = await tx.one("SELECT sum(price) AS total FROM raw_items");
      assert.equal(String(total), "5.00");
    });
  });
  await assert.rejects(
    pool().batchExecute("SELECT 1; SELEC 2"),
    (error) => error instanceof DatabaseError && error.sqlstate === "42601",
  );
  // @ts-expect-error: a script is text
  await assert.rejects(pool().batchExecute(select()), TypeError);
});

// [spec:pgorm:req:bindings.sql-check/test]
test("sql holds text to the grammar and returns it unchanged", () => {
  const text = "SELECT name FROM raw_items WHERE id = $1";
  assert.equal(sql(text), text);
  assert.equal(sql("CREATE TABLE a (id int); INSERT INTO a VALUES (1)"), "CREATE TABLE a (id int); INSERT INTO a VALUES (1)");
  assert.throws(
    () => sql("SELECT id FROM cake\nORDER name"),
    (error) =>
      error instanceof ConstructionError &&
      error.message === 'PostgreSQL rejected this SQL: syntax error at or near "name"\n\n  ORDER name\n        ^',
  );
  // @ts-expect-error: SQL text is a string
  assert.throws(() => sql(1), TypeError);
});

// [spec:pgorm:req:bindings.prql-text/test]
test("prql compiles to SQL whose placeholders take the values", async () => {
  await fresh();
  const { sql: text, values } = prql("from raw_items | filter price < $1 | sort name | select {name, price}", [3]);
  assert.equal(text, "SELECT name, price FROM raw_items WHERE price < $1 ORDER BY name");
  assert.deepEqual(values, [3]);
  assert.deepEqual((await pool().query(text, values)).map((row) => row.name), ["one", "two"]);
  assert.throws(() => prql("from raw_items | filter price < $1"), /nothing binds \$1/);
  assert.throws(() => prql("from raw_items", [1]), /no \$1 in the query/);
  assert.throws(() => prql("from raw_items | filter id > $1 | filter id < $3", [1, 2]), /uses \$3 but never \$2/);
  assert.throws(() => prql("from raw_items | take $1", [1]), (error) => error instanceof ConstructionError);
});

// [spec:pgorm:req:bindings.streams+1/test]
test("a transaction streams, reserved until its last row", async () => {
  await fresh();
  await pool().transaction(async (tx) => {
    await tx.execute("INSERT INTO raw_items VALUES (4, 'four', 4.50)");
    const seen: number[] = [];
    for await (const row of tx.stream("SELECT id FROM raw_items ORDER BY id")) {
      seen.push(row.id as number);
      if (row.id === 1) await assert.rejects(tx.one("SELECT 1 AS one"), LifecycleError);
    }
    assert.deepEqual(seen, [1, 2, 3, 4]);
    const labels: string[] = [];
    for await (const record of Item.find().orderBy(Item.col("id").asc()).stream(tx)) labels.push(record.label);
    assert.deepEqual(labels, ["one", "two", "three", "four"]);
  });
  assert.equal((await pool().one("SELECT count(*)::int AS n FROM raw_items")).n, 4);

  await assert.rejects(
    pool().transaction(async (tx) => {
      await tx.execute("INSERT INTO raw_items VALUES (5, 'five', 5.50)");
      for await (const _ of tx.stream("SELECT generate_series(1, 100000) AS n")) break;
      await tx.one("SELECT 1 AS one");
    }),
    LifecycleError,
  );
  assert.equal((await pool().one("SELECT count(*)::int AS n FROM raw_items")).n, 4);
});

// [spec:pgorm:req:bindings.raw-models/test]
test("raw rows decode into a declared model's records", async () => {
  await fresh();
  const query = Item.findByStatement("SELECT *, 'extra' AS unread FROM raw_items WHERE price > $1 ORDER BY id", [2]);
  const records = await query.all(pool());
  assert.deepEqual(records.map((record) => [record.id, record.label]), [[2, "two"], [3, "three"]]);
  const one = await Item.findByStatement("SELECT * FROM raw_items WHERE id = $1", [1]).one(pool());
  assert.equal(String(one.price), "1.50");
  assert.equal(await Item.findByStatement("SELECT * FROM raw_items WHERE id = 9").optional(pool()), null);
  const streamed: number[] = [];
  for await (const record of query.stream(pool())) streamed.push(record.id);
  assert.deepEqual(streamed, [2, 3]);
  await assert.rejects(Item.findByStatement("SELECT id, name FROM raw_items").all(pool()), DecodeError);
  await assert.rejects(Item.findByStatement("SELECT id, name, price::text AS price FROM raw_items").all(pool()), DecodeError);
});

// [spec:pgorm:req:bindings.verify/test]
test("verify checks a model without running the statement", async () => {
  await fresh();
  await pool().verify("SELECT id, name, price FROM raw_items WHERE id = $1", Item);
  await assert.rejects(
    pool().verify("SELECT id, name FROM raw_items", Item),
    (error) =>
      error instanceof DecodeError &&
      error.message === "`raw_items` reads column `price`, which the statement does not return; it returns: id, name",
  );
  await assert.rejects(
    pool().verify("SELECT id, name, price::float8 AS price FROM raw_items", Item),
    (error) =>
      error instanceof DecodeError &&
      error.message === "`raw_items` decodes column `price` as `decimal`, which cannot read PostgreSQL type `float8`",
  );
  await pool().transaction((tx) => tx.verify("SELECT * FROM raw_items", Item));
  await assert.rejects(pool().verify("SELECT * FROM no_such_table", Item), DatabaseError);
});

// [spec:pgorm:req:bindings.raw-pages/test]
test("SQL text pages and counts", async () => {
  await fresh();
  const statement = fromStatement("SELECT name, price FROM raw_items WHERE price > $1 ORDER BY price DESC", [1]);
  assert.equal(await statement.count(pool()), 3);
  assert.equal((await statement.all(pool())).length, 3);
  await assert.rejects(statement.optional(pool()), DecodeError);
  const pages = statement.paginate(2);
  assert.deepEqual(await pages.numItemsAndPages(pool()), { items: 3, pages: 2 });
  assert.deepEqual((await pages.fetchPage(pool(), 1)).map((row) => row.name), ["one"]);
  assert.ok(pages.inspect(0).sql.startsWith("SELECT * FROM (SELECT name, price FROM raw_items"));
  const records = Item.findByStatement("SELECT * FROM raw_items ORDER BY id").paginate(2);
  assert.deepEqual((await records.fetchPage(pool(), 0)).map((record) => record.label), ["one", "two"]);
  assert.equal(await Item.findByStatement("SELECT * FROM raw_items").count(pool()), 3);
  assert.throws(() => fromStatement("DELETE FROM raw_items").paginate(2), /pagination needs a SELECT/);
  await assert.rejects(fromStatement("SELECT 1; SELECT 2").count(pool()), ConstructionError);
  assert.throws(() => fromStatement("SELECT 1\u0000"), ConstructionError);
});

// [spec:pgorm:req:bindings.transaction-retry/test]
test("a serialization failure runs the callback again", async () => {
  await pool().batchExecute(
    "DROP TABLE IF EXISTS retry_items; CREATE TABLE retry_items (id integer PRIMARY KEY, n integer NOT NULL);" +
      "INSERT INTO retry_items VALUES (1, 10)",
  );
  await using other = await pool().acquire();
  let attempts = 0;
  const doubled = await pool().transactionWithRetry(async (tx: Transaction) => {
    attempts += 1;
    const { n } = await tx.one("SELECT n FROM retry_items WHERE id = 1");
    if (attempts === 1) await other.execute("UPDATE retry_items SET n = n + 1 WHERE id = 1");
    await tx.execute("UPDATE retry_items SET n = $1 WHERE id = 1", [(n as number) * 2]);
    return (n as number) * 2;
  }, { retries: 3, mode: "readWrite", isolation: "serializable" });
  assert.deepEqual([attempts, doubled], [2, 22]);

  attempts = 0;
  await assert.rejects(
    pool().transactionWithRetry(async (tx) => {
      attempts += 1;
      await tx.one("SELECT n FROM retry_items WHERE id = 1");
      await other.execute("UPDATE retry_items SET n = n + 1 WHERE id = 1");
      await tx.execute("UPDATE retry_items SET n = 0 WHERE id = 1");
    }, { retries: 2, mode: "readWrite", isolation: "serializable" }),
    (error) => error instanceof DatabaseError && error.sqlstate === "40001" && error.isRetryable,
  );
  assert.equal(attempts, 3);

  attempts = 0;
  await assert.rejects(
    pool().transactionWithRetry(async () => {
      attempts += 1;
      throw new RangeError("not retryable");
    }, { retries: 5 }),
    RangeError,
  );
  assert.equal(attempts, 1);
  await pool().connection(async (connection) => {
    assert.equal(await connection.transactionWithRetry(async () => 7, { retries: 0 }), 7);
    assert.equal(await connection.ping(), true);
  });
  // @ts-expect-error: retries is a number
  await assert.rejects(pool().transactionWithRetry(async () => 1, { retries: "3" }), TypeError);
  await assert.rejects(pool().transactionWithRetry(async () => 1, { retries: -1 }), ConstructionError);
  await assert.rejects(
    pool().query("SELECT * FROM no_such_table"),
    (error) => error instanceof DatabaseError && error.isRetryable === false,
  );
  assert.equal(new PgormError("x").isRetryable, false);
});

// [spec:pgorm:req:bindings.refusal-fields/test]
test("a driver's refusal carries its fields", async () => {
  const bound = await pool().one("SELECT $1::smallint AS n", [100_000]).catch((error: unknown) => error);
  assert.ok(bound instanceof ConstructionError);
  assert.equal(bound.parameter, 1);
  assert.equal(bound.reason, "value `100000` is out of range for Postgres type `int2`");
  assert.equal(new ConstructionError("x").parameter, null);
  assert.equal(new DecodeError("x").reading, null);
});

// [spec:pgorm:req:bindings.named-functions/test]
test("any function by name", async () => {
  await fresh();
  const docs = new Table("raw_items");
  const query = select(func("initcap", col("name")).as("title")).from(docs).where(col("id").eq(2));
  assert.equal(query.inspect().sql, `SELECT initcap("name") AS "title" FROM "raw_items" WHERE "id" = $1`);
  assert.equal((await pool().one(query)).title, "Two");
  assert.equal(select(func("Initcap", "x")).inspect().sql, `SELECT "Initcap"($1)`);
  await assert.rejects(
    pool().one(select(func("Initcap", "x").as("t"))),
    (error) => error instanceof DatabaseError && error.sqlstate === "42883",
  );
  assert.throws(() => func(""), ConstructionError);
  const keys = tableFunction(func("jsonb_object_keys", literal({ b: 1, a: 2 }))).columns("key").alias("keys");
  assert.deepEqual((await pool().query(select(keys.col("key")).from(keys).orderBy(keys.col("key").asc()))).map((row) => row.key), ["a", "b"]);
});

// [spec:pgorm:req:bindings.template/test]
test("a template binds its interpolations", async () => {
  await fresh();
  const limit = 4;
  const query = select(col("id")).from(new Table("raw_items")).where(template`char_length(${col("name")}) < ${limit}`).orderBy(col("id").asc());
  const { sql: text, values } = query.inspect();
  assert.equal(text, `SELECT "id" FROM "raw_items" WHERE char_length("name") < $1 ORDER BY "id" ASC`);
  assert.deepEqual(values.map((value) => value.value), [4n]);
  assert.deepEqual((await pool().query(query)).map((row) => row.id), [1, 2]);
  assert.equal(select(template`$$a$$ || ${"b"}`.as("ab")).inspect().sql, `SELECT $$a$$ || $1 AS "ab"`);
  // @ts-expect-error: a template is a tagged template literal
  assert.throws(() => template("char_length(name) < 4"), TypeError);
  assert.throws(() => template`${limit} < $1`, ConstructionError);
  assert.throws(() => template`'${limit}'`, ConstructionError);
});

// [spec:pgorm:req:bindings.expressions+4/test]    GROUPING and the full-text search functions
test("grouping and full-text search", async () => {
  await fresh();
  const items = new Table("raw_items");
  const kind = template`CASE WHEN ${col("id")} > 1 THEN 'many' ELSE 'one' END`;
  const query = select(kind.as("kind"), call("grouping", kind).as("rolled"), call("count", col("id")).as("n"))
    .from(items)
    .groupBy(GroupingElement.rollup(kind))
    .orderBy(kind.asc());
  assert.ok(query.inspect().sql.includes("GROUPING(CASE"));
  const rows = await pool().query(query);
  assert.deepEqual(rows.map((row) => [row.kind, row.rolled, row.n]), [["many", 0, 2n], ["one", 0, 1n], [null, 1, 3n]]);
  const found = select(col("id"))
    .from(items)
    .where(call("to_tsvector", col("name")).matches(call("websearch_to_tsquery", "three")));
  assert.deepEqual((await pool().query(found)).map((row) => row.id), [3]);
  const ranked = select(call("ts_rank", call("to_tsvector", "a b"), call("plainto_tsquery", "a")).as("rank"));
  assert.equal(typeof (await pool().one(ranked)).rank, "number");
  assert.throws(() => call("grouping"), ConstructionError);
});

// [spec:pgorm:req:bindings.json-operators/test]
test("the jsonb operators", async () => {
  const meta = literal({ tags: ["sweet"], size: { w: 3 } }).cast("jsonb");
  const query = select(
    meta.getJsonField("size").as("size"),
    meta.getJsonField("size").castJsonField("w").as("width"),
    meta.getJsonPath(["size", "w"]).as("w"),
    meta.castJsonPath(["size", "w"]).as("wText"),
    meta.hasJsonKey("tags").as("tagged"),
    meta.hasAnyJsonKeys(["none", "tags"]).as("anyKey"),
    meta.hasAllJsonKeys(["tags", "none"]).as("allKeys"),
    meta.getJsonField("tags").getJsonField(literal(0)).as("tag"),
  );
  const row = await pool().one(query);
  assert.deepEqual(
    [row.size, row.width, row.w, row.wText, row.tagged, row.anyKey, row.allKeys, row.tag],
    [{ w: 3 }, "3", 3, "3", true, true, false, "sweet"],
  );
  // @ts-expect-error: a JSON key is a string
  assert.throws(() => meta.hasJsonKey(1), TypeError);
});
