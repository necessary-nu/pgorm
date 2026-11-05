// Example 3 of guide/14-schema.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { alterTable, col, ColumnDef, createTable, dropTable, renameTable } from "@necessary/pgorm";
import { Order } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const create = createTable("delivery")
    .column(new ColumnDef("id", "integer").notNull().identity("always"))
    .column(new ColumnDef("order_id", "integer").notNull())
    .column(new ColumnDef("address", "text").notNull())
    .column(new ColumnDef("due_at", "timestamptz").notNull())
    .column(new ColumnDef("fragile", "boolean").notNull().default(false))
    .primaryKey("id")
    .foreignKey("order_id", Order.table, "id", { onDelete: "cascade" })
    .check(col("address").ne(""));
  assert.equal(
    create.inspect().sql,
    [
      `CREATE TABLE "delivery" (`,
      `"id" integer NOT NULL GENERATED ALWAYS AS IDENTITY,`,
      `"order_id" integer NOT NULL,`,
      `"address" text NOT NULL,`,
      `"due_at" timestamp with time zone NOT NULL,`,
      `"fragile" bool NOT NULL DEFAULT FALSE,`,
      `PRIMARY KEY ("id"),`,
      `FOREIGN KEY ("order_id") REFERENCES "order" ("id") ON DELETE CASCADE,`,
      `CHECK ("address" <> '')`,
      `)`,
    ].join(" "),
  );
  await pool.execute(create);

  // A column added later, and the table renamed.
  const alter = alterTable("delivery").addColumn(new ColumnDef("notes", "text"));
  assert.equal(alter.inspect().sql, `ALTER TABLE "delivery" ADD COLUMN "notes" text`);
  await pool.execute(alter);

  const rename = renameTable("delivery", "courier_run");
  assert.equal(rename.inspect().sql, `ALTER TABLE "delivery" RENAME TO "courier_run"`);
  await pool.execute(rename);

  const drop = dropTable("courier_run", { ifExists: true });
  assert.equal(drop.inspect().sql, `DROP TABLE IF EXISTS "courier_run"`);
  await pool.execute(drop);
}
