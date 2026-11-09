// Example 3 of guide/23-codegen.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Cake } from "../../../guide/bakery.ts";

export default async function example(): Promise<void> {
  const { table, primaryKey, fields } = Cake.describe();
  assert.equal(table, "cake");
  assert.deepEqual(primaryKey, ["id"]);
  assert.equal(fields.glutenFree?.column, "gluten_free");
  assert.equal(fields.price?.kind, "decimal");
  assert.equal(fields.flavor?.kind, `enum "public"."flavor"`);
}
