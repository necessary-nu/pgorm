// Example 3 of guide/24-registered-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { ConstructionError, createSchema, entity, fromEntity } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const Account = entity("app.Account");
  await pool.execute(createSchema("napi_entities"));
  for (const statement of fromEntity(Account).enums) await pool.execute(statement);
  await pool.execute(fromEntity(Account).table);

  const draft = Account.active().set("id", 1).set("display name", "Ann");
  // `new` set the version and the mood.
  assert.equal(draft.get("version").state, "set");

  const ann = await draft.insert(pool);
  assert.equal(ann["display name"], "Ann|before");
  assert.equal(ann.version, 1);

  const changed = await Account.intoActive(ann).set("note", "seen").update(pool);
  assert.equal(changed.version, 2);
  assert.equal(Account.intoActive(changed).get("note").state, "unchanged");

  await assert.rejects(
    Account.active().set("id", 2).set("display name", "reject_before").insert(pool),
    (error: unknown) => error instanceof ConstructionError && /before_save refused/.test(error.message),
  );
  assert.equal(await Account.intoActive(changed).delete(pool), 1);
}
