// Example 5 of guide/24-registered-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { createSchema, entity, type EntityRecord, fromEntity, graph } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await pool.execute(createSchema("napi_entities"));
  for (const registered of [entity("app.Account"), entity("app.Note")]) {
    for (const statement of fromEntity(registered).enums) await pool.execute(statement);
    await pool.execute(fromEntity(registered).table);
  }
  await pool.execute(`INSERT INTO napi_entities.accounts VALUES (1, 'Ann', NULL, 1, 'calm'), (2, 'Bob', NULL, 1, 'busy')`);
  await pool.execute(`INSERT INTO napi_entities.notes VALUES (10, 1, 'a1'), (11, 1, 'a2')`);

  const notes = graph("app.AccountNotes").find({ aliases: ["n"] });
  // Without generated declarations, a row is typed as any graph's might be.
  const rows = (await notes.orderBy(notes.col(0, "id").asc(), notes.col(1, "id").asc()).all(pool)) as [
    EntityRecord,
    EntityRecord | null,
  ][];
  assert.deepEqual(
    rows.map(([account, note]) => [account["display name"], note?.body ?? null]),
    [["Ann", "a1"], ["Ann", "a2"], ["Bob", null]],
  );

  const page = await notes.cursor("id").first(2).all(pool);
  assert.equal(page.length, 2);
}
