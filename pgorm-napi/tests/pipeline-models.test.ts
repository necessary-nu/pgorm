// Declared models in pgorm's pipeline, against a live server in either
// runtime: a model as a source, its fields as typed columns compared by
// kind, selectSources decoding each model's records with null for an absent
// side, and the staged reads intoModel and intoTuple.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import {
  column,
  ConstructionError,
  Decimal,
  DecodeError,
  model,
  pipeline as pl,
  Pool,
  TypeName,
  Value,
} from "../lib/index.js";
import { scratchDatabase } from "./support.ts";

let database: { dsn: string; drop(): Promise<void> } | undefined;
let shared: Pool | undefined;

function pool(): Pool {
  if (!shared) throw new Error("the scratch database was not created");
  return shared;
}

const flavor = new TypeName("flavor", { schema: "app" });

const Bakery = model("bakery", {
  schema: "app",
  columns: {
    id: column("i32", { primaryKey: true }),
    name: column("text"),
    city: column("text"),
  },
});

const Cake = model("cake", {
  schema: "app",
  columns: {
    id: column("i32", { primaryKey: true }),
    name: column("text"),
    flavor: column(flavor, { values: ["lemon", "carrot", "chocolate"] }),
    price: column("decimal"),
    bakeryId: column("i32", { name: "bakery_id", nullable: true }),
  },
});

before(async () => {
  database = await scratchDatabase("pgorm_napi_pipeline_models");
  shared = new Pool(database.dsn, { maxSize: 4 });
  for (
    const sql of [
      "CREATE SCHEMA app",
      "CREATE TYPE app.flavor AS ENUM ('lemon', 'carrot', 'chocolate')",
      "CREATE TABLE app.bakery (id int4 PRIMARY KEY, name text NOT NULL, city text NOT NULL)",
      "CREATE TABLE app.cake (id int4 PRIMARY KEY, name text NOT NULL, flavor app.flavor NOT NULL, " +
      "price numeric(10, 2) NOT NULL, bakery_id int4 REFERENCES app.bakery (id))",
      "INSERT INTO app.bakery VALUES (1, 'Sunrise', 'Oslo'), (2, 'Moonlight', 'Bergen'), (3, 'Noon', 'Tromsø')",
      "INSERT INTO app.cake VALUES (1, 'Fudge', 'chocolate', 4.50, 1), (2, 'Drizzle', 'lemon', 3.20, 1), " +
      "(3, 'Carrot', 'carrot', 3.50, 2), (4, 'Orphan', 'lemon', 2.00, NULL)",
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

// [spec:pgorm:req:bindings.pipeline-models/test]
test("a model is a pipeline source, read under its alias, and its fields are columns by SQL name", async () => {
  const cheap = pl.from(Cake).filter(pl.col(Cake, "price").lt(new Decimal("4"))).sort(pl.col(Cake, "id"))
    .select(pl.col(Cake, "name"), pl.col(Cake, "bakeryId"));
  assert.equal(
    cheap.inspect().sql,
    "WITH table_0 AS (SELECT name, bakery_id, id FROM app.cake WHERE price < $1) " +
      "SELECT name, bakery_id FROM table_0 ORDER BY id",
  );
  assert.deepStrictEqual(await cheap.all(pool()), [
    { name: "Drizzle", bakery_id: 1 },
    { name: "Carrot", bakery_id: 2 },
    { name: "Orphan", bakery_id: null },
  ]);

  const other = Cake.as("other");
  const pairs = pl.from(Cake)
    .join(other, pl.col(Cake, "bakeryId").eq(pl.col(other, "bakeryId")))
    .filter(pl.col(Cake, "id").lt(pl.col(other, "id")))
    .select(pl.col(Cake, "name").as("earlier"), pl.col(other, "name").as("later"));
  assert.match(pairs.inspect().sql, /INNER JOIN table_0 AS other ON cake\.bakery_id = other\.bakery_id/);
  assert.deepStrictEqual(await pairs.all(pool()), [{ earlier: "Fudge", later: "Drizzle" }]);

  assert.throws(() => pl.col(Cake, "nope" as never), refused(/no field "nope"/));
});

// [spec:pgorm:req:bindings.pipeline-models/test]
test("a model column converts what it is compared with through its declaration and casts it as its kind", async () => {
  const lemons = pl.from(Cake).filter(pl.col(Cake, "flavor").eq("lemon")).sort(pl.col(Cake, "id"))
    .select(pl.col(Cake, "id"));
  const compiled = lemons.inspect();
  assert.match(compiled.sql, /flavor = CAST\(\$1::text AS app\.flavor\)/);
  assert.deepStrictEqual((await lemons.all(pool())).map((row) => row.id), [2, 4]);

  const listed = pl.from(Cake).filter(pl.col(Cake, "flavor").inArray(["carrot", "chocolate"])).sort(pl.col(Cake, "id"))
    .select(pl.col(Cake, "id"));
  assert.deepStrictEqual((await listed.all(pool())).map((row) => row.id), [1, 3]);

  const tagged = pl.from(Cake).filter(pl.col(Cake, "flavor").ne(new Value("lemon", flavor))).select(pl.col(Cake, "id"));
  assert.equal((await tagged.all(pool())).length, 2);

  const priced = pl.from(Cake).filter(pl.col(Cake, "price").gte(new Decimal("4"))).select(pl.col(Cake, "name"));
  assert.deepStrictEqual(await priced.all(pool()), [{ name: "Fudge" }]);

  // A pipeline expression is compared as written, and an operator's result is a plain expression.
  const written = pl.col(Cake, "flavor").eq(pl.literal("carrot"));
  assert.match(pl.from(Cake).filter(written).inspect().sql, /flavor = CAST\('carrot' AS app\.flavor\)/);

  // @ts-expect-error: "banana" is not one of the flavor's labels
  assert.throws(() => pl.col(Cake, "flavor").eq("banana"), refused(/not one of flavor's values/));
  // @ts-expect-error: a comparison with null is never true
  assert.throws(() => pl.col(Cake, "bakeryId").eq(null), refused(/never true/));
  // @ts-expect-error: a decimal column takes a Decimal
  assert.throws(() => pl.col(Cake, "price").gt("cheap"), (error) => error instanceof Error);
});

// [spec:pgorm:req:bindings.pipeline-models/test]
test("selectSources over models decodes each model's record, null where a join left its side absent", async () => {
  const both = pl.from(Cake)
    .join(Bakery, pl.col(Cake, "bakeryId").eq(pl.col(Bakery, "id")), { kind: "full" })
    .sort(pl.col(Bakery, "id"), pl.col(Cake, "id"))
    .selectSources(Cake, Bakery);
  assert.match(both.inspect().sql, /SELECT cake\.id AS s0_id, cake\.name AS s0_name, cake\.flavor AS s0_flavor/);
  const rows = await both.all(pool());
  assert.deepStrictEqual(rows.map(([cake, bakery]) => [cake?.name ?? null, bakery?.name ?? null]), [
    ["Fudge", "Sunrise"],
    ["Drizzle", "Sunrise"],
    ["Carrot", "Moonlight"],
    [null, "Noon"],
    ["Orphan", null],
  ]);
  const first = rows[0]![0]!;
  assert.deepStrictEqual(first, { id: 1, name: "Fudge", flavor: "chocolate", price: first.price, bakeryId: 1 });
  assert.equal(first.price.toString(), "4.50");

  const one = await pl.from(Cake).sort(pl.col(Cake, "id")).selectSources(Cake).one(pool());
  assert.equal(one[0]?.name, "Fudge");
  assert.match(pl.from(Cake).selectSources(Cake).inspect("one").sql, /LIMIT 1/);
  assert.equal(await pl.from(Cake).filter(pl.col(Cake, "id").lt(0)).selectSources(Cake).optional(pool()), null);
  await assert.rejects(
    pl.from(Cake).filter(pl.col(Cake, "id").lt(0)).selectSources(Cake).one(pool()),
    (error) => error instanceof DecodeError,
  );

  const other = Cake.as("other");
  const sameBakery = pl.from(Cake)
    .join(other, pl.col(Cake, "bakeryId").eq(pl.col(other, "bakeryId")))
    .filter(pl.col(Cake, "id").lt(pl.col(other, "id")))
    .selectSources(Cake, other);
  assert.deepStrictEqual((await sameBakery.all(pool())).map(([a, b]) => [a?.name, b?.name]), [["Fudge", "Drizzle"]]);

  const reshaped = pl.from(Cake).select(pl.col(Cake, "id")).selectSources(Cake);
  assert.throws(() => reshaped.inspect(), refused(/select_sources after `select`/));
  await assert.rejects(reshaped.all(pool()), refused(/select_sources after `select`/));
  // @ts-expect-error: selectSources lists a source
  assert.throws(() => pl.from(Cake).selectSources(), refused(/at least one source/));
});

// [spec:pgorm:req:bindings.pipeline-models/test]
test("intoModel reads a model's records or a function's results, and intoTuple reads by position", async () => {
  const lemons = pl.from(Cake).filter(pl.col(Cake, "flavor").eq("lemon")).sort(pl.col(Cake, "id"));
  const records = await lemons.intoModel(Cake).all(pool());
  assert.deepStrictEqual(records.map((cake) => [cake.id, cake.bakeryId, cake.flavor]), [[2, 1, "lemon"], [4, null, "lemon"]]);
  assert.equal((await lemons.intoModel(Cake).one(pool())).name, "Drizzle");

  const named = await lemons.select(pl.col(Cake, "name")).intoModel((row) => `cake ${row.name}`).all(pool());
  assert.deepStrictEqual(named, ["cake Drizzle", "cake Orphan"]);

  await assert.rejects(
    lemons.select(pl.col(Cake, "name")).intoModel(Cake).all(pool()),
    (error) => error instanceof DecodeError,
  );

  const pairs = pl.from(Cake).join(Bakery, pl.col(Cake, "bakeryId").eq(pl.col(Bakery, "id")))
    .sort(pl.col(Cake, "id")).select(pl.col(Cake, "name"), pl.col(Bakery, "name")).intoTuple();
  assert.deepStrictEqual(await pairs.all(pool()), [["Fudge", "Sunrise"], ["Drizzle", "Sunrise"], ["Carrot", "Moonlight"]]);
  assert.deepStrictEqual(await pairs.optional(pool()), ["Fudge", "Sunrise"]);
  assert.match(pairs.inspect("one").sql, /LIMIT 1/);
});
