"""Keys, foreign keys, identities, multi-action ALTER TABLE, index entries, types, sequences,
extensions and comments from the installed native wheel, against a live server."""
import os
import unittest
from uuid import uuid4

import pgorm as p
from pgorm import schema as s


class SchemaObjectConstruction(unittest.TestCase):
    # [spec:pgorm:req:bindings.schema/test]    every complete statement is a DDL the terminals
    # take, and an ALTER with no action yet is none, with no inspect()
    # [spec:pgorm:req:bindings.schema-api+3/test]
    def test_statements_are_ddl_pending_alters_not(self):
        statements = [
            s.create_table("t").column(s.ColumnDef("a", "integer")),
            s.alter_table("t").drop_column("a"),
            s.create_index("t", "a"),
            s.create_type("mood").values(["calm"]),
            s.alter_type("pair").drop_attribute("a"),
            s.create_sequence("ticket"),
            s.alter_sequence("ticket").restart(),
            s.alter_domain("d").drop_default(),
            s.create_schema("app"),
            s.create_view("v", p.select(p.col("a")).from_(p.Table("t"))),
        ]
        for statement in statements:
            with self.subTest(statement=statement):
                self.assertIsInstance(statement, s.DDL)
                self.assertEqual(statement.inspect().params, [])
        for pending in (s.alter_table("t"), s.alter_type("t"), s.alter_domain("d"), s.alter_sequence("s")):
            with self.subTest(pending=pending):
                self.assertNotIsInstance(pending, s.DDL)
                self.assertFalse(hasattr(pending, "inspect"))
        self.assertEqual(s.drop_table(["a", p.Table("b", schema="x")]).inspect().sql, 'DROP TABLE "a", "x"."b"')

    # [spec:pgorm:req:bindings.schema-api+3/test]    a statement is made by its factory, never by
    # its class, which is exported for annotations and isinstance alone
    def test_statements_are_made_by_their_factories(self):
        query = p.select(p.col("a")).from_(p.Table("t"))
        made = {
            s.CreateTable: (s.create_table, ("t",)),
            s.CreateIndex: (s.create_index, ("t", "a")),
            s.CreateType: (s.create_type, ("mood",)),
            s.CreateDomain: (s.create_domain, ("d", "text")),
            s.CreateSequence: (s.create_sequence, ("ticket",)),
            s.CreateView: (s.create_view, ("v", query)),
            s.CreateRecursiveView: (s.create_recursive_view, ("r", ["a"], query)),
            s.CreateMaterializedView: (s.create_materialized_view, ("m", query)),
            s.CreateTableAs: (s.create_table_as, ("c", query)),
        }
        for cls, (factory, arguments) in made.items():
            with self.subTest(cls=cls.__name__):
                self.assertNotIsInstance(factory, type)
                self.assertIs(type(factory(*arguments)), cls)
                with self.assertRaises(TypeError):
                    cls(*arguments)
        # A column and a type are parts of a statement, made by calling their classes.
        parts = {s.ColumnDef, s.DataType}
        classes = [getattr(s, name) for name in s.__all__ if isinstance(getattr(s, name), type)]
        self.assertTrue(set(made) | {s.DDL, s.AlterTable, s.PendingAlterTable} <= set(classes))
        for cls in classes:
            if cls not in parts:
                with self.subTest(cls=cls.__name__), self.assertRaises(TypeError):
                    cls()

    # [spec:pgorm:req:bindings.schema-tables/test]
    # [spec:pgorm:req:bindings.schema-alter/test]
    # [spec:pgorm:req:bindings.argument-errors+1/test]
    def test_impossible_shapes_refused_as_built(self):
        a = p.col("a")
        for action in (
            lambda: s.create_table("t").column(s.ColumnDef("a")),
            lambda: s.alter_table("t").add_column(s.ColumnDef("a")),
            lambda: s.create_table(p.Table("t", alias="x")),
            lambda: s.create_table("t").primary_key([]),
            lambda: s.create_table("t").foreign_key(["a", "b"], "u", ["x"]),
            lambda: s.alter_table("t").modify_column(s.ColumnDef("a", "integer").generated(p.col("b"), "stored")),
            lambda: s.alter_table("t").modify_column(s.ColumnDef("a", "integer").auto_increment()),
            lambda: s.alter_table("t").modify_column(s.ColumnDef("a").collate("C")),
            lambda: s.alter_table("t").modify_column(s.ColumnDef("a")),
            lambda: s.drop_table([]),
            lambda: s.ColumnDef("", "integer"),
        ):
            with self.subTest(action=action), self.assertRaises(p.ConstructionError):
                action()
        for action in (
            lambda: s.ColumnDef("a", "integer").not_null(nam="x"),
            lambda: s.create_table("t").primary_key("a", nulls_not_distinct=True),
            lambda: s.alter_table("t").add_check(a > 0, not_valid="yes"),
            lambda: s.create_table("t").check(a > 0, "named"),
            lambda: s.ColumnDef("a", "integer").identity("always", start=1),
            lambda: s.ColumnDef("a", "integer").identity("always", start_with=1.5),
            lambda: s.ColumnDef("a", "integer").identity("always", cycle=1),
            lambda: s.create_table(3),
            lambda: s.create_table("t").primary_key(3),
            lambda: s.create_index("t", "a").include("a"),
            lambda: s.create_type("t").as_range("double", collation={"nam": "C"}),
            lambda: s.create_table("t").foreign_key("a", "u", "x", period=("p",)),
            lambda: s.ColumnDef("a", "integer").generated(p.col("b"), "computed"),
            lambda: s.ColumnDef("a", "integer").identity("sometimes"),
            lambda: s.ColumnDef("a", "integer").check(a > 0, enforcement="off"),
            lambda: s.create_table("t").unique("a", deferrability="later"),
            lambda: s.create_table("t").foreign_key("a", "u", "x", on_delete="explode"),
            lambda: s.alter_table("t").alter_constraint("c", "valid"),
            lambda: s.alter_table("t").drop_constraint("c", behavior="maybe"),
        ):
            with self.subTest(action=action), self.assertRaises(TypeError):
                action()
        with self.assertRaises(p.ConstructionError):
            s.ColumnDef("a", "integer").identity("always", max_value=2**63)

    # [spec:pgorm:req:bindings.schema-types/test]
    # [spec:pgorm:req:bindings.schema-sequences/test]
    # [spec:pgorm:req:bindings.schema-indexes/test]
    def test_bad_labels_types_sequences_refused(self):
        for action in (
            lambda: s.create_type("t").values(["x" * 64]),
            lambda: s.create_type("t").values(["a\0b"]),
            lambda: s.alter_type("t").add_value("x", before="a", after="b"),
            lambda: s.alter_sequence("s").options(),
            lambda: s.create_sequence("s").owned_by("t"),
            lambda: s.create_sequence("s").owned_by(None, "id"),
            lambda: s.create_extension("e", version="1\0"),
            lambda: s.comment_on_table("t", "a\0b"),
            lambda: s.drop_type([]),
        ):
            with self.subTest(action=action), self.assertRaises(p.ConstructionError):
                action()
        for action in (
            lambda: s.create_type("t").values("abc"),
            lambda: s.create_type("t").as_range("double", canonical="f"),
            lambda: s.create_type(42),
            lambda: s.create_index("t", "a", nulls="last"),
            lambda: s.create_index("t", p.Table("x")),
            lambda: s.alter_sequence("s").restart("1"),
            lambda: s.comment_on_column("t", "c", 3),
            lambda: s.create_sequence("s").as_type("numeric"),
            lambda: s.create_index("t", "a", order="up"),
        ):
            with self.subTest(action=action), self.assertRaises(TypeError):
                action()
        for action in (
            lambda: s.alter_sequence("s").options(cache=2**63),
            lambda: s.alter_sequence("s").restart(-(2**63) - 1),
        ):
            with self.subTest(action=action), self.assertRaises(p.ConstructionError):
                action()


class SchemaObjectDatabase(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=2)
        self.ns = "objects_" + uuid4().hex[:8]
        await self.pool.execute(s.create_schema(self.ns))

    async def asyncTearDown(self):
        await self.pool.execute(s.drop_schema(self.ns, behavior="cascade"))
        await self.pool.close()

    def table(self, name):
        return p.Table(name, schema=self.ns)

    def type(self, name):
        return p.TypeName(name, schema=self.ns)

    async def sql(self, sql, values=()):
        return await self.pool.execute(p.RawSQL(sql, list(values)))

    async def rows(self, sql, values=()):
        return [tuple(row.values()) for row in await self.pool.query(p.RawSQL(sql, list(values)))]

    async def refused(self, statement, sqlstate):
        with self.assertRaises(p.DatabaseError) as refused:
            await self.pool.execute(statement)
        self.assertEqual(refused.exception.sqlstate, sqlstate, str(refused.exception))

    async def constraints(self, table):
        """Each constraint of `table` but a key column's implied NOT NULL, as (name, type,
        valid, enforced, no_inherit); no_inherit is a CHECK's or a NOT NULL's alone."""
        return await self.rows(
            "SELECT conname::text, contype::text, convalidated, conenforced, "
            "CASE WHEN contype IN ('c', 'n') THEN connoinherit END "
            "FROM pg_constraint c WHERE conrelid = $1::text::regclass AND NOT (contype = 'n' AND EXISTS ("
            "SELECT FROM pg_constraint k WHERE k.conrelid = c.conrelid AND k.contype = 'p' AND c.conkey <@ k.conkey)) "
            "ORDER BY conname",
            [f"{self.ns}.{table}"],
        )

    # [spec:pgorm:req:bindings.schema-tables/test]
    async def test_identity_defaults_generated_columns_hold(self):
        visit = self.table("visit")
        await self.pool.execute(
            s.create_table(visit)
            .column(s.ColumnDef("id", "bigint").identity("always", start_with=10, increment_by=5))
            .column(s.ColumnDef("seats", "integer").not_null().default(2))
            .column(s.ColumnDef("doubled", "integer").generated(p.col("seats") * 2, "stored"))
            .column(s.ColumnDef("note", "text").collate("C"))
            .primary_key("id")
        )
        await self.sql(f"INSERT INTO {self.ns}.visit (seats, note) VALUES (DEFAULT, 'b'), (7, 'B')")
        self.assertEqual(await self.rows(f"SELECT id, seats, doubled FROM {self.ns}.visit ORDER BY id"), [(10, 2, 4), (15, 7, 14)])
        await self.refused(p.RawSQL(f"INSERT INTO {self.ns}.visit (id, seats) VALUES (1, 1)"), "428C9")

    # [spec:pgorm:req:bindings.schema-tables/test]
    async def test_keys_with_include_deferrability_and_without_overlaps(self):
        await self.sql(f"CREATE EXTENSION btree_gist SCHEMA {self.ns}")
        room = self.table("room")
        await self.pool.execute(
            s.create_table(room)
            .column(s.ColumnDef("id", "integer")).column(s.ColumnDef("code", "text"))
            .column(s.ColumnDef("wing", "text")).column(s.ColumnDef("open", "daterange").not_null())
            .primary_key("code", name="replaced")
            .primary_key("id", name="room_key", without_overlaps="open", include=["wing"])
            .unique("code", nulls_not_distinct=True, name="room_code")
        )
        keys = [row for row in await self.constraints("room") if row[1] != "n"]
        self.assertEqual(keys, [("room_code", "u", True, True, None), ("room_key", "p", True, True, None)])
        await self.sql(f"INSERT INTO {self.ns}.room VALUES (1, NULL, 'east', '[2020-01-01,2020-02-01)')")
        await self.refused(p.RawSQL(f"INSERT INTO {self.ns}.room VALUES (1, 'c', 'east', '[2020-01-15,2020-02-15)')"), "23P01")
        await self.refused(p.RawSQL(f"INSERT INTO {self.ns}.room VALUES (2, NULL, 'west', '[2020-01-01,2020-02-01)')"), "23505")
        slot = self.table("slot")
        await self.pool.execute(
            s.create_table(slot).column(s.ColumnDef("n", "integer")).column(s.ColumnDef("m", "integer"))
            .unique("n", deferrability="deferrable_initially_deferred")
            .unique("m", deferrability="deferrable_initially_immediate")
        )
        await self.sql(f"INSERT INTO {self.ns}.slot VALUES (1, 1), (2, 2)")
        async with self.pool.transaction() as tx:
            await tx.execute(p.RawSQL(f"UPDATE {self.ns}.slot SET n = 2 WHERE n = 1"))
            await tx.execute(p.RawSQL(f"UPDATE {self.ns}.slot SET n = 1 WHERE m = 2"))
        await self.sql(f"UPDATE {self.ns}.slot SET m = m + 1")
        with self.assertRaises(p.DatabaseError) as deferred:
            async with self.pool.transaction() as tx:
                await tx.execute(p.RawSQL(f"INSERT INTO {self.ns}.slot VALUES (1, 9)"))
        self.assertEqual(deferred.exception.sqlstate, "23505")

    # [spec:pgorm:req:bindings.schema-tables/test]
    async def test_foreign_keys_actions_deferral_enforcement_and_period(self):
        await self.sql(f"CREATE EXTENSION btree_gist SCHEMA {self.ns}")
        owner = self.table("owner")
        await self.pool.execute(
            s.create_table(owner).column(s.ColumnDef("id", "integer")).column(s.ColumnDef("during", "daterange"))
            .primary_key("id", without_overlaps="during").unique("id", name="owner_id")
        )
        await self.pool.execute(
            s.create_table(self.table("pet"))
            .column(s.ColumnDef("owner", "integer")).column(s.ColumnDef("loose", "integer"))
            .column(s.ColumnDef("late", "integer")).column(s.ColumnDef("held", "daterange"))
            .foreign_key("owner", owner, "id", on_delete="cascade", name="pet_owner")
            .foreign_key("loose", owner, "id", enforcement="not_enforced", name="pet_loose")
            .foreign_key("late", owner, "id", deferrability="deferrable_initially_deferred", name="pet_late")
            .foreign_key(["owner"], owner, ["id"], period=("held", "during"), name="pet_period")
        )
        await self.sql(f"INSERT INTO {self.ns}.owner VALUES (1, '[2020-01-01,2020-02-01)')")
        await self.sql(f"INSERT INTO {self.ns}.pet VALUES (1, 99, NULL, '[2020-01-05,2020-01-10)')")
        await self.refused(p.RawSQL(f"INSERT INTO {self.ns}.pet VALUES (1, NULL, NULL, '[2020-01-20,2020-03-01)')"), "23503")
        async with self.pool.transaction() as tx:
            await tx.execute(p.RawSQL(f"INSERT INTO {self.ns}.pet VALUES (NULL, NULL, 2, NULL)"))
            await tx.execute(p.RawSQL(f"INSERT INTO {self.ns}.owner VALUES (2, '[2020-01-01,2020-02-01)')"))
        await self.sql(f"DELETE FROM {self.ns}.pet WHERE late = 2")
        await self.sql(f"DELETE FROM {self.ns}.owner WHERE id = 1")
        self.assertEqual(await self.rows(f"SELECT count(*) FROM {self.ns}.pet"), [(0,)])
        self.assertEqual([row for row in await self.constraints("pet") if row[1] == "f"], [
            ("pet_late", "f", True, True, None),
            ("pet_loose", "f", False, False, None),
            ("pet_owner", "f", True, True, None),
            ("pet_period", "f", True, True, None),
        ])

    # [spec:pgorm:req:bindings.schema-alter/test]
    async def test_not_valid_constraints_await_validation(self):
        await self.sql(f"CREATE TABLE {self.ns}.ledger (id integer PRIMARY KEY, amount integer, ref integer)")
        await self.sql(f"CREATE TABLE {self.ns}.account (id integer PRIMARY KEY)")
        await self.sql(f"INSERT INTO {self.ns}.ledger VALUES (1, -5, 7), (2, NULL, NULL)")
        ledger = self.table("ledger")
        await self.refused(s.alter_table(ledger).add_check(p.col("amount") >= 0), "23514")
        await self.pool.execute(
            s.alter_table(ledger)
            .add_check(p.col("amount") >= 0, name="amount_positive", not_valid=True)
            .add_not_null("amount", name="amount_present", not_valid=True)
            .add_foreign_key("ref", self.table("account"), "id", name="ledger_account", not_valid=True)
        )
        self.assertEqual(await self.constraints("ledger"), [
            ("amount_positive", "c", False, True, False),
            ("amount_present", "n", False, True, False),
            ("ledger_account", "f", False, True, None),
            ("ledger_pkey", "p", True, True, None),
        ])
        for row, sqlstate in (("3, -1, NULL", "23514"), ("3, NULL, NULL", "23502"), ("3, 1, 7", "23503")):
            await self.refused(p.RawSQL(f"INSERT INTO {self.ns}.ledger VALUES ({row})"), sqlstate)
        await self.refused(s.alter_table(ledger).validate_constraint("amount_positive"), "23514")
        await self.sql(f"UPDATE {self.ns}.ledger SET amount = 5, ref = NULL")
        await self.pool.execute(
            s.alter_table(ledger).validate_constraint("amount_positive").validate_constraint("amount_present")
            .validate_constraint("ledger_account")
        )
        self.assertTrue(all(row[2] for row in await self.constraints("ledger")))

    # [spec:pgorm:req:bindings.schema-alter/test]
    async def test_one_alter_table_adds_changes_and_drops(self):
        stock = self.table("stock")
        await self.pool.execute(
            s.create_table(stock).column(s.ColumnDef("id", "integer")).column(s.ColumnDef("code", "smallint"))
            .column(s.ColumnDef("twice", "integer").generated(p.col("id") * 2, "stored"))
            .column(s.ColumnDef("legacy", "text"))
        )
        await self.sql(f"INSERT INTO {self.ns}.stock (id, code) VALUES (1, 3)")
        await self.pool.execute(
            s.alter_table(stock)
            .add_column(s.ColumnDef("label", "text").default("none"))
            .add_column(s.ColumnDef("label", "text"), if_not_exists=True)
            .modify_column(s.ColumnDef("code", "bigint").not_null().default(1))
            .drop_column("legacy")
            .add_primary_key("id", name="stock_key")
            .set_expression("twice", p.col("id") * 4)
        )
        self.assertEqual(await self.rows(f"SELECT * FROM {self.ns}.stock"), [(1, 3, 4, "none")])
        await self.sql(f"INSERT INTO {self.ns}.stock (id) VALUES (2)")
        await self.pool.execute(s.alter_table(stock).drop_expression("twice").modify_column(s.ColumnDef("code").null()))
        await self.sql(f"INSERT INTO {self.ns}.stock (id, code, twice) VALUES (3, NULL, 5)")
        self.assertEqual(await self.rows(f"SELECT code, twice FROM {self.ns}.stock ORDER BY id"), [(3, 4), (1, 8), (None, 5)])
        await self.pool.execute(s.alter_table(stock).drop_constraint("stock_key").drop_constraint("stock_key", if_exists=True))
        self.assertEqual(await self.constraints("stock"), [("stock_id_not_null", "n", True, True, False)])

    # [spec:pgorm:req:bindings.schema-alter/test]
    async def test_alter_constraint_moves_enforcement_and_inheritance(self):
        await self.sql(f"CREATE TABLE {self.ns}.tag (id integer PRIMARY KEY)")
        await self.sql(f"CREATE TABLE {self.ns}.label (tag integer CONSTRAINT label_tag REFERENCES {self.ns}.tag, name text NOT NULL)")
        label = self.table("label")
        await self.pool.execute(s.alter_table(label).alter_constraint("label_tag", "not_enforced"))
        await self.sql(f"INSERT INTO {self.ns}.label VALUES (1, 'orphan')")
        await self.refused(s.alter_table(label).alter_constraint("label_tag", "enforced"), "23503")
        await self.sql(f"DELETE FROM {self.ns}.label")
        await self.pool.execute(
            s.alter_table(label).alter_constraint("label_tag", "enforced").alter_constraint("label_name_not_null", "no_inherit")
        )
        self.assertEqual(await self.constraints("label"), [
            ("label_name_not_null", "n", True, True, True),
            ("label_tag", "f", True, True, None),
        ])

    # [spec:pgorm:req:bindings.schema-tables/test]
    # [spec:pgorm:req:bindings.schema-sequences/test]
    async def test_tables_renamed_commented_emptied_and_dropped(self):
        await self.sql(f"CREATE TABLE {self.ns}.draft (id integer CONSTRAINT draft_key PRIMARY KEY, body text)")
        await self.sql(f"CREATE TABLE {self.ns}.ref (draft integer REFERENCES {self.ns}.draft)")
        draft = self.table("draft")
        await self.pool.execute(s.rename_column(draft, "body", "text"))
        await self.pool.execute(s.rename_constraint(draft, "draft_key", "draft_pk"))
        await self.pool.execute(s.rename_table(draft, "essay"))
        essay = self.table("essay")
        await self.pool.execute(s.comment_on_table(essay, "it's \\ written"))
        await self.pool.execute(s.comment_on_column(essay, "text", "line one\nline two"))
        described = await self.rows(
            "SELECT obj_description($1::text::regclass, 'pg_class'), col_description($1::text::regclass, 2)",
            [f"{self.ns}.essay"],
        )
        self.assertEqual(described, [("it's \\ written", "line one\nline two")])
        self.assertEqual(await self.constraints("essay"), [("draft_pk", "p", True, True, None)])
        await self.refused(s.truncate_table(essay), "0A000")
        await self.pool.execute(s.drop_table(self.table("ref")))
        await self.pool.execute(s.truncate_table(essay))
        await self.sql(f"CREATE TABLE {self.ns}.ref (essay integer REFERENCES {self.ns}.essay)")
        await self.refused(s.drop_table(essay), "2BP01")
        await self.pool.execute(s.drop_table([essay, "missing"], if_exists=True, behavior="cascade"))
        self.assertEqual(await self.constraints("ref"), [])

    # [spec:pgorm:req:bindings.schema/test]
    async def test_terminals_refuse_what_is_no_statement(self):
        pending = [
            (s.alter_table("anything"), "ALTER TABLE needs an action"),
            (s.alter_type("t"), "ALTER TYPE needs a change"),
            (s.alter_domain("d"), "ALTER DOMAIN needs an action"),
            (s.alter_sequence("s"), "ALTER SEQUENCE needs a clause"),
            (s.ColumnDef("a", "integer"), "ColumnDef is not a statement to run"),
        ]
        async with self.pool.connection() as connection:
            for value, message in pending:
                for terminal in (self.pool.execute, self.pool.query, connection.execute):
                    with self.subTest(value=value, terminal=terminal):
                        with self.assertRaisesRegex(p.ConstructionError, message):
                            await terminal(value)

    # [spec:pgorm:req:bindings.schema-indexes/test]
    async def test_index_entries_are_the_index_postgres_holds(self):
        person = self.table("person")
        await self.pool.execute(
            s.create_table(person).column(s.ColumnDef("id", "integer")).column(s.ColumnDef("email", "text"))
            .column(s.ColumnDef("active", "boolean")).column(s.ColumnDef("nick", "text"))
        )
        await self.pool.execute(
            s.create_index(person, p.call("lower", p.col("email")), name="person_email", operator_class="text_pattern_ops", order="desc")
            .column("id", order="asc").unique().include(["nick"]).where(p.col("active") == p.literal(True))
        )

        async def indexdef(name):
            row = await self.pool.optional(p.RawSQL("SELECT indexdef FROM pg_indexes WHERE schemaname = $1 AND indexname = $2", [self.ns, name]))
            return row and row["indexdef"]

        self.assertEqual(
            await indexdef("person_email"),
            f"CREATE UNIQUE INDEX person_email ON {self.ns}.person USING btree (lower(email) text_pattern_ops DESC, id) "
            "INCLUDE (nick) WHERE (active = true)",
        )
        await self.sql(f"INSERT INTO {self.ns}.person VALUES (1, 'A@x', true, 'a'), (1, 'a@X', false, 'b')")
        await self.refused(p.RawSQL(f"INSERT INTO {self.ns}.person VALUES (1, 'a@x', true, 'c')"), "23505")
        await self.pool.execute(s.create_index(person, "nick", name="person_nick").nulls_not_distinct().if_not_exists())
        await self.pool.execute(s.create_index(person, "nick", name="person_nick").if_not_exists())
        await self.sql(f"INSERT INTO {self.ns}.person VALUES (2, 'n@x', true, NULL)")
        await self.refused(p.RawSQL(f"INSERT INTO {self.ns}.person VALUES (3, 'm@x', true, NULL)"), "23505")
        await self.pool.execute(s.create_index(person, p.col("email"), name="person_hash").using("hash"))
        self.assertIn("USING hash (email)", await indexdef("person_hash"))
        await self.pool.execute(s.drop_index(person, "person_nick"))
        await self.pool.execute(s.drop_index(person, "person_nick", if_exists=True))
        self.assertIsNone(await indexdef("person_nick"))
        await self.refused(s.drop_index(person, "person_nick"), "42704")

    # [spec:pgorm:req:bindings.schema-types/test]
    async def test_enum_labels_are_data_in_order(self):
        mood = self.type('Mood "name"')
        await self.pool.execute(s.create_type(mood).values(["calm", "O'Brien", "", "back\\slash"]))
        await self.pool.execute(s.alter_type(mood).add_value("tense", before="calm"))
        await self.pool.execute(s.alter_type(mood).add_value("last"))
        await self.pool.execute(s.alter_type(mood).add_value("middle", after="O'Brien"))
        await self.pool.execute(s.alter_type(mood).rename_value("calm", "serene"))
        labels = await self.rows(
            "SELECT enumlabel::text FROM pg_enum WHERE enumtypid = $1::text::regtype ORDER BY enumsortorder",
            [f'{self.ns}."Mood ""name"""'],
        )
        self.assertEqual([label for (label,) in labels], ["tense", "serene", "O'Brien", "middle", "", "back\\slash", "last"])
        feeling = self.table("feeling")
        await self.pool.execute(s.create_table(feeling).column(s.ColumnDef("mood", mood).default(p.Value("middle", mood))))
        await self.sql(f"INSERT INTO {self.ns}.feeling DEFAULT VALUES")
        self.assertEqual(await self.rows(f"SELECT mood::text FROM {self.ns}.feeling"), [("middle",)])
        await self.refused(s.drop_type(mood), "2BP01")
        await self.pool.execute(s.alter_type(mood).rename_to("feeling_kind"))
        await self.pool.execute(s.drop_type(self.type("feeling_kind"), behavior="cascade"))
        self.assertEqual(await self.rows(
            "SELECT count(*) FROM pg_attribute WHERE attrelid = $1::text::regclass AND attnum > 0 AND NOT attisdropped",
            [f"{self.ns}.feeling"],
        ), [(0,)])
        await self.pool.execute(s.drop_type(["nothing_here", "nor_here"], if_exists=True))

    # [spec:pgorm:req:bindings.schema-types/test]
    async def test_composite_attributes_change_cascading_to_typed_tables(self):
        pair = self.type("pair")
        await self.pool.execute(
            s.create_type(pair).attribute("a", "integer")
            .attribute("b", "text", collation={"name": "C", "schema": "pg_catalog"})
        )
        await self.sql(f"CREATE TABLE {self.ns}.pairs OF {self.ns}.pair")
        await self.refused(s.alter_type(pair).add_attribute("c", "bigint"), "2BP01")
        await self.pool.execute(
            s.alter_type(pair).add_attribute("c", s.DataType("varchar", length=5)).alter_attribute("a", "bigint")
            .drop_attribute("b").drop_attribute("missing", if_exists=True).behavior("cascade")
        )
        await self.pool.execute(s.alter_type(pair).rename_attribute("c", "label").behavior("cascade"))
        attributes = await self.rows(
            "SELECT attname::text, format_type(atttypid, atttypmod) FROM pg_attribute "
            "WHERE attrelid = $1::text::regclass AND attnum > 0 AND NOT attisdropped ORDER BY attnum",
            [f"{self.ns}.pair"],
        )
        self.assertEqual(attributes, [("a", "bigint"), ("label", "character varying(5)")])
        columns = await self.rows(
            "SELECT column_name::text FROM information_schema.columns WHERE table_schema = $1 "
            "AND table_name = 'pairs' ORDER BY ordinal_position",
            [self.ns],
        )
        self.assertEqual(columns, [("a",), ("label",)])
        self.assertEqual(await self.rows(f"SELECT (ROW(2, 'y')::{self.ns}.pair).label"), [("y",)])

    # [spec:pgorm:req:bindings.schema-types/test]
    async def test_range_type_with_subtype_and_multirange(self):
        await self.pool.execute(
            s.create_type(self.type("floatrange")).as_range(
                "double", subtype_diff="float8mi", multirange_type_name=self.type("floats"),
            )
        )
        found = await self.rows(
            "SELECT format_type(rngsubtype, NULL), rngsubdiff::text, rngmultitypid::regtype::text "
            "FROM pg_range WHERE rngtypid = $1::text::regtype",
            [f"{self.ns}.floatrange"],
        )
        self.assertEqual(found, [("double precision", "float8mi", f"{self.ns}.floats")])
        contained = await self.rows(
            f"SELECT {self.ns}.floatrange(1.5, 2.5) @> 2.0::float8, {self.ns}.floats({self.ns}.floatrange(1, 2))::text"
        )
        self.assertEqual(contained, [(True, "{[1,2)}")])
        await self.pool.execute(s.create_type(self.type("bare")))
        self.assertEqual(await self.rows(
            "SELECT typtype::text FROM pg_type WHERE typname = 'bare' AND typnamespace = $1::text::regnamespace", [self.ns],
        ), [("p",)])
        await self.pool.execute(s.drop_type([self.type("floatrange"), self.type("bare")]))

    # [spec:pgorm:req:bindings.schema-sequences/test]
    async def test_sequence_values_restart_and_ownership(self):
        ticket = self.table("ticket")
        await self.pool.execute(
            s.create_sequence(ticket).as_type("smallint").options(start_with=10, increment_by=-3, min_value=1, cycle=True)
            .options(max_value=10)
        )

        async def next_value(name="ticket"):
            return (await self.rows("SELECT nextval($1::text::regclass)", [f"{self.ns}.{name}"]))[0][0]

        self.assertEqual([await next_value() for _ in range(5)], [10, 7, 4, 1, 10])
        await self.pool.execute(s.alter_sequence(ticket).restart(4).options(increment_by=1, cycle=False))
        self.assertEqual([await next_value(), await next_value()], [4, 5])
        await self.pool.execute(s.alter_sequence(ticket).restart())
        self.assertEqual(await next_value(), 10)
        with self.assertRaises(p.DatabaseError) as exhausted:
            await next_value()
        self.assertEqual(exhausted.exception.sqlstate, "2200H")
        bounds = await self.rows(
            "SELECT data_type::text, maximum_value::text FROM information_schema.sequences "
            "WHERE sequence_schema = $1 AND sequence_name = 'ticket'",
            [self.ns],
        )
        self.assertEqual(bounds, [("smallint", "10")])
        await self.refused(s.alter_sequence(ticket).options(increment_by=0), "22023")
        await self.sql(f"CREATE TABLE {self.ns}.counter (id bigint)")
        await self.pool.execute(s.alter_sequence(ticket).owned_by(self.table("counter"), "id").if_exists())
        await self.pool.execute(s.rename_sequence(ticket, "voucher"))
        await self.sql(f"DROP TABLE {self.ns}.counter")
        self.assertEqual(await self.rows(
            "SELECT 1 FROM pg_class WHERE relname = 'voucher' AND relnamespace = $1::text::regnamespace", [self.ns],
        ), [])
        spare = self.table("spare")
        await self.pool.execute(s.create_sequence(spare).if_not_exists().options(max_value=9_007_199_254_740_993))
        await self.pool.execute(s.create_sequence(spare).if_not_exists())
        self.assertEqual(await self.rows(
            "SELECT seqmax FROM pg_sequence WHERE seqrelid = $1::text::regclass", [f"{self.ns}.spare"],
        ), [(9_007_199_254_740_993,)])
        await self.pool.execute(s.drop_sequence([spare, self.table("absent")], if_exists=True))
        await self.refused(s.drop_sequence(spare), "42P01")

    # [spec:pgorm:req:bindings.schema-sequences/test]
    async def test_identity_sequence_takes_sequence_options(self):
        numbered = self.table("numbered")
        await self.pool.execute(
            s.create_table(numbered)
            .column(s.ColumnDef("n", "bigint").identity("by_default", start_with=100, increment_by=10, cache=1))
            .column(s.ColumnDef("x", "integer"))
        )
        await self.sql(f"INSERT INTO {self.ns}.numbered (x) VALUES (1), (2)")
        await self.sql(f"INSERT INTO {self.ns}.numbered (n, x) VALUES (5, 3)")
        self.assertEqual(await self.rows(f"SELECT n FROM {self.ns}.numbered ORDER BY x"), [(100,), (110,), (5,)])

    # [spec:pgorm:req:bindings.schema-sequences/test]
    async def test_extension_schema_and_version_hold(self):
        await self.pool.execute(s.create_extension("btree_gist", schema=self.ns, version="1.7"))
        await self.pool.execute(s.create_extension("btree_gist", if_not_exists=True))
        installed = await self.rows(
            "SELECT extversion::text, extnamespace::regnamespace::text FROM pg_extension WHERE extname = 'btree_gist'"
        )
        self.assertEqual(installed, [("1.7", self.ns)])
        await self.refused(s.create_extension("btree_gist"), "42710")
        await self.pool.execute(s.drop_extension("btree_gist", behavior="cascade"))
        await self.pool.execute(s.drop_extension("btree_gist", if_exists=True))
        await self.refused(s.create_extension("no_such_extension"), "0A000")


if __name__ == "__main__":
    unittest.main()
