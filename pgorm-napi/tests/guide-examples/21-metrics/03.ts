// Example 3 of guide/21-metrics.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const timings = new Map<string, number[]>();

  /** Run `work`, recording how long it took under `label`, whether it succeeded or not. */
  async function timed<T>(label: string, work: () => Promise<T>): Promise<T> {
    const started = performance.now();
    try {
      return await work();
    } finally {
      const taken = timings.get(label) ?? [];
      taken.push(performance.now() - started);
      timings.set(label, taken);
    }
  }

  const cakes = await timed("cakes", () => Cake.find().all(pool));
  await timed("cakes", () => Cake.findByKey({ id: 1 }).one(pool));
  await assert.rejects(timed("missing table", () => pool.query("SELECT * FROM no_such_table")));

  assert.equal(cakes.length, 5);
  assert.equal(timings.get("cakes")?.length, 2);
  assert.equal(timings.get("missing table")?.length, 1);
}
