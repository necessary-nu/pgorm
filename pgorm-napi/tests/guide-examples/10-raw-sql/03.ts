// Example 3 of guide/10-raw-sql.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { DatabaseError } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const script = "CREATE TABLE tray (id int); CREATE TABLE rack (id int)";
  await assert.rejects(
    pool.execute(script),
    (error) => error instanceof DatabaseError && error.sqlstate === "42601",
  );

  await pool.batchExecute(script);
  const { tables } = await pool.one(
    "SELECT count(*) AS tables FROM pg_tables WHERE tablename = ANY($1)",
    [["tray", "rack"]],
  );
  assert.equal(tables, 2n);
}
