// Example 13 of guide/11-query-builder.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { bind, call, col, select, Table, With } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const days = new Table("days");
  const order = new Table("order");
  const calendar = select(bind(Temporal.PlainDate.from("2026-11-01")).cast("date").as("day"))
    .unionAll(
      select(col("day").add(bind(1).cast("integer")))
        .from(days)
        .where(col("day").lt(bind(Temporal.PlainDate.from("2026-11-04")).cast("date"))),
    );

  const query = select(days.col("day"), call("count", order.col("id")).as("orders"))
    .from(days)
    .join(order, order.col("placed_at").cast("date").eq(days.col("day")), { kind: "left" })
    .groupBy(days.col("day"))
    .orderBy(days.col("day").asc())
    .with(With.recursive("days", calendar, { columns: ["day"] }));

  const rows = await pool.query(query);
  assert.deepEqual(
    rows.map((row) => [String(row.day), row.orders]),
    [["2026-11-01", 0n], ["2026-11-02", 1n], ["2026-11-03", 1n], ["2026-11-04", 0n]],
  );
}
