// Example 1 of guide/22-migrations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ColumnDef, createIndex, createTable, Table } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const delivery = new Table("delivery");
  const create = createTable(delivery)
    .column(new ColumnDef("id", "integer").identity("always"))
    .column(new ColumnDef("order_id", "integer").notNull())
    .primaryKey("id");

  await assert.rejects(
    pool.transaction(async (tx) => {
      await tx.execute(createIndex("cake", { on: "name" }, { name: "cake_name_index" }));
      await tx.execute(create);
      await tx.execute(create);
    }),
    /already exists/,
  );

  const { indexes } = await pool.one(
    "SELECT count(*) AS indexes FROM pg_indexes WHERE indexname = 'cake_name_index'",
  );
  assert.equal(indexes, 0n);
}
