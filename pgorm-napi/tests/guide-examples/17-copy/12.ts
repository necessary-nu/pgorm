// Example 12 of guide/17-copy.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { copyFrom, copyTo } from "@necessary/pgorm";
import { Bakery } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const chunks: Uint8Array[] = [];
  for await (const chunk of await pool.copyOut(copyTo(Bakery.select("name", "city").statement, "binary"))) {
    chunks.push(chunk);
  }
  const written = await pool.copyIn(copyFrom("bakery", "binary").columns(["name", "city"]), chunks);
  assert.equal(written, 2);
  assert.equal(await Bakery.find().count(pool), 4);
}
