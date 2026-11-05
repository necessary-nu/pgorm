// Example 9 of guide/04-writing.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { DecodeError } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const changes = await Cake.update({ price: Cake.col("price").mul(2) })
    .where(Cake.col("name").eq("Vanilla Sponge"))
    .returningChanges(pool);
  assert.equal(changes.length, 1);
  assert.equal(changes[0]!.old.price.toString(), "2.90");
  assert.equal(changes[0]!.new.price.toString(), "5.80");

  // The one row a key names, before and after; a key that matches nothing is no change.
  const { old, new: now } = await Cake.update({ glutenFree: true }).where(Cake.key({ id: 1 })).returningChange(pool);
  assert.deepEqual([old.glutenFree, now.glutenFree], [false, true]);
  await assert.rejects(
    Cake.update({ glutenFree: true }).where(Cake.key({ id: 99 })).returningChange(pool),
    DecodeError,
  );
}
