// Example 4 of guide/22-migrations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { DatabaseError, type MigrationTrait, migrations } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const createNote: MigrationTrait = {
    name: "m20260201_000001_create_customer_note",
    async up(tx) {
      await tx.execute("CREATE TABLE customer_note (id serial PRIMARY KEY, note text NOT NULL)");
    },
  };
  // A migration that cannot succeed: the bakery has a customer table.
  const createCustomerAgain: MigrationTrait = {
    name: "m20260201_000002_create_customer_again",
    async up(tx) {
      await tx.execute("CREATE TABLE customer (id serial PRIMARY KEY)");
    },
  };

  // The second migration fails, so the first is rolled back with it.
  await assert.rejects(
    migrations([createNote, createCustomerAgain]).up(pool),
    (error) => error instanceof DatabaseError && error.sqlstate === "42P07",
  );
  const { tables } = await pool.one("SELECT count(*) AS tables FROM pg_tables WHERE tablename = 'customer_note'");
  assert.equal(tables, 0n);

  // Two runners at once: one applies the migration, the other waits and finds
  // nothing to do.
  const migrator = migrations([createNote]);
  await Promise.all([migrator.up(pool), migrator.up(pool)]);
  assert.equal((await migrator.getAppliedMigrations(pool)).length, 1);
}
