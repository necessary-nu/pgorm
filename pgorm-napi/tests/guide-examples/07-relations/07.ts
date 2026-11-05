// Example 7 of guide/07-relations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Baker, Cake, cakeBakerBaker, cakeCakeBakers } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const withBakers = Cake.graph().via(cakeCakeBakers).joinMaybe(cakeBakerBaker);
  assert.equal(
    withBakers.inspect().sql,
    `SELECT "cake"."id" AS "s0_id", "cake"."name" AS "s0_name", "cake"."flavor" AS "s0_flavor", "cake"."price" AS "s0_price", "cake"."gluten_free" AS "s0_gluten_free", "cake"."bakery_id" AS "s0_bakery_id", "baker"."id" AS "s1_id", "baker"."name" AS "s1_name", "baker"."hired_on" AS "s1_hired_on", "baker"."bakery_id" AS "s1_bakery_id" FROM "cake" LEFT JOIN "cake_baker" ON "cake"."id" = "cake_baker"."cake_id" LEFT JOIN "baker" ON "cake_baker"."baker_id" = "baker"."id"`,
  );

  // Each of Sunrise's cakes, with its bakers.
  const bakers = await withBakers
    .where(Cake.col("bakeryId").eq(1))
    .orderBy(Cake.col("id").asc(), Baker.col("name").asc())
    .allGrouped(pool);
  assert.deepEqual(
    bakers.map(([cake, list]) => [cake.name, list.map((baker) => baker.name)]),
    [["Chocolate Fudge", ["Ada"]], ["Lemon Drizzle", ["Ada", "Bo"]], ["Chocolate Torte", ["Bo"]]],
  );
}
