// Example 8 of guide/13-values.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Cake } from "../../../guide/bakery.ts";

export default async function example(): Promise<void> {
  const { sql, values } = Cake.select("name").where(Cake.col("flavor").eq("lemon")).inspect();
  assert.equal(
    sql,
    `SELECT "cake"."name" FROM "cake" WHERE "cake"."flavor" = CAST($1::text AS public.flavor)`,
  );
  assert.deepEqual(values.map((value) => value.value), ["lemon"]);
}
