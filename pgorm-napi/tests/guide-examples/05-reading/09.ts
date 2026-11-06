// Example 9 of guide/05-reading.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { call, column, fromExpr, partialModel, skip } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // A line of the price list.
  const tags = await Cake.select("name")
    .columnAs(Cake.col("price"), "euros")
    .where(Cake.col("bakeryId").eq(2))
    .orderBy(Cake.col("name").asc())
    .intoModel()
    .all(pool);
  assert.equal(tags[0]!.name, "Carrot Cake");
  assert.equal(tags[0]!.euros.toString(), "3.80");

  /** A cake on the menu board, its name shouted. */
  const MenuLine = partialModel(Cake, {
    shout: fromExpr(call("upper", Cake.col("name")), column("text")),
    freeFromGluten: Cake.col("glutenFree"),
    featured: skip(false),
  });

  const lemon = Cake.find().where(Cake.col("id").eq(2)).intoPartialModel(MenuLine);
  assert.equal(
    lemon.inspect().sql,
    `SELECT UPPER("cake"."name") AS "shout", "cake"."gluten_free" AS "freeFromGluten" FROM "cake" WHERE "cake"."id" = $1`,
  );
  assert.deepEqual(await lemon.one(pool), { shout: "LEMON DRIZZLE", freeFromGluten: true, featured: false });
}
