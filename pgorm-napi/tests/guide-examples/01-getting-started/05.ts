// Example 5 of guide/01-getting-started.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const scone = await Cake.insert({
    name: "Lemon Scone",
    flavor: "lemon",
    price: new Decimal("2.50"),
    glutenFree: false,
    bakeryId: 1,
  }).returning().one(pool);

  // The database chose the id; the row comes back as it was stored.
  assert.equal(scone.id, 6);
  assert.equal(scone.price.toString(), "2.50");
}
