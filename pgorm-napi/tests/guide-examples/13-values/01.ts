// Example 1 of guide/13-values.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, Decimal, Value } from "@necessary/pgorm";

export default async function example(): Promise<void> {
  const price = new Value(new Decimal("4.50"));
  assert.equal(price.kind, "decimal");
  assert.equal(String(price.value), "4.50");

  // SQL NULL of its own kind, and JSON's null, which is not SQL NULL.
  assert.ok(Value.null("text").isNull);
  assert.equal(Value.null("text").kind, "text");
  assert.equal(Value.json(null).isNull, false);

  // A plain number infers as an int8; a Value says otherwise.
  assert.equal(new Value(5).kind, "i64");
  assert.equal(new Value(5, "i16").kind, "i16");
  assert.throws(() => new Value(70000, "i16"), ConstructionError);
}
