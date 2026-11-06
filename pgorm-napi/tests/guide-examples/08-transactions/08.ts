// Example 8 of guide/08-transactions.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { DatabaseError, Decimal } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const retryable = (error: unknown) =>
    error instanceof DatabaseError && (error.sqlstate === "40001" || error.sqlstate === "40P01");

  await using other = await pool.acquire();
  let attempts = 0;
  let doubled: Decimal | undefined;
  for (let retries = 3; doubled === undefined; retries--) {
    try {
      doubled = await pool.transaction(async (tx) => {
        attempts += 1;
        const fudge = await Cake.findByKey({ id: 1 }).one(tx);
        if (attempts === 1) {
          // Someone else's change commits after this transaction read the row.
          await other.execute("UPDATE cake SET price = price + 1 WHERE id = 1");
        }
        const price = new Decimal(String(Number(fudge.price.toString()) * 2));
        const changed = await Cake.update({ price }).where(Cake.key({ id: 1 })).returning("price").one(tx);
        return changed.price;
      }, { mode: "readWrite", isolation: "serializable" });
    } catch (error) {
      if (!retryable(error) || retries === 0) throw error;
    }
  }
  assert.deepEqual([attempts, String(doubled)], [2, "11.00"]);
}
