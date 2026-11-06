// Example 7 of guide/22-migrations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { migrations } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // What SeaORM left behind: the table, and the ledger row saying it exists.
  await pool.transaction(async (tx) => {
    await tx.execute("CREATE TABLE delivery (id serial PRIMARY KEY)");
    await tx.execute("CREATE TABLE seaql_migrations (version varchar PRIMARY KEY, applied_at bigint NOT NULL)");
    await tx.execute("INSERT INTO seaql_migrations VALUES ('m20220101_000001_create_delivery', 1640995200)");
  });

  // The migration is not run again, which would fail: the table exists.
  await migrations([
    {
      name: "m20220101_000001_create_delivery",
      async up(tx) {
        await tx.execute("CREATE TABLE delivery (id serial PRIMARY KEY)");
      },
    },
  ]).up(pool);
  const { applied } = await pool.one("SELECT applied_at AS applied FROM pgorm_migrations");
  assert.equal(applied, 1640995200n);
}
