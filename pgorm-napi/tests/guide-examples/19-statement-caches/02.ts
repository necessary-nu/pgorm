// Example 2 of guide/19-statement-caches.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { DatabaseError } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await using db = await pool.acquire();
  const before = await db.query("SELECT * FROM baker");
  assert.equal(Object.keys(before[0]!).length, 4);

  await db.execute("ALTER TABLE baker ADD COLUMN apron text");

  // Outside a transaction, the refused statement is prepared again and run.
  const after = await db.query("SELECT * FROM baker");
  assert.equal(Object.keys(after[0]!).length, 5);

  await db.execute("ALTER TABLE baker DROP COLUMN apron");

  // Inside one, the refusal is the transaction's error.
  await assert.rejects(
    db.transaction((tx) => tx.query("SELECT * FROM baker")),
    (error) => error instanceof DatabaseError && error.sqlstate === "0A000",
  );

  // It was dropped from the cache, so running the transaction again succeeds.
  const rows = await db.transaction((tx) => tx.query("SELECT * FROM baker"));
  assert.equal(Object.keys(rows[0]!).length, 4);
}
