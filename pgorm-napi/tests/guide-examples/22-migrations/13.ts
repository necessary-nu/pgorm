// Example 13 of guide/22-migrations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { migrations, Pool } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ url }: GuideContext): Promise<void> {
  // One connection, so every statement runs on the same session.
  await using shop = new Pool(url, { maxSize: 1 });
  await shop.execute("CREATE DOMAIN ribbon AS text DEFAULT 'red'");
  await shop.execute("CREATE TABLE box (id serial PRIMARY KEY, contents text, ribbon ribbon)");
  const pack = "INSERT INTO box (contents) VALUES ('truffles') RETURNING ribbon";
  assert.equal((await shop.one(pack)).ribbon, "red");

  // The insert prepared before the change would still write the old default,
  // but the run clears the pool's caches as it commits.
  await migrations([
    {
      name: "m20260201_000001_gold_ribbon",
      async up(tx) {
        await tx.execute("ALTER DOMAIN ribbon SET DEFAULT 'gold'");
      },
    },
  ]).up(shop);
  assert.equal((await shop.one(pack)).ribbon, "gold");
}
