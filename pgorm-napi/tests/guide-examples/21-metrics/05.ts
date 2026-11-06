// Example 5 of guide/21-metrics.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import process from "node:process";
import { InstrumentedPool, NoOpMetrics, type QueryContext } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  class Exporter extends NoOpMetrics {
    readonly sent: string[] = [];

    override async recordQuerySuccess(query: QueryContext) {
      await new Promise((resolve) => setTimeout(resolve, 10)); // a send to a metrics backend
      this.sent.push(query.operation);
    }

    override recordTransactionCommit(): void {
      throw new Error("the backend is down");
    }
  }

  const warnings: Error[] = [];
  const listener = (warning: Error) => warnings.push(warning);
  process.on("warning", listener);

  const exporter = new Exporter();
  const instrumented = new InstrumentedPool(pool, exporter);

  // The promise has settled by the time the query does.
  assert.deepEqual(await instrumented.one("SELECT 1 AS one"), { one: 1 });
  assert.deepEqual(exporter.sent, ["queryAll"]);

  // The failing hook does not undo the commit.
  await instrumented.transaction((tx) => tx.execute("UPDATE cake SET price = price WHERE id = 1"));
  await new Promise((resolve) => setTimeout(resolve, 10));
  process.off("warning", listener);
  assert.equal(warnings.length, 1);
  assert.equal(warnings[0]?.name, "MetricsWarning");
  assert.equal((warnings[0]?.cause as Error).message, "the backend is down");
}
