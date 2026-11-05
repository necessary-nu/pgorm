// Example 1 of guide/26-typescript.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { type InsertOf, type KeyOf, type RowOf } from "@necessary/pgorm";
import { Cake, CakeBaker, Customer } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  type CakeRecord = RowOf<typeof Cake>;

  const lemon: CakeRecord = await Cake.findByKey({ id: 2 }).one(pool);
  const flavor: "chocolate" | "vanilla" | "lemon" | "carrot" = lemon.flavor;
  assert.equal(flavor, "lemon");

  // An id and the notes may be left out: the table fills one, and the other is nullable.
  const mo: InsertOf<typeof Customer> = { name: "Mo", email: "mo@example.com" };
  const saved = await Customer.insert(mo).returning().one(pool);
  const notes: string | null = saved.notes;
  assert.equal(notes, null);

  const key: KeyOf<typeof CakeBaker> = { cakeId: 2, bakerId: 2 };
  assert.ok(await CakeBaker.findByKey(key).optional(pool));
}
