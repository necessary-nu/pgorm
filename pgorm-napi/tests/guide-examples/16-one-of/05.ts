// Example 5 of guide/16-one-of.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, DecodeError, model, oneOf } from "@necessary/pgorm";
import { Bakery, Cake } from "../../../guide/bakery.ts";

export default async function example(): Promise<void> {
  const Review = model("review", {
    columns: {
      id: column("i32", { primaryKey: true, default: true }),
      stars: column("i16"),
      body: column("text"),
      cakeId: column("i32", { name: "cake_id", nullable: true }),
      bakeryId: column("i32", { name: "bakery_id", nullable: true }),
    },
  });
  const Subject = oneOf("subject", Review, { cake: ["cakeId", Cake], bakery: ["bakeryId", Bakery] });

  const both = { id: 9, stars: 1, body: "Confused", cakeId: 1, bakeryId: 1 };
  assert.throws(
    () => Subject.key(both),
    (error: unknown) => error instanceof DecodeError && /references more than one target/.test(error.message),
  );
}
