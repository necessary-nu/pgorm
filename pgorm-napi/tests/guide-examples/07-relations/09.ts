// Example 9 of guide/07-relations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError } from "@necessary/pgorm";
import { Bakery, bakeryCakes } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const limited = Bakery.graph().joinMaybe(bakeryCakes).limit(2);
  // @ts-expect-error a limited graph has no allGrouped
  await assert.rejects(limited.allGrouped(pool), ConstructionError);
  // @ts-expect-error nor a count, which would ignore the limit
  await assert.rejects(limited.count(pool), ConstructionError);
}
