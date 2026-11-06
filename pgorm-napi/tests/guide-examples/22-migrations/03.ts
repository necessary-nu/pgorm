// Example 3 of guide/22-migrations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { migrations, SqlMigration, sqlMigration } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const bakery = SqlMigration.read("guide/migrations/m20260101_000001_create_bakery.sql");
  assert.equal(bakery.name, "m20260101_000001_create_bakery");
  assert.match(bakery.sql, /^CREATE TABLE "bakery" \(/);
  assert.equal(bakery.checksum.length, 64);

  const index = sqlMigration(
    "m20260102_000001_index_bakery_city",
    "CREATE INDEX bakery_city_index ON bakery (city);",
  );
  await migrations([bakery, index]).up(pool);
  const { found } = await pool.one(
    "SELECT count(*) AS found FROM pg_indexes WHERE indexname = 'bakery_city_index'",
  );
  assert.equal(found, 1n);
}
