// Example 5 of guide/04-writing.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, Decimal } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";

export default async function example(): Promise<void> {
  const price = new Decimal("1.00");
  assert.throws(
    () =>
      Cake.insertMany([
        { name: "Plain Bun", flavor: "vanilla", price, glutenFree: false, bakeryId: 1 },
        { id: 50, name: "Iced Bun", flavor: "vanilla", price, glutenFree: false, bakeryId: 1 },
      ]).inspect(),
    ConstructionError,
  );
}
