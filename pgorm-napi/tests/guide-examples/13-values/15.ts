// Example 15 of guide/13-values.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { CheckViolationError, column, createDomain, domainValue, model } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const grams = createDomain("grams", "integer").check(domainValue().gt(0));
  assert.equal(grams.inspect().sql, `CREATE DOMAIN "grams" AS integer CHECK (VALUE > 0)`);
  await pool.execute(grams);
  await pool.execute("CREATE TABLE ingredient (id serial PRIMARY KEY, name varchar NOT NULL, per_batch grams NOT NULL)");

  const Ingredient = model("ingredient", {
    columns: {
      id: column("i32", { primaryKey: true, default: true }),
      name: column("text"),
      perBatch: column("i32", { name: "per_batch" }),
    },
  });

  const flour = await Ingredient.insert({ name: "Flour", perBatch: 500 }).returning().one(pool);
  assert.equal(flour.perBatch, 500);

  await assert.rejects(
    Ingredient.insert({ name: "Air", perBatch: 0 }).execute(pool),
    (error: unknown) => error instanceof CheckViolationError && error.datatype === "grams",
  );
}
