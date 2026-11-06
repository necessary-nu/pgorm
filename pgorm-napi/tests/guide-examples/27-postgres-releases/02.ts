// Example 2 of guide/27-postgres-releases.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { POSTGRES_TARGET, version } from "@necessary/pgorm";

export default async function example(): Promise<void> {
  assert.ok(POSTGRES_TARGET === 18 || POSTGRES_TARGET === 19);
  assert.match(version, /^\d+\.\d+\.\d+/);

  // In a program that relies on 19:
  // if (POSTGRES_TARGET < 19) throw new Error("load a pgorm addon built with its pg-19 feature");
}
