// Example 3 of guide/20-configuration.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConnectionError, type HookConnection, Pool } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ url }: GuideContext): Promise<void> {
  /** A hook refusing a server older than `minimum`, a `server_version_num` such as 180000. */
  function requiring(minimum: number) {
    return async (db: HookConnection) => {
      const { version } = await db.one("SELECT current_setting('server_version_num')::int AS version");
      if ((version as number) < minimum) throw new Error(`server version ${version} is older than ${minimum}`);
    };
  }

  await using checked = new Pool(url, { postCreate: requiring(180000) });
  assert.equal((await checked.one("SELECT 1 AS one")).one, 1);

  await using refusing = new Pool(url, { postCreate: requiring(990000) });
  await assert.rejects(refusing.acquire(), (error: unknown) =>
    error instanceof ConnectionError && error.message.startsWith("the postCreate hook refused the connection"));
}
