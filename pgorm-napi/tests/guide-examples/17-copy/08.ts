// Example 8 of guide/17-copy.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { copyFrom, copyTo } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const sunrise = copyTo(Cake.find().where(Cake.col("bakeryId").eq(1)).statement, "binary");
  const cakes = [];
  for await (const cake of (await pool.copyOutRows(sunrise)).models(Cake)) cakes.push(cake);
  assert.equal(cakes.length, 3);

  const into = copyFrom("cake", "binary").columns(["name", "flavor", "price", "gluten_free", "bakery_id"]);
  // The binary COPY TO read on a connection of its own, and the COPY FROM writes on another.
  const sink = await pool.copyInRows(into);
  assert.deepEqual(sink.columns.map((column) => column.name), ["name", "flavor", "price", "gluten_free", "bakery_id"]);
  for (const cake of cakes) await sink.sendModel({ ...cake, bakeryId: 2 }, Cake);
  assert.equal(await sink.finish(), 3);

  assert.equal(await Cake.find().where(Cake.col("bakeryId").eq(2)).count(pool), 5);
}
