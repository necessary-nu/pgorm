// Example 3 of guide/04-writing.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";

export default async function example(): Promise<void> {
  const { sql, values } = Cake.insert({
    name: "Mocha Cupcake",
    flavor: "chocolate",
    price: new Decimal("3.10"),
    glutenFree: false,
    bakeryId: 1,
  }).inspect();
  assert.equal(
    sql,
    `INSERT INTO "cake" ("name", "flavor", "price", "gluten_free", "bakery_id") VALUES ($1, CAST($2::text AS public.flavor), $3, $4, $5)`,
  );
  assert.equal(values.length, 5);
}
