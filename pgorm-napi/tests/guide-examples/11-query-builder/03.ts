// Example 3 of guide/11-query-builder.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { call, col, select, Table } from "@necessary/pgorm";

export default async function example(): Promise<void> {
  // A table loaded by a nightly job, which no model describes.
  const priceList = new Table("price_list");
  const query = select(call("sum", col("new_price")).as("total")).from(priceList);
  assert.equal(query.inspect().sql, `SELECT SUM("new_price") AS "total" FROM "price_list"`);

  const year = 2025;
  const archive = select(col("name")).from(new Table(`price_list_${year}`));
  assert.equal(archive.inspect().sql, `SELECT "name" FROM "price_list_2025"`);

  // A string where an expression belongs is refused, not written into the SQL.
  // @ts-expect-error: a projection is an expression
  assert.throws(() => select("name"), TypeError);
}
