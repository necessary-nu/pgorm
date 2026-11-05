// Example 2 of guide/04-writing.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cupcake = (name: string, price: string) =>
    Cake.insert({ name, flavor: "chocolate", price: new Decimal(price), glutenFree: false, bakeryId: 1 });

  const { id } = await cupcake("Mocha Cupcake", "3.10").returning("id").one(pool);
  assert.equal(id, 6);

  const cherry = await cupcake("Cherry Cupcake", "3.30").returning().one(pool);
  assert.deepEqual([cherry.id, cherry.name], [7, "Cherry Cupcake"]);

  const written = await cupcake("Plain Cupcake", "2.50").execute(pool);
  assert.equal(written, 1);
}
