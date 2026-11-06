// A declared model's columns read, written and compared through pgorm's casts
// for their types, and converted through the program's own functions.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import {
  call,
  column,
  ConstructionError,
  createDomain,
  createType,
  DataType,
  DecodeError,
  Decimal,
  type Expr,
  fromModel,
  model,
  Pool,
  TypeName,
} from "../lib/index.js";
import { mistyped, scratchDatabase } from "./support.ts";

const flavor = new TypeName("flavor", { schema: "model_types" });
const fancy = new TypeName("fancy", { schema: "model_types" });
const bits = new DataType("bit", { length: 7 });
const SHELVES = { top: 1, bottom: 2 } as const;
const BY_NUMBER: Record<number, "top" | "bottom"> = { 1: "top", 2: "bottom" };

const TipJar = model("tip_jar", {
  schema: "model_types",
  columns: {
    id: column("i32", { primaryKey: true, autoIncrement: true }),
    amount: column("decimal", { columnType: "money" }),
  },
});
const Special = model("special", {
  schema: "model_types",
  columns: {
    id: column("i32", { primaryKey: true, autoIncrement: true }),
    flavor: column(flavor, { values: ["lemon", "plain"], columnType: DataType.domain(fancy, flavor) }),
  },
});
const Opening = model("opening", {
  schema: "model_types",
  renameAll: "snake_case",
  columns: {
    bakeryId: column("i32", { primaryKey: true }),
    days: column("i64", { columnType: bits, selectAs: "bigint", saveAs: bits }),
  },
});
const Shelving = model("shelving", {
  schema: "model_types",
  columns: {
    id: column("i32", { primaryKey: true }),
    shelf: column("i16", {
      decode: (stored: number) => {
        const shelf = BY_NUMBER[stored];
        if (shelf === undefined) throw new Error(`no shelf ${stored}`);
        return shelf;
      },
      encode: (shelf: keyof typeof SHELVES) => SHELVES[shelf],
    }),
    contact: column("json", {
      nullable: true,
      decode: (doc: any) => {
        if (typeof doc?.phone !== "string") throw new Error("a contact has a phone");
        return { name: String(doc.name), phone: doc.phone as string };
      },
    }),
  },
});

let database: { pool: Pool; drop(): Promise<void> } | undefined;

before(async () => {
  const scratch = await scratchDatabase("pgorm_napi_model_casts");
  const pool = new Pool(scratch.dsn, { maxSize: 2 });
  await pool.execute("CREATE SCHEMA model_types");
  await pool.execute(createType(flavor).values(["lemon", "plain"]));
  await pool.execute(createDomain(fancy, flavor));
  for (const declared of [TipJar, Special, Opening, Shelving]) await pool.execute(fromModel(declared).table);
  database = {
    pool,
    async drop() {
      await pool.close();
      await scratch.drop();
    },
  };
});

after(async () => {
  await database?.drop();
});

function pool(): Pool {
  if (!database) throw new Error("the scratch database was not created");
  return database.pool;
}

// [spec:pgorm:req:bindings.model-casts/test]    money, a domain over an enum and modified casts are pgorm's
test("a column reads, writes and compares through its type's casts", () => {
  assert.equal(
    TipJar.find().where(TipJar.col("amount").gt(new Decimal("10"))).inspect().sql,
    `SELECT "tip_jar"."id", CAST("tip_jar"."amount" AS numeric) AS "amount" FROM "model_types"."tip_jar" ` +
      `WHERE "tip_jar"."amount" > CAST($1::numeric AS money)`,
  );
  assert.equal(
    Special.find().where(Special.col("flavor").eq("lemon")).inspect().sql,
    `SELECT "special"."id", CAST("special"."flavor" AS text) AS "flavor" FROM "model_types"."special" ` +
      `WHERE CAST("special"."flavor" AS model_types.flavor) = CAST($1::text AS model_types.flavor)`,
  );
  assert.equal(
    Opening.insert({ bakeryId: 1, days: 126n }).inspect().sql,
    `INSERT INTO "model_types"."opening" ("bakery_id", "days") VALUES ($1, CAST($2::int8 AS bit(7)))`,
  );
  const Shouted = model("t", { columns: { c: column("text", { selectWith: (c: Expr) => call("upper", c) }) } });
  assert.equal(Shouted.find().inspect().sql, `SELECT UPPER("t"."c") AS "c" FROM "t"`);
  const Slot = model("slot", {
    columns: { span: column("interval", { columnType: new DataType("interval", { fields: "hourToMinute" }) }) },
  });
  assert.equal(fromModel(Slot).table.inspect().sql, `CREATE TABLE "slot" ( "span" interval HOUR TO MINUTE NOT NULL )`);
});

// [spec:pgorm:req:bindings.model-casts/test]    each refusal comes before anything is sent
test("what the casts cannot say is refused", () => {
  const refusals = [
    () => column(flavor, { columnType: "text" }),
    () => column(flavor, { columnType: DataType.domain(fancy, "integer") }),
    () => column("text", { selectAs: "text", selectWith: (c: Expr) => c }),
    () => column("text", { saveAs: "text", saveWith: (v: Expr) => v }),
    () => column("interval", { selectAs: new DataType("interval", { fields: "hour" }) }),
    () => new DataType("interval", { fields: "hourToMinute", precision: 3 }),
  ];
  for (const [index, refusal] of refusals.entries()) {
    assert.throws(refusal, (error: unknown) => error instanceof ConstructionError, `refusal ${index}`);
  }
  // @ts-expect-error: selectWith is a function of the column's expression
  assert.throws(() => column("text", { selectWith: "upper" }), mistyped(/selectWith is a function/));
  // @ts-expect-error: no interval has fortnight fields
  assert.throws(() => new DataType("interval", { fields: "fortnight" }), mistyped(/fields are/));
});

// [spec:pgorm:req:bindings.model-casts/test]    the casts run against the server
test("money rounds, a domain over an enum compares, and bit(7) round-trips", async () => {
  const db = pool();
  const tip = await TipJar.insert({ amount: new Decimal("12.345") }).returning().one(db);
  assert.equal(tip.amount.toString(), "12.35");
  assert.equal(await TipJar.find().where(TipJar.col("amount").gt(new Decimal("10"))).count(db), 1);
  await Special.insertMany([{ flavor: "lemon" }, { flavor: "plain" }]).execute(db);
  const lemons = await Special.find().where(Special.col("flavor").eq("lemon")).all(db);
  assert.deepEqual(lemons.map((row) => row.flavor), ["lemon"]);
  const week = await Opening.insert({ bakeryId: 1, days: 0b111_1110n }).returning().one(db);
  assert.equal(week.days, 0b111_1110n);
  const later = await Opening.find().cursor("days").after(1n).all(db);
  assert.deepEqual(later.map((row) => row.bakeryId), [1]);
});

// [spec:pgorm:req:bindings.model-conversions/test]    decode reads the program's type, checked as it is read, and encode writes it
test("decode and encode convert a column's values", async () => {
  const db = pool();
  const row = await Shelving.insert({ id: 1, shelf: "bottom", contact: { name: "Kim", phone: "555" } }).returning().one(db);
  assert.equal(row.shelf, "bottom");
  const phone: string | undefined = row.contact?.phone;
  assert.equal(phone, "555");
  await Shelving.insert({ id: 2, shelf: "top", contact: null }).execute(db);
  const tops = await Shelving.find().where(Shelving.col("shelf").eq("top")).all(db);
  assert.deepEqual(tops.map((top) => [top.id, top.contact]), [[2, null]]);
  await db.execute(`INSERT INTO model_types.shelving (id, shelf, contact) VALUES (3, 9, '{"name": "x"}')`);
  await assert.rejects(
    Shelving.findByKey({ id: 3 }).one(db),
    (error: unknown) => error instanceof DecodeError,
  );
});
