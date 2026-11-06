// Example 6 of guide/03-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, fromModel, model } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
  const Note = model("note", {
    renameAll: "snake_case",
    columns: {
      tenantId: column("i32", { primaryKey: true }),
      id: column("i64", { primaryKey: true, generated: "always" }),
      body: column("text", { columnType: "varchar" }),
    },
  });

  const create = fromModel(Note).table;
  assert.equal(
    create.inspect().sql,
    `CREATE TABLE "note" ( "tenant_id" integer NOT NULL, "id" bigint NOT NULL GENERATED ALWAYS AS IDENTITY, "body" varchar NOT NULL, CONSTRAINT "pk-note" PRIMARY KEY ("tenant_id", "id") )`,
  );
  await pool.execute(create);

  const note = await Note.insert({ tenantId: 7, body: "Order more flour" }).returning().one(pool);
  assert.deepEqual(note, { tenantId: 7, id: 1n, body: "Order more flour" });
}
