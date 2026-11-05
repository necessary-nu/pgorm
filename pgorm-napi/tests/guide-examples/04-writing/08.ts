// Example 8 of guide/04-writing.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  assert.throws(() => Cake.update({}).where(Cake.key({ id: 1 })).inspect(), ConstructionError);
  assert.throws(() => Cake.update({ glutenFree: true }).inspect(), ConstructionError);
  assert.equal(await Cake.update({ glutenFree: true }).where(Cake.key({ id: 99 })).execute(pool), 0);
}
