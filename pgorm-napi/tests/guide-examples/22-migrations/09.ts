// Example 9 of guide/22-migrations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, type MigrationTrait, migrations } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const createDelivery = (revision: string): MigrationTrait => ({
    name: "m20260201_000001_create_delivery",
    checksum: revision,
    async up(tx) {
      await tx.execute("CREATE TABLE delivery (id serial PRIMARY KEY)");
    },
  });

  await migrations([createDelivery("1")]).up(pool);

  // The same migration, edited after it ran.
  await assert.rejects(
    migrations([createDelivery("2")]).up(pool),
    (error) =>
      error instanceof ConstructionError &&
      error.message.includes("'m20260201_000001_create_delivery' was applied with checksum '1'"),
  );
}
