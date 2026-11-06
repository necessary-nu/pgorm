// Example 12 of guide/22-migrations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { type MigrationTrait, migrations, SqlMigration } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const frozen = SqlMigration.readDir("guide/migrations");
  assert.deepEqual(frozen.map((file) => file.name), [
    "m20260101_000001_create_bakery",
    "m20260201_000001_create_payment",
    "m20260301_000001_add_voucher_1",
    "m20260301_000001_add_voucher_2",
  ]);
  assert.deepEqual(frozen.map((file) => file.commitsAfter), [false, false, true, false]);

  const firstVoucher: MigrationTrait = {
    name: "m20260301_000002_first_voucher",
    async up(tx) {
      // The base row and its kind's row reference each other, so the keys are
      // checked once both are written.
      await tx.execute("SET CONSTRAINTS ALL DEFERRED");
      await tx.execute("INSERT INTO payment (id, kind, order_id, amount) VALUES (1, 'voucher', 1, 9.50)");
      await tx.execute("INSERT INTO voucher_payment (payment_id, code) VALUES (1, 'SPRING')");
    },
  };
  const migrator = migrations([...frozen, firstVoucher]);

  await migrator.up(pool);
  assert.equal((await migrator.getAppliedMigrations(pool)).length, 5);
  const { checksum } = await pool.one(
    "SELECT checksum FROM pgorm_migrations WHERE version = 'm20260201_000001_create_payment'",
  );
  assert.equal(checksum, frozen[1]!.checksum);
  const { code } = await pool.one("SELECT code FROM voucher_payment JOIN payment ON payment.voucher_id = payment_id");
  assert.equal(code, "SPRING");
}
