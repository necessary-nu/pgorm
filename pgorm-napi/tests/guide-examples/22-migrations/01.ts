// Example 1 of guide/22-migrations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ColumnDef, createTable, type MigrationTrait, migrations, Table } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const createDelivery: MigrationTrait = {
    name: "m20260201_000001_create_delivery",
    async up(tx) {
      await tx.execute(
        createTable(new Table("delivery"))
          .column(new ColumnDef("id", "integer").autoIncrement().notNull())
          .column(new ColumnDef("order_id", "integer").notNull())
          .column(new ColumnDef("address", "text").notNull())
          .primaryKey("id"),
      );
      // SQL text of several statements runs as one script.
      await tx.batchExecute(
        "CREATE INDEX delivery_order_index ON delivery (order_id); COMMENT ON TABLE delivery IS 'What the bakery sends out'",
      );
    },
  };
  const migrator = migrations([createDelivery]);

  await migrator.up(pool);
  await pool.execute("INSERT INTO delivery (order_id, address) VALUES (1, 'Storgata 1, Oslo')");

  // Running it again applies nothing: the ledger says the table exists.
  await migrator.up(pool);
  const applied = await migrator.getAppliedMigrations(pool);
  assert.deepEqual(applied.map((migration) => migration.name), ["m20260201_000001_create_delivery"]);
}
