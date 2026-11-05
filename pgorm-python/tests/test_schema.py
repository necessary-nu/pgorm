"""Explicit DDL from the installed native wheel, with no implicit database work."""
import importlib
import os
import unittest
from uuid import uuid4

import pgorm as p
from pgorm import _native, schema as s

# The release the native module targets: PostgreSQL 19 when pgorm was built
# with its pg-19 feature, 18 otherwise. Where the two servers answer a
# statement differently, a test holds the answer of the release targeted.
# [spec:pgorm:req:sql.target+1/test]
PG_19 = "pg-19" in p.capabilities()["features"]


class SchemaConstruction(unittest.TestCase):
    # [spec:pgorm:req:bindings.schema/test]
    # [spec:pgorm:req:bindings.schema-api+2/test]
    def test_capabilities_and_detached_builders(self):
        manifest = p.capabilities()
        self.assertFalse(manifest["schema_policy"]["automatic_ddl"])
        self.assertFalse(manifest["operations"]["schema.create_table"]["registration_required"])
        self.assertTrue(manifest["operations"]["schema.from_entity"]["registration_required"])
        for operation in manifest["operations"]:
            if operation.startswith("schema."):
                p.require_capability(operation)
        table = s.create_table(p.Table("test"))
        column = s.ColumnDef("id", "integer")
        built = table.column(column.not_null()).primary_key("id")
        self.assertNotIn('"id"', table.inspect().sql)
        self.assertIn('"id" integer NOT NULL', built.inspect().sql)
        self.assertNotIn("NOT NULL", table.column(column).inspect().sql)
        self.assertEqual(built.inspect().params, [])
        with self.assertRaises(TypeError):
            s.from_entity(p.Model("test", {"id": p.Column("i32")}))

    # [spec:pgorm:req:bindings.schema/test]
    def test_all_declared_types_retain_native_identity(self):
        for kind in p.capabilities()["schema_policy"]["column_types"]:
            with self.subTest(kind=kind):
                data_type = s.DataType(kind, length=16) if kind == "varbit" else s.DataType(kind)
                ddl = s.create_table(p.Table("test")).column(s.ColumnDef("v", data_type))
                self.assertTrue(ddl.inspect().sql.startswith('CREATE TABLE "test"'))
                self.assertEqual(ddl.inspect().params, [])
        kind = p.TypeName('Mood "雪"', schema='schema "雪"')
        sql = s.create_table(p.Table("test")).column(s.ColumnDef("v", s.DataType(kind).array())).inspect().sql
        self.assertIn('"schema ""雪"""."Mood ""雪"""[]', sql)

    # [spec:pgorm:req:bindings.collations/test]    a collation is named by identifiers, quoted
    # and qualified as given, and a later call replaces an earlier
    def test_column_collation_names_are_identifiers(self):
        column = s.ColumnDef("word", "text").collate("C")
        sql = s.create_table(p.Table("t")).column(column.collate('C "x"', schema="pg_catalog")).inspect().sql
        self.assertEqual(sql, 'CREATE TABLE "t" ( "word" text COLLATE "pg_catalog"."C ""x""" )')
        self.assertIn('"word" text COLLATE "C"', s.alter_table(p.Table("t")).add_column(column).inspect().sql)
        for action in (
            lambda: column.collate(""),
            lambda: column.collate("C", schema=""),
            lambda: column.collate("雪" * 22),
        ):
            with self.subTest(action=action), self.assertRaises(p.ConstructionError):
                action()
        with self.assertRaises(TypeError):
            column.collate(1)

    # [spec:pgorm:req:bindings.schema/test]
    def test_invalid_options_and_unquoted_fragments_fail(self):
        for action in (
            lambda: s.DataType("text; DROP TABLE test"),
            lambda: s.DataType("text", length=4),
            lambda: s.DataType("numeric", scale=2),
            lambda: s.DataType("numeric", precision=0),
            lambda: s.DataType("numeric", precision=10, scale=1001),
            lambda: s.DataType("varchar", length=0),
            lambda: s.DataType(p.TypeName("custom"), length=3),
            lambda: s.create_table(p.Table("test", alias="t")),
            lambda: s.create_index(p.Table("test", alias="t"), "v"),
            lambda: s.drop_table(p.Table("test", alias="t")),
            lambda: s.ColumnDef("", "text"),
            lambda: s.create_type("mood").values(["a\0b"]),
            lambda: s.create_type("mood").values(["雪" * 22]),
            lambda: s.alter_type("mood").add_value("v", before="a", after="b"),
            lambda: s.alter_table(p.Table("test", alias="t")).add_not_null("c"),
        ):
            with self.subTest(action=action), self.assertRaises(p.PgormError):
                action()
        for action in (
            lambda: s.DataType("varchar", length=True),
            lambda: s.ColumnDef("v", "text").generated("raw SQL", "stored"),
            lambda: s.ColumnDef("v", "integer").generated(p.col("id"), "persisted"),
            lambda: s.alter_table(p.Table("test")).alter_constraint("c", "deferrable"),
        ):
            with self.subTest(action=action), self.assertRaises(TypeError):
                action()

    # [spec:pgorm:req:python.truth-testing+3/test]    every schema builder and pending ALTER
    # refuses truth testing too
    def test_schema_objects_refuse_truth_testing(self):
        table, query = p.Table("test"), p.select(p.col("id")).from_(p.Table("source"))
        samples = [
            s.DataType("integer"), s.ColumnDef("id", "integer"), s.create_table(table),
            s.create_index(table, "id"), s.drop_table(table), s.create_type("mood").values(["calm"]),
            s.alter_table(table), s.alter_table(table).drop_column("id"), s.alter_type("mood"),
            s.alter_type("pair").drop_attribute("a"), s.alter_type("pair").rename_attribute("a", "b"),
            s.create_view(table, query), s.create_recursive_view(table, ["id"], query),
            s.create_materialized_view(table, query), s.refresh_materialized_view(table),
            s.create_table_as(table, query), s.create_domain("quantity", "integer"),
            s.alter_domain("quantity"), s.create_sequence("ticket"), s.alter_sequence("ticket"),
            s.alter_sequence("ticket").restart(), s.DisjointGap("bike", table),
            s.DisjointStep((), None), s.DisjointChange(()),
        ]
        for sample in [*samples, s.create_table(table).inspect()]:
            with self.subTest(sample=sample), self.assertRaises(p.ConstructionError):
                bool(sample)
        # Every class the native schema module defines or the package exports
        # needs a sample above, so a new one fails here until it refuses too.
        # EntitySchema needs a registered entity: registered_schema.py tests it.
        native = {kind for kind in vars(_native).values()
                  if isinstance(kind, type) and kind.__module__ == "pgorm.schema"}
        exported = {kind for kind in map(vars(s).get, s.__all__) if isinstance(kind, type)}
        unsampled = (native | exported) - {type(sample) for sample in samples}
        self.assertEqual(unsampled, {s.EntitySchema})
        self.assertIn("__bool__", vars(s.EntitySchema))

    # [spec:pgorm:req:bindings.schema-indexes/test]    an access method is any identifier,
    # quoted where pgorm-query writes it, never SQL
    def test_index_access_method_is_an_identifier(self):
        index = s.create_index(p.Table("test"), "v").using("gin); SELECT 1 --")
        self.assertEqual(index.inspect().sql, 'CREATE INDEX  ON "test" USING "gin); SELECT 1 --" ("v")')


class SchemaDatabase(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        self.namespace = 'ddl "雪 ' + uuid4().hex[:8]
        self.quoted = '"' + self.namespace.replace('"', '""') + '"'
        await self.pool.execute(p.RawSQL(f"CREATE SCHEMA {self.quoted}"))

    async def asyncTearDown(self):
        await self.pool.execute(p.RawSQL(f"DROP SCHEMA {self.quoted} CASCADE"))
        await self.pool.close()

    # [spec:pgorm:req:bindings.schema-tables/test]
    # [spec:pgorm:req:bindings.schema-indexes/test]
    # [spec:pgorm:req:bindings.schema-alter/test]
    async def test_explicit_table_index_and_alter_execution(self):
        model = p.Model('items "x"', {"id": p.Column("i32", primary_key=True), "name": p.Column("text")}, schema=self.namespace)
        importlib.reload(s)
        table = model.table
        ddl = s.create_table(table).column(s.ColumnDef("id", "integer").auto_increment()).primary_key("id")
        ddl = ddl.column(s.ColumnDef("name", "text").not_null().default("O'Brien \\ 雪"))
        ddl = ddl.column(s.ColumnDef("twice", "integer").generated(p.col("id") * 2, "stored"))
        ddl = ddl.column(s.ColumnDef("next", "integer").generated(p.col("id") + 1, "virtual")).check(p.col("id") > 0)
        ddl.inspect()
        count = await self.pool.one(p.RawSQL("SELECT count(*) AS n FROM information_schema.tables WHERE table_schema = $1", [self.namespace]))
        self.assertEqual(count["n"], 0)
        await self.pool.execute(ddl)
        await self.pool.execute(ddl.if_not_exists())
        inserted = await self.pool.one(p.insert(table).default_values().returning(p.col("id"), p.col("name"), p.col("twice"), p.col("next")))
        self.assertEqual(dict(inserted), {"id": 1, "name": "O'Brien \\ 雪", "twice": 2, "next": 2})
        async with self.pool.connection() as connection:
            index = s.create_index(table, "name", name='idx "雪"').column("id", order="desc").nulls_not_distinct()
            await connection.execute(index)
            await connection.execute(index.if_not_exists())
            await connection.execute(s.alter_table(table).add_column(s.ColumnDef("extra", "text").default("hello")))
            await connection.execute(s.alter_table(table).add_column(s.ColumnDef("extra", "text"), if_not_exists=True))
            await connection.execute(s.alter_table(table).modify_column(s.ColumnDef("extra", "varchar").not_null().default("changed")))
            await connection.execute(s.alter_table(table).add_unique(["extra", "name"], name='unique "x"', nulls_not_distinct=True))
            keyed = s.alter_table(table).add_primary_key("id")
            self.assertTrue(keyed.inspect().sql.endswith('ADD PRIMARY KEY ("id")'), keyed.inspect().sql)
            with self.assertRaises(p.DatabaseError) as refused:
                await connection.execute(keyed)
            self.assertEqual(refused.exception.sqlstate, "42P16")
            derived = p.RawSQL(f'SELECT twice, next FROM {self.quoted}."items ""x"""')
            await connection.execute(s.alter_table(table).set_expression("twice", p.col("id") * 10))
            self.assertEqual(dict(await connection.one(derived)), {"twice": 10, "next": 2})
            virtual = [s.alter_table(table).drop_expression("next", if_exists=True)]
            if PG_19:
                # 19 checks the rows against the table's CHECK instead of refusing.
                await connection.execute(s.alter_table(table).set_expression("next", p.col("id") + 100))
                self.assertEqual(dict(await connection.one(derived)), {"twice": 10, "next": 101})
            else:
                # 18 refuses a virtual column a new expression while its table has a CHECK.
                virtual.insert(0, s.alter_table(table).set_expression("next", p.col("id") + 100))
            for refused_action in virtual:
                with self.assertRaises(p.DatabaseError) as refused_virtual:
                    await connection.execute(refused_action)
                self.assertEqual(refused_virtual.exception.sqlstate, "0A000")
            await connection.execute(s.alter_table(table).drop_expression("twice"))
            await connection.execute(s.alter_table(table).drop_expression("twice", if_exists=True))
            with self.assertRaises(p.DatabaseError) as plain:
                await connection.execute(s.alter_table(table).drop_expression("twice"))
            self.assertEqual(plain.exception.sqlstate, "55000")
            await connection.execute(s.rename_column(table, "extra", 'renamed "x"'))
            await connection.execute(s.alter_table(table).drop_column('renamed "x"'))
            await connection.execute(s.drop_index(table, 'idx "雪"'))
            await connection.execute(s.drop_index(table, 'idx "雪"', if_exists=True))
            await connection.execute(s.truncate_table(table))
            self.assertEqual(await connection.query(p.select(p.col("id")).from_(table)), [])
            await connection.execute(s.rename_table(table, 'renamed "table"'))
            renamed = p.Table('renamed "table"', schema=self.namespace)
            await connection.execute(s.drop_table(renamed))
            await connection.execute(s.drop_table(renamed, if_exists=True))

    # [spec:pgorm:req:bindings.expressions+2/test]
    async def test_uuidv7_default_keys_rows_in_write_order(self):
        table = p.Table("minted", schema=self.namespace)
        ddl = s.create_table(table).column(s.ColumnDef("id", "uuid").default(p.call("uuidv7"))).primary_key("id")
        ddl = ddl.column(s.ColumnDef("written", "integer").not_null())
        self.assertIn('"id" uuid DEFAULT UUIDV7()', ddl.inspect().sql)
        await self.pool.execute(ddl)
        for written in range(16):
            await self.pool.execute(p.insert(table).columns("written").values(written))
        version = p.call("uuid_extract_version", p.col("id")).as_("version")
        rows = await self.pool.query(p.select(p.col("written"), version).from_(table).order_by(p.col("id").asc()))
        self.assertEqual([dict(row) for row in rows], [{"written": n, "version": 7} for n in range(16)])

    # [spec:pgorm:req:bindings.schema-tables/test]
    async def test_not_null_constraints_by_name(self):
        table = p.Table('kept "x"', schema=self.namespace)
        quoted = f'{self.quoted}."kept ""x"""'
        ddl = s.create_table(table).column(s.ColumnDef("id", "integer").not_null(name='id "present"'))
        await self.pool.execute(ddl.column(s.ColumnDef("note", "text")))
        await self.pool.execute(p.RawSQL(f"INSERT INTO {quoted} VALUES (1, NULL)"))
        catalog = p.RawSQL(
            "SELECT conname::text AS name, convalidated AS valid, connoinherit AS no_inherit "
            "FROM pg_constraint WHERE conrelid = $1::text::regclass AND contype = 'n' ORDER BY conname",
            [quoted],
        )
        async with self.pool.connection() as connection:
            await connection.execute(s.alter_table(table).add_not_null("note", name="note present", not_valid=True))
            rows = [dict(row) for row in await connection.query(catalog)]
            self.assertEqual(rows, [
                {"name": 'id "present"', "valid": True, "no_inherit": False},
                {"name": "note present", "valid": False, "no_inherit": False},
            ])
            with self.assertRaises(p.DatabaseError) as refused:
                await connection.execute(s.alter_table(table).validate_constraint("note present"))
            self.assertEqual(refused.exception.sqlstate, "23502")
            await connection.execute(p.RawSQL(f"UPDATE {quoted} SET note = ''"))
            await connection.execute(s.alter_table(table).validate_constraint("note present"))
            await connection.execute(s.alter_table(table).alter_constraint('id "present"', "no_inherit"))
            rows = [dict(row) for row in await connection.query(catalog)]
            self.assertEqual(rows, [
                {"name": 'id "present"', "valid": True, "no_inherit": True},
                {"name": "note present", "valid": True, "no_inherit": False},
            ])

    # [spec:pgorm:req:bindings.schema-alter/test]
    async def test_constraints_drop_and_rename_by_name(self):
        table = p.Table('checked "x"', schema=self.namespace)
        quoted = f'{self.quoted}."checked ""x"""'
        ddl = s.create_table(table).column(s.ColumnDef("n", "integer").check(p.col("n") > 0, name='positive "x"'))
        await self.pool.execute(ddl)
        catalog = p.RawSQL(
            "SELECT conname::text AS name FROM pg_constraint "
            "WHERE conrelid = $1::text::regclass AND contype = 'c' ORDER BY conname",
            [quoted],
        )
        async with self.pool.connection() as connection:
            await connection.execute(s.rename_constraint(table, 'positive "x"', "above zero"))
            self.assertEqual([dict(row) for row in await connection.query(catalog)], [{"name": "above zero"}])
            with self.assertRaises(p.DatabaseError) as refused:
                await connection.execute(p.RawSQL(f"INSERT INTO {quoted} VALUES (-1)"))
            self.assertEqual(refused.exception.sqlstate, "23514")
            with self.assertRaises(p.DatabaseError) as refused:
                await connection.execute(s.alter_table(table).drop_constraint('positive "x"'))
            self.assertEqual(refused.exception.sqlstate, "42704")
            await connection.execute(s.alter_table(table).drop_constraint('positive "x"', if_exists=True))
            await connection.execute(s.alter_table(table).drop_constraint("above zero", behavior="cascade"))
            self.assertEqual(await connection.query(catalog), [])
            await connection.execute(p.RawSQL(f"INSERT INTO {quoted} VALUES (-1)"))

    # [spec:pgorm:req:bindings.schema-alter/test]
    async def test_check_not_valid_and_no_inherit(self):
        table = p.Table('validated "x"', schema=self.namespace)
        quoted = f'{self.quoted}."validated ""x"""'
        await self.pool.execute(s.create_table(table).column(s.ColumnDef("n", "integer")))
        await self.pool.execute(p.RawSQL(f"INSERT INTO {quoted} VALUES (-1)"))
        catalog = p.RawSQL(
            "SELECT conname::text AS name, convalidated AS valid, connoinherit AS no_inherit "
            "FROM pg_constraint WHERE conrelid = $1::text::regclass AND contype = 'c' ORDER BY conname",
            [quoted],
        )
        async with self.pool.connection() as connection:
            with self.assertRaises(p.DatabaseError) as refused:
                await connection.execute(s.alter_table(table).add_check(p.col("n") > 0, name="positive"))
            self.assertEqual(refused.exception.sqlstate, "23514")
            await connection.execute(s.alter_table(table).add_check(p.col("n") > 0, name="positive", no_inherit=True, not_valid=True))
            self.assertEqual([dict(row) for row in await connection.query(catalog)], [
                {"name": "positive", "valid": False, "no_inherit": True},
            ])
            with self.assertRaises(p.DatabaseError) as refused:
                await connection.execute(p.RawSQL(f"INSERT INTO {quoted} VALUES (-2)"))
            self.assertEqual(refused.exception.sqlstate, "23514")
            await connection.execute(p.RawSQL(f"UPDATE {quoted} SET n = 1"))
            await connection.execute(s.alter_table(table).validate_constraint("positive"))
            self.assertEqual([dict(row) for row in await connection.query(catalog)], [
                {"name": "positive", "valid": True, "no_inherit": True},
            ])

    # [spec:pgorm:req:bindings.schema-tables/test]
    async def test_temporal_keys(self):
        table = p.Table('booked "x"', schema=self.namespace)
        quoted = f'{self.quoted}."booked ""x"""'
        await self.pool.execute(p.RawSQL(f"CREATE EXTENSION btree_gist SCHEMA {self.quoted}"))
        ddl = s.create_table(table).column(s.ColumnDef("room", "integer"))
        ddl = ddl.column(s.ColumnDef("during", p.TypeName("daterange")))
        await self.pool.execute(ddl.primary_key("room", without_overlaps="during"))
        await self.pool.execute(s.alter_table(table).add_unique("room", name="room period", nulls_not_distinct=True, without_overlaps="during"))
        catalog = p.RawSQL(
            "SELECT pg_get_constraintdef(oid) AS definition FROM pg_constraint "
            "WHERE conrelid = $1::text::regclass AND conperiod ORDER BY conname",
            [quoted],
        )
        rows = [row["definition"] for row in await self.pool.query(catalog)]
        self.assertEqual(rows, [
            "PRIMARY KEY (room, during WITHOUT OVERLAPS)",
            "UNIQUE NULLS NOT DISTINCT (room, during WITHOUT OVERLAPS)",
        ])
        await self.pool.execute(p.RawSQL(f"INSERT INTO {quoted} VALUES (1, '[2020-01-01,2020-02-01)'), (1, '[2020-02-01,2020-03-01)')"))
        with self.assertRaises(p.DatabaseError) as overlap:
            await self.pool.execute(p.RawSQL(f"INSERT INTO {quoted} VALUES (1, '[2020-01-15,2020-02-15)')"))
        self.assertEqual(overlap.exception.sqlstate, "23P01")

    # [spec:pgorm:req:bindings.schema-tables/test]
    async def test_not_enforced_constraints(self):
        table = p.Table('checked "x"', schema=self.namespace)
        quoted = f'{self.quoted}."checked ""x"""'
        ddl = s.create_table(table).column(s.ColumnDef("id", "integer").check(p.col("id") > 0, name="positive", enforcement="not_enforced"))
        await self.pool.execute(ddl.column(s.ColumnDef("parent", "integer")).check(p.col("id") < 100, name="small"))
        catalog = p.RawSQL(
            "SELECT conname::text AS name, conenforced AS enforced, convalidated AS valid "
            "FROM pg_constraint WHERE conrelid = $1::text::regclass ORDER BY conname",
            [quoted],
        )
        async with self.pool.connection() as connection:
            await connection.execute(p.RawSQL(f"INSERT INTO {quoted} VALUES (-1, 5)"))
            await connection.execute(s.alter_table(table).add_check(p.col("parent") < 0, name="negative parent", enforcement="not_enforced"))
            await connection.execute(p.RawSQL(f"CREATE UNIQUE INDEX ON {quoted} (id)"))
            await connection.execute(p.RawSQL(
                f"ALTER TABLE {quoted} ADD CONSTRAINT parent_key FOREIGN KEY (parent) REFERENCES {quoted} (id) NOT ENFORCED"
            ))
            with self.assertRaises(p.DatabaseError) as orphan:
                await connection.execute(s.alter_table(table).alter_constraint("parent_key", "enforced"))
            self.assertEqual(orphan.exception.sqlstate, "23503")
            # 19 enforces a CHECK as it does a foreign key, and the row already
            # there breaks it; 18 cannot alter a CHECK's enforcement.
            with self.assertRaises(p.DatabaseError) as check:
                await connection.execute(s.alter_table(table).alter_constraint("positive", "enforced"))
            self.assertEqual(check.exception.sqlstate, "23514" if PG_19 else "42809")
            rows = [dict(row) for row in await connection.query(catalog)]
            self.assertEqual(rows, [
                {"name": "negative parent", "enforced": False, "valid": False},
                {"name": "parent_key", "enforced": False, "valid": False},
                {"name": "positive", "enforced": False, "valid": False},
                {"name": "small", "enforced": True, "valid": True},
            ])

    # [spec:pgorm:req:bindings.schema-types/test]
    async def test_qualified_enum_arrays_and_label_escaping(self):
        # A same-named public type must not capture a qualified column reference.
        shadow_name = 'Mood shadow ' + uuid4().hex[:8]
        shadow = p.TypeName(shadow_name, schema="public")
        kind = p.TypeName(shadow_name, schema=self.namespace)
        await self.pool.execute(s.create_type(shadow).values(["wrong"]))
        try:
            values = ["", "O'Brien \\ 雪", "busy"]
            await self.pool.execute(s.create_type(kind).values(values))
            table = p.Table('enum "items"', schema=self.namespace)
            ddl = s.create_table(table).column(s.ColumnDef("mood", kind).default(p.Value(values[1], kind)))
            ddl = ddl.column(s.ColumnDef("moods", s.DataType(kind).array()))
            await self.pool.execute(ddl)
            query = p.insert(table).columns("moods").values(p.Value.array(kind, [values[0], None, values[2]])).returning(p.col("mood"), p.col("moods"))
            row = await self.pool.one(query)
            self.assertEqual(row["mood"], values[1])
            self.assertEqual(row["moods"], ["", None, "busy"])
            self.assertEqual(row.tagged("mood").type_name, kind)
            self.assertEqual(row.tagged("moods").element_type, kind)
            await self.pool.execute(s.alter_type(kind).add_value("first", before=""))
            await self.pool.execute(s.alter_type(kind).add_value("last", after="busy"))
            await self.pool.execute(s.alter_type(kind).rename_value("busy", "calm"))
            # Rust's driver caches type metadata on physical connections.
            # A schema migration finishes before application pools restart.
            await self.pool.close()
            self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
            changed = await self.pool.one(p.select(p.col("moods")).from_(table))
            self.assertEqual(changed["moods"], ["", None, "calm"])
            await self.pool.execute(s.alter_type(kind).rename_to('Renamed "雪"'))
            await self.pool.close()
            self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
            renamed = p.TypeName('Renamed "雪"', schema=self.namespace)
            row = await self.pool.one(p.select(p.col("mood")).from_(table))
            self.assertEqual(row.tagged("mood").type_name, renamed)
            await self.pool.execute(s.drop_table(table))
            await self.pool.execute(s.drop_type(renamed))
            await self.pool.execute(s.drop_type(renamed, if_exists=True))
        finally:
            await self.pool.execute(s.drop_type(shadow, if_exists=True, behavior="cascade"))

    # [spec:pgorm:req:bindings.collations/test]    a column created under "C" compares and sorts
    # bytewise, a retype changes its collation, and the server refuses what it cannot apply
    async def test_column_collation_orders_its_text(self):
        table = p.Table('words "x"', schema=self.namespace)
        ddl = s.create_table(table).column(s.ColumnDef("id", "integer").auto_increment()).primary_key("id")
        ddl = ddl.column(s.ColumnDef("word", "text").collate("C").not_null())
        await self.pool.execute(ddl.column(s.ColumnDef("label", "text").collate("POSIX", schema="pg_catalog")))
        for word in ("b", "a", "B", "A"):
            await self.pool.execute(p.insert(table).columns("word", "label").values(word, word))
        ordered = p.select(p.col("word")).from_(table).order_by(p.col("word").asc())
        self.assertEqual([row["word"] for row in await self.pool.query(ordered)], ["A", "B", "a", "b"])
        below = p.select(p.col("word")).from_(table).where(p.col("word") < "a").order_by(p.col("id").asc())
        self.assertEqual([row["word"] for row in await self.pool.query(below)], ["B", "A"])
        await self.pool.execute(s.alter_table(table).add_column(s.ColumnDef("note", "text").collate("C")))
        await self.pool.execute(s.alter_table(table).modify_column(s.ColumnDef("note", "varchar").collate("POSIX", schema="pg_catalog")))
        catalog = p.RawSQL(
            "SELECT attname::text AS column, collname::text AS collation FROM pg_attribute "
            "JOIN pg_collation c ON c.oid = attcollation "
            "WHERE attrelid = $1::text::regclass AND attnum > 0 ORDER BY attnum",
            [f'{self.quoted}."words ""x"""'],
        )
        rows = [(row["column"], row["collation"]) for row in await self.pool.query(catalog)]
        self.assertEqual(rows, [("word", "C"), ("label", "POSIX"), ("note", "POSIX")])
        for column, sqlstate in (
            (s.ColumnDef("missing", "text").collate("no such collation"), "42704"),
            (s.ColumnDef("n", "integer").collate("C"), "42804"),
        ):
            with self.subTest(sqlstate=sqlstate), self.assertRaises(p.DatabaseError) as refused:
                await self.pool.execute(s.alter_table(table).add_column(column))
            self.assertEqual(refused.exception.sqlstate, sqlstate)


if __name__ == "__main__":
    unittest.main()
