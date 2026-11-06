// Example 8 of guide/17-copy.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, copyTo, select, Table } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cake = new Table("cake");
  const sunrise = select(cake.col("name")).from(cake).where(cake.col("bakery_id").eq(1));
  assert.throws(() => copyTo(sunrise.limitWithTies(1), "csv"), ConstructionError);

  const cheapest = copyTo(sunrise.orderBy(cake.col("price").asc()).limitWithTies(1), "csv");
  const decoder = new TextDecoder();
  let csv = "";
  for await (const chunk of await pool.copyOut(cheapest)) csv += decoder.decode(chunk, { stream: true });
  assert.equal(csv, "Lemon Drizzle\n");
}
