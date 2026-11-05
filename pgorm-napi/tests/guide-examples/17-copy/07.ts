// Example 7 of guide/17-copy.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { copyFrom } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const copy = copyFrom("cake", "csv")
    .columns(["name", "flavor", "price", "gluten_free", "bakery_id"])
    .header()
    .quote("'")
    .onErrorIgnore();
  assert.equal(
    copy.inspect().sql,
    `COPY "cake" ("name", "flavor", "price", "gluten_free", "bakery_id") FROM STDIN WITH (FORMAT csv, HEADER true, QUOTE E'\\'', ON_ERROR ignore)`,
  );

  const data = "name,flavor,price,gluten_free,bakery_id\n" +
    "'Vanilla Slice, iced',vanilla,3.40,false,2\n" +
    "Lemon Bun,lemon,ask,false,1\n";
  // The second line's price did not convert, so it was skipped.
  assert.equal(await pool.copyIn(copy, new TextEncoder().encode(data)), 1);

  const slice = await Cake.find().where(Cake.col("name").eq("Vanilla Slice, iced")).one(pool);
  assert.equal(slice.price.toString(), "3.40");
}
