// Example 9 of guide/05-reading.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { call, select } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // A line of the price list.
  const tags = await pool.query(
    select(Cake.col("name"), Cake.col("price").as("euros"))
      .from(Cake.table)
      .where(Cake.col("bakeryId").eq(2))
      .orderBy(Cake.col("name").asc()),
  );
  assert.equal(tags[0]!.name, "Carrot Cake");
  assert.equal(String(tags[0]!.euros), "3.80");

  // A cake on the menu board, its name shouted.
  const shout = select(call("upper", Cake.col("name")).as("shout"), Cake.col("glutenFree").as("free_from_gluten"))
    .from(Cake.table)
    .where(Cake.col("id").eq(2));
  assert.equal(
    shout.inspect().sql,
    `SELECT UPPER("cake"."name") AS "shout", "cake"."gluten_free" AS "free_from_gluten" FROM "cake" WHERE "cake"."id" = $1`,
  );
  assert.deepEqual(await pool.one(shout), { shout: "LEMON DRIZZLE", free_from_gluten: true });
}
