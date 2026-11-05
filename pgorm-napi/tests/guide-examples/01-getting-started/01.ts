// Example 1 of guide/01-getting-started.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { connect } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ url }: GuideContext): Promise<void> {
  // The URL of the bakery database, such as
  // "postgres://postgres:postgres@localhost:5432/bakery?sslmode=disable".
  await using pool = await connect(url);

  const row = await pool.one("SELECT count(*) AS cakes FROM cake");
  assert.equal(row.cakes, 5n);
}
