// Example 6 of guide/21-metrics.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { QueryContext, type QueryFingerprint } from "@necessary/pgorm";

export default async function example(): Promise<void> {
  function fingerprint(sql: string): QueryFingerprint {
    const found = new QueryContext("queryAll", sql).fingerprint;
    assert.ok(found, "the statement parses");
    return found;
  }

  const byId = fingerprint("SELECT id FROM widget WHERE id = 1");
  assert.equal(String(byId), "394a2f90c244bffe");
  assert.equal(byId.value, 0x394a2f90c244bffen);
  assert.ok(byId.equals(fingerprint("SELECT id FROM widget WHERE id = 4242")));
  assert.ok(byId.equals(fingerprint("SELECT id FROM public.widget WHERE id = 1")));
  assert.ok(!byId.equals(fingerprint("SELECT name FROM widget WHERE id = 1")));
  assert.ok(fingerprint("SELECT id FROM widget w WHERE id = 1").equals(fingerprint("SELECT id FROM gadget w WHERE id = 1")));

  assert.equal(new QueryContext("begin").fingerprint, null);
}
