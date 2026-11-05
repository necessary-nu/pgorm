// Example 10 of guide/14-schema.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { commentOnColumn, createExtension } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const btreeGist = createExtension("btree_gist", { ifNotExists: true });
  assert.equal(btreeGist.inspect().sql, `CREATE EXTENSION IF NOT EXISTS "btree_gist"`);
  await pool.execute(btreeGist);

  const note = commentOnColumn(Cake.table, "price", "In kroner, VAT included");
  assert.equal(note.inspect().sql, `COMMENT ON COLUMN "cake"."price" IS 'In kroner, VAT included'`);
  await pool.execute(note);
}
