// Example 2 of guide/26-typescript.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, Decimal, type InsertOf } from "@necessary/pgorm";
import { Cake, Customer } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // @ts-expect-error: a customer needs an email
  const incomplete: InsertOf<typeof Customer> = { name: "Mo" };
  assert.equal(incomplete.name, "Mo");

  assert.throws(() => {
    // @ts-expect-error: a cake has no field `colour`
    Cake.col("colour");
  }, ConstructionError);

  assert.throws(
    () =>
      Cake.update({
        // @ts-expect-error: a price is a Decimal, never a fractional number, which could round it
        price: 3.2,
      }).where(Cake.key({ id: 2 })),
    ConstructionError,
  );

  assert.throws(
    // @ts-expect-error: NULL is tested with isNull(), never compared
    () => Customer.find().where(Customer.col("notes").eq(null)),
    ConstructionError,
  );

  const fine = Cake.update({ price: new Decimal("3.25") }).where(Cake.key({ id: 2 }));
  assert.equal(await fine.execute(pool), 1);
}
