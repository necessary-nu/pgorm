// Example 4 of guide/06-pagination.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, Decimal } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";

export default async function example(): Promise<void> {
  const feed = Cake.find().cursor("price", "id");
  // @ts-expect-error: the key is two fields, so the boundary is two values.
  assert.throws(() => feed.after(new Decimal("3.20")), ConstructionError);
}
