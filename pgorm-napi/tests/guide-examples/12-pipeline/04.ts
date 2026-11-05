// Example 4 of guide/12-pipeline.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { pipeline as pl } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const glutenFree = pl.from(Cake)
    .filter(pl.col(Cake, "glutenFree").eq(pl.literal(true)))
    .sort(pl.col(Cake, "price"));

  const cakes = await glutenFree.intoModel(Cake).all(pool);
  assert.deepEqual(cakes.map((cake) => [cake.name, cake.glutenFree]), [
    ["Vanilla Sponge", true],
    ["Lemon Drizzle", true],
    ["Chocolate Torte", true],
  ]);

  const priced = glutenFree.select(pl.col(Cake, "name"), pl.col(Cake, "price"));
  const label = await priced.intoModel((row) => `${row.name} at ${row.price}`).one(pool);
  assert.equal(label, "Vanilla Sponge at 2.90");
  const pairs = await priced.intoTuple().all(pool);
  assert.deepEqual(pairs.map(([name, price]) => [name, String(price)]), [
    ["Vanilla Sponge", "2.90"],
    ["Lemon Drizzle", "3.20"],
    ["Chocolate Torte", "6.00"],
  ]);
}
