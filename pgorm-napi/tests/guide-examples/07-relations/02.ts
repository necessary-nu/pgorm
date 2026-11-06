// Example 2 of guide/07-relations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";
import { Baker, cakeBakers } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // A cake's bakers, through the cake_baker junction.
  const lemonDrizzle = {
    id: 2,
    name: "Lemon Drizzle",
    flavor: "lemon",
    price: new Decimal("3.20"),
    glutenFree: true,
    bakeryId: 1,
  } as const;
  const lemonDrizzlesBakers = cakeBakers.find(lemonDrizzle).orderBy(Baker.col("name").asc());
  assert.equal(
    lemonDrizzlesBakers.inspect().sql,
    `SELECT "baker"."id", "baker"."name", "baker"."hired_on", "baker"."bakery_id" FROM "baker" INNER JOIN "cake_baker" ON "cake_baker"."baker_id" = "baker"."id" INNER JOIN "cake" ON "cake"."id" = "cake_baker"."cake_id" WHERE "cake"."id" = $1 ORDER BY "baker"."name" ASC`,
  );
  const names = (await lemonDrizzlesBakers.all(pool)).map((baker) => baker.name);
  assert.deepEqual(names, ["Ada", "Bo"]);
}
