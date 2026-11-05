// Example 3 of guide/10-raw-sql.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { DatabaseError } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await assert.rejects(
    pool.execute("CREATE TABLE tray (id int); CREATE TABLE rack (id int)"),
    (error) => error instanceof DatabaseError && error.sqlstate === "42601",
  );

  for (const statement of ["CREATE TABLE tray (id int)", "CREATE TABLE rack (id int)"]) {
    await pool.execute(statement);
  }
  const { tables } = await pool.one(
    "SELECT count(*) AS tables FROM pg_tables WHERE tablename = ANY($1)",
    [["tray", "rack"]],
  );
  assert.equal(tables, 2n);
}
