// Example 3 of guide/21-metrics.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { InstrumentedPool, LoggingMetrics } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const lines: string[] = [];
  const logger = {
    debug: (message: string) => lines.push(message),
    warn: (message: string) => lines.push(message),
    error: (message: string) => lines.push(message),
  };
  const instrumented = new InstrumentedPool(pool, new LoggingMetrics(logger));

  // Reports the acquisition, then the query.
  const cakes = await Cake.find().all(instrumented);
  assert.equal(cakes.length, 5);
  assert.match(lines[0] ?? "", /^Connection acquired in /);
  assert.match(lines[1] ?? "", /^Query queryAll succeeded in /);

  // It is the same pool, sharing its connections.
  assert.deepEqual(instrumented.status(), pool.status());
  assert.equal(instrumented.inner, pool);
}
