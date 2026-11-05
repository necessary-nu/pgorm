// Example 8 of guide/03-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, Decimal } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  assert.equal(Cake.columns.flavor.kind.toString(), `"public"."flavor"`);
  assert.deepEqual(Cake.columns.flavor.values, ["chocolate", "vanilla", "lemon", "carrot"]);

  const carrot = await Cake.find().where(Cake.col("flavor").eq("carrot")).one(pool);
  assert.equal(carrot.name, "Carrot Cake");

  // A label the declaration does not list is refused before anything is sent.
  assert.throws(
    () =>
      Cake.insert({
        name: "Banana Bread",
        // @ts-expect-error: "banana" is not one of the flavor's labels
        flavor: "banana",
        price: new Decimal("3.00"),
        glutenFree: false,
        bakeryId: 1,
      }),
    ConstructionError,
  );
}
