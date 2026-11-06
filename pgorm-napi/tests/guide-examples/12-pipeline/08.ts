// Example 8 of guide/12-pipeline.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Decimal, pipeline as pl } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cake = (column: string) => pl.col("cake", column);
  const budget = new Decimal("3.50");

  const affordable = pl.from(Cake.table)
    .filterWith((binder) => cake("price").lte(binder.bind(budget)))
    .sort(cake("price"))
    .select(cake("name"));

  const { sql, values } = affordable.inspect();
  assert.equal(
    sql,
    "WITH table_0 AS (SELECT name, price FROM cake WHERE price <= $1) " +
      "SELECT name FROM table_0 ORDER BY price",
  );
  assert.deepEqual(values.map((value) => String(value.value)), ["3.50"]);

  assert.deepEqual(await affordable.all(pool), [{ name: "Vanilla Sponge" }, { name: "Lemon Drizzle" }]);
}
