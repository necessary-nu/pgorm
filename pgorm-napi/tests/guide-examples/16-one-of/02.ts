// Example 2 of guide/16-one-of.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, ConstructionError, model, oneOf } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";

export default async function example(): Promise<void> {
  const Review = model("review", {
    columns: {
      id: column("i32", { primaryKey: true, default: true }),
      stars: column("i16"),
      cakeId: column("i32", { name: "cake_id", nullable: true }),
    },
  });

  assert.throws(
    () => oneOf("subject", Review, { cake: ["stars", Cake] }),
    (error: unknown) => error instanceof ConstructionError && /is not nullable/.test(error.message),
  );
}
