// Example 6 of guide/04-writing.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Decimal } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const fudge = await Cake.findByKey({ id: 1 }).one(pool);

  const cheaper = Cake.update({ price: new Decimal("3.99") }).where(Cake.key({ id: fudge.id }));
  assert.equal(cheaper.inspect().sql, `UPDATE "cake" SET "price" = $1 WHERE "cake"."id" = $2`);

  const updated = await cheaper.returning().one(pool);
  assert.equal(updated.price.toString(), "3.99");
  assert.equal(updated.name, "Chocolate Fudge");
}
