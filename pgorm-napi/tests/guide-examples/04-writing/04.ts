// Example 4 of guide/04-writing.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";
import { Cake, flavor } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const tart = (name: string, flavor: "lemon" | "chocolate" | "vanilla" | "carrot", price: string) => ({
    name,
    flavor,
    price: new Decimal(price),
    glutenFree: true,
    bakeryId: 2,
  });

  const ids = await Cake.insertMany([
    tart("Lemon Tart", "lemon", "4.20"),
    tart("Chocolate Tart", "chocolate", "4.50"),
  ]).returning("id").all(pool);
  assert.deepEqual(ids.map((row) => row.id), [6, 7]);

  const tarts = await Cake.insertMany([tart("Vanilla Tart", "vanilla", "4.00")])
    .add(tart("Carrot Tart", "carrot", "4.10"))
    .returning()
    .all(pool);
  assert.deepEqual(tarts.map((row) => row.name), ["Vanilla Tart", "Carrot Tart"]);

  assert.equal(await Cake.insertMany([]).execute(pool), 0);
}
