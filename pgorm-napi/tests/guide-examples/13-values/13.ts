// Example 13 of guide/13-values.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, Multirange, Range, Value } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await assert.rejects(pool.one("SELECT $1 AS span", [new Range(1, 5) as never]), ConstructionError);

  const empty = await pool.one("SELECT $1::int4range AS span", [new Value(new Range(5, 5), "int4range")]);
  assert.ok(empty.span instanceof Range);
  assert.equal(empty.span.isEmpty, true);

  const merged = await pool.one("SELECT $1::int4multirange AS spans", [
    new Value(new Multirange([new Range(5, 8), new Range(1, 3), new Range(3, 4)]), "int4multirange"),
  ]);
  assert.equal(String(merged.spans), "{[1,4),[5,8)}");
}
