// Built statements against a live server, in either runtime: SELECT with its
// clauses, expressions and conditions, run through the same terminals as SQL
// text, and the refusals that keep a statement that cannot be built from
// existing.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import {
  all,
  any,
  array,
  bind,
  call,
  caseOf,
  caseWhen,
  col,
  Condition,
  ConstructionError,
  DatabaseError,
  exists,
  insert,
  Interval,
  literal,
  Pool,
  scalar,
  select,
  Table,
  tuple,
  TypeName,
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

/** Three bags of values for chained set operations, so that every chain whose
 * flat reading PostgreSQL's precedence would regroup reads differently. */
const BAGS = { bag_left: [1, 1, 1, 2, 2, 3], bag_right: [1, 1, 2], bag_third: [1, 2, 2, 4] };

const SET_OPERATIONS = ["union", "unionAll", "intersect", "intersectAll", "except", "exceptAll"] as const;
type SetOperation = (typeof SET_OPERATIONS)[number];

/** `op` over two bags as PostgreSQL computes it: the plain forms yield each
 * qualifying value once, the `ALL` forms by multiplicity. */
function combine(op: SetOperation, left: number[], right: number[]): number[] {
  const count = (bag: number[]) => bag.reduce((counts, v) => counts.set(v, (counts.get(v) ?? 0) + 1), new Map<number, number>());
  const [l, r] = [count(left), count(right)];
  const times = (v: number): number => {
    const [a, b] = [l.get(v) ?? 0, r.get(v) ?? 0];
    switch (op) {
      case "union":
        return Number(a + b > 0);
      case "unionAll":
        return a + b;
      case "intersect":
        return Number(a > 0 && b > 0);
      case "intersectAll":
        return Math.min(a, b);
      case "except":
        return Number(a > 0 && b === 0);
      case "exceptAll":
        return Math.max(a - b, 0);
    }
  };
  return [...new Set([...left, ...right])].sort((x, y) => x - y).flatMap((v) => Array<number>(times(v)).fill(v));
}

const account = new Table("account", { schema: "app", alias: "a" });
const event = new Table("event", { alias: "e" });
const mood = new TypeName("mood", { schema: "app" });

before(async () => {
  database = await scratchDatabase("pgorm_napi_statements");
  shared = new Pool(database.dsn, { maxSize: 4 });
  for (
    const sql of [
      "CREATE SCHEMA app",
      "CREATE TYPE app.mood AS ENUM ('calm', 'glad', 'sad')",
      `CREATE TABLE app.account (
         id int8 PRIMARY KEY, name text NOT NULL, active bool NOT NULL,
         mood app.mood, tags text[] NOT NULL DEFAULT '{}', score numeric
       )`,
      "CREATE TABLE event (id int8 PRIMARY KEY, account_id int8 REFERENCES app.account, kind text NOT NULL)",
      `INSERT INTO app.account VALUES
         (1, 'Alice', true, 'calm', '{a,b}', 19.9900),
         (2, 'Bob', true, 'glad', '{c}', 5),
         (3, 'O''Brien', false, NULL, '{}', NULL),
         (4, '50%_off', true, 'sad', '{d,e,f}', 1.5)`,
      `INSERT INTO event VALUES
         (1, 1, 'click'), (2, 1, 'click'), (3, 1, 'view'), (4, 2, 'click'),
         (5, 2, 'view'), (6, 2, 'view'), (7, 4, 'click')`,
      `CREATE TABLE "we""ird" ("Mixed Case" int4, "select" text)`,
      `INSERT INTO "we""ird" VALUES (1, 'kept')`,
      ...Object.entries(BAGS).flatMap(([name, bag]) => [
        `CREATE TABLE ${name} (v int4 NOT NULL)`,
        `INSERT INTO ${name} VALUES ${bag.map((v) => `(${v})`).join(", ")}`,
      ]),
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

const ids = (rows: Record<string, unknown>[], key = "id") => rows.map((row) => row[key]);

// [spec:pgorm:req:bindings.select+3/test]
test("a SELECT with a join, a nested filter, grouping, HAVING, ordering and a limit runs", async () => {
  const count = call("count", event.col("id"));
  const query = select(account.col("name"), count.as("clicks"))
    .from(account)
    .join(event, Condition.all(account.col("id").eq(event.col("account_id")), event.col("kind").eq("click")), {
      kind: "left",
    })
    .where(Condition.any(account.col("active").eq(true), account.col("mood").isNull()))
    .groupBy(account.col("name"))
    .having(count.gte(1))
    .orderBy(count.desc(), account.col("name").asc())
    .limit(2);
  const rows = await pool().query(query);
  assert.deepStrictEqual(rows, [{ name: "Alice", clicks: 2n }, { name: "50%_off", clicks: 1n }]);
  const tagged = await pool().query(query, { tagged: true });
  assert.equal(tagged[0]?.clicks?.kind, "i64");
});

// [spec:pgorm:req:bindings.statements/test]
test("every terminal runs a built statement as it runs SQL text, on a pool, a connection and a transaction", async () => {
  const one = select(account.col("name")).from(account).where(account.col("id").eq(1));
  const none = select(account.col("name")).from(account).where(account.col("id").eq(99));
  assert.deepStrictEqual(await pool().one(one), { name: "Alice" });
  assert.equal(await pool().optional(none), null);
  assert.equal(await pool().execute(select().from(account)), 4);
  await using connection = await pool().acquire();
  assert.deepStrictEqual(await connection.one(one), { name: "Alice" });
  const streamed: unknown[] = [];
  for await (const row of connection.stream(select(account.col("id")).from(account).orderBy(account.col("id").asc()))) {
    streamed.push(row.id);
  }
  assert.deepStrictEqual(streamed, [1n, 2n, 3n, 4n]);
  const pooled: unknown[] = [];
  for await (const row of pool().stream(select(account.col("id")).from(account).where(account.col("id").lt(3)))) {
    pooled.push(row.id);
  }
  assert.deepStrictEqual(pooled.sort(), [1n, 2n]);
  await connection.transaction(async (transaction) => {
    assert.deepStrictEqual(await transaction.query(one), [{ name: "Alice" }]);
  });
});

// [spec:pgorm:req:bindings.statements/test]
// [spec:pgorm:req:bindings.argument-errors+1/test]
test("a built statement binds its own values, so parameters passed beside it are refused", async () => {
  const query = select().from(account);
  await assert.rejects(pool().query(query, [1] as never), (error: unknown) => {
    assert.ok(error instanceof TypeError);
    assert.match(error.message, /binds its own values/);
    return true;
  });
  assert.throws(() => pool().stream(query, [] as never), TypeError);
});

// [spec:pgorm:req:bindings.statements/test]
test("a builder never changes: extending one in two directions leaves it and each other unchanged", async () => {
  const base = select(account.col("id")).from(account);
  const before = base.inspect().sql;
  const active = base.where(account.col("active").eq(true));
  const named = base.where(account.col("name").eq("Bob"));
  assert.equal(base.inspect().sql, before);
  assert.deepStrictEqual(active.inspect().values.map((value) => value.value), [true]);
  assert.deepStrictEqual(named.inspect().values.map((value) => value.value), ["Bob"]);
  const shared = account.col("id");
  const left = shared.eq(1);
  const right = shared.eq(2);
  assert.equal(shared.inspect().sql, 'SELECT "a"."id"');
  assert.deepStrictEqual(ids(await pool().query(base.where(left))), [1n]);
  assert.deepStrictEqual(ids(await pool().query(base.where(right))), [2n]);
  assert.deepStrictEqual(ids(await pool().query(base.orderBy(account.col("id").asc()))), [1n, 2n, 3n, 4n]);
});

// [spec:pgorm:req:bindings.expressions+2/test]
// [spec:pgorm:req:bindings.identifiers+2/test]
test("values are bound and identifiers quoted: neither text becomes SQL", async () => {
  const hostile = "x' OR '1'='1";
  const query = select(account.col("id")).from(account).where(account.col("name").eq(hostile));
  const { sql, values } = query.inspect();
  assert.ok(!sql.includes(hostile), sql);
  assert.deepStrictEqual(values.map((value) => value.value), [hostile]);
  assert.deepStrictEqual(await pool().query(query), []);
  assert.deepStrictEqual(
    ids(await pool().query(select(account.col("id")).from(account).where(account.col("name").eq("O'Brien")))),
    [3n],
  );
  const weird = new Table('we"ird');
  const row = await pool().one(select(weird.col("Mixed Case"), weird.col("select")).from(weird));
  assert.deepStrictEqual(row, { "Mixed Case": 1, select: "kept" });
});

// [spec:pgorm:req:bindings.expressions+2/test]
test("membership in an empty list matches no row, and its negation every row", async () => {
  const base = select(account.col("id")).from(account).orderBy(account.col("id").asc());
  assert.deepStrictEqual(await pool().query(base.where(account.col("id").isIn([]))), []);
  assert.deepStrictEqual(ids(await pool().query(base.where(account.col("id").isNotIn([])))), [1n, 2n, 3n, 4n]);
  assert.deepStrictEqual(ids(await pool().query(base.where(account.col("id").isIn([2, 4])))), [2n, 4n]);
  const pairs = base.where(tuple(account.col("id"), account.col("name")).isIn([tuple(1, "Alice"), tuple(2, "Nobody")]));
  assert.deepStrictEqual(ids(await pool().query(pairs)), [1n]);
});

// [spec:pgorm:req:bindings.expressions+2/test]
test("text tests read their argument as text, and LIKE reads it as a pattern with its escape", async () => {
  const names = async (predicate: ReturnType<typeof col>) =>
    ids(await pool().query(select(account.col("name")).from(account).where(predicate).orderBy(account.col("id").asc())), "name");
  assert.deepStrictEqual(await names(account.col("name").startsWith("50%_")), ["50%_off"]);
  assert.deepStrictEqual(await names(account.col("name").startsWith("5%")), []);
  assert.deepStrictEqual(await names(account.col("name").endsWith("Brien")), ["O'Brien"]);
  assert.deepStrictEqual(await names(account.col("name").containsText("%_")), ["50%_off"]);
  assert.deepStrictEqual(await names(account.col("name").like("%b%")), ["Bob"]);
  assert.deepStrictEqual(await names(account.col("name").ilike("%b%")), ["Bob", "O'Brien"]);
  assert.deepStrictEqual(await names(account.col("name").like("50!%!_%", { escape: "!" })), ["50%_off"]);
  assert.deepStrictEqual(await names(account.col("name").notLike("%o%")), ["Alice", "O'Brien"]);
});

// [spec:pgorm:req:bindings.expressions+2/test]
// [spec:pgorm:req:sql.ast.expr.similar-to+1/test]
test("an empty escape turns escaping off, so a backslash in a pattern is itself", async () => {
  const text = (value: string) => bind(value).cast("text");
  const answers = await pool().one(select(
    text("a\\b").like("a\\b").as("plain"),
    text("a\\b").like("a\\b", { escape: "" }).as("off"),
    text("A\\B").ilike("a\\b", { escape: "" }).as("folded"),
    text("a\\b").similarTo("a\\b", { escape: "" }).as("similar"),
    text("50%").notLike("50\\%", { escape: "" }).as("percent"),
  ));
  assert.deepStrictEqual(answers, { plain: false, off: true, folded: true, similar: true, percent: true });
  assert.match(text("a").like("a", { escape: "" }).inspect().sql, /ESCAPE ''$/);
});

// [spec:pgorm:req:bindings.expressions+2/test]
test("regex, notRegex, iregex and notIregex, and pathExists, answer as PostgreSQL's operators", async () => {
  const names = async (predicate: ReturnType<typeof col>) =>
    ids(await pool().query(select(account.col("name")).from(account).where(predicate).orderBy(account.col("id").asc())), "name");
  const written = async (predicate: string) =>
    ids(await pool().query(`SELECT name FROM app.account WHERE ${predicate} ORDER BY id`), "name");
  const cases: [ReturnType<typeof col>, string, string[]][] = [
    [account.col("name").regex("^[A-B]"), "name ~ '^[A-B]'", ["Alice", "Bob"]],
    [account.col("name").notRegex("^[A-B]"), "name !~ '^[A-B]'", ["O'Brien", "50%_off"]],
    [account.col("name").iregex("b"), "name ~* 'b'", ["Bob", "O'Brien"]],
    [account.col("name").notIregex("b"), "name !~* 'b'", ["Alice", "50%_off"]],
    [account.col("name").regex("%_"), "name ~ '%_'", ["50%_off"]],
  ];
  for (const [built, predicate, expected] of cases) {
    assert.deepStrictEqual(await names(built), expected, predicate);
    assert.deepStrictEqual(await written(predicate), expected, predicate);
  }
  const doc = bind(Value.json({ a: [1, 2] })).cast("jsonb");
  const answers = await pool().one(select(
    doc.pathExists("$.a[*] ? (@ > 1)").as("found"),
    doc.pathExists("$.b").as("missing"),
    doc.pathExists("strict $.b").as("strict"),
  ));
  assert.deepStrictEqual(answers, await pool().one(
    `SELECT '{"a": [1, 2]}'::jsonb @? '$.a[*] ? (@ > 1)' AS found, '{"a": [1, 2]}'::jsonb @? '$.b' AS missing,
       '{"a": [1, 2]}'::jsonb @? 'strict $.b' AS strict`,
  ));
  assert.deepStrictEqual(answers, { found: true, missing: false, strict: null });
  assert.match(doc.pathExists("$.a").inspect().sql, /@\? CAST\(\$2::text AS jsonpath\)$/);
  assert.throws(() => doc.pathExists(1 as never), mistyped(/JSON path is a string/));
});

// [spec:pgorm:req:bindings.quantifiers+2/test]    matches is @@, and any and all quantify each
// comparison over an array, bound or an expression, and over a subquery's rows or its ARRAY(..),
// an empty array false under ANY and true under ALL, each answering as the SQL written by hand
test("matches, and any and all after a comparison, answer as PostgreSQL's operators", async () => {
  const names = async (predicate: ReturnType<typeof col>) =>
    ids(await pool().query(select(account.col("name")).from(account).where(predicate).orderBy(account.col("id").asc())), "name");
  const written = async (predicate: string) =>
    ids(await pool().query(`SELECT name FROM app.account WHERE ${predicate} ORDER BY id`), "name");
  const kind = (name: string) => select(event.col("account_id")).from(event).where(event.col("kind").eq(name));
  const cases: [ReturnType<typeof col>, string, string[]][] = [
    [account.col("name").matches("alice"), "name @@ 'alice'", ["Alice"]],
    [account.col("id").eq(any([2, 4])), "id = ANY('{2,4}'::int8[])", ["Bob", "50%_off"]],
    [account.col("id").ne(all([2, 4])), "id <> ALL('{2,4}'::int8[])", ["Alice", "O'Brien"]],
    [account.col("id").eq(any(Value.array("i64", []))), "id = ANY('{}'::int8[])", []],
    [account.col("id").ne(all(Value.array("i64", []))), "id <> ALL('{}'::int8[])", ["Alice", "Bob", "O'Brien", "50%_off"]],
    [bind("a").eq(any(account.col("tags"))), "'a' = ANY(tags)", ["Alice"]],
    [account.col("id").lte(any(kind("click"))), "id <= ANY(SELECT account_id FROM event WHERE kind = 'click')", [
      "Alice",
      "Bob",
      "O'Brien",
      "50%_off",
    ]],
    [account.col("id").gt(all(kind("view"))), "id > ALL(SELECT account_id FROM event WHERE kind = 'view')", [
      "O'Brien",
      "50%_off",
    ]],
    [account.col("id").ne(all(array(kind("click")))), "id <> ALL(ARRAY(SELECT account_id FROM event WHERE kind = 'click'))", [
      "O'Brien",
    ]],
  ];
  for (const [built, predicate, expected] of cases) {
    assert.deepStrictEqual(await names(built), expected, predicate);
    assert.deepStrictEqual(await written(predicate), expected, predicate);
  }
  const doc = bind(Value.json({ a: 1 })).cast("jsonb");
  const answers = await pool().one(select(
    doc.matches(bind("$.a == 1").cast("jsonpath")).as("holds"),
    doc.matches(bind("$.a == 2").cast("jsonpath")).as("fails"),
  ));
  assert.deepStrictEqual(answers, { holds: true, fails: false });
  assert.match(account.col("id").eq(any([2, 4])).inspect().sql, /"a"\."id" = ANY\(\$1\)$/);
  const quantified = any(account.col("tags"));
  assert.throws(() => account.col("id").add(quantified as never), refused(/ANY and ALL follow .*, not "add"/));
  assert.throws(() => account.col("name").concat(quantified as never), refused(/ANY and ALL follow .*, not "concat"/));
  assert.throws(() => account.col("id").isDistinctFrom(quantified as never), refused(/ANY and ALL follow a comparison/));
  assert.throws(() => account.col("name").similarTo(quantified as never), refused(/not "similarTo"/));
  assert.throws(() => account.col("name").like(quantified as never, { escape: "!" }), refused(/takes no escape/));
  assert.throws(() => account.col("id").between(quantified as never, 3), refused(/got an ANY or ALL operand/));
  assert.throws(() => account.col("id").isIn([quantified as never]), refused(/expected an expression or a value, got an ANY or ALL operand/));
  assert.throws(() => bind(quantified as never), refused(/cannot be inferred/));
});

// [spec:pgorm:req:bindings.quantifiers+2/test]    any and all after LIKE's and ILIKE's family,
// the regular-expression matches, containment, the range positions, @@ and @?, over a bound
// array or a subquery's rows, answer as the SQL written by hand
test("any and all follow LIKE, regex, containment, range, @@ and @? operators as PostgreSQL's", async () => {
  await pool().execute(`CREATE TABLE app.quantified (id int8 PRIMARY KEY, label text NOT NULL, span int4range NOT NULL, doc jsonb NOT NULL)`);
  await pool().execute(`INSERT INTO app.quantified VALUES (1, 'pgorm', '[1,5)', '{"a": 1}'), (2, 'PGORM', '[5,8)', '{"b": 2}'),
    (3, 'sql', '[10,12)', '{"a": "x"}'), (4, 'other', '[30,40)', '{"c": true}')`);
  const q = new Table("quantified", { schema: "app" });
  const built = async (predicate: ReturnType<typeof col>) =>
    ids(await pool().query(select(q.col("id")).from(q).where(predicate).orderBy(q.col("id").asc())));
  const written = async (predicate: string) =>
    ids(await pool().query(`SELECT id FROM app.quantified WHERE ${predicate} ORDER BY id`));
  const windows = select(literal("[0,6)").cast("int4range")).union(select(literal("[8,20)").cast("int4range")));
  const windowsSql = "SELECT '[0,6)'::int4range UNION SELECT '[8,20)'::int4range";
  const paths = (...items: string[]) => bind(items).cast("jsonpath", { array: true });
  const [label, span, doc] = [q.col("label"), q.col("span"), q.col("doc")];
  const cases: [ReturnType<typeof col>, string, bigint[]][] = [
    [label.like(any(["pg%", "s%"])), "label LIKE ANY('{pg%,s%}')", [1n, 3n]],
    [label.notLike(all(["pg%", "s%"])), "label NOT LIKE ALL('{pg%,s%}')", [2n, 4n]],
    [label.ilike(any(["pg%"])), "label ILIKE ANY('{pg%}')", [1n, 2n]],
    [label.notIlike(all(["pg%", "o%"])), "label NOT ILIKE ALL('{pg%,o%}')", [3n]],
    [label.like(any(select(literal("o%")))), "label LIKE ANY(SELECT 'o%')", [4n]],
    [label.regex(any(["^p", "^s"])), "label ~ ANY('{^p,^s}')", [1n, 3n]],
    [label.notRegex(all(["^p", "^s"])), "label !~ ALL('{^p,^s}')", [2n, 4n]],
    [label.iregex(any(["^PG", "^O"])), "label ~* ANY('{^PG,^O}')", [1n, 2n, 4n]],
    [label.notIregex(all(["^pg"])), "label !~* ALL('{^pg}')", [3n, 4n]],
    [span.contains(any(bind(Value.array("i32", [3, 11])).cast("int4", { array: true }))), "span @> ANY('{3,11}'::int4[])", [1n, 3n]],
    [span.containedBy(any(windows)), `span <@ ANY(${windowsSql})`, [1n, 3n]],
    [span.overlaps(any(windows)), `span && ANY(${windowsSql})`, [1n, 2n, 3n]],
    [span.strictlyLeftOf(any(windows)), `span << ANY(${windowsSql})`, [1n, 2n]],
    [span.strictlyRightOf(any(windows)), `span >> ANY(${windowsSql})`, [3n, 4n]],
    [span.notExtendRightOf(all(windows)), `span &< ALL(${windowsSql})`, [1n]],
    [span.notExtendLeftOf(all(windows)), `span &> ALL(${windowsSql})`, [3n, 4n]],
    [span.adjacentTo(any(windows)), `span -|- ANY(${windowsSql})`, [2n]],
    [label.matches(any(["pgorm", "sql"])), "label @@ ANY('{pgorm,sql}'::text[])", [1n, 2n, 3n]],
    [doc.pathExists(any(paths("$.a", "$.c"))), "doc @? ANY('{$.a,$.c}'::jsonpath[])", [1n, 3n, 4n]],
    [doc.matches(all(paths("$.a == 1"))), `doc @@ ALL('{"$.a == 1"}'::jsonpath[])`, [1n]],
  ];
  for (const [predicate, sql, expected] of cases) {
    assert.deepStrictEqual(await built(predicate), expected, sql);
    assert.deepStrictEqual(await written(sql), expected, sql);
  }
  await assert.rejects(built(doc.pathExists(any(["$.a"]))), refused(/cannot bind a `String` value to Postgres type `jsonpath`/));
});

// [spec:pgorm:req:bindings.expressions+2/test]
test("an enum value binds through its type's cast, scalar and array alike", async () => {
  const calm = new Value("calm", mood);
  const rows = await pool().query(select(account.col("id")).from(account).where(account.col("mood").eq(calm)));
  assert.deepStrictEqual(ids(rows), [1n]);
  const listed = await pool().one(select(bind(Value.array(mood, ["glad", "sad"])).as("moods")));
  assert.deepStrictEqual(listed, { moods: ["glad", "sad"] });
});

// [spec:pgorm:req:bindings.case/test]
// [spec:pgorm:req:bindings.expressions+2/test]
// [spec:pgorm:req:bindings.collations/test]
test("CASE, casts, collations and subscripts compute on the server", async () => {
  const row = await pool().one(
    select(
      caseWhen(account.col("score").gt(10), "high").when(account.col("score").isNull(), "none").else("low").as("band"),
      caseOf(account.col("mood")).when(new Value("calm", mood), bind(1).cast("int4")).else(bind(0).cast("int4")).as("calm"),
      bind("1 day").cast("interval").as("day"),
      account.col("score").cast("integer").as("whole"),
      account.col("tags").at(2).as("second"),
      account.col("tags").slice(1, 1).as("head"),
      account.col("tags").slice(null, null).as("all"),
    ).from(account).where(account.col("id").eq(1)),
  );
  assert.equal(row.band, "high");
  assert.equal(row.calm, 1);
  assert.deepStrictEqual(row.day, new Interval({ days: 1 }));
  assert.equal(row.whole, 20);
  assert.equal(row.second, "b");
  assert.deepStrictEqual(row.head, ["a"]);
  assert.deepStrictEqual(row.all, ["a", "b"]);
  const letters = select(bind("b").as("v")).union(select(bind("B").as("v"))).union(select(bind("a").as("v")));
  const ordered = await pool().query(
    select(col("v")).from(letters.as("letters")).orderBy(col("v").collate("C").asc()),
  );
  assert.deepStrictEqual(ids(ordered, "v"), ["B", "a", "b"]);
});

// [spec:pgorm:req:bindings.from-subqueries/test]
// [spec:pgorm:req:bindings.expressions+2/test]
test("subqueries: EXISTS, a scalar subquery, IN a subquery, a lateral join and a subquery as a FROM item", async () => {
  const clickers = select(event.col("account_id")).from(event).where(event.col("kind").eq("click"));
  const inSubquery = await pool().query(
    select(account.col("id")).from(account).where(account.col("id").isIn(clickers)).orderBy(account.col("id").asc()),
  );
  assert.deepStrictEqual(ids(inSubquery), [1n, 2n, 4n]);
  const flagged = await pool().query(
    select(account.col("id")).from(account)
      .where(exists(select(event.col("id")).from(event).where(event.col("account_id").eq(account.col("id")))).not())
      .orderBy(account.col("id").asc()),
  );
  assert.deepStrictEqual(ids(flagged), [3n]);
  const latest = select(event.col("id").as("last")).from(event).where(event.col("account_id").eq(account.col("id")))
    .orderBy(event.col("id").desc()).limit(1);
  const lateral = await pool().query(
    select(account.col("id"), col("last", { table: "l" }), scalar(select(call("max", event.col("id"))).from(event)).as("top"))
      .from(account)
      .join(latest.as("l"), Condition.all(), { kind: "left", lateral: true })
      .orderBy(account.col("id").asc()),
  );
  assert.deepStrictEqual(lateral.map((row) => [row.id, row.last, row.top]), [
    [1n, 3n, 7n],
    [2n, 6n, 7n],
    [3n, null, 7n],
    [4n, 7n, 7n],
  ]);
  const sub = select(account.col("id"), account.col("name")).from(account).where(account.col("active").eq(true)).as("s");
  assert.equal(await pool().execute(select(sub.star()).from(sub)), 3);
});

// [spec:pgorm:req:bindings.expressions+2/test]    array() collects a subquery's one column into an
// array value, empty for no rows, which is projected, cast and compared as one value, and a
// subquery of two columns is refused with 42601
test("array() collects a subquery's column into one array value", async () => {
  const events = select(event.col("id")).from(event).where(event.col("account_id").eq(account.col("id")));
  const collected = await pool().query(
    select(account.col("id"), array(events).as("events")).from(account).orderBy(account.col("id").asc()),
  );
  assert.deepStrictEqual(
    collected.map((row) => [row.id, [...(row.events as bigint[])].sort((x, y) => Number(x - y))]),
    [[1n, [1n, 2n, 3n]], [2n, [4n, 5n, 6n]], [3n, []], [4n, [7n]]],
  );
  const holding = await pool().query(select(account.col("id")).from(account).where(array(events).contains([4n, 6n])));
  assert.deepStrictEqual(ids(holding), [2n]);
  const moods = select(account.col("mood")).from(account).where(account.col("mood").isNotNull());
  const [spelled] = await pool().query(select(array(moods).cast("text", { array: true }).as("moods")));
  assert.deepStrictEqual([...(spelled?.moods as string[])].sort(), ["calm", "glad", "sad"]);
  await assert.rejects(
    pool().query(select(array(select(account.col("id"), account.col("name")).from(account)).as("pairs"))),
    (error: unknown) => {
      assert.ok(error instanceof DatabaseError, `expected a DatabaseError, got ${error}`);
      assert.equal(error.sqlstate, "42601");
      return true;
    },
  );
});

// [spec:pgorm:req:bindings.select+3/test]
test("set operations combine rows with and without duplicates", async () => {
  const kinds = select(event.col("kind")).from(event);
  const count = async (query: ReturnType<typeof select>) => (await pool().query(query)).length;
  assert.equal(await count(kinds.union(kinds)), 2);
  assert.equal(await count(kinds.unionAll(kinds)), 14);
  assert.equal(await count(kinds.intersect(select(bind("click")))), 1);
  assert.equal(await count(kinds.except(select(bind("click")))), 1);
  assert.equal(await count(kinds.exceptAll(select(bind("click")))), 6);
  assert.equal(await count(kinds.intersectAll(kinds)), 7);
});

// [spec:pgorm:req:bindings.select+3/test]
// [spec:pgorm:sem:sql.ast.select.union+2/test]
test("a chain of set operations combines in call order, for every pair of operators", async () => {
  const bag = (name: keyof typeof BAGS) => select(col("v")).from(new Table(name));
  const intersects = (op: SetOperation) => op === "intersect" || op === "intersectAll";
  for (const first of SET_OPERATIONS) {
    for (const second of SET_OPERATIONS) {
      const chained = bag("bag_left")[first](bag("bag_right"))[second](bag("bag_third")).orderBy(col("v").asc());
      const inCallOrder = combine(second, combine(first, BAGS.bag_left, BAGS.bag_right), BAGS.bag_third);
      const rows = (await pool().query(chained)).map((row) => row.v);
      assert.deepStrictEqual(rows, inCallOrder, `${first} then ${second}`);
      if (intersects(second) && !intersects(first)) {
        const flat = combine(first, BAGS.bag_left, combine(second, BAGS.bag_right, BAGS.bag_third));
        assert.notDeepStrictEqual(flat, inCallOrder, `${first} then ${second} tells the readings apart`);
      }
    }
  }

  const one = select(bind(1).as("v"));
  const two = select(bind(2).as("v"));
  assert.equal(
    one.union(two).intersect(one).inspect().sql,
    '(SELECT $1 AS "v" UNION (SELECT $2 AS "v")) INTERSECT (SELECT $3 AS "v")',
  );
  assert.equal(
    one.intersect(two).union(one).inspect().sql,
    'SELECT $1 AS "v" INTERSECT (SELECT $2 AS "v") UNION (SELECT $3 AS "v")',
  );
});

// [spec:pgorm:req:bindings.row-locks/test]
test("a locking read takes its rows' locks, OF an aliased schema-qualified table, and SKIP LOCKED skips them", async () => {
  await using holder = await pool().acquire();
  await using other = await pool().acquire();
  const locked = select(account.col("id")).from(account).join(event, account.col("id").eq(event.col("account_id")))
    .where(account.col("id").eq(1)).lock("update", { of: [account] });
  await holder.transaction(async (first) => {
    assert.equal((await first.query(locked)).length, 3);
    const skipping = select(account.col("id")).from(account).orderBy(account.col("id").asc())
      .lock("update", { wait: "skipLocked" });
    assert.deepStrictEqual(ids(await other.query(skipping)), [2n, 3n, 4n]);
    await assert.rejects(
      other.query(select().from(account).where(account.col("id").eq(1)).lock("share", { wait: "nowait" })),
      (error: unknown) => error instanceof DatabaseError && error.sqlstate === "55P03",
    );
  });
});

// [spec:pgorm:req:bindings.with+1/test]
// [spec:pgorm:req:bindings.cte-options/test]
test("common table expressions, recursive ones with SEARCH and CYCLE included, run", async () => {
  const clicks = new With("clicks", select(event.col("account_id")).from(event).where(event.col("kind").eq("click")), {
    columns: ["who"],
    materialized: true,
  }).cte("busy", select(col("who")).from(new Table("clicks")).groupBy(col("who")).having(call("count", col("who")).gt(1)));
  assert.deepStrictEqual(await pool().query(select().from(new Table("busy")).with(clicks)), [{ who: 1n }]);
  const body = select(bind(1).cast("int4").as("n"))
    .unionAll(select(col("n").add(1)).from(new Table("r")).where(col("n").lt(5)));
  const counted = await pool().query(
    select(col("n")).from(new Table("r")).orderBy(col("ord").asc()).with(
      With.recursive("r", body, {
        columns: ["n"],
        search: { order: "breadth", by: col("n"), set: "ord" },
        cycle: { by: col("n"), set: "looped", using: "path" },
      }),
    ),
  );
  assert.deepStrictEqual(ids(counted, "n"), [1, 2, 3, 4, 5]);
});

// [spec:pgorm:req:bindings.statements/test]
test("inspect gives the SQL and each bound value with its kind, expressions and conditions included", () => {
  const { sql, values } = select(col("x")).from(new Table("t")).where(col("x").eq(new Value(7, "i16"))).limit(3).inspect();
  assert.equal(sql, 'SELECT "x" FROM "t" WHERE "x" = $1 LIMIT $2');
  assert.deepStrictEqual(values.map((value) => [value.kind, value.value]), [["i16", 7], ["u64", 3n]]);
  assert.ok(Object.isFrozen(values));
  assert.equal(col("x").add(1).inspect().sql, 'SELECT "x" + $1');
  assert.equal(Condition.any().inspect().sql, "SELECT TRUE WHERE FALSE");
  assert.equal(Condition.all().inspect().sql, "SELECT TRUE WHERE TRUE");
});

// [spec:pgorm:req:bindings.statements/test]
test("a failing built statement rejects with the server's error", async () => {
  await assert.rejects(
    pool().query(select(account.col("name").cast("integer")).from(account)),
    (error: unknown) => error instanceof DatabaseError && error.sqlstate === "22P02",
  );
});

// [spec:pgorm:req:bindings.expressions+2/test]
test("a value with no statement form is refused as it is bound", () => {
  assert.throws(() => col("x").eq(null as never), refused(/null has no kind.*isNull\(\)/));
  assert.throws(() => bind(undefined as never), refused(/undefined/));
  assert.throws(() => col("x").eq(new Table("t") as never), refused(/expected an expression or a value, got a Table/));
});

// [spec:pgorm:req:bindings.expressions+2/test]
test("an interval binds in a built statement, each field and sign kept", async () => {
  const intervals = [
    new Interval({ months: 1, days: -2, microseconds: 3_000_004n }),
    new Interval({ months: -1, days: 2, microseconds: -3_000_004n }),
    new Interval({}),
    new Interval({ months: 2 ** 31 - 1 }),
    new Interval({ months: -(2 ** 31) }),
    new Interval({ days: 2 ** 31 - 1 }),
    new Interval({ days: -(2 ** 31) }),
    new Interval({ microseconds: 2n ** 63n - 1n }),
    new Interval({ microseconds: -(2n ** 63n) }),
    new Interval({ months: 2 ** 31 - 1, days: 2 ** 31 - 1, microseconds: 2n ** 63n - 1n }),
    new Interval({ months: -(2 ** 31), days: -(2 ** 31), microseconds: -(2n ** 63n) }),
  ];
  await pool().execute("CREATE TABLE app.span (id int8 PRIMARY KEY, span interval, spans interval[])");
  let written = insert(new Table("span", { schema: "app" })).columns("id", "span", "spans");
  intervals.forEach((interval, id) => {
    written = written.values(id, interval, Value.array("interval", [interval, null]));
  });
  assert.equal(await pool().execute(written), intervals.length);
  const spans = new Table("span", { schema: "app" });
  const rows = await pool().query(select(spans.col("span"), spans.col("spans")).from(spans).orderBy(spans.col("id").asc()));
  assert.deepStrictEqual(rows.map((row) => row.span), intervals);
  assert.deepStrictEqual(rows.map((row) => row.spans), intervals.map((interval) => [interval, null]));

  // The server reads the fields it was sent, the two all-limit triples as its
  // infinities, and a Duration as the interval Interval.from makes of it.
  const read = await pool().one(select(
    bind(intervals[0]).cast("text").as("mixed"),
    bind(intervals[9]).cast("text").as("infinite"),
    bind(intervals[10]).cast("text").as("negative"),
    bind(Temporal.Duration.from({ days: 1, hours: 2 })).cast("text").as("duration"),
  ));
  assert.deepStrictEqual(read, {
    mixed: "1 mon -2 days +00:00:03.000004",
    infinite: "infinity",
    negative: "-infinity",
    duration: "1 day 02:00:00",
  });
  const found = await pool().query(
    select(spans.col("id")).from(spans).where(spans.col("span").eq(new Value(intervals[1], "interval"))),
  );
  assert.deepStrictEqual(found, [{ id: 1n }]);
  await assert.rejects(
    pool().query(select(spans.col("id")).from(spans).where(spans.col("id").eq(intervals[0]))),
    refused(/cannot bind a `Interval` value to Postgres type `int8`/),
  );
});

// [spec:pgorm:req:bindings.operands/test]
test("literal() writes a value into the SQL as pgorm-query renders a constant, and every other operand is bound", async () => {
  const query = select(
    literal("O'Brien").as("name"),
    literal(new Value("glad", mood)).as("mood"),
    literal(7).add(3).as("sum"),
    literal(Value.null("i32")).as("nothing"),
  );
  const { sql, values } = query.inspect();
  assert.equal(
    sql,
    `SELECT E'O\\'Brien' AS "name", CAST('glad' AS app.mood) AS "mood", 7 + $1 AS "sum", NULL AS "nothing"`,
  );
  assert.deepStrictEqual(values.map((value) => [value.kind, value.value]), [["i64", 3n]]);
  assert.deepStrictEqual(await pool().one(query), { name: "O'Brien", mood: "glad", sum: 10, nothing: null });
  const written = select(account.col("id"))
    .from(account)
    .where(Condition.any(account.col("name").eq(literal("O'Brien")), account.col("mood").eq(literal(new Value("glad", mood)))))
    .orderBy(account.col("id").asc());
  assert.deepStrictEqual(written.inspect().values, []);
  assert.deepStrictEqual(ids(await pool().query(written)), [2n, 3n]);
  assert.throws(() => literal(null as never), refused(/null has no kind to write/));
  assert.throws(() => literal(undefined as never), refused(/undefined is no SQL value/));
});

// [spec:pgorm:req:bindings.identifiers+2/test]
// [spec:pgorm:req:bindings.argument-errors+1/test]
test("an identifier is 1–63 UTF-8 bytes without NUL, or refused", () => {
  assert.throws(() => col(""), refused(/identifier/));
  assert.throws(() => col("x".repeat(64)), refused(/identifier/));
  assert.throws(() => col("é".repeat(32)), refused(/identifier/));
  assert.equal(col("é".repeat(31)).inspect().sql, `SELECT "${"é".repeat(31)}"`);
  assert.throws(() => new Table("a\0b"), refused(/identifier/));
  assert.throws(() => col("x", { schema: "s" }), refused(/needs its table/));
  assert.throws(() => col(1 as never), mistyped(/identifier is a string/));
  assert.throws(() => new Table("t", { alias: 7 as never }), mistyped(/identifier is a string/));
});

// [spec:pgorm:req:bindings.expressions+2/test]
test("call() reaches only the functions pgorm-query constructs, at their argument counts", () => {
  assert.throws(() => call("pg_sleep" as never, 1), refused(/no "pg_sleep"/));
  assert.throws(() => call("lower", col("a"), col("b")), refused(/taking 2 argument/));
  assert.throws(() => call("coalesce"), refused(/coalesce/));
  assert.throws(() => call("uuidv7", 1, 2), refused(/uuidv7/));
  assert.equal(call("uuidv7").inspect().sql, "SELECT UUIDV7()");
});

// [spec:pgorm:req:bindings.select+3/test]
test("limits and offsets are non-negative integers within PostgreSQL's bigint", () => {
  const base = select().from(new Table("t"));
  for (const bad of [-1, 1.5, Number.MAX_SAFE_INTEGER + 1, 2n ** 63n, -1n, Number.NaN]) {
    assert.throws(() => base.limit(bad as never), refused(/row count/));
    assert.throws(() => base.offset(bad as never), refused(/row count/));
  }
  for (const bad of ["3", true]) {
    assert.throws(() => base.limit(bad as never), mistyped(/row count/));
    assert.throws(() => base.offset(bad as never), mistyped(/row count/));
  }
  assert.deepStrictEqual(base.limit(2n ** 63n - 1n).inspect().values.map((value) => value.value), [2n ** 63n - 1n]);
});

// [spec:pgorm:req:bindings.expressions+2/test]
// [spec:pgorm:req:bindings.argument-errors+1/test]
test("shapes PostgreSQL has no SQL for are refused as they are built", () => {
  assert.throws(() => (select().select as () => unknown).call(select()), refused(/at least one item/));
  assert.throws(() => (tuple as () => unknown)(), refused(/at least one item/));
  assert.throws(() => col("x").like("a", { escape: "ab" }), refused(/escape is one character/));
  assert.throws(() => col("x").like("a", { escape: "\0" }), refused(/escape is one character/));
  assert.throws(() => col("x").like("a", { escape: 1 as never }), mistyped(/escape is a string/));
  assert.throws(() => col("x").like(1 as never), mistyped(/pattern is a string/));
  assert.throws(() => select(bind(1).asc() as never), mistyped(/projection is an expression, not an ordering/));
  assert.throws(() => select(1 as never), mistyped(/projection is an expression/));
  assert.throws(() => select().from(col("x") as never), mistyped(/FROM item is a Table or a FromItem/));
  assert.throws(() => select().where(1 as never), mistyped(/expression or a Condition/));
  assert.throws(() => select().join(new Table("t"), col("x"), { kind: "outer" as never }), mistyped(/join's kind/));
  assert.throws(() => select().join(new Table("t"), col("x"), { lateral: true }), refused(/lateral applies to a subquery/));
  assert.throws(() => select().lock("exclusive" as never), mistyped(/lock strength/));
  assert.throws(() => select().union(col("x") as never), mistyped(/expected a Select/));
  assert.throws(() => col("x").add(any([1]) as never), refused(/ANY and ALL follow a comparison/));
  assert.throws(() => col("x").eq(undefined as never), refused(/undefined is no SQL value/));
  const recursive = With.recursive("r", select(bind(1)));
  assert.throws(() => recursive.cte("s", select(bind(1))), refused(/recursive WITH holds exactly one/));
  assert.throws(() => col("x").asc({ nulls: "middle" as never }), mistyped(/nulls/));
});

// [spec:pgorm:req:bindings.statements/test]
test("a statement binding more values than PostgreSQL takes is refused before it is sent", async () => {
  const many = select().from(account).where(account.col("id").isIn(Array.from({ length: 65_536 }, (_, index) => index)));
  assert.throws(() => many.inspect(), refused(/65536 values, past PostgreSQL's 65535/));
  await assert.rejects(pool().query(many), refused(/65535/));
  const most = select().from(account).where(account.col("id").isIn(Array.from({ length: 65_535 }, (_, index) => index)));
  assert.equal((await pool().query(most)).length, 4);
});

// [spec:pgorm:req:bindings.statements/test]
test("builders come only from the module's functions", () => {
  const Select = select().constructor as new (handle: unknown) => unknown;
  assert.throws(() => new Select({}), TypeError);
  const Expr = col("x").constructor as new (handle: unknown) => unknown;
  assert.throws(() => new Expr({}), TypeError);
  const Quantified = any(col("x")).constructor as new (handle: unknown) => unknown;
  assert.throws(() => new Quantified({}), TypeError);
});
