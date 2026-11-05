// Example 3 of guide/17-copy.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { copyFrom, LifecycleError } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await using db = await pool.acquire();
  const copy = copyFrom("bakery", "text").columns(["name", "city"]);

  const { promise: sent, resolve: send } = Promise.withResolvers<void>();
  async function* bakeries() {
    await sent;
    yield new TextEncoder().encode("Noon\tStavanger\n");
  }
  const copying = db.copyIn(copy, bakeries());

  // The COPY still holds the connection.
  await assert.rejects(db.one("SELECT count(*) AS bakeries FROM bakery"), LifecycleError);

  send();
  assert.equal(await copying, 1);
  const { bakeries: count } = await db.one("SELECT count(*) AS bakeries FROM bakery");
  assert.equal(count, 3n);
}
