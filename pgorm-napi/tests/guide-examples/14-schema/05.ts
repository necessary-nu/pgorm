// Example 5 of guide/14-schema.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { col, createIndex, dropIndex } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // One cake of each name per bakery.
  const unique = createIndex(Cake.table, "bakery_id", { name: "uq-cake-bakery-name" }).column("name").unique();
  assert.equal(unique.inspect().sql, `CREATE UNIQUE INDEX "uq-cake-bakery-name" ON "cake" ("bakery_id", "name")`);
  await pool.execute(unique);

  // Only the gluten-free cakes, which the shop's filter asks for most.
  const partial = createIndex(Cake.table, "price", { name: "idx-cake-gluten-free-price" })
    .where(col("gluten_free").eq(true));
  assert.equal(
    partial.inspect().sql,
    `CREATE INDEX "idx-cake-gluten-free-price" ON "cake" ("price") WHERE "gluten_free" = TRUE`,
  );
  await pool.execute(partial);

  const drop = dropIndex(Cake.table, "idx-cake-gluten-free-price");
  assert.equal(drop.inspect().sql, `DROP INDEX "idx-cake-gluten-free-price"`);
  await pool.execute(drop);
}
