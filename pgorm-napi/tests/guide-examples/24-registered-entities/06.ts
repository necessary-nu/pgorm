// Example 6 of guide/24-registered-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { createSchema, entity, fromEntity, pipeline as pl } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const Account = entity("app.Account");
  const Note = entity("app.Note");
  await pool.execute(createSchema("napi_entities"));
  for (const registered of [Account, Note]) {
    for (const statement of fromEntity(registered).enums) await pool.execute(statement);
    await pool.execute(fromEntity(registered).table);
  }
  await pool.execute(`INSERT INTO napi_entities.accounts VALUES (1, 'Ann', NULL, 1, 'calm'), (2, 'Bob', NULL, 1, 'busy')`);
  await pool.execute(`INSERT INTO napi_entities.notes VALUES (10, 1, 'a1')`);

  const rows = await pl.from(Account)
    .join(pl.source(Note).named("n"), pl.col("accounts", "id").eq(pl.col("n", "account_id")), { kind: "left" })
    .sort(pl.col("accounts", Account.col("id")))
    .selectSources(pl.sources("app.AccountWithNote"), { qualifiers: ["accounts", "n"] })
    .all(pool);
  assert.deepEqual(
    rows.map(([account, note]) => [account?.["display name"], note?.body ?? null]),
    [["Ann", "a1"], ["Bob", null]],
  );
}
