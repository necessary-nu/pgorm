// Example 10 of guide/04-writing.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { select } from "@necessary/pgorm";
import { Cake, CakeBaker } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // Cy no longer bakes the Vanilla Sponge.
  const row = await CakeBaker.findByKey({ cakeId: 4, bakerId: 3 }).one(pool);
  assert.equal(await CakeBaker.delete().where(CakeBaker.key(row)).execute(pool), 1);

  // Bo leaves the Lemon Drizzle to Ada.
  assert.equal(await CakeBaker.delete().where(CakeBaker.key({ cakeId: 2, bakerId: 2 })).execute(pool), 1);

  // Moonlight stops selling the cakes nobody bakes.
  const unbaked = Cake.delete()
    .where(Cake.col("bakeryId").eq(2))
    .where(Cake.col("id").isNotIn(select(CakeBaker.col("cakeId")).from(CakeBaker.table)));
  assert.equal(await unbaked.execute(pool), 1);
  assert.equal(await Cake.find().count(pool), 4);
}
