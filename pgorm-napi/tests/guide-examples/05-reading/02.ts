// Example 2 of guide/05-reading.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const chocolateUnderFive = Cake.find()
    .where(Cake.col("flavor").eq("chocolate"))
    .where(Cake.col("price").lt(new Decimal("5.00")));
  assert.equal(
    chocolateUnderFive.inspect().sql,
    `SELECT "cake"."id", "cake"."name", "cake"."flavor", "cake"."price", "cake"."gluten_free", "cake"."bakery_id" FROM "cake" WHERE "cake"."flavor" = CAST($1::text AS public.flavor) AND "cake"."price" < $2`,
  );
  const found = await chocolateUnderFive.all(pool);
  assert.deepEqual(found.map((cake) => cake.name), ["Chocolate Fudge"]);

  const midPriced = await Cake.find()
    .where(Cake.col("price").between(new Decimal("3.00"), new Decimal("4.00")))
    .count(pool);
  assert.equal(midPriced, 2);

  const named = await Cake.find()
    .where(Cake.col("name").startsWith("Chocolate"))
    .where(Cake.col("id").isIn([1, 2, 3]))
    .all(pool);
  assert.equal(named.length, 1);
}
