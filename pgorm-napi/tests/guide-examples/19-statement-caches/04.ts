// Example 4 of guide/19-statement-caches.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Pool, STATEMENT_CACHE_CHANNEL } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool, url }: GuideContext): Promise<void> {
  assert.equal(STATEMENT_CACHE_CHANNEL, "pgorm_statement_caches");
  const INSERT = "INSERT INTO box (contents) VALUES ($1) RETURNING ribbon";

  // The process that migrates.
  await pool.execute("CREATE DOMAIN ribbon AS text DEFAULT 'red'");
  await pool.execute("CREATE TABLE box (id serial PRIMARY KEY, contents text, ribbon ribbon)");

  // Another process's pool, listening for clears.
  await using shop = new Pool(url, { maxSize: 1, statementCacheChannel: STATEMENT_CACHE_CHANNEL });
  assert.equal((await shop.one(INSERT, ["truffles"])).ribbon, "red");

  await pool.execute("ALTER DOMAIN ribbon SET DEFAULT 'gold'");
  await pool.notify(STATEMENT_CACHE_CHANNEL, "");

  // The clear arrives as a notification, so the shop sees it a moment later.
  const deadline = Date.now() + 10_000;
  let ribbon = "red";
  while (ribbon !== "gold" && Date.now() < deadline) {
    ribbon = String((await shop.one(INSERT, ["fudge"])).ribbon);
    if (ribbon !== "gold") await new Promise((resolve) => setTimeout(resolve, 50));
  }
  assert.equal(ribbon, "gold");
}
