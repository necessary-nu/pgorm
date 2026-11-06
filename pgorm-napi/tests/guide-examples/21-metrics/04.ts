// Example 4 of guide/21-metrics.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { DatabaseError, InstrumentedPool, NoOpMetrics, type QueryContext } from "@necessary/pgorm";
import { Cake } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  class Counts extends NoOpMetrics {
    readonly counts = new Map<string, number>();

    bump(key: string) {
      this.counts.set(key, (this.counts.get(key) ?? 0) + 1);
    }

    override recordQuerySuccess(query: QueryContext) {
      this.bump(query.operation);
    }
    override recordQueryError(query: QueryContext) {
      this.bump(`${query.operation} failed`);
    }
    override recordConnectionAcquired() {
      this.bump("connection");
    }
    override recordTransactionBegin() {
      this.bump("begin");
    }
    override recordTransactionCommit() {
      this.bump("commit");
    }
    override recordTransactionRollback() {
      this.bump("rollback");
    }
  }

  const counts = new Counts();
  const instrumented = new InstrumentedPool(pool, counts);
  await instrumented.connection(async (db) => {
    await Cake.find().all(db);
    await Cake.findByKey({ id: 1 }).one(db);
    await assert.rejects(db.one("SELECT * FROM no_such_table"), DatabaseError);

    // One transaction commits, with a savepoint inside it; one rolls back.
    await db.transaction(async (tx) => {
      await Cake.find().count(tx);
      await tx.transaction((savepoint) => Cake.find().count(savepoint));
    });
    await assert.rejects(db.transaction(async () => {
      throw new Error("changed my mind");
    }));

    // A transaction opened in a mode is reported as the others are.
    await db.transaction((tx) => Cake.find().count(tx), { mode: "readOnly" });
  });

  assert.deepEqual(Object.fromEntries(counts.counts), {
    connection: 1,
    queryAll: 5,
    "queryAll failed": 1,
    begin: 4,
    commit: 3,
    rollback: 1,
  });
}
