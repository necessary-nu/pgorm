// Example 9 of guide/06-pagination.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const stream = Cake.find().orderBy(Cake.col("id").asc()).stream(pool);
  const { value: first } = await stream.next();
  assert.equal(first?.name, "Chocolate Fudge");

  // Four rows were left unread, so closing discards the stream's connection.
  await stream.close();
  assert.equal(stream.closed, true);
}
