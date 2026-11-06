// Example 13 of guide/04-writing.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Conflict } from "@necessary/pgorm";
import { Customer } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const signUp = (name: string, email: string) =>
    Customer.insert({ name, email }).onConflict(Conflict.on("email").update("name"));

  const kim = await signUp("Kimberly", "kim@example.com").returningUpsert(pool);
  assert.equal(kim?.kind, "updated");
  if (kim?.kind === "updated") assert.deepEqual([kim.old.name, kim.new.name], ["Kim", "Kimberly"]);

  const mo = await signUp("Mo", "mo@example.com").returningUpsert(pool);
  // Kim's upsert drew id 3 before it conflicted, so Mo gets 4.
  assert.equal(mo?.kind, "inserted");
  assert.equal(mo?.new.id, 4);
}
