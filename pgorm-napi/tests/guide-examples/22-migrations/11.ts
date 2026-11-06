// Example 11 of guide/22-migrations.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { alterType, migrationFile } from "@necessary/pgorm";
import { flavor } from "../../../guide/bakery.ts";

export default async function example(): Promise<void> {
  const almond = migrationFile("m20260401_000001_add_almond", [alterType(flavor).addValue("almond")], {
    commitsAfter: true,
  });
  assert.equal(almond.commitsAfter, true);
  assert.equal(almond.text, `-- pgorm: commits-after\nALTER TYPE "public"."flavor" ADD VALUE 'almond';\n`);
}
