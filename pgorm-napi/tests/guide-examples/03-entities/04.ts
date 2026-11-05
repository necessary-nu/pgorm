// Example 4 of guide/03-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, model } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const CakeBaker = model("cake_baker", {
    columns: {
      cakeId: column("i32", { name: "cake_id", primaryKey: true }),
      bakerId: column("i32", { name: "baker_id", primaryKey: true }),
    },
  });

  assert.deepEqual(CakeBaker.primaryKey, ["cakeId", "bakerId"]);

  // Bo bakes the Lemon Drizzle.
  const row = await CakeBaker.findByKey({ cakeId: 2, bakerId: 2 }).optional(pool);
  assert.deepEqual(row, { cakeId: 2, bakerId: 2 });
}
