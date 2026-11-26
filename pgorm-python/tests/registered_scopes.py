"""A registered scoped Rust entity stays scoped from Python: every statement
on it starts from a ``_for`` form given a context holding its scopes, which
Rust's own ``_for`` forms hold it to, and every statement built without one
is refused before anything is sent. Run only against the application wheel."""

from dataclasses import dataclass
import os
import unittest

import pgorm as p
from pgorm import pipeline as pl, schema as s


class CompanyId(p.Scope[int]):
    pass


class UserId(p.Scope[int]):
    pass


@dataclass(frozen=True)
class Member(p.ScopeContext):
    company: CompanyId
    user: UserId


@dataclass(frozen=True)
class CompanyAdmin(p.ScopeContext):
    company: CompanyId
    users: p.Every[UserId]


ADA = Member(CompanyId(1), UserId(10))
ADMIN = CompanyAdmin(CompanyId(1), p.Every(UserId))
NO_CONTEXT = (
    "`app.ScopedNote` is scoped by `CompanyId` and `UserId`, which a statement taking no "
    "context does not hold"
)


class RegisteredScopes(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        self.company = p.entity("app.ScopedCompany")
        self.note = p.entity("app.ScopedNote")
        await self.pool.execute(p.RawSQL("CREATE SCHEMA python_scopes"))
        for entity in (self.company, self.note):
            await self.pool.execute(s.from_entity(entity).table)
        for identity, name in ((1, "Acme"), (2, "Globex")):
            row = self.company.active().set("name", name)
            await self.company.insert_for(CompanyId(identity), row).execute(self.pool)
        for identity, company, author, body in (
            (1, 1, 10, "Ada's plan"),
            (2, 1, 11, "Bo's draft"),
            (3, 2, 20, "Cy's memo"),
        ):
            row = self.note.active().set("id", identity).set("body", body)
            writer = Member(CompanyId(company), UserId(author))
            await self.note.insert_for(writer, row).execute(self.pool)

    async def asyncTearDown(self):
        await self.pool.execute(p.RawSQL("DROP SCHEMA python_scopes CASCADE"))
        await self.pool.close()

    def bodies(self, models):
        return sorted(model["body"] for model in models)

    # [spec:pgorm:req:bindings.scopes-registered/test]    a registered scoped entity's forms taking no context are refused as they are built
    async def test_forms_taking_no_context_are_refused(self):
        self.assertEqual(self.note.scopes, ["CompanyId", "UserId"])
        row = self.note.active().set("body", "x")
        refused = [
            lambda: self.note.find(),
            lambda: self.note.insert(row),
            lambda: self.note.insert_many([row]),
            lambda: self.note.update(row),
            lambda: self.note.update_many(),
            lambda: pl.from_(self.note),
        ]
        for build in refused:
            with self.subTest(build=build), self.assertRaises(p.ConstructionError) as raised:
                build()
            self.assertIn(NO_CONTEXT, str(raised.exception))
        with self.assertRaises(p.ConstructionError) as raised:
            await row.insert(self.pool)
        self.assertIn(NO_CONTEXT, str(raised.exception))
        with self.assertRaises(p.ConstructionError) as raised:
            self.note.find_for(CompanyId(1))
        self.assertIn(
            "`app.ScopedNote` is scoped by `CompanyId` and `UserId`, which `CompanyId` does not hold",
            str(raised.exception),
        )
        with self.assertRaises(TypeError):
            self.note.find_for(1)
        self.assertEqual(await self.note.find_for(ADA).count(self.pool), 1)

    # [spec:pgorm:req:bindings.scopes-registered/test]    reads, joins and pipelines carry the scopes Rust's _for forms write
    async def test_reads_carry_the_scopes(self):
        self.assertEqual(
            self.note.find_for(ADA).inspect().sql,
            'SELECT "note"."id", "note"."company_id", "note"."author_id", "note"."body" '
            'FROM "python_scopes"."note" WHERE "note"."company_id" = $1 AND "note"."author_id" = $2',
        )
        self.assertEqual(self.bodies(await self.note.find_for(ADA).all(self.pool)), ["Ada's plan"])
        self.assertEqual(
            self.bodies(await self.note.find_for(ADMIN).all(self.pool)), ["Ada's plan", "Bo's draft"]
        )
        notes = self.company.relation("Note")
        with self.assertRaises(p.ConstructionError) as raised:
            self.company.find_for(ADA).join(notes)
        self.assertIn("join it with a form taking the statement's context, such as `join_for`", str(raised.exception))
        joined = self.company.find_for(ADA).join_for(ADA, notes)
        self.assertIn('"note"."company_id" = $', joined.inspect().sql)
        self.assertEqual([model["name"] for model in await joined.all(self.pool)], ["Acme"])
        piped = pl.from_for(ADA, self.note)
        self.assertEqual(self.bodies(await piped.into_model(lambda **row: row).all(self.pool)), ["Ada's plan"])

    # [spec:pgorm:req:bindings.scopes-registered/test]    inserts fill or refuse, and updates and deletes reach only the rows the context holds
    async def test_writes_carry_the_scopes(self):
        written = await self.note.insert_for(ADA, self.note.active().set("id", 4).set("body", "Ada's next")).returning_model(self.pool)
        self.assertEqual((written["company_id"], written["author_id"]), (1, 10))
        elsewhere = self.note.active().set("id", 5).set("company_id", 2).set("body", "for Globex")
        with self.assertRaises(p.ConstructionError) as raised:
            await self.note.insert_for(ADA, elsewhere).execute(self.pool)
        self.assertIn("to a value outside the context's `CompanyId`", str(raised.exception))
        unset = self.note.active().set("id", 6).set("body", "whose?")
        with self.assertRaises(p.ConstructionError) as raised:
            await self.note.insert_for(ADMIN, unset).execute(self.pool)
        self.assertIn("the context holds every `UserId`", str(raised.exception))
        edit = self.note.active().set("id", 2).set("body", "reviewed")
        with self.assertRaises(p.DecodeError):
            await edit.update_for(ADA, self.pool)
        self.assertEqual((await edit.update_for(ADMIN, self.pool))["body"], "reviewed")
        changed = await self.note.update_many_for(ADA).set("body", "all mine").all_rows().returning_changes(self.pool)
        self.assertEqual(sorted(change.new["id"] for change in changed), [1, 4])
        self.assertEqual(await self.note.active().set("id", 2).delete_for(ADA, self.pool), 0)
        self.assertEqual(await self.note.active().set("id", 2).delete_for(ADMIN, self.pool), 1)
        handed = self.note.active().set("id", 1).set("author_id", 11)
        with self.assertRaises(p.ConstructionError) as raised:
            await handed.update_for(ADMIN, self.pool)
        self.assertIn("is written when its row is inserted and never changed", str(raised.exception))


if __name__ == "__main__":
    unittest.main()
