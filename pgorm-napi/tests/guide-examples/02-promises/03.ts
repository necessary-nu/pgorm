// Example 3 of guide/02-promises.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await assert.rejects(
    pool.execute("SELECT pg_sleep(10)", [], { signal: AbortSignal.timeout(100) }),
    (error: unknown) => error instanceof DOMException && error.name === "TimeoutError",
  );
}
