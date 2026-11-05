// Example 3 of guide/13-values.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const precise = Temporal.Instant.from("2026-11-06T04:28:00.0000001Z");
  await assert.rejects(pool.one("SELECT $1::timestamptz AS at", [precise]), ConstructionError);

  const rounded = precise.round({ smallestUnit: "microsecond" });
  const row = await pool.one("SELECT $1::timestamptz AS at", [rounded]);
  assert.ok(row.at instanceof Temporal.Instant);
  assert.equal(row.at.toString(), "2026-11-06T04:28:00Z");

  await assert.rejects(pool.one("SELECT $1::timestamptz AS at", [new Date() as never]), /a Date is refused/);
}
