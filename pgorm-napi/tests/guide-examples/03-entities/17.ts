// Example 17 of guide/03-entities.md, written by tests/guide.test.ts: edit the
// chapter, then run the test with PGORM_GUIDE_WRITE=1 to write this again.

import assert from "node:assert/strict";
import { column, DecodeError, every, fromModel, model, scope } from "@necessary/pgorm";
import type { GuideContext } from "../../../guide/context.ts";

export default async function example({ pool }: GuideContext): Promise<void> {
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
  const companyNotes = Company.hasMany(Note, { from: "id", to: "companyId" });

  for (const declared of [Company, Note]) await pool.execute(fromModel(declared).table);
  for (const [id, name] of [[1n, "Acme"], [2n, "Globex"]] as const) {
    await Company.insertFor(CompanyId(id), { name }).execute(pool);
  }
  for (const [company, user, body] of [[1n, 10n, "Ada's plan"], [1n, 11n, "Bo's draft"], [2n, 20n, "Cy's memo"]] as const) {
    await Note.insertFor({ company: CompanyId(company), user: UserId(user) }, { body }).execute(pool);
  }

  const ada = { company: CompanyId(1n), user: UserId(10n) };
  const admin = { company: CompanyId(1n), users: every(UserId) };
  const hers = await Note.findFor(ada).all(pool);
  assert.deepEqual(hers.map((note) => note.body), ["Ada's plan"]);
  const theirs = await Note.findFor(admin).orderBy(Note.col("id").asc()).all(pool);
  assert.equal(theirs.length, 2, "Ada's and Bo's notes, and not Globex's");

  // Cy's note belongs to Globex, so Ada finds no note of its key.
  await assert.rejects(Note.findByKeyFor(ada, { id: 3n }).one(pool), DecodeError);

  // A graph keeps its context, and each slot's scopes go in its join's ON.
  const acme = await Company.graphFor(ada).joinMaybe(companyNotes).all(pool);
  assert.deepEqual(acme.map(([company, note]) => [company.name, note?.authorId]), [["Acme", 10n]]);

  assert.equal((await Company.findFor(ada).joinFor(ada, companyNotes).all(pool)).length, 1);
  // @ts-expect-error: a scoped model joins by joinFor
  assert.throws(() => Company.findFor(ada).join(companyNotes), /join it with a form taking the statement's context, such as `joinFor`/);
}
