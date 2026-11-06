// Example 14 of guide/13-values.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, CreatedRange, createType, model, Range } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const createTemperature = createType("temperature").asRange("double");
  assert.equal(createTemperature.inspect().sql, `CREATE TYPE "temperature" AS RANGE (SUBTYPE = double precision)`);
  await pool.execute(createTemperature);
  await pool.execute("CREATE TABLE oven (id serial PRIMARY KEY, name varchar NOT NULL, heat temperature NOT NULL)");

  const temperature = new CreatedRange("temperature", "f64", { schema: "public" });
  const Oven = model("oven", {
    columns: {
      id: column("i32", { primaryKey: true, default: true }),
      name: column("text"),
      heat: column(temperature),
    },
  });

  const deck = await Oven.insert({ name: "Deck oven", heat: new Range(180, 250.5) }).returning().one(pool);
  assert.equal(deck.heat.toString(), "[180,250.5)");
  assert.equal(
    Oven.insert({ name: "Deck oven", heat: new Range(180, 250.5) }).inspect().sql,
    `INSERT INTO "oven" ("name", "heat") VALUES ($1, CAST($2::text AS public.temperature))`,
  );

  const hotEnough = await Oven.find().where(Oven.col("heat").overlaps(new Range(240, 260))).count(pool);
  assert.equal(hotEnough, 1);
}
