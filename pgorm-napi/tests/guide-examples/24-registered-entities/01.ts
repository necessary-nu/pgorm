// Example 1 of guide/24-registered-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { createSchema, entities, entity, fromEntity } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  assert.ok(entities().includes("app.Account"));
  const Account = entity("app.Account");
  const Note = entity("app.Note");

  await pool.execute(createSchema("napi_entities"));
  for (const registered of [Account, Note]) {
    const generated = fromEntity(registered);
    for (const statement of generated.enums) await pool.execute(statement);
    await pool.execute(generated.table);
    for (const statement of [...generated.indexes, ...generated.comments]) await pool.execute(statement);
  }

  assert.match(fromEntity(Account).table.inspect().sql, /^CREATE TABLE "napi_entities"\."accounts"/);
  const comment = await pool.one(
    "SELECT obj_description('napi_entities.accounts'::regclass, 'pg_class') AS comment",
  );
  assert.equal(comment.comment, "Accounts' native schema");
}
