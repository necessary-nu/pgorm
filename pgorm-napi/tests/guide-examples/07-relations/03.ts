// Example 3 of guide/07-relations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Baker, bakerCakeBakers, cakeBakerCake, cakeLines, lineOrder, orderCustomer } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // From a baker to the customers who ordered what they bake.
  const bakedFor = Baker.linked(bakerCakeBakers, cakeBakerCake, cakeLines, lineOrder, orderCustomer);

  const cy = await Baker.findByKey({ id: 3 }).one(pool);
  assert.deepEqual((await bakedFor.find(cy).all(pool)).map((customer) => customer.name), ["Lee"]);
}
