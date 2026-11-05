// Example 2 of guide/13-values.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, model } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await pool.execute(
    `CREATE TABLE shift (id serial PRIMARY KEY, baker_id integer NOT NULL, "on" date NOT NULL,
       starts time NOT NULL, planned timestamp NOT NULL, clocked_in timestamptz)`,
  );
  const Shift = model("shift", {
    columns: {
      id: column("i32", { primaryKey: true, default: true }),
      bakerId: column("i32", { name: "baker_id" }),
      on: column("date"),
      starts: column("time"),
      planned: column("datetime"),
      clockedIn: column("datetime_utc", { name: "clocked_in", nullable: true }),
    },
  });

  const early = await Shift.insert({
    bakerId: 1,
    on: Temporal.PlainDate.from("2026-11-06"),
    starts: Temporal.PlainTime.from("05:30"),
    planned: Temporal.PlainDateTime.from("2026-11-06T05:30"),
    clockedIn: Temporal.Instant.from("2026-11-06T04:28:00Z"),
  }).returning().one(pool);

  assert.equal(early.on.toString(), "2026-11-06");
  assert.equal(early.starts.toString(), "05:30:00");
  assert.equal(early.clockedIn?.toString(), "2026-11-06T04:28:00Z");

  // Dates compare as dates.
  const thisWeek = await Shift.find()
    .where(Shift.col("on").between(Temporal.PlainDate.from("2026-11-02"), Temporal.PlainDate.from("2026-11-08")))
    .count(pool);
  assert.equal(thisWeek, 1);
}
