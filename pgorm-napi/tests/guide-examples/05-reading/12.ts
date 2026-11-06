// Example 12 of guide/05-reading.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { call, column } from "@necessary/pgorm";
import { Bakery, Cake, cakeBakery } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cakes = call("count", Cake.col("id"));
  const busy = Cake.find()
    .join(cakeBakery)
    .selectOnly()
    .column(Bakery.col("name"))
    .exprAs(cakes, "cakes", column("i64"))
    .groupBy(Bakery.col("name"))
    .having(cakes.gt(2));
  assert.equal(
    busy.inspect().sql,
    `SELECT "bakery"."name", COUNT("cake"."id") AS "cakes" FROM "cake" INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" GROUP BY "bakery"."name" HAVING COUNT("cake"."id") > $1`,
  );
  assert.deepEqual(await busy.intoTuple().all(pool), [["Sunrise", 3n]]);

  const glutenFree = await Cake.find().where(Cake.col("glutenFree").eq(true)).count(pool);
  assert.equal(glutenFree, 3);
}
