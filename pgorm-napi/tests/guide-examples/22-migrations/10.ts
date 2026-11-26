// Example 10 of guide/22-migrations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, type Migration, migrations } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const create = (name: string, table: string): Migration => ({
    name,
    async up(tx) {
      await tx.execute(`CREATE TABLE ${table} (id serial PRIMARY KEY)`);
    },
  });
  const delivery = create("m20260201_000001_create_delivery", "delivery");
  const voucher = create("m20260201_000002_create_voucher", "voucher");

  // The newer build, which added the voucher table.
  await migrations([delivery, voucher]).up(pool);

  // The build before it, still starting while the newer one rolls out: it
  // warns that the database records m20260201_000002_create_voucher, and
  // applies nothing.
  const older = migrations([delivery], { unknownMigrations: "warn" });
  await older.up(pool);
  const warned: string[] = [];
  await older.status(pool, { logger: { info() {}, warn: (line) => warned.push(line) } });
  assert.deepEqual(warned, [
    "The database records migration 'm20260201_000002_create_voucher' as applied, " +
      "but this migrator does not know it, so the database is ahead of this build",
  ]);

  // Without the opt-in, the same build refuses to start.
  await assert.rejects(
    migrations([delivery]).up(pool),
    (error) => error instanceof ConstructionError && error.message.includes("'m20260201_000002_create_voucher' is missing"),
  );

  // A build with a migration of its own to apply refuses, even when it warns.
  const courier = create("m20260301_000001_create_courier", "courier");
  await assert.rejects(
    migrations([delivery, courier], { unknownMigrations: "warn" }).up(pool),
    (error) => error instanceof ConstructionError && error.message.includes("out of order"),
  );
}
