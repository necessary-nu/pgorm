// Example 7 of guide/10-raw-sql.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { DecodeError } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const glutenFree = await Cake.findByStatement(
    "SELECT *, 'unread' AS note FROM cake WHERE gluten_free = $1 ORDER BY id",
    [true],
  ).all(pool);
  assert.deepEqual(glutenFree.map((cake) => cake.name), ["Lemon Drizzle", "Vanilla Sponge", "Chocolate Torte"]);

  const lemon = await Cake.findByStatement("SELECT * FROM cake WHERE id = $1", [2]).one(pool);
  assert.equal(lemon.flavor, "lemon");
  assert.equal(lemon.bakeryId, 1);

  // The statement returns no price, which Cake reads.
  await assert.rejects(Cake.findByStatement("SELECT id, name FROM cake").all(pool), DecodeError);
}
