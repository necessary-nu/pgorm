// Example 1 of guide/04-writing.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Decimal, type InsertOf } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";

export default async function example(): Promise<void> {
  const muffin: InsertOf<typeof Cake> = {
    name: "Blueberry Muffin",
    flavor: "vanilla",
    price: new Decimal("2.75"),
    glutenFree: false,
    bakeryId: 2,
  };

  // The id is left out, so the database chooses it.
  const { sql } = Cake.insert(muffin).inspect();
  assert.equal(
    sql,
    `INSERT INTO "cake" ("name", "flavor", "price", "gluten_free", "bakery_id") VALUES ($1, CAST($2::text AS public.flavor), $3, $4, $5)`,
  );
}
