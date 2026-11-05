// Example 6 of guide/13-values.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, model } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await pool.execute("CREATE TABLE profile (baker_id integer PRIMARY KEY, contact jsonb NOT NULL, extra json NOT NULL)");
  const Profile = model("profile", {
    columns: {
      bakerId: column("i32", { name: "baker_id", primaryKey: true }),
      contact: column("json"),
      extra: column("json"),
    },
  });

  const ada = {
    bakerId: 1,
    contact: { phone: "+47 400 00 001", languages: ["nb", "en"] },
    extra: { apron: "L" },
  };
  await Profile.insert(ada).execute(pool);

  const english = await Profile.find()
    .where(Profile.col("contact").contains({ languages: ["en"] }))
    .all(pool);
  assert.deepEqual(english, [ada]);
}
