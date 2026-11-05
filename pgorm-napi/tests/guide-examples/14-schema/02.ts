// Example 2 of guide/14-schema.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ColumnDef, commentOnColumn, commentOnTable, createIndex, createTable } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const recipe = createTable("recipe")
    .column(new ColumnDef("id", "integer").notNull().autoIncrement())
    .column(new ColumnDef("cake_id", "integer").notNull())
    .column(new ColumnDef("bake_minutes", "integer").notNull())
    .primaryKey("id");

  const index = createIndex("recipe", "cake_id", { name: "idx-recipe-cake_id" });
  assert.equal(index.inspect().sql, `CREATE INDEX "idx-recipe-cake_id" ON "recipe" ("cake_id")`);

  const comments = [
    commentOnTable("recipe", "How each cake is made"),
    commentOnColumn("recipe", "bake_minutes", "Minutes in the oven"),
  ];
  assert.deepEqual(comments.map((comment) => comment.inspect().sql), [
    `COMMENT ON TABLE "recipe" IS 'How each cake is made'`,
    `COMMENT ON COLUMN "recipe"."bake_minutes" IS 'Minutes in the oven'`,
  ]);

  for (const statement of [recipe, index, ...comments]) await pool.execute(statement);
  const comment = await pool.one("SELECT obj_description('recipe'::regclass, 'pg_class') AS comment");
  assert.equal(comment.comment, "How each cake is made");
}
