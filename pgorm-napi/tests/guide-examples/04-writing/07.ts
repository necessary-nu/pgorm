// Example 7 of guide/04-writing.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // Sunrise raises every price by fifty cents.
  const raise = Cake.update({ price: Cake.col("price").add(new Decimal("0.50")) })
    .where(Cake.col("bakeryId").eq(1));
  assert.equal(
    raise.inspect().sql,
    `UPDATE "cake" SET "price" = "cake"."price" + $1 WHERE "cake"."bakery_id" = $2`,
  );
  assert.equal(await raise.execute(pool), 3);

  // Moonlight's cakes all become gluten-free.
  const changed = await Cake.update({ glutenFree: true })
    .where(Cake.col("bakeryId").eq(2))
    .returning()
    .all(pool);
  assert.ok(changed.every((cake) => cake.glutenFree));
  assert.equal(changed.length, 2);
}
