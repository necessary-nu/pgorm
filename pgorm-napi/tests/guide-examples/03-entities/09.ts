// Example 9 of guide/03-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Bakery, Cake } from "../../../guide/bakery.ts";

export default async function example(): Promise<void> {
  const cakeInOslo = Cake.belongsTo(Bakery, {
    from: "bakeryId",
    to: "id",
    onCondition: (_cake, bakery) => bakery.col("city").eq("Oslo"),
  });
  assert.equal(
    Cake.find().join(cakeInOslo).inspect().sql,
    `SELECT "cake"."id", "cake"."name", "cake"."flavor", "cake"."price", "cake"."gluten_free", "cake"."bakery_id" FROM "cake" INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" AND "bakery"."city" = $1`,
  );
  assert.equal(cakeInOslo.conditionType, "all");
}
