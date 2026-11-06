// Example 2 of guide/22-migrations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, type MigrationTrait, migrations } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const noop = (name: string): MigrationTrait => ({ name, async up() {} });

  await assert.rejects(
    migrations([noop("m20260201_000001_noop"), noop("m20260201_000001_noop")]).up(pool),
    (error) => error instanceof ConstructionError && /Duplicate migration name\(s\): 'm20260201_000001_noop'/.test(error.message),
  );

  await migrations([noop("m20260201_000001_noop")]).up(pool);
  await assert.rejects(
    migrations([noop("m20260201_000001_renamed")]).getMigrationWithStatus(pool),
    (error) => error instanceof ConstructionError && /'m20260201_000001_noop' is missing/.test(error.message),
  );
}
