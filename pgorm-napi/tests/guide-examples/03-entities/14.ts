// Example 14 of guide/03-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, model, scope } from "@necessary/pgorm";

export default async function example(): Promise<void> {
  const CompanyId = scope("CompanyId");
  const UserId = scope("UserId");

  const Note = model("note", {
    columns: {
      id: column("i64", { primaryKey: true, generated: "byDefault" }),
      companyId: column("i64", { name: "company_id", scope: CompanyId }),
      authorId: column("i64", { name: "author_id", scope: UserId }),
      body: column("text"),
    },
  });

  assert.deepEqual(Note.scopes, ["CompanyId", "UserId"]);
  // A scope column is never NULL: a row is in one value of its scope.
  assert.throws(() => column("i64", { scope: UserId, nullable: true }), /never NULL/);
}
