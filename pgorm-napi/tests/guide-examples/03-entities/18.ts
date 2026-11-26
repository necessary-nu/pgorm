// Example 18 of guide/03-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, DecodeError, every, fromModel, model, scope } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
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

  await pool.execute(fromModel(Note).table);
  const ada = { company: CompanyId(1n), user: UserId(10n) };
  const bo = { company: CompanyId(1n), user: UserId(11n) };
  const admin = { company: CompanyId(1n), users: every(UserId) };
  await Note.insertFor(bo, { body: "Bo's draft" }).execute(pool);

  const written = await Note.insertFor(ada, { body: "Ada's plan" }).returning().one(pool);
  assert.deepEqual([written.companyId, written.authorId], [1n, 10n]);
  assert.throws(
    () => Note.insertFor(ada, { companyId: 2n, body: "for Globex" }),
    /the row sets `note`.`company_id` to a value outside the context's `CompanyId`/,
  );
  assert.throws(
    () => Note.insertFor(admin, { body: "whose?" }),
    /the context holds every `UserId`, so the row sets `note`.`author_id` itself/,
  );

  // Bo's note is the company's, so the administrator edits it and Ada cannot.
  const edit = Note.active({ id: 1n, body: "Bo's draft, reviewed" });
  await assert.rejects(edit.updateFor(ada, pool), DecodeError);
  assert.equal((await edit.updateFor(admin, pool)).body, "Bo's draft, reviewed");

  // Handing Bo's note to Ada would move it out of Bo's scope.
  assert.throws(
    // @ts-expect-error: a scope field is never updated
    () => Note.updateFor(admin, { authorId: 10n }),
    /`note`.`author_id` is written when its row is inserted and never changed/,
  );
  assert.equal(await Note.deleteFor(ada).where(Note.col("id").eq(1n)).execute(pool), 0);
}
