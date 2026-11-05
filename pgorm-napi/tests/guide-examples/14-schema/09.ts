// Example 9 of guide/14-schema.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { call, col, createMaterializedView, createView, refreshMaterializedView, select } from "@necessary/pgorm";
import { Cake, OrderLine } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const glutenFree = createView(
    "gluten_free_cake",
    Cake.select("id", "name").where(Cake.col("glutenFree").eq(true)).statement,
  ).orReplace();
  assert.equal(
    glutenFree.inspect().sql,
    `CREATE OR REPLACE VIEW "gluten_free_cake" AS SELECT "cake"."id", "cake"."name" FROM "cake" WHERE "cake"."gluten_free" = TRUE`,
  );
  await pool.execute(glutenFree);

  const sold = createMaterializedView(
    "cakes_sold",
    select(col("cake_id"), call("sum", col("quantity"))).from(OrderLine.table).groupBy(col("cake_id")),
  ).columns(["cake_id", "sold"]);
  await pool.execute(sold);
  await pool.execute(refreshMaterializedView("cakes_sold"));

  const fudge = await pool.one("SELECT sold FROM cakes_sold WHERE cake_id = 1");
  assert.equal(fudge.sold, 2n);
  const count = await pool.one("SELECT count(*) AS cakes FROM gluten_free_cake");
  assert.equal(count.cakes, 3n);
}
