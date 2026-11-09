// Example 11 of guide/11-query-builder.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, merge, MergeAction, Table } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await pool.execute("CREATE TABLE price_list (name text NOT NULL, new_price numeric(10, 2) NOT NULL)");
  await pool.execute("INSERT INTO price_list VALUES ('Chocolate Fudge', 4.80), ('Lemon Drizzle', 3.20)");

  const cake = new Table("cake");
  const priceList = new Table("price_list");
  const pending = merge(cake, priceList, cake.col("name").eq(priceList.col("name")));
  // @ts-expect-error: a MERGE with no WHEN arm is no statement
  await assert.rejects(pool.query(pending), ConstructionError);

  const reprice = pending
    .whenMatched(MergeAction.update("price", priceList.col("new_price")), {
      condition: cake.col("price").ne(priceList.col("new_price")),
    })
    .whenNotMatchedBySource(MergeAction.delete(), { condition: cake.col("bakery_id").eq(1) })
    .returningAction()
    .returning([cake.col("name")]);

  assert.equal(
    reprice.inspect().sql,
    [
      `MERGE INTO "cake" USING "price_list" ON "cake"."name" = "price_list"."name"`,
      `WHEN MATCHED AND "cake"."price" <> "price_list"."new_price" THEN UPDATE SET "price" = "price_list"."new_price"`,
      `WHEN NOT MATCHED BY SOURCE AND "cake"."bakery_id" = $1 THEN DELETE`,
      `RETURNING merge_action(), "cake"."name"`,
    ].join(" "),
  );

  const changes = (await pool.query(reprice)).map((row) => [row.merge_action, row.name]).sort();
  assert.deepEqual(changes, [
    ["DELETE", "Chocolate Torte"],
    ["UPDATE", "Chocolate Fudge"],
  ]);
}
