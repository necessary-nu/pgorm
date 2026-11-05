// Example 5 of guide/13-values.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { DecodeError, Decimal } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await pool.execute("CREATE TABLE tip_jar (id serial PRIMARY KEY, amount money NOT NULL)");
  await pool.execute("INSERT INTO tip_jar (amount) VALUES ($1::numeric::money)", [new Decimal("12.345")]);

  // Rounded to the currency's two places.
  const tips = await pool.one("SELECT amount::numeric AS amount FROM tip_jar");
  assert.equal(String(tips.amount), "12.35");

  await assert.rejects(pool.query("SELECT amount FROM tip_jar"), DecodeError);
}
