// Example 2 of guide/09-errors.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { CheckViolationError, Decimal, ExclusionViolationError } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await pool.batchExecute(`
    ALTER TABLE cake ADD CONSTRAINT cake_price_positive CHECK (price > 0);
    CREATE EXTENSION IF NOT EXISTS btree_gist;
    CREATE TABLE oven_booking (
      oven integer NOT NULL,
      during tstzrange NOT NULL,
      EXCLUDE USING gist (oven WITH =, during WITH &&)
    )`);

  const free = await pool
    .execute("UPDATE cake SET price = $1 WHERE id = 1", [new Decimal("0")])
    .catch((error: unknown) => error);
  assert.ok(free instanceof CheckViolationError);
  assert.equal(free.constraint, "cake_price_positive");

  const book = "INSERT INTO oven_booking (oven, during) VALUES ($1, tstzrange($2, $3))";
  const at = (time: string) => Temporal.Instant.from(`2026-11-04T${time}:00Z`);
  await pool.execute(book, [1, at("06:00"), at("08:00")]);
  await assert.rejects(pool.execute(book, [1, at("07:00"), at("09:00")]), ExclusionViolationError);
}
