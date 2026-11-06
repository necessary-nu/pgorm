// Example 12 of guide/03-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, model } from "@necessary/pgorm";

export default async function example(): Promise<void> {
  // The schema this deployment keeps its tables in, chosen at startup.
  const region = "oslo";

  const RegionalCake = model("cake", {
    schema: region,
    columns: {
      id: column("i32", { primaryKey: true }),
      name: column("text"),
    },
  });

  assert.equal(RegionalCake.find().inspect().sql, `SELECT "cake"."id", "cake"."name" FROM "oslo"."cake"`);
}
