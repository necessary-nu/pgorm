// Example 1 of guide/28-bindings.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { select, Table } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cake = new Table("cake");
  const query = select(cake.col("name")).from(cake).where(cake.col("price").gt(4));
  assert.equal(query.inspect().sql, `SELECT "cake"."name" FROM "cake" WHERE "cake"."price" > $1`);

  const names = (await pool.query(query)).map((row) => row.name).sort();
  assert.deepEqual(names, ["Chocolate Fudge", "Chocolate Torte"]);
}
