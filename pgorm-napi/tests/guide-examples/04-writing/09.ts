// Example 9 of guide/04-writing.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Bakery, Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const bergen = Cake.update({ glutenFree: true })
    .from(Bakery)
    .where(Cake.col("bakeryId").eq(Bakery.col("id")))
    .where(Bakery.col("city").eq("Bergen"));
  assert.equal(
    bergen.inspect().sql,
    `UPDATE "cake" SET "gluten_free" = $1 FROM "bakery" WHERE "cake"."bakery_id" = "bakery"."id" AND "bakery"."city" = $2`,
  );
  const changed = await bergen.returning("name").all(pool);
  assert.deepEqual(changed.map((cake) => cake.name).sort(), ["Carrot Cake", "Vanilla Sponge"]);
}
