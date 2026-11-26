// Example 16 of guide/03-entities.md, written by tests/guide.test.ts: edit the
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

  // @ts-expect-error: a scoped model's find() takes a context: findFor(cx)
  assert.throws(() => Note.find(), {
    message:
      "`note` is scoped by `CompanyId` and `UserId`, which a statement taking no context does not hold: a statement on `note` starts from a `For` form, given a context holding its scopes first: `findFor(cx)`, `insertFor(cx, values)`, `updateFor(cx, values)`",
  });
  // @ts-expect-error: the context holds no UserId
  assert.throws(() => Note.findFor(CompanyId(1n)), {
    message: "`note` is scoped by `CompanyId` and `UserId`, which `CompanyId` does not hold",
  });
}
