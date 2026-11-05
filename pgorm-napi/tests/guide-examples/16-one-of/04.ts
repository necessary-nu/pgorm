// Example 4 of guide/16-one-of.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, createOneOf, model, oneOf } from "@necessary/pgorm";
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
  await pool.execute(createOneOf(Subject));

  await Review.insertMany([
    { stars: 5, body: "Rich and dark", ...Subject.set("cake", 1) },
    { stars: 4, body: "Friendly staff", ...Subject.set("bakery", 2) },
    { stars: 3, body: "A little dry", ...Subject.set("cake", 3) },
  ]).execute(pool);

  const reviews = await Review.find().orderBy(Review.col("id").asc()).all(pool);
  const subjects = await Subject.load(pool, reviews);
  const named = subjects.map((subject) =>
    subject === null ? "nothing" : subject[0] === "cake" ? `cake ${subject[1].name}` : `bakery ${subject[1].name}`
  );
  assert.deepEqual(named, ["cake Chocolate Fudge", "bakery Moonlight", "cake Carrot Cake"]);

  // The third review was really about the bakery that sells the cake.
  const moved = await Review.update(Subject.set("bakery", 2))
    .where(Review.key({ id: reviews[2]!.id }))
    .returning()
    .one(pool);
  assert.deepEqual([moved.cakeId, moved.bakeryId], [null, 2]);
  assert.deepEqual(Subject.key(moved), ["bakery", 2]);
}
