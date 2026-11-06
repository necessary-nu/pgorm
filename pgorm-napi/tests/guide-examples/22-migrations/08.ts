// Example 8 of guide/22-migrations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { type MigrationTrait, migrations } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const create = (name: string, table: string): MigrationTrait => ({
    name,
    async up(tx) {
      await tx.execute(`CREATE TABLE ${table} (id serial PRIMARY KEY)`);
    },
  });
  const migrator = migrations([
    create("m20260201_000001_create_delivery", "delivery"),
    create("m20260201_000002_create_voucher", "voucher"),
  ]);

  assert.equal((await migrator.getPendingMigrations(pool)).length, 2);

  // Apply one migration of the two.
  await migrator.up(pool, 1);

  assert.deepEqual(await migrator.getMigrationWithStatus(pool), [
    { name: "m20260201_000001_create_delivery", status: "applied" },
    { name: "m20260201_000002_create_voucher", status: "pending" },
  ]);

  const logged: string[] = [];
  await migrator.status(pool, { logger: { info: (line) => logged.push(line) } });
  assert.equal(logged.at(-1), "Migration 'm20260201_000002_create_voucher'... Pending");
}
