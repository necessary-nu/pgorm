// Example 8 of guide/07-relations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Cake, bakerCakeBakers, cakeBakerBaker, cakeBakerCake, cakeCakeBakers } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const sibling = Cake.as("sibling");
  const siblings = await Cake.graph()
    .via(cakeCakeBakers)
    .joinOne(cakeBakerBaker)
    .joinMaybe(bakerCakeBakers, { alias: "shared", from: 1 })
    .joinMaybe(cakeBakerCake, { alias: "sibling", from: 2 })
    .where(sibling.col("id").ne(Cake.col("id")))
    .orderBy(sibling.col("name").asc())
    .all(pool);

  const lemonDrizzle = siblings.filter(([cake]) => cake.id === 2).map(([, , , other]) => other?.name);
  assert.deepEqual(lemonDrizzle, ["Chocolate Fudge", "Chocolate Torte"]);
}
