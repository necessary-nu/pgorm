// Handles nothing references any more are collected: work still running on
// what they hold finishes, an idle transaction is rolled back and its
// connection returned to the pool, and an idle connection is returned. Run
// with the garbage collector exposed.

import process from "node:process";

import { Pool } from "../../lib/index.js";

const collect = (globalThis as { gc?: () => void }).gc;
if (collect === undefined) throw new Error("the garbage collector is not exposed");

async function churn(): Promise<void> {
  for (let round = 0; round < 10; round += 1) {
    collect?.();
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}

/** Churn the collector until `pool` has `available` idle connections. */
async function settles(pool: Pool, available: number): Promise<string> {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    await churn();
    const status = pool.status();
    if (status.available === available) return `size ${status.size} available ${status.available}`;
  }
  const status = pool.status();
  return `size ${status.size} available ${status.available}`;
}

const dsn = process.env.PGORM_TEST_DSN ?? "";

// A statement on a connection whose object nothing else holds, and a
// savepoint whose transaction and connection objects nothing else holds.
const working = new Pool(dsn, { maxSize: 2 });
const running = working.acquire().then((connection) => connection.one("SELECT pg_sleep(0.3)::text, 1 AS n"));
const finished = await (await working.acquire()).transaction((transaction) =>
  transaction.transaction(async (savepoint) => {
    await churn();
    const row = await running;
    await savepoint.execute("SELECT 1");
    return `finished ${row.n} ${savepoint.parent?.closed}`;
  })
);
console.log(finished);

// An idle connection whose object nothing holds goes back to its pool.
const idle = new Pool(dsn, { maxSize: 1 });
await idle.acquire();
console.log(`idle ${await settles(idle, 1)}`);

// An idle transaction whose callback never settles, and whose objects
// nothing holds, is rolled back, and its connection goes back to the pool
// rather than being discarded.
const abandoned = new Pool(dsn, { maxSize: 1 });
const applied = Promise.withResolvers<void>();
void (await abandoned.acquire()).transaction(async (transaction) => {
  await transaction.execute("SET LOCAL application_name = 'abandoned'");
  applied.resolve();
  await new Promise(() => {});
});
await applied.promise;
console.log(`abandoned ${await settles(abandoned, 1)}`);
const after = await abandoned.one("SELECT current_setting('application_name') AS name");
console.log(`rolled back ${after.name !== "abandoned"}`);
