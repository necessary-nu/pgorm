// Example 3 of guide/27-postgres-releases.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { jsonArrayQuery, select, Table } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // The names of the cakes over 10.00: there are none.
  const cake = new Table("cake");
  const dear = select(cake.col("name")).from(cake).where(cake.col("price").gt(10));
  const { names } = await pool.one(select(jsonArrayQuery(dear).as("names")));

  const { release } = await pool.one("SELECT current_setting('server_version_num')::int / 10000 AS release");
  assert.deepEqual(names, release === 18 ? null : []);
}
