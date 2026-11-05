"""CREATE SCHEMA, DROP SCHEMA and ALTER SCHEMA ... RENAME TO from the installed native wheel."""
import os
import unittest
from uuid import uuid4

import pgorm as p
from pgorm import schema as s


class SchemaConstruction(unittest.TestCase):
    # [spec:pgorm:req:bindings.schema-schemas/test]    each statement names its schema as one
    # quoted identifier, a drop names every schema given in order, and nothing is bound
    def test_schema_statements_render_quoted_names(self):
        manifest = p.capabilities()["operations"]
        for operation in ("create_schema", "drop_schema", "rename_schema"):
            self.assertFalse(manifest["schema." + operation]["registration_required"])
            p.require_capability("schema." + operation)
        create = s.create_schema('bill "雪"', if_not_exists=True).inspect()
        self.assertEqual(create.sql, 'CREATE SCHEMA IF NOT EXISTS "bill ""雪"""')
        self.assertEqual(create.params, [])
        self.assertEqual(
            s.drop_schema(["a", "B", "c"], if_exists=True, behavior="cascade").inspect().sql,
            'DROP SCHEMA IF EXISTS "a", "B", "c" CASCADE',
        )
        self.assertEqual(s.drop_schema("a").inspect().sql, 'DROP SCHEMA "a"')
        self.assertEqual(s.drop_schema(("a",), behavior="restrict").inspect().sql, 'DROP SCHEMA "a" RESTRICT')
        self.assertEqual(
            s.rename_schema("a", 'b "x"').inspect().sql,
            'ALTER SCHEMA "a" RENAME TO "b ""x"""',
        )

    # [spec:pgorm:req:bindings.schema-schemas/test]    a name that is not an identifier is
    # refused as the statement is built, and a drop needs at least one name
    def test_invalid_schema_names_are_refused(self):
        for action in (
            lambda: s.create_schema(""),
            lambda: s.create_schema("x" * 64),
            lambda: s.create_schema("nul\0"),
            lambda: s.drop_schema(["a", ""]),
            lambda: s.drop_schema([]),
            lambda: s.rename_schema("a", ""),
        ):
            with self.subTest(action=action), self.assertRaises(p.ConstructionError):
                action()
        for action in (
            lambda: s.drop_schema(),
            lambda: s.drop_schema("a", behavior="both"),
            lambda: s.drop_schema(p.Table("t")),
            lambda: s.create_schema(p.Table("t")),
            lambda: s.drop_schema(["a", 3]),
            lambda: s.drop_schema("a", "b"),
        ):
            with self.subTest(action=action), self.assertRaises(TypeError):
                action()
        with self.assertRaises(TypeError):
            s.rename_schema("a", "b", if_exists=True)
        with self.assertRaises(p.ConstructionError):
            bool(s.create_schema("a"))


class SchemaDatabase(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        suffix = uuid4().hex[:8]
        self.namespace = 'schemas "雪 ' + suffix
        self.renamed = 'renamed "雪 ' + suffix

    async def asyncTearDown(self):
        await self.pool.execute(s.drop_schema([self.namespace, self.renamed], if_exists=True, behavior="cascade"))
        await self.pool.close()

    async def exists(self, name):
        row = await self.pool.one(p.RawSQL(
            "SELECT count(*) AS n FROM pg_namespace WHERE nspname = $1", [name],
        ))
        return row["n"] == 1

    # [spec:pgorm:req:bindings.schema-schemas/test]    a created schema holds a table qualified by
    # it, a drop is refused while it holds one until it cascades, and a rename carries the table
    async def test_schema_holds_renames_and_drops(self):
        await self.pool.execute(s.create_schema(self.namespace))
        self.assertTrue(await self.exists(self.namespace))
        table = p.Table("item", schema=self.namespace)
        await self.pool.execute(s.create_table(table).column(s.ColumnDef("id", "integer").not_null()))
        await self.pool.execute(p.insert(table).columns("id").values(1))

        with self.assertRaises(p.DatabaseError) as refused:
            await self.pool.execute(s.drop_schema(self.namespace))
        self.assertEqual(refused.exception.sqlstate, "2BP01")

        await self.pool.execute(s.rename_schema(self.namespace, self.renamed))
        self.assertFalse(await self.exists(self.namespace))
        rows = await self.pool.query(p.Select(p.col("id")).from_(p.Table("item", schema=self.renamed)))
        self.assertEqual([row["id"] for row in rows], [1])

        await self.pool.execute(s.drop_schema(self.renamed, behavior="cascade"))
        self.assertFalse(await self.exists(self.renamed))

    # [spec:pgorm:req:bindings.schema-schemas/test]    the server's refusals arrive by SQLSTATE: a
    # schema that exists, a pg_ name and a missing schema
    async def test_server_refusals_by_sqlstate(self):
        await self.pool.execute(s.create_schema(self.namespace))
        with self.assertRaises(p.DatabaseError) as refused:
            await self.pool.execute(s.create_schema(self.namespace))
        self.assertEqual(refused.exception.sqlstate, "42P06")
        await self.pool.execute(s.create_schema(self.namespace, if_not_exists=True))
        with self.assertRaises(p.DatabaseError) as refused:
            await self.pool.execute(s.create_schema("pg_" + uuid4().hex[:8]))
        self.assertEqual(refused.exception.sqlstate, "42939")
        with self.assertRaises(p.DatabaseError) as refused:
            await self.pool.execute(s.rename_schema(self.renamed, "other"))
        self.assertEqual(refused.exception.sqlstate, "3F000")
        with self.assertRaises(p.DatabaseError) as refused:
            await self.pool.execute(s.drop_schema(self.renamed))
        self.assertEqual(refused.exception.sqlstate, "3F000")
        await self.pool.execute(s.drop_schema([self.renamed, self.namespace], if_exists=True))
        self.assertFalse(await self.exists(self.namespace))


if __name__ == "__main__":
    unittest.main()
