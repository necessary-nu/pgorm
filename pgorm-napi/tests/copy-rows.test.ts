// Binary COPY as rows of values against a live server, in either runtime:
// each value kind written as a parameter is bound and read back as a result
// column is decoded, the server's own reading of the same literals as the
// check; rows of the wrong width or type refused; and the description, the
// stream and the connection under them in a pool, a connection and a
// transaction.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import {
  ConstructionError,
  copyFrom,
  copyTo,
  CreatedMultirange,
  CreatedRange,
  DatabaseError,
  Decimal,
  DecodeError,
  insert,
  Interval,
  Multirange,
  type Param,
  Pool,
  Range,
  select,
  Table,
  Uuid,
  Value,
} from "../lib/index.js";
import { same, scratchDatabase } from "./support.ts";

let database: { dsn: string; drop(): Promise<void> } | undefined;
let shared: Pool | undefined;

function pool(): Pool {
  if (!shared) throw new Error("the scratch database was not created");
  return shared;
}

const KINDS = `CREATE TABLE kinds (
  id int4 PRIMARY KEY, c_bool bool, c_int2 int2, c_int8 int8, c_float8 float8,
  c_text text, c_bytea bytea, c_jsonb jsonb, c_uuid uuid, c_date date,
  c_time time, c_timestamp timestamp, c_timestamptz timestamptz,
  c_numeric numeric, c_interval interval, c_int4range int4range,
  c_int4_array int4[], c_text_array text[], c_mood mood, c_positive positive,
  c_null text
)`;

before(async () => {
  database = await scratchDatabase("pgorm_napi_copy_rows");
  shared = new Pool(database.dsn, { maxSize: 4 });
  for (
    const statement of [
      "CREATE TYPE mood AS ENUM ('calm', 'loud')",
      "CREATE DOMAIN positive AS int4 CHECK (VALUE > 0)",
      KINDS,
      "CREATE TABLE item (id int PRIMARY KEY, name text, qty int NOT NULL DEFAULT 7)",
    ]
  ) {
    await pool().execute(statement);
  }
});

after(async () => {
  await shared?.close();
  await database?.drop();
});

/** One value of each kind, as JavaScript writes it. */
function row(id: number): Param[] {
  return [
    id,
    true,
    new Value(-7, "i16"),
    9007199254740993n,
    -2.25,
    "it's ü ☃",
    new Uint8Array([0, 1, 255]),
    { k: "v", n: [1, null] },
    new Uuid("67e55044-10b1-426f-9247-bb680e5fe0c8"),
    Temporal.PlainDate.from("2026-10-19"),
    Temporal.PlainTime.from("23:59:58.123456"),
    Temporal.PlainDateTime.from("1999-12-31T23:00:00"),
    Temporal.Instant.from("2023-11-14T22:13:20Z"),
    new Decimal("12345.6789"),
    new Interval({ months: 14, days: 3, microseconds: 4_000_005n }),
    new Value(new Range(1, 5), "int4range"),
    [1, null, 3],
    ["a", "b,c"],
    "loud",
    7,
    null,
  ];
}

/** The same values as SQL literals, which the server parses itself. */
const LITERALS = `INSERT INTO kinds VALUES (2,
  true, -7, 9007199254740993, -2.25, 'it''s ü ☃', '\\x0001ff', '{"k": "v", "n": [1, null]}',
  '67e55044-10b1-426f-9247-bb680e5fe0c8', '2026-10-19', '23:59:58.123456',
  '1999-12-31 23:00:00', '2023-11-14 22:13:20Z', 12345.6789,
  '1 year 2 mons 3 days 00:00:04.000005', '[1,5)', '{1,NULL,3}', '{a,"b,c"}',
  'loud', 7, NULL
)`;

function refusedWith(pattern: RegExp): (error: unknown) => true {
  return (error: unknown) => {
    assert.ok(error instanceof ConstructionError, `expected a ConstructionError, got ${error}`);
    assert.match(error.message, pattern);
    return true;
  };
}

async function rowsOf(stream: AsyncIterable<unknown[]>): Promise<unknown[][]> {
  const rows: unknown[][] = [];
  for await (const item of stream) rows.push(item);
  return rows;
}

const kinds = new Table("kinds");
const ordered = () => copyTo(select(kinds.star()).from(kinds).orderBy(kinds.col("id").asc()), "binary");

// [spec:pgorm:req:bindings.copy-rows+2/test]    each value kind is written as a parameter binds and read back as a result decodes
test("copyInRows writes each value kind as the server reads its literal, and copyOutRows reads it back", async () => {
  assert.equal(await pool().copyInRows(copyFrom("kinds", "binary"), [row(1)]), 1);
  await pool().execute(LITERALS);
  const compared = await pool().one(
    "SELECT (to_jsonb(a) - 'id') = (to_jsonb(b) - 'id') AS same FROM kinds a, kinds b WHERE a.id = 1 AND b.id = 2",
  );
  assert.equal(compared.same, true);

  const stream = await pool().copyOutRows(ordered());
  assert.deepEqual(stream.columns.slice(0, 3), ["id", "c_bool", "c_int2"]);
  assert.equal(stream.columns.length, 21);
  const rows = await rowsOf(stream);
  const queried = await pool().query("SELECT * FROM kinds ORDER BY id");
  assert.equal(rows.length, 2);
  rows.forEach((copied, at) => same(copied, Object.values(queried[at] ?? {}), `row ${at}`));
  same(rows[0]?.slice(1, 4), [true, -7, 9007199254740993n]);
  assert.equal(rows[0]?.[18], "loud");
  assert.equal(rows[0]?.[20], null);
  await pool().execute("TRUNCATE kinds");
});

// [spec:pgorm:req:bindings.copy-rows+2/test]    a created range or multirange whose subtype is a domain, nested included, travels as the type the domains are over
test("a created range and multirange over a domain are written and read through the domain's base", async () => {
  for (
    const statement of [
      "CREATE DOMAIN height AS float8",
      "CREATE DOMAIN tall AS height CHECK (VALUE > 0)",
      "CREATE TYPE heightrange AS RANGE (subtype = tall)",
      "CREATE TABLE heights (id int PRIMARY KEY, span heightrange, spans heightmultirange)",
    ]
  ) {
    await pool().execute(statement);
  }
  const span = new Value(new Range(1.5, 2), new CreatedRange("heightrange", "f64"));
  const spans = new Value(
    new Multirange([new Range(1.5, 2), new Range(3, null)]),
    new CreatedMultirange("heightmultirange", "f64"),
  );
  assert.equal(await pool().copyInRows(copyFrom("heights", "binary"), [[1, span, spans]]), 1);
  same(await rowsOf(await pool().copyOutRows(copyTo("heights", "binary"))), [[1, span.value, spans.value]]);
  assert.equal((await pool().one("SELECT spans::text AS t FROM heights")).t, "{[1.5,2),[3,)}");
});

// [spec:pgorm:req:bindings.copy-rows+2/test]    a row of the wrong width or type is a ConstructionError that abandons the COPY
// [spec:pgorm:req:napi.typing+1/test]
test("a row of the wrong width or holding a value its column cannot take abandons the COPY", async () => {
  const copy = copyFrom("item", "binary").columns(["id", "name"]);
  const cases: [unknown[][], RegExp][] = [
    [[[1, "one"], [2]], /1 values for the COPY's 2 columns \(id, name\)/],
    [[[1, "one", 3]], /3 values for the COPY's 2 columns/],
    [[["one", "one"]], /COPY column `id` of type `int4`/],
    [[[1, new Decimal("1.5")]], /COPY column `name` of type `text`/],
  ];
  for (const [rows, pattern] of cases) {
    await assert.rejects(pool().copyInRows(copy, rows as Param[][]), refusedWith(pattern));
  }
  await assert.rejects(pool().copyInRows(copy, [[1, "a"], "b" as never]), TypeError);
  await assert.rejects(pool().copyInRows(copy, "rows" as never), TypeError);
  await assert.rejects(pool().copyInRows(copy, new Uint8Array(1) as never), TypeError);
  // @ts-expect-error rows travel in the binary format alone
  await assert.rejects(pool().copyInRows(copyFrom("item", "text"), [[1, "a"]]), /binary COPY FROM/);
  // @ts-expect-error rows travel in the binary format alone
  await assert.rejects(pool().copyOutRows(copyTo("item", "csv")), /binary COPY TO/);
  assert.equal((await pool().one("SELECT count(*)::int AS n FROM item")).n, 0);

  // A refused row leaves nothing written, so the next COPY starts clean.
  assert.equal(await pool().copyInRows(copy, [[1, "one"], [2, null]]), 2);
  const rows = await rowsOf(await pool().copyOutRows(copyTo("item", "binary")));
  same(rows.sort((a, b) => Number(a[0]) - Number(b[0])), [[1, "one", 7], [2, null, 7]]);
  await assert.rejects(pool().copyInRows(copy, [[1, "again"]]), (error: unknown) => {
    assert.ok(error instanceof DatabaseError);
    assert.equal(error.sqlstate, "23505");
    return true;
  });
  await pool().execute("TRUNCATE item");
});

// [spec:pgorm:req:bindings.copy-rows+2/test]    a money column, an array of money and a domain over money copy in
// from a Decimal or a whole number of units, rounded to the currency's places, and copy out as the
// Decimal the server's numeric cast gives
test("a money column copies in from a Decimal and out as one, at its currency's places", async () => {
  for (
    const statement of [
      "CREATE DOMAIN takings AS money CHECK (VALUE >= 0::money)",
      "CREATE TABLE tills (id int PRIMARY KEY, cash money, floats money[], banked takings)",
    ]
  ) {
    await pool().execute(statement);
  }
  const rows: Param[][] = [
    [1, new Decimal("12.345"), [new Decimal("-0.005"), null, new Decimal("3")], 40],
    [2, null, null, null],
  ];
  assert.equal(await pool().copyInRows(copyFrom("tills", "binary"), rows), 2);
  same(
    await pool().one(
      "SELECT cash::numeric AS cash, floats::numeric[] AS floats, banked::numeric AS banked FROM tills WHERE id = 1",
    ),
    { cash: new Decimal("12.35"), floats: [new Decimal("-0.01"), null, new Decimal("3.00")], banked: new Decimal("40.00") },
  );
  const tills = new Table("tills");
  same(await rowsOf(await pool().copyOutRows(copyTo(select(tills.star()).from(tills).orderBy(tills.col("id").asc()), "binary"))), [
    [1, new Decimal("12.35"), [new Decimal("-0.01"), null, new Decimal("3.00")], new Decimal("40.00")],
    [2, null, null, null],
  ]);
  await assert.rejects(
    pool().copyInRows(copyFrom("tills", "binary"), [[3, new Decimal("100000000000000000000"), null, null]]),
    refusedWith(/COPY column `cash` of type `money`/),
  );
  await assert.rejects(pool().copyInRows(copyFrom("tills", "binary"), [[3, null, null, new Decimal("-1")]]), (error: unknown) => {
    assert.ok(error instanceof DatabaseError, `expected a DatabaseError, got ${error}`);
    assert.equal(error.sqlstate, "23514");
    return true;
  });
  await pool().execute("DROP TABLE tills");
  await pool().execute("DROP DOMAIN takings");
});

// [spec:pgorm:req:bindings.copy-rows+2/test]    rows run on a connection and in a transaction, from an async generator, and an empty source writes none
test("copyInRows and copyOutRows run on a connection and in a transaction, holding each until the COPY ends", async () => {
  await using connection = await pool().acquire();
  async function* generated() {
    for (let id = 1; id <= 600; id += 1) yield [id, `n${id}`] as Param[];
  }
  const copy = copyFrom("item", "binary").columns(["id", "name"]);
  assert.equal(await connection.copyInRows(copy, generated()), 600);
  assert.equal(await connection.copyInRows(copy, []), 0);
  const counted = await rowsOf(
    await connection.copyOutRows(copyTo(select(new Table("item").col("id")).from(new Table("item")), "binary")),
  );
  assert.equal(counted.length, 600);

  await connection.transaction(async (transaction) => {
    assert.equal(await transaction.copyInRows(copy, [[601, "late"]]), 1);
    const inserted = insert(new Table("item")).columns("id", "name").values(602, "returned").returning([
      new Table("item").col("qty"),
      new Table("item").col("name"),
    ]);
    const stream = await transaction.copyOutRows(copyTo(inserted, "binary"));
    assert.deepEqual(stream.columns, ["qty", "name"]);
    same(await rowsOf(stream), [[7, "returned"]]);
    assert.equal((await transaction.one("SELECT count(*)::int AS n FROM item")).n, 602);
    throw new Error("roll back");
  }).catch((error: unknown) => assert.match(String(error), /roll back/));
  assert.equal((await connection.one("SELECT count(*)::int AS n FROM item")).n, 600);
  await connection.execute("TRUNCATE item");
});

// [spec:pgorm:req:bindings.copy-rows+2/test]    a refused description keeps the connection; a field no value holds is a DecodeError that closes the stream
test("a description the server refuses keeps the connection, and an undecodable field closes the stream", async () => {
  await using connection = await pool().acquire();
  await assert.rejects(connection.copyInRows(copyFrom("missing", "binary"), [[1]]), (error: unknown) => {
    assert.ok(error instanceof DatabaseError, `expected a DatabaseError, got ${error}`);
    assert.equal(error.sqlstate, "42P01");
    return true;
  });
  assert.equal((await connection.one("SELECT 1 AS n")).n, 1);

  const points = await connection.copyOutRows(copyTo(new Table("pts"), "binary")).catch((error: unknown) => error);
  assert.ok(points instanceof DatabaseError);
  await connection.execute("CREATE TABLE pts (p point)");
  await connection.execute("INSERT INTO pts SELECT point(n, n) FROM generate_series(1, 3) AS n");
  const stream = await connection.copyOutRows(copyTo(new Table("pts"), "binary"));
  const failed = stream.next();
  await assert.rejects(failed, (error: unknown) => {
    assert.ok(error instanceof DecodeError, `expected a DecodeError, got ${error}`);
    assert.match(error.message, /COPY column "p" of type `point`/);
    return true;
  });
  assert.equal(stream.closed, true);
  same(await stream.next(), { done: true, value: undefined });
  await connection.execute("DROP TABLE pts").catch(() => undefined);
  assert.equal((await pool().one("SELECT 1 AS n")).n, 1);
});

/**
 * The multiranges created beside a range over `float8` and one over `int4`
 * named apart from its range, which has no canonical function and so keeps
 * `[1,5]` as written; a domain over one; and a table of them and an array of
 * one.
 */
const MULTISPANS = [
  "CREATE SCHEMA geo",
  "CREATE TYPE geo.floatrange AS RANGE (subtype = float8)",
  `CREATE TYPE geo.slot AS RANGE (subtype = int4, multirange_type_name = geo."Slot Set")`,
  "CREATE DOMAIN geo.reaches AS geo.floatmultirange",
  `CREATE TABLE geo.spans (id int PRIMARY KEY, reach geo.floatmultirange NOT NULL, slots geo."Slot Set", kept geo.reaches)`,
  `INSERT INTO geo.spans VALUES
    (1, '{[1.5,2.5), [3,4]}', '{[1,5], [7,9)}', '{[0,1)}'),
    (2, '{}', NULL, NULL),
    (3, '{[-Infinity,0]}', '{(,7)}', '{}')`,
  `CREATE TABLE geo.many (slots geo."Slot Set"[])`,
  `INSERT INTO geo.many VALUES ('{"{[1,3)}"}')`,
];

// [spec:pgorm:req:bindings.copy-rows+2/test]    a multirange type a schema created reads as a Multirange and writes back from a Value of its kind or from its text
// [spec:pgorm:req:bindings.created-ranges+5/test]
test("a multirange type a schema created reads out of a binary COPY and writes back in", async () => {
  for (const statement of MULTISPANS) await pool().execute(statement);
  const spans = new Table("spans", { schema: "geo" });
  const reach = new CreatedMultirange("floatmultirange", "f64", { schema: "geo" });
  const slots = new CreatedMultirange("Slot Set", "i32", { schema: "geo" });
  const ordered = copyTo(select(spans.star()).from(spans).orderBy(spans.col("id").asc()), "binary");

  const stream = await pool().copyOutRows(ordered);
  assert.deepEqual(stream.columns, ["id", "reach", "slots", "kept"]);
  const rows = await rowsOf(stream);
  same(rows, [
    [
      1,
      new Multirange([new Range(1.5, 2.5), new Range(3, 4, "[]")]),
      new Multirange([new Range(1, 5, "[]"), new Range(7, 9)]),
      new Multirange([new Range(0, 1)]),
    ],
    [2, new Multirange([]), null, null],
    [3, new Multirange([new Range(-Infinity, 0, "[]")]), new Multirange([new Range(null, 7, "()")]), new Multirange([])],
  ]);
  // A query's column of one has no subtype the driver reports, so only a COPY, whose description completes it, reads one.
  await assert.rejects(pool().one("SELECT reach FROM geo.spans WHERE id = 1"), DecodeError);

  const [first, second] = rows;
  const written: Param[][] = [
    [11, new Value(first?.[1], reach), new Value(first?.[2], slots), new Value(first?.[3], reach)],
    [12, new Value(second?.[1], reach), null, null],
    [13, "{[-Infinity,0]}", "{(,7)}", "{}"],
  ];
  assert.equal(await pool().copyInRows(copyFrom(spans, "binary"), written), 3);
  const compared = await pool().one(
    `SELECT count(*)::int AS n FROM geo.spans a JOIN geo.spans b ON b.id = a.id + 10
     WHERE a.reach = b.reach AND a.slots IS NOT DISTINCT FROM b.slots
       AND a.kept::geo.floatmultirange IS NOT DISTINCT FROM b.kept::geo.floatmultirange`,
  );
  assert.equal(compared.n, 3);

  const copy = copyFrom(spans, "binary").columns(["id", "reach"]);
  for (const text of ["{[a,b)}", "[1,2)"]) {
    await assert.rejects(
      pool().copyInRows(copy, [[20, text]]),
      refusedWith(/COPY column `reach` of type `geo.floatmultirange`/),
    );
  }
  assert.equal((await pool().one("SELECT count(*)::int AS n FROM geo.spans")).n, 6);

  // An array of one reads and writes as an array of its kind, its elements completed as the column's are.
  const many = new Table("many", { schema: "geo" });
  same(await rowsOf(await pool().copyOutRows(copyTo(many, "binary"))), [[[new Multirange([new Range(1, 3)])]]]);
  const array = Value.array(slots, [new Multirange([new Range(4, 6)]), null]);
  assert.equal(await pool().copyInRows(copyFrom(many, "binary"), [[array], [["{[8,9)}"]]]), 2);
  const read = await rowsOf(await pool().copyOutRows(copyTo(many, "binary")));
  same(read.slice(1), [[[new Multirange([new Range(4, 6)]), null]], [[new Multirange([new Range(8, 9)])]]]);
  await pool().execute("DROP SCHEMA geo CASCADE");
});

// [spec:pgorm:req:bindings.copy-rows.typed/test]    rows copyOutRows read write back through copyInRows, each value as its column's type
test("rows read by copyOutRows write back through copyInRows", async () => {
  const columns =
    "r int4range, m int4multirange, j jsonb, ja jsonb, jn jsonb, js jsonb, a int4[], e int4[], n int4[], " +
    "jl jsonb[], d numeric, t timestamptz, i interval, b bytea, u uuid";
  await pool().execute(`CREATE TABLE round_from (${columns})`);
  await pool().execute(`CREATE TABLE round_to (${columns})`);
  await pool().execute(
    `INSERT INTO round_from VALUES ('[1,5)', '{[1,2),[4,6)}', '{"a":1}', '[1,2]', '3', '"s"', '{1,2}', '{}', ` +
      `'{NULL}', ARRAY['[1]'::jsonb, '2'], 1.50, '2026-01-01T00:00:00Z', '1 day', '\\x0102', ` +
      `'00000000-0000-0000-0000-000000000001')`,
  );
  // The stream's rows are CopyRows as they come, with no cast.
  const written = await pool().copyInRows(
    copyFrom(new Table("round_to"), "binary"),
    await pool().copyOutRows(copyTo(new Table("round_from"), "binary")),
  );
  assert.equal(written, 1);
  const { differ } = await pool().one(
    "SELECT count(*)::int AS differ FROM (SELECT * FROM round_from EXCEPT ALL SELECT * FROM round_to) AS d",
  );
  assert.equal(differ, 0);
  await pool().execute("DROP TABLE round_from, round_to");
});
