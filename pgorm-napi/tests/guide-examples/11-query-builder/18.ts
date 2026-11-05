// Example 18 of guide/11-query-builder.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { col, select, Table } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // The dearest cakes, the second-dearest's ties included.
  const dearest = select(col("name"))
    .from(new Table("cake"))
    .orderBy(col("price").desc())
    .limitWithTies(2);
  assert.equal(
    dearest.inspect().sql,
    `SELECT "name" FROM "cake" ORDER BY "price" DESC FETCH FIRST $1 ROWS WITH TIES`,
  );
  assert.deepEqual((await pool.query(dearest)).map((row) => row.name), ["Chocolate Torte", "Chocolate Fudge"]);

  const chocolate = select(col("name"))
    .from(new Table("cake"))
    .where(col("name").iregex("^choc"))
    .orderBy(col("name").asc());
  assert.deepEqual((await pool.query(chocolate)).map((row) => row.name), ["Chocolate Fudge", "Chocolate Torte"]);

  // Every row, sampled at 100 percent with a fixed seed.
  const sampled = select(col("name")).from(new Table("cake").tablesample("bernoulli", 100, { repeatable: 7 }));
  assert.equal((await pool.query(sampled)).length, 5);
}
