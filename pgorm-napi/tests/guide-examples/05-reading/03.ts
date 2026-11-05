// Example 3 of guide/05-reading.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError } from "@necessary/pgorm";
import { Customer } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // @ts-expect-error: a column is never compared with null.
  assert.throws(() => Customer.col("notes").eq(null), ConstructionError);

  const noNotes = await Customer.find().where(Customer.col("notes").isNull()).all(pool);
  assert.deepEqual(noNotes.map((customer) => customer.name), ["Kim"]);
}
