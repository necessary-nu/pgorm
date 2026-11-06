// Example 9 of guide/09-errors.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { DatabaseError, PgormError, UniqueViolationError } from "@necessary/pgorm";
import { Customer } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  async function signUp(name: string, email: string): Promise<"welcome" | "known"> {
    try {
      await Customer.insert({ name, email }).execute(pool);
      return "welcome";
    } catch (error) {
      if (error instanceof UniqueViolationError) return "known";
      throw error;
    }
  }

  assert.equal(await signUp("Mo", "mo@example.com"), "welcome");
  assert.equal(await signUp("Kim again", "kim@example.com"), "known");

  const refused = await pool.query("SELECT * FROM pastry").catch((error: unknown) => error);
  assert.ok(refused instanceof DatabaseError && refused instanceof PgormError);
  assert.equal(refused.name, "DatabaseError");
}
