// Example 11 of guide/10-raw-sql.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, sql } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const BAKERS_AT = sql("SELECT name FROM baker WHERE bakery_id = $1 ORDER BY name");

  const names = (await pool.query(BAKERS_AT, [1])).map((row) => row.name);
  assert.deepEqual(names, ["Ada", "Bo"]);

  assert.throws(
    () => sql("SELECT name FROM baker\nORDER name"),
    (error) =>
      error instanceof ConstructionError &&
      error.message === 'PostgreSQL rejected this SQL: syntax error at or near "name"\n\n  ORDER name\n        ^',
  );
}
