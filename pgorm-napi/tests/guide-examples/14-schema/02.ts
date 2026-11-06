// Example 2 of guide/14-schema.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, fromModel, model } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const Recipe = model("recipe", {
    renameAll: "snake_case",
    comment: "How each cake is made",
    columns: {
      id: column("i32", { primaryKey: true, autoIncrement: true }),
      cakeId: column("i32", { indexed: true }),
      bakeMinutes: column("i32", { comment: "Minutes in the oven" }),
    },
  });

  const recipe = fromModel(Recipe);
  assert.deepEqual(recipe.indexes.map((index) => index.inspect().sql), [
    `CREATE INDEX "idx-recipe-cake_id" ON "recipe" ("cake_id")`,
  ]);
  assert.deepEqual(recipe.comments.map((comment) => comment.inspect().sql), [
    `COMMENT ON TABLE "recipe" IS 'How each cake is made'`,
    `COMMENT ON COLUMN "recipe"."bake_minutes" IS 'Minutes in the oven'`,
  ]);

  for (const statement of [recipe.table, ...recipe.indexes, ...recipe.comments]) await pool.execute(statement);
  const comment = await pool.one("SELECT obj_description('recipe'::regclass, 'pg_class') AS comment");
  assert.equal(comment.comment, "How each cake is made");
}
