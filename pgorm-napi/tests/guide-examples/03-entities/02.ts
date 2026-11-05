// Example 2 of guide/03-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, ConstructionError } from "@necessary/pgorm";

export default async function example(): Promise<void> {
  // A misspelt option is never silently left out.
  assert.throws(() => column("i32", { primary_key: true } as never), TypeError);

  // A key column cannot hold NULL.
  assert.throws(() => column("i32", { primaryKey: true, nullable: true }), ConstructionError);
}
