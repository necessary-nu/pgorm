"""Views, materialized views and CREATE TABLE AS from the installed native wheel."""
import os
import unittest
from uuid import uuid4

import pgorm as p
from pgorm import schema as s


def accounts_query(table):
    return p.Select(p.col("id"), p.col("name"), p.col("active")).from_(table).where(p.col("active") == True)


class ViewConstruction(unittest.TestCase):
    # [spec:pgorm:req:bindings.schema-views+3/test]    the Select's values are written into the
    # definition, and each option renders where the Rust builder writes it
    def test_views_render_their_query_inlined(self):
        manifest = p.capabilities()["operations"]
        for operation in (
            "create_view", "create_recursive_view", "drop_view", "create_materialized_view",
            "refresh_materialized_view", "drop_materialized_view", "create_table_as",
        ):
            self.assertFalse(manifest["schema." + operation]["registration_required"])
            p.require_capability("schema." + operation)
        table = p.Table("account")
        query = p.Select(p.col("id")).from_(table).where(p.col("name") != "it's")
        view = s.create_view(p.Table("v", schema="app"), query).columns(["key"]).check_option("local")
        compiled = view.inspect()
        self.assertEqual(
            compiled.sql,
            'CREATE VIEW "app"."v" ("key") AS SELECT "id" FROM "account" '
            "WHERE \"name\" <> E'it\\'s' WITH LOCAL CHECK OPTION",
        )
        self.assertEqual(compiled.params, [])
        self.assertEqual(
            s.refresh_materialized_view(p.Table("m"), concurrently=True).inspect().sql,
            'REFRESH MATERIALIZED VIEW CONCURRENTLY "m"',
        )
        counting = p.Select(p.literal(1).as_("n")).union_all(
            p.Select(p.col("n") + 1).from_(p.Table("counter")).where(p.col("n") < 4)
        )
        recursive = s.create_recursive_view(p.Table("counter", schema="app"), ["n"], counting).inspect()
        self.assertEqual(
            recursive.sql,
            'CREATE RECURSIVE VIEW "app"."counter" ("n") AS SELECT 1 AS "n" '
            'UNION ALL (SELECT "n" + 1 FROM "counter" WHERE "n" < 4)',
        )
        self.assertEqual(recursive.params, [])

    # [spec:pgorm:req:bindings.schema-views+3/test]    what PostgreSQL refuses and the Rust types
    # cannot hold is refused as the statement is built
    def test_invalid_view_options_are_refused(self):
        table = p.Table("account")
        query = p.Select(p.col("id")).from_(table)
        for action in (
            lambda: s.create_recursive_view(p.Table("r"), [], query),
            lambda: s.refresh_materialized_view(p.Table("m"), concurrently=True, with_no_data=True),
            lambda: s.create_view(p.Table("v", alias="w"), query),
            lambda: s.create_materialized_view(p.Table("m", alias="w"), query),
            lambda: s.create_table_as(p.Table("t"), query).columns([""]),
            lambda: s.drop_view(p.Table("v", alias="w")),
        ):
            with self.subTest(action=action), self.assertRaises(p.ConstructionError):
                action()
        for action in (
            lambda: s.create_view(p.Table("v"), p.RawSQL("SELECT 1")),
            lambda: s.create_view(p.Table("v"), query).check_option("both"),
            lambda: s.create_table_as(p.Table("t"), query).columns("key"),
        ):
            with self.subTest(action=action), self.assertRaises(TypeError):
                action()
        # The grammar takes no check option on a recursive view, no IF NOT EXISTS on a view
        # and no OR REPLACE on a materialized view or a table, so their builders have none.
        self.assertFalse(hasattr(s.CreateRecursiveView, "check_option"))
        self.assertFalse(hasattr(s.CreateView, "if_not_exists"))
        self.assertFalse(hasattr(s.CreateMaterializedView, "or_replace"))
        self.assertFalse(hasattr(s.CreateTableAs, "or_replace"))

    # [spec:pgorm:req:bindings.schema-views+3/test]    each builder refuses a query whose tie limit
    # has no ORDER BY, or whose window is undeclared, as it is called, with the Rust constructor's error
    def test_view_bodies_check_tie_limit_and_windows(self):
        table = p.Table("account")
        step = p.Select(p.col("n") + 1).from_(p.Table("r")).where(p.col("n") < 4)
        builders = {
            "create_view": lambda q: s.create_view(p.Table("v"), q),
            "create_recursive_view": lambda q: s.create_recursive_view(p.Table("r"), ["n"], q.union_all(step)),
            "create_materialized_view": lambda q: s.create_materialized_view(p.Table("m"), q),
            "create_table_as": lambda q: s.create_table_as(p.Table("t"), q),
        }
        unordered = p.Select(p.col("id").as_("n")).from_(table).limit_with_ties(2)
        undeclared = p.Select(p.call("count", p.col("id")).over("w").as_("n")).from_(table)
        ordered = unordered.order_by(p.col("id").asc())
        for name, build in builders.items():
            with self.subTest(builder=name):
                with self.assertRaisesRegex(p.ConstructionError, "ORDER BY"):
                    build(unordered)
                with self.assertRaisesRegex(p.ConstructionError, 'no window "w"'):
                    build(undeclared)
                compiled = build(ordered).inspect()
                self.assertIn('ORDER BY "id" ASC FETCH FIRST 2 ROWS WITH TIES', compiled.sql)
                self.assertEqual(compiled.params, [])


class ViewDatabase(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        self.namespace = 'views "雪 ' + uuid4().hex[:8]
        self.quoted = '"' + self.namespace.replace('"', '""') + '"'
        await self.pool.execute(p.RawSQL(f"CREATE SCHEMA {self.quoted}"))
        self.table = p.Table("account", schema=self.namespace)
        create = s.create_table(self.table).column(s.ColumnDef("id", "integer").auto_increment()).primary_key("id")
        create = create.column(s.ColumnDef("name", "text").not_null())
        await self.pool.execute(create.column(s.ColumnDef("active", "boolean").not_null().default(True)))
        await self.pool.execute(p.RawSQL(
            f"INSERT INTO {self.quoted}.account (name, active) VALUES ('ada', true), ('it''s', true), ('cy', false)"
        ))

    async def asyncTearDown(self):
        await self.pool.execute(p.RawSQL(f"DROP SCHEMA {self.quoted} CASCADE"))
        await self.pool.close()

    def relation(self, name):
        return p.Table(name, schema=self.namespace)

    async def names(self, relation):
        rows = await self.pool.query(p.Select(p.col("name")).from_(relation).order_by(p.col("id").asc()))
        return [row["name"] for row in rows]

    # [spec:pgorm:req:bindings.schema-views+3/test]    a view built from a Select reads the rows it
    # shows, an insert through it reaches the table, and its check option refuses a hidden row
    async def test_view_reads_writes_and_checks_rows(self):
        view = self.relation('active "x"')
        await self.pool.execute(s.create_view(view, accounts_query(self.table)).check_option("cascaded"))
        self.assertEqual(await self.names(view), ["ada", "it's"])
        await self.pool.execute(p.insert(view).columns("name").values("dee"))
        self.assertEqual(await self.names(self.table), ["ada", "it's", "cy", "dee"])
        with self.assertRaises(p.DatabaseError) as refused:
            await self.pool.execute(p.insert(view).columns("name", "active").values("hidden", False))
        self.assertEqual(refused.exception.sqlstate, "44000")

        await self.pool.execute(s.create_view(
            view, accounts_query(self.table).select(p.col("id"), p.col("name"), p.col("active"), p.col("id").as_("again")),
        ).or_replace().security_barrier())
        options = await self.pool.one(p.RawSQL(
            "SELECT reloptions::text[] AS options FROM pg_class WHERE oid = $1::text::regclass",
            [f'{self.quoted}."active ""x"""'],
        ))
        self.assertEqual(options["options"], ["security_barrier=true"])
        with self.assertRaises(p.DatabaseError) as refused:
            await self.pool.execute(s.drop_view(self.table))
        self.assertEqual(refused.exception.sqlstate, "42809")
        await self.pool.execute(s.drop_view(view, behavior="cascade"))
        await self.pool.execute(s.drop_view(view, if_exists=True))

    # [spec:pgorm:req:bindings.schema-views+3/test]    an ordered tie limit the check admits is
    # stored in the definition, and the relation holds every row tied with the last
    async def test_ordered_tie_limit_view_keeps_ties(self):
        tied = p.Select(p.col("id"), p.col("name")).from_(self.table).order_by(p.col("active").desc()).limit_with_ties(1)
        view, copy = self.relation("first active"), self.relation("first active copy")
        await self.pool.execute(s.create_view(view, tied))
        await self.pool.execute(s.create_table_as(copy, tied))
        for relation in (view, copy):
            self.assertEqual(await self.names(relation), ["ada", "it's"])

    # [spec:pgorm:req:bindings.schema-views+3/test]    a recursive view's query reads the view
    # through its UNION ALL arm, by the bare name its WITH RECURSIVE form gives it
    async def test_recursive_view_reads_itself(self):
        counter = self.relation("counter")

        def counting(source):
            step = p.Select(p.col("n") + 1).from_(source).where(p.col("n") < 4)
            return p.Select(p.literal(1).as_("n")).union_all(step)

        with self.assertRaises(p.DatabaseError) as refused:
            await self.pool.execute(s.create_recursive_view(counter, ["n"], counting(counter)))
        self.assertEqual(refused.exception.sqlstate, "42P01")
        await self.pool.execute(s.create_recursive_view(counter, ["n"], counting(p.Table("counter"))))
        counted = await self.pool.query(p.Select(p.col("n")).from_(counter).order_by(p.col("n").asc()))
        self.assertEqual([row["n"] for row in counted], [1, 2, 3, 4])
        await self.pool.execute(s.drop_view(counter))

    # [spec:pgorm:req:bindings.schema-views+3/test]    a materialized view holds its rows until a
    # refresh, and a concurrent refresh needs a unique index
    async def test_materialized_view_refreshes_concurrently(self):
        view = self.relation('snapshot "x"')
        await self.pool.execute(s.create_materialized_view(view, accounts_query(self.table)).with_no_data())
        with self.assertRaises(p.DatabaseError) as refused:
            await self.names(view)
        self.assertEqual(refused.exception.sqlstate, "55000")
        await self.pool.execute(s.refresh_materialized_view(view))
        self.assertEqual(await self.names(view), ["ada", "it's"])
        await self.pool.execute(p.insert(self.table).columns("name").values("dee"))
        self.assertEqual(await self.names(view), ["ada", "it's"])
        with self.assertRaises(p.DatabaseError) as refused:
            await self.pool.execute(s.refresh_materialized_view(view, concurrently=True))
        self.assertEqual(refused.exception.sqlstate, "55000")
        await self.pool.execute(s.create_index(view, "id", name="snapshot_id").unique())
        await self.pool.execute(s.refresh_materialized_view(view, concurrently=True))
        self.assertEqual(await self.names(view), ["ada", "it's", "dee"])
        await self.pool.execute(s.refresh_materialized_view(view, with_no_data=True))
        with self.assertRaises(p.DatabaseError) as refused:
            await self.names(view)
        self.assertEqual(refused.exception.sqlstate, "55000")
        await self.pool.execute(s.create_materialized_view(view, p.Select(p.literal(1))).if_not_exists())
        await self.pool.execute(s.drop_materialized_view(view))
        await self.pool.execute(s.drop_materialized_view(view, if_exists=True))

    # [spec:pgorm:req:bindings.schema-views+3/test]    a table created from a Select with no data
    # has its columns, renamed where names are given, and keeps the rows inserted later
    async def test_table_created_from_query_without_data(self):
        copy = self.relation('copy "x"')
        await self.pool.execute(s.create_table_as(copy, accounts_query(self.table)).columns(["account_id"]).with_no_data())
        empty = await self.pool.query(p.Select(p.col("account_id")).from_(copy))
        self.assertEqual(empty, [])
        await self.pool.execute(p.RawSQL(
            f'INSERT INTO {self.quoted}."copy ""x""" SELECT id, name, active FROM {self.quoted}.account WHERE active'
        ))
        rows = await self.pool.query(p.Select(p.col("account_id"), p.col("name")).from_(copy).order_by(p.col("account_id").asc()))
        self.assertEqual([dict(row) for row in rows], [{"account_id": 1, "name": "ada"}, {"account_id": 2, "name": "it's"}])
        await self.pool.execute(s.create_table_as(copy, p.Select(p.literal(1))).if_not_exists())
        with self.assertRaises(p.DatabaseError) as refused:
            await self.pool.execute(s.create_table_as(self.relation("too_many"), p.Select(p.literal(1))).columns(["a", "b"]))
        self.assertEqual(refused.exception.sqlstate, "42601")


if __name__ == "__main__":
    unittest.main()
