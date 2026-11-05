// Example 1 of guide/27-postgres-releases.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import {
  call, col, ColumnDef, createExtension, createTable, ExclusionViolationError, Table,
} from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const create = createTable(new Table("oven_booking"))
    .column(new ColumnDef("oven_id", "integer").notNull())
    .column(new ColumnDef("during", "tstzrange").notNull())
    .column(new ColumnDef("batch", "uuid").notNull().default(call("uuidv7")))
    .column(new ColumnDef("trays", "integer").notNull())
    .column(new ColumnDef("cakes", "integer").notNull().generated(col("trays").mul(12), "virtual"))
    .primaryKey("oven_id", { withoutOverlaps: "during" });
  assert.equal(
    create.inspect().sql,
    `CREATE TABLE "oven_booking" ( "oven_id" integer NOT NULL, "during" tstzrange NOT NULL, "batch" uuid NOT NULL DEFAULT UUIDV7(), "trays" integer NOT NULL, "cakes" integer NOT NULL GENERATED ALWAYS AS ("trays" * 12) VIRTUAL, PRIMARY KEY ("oven_id", "during" WITHOUT OVERLAPS) )`,
  );

  await pool.execute(createExtension("btree_gist", { ifNotExists: true }));
  await pool.execute(create);

  const book = "INSERT INTO oven_booking (oven_id, during, trays) VALUES ($1, tstzrange($2, $3), $4) RETURNING cakes";
  const at = (time: string) => Temporal.Instant.from(`2026-11-04T${time}:00Z`);
  const booked = await pool.one(book, [1, at("06:00"), at("08:00"), 3]);
  assert.equal(booked.cakes, 36);

  // Oven 1 is booked from six to eight, so seven to nine overlaps it.
  await assert.rejects(pool.one(book, [1, at("07:00"), at("09:00"), 2]), ExclusionViolationError);
}
