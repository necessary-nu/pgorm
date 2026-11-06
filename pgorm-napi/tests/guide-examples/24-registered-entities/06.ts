// Example 6 of guide/24-registered-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { createSchema, entity, fromEntity } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const Account = entity("app.Account");
  await pool.execute(createSchema("napi_entities"));
  for (const statement of fromEntity(Account).enums) await pool.execute(statement);
  await pool.execute(fromEntity(Account).table);

  const ann = Account.active().set("id", 1).set("display name", "Ann");
  assert.deepEqual(await Account.insert(ann).onConflictDoNothing().execute(pool), { kind: "inserted", value: 1 });
  // Ann's key is taken now, so the second insert writes nothing.
  assert.deepEqual(await Account.insert(ann).onConflictDoNothing().returningPk(pool), { kind: "conflicted" });

  const bob = await Account.insert(Account.active().set("id", 2).set("display name", "Bob"))
    .onEmptyDoNothing()
    .returningModel(pool);
  assert.equal(bob.kind === "inserted" && bob.value["display name"], "Bob");
  assert.deepEqual(await Account.insertMany([]).onConflictDoNothing().returningModels(pool), { kind: "empty" });
}
