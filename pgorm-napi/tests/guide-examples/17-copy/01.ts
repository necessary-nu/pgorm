// Example 1 of guide/17-copy.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, copyFrom, copyTo, type CopyFrom } from "@necessary/pgorm";

export default async function example(): Promise<void> {
  const text: CopyFrom<"text"> = copyFrom("cake", "text");
  assert.throws(() => (text as CopyFrom<any>).quote("'"), ConstructionError);
  assert.throws(() => (copyTo("cake", "binary") as any).header(), ConstructionError);
}
