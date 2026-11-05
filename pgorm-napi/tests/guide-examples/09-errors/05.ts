// Example 5 of guide/09-errors.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, DecodeError, model } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // A model that forgets that a customer's notes may be missing.
  const Contact = model("customer", {
    columns: { id: column("i32", { primaryKey: true }), notes: column("text") },
  });
  await assert.rejects(Contact.find().all(pool), (error) =>
    error instanceof DecodeError && error.message === "notes is not nullable, and the row holds NULL");
}
