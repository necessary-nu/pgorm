// Example 1 of guide/05-reading.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { DecodeError } from "@necessary/pgorm";
import { Cake, CakeBaker } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cakes = await Cake.find().all(pool);
  assert.equal(cakes.length, 5);

  const torte = await Cake.findByKey({ id: 5 }).one(pool);
  assert.equal(torte.name, "Chocolate Torte");

  await assert.rejects(Cake.findByKey({ id: 99 }).one(pool), DecodeError);
  assert.equal(await Cake.findByKey({ id: 99 }).optional(pool), null);

  // Bo bakes the Lemon Drizzle.
  assert.notEqual(await CakeBaker.findByKey({ cakeId: 2, bakerId: 2 }).optional(pool), null);
}
