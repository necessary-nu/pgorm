// Example 6 of guide/20-configuration.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Pool } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ url }: GuideContext): Promise<void> {
  await using single = new Pool(url, { maxSize: 1 });
  const db = await single.acquire();
  const { pid } = await db.one("SELECT pg_backend_pid() AS pid");
  await db.execute("SELECT set_config('application_name', 'half-finished', false)");
  await db.discard();

  const fresh = await single.one("SELECT pg_backend_pid() AS pid, current_setting('application_name') AS name");
  assert.notEqual(fresh.pid, pid);
  assert.notEqual(fresh.name, "half-finished");
}
