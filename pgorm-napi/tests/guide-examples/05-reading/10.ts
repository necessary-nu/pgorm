// Example 10 of guide/05-reading.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, model } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  /** A cake on the menu board: two of the cake table's columns. */
  const MenuLine = model("cake", {
    columns: {
      id: column("i32", { primaryKey: true }),
      name: column("text"),
      freeFromGluten: column("bool", { name: "gluten_free" }),
    },
  });

  const lemon = await MenuLine.find().where(MenuLine.col("id").eq(2)).one(pool);
  assert.deepEqual(lemon, { id: 2, name: "Lemon Drizzle", freeFromGluten: true });
}
