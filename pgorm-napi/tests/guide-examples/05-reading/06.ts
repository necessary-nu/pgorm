// Example 6 of guide/05-reading.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const dearest = Cake.select("flavor", "name")
    .distinctOn(Cake.col("flavor"))
    .orderBy(Cake.col("flavor").asc(), Cake.col("price").desc());
  assert.equal(
    dearest.inspect().sql,
    `SELECT DISTINCT ON ("cake"."flavor") "cake"."flavor", "cake"."name" FROM "cake" ORDER BY "cake"."flavor" ASC, "cake"."price" DESC`,
  );
  assert.deepEqual(
    (await dearest.all(pool)).map((cake) => cake.name),
    ["Chocolate Torte", "Vanilla Sponge", "Lemon Drizzle", "Carrot Cake"],
  );
}
