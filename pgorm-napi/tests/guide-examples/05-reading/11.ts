// Example 11 of guide/05-reading.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Bakery, Cake, cakeBakery } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const inOslo = Cake.find().join(cakeBakery).where(Bakery.col("city").eq("Oslo"));
  assert.equal(
    inOslo.inspect().sql,
    `SELECT "cake"."id", "cake"."name", "cake"."flavor", "cake"."price", "cake"."gluten_free", "cake"."bakery_id" FROM "cake" INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" WHERE "bakery"."city" = $1`,
  );
  assert.equal((await inOslo.all(pool)).length, 3);

  const menu = Cake.find()
    .join(cakeBakery)
    .selectOnly()
    .columns(Bakery.col("city"), Cake.col("name"))
    .where(Cake.col("glutenFree").eq(true))
    .orderBy(Bakery.col("city").asc(), Cake.col("name").asc());
  assert.equal(
    menu.inspect().sql,
    `SELECT "bakery"."city", "cake"."name" FROM "cake" INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" WHERE "cake"."gluten_free" = $1 ORDER BY "bakery"."city" ASC, "cake"."name" ASC`,
  );
  const rows: [string, string][] = await menu.intoTuple().all(pool);
  assert.deepEqual(rows, [
    ["Bergen", "Vanilla Sponge"],
    ["Oslo", "Chocolate Torte"],
    ["Oslo", "Lemon Drizzle"],
  ]);

  // The bakeries that sell a chocolate cake, read from the bakery's side.
  const chocolateShops = Bakery.find()
    .joinRev(cakeBakery)
    .where(Cake.col("name").startsWith("Chocolate"))
    .distinct();
  assert.deepEqual((await chocolateShops.all(pool)).map((bakery) => bakery.name), ["Sunrise"]);
}
