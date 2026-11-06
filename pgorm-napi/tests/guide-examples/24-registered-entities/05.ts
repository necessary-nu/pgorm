// Example 5 of guide/24-registered-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { Conflict, createSchema, entity, fromEntity } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const Account = entity("app.Account");
  await pool.execute(createSchema("napi_entities"));
  for (const statement of fromEntity(Account).enums) await pool.execute(statement);
  await pool.execute(fromEntity(Account).table);

  const ids = await Account.insertMany([
    Account.active().set("id", 1).set("display name", "Ann"),
    Account.active().set("id", 2).set("display name", "Bob"),
  ]).returningPks(pool);
  assert.deepEqual(ids, [1, 2]);

  const raised = await Account.updateMany()
    .set("version", 5)
    .where(Account.col("id").gte(2))
    .returningChanges(pool);
  assert.deepEqual(raised.map(({ old, new: now }) => [old.version, now.version]), [[1, 5]]);

  const upserted = await Account.insert(Account.active().set("id", 1).set("display name", "Anne"))
    .onConflict(Conflict.on("id").update("display name"))
    .returningUpsert(pool);
  assert.equal(upserted?.kind, "updated");
  assert.equal(upserted?.new["display name"], "Anne");
}
