// Example 1 of guide/20-configuration.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Pool } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ url }: GuideContext): Promise<void> {
  const settings = "-c search_path=kitchen,public -c statement_timeout=5s";
  const configured = new URL(url);
  configured.search += `&application_name=bakery-web&target_session_attrs=read-write&options=${encodeURIComponent(settings)}`;

  await using web = new Pool(configured.toString());
  const seen = await web.one(
    "SELECT current_setting('application_name') AS name, current_setting('search_path') AS path, " +
      "current_setting('statement_timeout') AS timeout",
  );
  assert.deepEqual(seen, { name: "bakery-web", path: "kitchen,public", timeout: "5s" });
}
