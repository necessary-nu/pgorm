// Example 8 of guide/05-reading.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { call, column } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const names: [string][] = await Cake.select("name").orderBy(Cake.col("id").asc()).intoTuple().all(pool);
  assert.deepEqual(names[0], ["Chocolate Fudge"]);

  const flavors = Cake.select("flavor")
    .exprAs(call("count", Cake.col("id")), "cakes", column("i64"))
    .groupBy(Cake.col("flavor"))
    .orderBy(Cake.col("flavor").asc());
  assert.equal(
    flavors.inspect().sql,
    `SELECT "cake"."flavor", COUNT("cake"."id") AS "cakes" FROM "cake" GROUP BY "cake"."flavor" ORDER BY "cake"."flavor" ASC`,
  );
  const counted = await flavors.intoTuple().all(pool);
  assert.deepEqual(counted[0], ["chocolate", 2n]);
  const byName: [bigint, string][] = await flavors.intoValues("cakes", "flavor").all(pool);
  assert.deepEqual(byName[0], [2n, "chocolate"]);
}
