// Example 14 of guide/22-migrations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { migrations, STATEMENT_CACHE_CHANNEL } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await using listener = await pool.listener();
  await listener.listen(STATEMENT_CACHE_CHANNEL);

  await migrations([
    {
      name: "m20260201_000001_create_ribbon",
      async up(tx) {
        await tx.execute("CREATE DOMAIN ribbon AS text DEFAULT 'red'");
      },
    },
  ]).up(pool);

  // Every pool listening on the channel, in any process, hears it at the commit.
  const heard = await listener.next({ signal: AbortSignal.timeout(5000) });
  assert.ok(!heard.done);
  assert.equal(heard.value.channel, "pgorm_statement_caches");
}
