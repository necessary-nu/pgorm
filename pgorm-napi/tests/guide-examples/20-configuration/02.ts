// Example 2 of guide/20-configuration.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, Pool, TimeoutError } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ url }: GuideContext): Promise<void> {
  await using small = new Pool(url, { maxSize: 2, acquireTimeout: 200, connectTimeout: 5000 });

  const first = await small.acquire();
  const second = await small.acquire();
  const status = small.status();
  assert.deepEqual(
    [status.maxSize, status.size, status.available, status.waiting],
    [2, 2, 0, 0],
  );

  // Both connections are lent, so a third waits, then gives up.
  await assert.rejects(small.acquire(), TimeoutError);
  await first.close();
  await second.close();
  assert.equal(small.status().available, 2);

  // waitTimeout gives up sooner, and says which bound it was.
  await using strict = new Pool(url, { maxSize: 1, waitTimeout: 50 });
  const only = await strict.acquire();
  await assert.rejects(strict.acquire(), (error: unknown) => error instanceof TimeoutError && /waitTimeout/.test(error.message));
  await only.close();

  // A pool that cannot be built is refused as it is made.
  assert.throws(() => new Pool(url, { maxSize: 0 }), ConstructionError);
  assert.throws(() => new Pool(url, { maxSize: "2" as unknown as number }), TypeError);
}
