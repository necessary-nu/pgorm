// Example 15 of guide/03-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, every, model, scope } from "@necessary/pgorm";

export default async function example(): Promise<void> {
  const CompanyId = scope("CompanyId");
  const UserId = scope("UserId");

  const Company = model("company", {
    columns: { id: column("i64", { primaryKey: true, scope: CompanyId }), name: column("text") },
  });
  const Note = model("note", {
    columns: {
      id: column("i64", { primaryKey: true, generated: "byDefault" }),
      companyId: column("i64", { name: "company_id", scope: CompanyId }),
      authorId: column("i64", { name: "author_id", scope: UserId }),
      body: column("text"),
    },
  });

  const ada = { company: CompanyId(1n), user: UserId(10n) };
  const admin = { company: CompanyId(1n), users: every(UserId) };
  assert.equal(
    Note.findFor(ada).inspect().sql,
    `SELECT "note"."id", "note"."company_id", "note"."author_id", "note"."body" FROM "note" WHERE "note"."company_id" = $1 AND "note"."author_id" = $2`,
  );
  assert.equal(
    Note.findFor(admin).inspect().sql,
    `SELECT "note"."id", "note"."company_id", "note"."author_id", "note"."body" FROM "note" WHERE "note"."company_id" = $1`,
  );
  // A scope's value alone is a context, and the company's key is its scope.
  assert.equal(
    Company.findFor(CompanyId(1n)).inspect().sql,
    `SELECT "company"."id", "company"."name" FROM "company" WHERE "company"."id" = $1`,
  );
}
