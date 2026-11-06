// Example 12 of guide/13-values.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, model, Range } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await pool.execute(
    "CREATE TABLE season (id serial PRIMARY KEY, cake varchar NOT NULL, on_sale daterange NOT NULL, batch_sizes int4range NOT NULL)",
  );
  const Season = model("season", {
    columns: {
      id: column("i32", { primaryKey: true, default: true }),
      cake: column("text"),
      onSale: column("daterange", { name: "on_sale" }),
      batchSizes: column("int4range", { name: "batch_sizes" }),
    },
  });

  const date = (text: string) => Temporal.PlainDate.from(text);
  const gingerbread = await Season.insert({
    cake: "Gingerbread",
    onSale: new Range(date("2026-11-15"), date("2027-01-07")),
    batchSizes: new Range(12, 48, "[]"),
  }).returning().one(pool);

  // A discrete range comes back in PostgreSQL's canonical form: [12,49).
  assert.equal(gingerbread.batchSizes.toString(), "[12,49)");
  assert.deepEqual([gingerbread.batchSizes.lower, gingerbread.batchSizes.upper], [12, 49]);

  // What is on sale over the Christmas week?
  const christmas = new Range(date("2026-12-21"), date("2026-12-28"));
  const onSale = await Season.select("cake").where(Season.col("onSale").overlaps(christmas)).all(pool);
  assert.deepEqual(onSale, [{ cake: "Gingerbread" }]);
}
