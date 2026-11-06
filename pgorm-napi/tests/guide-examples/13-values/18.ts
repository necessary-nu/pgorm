// Example 18 of guide/13-values.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Value } from "@necessary/pgorm";

export default async function example(): Promise<void> {
  const embedding = new Value(new Float32Array([0.25, 0.5, 1]), "vector");
  assert.equal(embedding.kind, "vector");
  assert.deepEqual([...(embedding.value as Float32Array)], [0.25, 0.5, 1]);
}
