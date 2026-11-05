// Example 8 of guide/10-raw-sql.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { DatabaseError } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await assert.rejects(
    pool.query("SELEC name FROM baker WHERE bakery_id = $1", [1]),
    (error) => error instanceof DatabaseError && error.sqlstate === "42601",
  );
}
