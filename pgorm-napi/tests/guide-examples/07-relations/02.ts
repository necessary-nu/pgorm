// Example 2 of guide/07-relations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Baker, CakeBaker, bakerCakeBakers } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // A cake's bakers, through the cake_baker junction.
  const lemonDrizzlesBakers = Baker.find()
    .join(bakerCakeBakers)
    .where(CakeBaker.col("cakeId").eq(2))
    .orderBy(Baker.col("name").asc());
  assert.equal(
    lemonDrizzlesBakers.inspect().sql,
    `SELECT "baker"."id", "baker"."name", "baker"."hired_on", "baker"."bakery_id" FROM "baker" INNER JOIN "cake_baker" ON "baker"."id" = "cake_baker"."baker_id" WHERE "cake_baker"."cake_id" = $1 ORDER BY "baker"."name" ASC`,
  );
  const names = (await lemonDrizzlesBakers.all(pool)).map((baker) => baker.name);
  assert.deepEqual(names, ["Ada", "Bo"]);
}
