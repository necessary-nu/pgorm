// Example 7 of guide/17-copy.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { col, ConstructionError, copyTo, Table, update } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cake = new Table("cake");
  const raise = update(cake).set("price", col("price").add(1)).where(col("bakery_id").eq(2));
  assert.throws(() => copyTo(raise, "csv"), ConstructionError);

  const raised = copyTo(raise.returning([col("name"), col("price")]), "csv");
  const decoder = new TextDecoder();
  let csv = "";
  for await (const chunk of await pool.copyOut(raised)) csv += decoder.decode(chunk, { stream: true });
  assert.deepEqual(csv.trim().split("\n").sort(), ["Carrot Cake,4.80", "Vanilla Sponge,3.90"]);
}
