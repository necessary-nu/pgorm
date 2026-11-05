// Example 6 of guide/18-listen-notify.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { PgormError } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  let listener = await pool.listener();
  await listener.listen("kitchen");

  const ended = await pool.execute(
    `SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = current_database() AND query = 'LISTEN "kitchen"'`,
  );
  assert.equal(ended, 1);

  await assert.rejects(listener.next({ signal: AbortSignal.timeout(5000) }), PgormError);
  assert.equal((await listener.next()).done, true);

  // Nothing reconnects: open a new listener and listen again.
  listener = await pool.listener();
  await listener.listen("kitchen");
  await pool.notify("kitchen", "back");
  const heard = await listener.next({ signal: AbortSignal.timeout(5000) });
  assert.ok(!heard.done);
  assert.equal(heard.value.payload, "back");
  await listener.close();
}
