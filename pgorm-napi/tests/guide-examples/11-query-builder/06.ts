// Example 6 of guide/11-query-builder.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { col, ConstructionError, select, Table, template } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const short = select(col("name"))
    .from(new Table("cake"))
    .where(template`char_length(${col("name")}) < ${14}`)
    .orderBy(col("name").asc());
  assert.equal(short.inspect().sql, `SELECT "name" FROM "cake" WHERE char_length("name") < $1 ORDER BY "name" ASC`);
  assert.deepEqual((await pool.query(short)).map((row) => row.name), ["Carrot Cake", "Lemon Drizzle"]);

  assert.throws(() => template`'${14}'`, ConstructionError);
}
