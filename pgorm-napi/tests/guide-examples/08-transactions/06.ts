// Example 6 of guide/08-transactions.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { DatabaseError } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const report = { mode: "readOnly", isolation: "repeatableRead" } as const;

  // Two reads that see the same snapshot of the database.
  const [cakes, total] = await pool.transaction(async (tx) => {
    const cakes = await Cake.find().count(tx);
    const { total } = await tx.one("SELECT sum(price) AS total FROM cake");
    return [cakes, String(total)];
  }, report);
  assert.deepEqual([cakes, total], [5, "20.40"]);

  // A write in a read-only transaction is refused.
  await assert.rejects(
    pool.transaction((tx) => Cake.delete().where(Cake.col("id").eq(1)).execute(tx), report),
    (error) => error instanceof DatabaseError && error.sqlstate === "25006",
  );
}
