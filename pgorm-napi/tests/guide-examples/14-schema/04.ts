// Example 4 of guide/14-schema.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { alterTable, ColumnDef } from "@necessary/pgorm";
import { Baker, Bakery } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // Each bakery names its head baker, and each baker their bakery.
  await pool.execute(alterTable(Bakery.table).addColumn(new ColumnDef("head_baker_id", "integer")));
  const headBaker = alterTable(Bakery.table)
    .addForeignKey("head_baker_id", Baker.table, "id", { deferrability: "deferrableInitiallyDeferred" });
  assert.equal(
    headBaker.inspect().sql,
    `ALTER TABLE "bakery" ADD FOREIGN KEY ("head_baker_id") REFERENCES "baker" ("id") DEFERRABLE INITIALLY DEFERRED`,
  );
  await pool.execute(headBaker);

  // Point the bakery at a baker who does not exist yet: the key is checked at
  // commit, by which time the baker does.
  await pool.transaction(async (tx) => {
    await tx.execute(`UPDATE "bakery" SET "head_baker_id" = 4 WHERE "id" = 1`);
    await Baker.insert({ name: "Di", hiredOn: Temporal.PlainDate.from("2026-11-01"), bakeryId: 1 }).execute(tx);
  });
  const sunrise = await pool.one(`SELECT "head_baker_id" FROM "bakery" WHERE "id" = 1`);
  assert.equal(sunrise.head_baker_id, 4);
}
