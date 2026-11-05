// Example 1 of guide/14-schema.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ColumnDef, createTable, createType, DataType, Decimal, ForeignKeyViolationError } from "@necessary/pgorm";
import { Bakery, Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const flavors = Cake.columns.flavor;
  const createFlavor = createType(flavors.kind).values(flavors.values!);
  assert.equal(
    createFlavor.inspect().sql,
    `CREATE TYPE "public"."flavor" AS ENUM ('chocolate', 'vanilla', 'lemon', 'carrot')`,
  );

  const bakeries = createTable(Bakery.table)
    .column(new ColumnDef("id", "integer").notNull().autoIncrement())
    .column(new ColumnDef("name", "varchar").notNull())
    .column(new ColumnDef("city", "varchar").notNull())
    .primaryKey("id");

  const cakes = createTable(Cake.table)
    .column(new ColumnDef("id", "integer").notNull().autoIncrement())
    .column(new ColumnDef("name", "varchar").notNull())
    .column(new ColumnDef("flavor", flavors.kind).notNull())
    .column(new ColumnDef("price", new DataType("numeric", { precision: 10, scale: 2 })).notNull())
    .column(new ColumnDef("gluten_free", "boolean").notNull())
    .column(new ColumnDef("bakery_id", "integer").notNull())
    .primaryKey("id")
    .foreignKey("bakery_id", Bakery.table, "id", { name: "fk-cake-bakery_id" });
  assert.equal(
    cakes.inspect().sql,
    `CREATE TABLE "cake" ( "id" serial NOT NULL, "name" varchar NOT NULL, "flavor" public.flavor NOT NULL, "price" decimal(10, 2) NOT NULL, "gluten_free" bool NOT NULL, "bakery_id" integer NOT NULL, PRIMARY KEY ("id"), CONSTRAINT "fk-cake-bakery_id" FOREIGN KEY ("bakery_id") REFERENCES "bakery" ("id") )`,
  );

  for (const statement of [createFlavor, bakeries, cakes]) await pool.execute(statement);

  // The foreign key holds: a cake needs its bakery.
  await assert.rejects(
    Cake.insert({ name: "Nowhere Cake", flavor: "vanilla", price: new Decimal("1"), glutenFree: false, bakeryId: 99 })
      .execute(pool),
    ForeignKeyViolationError,
  );
}
