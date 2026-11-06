// Example 13 of guide/10-raw-sql.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, prql } from "@necessary/pgorm";

export default async function example(): Promise<void> {
  // `$1` is used, and nothing binds it.
  assert.throws(
    () => prql("from cake | filter price < $1"),
    (error) =>
      error instanceof ConstructionError &&
      error.message === "the query has 1 placeholder(s) but 0 argument(s) were given: nothing binds $1",
  );
  assert.throws(() => prql("from cake | filter id > $1 | filter id < $3", [1, 2]), ConstructionError);
}
