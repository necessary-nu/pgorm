// Example 11 of guide/13-values.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, Interval, model } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await pool.execute(
    "CREATE TABLE recipe (id serial PRIMARY KEY, name varchar NOT NULL, proving interval NOT NULL, keeps_for interval NOT NULL)",
  );
  const Recipe = model("recipe", {
    columns: {
      id: column("i32", { primaryKey: true, default: true }),
      name: column("text"),
      proving: column("interval"),
      keepsFor: column("interval", { name: "keeps_for" }),
    },
  });

  const HOUR = 3_600_000_000n;
  const sourdough = await Recipe.insert({
    name: "Sourdough",
    proving: new Interval({ microseconds: 14n * HOUR }),
    keepsFor: new Interval({ days: 3 }),
  }).returning().one(pool);

  assert.equal(sourdough.proving.microseconds, 14n * HOUR);
  assert.equal(sourdough.keepsFor.toString(), "P3D");
  assert.equal(sourdough.proving.toDuration().toString(), "PT14H");

  const slow = await Recipe.find()
    .where(Recipe.col("proving").gt(Temporal.Duration.from({ hours: 12 })))
    .count(pool);
  assert.equal(slow, 1);

  // Fields of different signs, which a Duration cannot hold.
  assert.equal(new Interval({ months: 1, days: -2 }).toString(), "P1M-2D");
  assert.throws(() => new Interval({ months: 1, days: -2 }).toDuration(), RangeError);
}
