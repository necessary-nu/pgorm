// Example 4 of guide/17-copy.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { copyFrom, DatabaseError } from "@necessary/pgorm";
import { Bakery } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const copy = copyFrom("bakery", "csv").columns(["name", "city"]);
  const encoder = new TextEncoder();

  // The second line has a third field the column list does not hold.
  const refused = pool.copyIn(copy, encoder.encode("Noon,Stavanger\nDusk,Tromsø,extra\n"));
  await assert.rejects(refused, DatabaseError);

  // A source that fails part way writes nothing either.
  async function* failing() {
    yield encoder.encode("Noon,Stavanger\n");
    throw new Error("the upload was cut off");
  }
  await assert.rejects(pool.copyIn(copy, failing()), /the upload was cut off/);

  assert.equal(await Bakery.find().count(pool), 2);
}
