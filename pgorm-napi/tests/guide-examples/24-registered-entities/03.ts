// Example 3 of guide/24-registered-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { createSchema, entity, fromEntity } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const Account = entity("app.Account");
  const Note = entity("app.Note");
  await pool.execute(createSchema("napi_entities"));
  for (const registered of [Account, Note]) {
    for (const statement of fromEntity(registered).enums) await pool.execute(statement);
    await pool.execute(fromEntity(registered).table);
  }
  await pool.execute(
    `INSERT INTO napi_entities.accounts VALUES (1, 'Ann', NULL, 1, 'calm'), (2, 'Bob', NULL, 1, 'busy'), (3, 'Cy', NULL, 1, 'calm')`,
  );
  await pool.execute(`INSERT INTO napi_entities.notes VALUES (10, 1, 'a1'), (11, 2, 'b1')`);

  const byId = Account.find().orderBy(Account.col("id").asc());
  assert.equal(await byId.count(pool), 3);
  const pages = byId.paginate(2);
  assert.deepEqual(await pages.numItemsAndPages(pool), { items: 3, pages: 2 });
  assert.deepEqual((await pages.fetchPage(pool, 1)).map((account) => account.id), [3]);

  const cursor = Account.find().cursor("mood", "id");
  assert.deepEqual((await cursor.first(2).all(pool)).map((account) => account.id), [1, 3]);
  assert.deepEqual((await cursor.after("calm", 3).all(pool)).map((account) => account.id), [2]);

  const streamed = [];
  for await (const account of byId.stream(pool)) streamed.push(account["display name"]);
  assert.deepEqual(streamed, ["Ann", "Bob", "Cy"]);

  const withB1 = Account.find().join(Account.relation("Note")).where(Note.col("body").eq("b1"));
  assert.deepEqual((await withB1.all(pool)).map((account) => account.id), [2]);
  assert.deepEqual(await byId.select("display name", "mood").all(pool), [
    { "display name": "Ann", mood: "calm" },
    { "display name": "Bob", mood: "busy" },
    { "display name": "Cy", mood: "calm" },
  ]);
}
