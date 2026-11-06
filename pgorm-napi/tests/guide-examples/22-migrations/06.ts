// Example 6 of guide/22-migrations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { migrations } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const migrator = migrations(
    [
      {
        name: "m20260201_000001_create_delivery",
        async up(tx) {
          await tx.execute("CREATE TABLE delivery (id serial PRIMARY KEY, address text NOT NULL)");
        },
      },
    ],
    { migrationTableSchema: "ops", migrationTableName: "bakery_migrations" },
  );

  await migrator.up(pool);
  const row = await pool.one("SELECT version, checksum FROM ops.bakery_migrations");
  assert.equal(row.version, "m20260201_000001_create_delivery");
  assert.equal(row.checksum, null);
}
