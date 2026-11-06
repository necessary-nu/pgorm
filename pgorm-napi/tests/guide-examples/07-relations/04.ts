// Example 4 of guide/07-relations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Baker, Cake, bakerCakeBakers, cakeBakerCake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const bakes = Baker.linked(bakerCakeBakers, cakeBakerCake);
  assert.equal(bakes.lastHop().alias, "r1");
  assert.equal(
    bakes.find({ id: 1 }).inspect().sql,
    `SELECT "cake"."id", "cake"."name", "cake"."flavor", "cake"."price", "cake"."gluten_free", "cake"."bakery_id" FROM "cake" INNER JOIN "cake_baker" AS "r0" ON "r0"."cake_id" = "cake"."id" INNER JOIN "baker" AS "r1" ON "r1"."id" = "r0"."baker_id" WHERE "r1"."id" = $1`,
  );
  const adas = bakes.findLinked().where(bakes.lastHop().col("name").eq("Ada")).orderBy(Cake.col("id").asc());
  assert.deepEqual((await adas.all(pool)).map((cake) => cake.name), ["Chocolate Fudge", "Lemon Drizzle"]);
}
