// Example 7 of guide/03-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Bakery, Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cakeBakery = Cake.belongsTo(Bakery, { from: "bakeryId", to: "id" });
  const bakeryCakes = Bakery.hasMany(Cake, { from: "id", to: "bakeryId" });

  assert.equal(cakeBakery.cardinality, "one");
  assert.equal(bakeryCakes.cardinality, "many");
  assert.deepEqual([cakeBakery.fromFields, cakeBakery.toFields], [["bakeryId"], ["id"]]);

  const lemonDrizzle = await Cake.findByKey({ id: 2 }).one(pool);
  const sunrise = await cakeBakery.find(lemonDrizzle).one(pool);
  assert.equal(sunrise.name, "Sunrise");
}
