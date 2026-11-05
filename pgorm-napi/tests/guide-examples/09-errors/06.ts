// Example 6 of guide/09-errors.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, DecodeError } from "@necessary/pgorm";
import { Cake, Customer } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // No cake 99.
  await assert.rejects(Cake.findByKey({ id: 99 }).one(pool), DecodeError);
  assert.equal(await Cake.findByKey({ id: 99 }).optional(pool), null);

  // An update or delete names its rows, or says it means every row.
  assert.throws(() => Customer.update({ name: "Kim" }).inspect(), ConstructionError);

  // An update that sets nothing.
  assert.throws(() => Cake.update({}), ConstructionError);

  // Five rows, where `one` wants exactly one.
  await assert.rejects(pool.one("SELECT name FROM cake"), DecodeError);
}
