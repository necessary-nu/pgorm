// Example 3 of guide/17-copy.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { copyFrom } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const copy = copyFrom("cake", "text").columns(["name", "flavor", "price", "gluten_free", "bakery_id"]);
  assert.equal(
    copy.inspect().sql,
    `COPY "cake" ("name", "flavor", "price", "gluten_free", "bakery_id") FROM STDIN WITH (FORMAT text)`,
  );

  const lines = ["Lemon Tart\tlemon\t4.10\tf\t1\n", "Carrot Muffin\tcarrot\t2.20\tt\t2\n"];
  const encoder = new TextEncoder();
  const written = await pool.copyIn(copy, lines.map((line) => encoder.encode(line)));
  assert.equal(written, 2);

  const cakes = await Cake.find().count(pool);
  assert.equal(cakes, 7);
}
