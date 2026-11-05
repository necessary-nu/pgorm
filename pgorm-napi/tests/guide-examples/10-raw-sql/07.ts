// Example 7 of guide/10-raw-sql.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { col, Table, update } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const rename = update(new Table("cake")).set("name", "Lemon Drizzle Loaf").where(col("id").eq(2));
  const { sql, values } = rename.inspect();
  assert.equal(sql, `UPDATE "cake" SET "name" = $1 WHERE "id" = $2`);

  assert.equal(await pool.execute(sql, values), 1);
  assert.equal(await pool.execute(rename), 1);
}
