// Example 2 of guide/22-migrations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { alterType } from "@necessary/pgorm";
import { flavor } from "../../../guide/bakery.ts";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await pool.transaction(async (tx) => {
    await tx.execute(alterType(flavor).addValue("almond"));
  });
  await pool.transaction(async (tx) => {
    await tx.execute(
      "INSERT INTO cake (name, flavor, price, gluten_free, bakery_id) VALUES ('Almond Cake', 'almond', 3.60, true, 2)",
    );
  });

  const { flavor: added } = await pool.one("SELECT flavor::text AS flavor FROM cake WHERE name = 'Almond Cake'");
  assert.equal(added, "almond");
}
