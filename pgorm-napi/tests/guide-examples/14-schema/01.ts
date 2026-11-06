// Example 1 of guide/14-schema.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Decimal, ForeignKeyViolationError, fromModel } from "@necessary/pgorm";
import { Bakery, Cake, cakeBakery } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const cake = fromModel(Cake, { relations: [cakeBakery] });
  assert.deepEqual(cake.enums.map((statement) => statement.inspect().sql), [
    `CREATE TYPE "public"."flavor" AS ENUM ('chocolate', 'vanilla', 'lemon', 'carrot')`,
  ]);
  assert.equal(
    cake.table.inspect().sql,
    `CREATE TABLE "cake" ( "id" serial NOT NULL, "name" varchar NOT NULL, "flavor" public.flavor NOT NULL, "price" decimal(10, 2) NOT NULL, "gluten_free" bool NOT NULL, "bakery_id" integer NOT NULL, PRIMARY KEY ("id"), CONSTRAINT "fk-cake-bakery_id" FOREIGN KEY ("bakery_id") REFERENCES "bakery" ("id") )`,
  );

  for (const statement of [...cake.enums, fromModel(Bakery).table, cake.table]) await pool.execute(statement);

  // The foreign key holds: a cake needs its bakery.
  await assert.rejects(
    Cake.insert({ name: "Nowhere Cake", flavor: "vanilla", price: new Decimal("1"), glutenFree: false, bakeryId: 99 })
      .execute(pool),
    ForeignKeyViolationError,
  );
}
