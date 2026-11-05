// Every value kind crosses into and out of JavaScript against a live server,
// in either runtime: bound as a parameter, read back from a result, and
// refused — never rounded, wrapped, truncated or stringified — where its
// JavaScript type cannot hold it exactly.

import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import { after, before, test } from "node:test";

import {
  col as statementCol,
  ConstructionError,
  CreatedMultirange,
  CreatedRange,
  DatabaseError,
  Decimal,
  DecodeError,
  Interval,
  Multirange,
  type Param,
  Pool,
  Range,
  type Row,
  TypeName,
  Uuid,
  Value,
} from "../lib/index.js";
import { native } from "../lib/operations.js";
import { same, scratchDatabase } from "./support.ts";

let database: { dsn: string; drop(): Promise<void> } | undefined;
let pool: Pool | undefined;

function db(): Pool {
  if (!pool) throw new Error("the scratch database was not created");
  return pool;
}

const mood = new TypeName("mood", { schema: "app" });
const floatrange = new CreatedRange("floatrange", "f64", { schema: "app" });
const floatmultirange = new CreatedMultirange("floatmultirange", "f64", { schema: "app" });

before(async () => {
  database = await scratchDatabase("pgorm_napi_values");
  pool = new Pool(database.dsn);
  for (
    const statement of [
      "CREATE SCHEMA app",
      `CREATE TYPE app.mood AS ENUM ('calm', 'tense', 'a "quoted", label')`,
      "CREATE TYPE app.floatrange AS RANGE (subtype = float8, multirange_type_name = app.floatmultirange)",
      "CREATE TYPE app.slot AS RANGE (subtype = int4)",
      "CREATE DOMAIN app.email AS text CHECK (VALUE LIKE '%@%')",
      "CREATE DOMAIN app.span AS interval",
      "CREATE DOMAIN app.height AS float8",
      "CREATE DOMAIN app.tall AS app.height CHECK (VALUE > 0)",
      "CREATE TYPE app.heightrange AS RANGE (subtype = app.tall)",
    ]
  ) {
    await db().execute(statement);
  }
});

after(async () => {
  await pool?.close();
  await database?.drop();
});

async function one(sql: string, params: readonly Param[] = []): Promise<Row> {
  return await db().one(sql, params);
}

/** `value` bound to a placeholder of `type`, and read back. */
async function roundTrip(type: string, value: Param): Promise<unknown> {
  return (await one(`SELECT $1::${type} AS v`, [value])).v;
}

async function refused(pending: Promise<unknown>, pattern?: RegExp): Promise<void> {
  await assert.rejects(pending, (error: unknown) => {
    assert.ok(error instanceof ConstructionError, `expected a ConstructionError, got ${error}`);
    if (pattern) assert.match(error.message, pattern);
    return true;
  });
}

async function undecodable(sql: string, pattern?: RegExp): Promise<void> {
  await assert.rejects(db().query(sql), (error: unknown) => {
    assert.ok(error instanceof DecodeError, `expected a DecodeError, got ${error}`);
    if (pattern) assert.match(error.message, pattern);
    return true;
  });
}

function throwsConstruction(make: () => unknown, pattern?: RegExp): void {
  assert.throws(make, (error: unknown) => {
    assert.ok(error instanceof ConstructionError, `expected a ConstructionError, got ${error}`);
    if (pattern) assert.match(error.message, pattern);
    return true;
  });
}

// [spec:pgorm:req:bindings.values/test]
test("SQL NULL reads as null, and an untyped null binds to any type", async () => {
  const row = await one("SELECT $1::int4 AS a, $2::text AS b, $3::jsonb AS c, NULL::numeric AS d, $4::int4[] AS e", [
    null,
    null,
    null,
    null,
  ]);
  same(row, { a: null, b: null, c: null, d: null, e: null });
});

// [spec:pgorm:req:bindings.values/test]
test("booleans round trip, and only a boolean binds as bool", async () => {
  assert.equal(await roundTrip("bool", true), true);
  assert.equal(await roundTrip("bool", false), false);
  await refused(roundTrip("bool", 1), /cannot bind a `BigInt` value to Postgres type `bool`/);
});

// [spec:pgorm:req:napi.values+1/test]
test("int8 reads as an exact bigint at both ends of its range", async () => {
  for (const value of [9007199254740993n, -(2n ** 63n), 2n ** 63n - 1n, 0n]) {
    assert.equal(await roundTrip("int8", value), value);
  }
  const row = await one("SELECT 9007199254740993::int8 AS v, 9007199254740993::int8::text AS t");
  assert.equal(typeof row.v, "bigint");
  assert.equal(String(row.v), row.t);
});

// [spec:pgorm:req:bindings.values/test]
test("int2, int4, oid and \"char\" read as numbers", async () => {
  same(await one(`SELECT 32767::int2 AS a, (-2147483648)::int4 AS b, 4294967295::oid AS c, 'x'::"char" AS d`), {
    a: 32767,
    b: -2147483648,
    c: 4294967295,
    d: 120,
  });
});

// [spec:pgorm:req:bindings.values/test]
// [spec:pgorm:req:napi.values+1/test]
test("an integer kind holds exactly its range, and a number past 2^53 is no exact integer", async () => {
  throwsConstruction(() => new Value(32768, "i16"), /outside i16's range/);
  throwsConstruction(() => new Value(-1, "u32"), /outside u32's range/);
  throwsConstruction(() => new Value(2n ** 63n, "i64"), /outside i64's range/);
  throwsConstruction(() => new Value(2 ** 53 + 2, "i64"), /pass a bigint/);
  throwsConstruction(() => new Value(1.5, "i32"), /not an integer/);
  const unsigned = new Value(2n ** 64n - 1n, "u64");
  assert.equal(unsigned.value, 2n ** 64n - 1n);
  await refused(roundTrip("int8", unsigned), /out of range/);
  assert.equal(await roundTrip("int2", new Value(255n, "i16")), 255);
  await refused(roundTrip("int8", 2 ** 60), /without loss/);
});

// [spec:pgorm:req:bindings.values/test]
test("float8 round trips signed zero, the infinities, NaN and every finite double", async () => {
  for (const value of [0.1, -0, 0, Infinity, -Infinity, NaN, 5e-324, Number.MAX_VALUE, 1 / 3]) {
    assert.ok(Object.is(await roundTrip("float8", value), value), `${value}`);
  }
});

// [spec:pgorm:req:napi.values+1/test]
test("float4 reads exactly, and f32 refuses a number it would round", async () => {
  assert.equal((await one("SELECT 0.1::float4 AS v")).v, Math.fround(0.1));
  throwsConstruction(() => new Value(0.1, "f32"), /no exact f32 form/);
  assert.equal(await roundTrip("float4", new Value(1.5, "f32")), 1.5);
  assert.ok(Object.is(await roundTrip("float4", new Value(-0, "f32")), -0));
});

// [spec:pgorm:req:napi.values+1/test]
test("text round trips any Unicode, and a lone surrogate or NUL is refused", async () => {
  for (const text of ["", "plain", `quotes ' " and \\ backslash`, "100% _like_", "日本語 ✓ 🦀", "é"]) {
    assert.equal(await roundTrip("text", text), text);
  }
  assert.equal(await roundTrip("varchar(3)", "abc"), "abc");
  await refused(roundTrip("text", "\ud800"), /lone surrogate/);
  await assert.rejects(roundTrip("text", "a\u0000b"), (error: unknown) => {
    assert.ok(error instanceof DatabaseError);
    assert.equal(error.sqlstate, "22021");
    return true;
  });
  await refused(db().query("SELECT 'a\u0000b'"), /NUL/);
  assert.equal(await roundTrip("text", new Value("é", "char")), "é");
  throwsConstruction(() => new Value("ab", "char"), /exactly one code point/);
});

// [spec:pgorm:req:bindings.values/test]
test("bytea round trips every byte value as a Uint8Array", async () => {
  const every = Uint8Array.from({ length: 256 }, (_, index) => index);
  same(await roundTrip("bytea", every), every);
  same(await roundTrip("bytea", new Uint8Array()), new Uint8Array());
  const fromBuffer = await roundTrip("bytea", Buffer.from([1, 2, 3]));
  same(fromBuffer, new Uint8Array([1, 2, 3]));
});

// [spec:pgorm:req:napi.values+1/test]
test("numeric reads as an exact Decimal that keeps its scale", async () => {
  for (const text of ["19.9900", "-0.000001", "79228162514264337593543950335", "0.0000000000000000000000000001", "0"]) {
    const value = await roundTrip("numeric", new Decimal(text));
    assert.ok(value instanceof Decimal);
    assert.equal(String(value), text);
  }
  assert.equal(String((await one("SELECT 1.5::numeric(10, 4) AS v")).v), "1.5000");
  await undecodable("SELECT 1e-29::numeric AS v", /28 fractional digits/);
  await undecodable("SELECT 1e30::numeric AS v", /cannot be decoded/);
  await undecodable("SELECT 12.3456789012345678901234567891::numeric AS v", /no exact decimal/);
  await undecodable("SELECT 'NaN'::numeric AS v", /NaN or infinite/);
  await undecodable("SELECT 'Infinity'::numeric AS v", /NaN or infinite/);
});

// [spec:pgorm:req:napi.values+1/test]
test("a Decimal is made only from exact text or a bigint, and never becomes a number implicitly", () => {
  throwsConstruction(() => new Decimal(1.5 as never), /never a number/);
  for (const text of ["1e5", "1.", ".5", "", "1.2.3", "79228162514264337593543950336", "0.00000000000000000000000000001"]) {
    throwsConstruction(() => new Decimal(text));
  }
  assert.equal(String(new Decimal(12n)), "12");
  assert.equal(String(new Decimal("+007.50")), "7.50");
  const decimal = new Decimal("1.5");
  assert.throws(() => +decimal, TypeError);
  assert.equal(`${decimal}`, "1.5");
  assert.equal(JSON.stringify({ decimal }), '{"decimal":"1.5"}');
});

// [spec:pgorm:req:bindings.values/test]
test("uuid reads as a Uuid, and a UUID string binds only when declared", async () => {
  const text = "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11";
  const value = await roundTrip("uuid", new Uuid(text.toUpperCase()));
  assert.ok(value instanceof Uuid);
  assert.equal(String(value), text);
  assert.ok(value.equals(new Uuid(`{${text}}`)));
  await refused(roundTrip("uuid", text), /cannot bind a `String` value to Postgres type `uuid`/);
  assert.equal(String(await roundTrip("uuid", new Value(text, "uuid"))), text);
  throwsConstruction(() => new Uuid("not-a-uuid"), /not a UUID/);
});

// [spec:pgorm:req:napi.values+1/test]
test("json and jsonb read as JavaScript values, an integer past the safe range as a bigint", async () => {
  const text = `{"big": 12345678901234567890, "small": -7, "half": 0.5, "s": "x\\"y", "n": null, "a": [true, false]}`;
  for (const type of ["json", "jsonb"]) {
    const { v } = await one(`SELECT '${text}'::${type} AS v`);
    same(v, { a: [true, false], big: 12345678901234567890n, half: 0.5, n: null, s: 'x"y', small: -7 });
  }
  assert.ok(Object.is((await one(`SELECT '-0.0'::json AS v`)).v, -0));
  const { v } = await one(`SELECT '{"__proto__": {"polluted": true}}'::jsonb AS v`);
  assert.equal(Object.getPrototypeOf(v), Object.prototype);
  assert.ok(Object.hasOwn(v as object, "__proto__"));
  await undecodable(`SELECT '{"a": 1e400}'::json AS v`);
  await undecodable(`SELECT '[0.10000000000000000001]'::json AS v`, /no exact binary form/);
  await undecodable(`SELECT '18446744073709551616'::json AS v`, /exceeds 64 bits/);
});

// [spec:pgorm:req:bindings.values/test]
test("a plain object binds as JSON, and anything JSON cannot hold is refused", async () => {
  const document = { a: 1, b: [1, "x", null, true], c: { d: 9007199254740993n, e: 0.25 } };
  same(await roundTrip("jsonb", document), { a: 1, b: [1, "x", null, true], c: { d: 9007199254740993n, e: 0.25 } });
  same(await roundTrip("jsonb", Value.json([1, 2])), [1, 2]);
  same(await roundTrip("jsonb", Value.json("text")), "text");
  await refused(roundTrip("jsonb", { a: undefined } as unknown as Param), /undefined/);
  await refused(roundTrip("jsonb", { a: NaN }), /NaN/);
  await refused(roundTrip("jsonb", { a: new Decimal("1") } as unknown as Param), /JSON holds only/);
  await refused(roundTrip("jsonb", { a: 2n ** 64n }), /64 bits/);
  const cycle: Record<string, unknown> = {};
  cycle.self = cycle;
  await refused(roundTrip("jsonb", cycle as never), /cycle/);
  await refused(roundTrip("text", { a: 1 }), /cannot bind a `Json` value to Postgres type `text`/);
});

// [spec:pgorm:req:bindings.values/test]
test("inet, cidr and macaddr round trip", async () => {
  assert.equal(await roundTrip("inet", new Value("10.1.2.3/8", "ipnetwork")), "10.1.2.3/8");
  assert.equal(await roundTrip("cidr", new Value("10.0.0.0/8", "ipnetwork")), "10.0.0.0/8");
  assert.equal(await roundTrip("inet", new Value("2001:db8::1/128", "ipnetwork")), "2001:db8::1/128");
  const mac = new Uint8Array([8, 0, 0x2b, 1, 2, 3]);
  same(await roundTrip("macaddr", new Value(mac, "mac_address")), mac);
  throwsConstruction(() => new Value(new Uint8Array(5), "mac_address"), /six bytes/);
  throwsConstruction(() => new Value("10.0.0.0/33", "ipnetwork"), /not an IP network/);
});

// [spec:pgorm:req:bindings.values/test]
test("vector round trips as a Float32Array where pgvector is installed", async (context) => {
  const rows = await db().query("SELECT 1 FROM pg_available_extensions WHERE name = 'vector'");
  if (rows.length === 0) {
    context.skip("pgvector is not installed on the server");
    return;
  }
  await db().query("CREATE EXTENSION IF NOT EXISTS vector");
  const vector = new Float32Array([1.5, -0, 3.25]);
  same(await roundTrip("vector", new Value(vector, "vector")), vector);
  throwsConstruction(() => new Value([0.1], "vector"), /no exact f32 form/);
});

// [spec:pgorm:req:bindings.values/test]
test("a type the binding has no JavaScript form for is a DecodeError, never a string", async () => {
  await undecodable("SELECT point(1, 2) AS v", /`point`/);
  await undecodable("SELECT 1::money AS v", /`money`/);
});

// [spec:pgorm:req:bindings.values/test]
test("arrays read as JavaScript arrays, a NULL item as null", async () => {
  same(await one("SELECT '{1,NULL,3}'::int4[] AS v"), { v: [1, null, 3] });
  same(await one("SELECT ARRAY['a', NULL]::text[] AS v"), { v: ["a", null] });
  same(await one("SELECT '{1.50,-2}'::numeric[] AS v"), { v: [new Decimal("1.50"), new Decimal("-2")] });
  same(await one("SELECT ARRAY['2026-10-09'::date] AS v"), { v: [Temporal.PlainDate.from("2026-10-09")] });
  same(await one(`SELECT ARRAY['{"a":1}'::jsonb, NULL] AS v`), { v: [{ a: 1 }, null] });
  same(await one("SELECT ARRAY['\\x00ff'::bytea] AS v"), { v: [new Uint8Array([0, 255])] });
  same(await one("SELECT '{}'::int8[] AS v, NULL::int8[] AS w"), { v: [], w: null });
  await undecodable("SELECT '{{1,2},{3,4}}'::int4[] AS v", /one-dimensional/);
  await undecodable("SELECT '[2:3]={1,2}'::int4[] AS v", /lower bound 1/);
});

// [spec:pgorm:req:napi.inference/test]
test("a plain array infers its items' one kind", async () => {
  same(await roundTrip("int4[]", [1, 2, null]), [1, 2, null]);
  same(await roundTrip("float8[]", [1.5, 2]), [1.5, 2]);
  same(await roundTrip("text[]", ["a", null]), ["a", null]);
  same(await roundTrip("numeric[]", [new Decimal("1.5")]), [new Decimal("1.5")]);
  same(await roundTrip("int8[]", [1n, null, 2n]), [1n, null, 2n]);
  await refused(roundTrip("int4[]", []), /Value\.array/);
  await refused(roundTrip("int4[]", [null]), /Value\.array/);
  await refused(roundTrip("text[]", [1, "a"]), /mixes numbers/);
  await refused(roundTrip("text[]", ["a", true]), /different kinds/);
  await refused(roundTrip("int4[]", [[1]]), /nested arrays/);
});

// [spec:pgorm:req:bindings.values/test]
test("the built-in ranges and multiranges round trip with their bounds", async () => {
  same(await roundTrip("int4range", new Value(new Range(1, 5, "[]"), "int4range")), new Range(1, 6));
  same(await roundTrip("int8range", new Value(new Range(null, 2n ** 62n), "int8range")), new Range(null, 2n ** 62n));
  same(
    await roundTrip("numrange", new Value(new Range(new Decimal("0.50"), null, "(]"), "numrange")),
    new Range(new Decimal("0.50"), null, "()"),
  );
  same(
    await roundTrip("daterange", new Value(new Range(Temporal.PlainDate.from("2026-01-01"), null), "daterange")),
    new Range(Temporal.PlainDate.from("2026-01-01"), null),
  );
  const start = Temporal.PlainDateTime.from("2026-01-01T00:00:00.000001");
  same(await roundTrip("tsrange", new Value(new Range(start, null), "tsrange")), new Range(start, null));
  const instant = Temporal.Instant.from("2026-01-01T00:00:00.000001Z");
  same(await roundTrip("tstzrange", new Value(new Range(null, instant, "(]"), "tstzrange")), new Range(null, instant, "(]"));
  same(await roundTrip("int4range", new Value(Range.empty(), "int4range")), Range.empty());
  same(await roundTrip("int4range", new Value(new Range(), "int4range")), new Range(null, null, "()"));
  same(
    await roundTrip("int4multirange", new Value(new Multirange([new Range(5, 8), new Range(1, 3)]), "int4multirange")),
    new Multirange([new Range(1, 3), new Range(5, 8)]),
  );
  await refused(roundTrip("int4range", new Range(1, 5) as unknown as Param), /not inferred/);
  throwsConstruction(() => new Value(new Range(1.5, 2), "int4range"), /not an integer/);
});

// [spec:pgorm:req:bindings.values/test]
test("a range's lower bound is written as its lower bound", async () => {
  await assert.rejects(roundTrip("int4range", new Value(new Range(5, 1), "int4range")), (error: unknown) => {
    assert.ok(error instanceof DatabaseError);
    assert.equal(error.sqlstate, "22000");
    return true;
  });
  const range = await roundTrip("int4range", new Value(new Range(2, 9, "(]"), "int4range"));
  same(range, new Range(3, 10));
});

// [spec:pgorm:req:bindings.value-tags+1/test]
test("a typed NULL keeps its kind, and JSON's null is not SQL NULL", async () => {
  const typed = Value.null("i64");
  assert.equal(typed.kind, "i64");
  assert.equal(typed.isNull, true);
  assert.equal(typed.value, null);
  const jsonNull = Value.json(null);
  assert.equal(jsonNull.kind, "json");
  assert.equal(jsonNull.isNull, false);
  same(await one("SELECT $1::jsonb IS NULL AS sql_null, $1::jsonb::text AS t", [jsonNull]), {
    sql_null: false,
    t: "null",
  });
  same(await one("SELECT $1::jsonb IS NULL AS sql_null", [Value.null("json")]), { sql_null: true });
  const [row] = await db().query("SELECT NULL::jsonb AS sql_null, 'null'::jsonb AS json_null", [], { tagged: true });
  assert.ok(row);
  assert.equal(row.sql_null?.isNull, true);
  assert.equal(row.json_null?.isNull, false);
  assert.equal(row.json_null?.kind, "json");
});

// [spec:pgorm:req:bindings.value-tags+1/test]
// [spec:pgorm:req:napi.records/test]
test("tagged rows carry each column's kind, an integer's width and an enum's type included", async () => {
  const [row] = await db().query(
    "SELECT 1::int2 AS a, 1::int8 AS b, NULL::int4 AS c, 'calm'::app.mood AS d, ARRAY[]::int4[] AS e, NULL::int4[] AS f",
    [],
    { tagged: true },
  );
  assert.ok(row);
  for (const value of Object.values(row)) assert.ok(value instanceof Value);
  assert.deepEqual(Object.values(row).map((value) => value.kind), ["i16", "i64", "i32", "enum", "array", "array"]);
  assert.equal(row.a?.value, 1);
  assert.equal(row.b?.value, 1n);
  assert.equal(row.c?.isNull, true);
  assert.deepEqual(row.d?.typeName, new TypeName("mood", { schema: "app" }));
  assert.equal(row.d?.value, "calm");
  assert.equal(row.e?.elementType, "i32");
  assert.deepEqual(row.e?.items(), []);
  assert.equal(row.f?.isNull, true);
  assert.equal(row.f?.elementType, "i32");
  assert.equal(row.f?.items(), null);
});

// [spec:pgorm:req:bindings.value-tags+1/test]
test("an enum label binds as a string and reads back with its type", async () => {
  assert.equal(await roundTrip("app.mood", "calm"), "calm");
  assert.equal(await roundTrip("app.mood", new Value('a "quoted", label', mood)), 'a "quoted", label');
  same(await roundTrip("app.mood[]", Value.array(mood, ["tense", null])), ["tense", null]);
  same(await roundTrip("app.mood[]", ["calm"]), ["calm"]);
  const value = new Value("calm", mood);
  assert.equal(value.kind, "enum");
  assert.deepEqual(value.typeName, mood);
  assert.deepEqual(Value.array(mood, ["calm"]).typeName, mood);
  assert.deepEqual(Value.array(mood, null).typeName, mood);
  assert.equal(Value.array("i32", [1]).typeName, null);
  await assert.rejects(roundTrip("app.mood", "furious"), (error: unknown) => {
    assert.ok(error instanceof DatabaseError);
    assert.equal(error.sqlstate, "22P02");
    return true;
  });
  throwsConstruction(() => new TypeName(""), /1–63 UTF-8 bytes/);
  throwsConstruction(() => new TypeName("x".repeat(64)), /1–63 UTF-8 bytes/);
});

// [spec:pgorm:req:bindings.value-tags+1/test]
test("an empty array and an SQL NULL array still name their element kind", async () => {
  const empty = Value.array("i32", []);
  assert.equal(empty.kind, "array");
  assert.equal(empty.elementType, "i32");
  same(await roundTrip("int4[]", empty), []);
  same(await roundTrip("int4[]", Value.array("i32", null)), null);
  const items = Value.array("i16", [1, null, 3]).items();
  assert.deepEqual(items?.map((item) => [item.kind, item.value]), [["i16", 1], ["i16", null], ["i16", 3]]);
  same(await roundTrip("interval[]", Value.array("interval", [new Interval({ days: 1 }), null])), [
    new Interval({ days: 1 }),
    null,
  ]);
  same(await roundTrip("interval[]", [new Interval({ months: -1 })]), [new Interval({ months: -1 })]);
  throwsConstruction(() => Value.array("i32", [new Value(1, "i64")]), /i64 value is not a value of kind i32/);
});

// [spec:pgorm:req:bindings.created-ranges+3/test]
test("a created range converts each bound through its subtype and travels as its text", async () => {
  const value = new Value(new Range(1.5, 2.5), floatrange);
  assert.equal(value.kind, "created_range");
  assert.deepEqual(value.createdType, floatrange);
  assert.deepEqual(value.typeName, new TypeName("floatrange", { schema: "app" }));
  same(value.value, new Range(1.5, 2.5));
  const read = await roundTrip("text", value);
  assert.equal(read, "[1.5,2.5)");
  same((await one("SELECT CAST($1::text AS app.floatrange) AS v", [value])).v, new Range(1.5, 2.5));
  same(new Value("[1.5, 2.5]", floatrange).value, new Range(1.5, 2.5, "[]"));
  const [tagged] = await db().query("SELECT '[1,5]'::app.slot AS v", [], { tagged: true });
  assert.equal(tagged?.v?.kind, "created_range");
  assert.deepEqual(tagged?.v?.createdType, new CreatedRange("slot", "i32", { schema: "app" }));
  same(tagged?.v?.value, new Range(1, 5, "[]"));
  const multi = new Value(new Multirange([new Range(1, 3)]), floatmultirange);
  assert.equal(multi.kind, "created_multirange");
  same(multi.value, new Multirange([new Range(1, 3)]));
  same(
    (await one("SELECT CAST($1::text AS app.floatmultirange)::text AS v", [multi])).v,
    "{[1,3)}",
  );
  throwsConstruction(() => new Value(new Range<unknown>(1n, 2), floatrange), /f64 takes a number/);
  throwsConstruction(() => new CreatedRange("r", "bool" as "i32"), /subtype/);
});

// [spec:pgorm:req:bindings.created-ranges+3/test]    an array of a created range is a value of its own
// kind both ways: each element held as its text, bound as the array of the texts, and decoded element by
// element through the subtype's codec; a query's array of a created multirange, reported with no subtype,
// is undecodable
test("an array of a created range holds each element as its text and decodes element by element", async () => {
  const array = Value.array(floatrange, [new Range(1.5, 2.5), null, "[3,4]", new Value(new Range(5, 6), floatrange)]);
  assert.equal(array.kind, "array");
  assert.deepEqual(array.elementType, floatrange);
  assert.deepEqual(array.typeName, new TypeName("floatrange", { schema: "app" }));
  assert.equal(array.createdType, null);
  same(array.value, [new Range(1.5, 2.5), null, new Range(3, 4, "[]"), new Range(5, 6)]);
  assert.deepEqual(array.items()?.map((item) => item.kind), Array(4).fill("created_range"));
  same(Value.array(floatrange, []).value, []);
  assert.equal(Value.array(floatrange, null).isNull, true);
  same(Value.array(floatmultirange, [new Multirange([new Range(1, 2)])]).value, [new Multirange([new Range(1, 2)])]);
  assert.equal(statementCol("x").eq(array).inspect().sql, 'SELECT "x" = CAST($1::text[] AS app.floatrange[])');
  const [tagged] = await db().query("SELECT $1::app.floatrange[] AS v, '{}'::app.floatrange[] AS e, NULL::app.floatrange[] AS n", [array], { tagged: true });
  assert.equal(tagged?.v?.kind, "array");
  assert.deepEqual(tagged?.v?.elementType, floatrange);
  same(tagged?.v?.value, [new Range(1.5, 2.5), null, new Range(3, 4, "[]"), new Range(5, 6)]);
  same(tagged?.e?.value, []);
  assert.equal(tagged?.n?.isNull, true);
  throwsConstruction(() => Value.array(floatrange, [new Value(new Range(1, 2), new CreatedRange("other", "f64"))]));
  await undecodable(`SELECT '{"{[1,2)}"}'::app.floatmultirange[] AS v`);
});

// [spec:pgorm:req:bindings.value-tags+1/test]
test("a Value converts at construction, can be bound again, and hands out independent copies", async () => {
  const value = new Value([1, 2, 3]);
  assert.equal(value.kind, "array");
  assert.equal(value.elementType, "i64");
  const copy = value.value as number[];
  copy.push(4);
  same(value.value, [1n, 2n, 3n]);
  same(await roundTrip("int8[]", value), [1n, 2n, 3n]);
  same(await roundTrip("int8[]", value), [1n, 2n, 3n]);
  assert.ok(new Value(5, "i32").equals(new Value(5, "i32")));
  assert.ok(!new Value(5, "i32").equals(new Value(5, "i64")));
  assert.ok(!new Value(0, "f64").equals(new Value(-0, "f64")));
  assert.ok(new Value(NaN, "f64").equals(new Value(NaN, "f64")));
  assert.equal(new Value(value).kind, "array");
  throwsConstruction(() => new Value(null), /Value\.null/);
  throwsConstruction(() => new Value(1, "no-such-kind" as "i32"), /not a value kind/);
});

// [spec:pgorm:req:bindings.values/test]
test("a domain decodes and binds as the type it is built over", async () => {
  assert.equal((await one("SELECT 'a@b'::app.email AS v")).v, "a@b");
  const [height] = await db().query("SELECT '[1.5,2)'::app.heightrange AS v", [], { tagged: true });
  assert.deepEqual(height?.v?.createdType, new CreatedRange("heightrange", "f64", { schema: "app" }));
  same(height?.v?.value, new Range(1.5, 2));
  assert.equal(await roundTrip("app.email", "c@d"), "c@d");
  same((await one("SELECT ARRAY['a@b'::app.email, NULL] AS v")).v, ["a@b", null]);
  same(await roundTrip("app.span", new Interval({ days: 2 })), new Interval({ days: 2 }));
});

// [spec:pgorm:req:napi.inference/test]
test("a safe-integer number binds to integer columns exactly, any other number as a float", async () => {
  assert.equal(await roundTrip("int2", 7), 7);
  assert.equal(await roundTrip("int8", -(2 ** 53) + 1), -(2n ** 53n) + 1n);
  assert.equal(String(await roundTrip("numeric", 42)), "42");
  assert.equal(await roundTrip("float8", 3), 3);
  await refused(roundTrip("int4", 1.5), /without loss/);
  assert.ok(Object.is(await roundTrip("float8", -0), -0));
  await refused(roundTrip("int4", -0), /without loss/);
  assert.equal(await roundTrip("int8", 9007199254740993n), 9007199254740993n);
  await refused(roundTrip("int8", 2n ** 63n), /outside i64/);
  await refused(roundTrip("text", 1), /cannot bind a `BigInt` value to Postgres type `text`/);
});

// [spec:pgorm:req:napi.inference/test]
test("undefined, a function, a symbol and an unknown object are refused; null is NULL", async () => {
  await refused(roundTrip("int4", undefined as unknown as Param), /undefined is no SQL value/);
  await refused(roundTrip("int4", (() => 1) as unknown as Param), /cannot be inferred/);
  await refused(roundTrip("int4", Symbol("x") as unknown as Param), /cannot be inferred/);
  await refused(roundTrip("jsonb", new Map() as unknown as Param), /cannot be inferred/);
  assert.equal(await roundTrip("int4", null), null);
});

// [spec:pgorm:req:napi.temporal+1/test]
test("date, time and timestamp read as Temporal plain values, to the microsecond", async () => {
  for (const date of ["2026-10-09", "0001-01-01", "9999-12-31", "-000100-01-01"]) {
    same(await roundTrip("date", Temporal.PlainDate.from(date)), Temporal.PlainDate.from(date));
  }
  same((await one("SELECT '0101-01-01 BC'::date AS v")).v, Temporal.PlainDate.from("-000100-01-01"));
  same(await roundTrip("time", Temporal.PlainTime.from("23:59:59.999999")), Temporal.PlainTime.from("23:59:59.999999"));
  same((await one("SELECT '12:34:56.000001'::time AS v")).v, Temporal.PlainTime.from("12:34:56.000001"));
  const datetime = Temporal.PlainDateTime.from("2026-10-09T12:34:56.123456");
  same(await roundTrip("timestamp", datetime), datetime);
  same(await roundTrip("date", Temporal.PlainDate.from("2026-10-09").withCalendar("japanese")), Temporal.PlainDate.from("2026-10-09"));
});

// [spec:pgorm:req:napi.temporal+1/test]
test("timestamptz reads as the same Temporal.Instant whatever the session time zone", async () => {
  const instant = Temporal.Instant.from("2026-01-01T12:00:00.000001Z");
  same(await roundTrip("timestamptz", instant), instant);
  same((await one("SELECT '2026-01-01 12:00:00+05:30'::timestamptz AS v")).v, Temporal.Instant.from("2026-01-01T06:30:00Z"));
  const row = await one(
    "SELECT x.v, x.t FROM (SELECT set_config('TimeZone', 'Asia/Kolkata', true) AS z) AS s, " +
      "LATERAL (SELECT $1::timestamptz AS v, $1::timestamptz::text AS t, s.z) AS x",
    [instant],
  );
  same(row.v, instant);
  assert.equal(row.t, "2026-01-01 17:30:00.000001+05:30");
});

// [spec:pgorm:req:napi.temporal+1/test]
test("a value finer than PostgreSQL's microseconds is refused, never truncated", async () => {
  await refused(roundTrip("time", Temporal.PlainTime.from("12:00:00.0000001")), /sub-microsecond/);
  await refused(roundTrip("timestamp", Temporal.PlainDateTime.from("2026-01-01T00:00:00.000000001")), /sub-microsecond/);
  const fine = Temporal.Instant.fromEpochNanoseconds(1_000_000_001n);
  await refused(roundTrip("timestamptz", fine), /sub-microsecond/);
  await refused(roundTrip("interval", Temporal.Duration.from({ nanoseconds: 1 })), /sub-microsecond/);
  const rounded = fine.round({ smallestUnit: "microsecond", roundingMode: "halfExpand" });
  same(await roundTrip("timestamptz", rounded), Temporal.Instant.fromEpochNanoseconds(1_000_000_000n));
});

// [spec:pgorm:req:napi.temporal+1/test]
test("a civil date-time and an instant bind only to their own type", async () => {
  await refused(roundTrip("timestamptz", Temporal.PlainDateTime.from("2026-01-01T00:00")), /does not bind to timestamptz/);
  await refused(roundTrip("timestamp", Temporal.Instant.from("2026-01-01T00:00Z")), /does not bind to timestamp/);
  await refused(roundTrip("timestamptz[]", [Temporal.PlainDateTime.from("2026-01-01T00:00")]), /timestamptz/);
  const civil = new Value(new Range(Temporal.PlainDateTime.from("2026-01-01T00:00"), null), "tsrange");
  await refused(roundTrip("tstzrange", civil), /timestamptz/);
  await refused(roundTrip("timestamptz", Temporal.ZonedDateTime.from("2026-01-01T00:00[Europe/Paris]")), /ZonedDateTime/);
  await refused(roundTrip("timestamptz", new Date(0) as unknown as Param), /a Date is refused/);
  throwsConstruction(() => new Value(new Date(0), "datetime_utc"), /a Date is refused/);
});

// [spec:pgorm:req:napi.temporal+1/test]
// [spec:pgorm:req:bindings.values/test]
test("dates beyond pgorm's years and PostgreSQL's infinities are refused both ways", async () => {
  await refused(roundTrip("date", new Temporal.PlainDate(10000, 1, 1)), /-9999 to 9999/);
  await undecodable("SELECT '10000-01-01'::date AS v");
  await undecodable("SELECT 'infinity'::timestamptz AS v");
  await undecodable("SELECT 'infinity'::date AS v");
  await undecodable("SELECT '24:00'::time AS v");
});

// [spec:pgorm:req:bindings.values/test]
// [spec:pgorm:req:napi.temporal+1/test]
test("an interval keeps its months, days and microseconds, each with its own sign", async () => {
  same((await one("SELECT '1 mon -2 days 03:00:00.000001'::interval AS v")).v, new Interval({
    months: 1,
    days: -2,
    microseconds: 10_800_000_001n,
  }));
  const mixed = new Interval({ months: -14, days: 3, microseconds: -3_723_000_450n });
  same(await roundTrip("interval", mixed), mixed);
  const iso = await one(
    "SELECT x.t FROM (SELECT set_config('IntervalStyle', 'iso_8601', true) AS z) AS s, " +
      "LATERAL (SELECT $1::interval::text AS t, s.z) AS x",
    [mixed],
  );
  assert.equal(iso.t, mixed.toString());
  assert.throws(() => mixed.toDuration(), RangeError);
  const duration = Temporal.Duration.from("P1Y2M3W4DT5H6M7.008009S");
  const fromDuration = await roundTrip("interval", duration);
  same(fromDuration, new Interval({ months: 14, days: 25, microseconds: 18_367_008_009n }));
  assert.equal((fromDuration as Interval).toDuration().toString(), "P14M25DT5H6M7.008009S");
  same(Interval.from("PT1.5S"), new Interval({ microseconds: 1_500_000n }));
  throwsConstruction(() => Interval.from("1 day"), /not an ISO 8601 duration/);
  throwsConstruction(() => new Interval({ months: 2 ** 31 }), /32-bit/);
});

// [spec:pgorm:req:bindings.values/test]
test("timetz is refused with a DecodeError naming why", async () => {
  await undecodable("SELECT '12:00+05'::timetz AS v", /no Temporal type/);
});

// [spec:pgorm:req:bindings.rows/test]
// [spec:pgorm:req:napi.records/test]
test("rows are objects keyed by column name, in column order", async () => {
  const rows = await db().query("SELECT 1 AS b, 2 AS a, 3 AS c UNION ALL SELECT 4, 5, 6");
  assert.equal(rows.length, 2);
  assert.deepEqual(rows.map((row) => Object.keys(row)), [["b", "a", "c"], ["b", "a", "c"]]);
  assert.deepEqual(rows[1], { b: 4, a: 5, c: 6 });
  assert.equal(Object.getPrototypeOf(rows[0]), Object.prototype);
  assert.deepEqual(await db().query("SELECT 1 AS a WHERE false"), []);
});

// [spec:pgorm:req:napi.records/test]
test("a column named __proto__ is a property of its row, not the row's prototype", async () => {
  const row = await one(`SELECT '{"x": 1}'::jsonb AS "__proto__", 2 AS constructor`);
  assert.equal(Object.getPrototypeOf(row), Object.prototype);
  assert.deepEqual(Object.keys(row), ["__proto__", "constructor"]);
  same(Object.getOwnPropertyDescriptor(row, "__proto__")?.value, { x: 1 });
  assert.equal(row.constructor, 2);
});

// [spec:pgorm:req:bindings.rows/test]
test("two columns of one name are a DecodeError, not one key for both", async () => {
  await undecodable("SELECT 1 AS a, 2 AS a", /more than one column named "a"/);
});

// [spec:pgorm:req:bindings.values/test]
test("every kind pgorm names converts both ways, and has a NULL", () => {
  const plain = Temporal.PlainDateTime.from("2024-02-29T12:30:15.25");
  const instant = Temporal.Instant.from("2024-02-29T12:30:15.25Z");
  const ranges: Record<string, Range> = {
    int4: new Range(1, 5),
    int8: new Range(1n, 5n),
    num: new Range(new Decimal("1.5"), new Decimal("2.5")),
    date: new Range(plain.toPlainDate(), null),
    ts: new Range(plain, null),
    tstz: new Range(null, instant),
  };
  const samples: Record<string, unknown> = {
    bool: true,
    i8: -8,
    i16: -16,
    i32: 32,
    i64: -64n,
    u32: 32,
    u64: 64n,
    f32: 1.5,
    f64: -2.5,
    text: "text",
    char: "c",
    bytes: new Uint8Array([0, 255]),
    json: { a: [1, null] },
    decimal: new Decimal("123.45"),
    uuid: new Uuid("67e55044-10b1-426f-9247-bb680e5fe0c8"),
    date: plain.toPlainDate(),
    time: plain.toPlainTime(),
    datetime: plain,
    datetime_utc: instant,
    ipnetwork: "192.0.2.7/32",
    mac_address: new Uint8Array([0, 1, 2, 3, 4, 5]),
    vector: new Float32Array([1, -0.5]),
    interval: new Interval({ months: 1, days: -2, microseconds: 3n }),
  };
  for (const [prefix, range] of Object.entries(ranges)) {
    samples[`${prefix}range`] = range;
    samples[`${prefix}multirange`] = new Multirange([range]);
  }
  const kinds: string[] = native.valueKinds();
  assert.deepEqual(new Set(kinds), new Set(Object.keys(samples)));
  for (const kind of kinds) {
    const value = new Value(samples[kind], kind as never);
    assert.ok(new Value(value.value, kind as never).equals(value), kind);
    assert.equal(Value.null(kind as never).value, null, kind);
  }
});
