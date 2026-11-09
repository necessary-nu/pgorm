// Example 12 of guide/11-query-builder.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { call, col, select, Table, With } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cake = new Table("cake");
  const sold = new Table("sold");
  const query = select(cake.col("name"), sold.col("units"))
    .from(cake)
    .join(sold, cake.col("id").eq(sold.col("cake_id")))
    .orderBy(cake.col("name").asc())
    .with(new With(
      "sold",
      select(col("cake_id"), call("sum", col("quantity")).as("units")).from(new Table("order_line")).groupBy(col("cake_id")),
    ));

  assert.equal(
    query.inspect().sql,
    [
      `WITH "sold" AS (SELECT "cake_id", SUM("quantity") AS "units" FROM "order_line" GROUP BY "cake_id")`,
      `SELECT "cake"."name", "sold"."units" FROM "cake"`,
      `INNER JOIN "sold" ON "cake"."id" = "sold"."cake_id" ORDER BY "cake"."name" ASC`,
    ].join(" "),
  );
  assert.deepEqual(await pool.query(query), [
    { name: "Carrot Cake", units: 1n },
    { name: "Chocolate Fudge", units: 2n },
    { name: "Lemon Drizzle", units: 1n },
  ]);
}
