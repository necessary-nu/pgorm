// Example 9 of guide/10-raw-sql.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Decimal, pipeline as pl, Table } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const under = new Decimal("4.00");
  const cheap = pl.from(new Table("cake"))
    .filterWith((binder) => pl.col("cake", "price").lt(binder.bind(under)))
    .sort(pl.col("cake", "name"))
    .select(pl.col("cake", "name"), pl.col("cake", "price"));

  const { sql, values } = cheap.inspect();
  assert.equal(sql, "SELECT name, price FROM cake WHERE price < $1 ORDER BY name");
  assert.deepEqual(values.map((value) => String(value.value)), ["4.00"]);

  const rows = await cheap.all(pool);
  assert.deepEqual(rows.map((row) => row.name), ["Carrot Cake", "Lemon Drizzle", "Vanilla Sponge"]);
}
