// Example 20 of guide/11-query-builder.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { bind, col, Range, select, Table, Value } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const secondOfNovember = new Range(
    Temporal.Instant.from("2026-11-02T00:00:00Z"),
    Temporal.Instant.from("2026-11-03T00:00:00Z"),
  );
  const query = select(col("id"))
    .from(new Table("order"))
    .where(bind(new Value(secondOfNovember, "tstzrange")).cast("tstzrange").contains(col("placed_at")));

  assert.equal(
    query.inspect().sql,
    `SELECT "id" FROM "order" WHERE CAST($1::tstzrange AS tstzrange) @> "placed_at"`,
  );
  assert.deepEqual(await pool.query(query), [{ id: 1 }]);
}
