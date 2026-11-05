// Example 8 of guide/14-schema.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { createSchema, DatabaseError, dropSchema, renameSchema } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await pool.execute(createSchema("archive"));
  await pool.execute(renameSchema("archive", "attic"));
  await pool.execute("CREATE TABLE attic.box (id integer)");

  await assert.rejects(pool.execute(dropSchema("attic")), (error: unknown) => {
    return error instanceof DatabaseError && error.sqlstate === "2BP01";
  });
  const dropped = dropSchema("attic", { behavior: "cascade" });
  assert.equal(dropped.inspect().sql, `DROP SCHEMA "attic" CASCADE`);
  await pool.execute(dropped);
}
