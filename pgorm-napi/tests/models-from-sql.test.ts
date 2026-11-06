// Declared models generated from DDL: pgorm-codegen's TypeScript writer
// behind `modelsFromSql`, the committed module it writes for pgorm-codegen's
// fixture schema, which `deno check` holds to its types, and rows written and
// read through that module on a database the schema creates, in either
// runtime.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { after, before, test } from "node:test";

import {
  CheckViolationError,
  ConstructionError,
  currentDate,
  currentTime,
  currentTimestamp,
  Decimal,
  Interval,
  modelsFromSql,
  Multirange,
  Pool,
  Range,
  Uuid,
} from "../lib/index.js";
import * as m from "./models-from-sql/models.ts";
import { same, scratchDatabase } from "./support.ts";

const schema = readFileSync(new URL("../../pgorm-codegen/tests/models/schema.sql", import.meta.url), "utf8");
const committed = readFileSync(new URL("./models-from-sql/models.ts", import.meta.url), "utf8");

let database: { dsn: string; drop(): Promise<void> } | undefined;
let shared: Pool | undefined;

function pool(): Pool {
  if (!shared) throw new Error("the scratch database was not created");
  return shared;
}

before(async () => {
  database = await scratchDatabase("pgorm_napi_models_from_sql");
  shared = new Pool(database.dsn, { maxSize: 2 });
  await shared.batchExecute(schema);
});

after(async () => {
  await shared?.close();
  await database?.drop();
});

// [spec:pgorm:req:bindings.models-from-sql/test]    modelsFromSql writes pgorm-codegen's TypeScript module for the DDL, the committed module byte for byte, which imports as declared models
test("the module is pgorm-codegen's", () => {
  assert.equal(modelsFromSql(schema), committed);
  assert.deepEqual(m.Cake.describe().primaryKey, ["id"]);
  assert.equal(m.Sample.describe().fields.flavors?.kind, `enum "public"."flavor"[]`);
  assert.deepEqual(m.PaymentDisjoint.describe().kindType, { schema: "public", name: "payment_kind" });
  assert.deepEqual(m.ReviewSubject.describe().targets, [
    { value: "cake", field: "cakeId", target: "cake" },
    { value: "bakery", field: "bakeryId", target: "bakery" },
  ]);
});

// [spec:pgorm:req:bindings.models-from-sql/test]    schemaName and unkindedDisjoint reach pgorm-codegen, each table a name or a [schema, name] pair; a refusal is a ConstructionError with its message and a wrong argument a TypeError
test("options reach pgorm-codegen, and its refusals are ConstructionErrors", () => {
  const ddl = "CREATE TABLE shop (id int PRIMARY KEY); " +
    "CREATE TABLE web_shop (shop_id int PRIMARY KEY REFERENCES shop (id));";
  const source = modelsFromSql(ddl, {
    schemaName: "app",
    unkindedDisjoint: [{ base: "shop", kinds: [["app", "web_shop"]], unit: "dormant" }],
  });
  assert.match(source, /schema: "app"/);
  assert.match(source, /unkinded: true/);
  assert.throws(
    () => modelsFromSql("CREATE TRIGGER t AFTER INSERT ON a EXECUTE FUNCTION f();"),
    (error: unknown) =>
      error instanceof ConstructionError && error.message === "unsupported DDL: CREATE TRIGGER at statement 1",
  );
  assert.throws(
    () => modelsFromSql("CREATE TABLE a (id int PRIMARY KEY, note text DEFAULT NULL);"),
    (error: unknown) => error instanceof ConstructionError && /defaults to NULL/.test(error.message),
  );
  // @ts-expect-error: the DDL is text
  assert.throws(() => modelsFromSql(1), TypeError);
  // @ts-expect-error: a schema is named by a string
  assert.throws(() => modelsFromSql("", { schemaName: 1 }), TypeError);
  // @ts-expect-error: a table is a name or a [schema, name] pair
  assert.throws(() => modelsFromSql("", { unkindedDisjoint: [{ base: 1, kinds: [] }] }), TypeError);
});

// [spec:pgorm:req:bindings.models-from-sql/test]    cli.js models writes the module to the file --output names
test("the command line writes the module", () => {
  const directory = mkdtempSync(join(tmpdir(), "pgorm-models-"));
  try {
    const output = join(directory, "models.ts");
    const cli = fileURLToPath(new URL("../codegen/cli.js", import.meta.url));
    const schemaFile = fileURLToPath(new URL("../../pgorm-codegen/tests/models/schema.sql", import.meta.url));
    const ran = spawnSync("node", [cli, "models", schemaFile, "--output", output], { encoding: "utf8" });
    assert.equal(ran.status, 0, ran.stderr);
    assert.equal(readFileSync(output, "utf8"), committed);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

// [spec:pgorm:req:bindings.current-keywords/test]    currentDate(), currentTime() and currentTimestamp() are the SQL keywords
test("the current keywords are SQL's", () => {
  assert.equal(currentDate().inspect().sql, "CURRENT_DATE");
  assert.equal(currentTime().inspect().sql, "CURRENT_TIME");
  assert.equal(currentTimestamp().inspect().sql, "CURRENT_TIMESTAMP");
});

// [spec:pgorm:req:bindings.models-from-sql/test]    the generated models write and read rows of every column type they declare, an enum, a domain over an enum and money among them, and follow the relations and the junction path
test("rows round-trip through the generated models", async () => {
  const bakery = await m.Bakery.insert({ name: "Sunrise", city: "Oslo" }).returning().one(pool());
  const cake = await m.Cake.insert({
    name: "Lemon Drizzle",
    flavor: "lemon",
    special: "lemon",
    price: new Decimal("3.20"),
    tip: new Decimal("0.50"),
    bakeryId: bakery.id,
  }).returning().one(pool());
  same([cake.special, cake.price, cake.tip, cake.glutenFree], ["lemon", new Decimal("3.20"), new Decimal("0.50"), false]);
  const lemon = await m.Cake.find().where(m.Cake.col("special").eq("lemon")).all(pool());
  assert.deepEqual(lemon.map((row) => row.id), [cake.id]);
  const baker = await m.Baker.insert({ name: "Ada", email: "ada@example.com" }).returning().one(pool());
  await m.CakeBaker.insert({ cakeId: cake.id, bakerId: baker.id }).execute(pool());
  const joined = await m.Cake.find().join(m.cakeBakery).where(m.Bakery.col("city").eq("Oslo")).all(pool());
  assert.deepEqual(joined.map((row) => row.name), ["Lemon Drizzle"]);
  const bakers = await m.cakeRelatedBaker.find(cake).all(pool());
  assert.deepEqual(bakers.map((row) => row.name), ["Ada"]);

  const values = {
    small: 7,
    whole: 70,
    big: 7_000_000_000n,
    single: 1.5,
    double: 2.25,
    amount: new Decimal("10.5"),
    price: new Decimal("1.250"),
    tip: new Decimal("2.00"),
    ok: true,
    body: "text",
    label: "a label",
    code: "C-1",
    initials: "ADA",
    raw: new Uint8Array([0, 1]),
    doc: { a: [1, 2] },
    docb: { b: null },
    ref: new Uuid("6f1c4b7e-7a3e-4c55-9f43-3f1f0b6a2c10"),
    day: Temporal.PlainDate.from("2026-11-06"),
    atTime: Temporal.PlainTime.from("09:30"),
    at: Temporal.PlainDateTime.from("2026-11-06T09:30"),
    atUtc: Temporal.Instant.from("2026-11-06T09:30:00Z"),
    span: new Interval({ days: 2 }),
    shortSpan: new Interval({ microseconds: 5_400_000_000n }),
    ints: new Range(1, 5),
    bigs: new Range(1n, 9_000_000_000n),
    nums: new Range(new Decimal("1.5"), new Decimal("2.5")),
    days: new Range(Temporal.PlainDate.from("2026-01-01"), Temporal.PlainDate.from("2026-02-01")),
    stamps: new Range(Temporal.PlainDateTime.from("2026-01-01T00:00"), Temporal.PlainDateTime.from("2026-01-02T00:00")),
    instants: new Range(Temporal.Instant.from("2026-01-01T00:00:00Z"), Temporal.Instant.from("2026-01-02T00:00:00Z")),
    intSets: new Multirange([new Range(1, 3), new Range(5, 7)]),
    daySets: new Multirange([new Range(Temporal.PlainDate.from("2026-01-01"), Temporal.PlainDate.from("2026-01-03"))]),
    heat: new Range(180.0, 250.5),
    flavor: "vanilla",
    special: "carrot",
    flavors: ["lemon", null, "chocolate"],
    tags: ["x", "y"],
    counts: [1, null, 3],
    weight: 250,
  } as const;
  const sample = await m.Sample.insert(values).returning().one(pool());
  const read = await m.Sample.findByKey({ id: sample.id }).one(pool());
  for (const [field, value] of Object.entries(values)) {
    same(read[field as keyof typeof read], value, field);
  }
  same([read.quantity, read.note, read.doubled], [1, "none", 2]);
  assert.ok(read.created instanceof Temporal.Instant);

  const [base, kind, card] = await m.PaymentDisjoint.insert(
    { orderId: 1, amount: new Decimal("12.20") },
    "card",
    { lastFour: "4242" },
  ).returningModel(pool());
  assert.deepEqual([kind, base.kind], ["card", "card"]);
  assert.deepEqual(card, { paymentId: base.id, lastFour: "4242" });
  await m.PaymentDisjoint.insert({ orderId: 2, amount: new Decimal("3.80") }, "cash").returningModel(pool());
  const payments = await m.PaymentDisjoint.find().orderBy(m.Payment.col("id").asc()).all(pool());
  assert.deepEqual(payments.map(([, kind]) => kind), ["card", "cash"]);

  const review = await m.Review.insert({ stars: 5, body: "Lovely", ...m.ReviewSubject.set("cake", cake.id) })
    .returning().one(pool());
  assert.deepEqual(m.ReviewSubject.key(review), ["cake", cake.id]);
  const [loaded] = await m.ReviewSubject.load(pool(), [review]);
  assert.deepEqual([loaded?.[0], loaded?.[1].name], ["cake", "Lemon Drizzle"]);
  await assert.rejects(
    m.Review.insert({ stars: 1, body: "Both", cakeId: cake.id, bakeryId: bakery.id }).execute(pool()),
    CheckViolationError,
  );
});
