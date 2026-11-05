// Example 5 of guide/03-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, model } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  await pool.execute(
    `CREATE TABLE note ( tenant_id integer NOT NULL, id bigint NOT NULL GENERATED ALWAYS AS IDENTITY, body varchar NOT NULL, PRIMARY KEY (tenant_id, id) )`,
  );

  const Note = model("note", {
    columns: {
      tenantId: column("i32", { name: "tenant_id", primaryKey: true }),
      id: column("i64", { primaryKey: true, generated: "always" }),
      body: column("text"),
    },
  });

  const note = await Note.insert({ tenantId: 7, body: "Order more flour" }).returning().one(pool);
  assert.deepEqual(note, { tenantId: 7, id: 1n, body: "Order more flour" });
}
