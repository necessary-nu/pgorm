// Example 7 of guide/05-reading.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { call, select } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const flavors = select(Cake.col("flavor"), call("count", Cake.col("id")).as("cakes"))
    .from(Cake.table)
    .groupBy(Cake.col("flavor"))
    .orderBy(Cake.col("flavor").asc());
  assert.equal(
    flavors.inspect().sql,
    `SELECT "cake"."flavor", COUNT("cake"."id") AS "cakes" FROM "cake" GROUP BY "cake"."flavor" ORDER BY "cake"."flavor" ASC`,
  );

  const counted = await pool.query(flavors);
  assert.deepEqual(counted[0], { flavor: "chocolate", cakes: 2n });
}
