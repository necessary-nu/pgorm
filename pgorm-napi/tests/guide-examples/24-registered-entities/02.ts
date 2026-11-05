// Example 2 of guide/24-registered-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { createSchema, entity, fromEntity } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const Account = entity("app.Account");
  await pool.execute(createSchema("napi_entities"));
  for (const statement of fromEntity(Account).enums) await pool.execute(statement);
  await pool.execute(fromEntity(Account).table);
  await pool.execute(
    `INSERT INTO napi_entities.accounts VALUES (1, 'Ann', NULL, 1, 'calm'), (2, 'Bob', 'b', 1, 'busy'), (3, 'Cy', NULL, 1, 'calm')`,
  );

  const calm = Account.find().where(Account.col("mood").eq("calm")).orderBy(Account.col("id").desc());
  assert.match(calm.inspect().sql, /WHERE "accounts"\."mood" = CAST\(\$1::text AS napi_entities\.mood\)/);

  const accounts = await calm.all(pool);
  assert.deepEqual(accounts.map((account) => account["display name"]), ["Cy", "Ann"]);
  const first = await calm.optional(pool);
  assert.equal(first?.id, 3);
}
