// Example 11 of guide/05-reading.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Bakery, Cake, bakeryCakes, cakeBakery } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const menu = Cake.find()
    .join(cakeBakery)
    .where(Cake.col("glutenFree").eq(true))
    .statement.select(Bakery.col("city"), Cake.col("name"))
    .orderBy(Bakery.col("city").asc(), Cake.col("name").asc());
  assert.equal(
    menu.inspect().sql,
    `SELECT "bakery"."city", "cake"."name" FROM "cake" INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" WHERE "cake"."gluten_free" = $1 ORDER BY "bakery"."city" ASC, "cake"."name" ASC`,
  );
  assert.deepEqual(await pool.query(menu), [
    { city: "Bergen", name: "Vanilla Sponge" },
    { city: "Oslo", name: "Chocolate Torte" },
    { city: "Oslo", name: "Lemon Drizzle" },
  ]);

  // The bakeries that sell a chocolate cake, read from the bakery's side.
  const chocolateShops = Bakery.find().join(bakeryCakes).where(Cake.col("name").startsWith("Chocolate"));
  // Sunrise sells two, so the join reads it twice.
  assert.deepEqual((await chocolateShops.all(pool)).map((bakery) => bakery.name), ["Sunrise", "Sunrise"]);
  assert.deepEqual(await pool.query(chocolateShops.statement.distinct()), [{ id: 1, name: "Sunrise", city: "Oslo" }]);
}
