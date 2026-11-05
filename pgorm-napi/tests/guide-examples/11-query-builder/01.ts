// Example 1 of guide/11-query-builder.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { select, Table } from "@necessary/pgorm";

export default async function example(): Promise<void> {
  const cake = new Table("cake");
  const bakery = new Table("bakery");
  const query = select(cake.col("name"), bakery.col("city"))
    .from(cake)
    .join(bakery, cake.col("bakery_id").eq(bakery.col("id")))
    .where(cake.col("gluten_free").eq(true))
    .orderBy(cake.col("name").asc());

  const { sql, values } = query.inspect();
  assert.equal(
    sql,
    `SELECT "cake"."name", "bakery"."city" FROM "cake" INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" WHERE "cake"."gluten_free" = $1 ORDER BY "cake"."name" ASC`,
  );
  assert.deepEqual(values.map((value) => value.value), [true]);
}
