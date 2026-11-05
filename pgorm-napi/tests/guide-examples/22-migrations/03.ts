// Example 3 of guide/22-migrations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, model } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  // What pgorm-migration's runner leaves behind, written here by hand.
  await pool.execute(
    `CREATE TABLE "pgorm_migrations" ( "version" varchar NOT NULL, "applied_at" bigint NOT NULL, "checksum" varchar, PRIMARY KEY ("version") )`,
  );
  await pool.execute(
    `INSERT INTO "pgorm_migrations" VALUES ('m20260101_000001_create_bakery', 1767225600, NULL)`,
  );

  const Ledger = model("pgorm_migrations", {
    columns: {
      version: column("text", { primaryKey: true }),
      appliedAt: column("i64", { name: "applied_at" }),
      checksum: column("text", { nullable: true }),
    },
  });

  const needed = "m20260101_000001_create_bakery";
  const applied = await Ledger.findByKey({ version: needed }).optional(pool);
  assert.equal(applied?.appliedAt, 1767225600n);
  assert.equal(applied?.checksum, null);
}
