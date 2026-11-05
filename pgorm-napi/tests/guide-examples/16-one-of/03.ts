// Example 3 of guide/16-one-of.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { CheckViolationError, column, createOneOf, model, oneOf } from "@necessary/pgorm";
import { Bakery, Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
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

  await pool.execute(
    `CREATE TABLE review (id serial PRIMARY KEY, stars smallint NOT NULL, body varchar NOT NULL,
       cake_id integer REFERENCES cake (id), bakery_id integer REFERENCES bakery (id))`,
  );

  const check = createOneOf(Subject);
  assert.equal(
    check.inspect().sql,
    `ALTER TABLE "review" ADD CONSTRAINT "ck-review-subject" CHECK (num_nonnulls("cake_id", "bakery_id") = 1)`,
  );
  await pool.execute(check);

  // A review of a cake and a bakery at once is refused.
  await assert.rejects(
    Review.insert({ stars: 3, body: "Both?", cakeId: 1, bakeryId: 1 }).execute(pool),
    (error: unknown) => error instanceof CheckViolationError && error.constraint === "ck-review-subject",
  );
}
