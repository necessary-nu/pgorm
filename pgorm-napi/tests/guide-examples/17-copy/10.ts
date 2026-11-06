// Example 10 of guide/17-copy.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { copyFrom, copyTo, type Decimal } from "@necessary/pgorm";
import { Cake, flavor } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const sunrise = copyTo(
    Cake.select("name", "flavor", "price", "glutenFree").where(Cake.col("bakeryId").eq(1)).statement,
    "binary",
  );
  const out = await pool.copyOutRows(sunrise);
  assert.deepEqual(out.columns, ["name", "flavor", "price", "gluten_free"]);

  const into = copyFrom("cake", "binary").columns(["name", "flavor", "price", "gluten_free", "bakery_id"]);
  async function* atMoonlight() {
    for await (const [name, flavor, price, glutenFree] of out) {
      yield [name as string, flavor as string, price as Decimal, glutenFree as boolean, 2];
    }
  }
  // The binary COPY TO reads on a connection of its own, while the COPY FROM writes on another.
  assert.equal(await pool.copyInRows(into, atMoonlight()), 3);

  assert.equal(await Cake.find().where(Cake.col("bakeryId").eq(2)).count(pool), 5);
}
