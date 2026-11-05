// Example 6 of guide/10-raw-sql.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const row = await pool.one(
    "SELECT flavor, price, NULL::text AS note FROM cake WHERE id = $1",
    [2],
    { tagged: true },
  );
  assert.equal(row.flavor.kind, "enum");
  assert.equal(String(row.flavor.typeName), `"public"."flavor"`);
  assert.equal(row.flavor.value, "lemon");
  assert.equal(row.price.kind, "decimal");
  assert.ok(row.note.isNull);
}
