// Example 2 of guide/11-query-builder.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { col, Decimal, deleteFrom, select, Table, update } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cake = new Table("cake");
  const bakery = new Table("bakery");
  const query = select(cake.col("name"), bakery.col("city"))
    .from(cake)
    .join(bakery, cake.col("bakery_id").eq(bakery.col("id")))
    .where(cake.col("gluten_free").eq(true))
    .orderBy(cake.col("name").asc());
  assert.deepEqual(await pool.query(query), [
    { name: "Chocolate Torte", city: "Oslo" },
    { name: "Lemon Drizzle", city: "Oslo" },
    { name: "Vanilla Sponge", city: "Bergen" },
  ]);

  const raise = update(cake).set("price", col("price").add(new Decimal("0.50"))).where(col("bakery_id").eq(2));
  assert.equal(raise.inspect().sql, `UPDATE "cake" SET "price" = "price" + $1 WHERE "bakery_id" = $2`);
  assert.equal(await pool.execute(raise), 2);

  const deleted = await pool.query(deleteFrom(cake).where(col("id").eq(5)).returning([col("name")]));
  assert.deepEqual(deleted, [{ name: "Chocolate Torte" }]);
}
