// A registered scoped Rust entity stays scoped from JavaScript: every
// statement on it starts from a `For` form given a context holding its
// scopes, which Rust's own `_for` forms hold it to, and every statement built
// without one is refused before anything is sent.

import assert from "node:assert/strict";
import { after, before, beforeEach, test } from "node:test";

import {
  ConstructionError,
  DecodeError,
  entity,
  every,
  fromEntity,
  pipeline as pl,
  Pool,
  scope,
} from "../../../lib/index.js";
import { scratchDatabase } from "../../support.ts";

const CompanyId = scope("CompanyId");
const UserId = scope("UserId");
const Company = entity("app.ScopedCompany");
const Note = entity("app.ScopedNote");
const ada = { company: CompanyId(1n), user: UserId(10n) };
const admin = { company: CompanyId(1n), users: every(UserId) };
const NO_CONTEXT = "`app.ScopedNote` is scoped by `CompanyId` and `UserId`, which a statement taking no context does not hold";

let database: { dsn: string; drop(): Promise<void> } | undefined;
let shared: Pool | undefined;

function pool(): Pool {
  if (!shared) throw new Error("the scratch database was not created");
  return shared;
}

before(async () => {
  database = await scratchDatabase("pgorm_napi_registered_scopes");
  shared = new Pool(database.dsn, { maxSize: 2 });
  await pool().execute("CREATE SCHEMA napi_scopes");
  for (const registered of [Company, Note]) await pool().execute(fromEntity(registered).table);
});

beforeEach(async () => {
  await pool().execute("TRUNCATE napi_scopes.note, napi_scopes.company");
  for (const [id, name] of [[1n, "Acme"], [2n, "Globex"]] as const) {
    await Company.insertFor(CompanyId(id), Company.active().set("name", name)).execute(pool());
  }
  for (const [id, company, author, body] of [[1n, 1n, 10n, "Ada's plan"], [2n, 1n, 11n, "Bo's draft"], [3n, 2n, 20n, "Cy's memo"]] as const) {
    const writer = { company: CompanyId(company), user: UserId(author) };
    await Note.insertFor(writer, Note.active().set("id", id).set("body", body)).execute(pool());
  }
});

after(async () => {
  await shared?.close();
  await database?.drop();
});

const bodies = (records: readonly { readonly [column: string]: unknown }[]) => records.map((record) => record.body).sort();

// [spec:pgorm:req:bindings.scopes-registered/test]    a registered scoped entity's forms taking no context are refused as they are built
test("forms taking no context are refused", async () => {
  assert.deepEqual(Note.scopes, ["CompanyId", "UserId"]);
  const row = Note.active().set("body", "x");
  for (const build of [() => Note.find(), () => Note.insert(row), () => Note.insertMany([row]), () => Note.update(row), () => Note.updateMany(), () => pl.from(Note)]) {
    assert.throws(build, (error: unknown) => error instanceof ConstructionError && error.message.startsWith(NO_CONTEXT));
  }
  await assert.rejects(row.insert(pool()), (error: unknown) => error instanceof ConstructionError && error.message.startsWith(NO_CONTEXT));
  assert.throws(() => Note.findFor(CompanyId(1n)), {
    message: "`app.ScopedNote` is scoped by `CompanyId` and `UserId`, which `CompanyId` does not hold",
  });
  // @ts-expect-error: no context
  assert.throws(() => Note.findFor(1), TypeError);
  assert.equal(await Note.findFor(ada).count(pool()), 1);
});

// [spec:pgorm:req:bindings.scopes-registered/test]    reads, joins and pipelines carry the scopes Rust's _for forms write
test("reads carry the scopes", async () => {
  assert.equal(
    Note.findFor(ada).inspect().sql,
    `SELECT "note"."id", "note"."company_id", "note"."author_id", "note"."body" FROM "napi_scopes"."note" ` +
      `WHERE "note"."company_id" = $1 AND "note"."author_id" = $2`,
  );
  assert.deepEqual(bodies(await Note.findFor(ada).all(pool())), ["Ada's plan"]);
  assert.deepEqual(bodies(await Note.findFor(admin).all(pool())), ["Ada's plan", "Bo's draft"]);
  const notes = Company.relation("Note");
  assert.throws(() => Company.findFor(ada).join(notes), /join it with a form taking the statement's context, such as `joinFor`/);
  const joined = Company.findFor(ada).joinFor(ada, notes);
  assert.ok(joined.inspect().sql.includes(`"note"."company_id" = $`));
  assert.deepEqual((await joined.all(pool())).map((company) => company.name), ["Acme"]);
  assert.deepEqual(bodies(await pl.fromFor(ada, Note).intoModel((row) => row).all(pool())), ["Ada's plan"]);
});

// [spec:pgorm:req:bindings.scopes-registered/test]    inserts fill or refuse, and updates and deletes reach only the rows the context holds
test("writes carry the scopes", async () => {
  const written = await Note.insertFor(ada, Note.active().set("id", 4n).set("body", "Ada's next")).returningModel(pool());
  assert.deepEqual([written.company_id, written.author_id], [1n, 10n]);
  await assert.rejects(
    Note.insertFor(ada, Note.active().set("id", 5n).set("company_id", 2n).set("body", "for Globex")).execute(pool()),
    /to a value outside the context's `CompanyId`/,
  );
  await assert.rejects(Note.insertFor(admin, Note.active().set("id", 6n).set("body", "whose?")).execute(pool()), /the context holds every `UserId`/);
  const edit = Note.active().set("id", 2n).set("body", "reviewed");
  await assert.rejects(edit.updateFor(ada, pool()), DecodeError);
  assert.equal((await edit.updateFor(admin, pool())).body, "reviewed");
  const changed = await Note.updateManyFor(ada).set("body", "all mine").allRows().returningChanges(pool());
  assert.deepEqual(changed.map((change) => change.new.id).sort(), [1n, 4n]);
  assert.equal(await Note.active().set("id", 2n).deleteFor(ada, pool()), 0);
  assert.equal(await Note.active().set("id", 2n).deleteFor(admin, pool()), 1);
  await assert.rejects(
    Note.active().set("id", 1n).set("author_id", 11n).updateFor(admin, pool()),
    /is written when its row is inserted and never changed/,
  );
});
