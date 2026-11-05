// Example 1 of guide/19-statement-caches.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { type Connection, Pool } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ url }: GuideContext): Promise<void> {
  /** How many statements this session holds prepared whose text starts with `prefix`. */
  async function prepared(db: Connection, prefix: string): Promise<bigint> {
    const row = await db.one(
      "SELECT count(*) AS prepared FROM pg_prepared_statements WHERE starts_with(statement, $1)",
      [prefix],
    );
    return row.prepared as bigint;
  }

  for (const [statementCacheSize, kept] of [[64, 1n], [0, 0n]] as const) {
    await using cached = new Pool(url, { maxSize: 1, statementCacheSize });
    await using db = await cached.acquire();
    await db.query("SELECT name FROM bakery");
    assert.equal(await prepared(db, "SELECT name FROM bakery"), kept);
  }
}
