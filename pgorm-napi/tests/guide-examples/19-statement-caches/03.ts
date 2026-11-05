// Example 3 of guide/19-statement-caches.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Pool } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ url }: GuideContext): Promise<void> {
  const INSERT = "INSERT INTO box (contents) VALUES ($1) RETURNING ribbon";
  const PREPARED =
    "SELECT count(*) AS kept FROM pg_prepared_statements WHERE starts_with(statement, 'INSERT INTO box')";

  // One connection, so every checkout is the same session.
  await using single = new Pool(url, { maxSize: 1 });
  let db = await single.acquire();
  await db.execute("CREATE DOMAIN ribbon AS text DEFAULT 'red'");
  await db.execute("CREATE TABLE box (id serial PRIMARY KEY, contents text, ribbon ribbon)");
  assert.equal((await db.one(INSERT, ["truffles"])).ribbon, "red");

  await db.execute("ALTER DOMAIN ribbon SET DEFAULT 'gold'");
  single.clearStatementCaches();

  // The connection was checked out, so it keeps its statement until returned.
  assert.equal((await db.one(PREPARED)).kept, 1n);
  await db.close();

  // Lent again, it was cleared first, and the insert is prepared afresh.
  db = await single.acquire();
  assert.equal((await db.one(PREPARED)).kept, 0n);
  assert.equal((await db.one(INSERT, ["fudge"])).ribbon, "gold");
  await db.close();
}
