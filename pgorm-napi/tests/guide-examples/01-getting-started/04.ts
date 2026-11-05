// Example 4 of guide/01-getting-started.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Cake } from "../../../guide/bakery.ts";

export default async function example(): Promise<void> {
  const { sql, values } = Cake.find()
    .where(Cake.col("glutenFree").eq(true))
    .orderBy(Cake.col("name").asc())
    .inspect();

  assert.equal(
    sql,
    `SELECT "cake"."id", "cake"."name", "cake"."flavor", "cake"."price", "cake"."gluten_free", "cake"."bakery_id" FROM "cake" WHERE "cake"."gluten_free" = $1 ORDER BY "cake"."name" ASC`,
  );
  assert.deepEqual(values.map((value) => value.value), [true]);
}
