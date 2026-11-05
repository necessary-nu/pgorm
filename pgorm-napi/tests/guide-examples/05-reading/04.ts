// Example 4 of guide/05-reading.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Condition, Decimal } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";

export default async function example(): Promise<void> {
  const lemonOrGlutenFree = Cake.select("name").where(
    Condition.any(Cake.col("flavor").eq("lemon"), Cake.col("glutenFree").eq(true)),
  );
  assert.equal(
    lemonOrGlutenFree.inspect().sql,
    `SELECT "cake"."name" FROM "cake" WHERE "cake"."flavor" = CAST($1::text AS public.flavor) OR "cake"."gluten_free" = $2`,
  );

  // A search with an optional upper price: no condition at all when it is absent.
  function search(maxPrice: Decimal | null) {
    let query = Cake.select("name").where(Cake.col("bakeryId").eq(1));
    if (maxPrice !== null) query = query.where(Cake.col("price").lte(maxPrice));
    return query;
  }
  assert.equal(search(null).inspect().sql, `SELECT "cake"."name" FROM "cake" WHERE "cake"."bakery_id" = $1`);
  assert.equal(
    search(new Decimal("4.00")).inspect().sql,
    `SELECT "cake"."name" FROM "cake" WHERE "cake"."bakery_id" = $1 AND "cake"."price" <= $2`,
  );
}
