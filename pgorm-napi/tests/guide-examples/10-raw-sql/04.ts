// Example 4 of guide/10-raw-sql.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const names: string[] = [];
  for await (const row of pool.stream("SELECT name FROM cake WHERE gluten_free = $1 ORDER BY name", [true])) {
    names.push(String(row.name));
  }
  assert.deepEqual(names, ["Chocolate Torte", "Lemon Drizzle", "Vanilla Sponge"]);
}
