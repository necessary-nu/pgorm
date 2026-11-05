// Example 10 of guide/06-pagination.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { LifecycleError } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const names = await pool.transaction(async (tx) => {
    await tx.execute("UPDATE cake SET price = price + 1 WHERE bakery_id = 2");
    const names: string[] = [];
    for await (const cake of Cake.find().where(Cake.col("bakeryId").eq(2)).orderBy(Cake.col("name").asc()).stream(tx)) {
      names.push(`${cake.name} ${cake.price}`);
      await assert.rejects(tx.one("SELECT 1 AS one"), LifecycleError);
    }
    return names;
  });
  assert.deepEqual(names, ["Carrot Cake 4.80", "Vanilla Sponge 3.90"]);
}
