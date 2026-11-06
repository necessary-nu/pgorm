// Example 4 of guide/03-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Customer } from "../../../guide/bakery.ts";

export default async function example(): Promise<void> {
  const { primaryKey, uniqueKeys, fields } = Customer.describe();
  assert.deepEqual(primaryKey, ["id"]);
  assert.deepEqual(uniqueKeys, [{ name: null, fields: ["email"], nullsNotDistinct: false }]);
  assert.equal(fields.notes?.kind, "text");
  assert.equal(fields.notes?.nullable, true);
}
