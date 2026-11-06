"""Runtime Python model descriptors executed through installed native builders."""

from dataclasses import FrozenInstanceError
import datetime as dt
from decimal import Decimal
import os
import unittest
from unittest.mock import patch
from uuid import UUID

import pgorm as p


def accounts():
    return p.Model(
        'accounts "log',
        {
            "id": p.Column("i32", primary_key=True, generated="by_default"),
            "display_name": p.Column("text", name="display name", default=True),
            "note": p.Column("text", nullable=True),
            "mood": p.Column(
                p.TypeName("Mood", schema="python_models"),
                default=True,
                values=["calm", "busy"],
            ),
            "data": p.Column("json", nullable=True),
            "tags": p.Column("text", array=True, nullable=True),
        },
        schema="python_models",
    )


class ModelConstruction(unittest.TestCase):
    # [spec:pgorm:req:bindings.models+5/test]
    def test_descriptor_names_and_values_are_owned(self):
        from pgorm.models.columns import MODEL_KINDS

        self.assertEqual(
            set(p.capabilities()["model_policy"]["scalar_kinds"]), MODEL_KINDS
        )
        for operation in (
            "model.declare",
            "model.column",
            "model.select",
            "model.insert",
            "model.update",
            "model.delete",
            "model.records",
            "model.relation",
            "model.graph",
            "model.cursor",
            "model.paginate",
        ):
            p.require_capability(operation)
        fields = {"identity": p.Column("i32", name='id"雪', primary_key=True)}
        model = p.Model('t"表', fields, schema='s"文')
        fields.clear()
        self.assertEqual(model.primary_key, ("identity",))
        self.assertEqual(
            model.find().inspect().sql,
            'SELECT "t""表"."id""雪" AS "identity" FROM "s""文"."t""表"',
        )
        with self.assertRaises(TypeError):
            model.columns["other"] = p.Column("text")
        with self.assertRaises(FrozenInstanceError):
            model.table = p.Table("other")
        data = {"id": 1, "display_name": "Nora", "mood": "calm", "tags": ["a"]}
        write = accounts().insert(data)
        data["tags"].append("changed")
        self.assertEqual(write.inspect().params[-1].value, ["a"])
        self.assertEqual(write.inspect().params[0].kind, "i32")

    # [spec:pgorm:req:bindings.models+5/test]
    def test_describe_reports_each_fields_declaration(self):
        model = p.Model(
            "accounts",
            {
                "id": p.Column("i64", primary_key=True, generated="always"),
                "display_name": p.Column("text", name="display name", default=True),
                "mood": p.Column(p.TypeName("Mood", schema="app"), nullable=True, values=["calm"]),
                "tags": p.Column("text", array=True, nullable=True),
                "span": p.Column(p.CreatedRange("floatrange", "f64", schema="app")),
            },
            schema="app",
        ).as_("a")
        described = model.describe()
        self.assertEqual(
            {key: described[key] for key in ("schema", "table", "alias", "primary_key")},
            {"schema": "app", "table": "accounts", "alias": "a", "primary_key": ["id"]},
        )
        self.assertEqual(described["fields"], {
            "id": {"column": "id", "kind": "i64", "nullable": False, "primary_key": True,
                   "default": False, "generated": "always", "values": None},
            "display_name": {"column": "display name", "kind": "text", "nullable": False,
                             "primary_key": False, "default": True, "generated": None, "values": None},
            "mood": {"column": "mood", "kind": 'enum "app"."Mood"',
                     "nullable": True, "primary_key": False, "default": False, "generated": None,
                     "values": ["calm"]},
            "tags": {"column": "tags", "kind": "text[]",
                     "nullable": True, "primary_key": False, "default": False, "generated": None,
                     "values": None},
            "span": {"column": "span", "kind": 'range "app"."floatrange" of f64',
                     "nullable": False, "primary_key": False, "default": False, "generated": None,
                     "values": None},
        })
        self.assertEqual(
            set(described), {"schema", "table", "alias", "primary_key", "unique_keys", "hooks", "fields"}
        )
        self.assertEqual(described["unique_keys"], [])
        self.assertEqual(described["hooks"], [])
        for name in ("describe", "validate", "value"):
            with self.subTest(method=name):
                self.assertFalse(hasattr(model.columns["id"], name))
        self.assertFalse(hasattr(model.col("id"), "describe"))

    # [spec:pgorm:req:bindings.model-unique-keys+2/test]    a model declares its unique keys in order, one field or several, named or not and NULLS NOT DISTINCT or not, describes each and keeps them under an alias, sending nothing
    def test_unique_keys_are_declared_and_described(self):
        columns = {
            "id": p.Column("i32", primary_key=True),
            "handle": p.Column("text", name="user handle"),
            "realm": p.Column("text", nullable=True),
        }
        model = p.Model(
            "members",
            columns,
            unique_keys=[
                {"fields": ("realm", "handle"), "name": "members_realm_handle", "nulls_not_distinct": True},
                {"fields": "handle"},
            ],
        )
        expected = [
            {"name": "members_realm_handle", "fields": ["realm", "handle"], "nulls_not_distinct": True},
            {"name": None, "fields": ["handle"], "nulls_not_distinct": False},
        ]
        self.assertEqual(model.describe()["unique_keys"], expected)
        self.assertEqual(model.as_("m").describe()["unique_keys"], expected)
        self.assertEqual(p.Model("members", columns).describe()["unique_keys"], [])

    # [spec:pgorm:req:bindings.model-unique-keys+2/test]    a unique key naming no field, a field the model lacks or a field twice, or a name that is no identifier, is a ConstructionError as the model is declared; a key of another type, an unknown option and a mistyped field, name or flag a TypeError
    def test_unique_keys_refuse_what_postgres_refuses(self):
        columns = {"id": p.Column("i32", primary_key=True), "handle": p.Column("text")}

        def declare(*keys):
            return p.Model("members", columns, unique_keys=list(keys))

        for keys, message in (
            (({"fields": []},), "at least one field"),
            (({},), "at least one field"),
            (({"fields": ["handle", "missing"]},), "'missing', which is no field"),
            (({"fields": ["handle", "id", "handle"]},), "'handle' twice"),
            (({"fields": ["handle"], "name": ""},), "identifier"),
            (({"fields": ["handle"], "name": "k" * 64},), "identifier"),
        ):
            with self.subTest(keys=keys), self.assertRaises(p.ConstructionError) as raised:
                declare(*keys)
            self.assertIn(message, str(raised.exception))
        for operation in (
            lambda: p.Model("members", columns, unique_keys={"fields": "handle"}),
            lambda: declare(("handle", "id")),
            lambda: declare({"fields": ["handle"], "nulls_distinct": False}),
            lambda: declare({"fields": 7}),
            lambda: declare({"fields": [7]}),
            lambda: declare({"fields": ["handle"], "name": 7}),
            lambda: declare({"fields": ["handle"], "nulls_not_distinct": 1}),
        ):
            with self.assertRaises(TypeError):
                operation()

    # [spec:pgorm:req:bindings.models+5/test]
    def test_unknown_fields_and_collisions_fail_early(self):
        model = accounts()
        for operation in (
            lambda: model.col("missing"),
            lambda: model.select("id", "id"),
            lambda: model.insert({"missing": 1}),
            lambda: model.update({}),
            lambda: model.key({}),
            lambda: model.key({"id": 1, "note": "extra"}),
            lambda: p.Model("t", {}),
            lambda: p.Model(
                "t", {"a": p.Column("i32", name="x"), "b": p.Column("i32", name="x")}
            ),
            lambda: p.Column("i32", nullable=True, primary_key=True),
            lambda: p.Column(p.TypeName("Unqualified")),
            lambda: p.Column("i32", default=True, generated="always"),
            lambda: p.Column("text", values=["a"]),
            lambda: model.as_("a").insert({"id": 1}),
            lambda: model.insert({"note": "no mood"}).returning("note", "note"),
            lambda: p.Model("t", {"a": p.Column("i32", generated="always")}).insert({"a": 1}),
            lambda: p.Model("t", {"a": p.Column("i32"), "b": p.Column("i32")}).insert({"a": 1}),
            lambda: model.insert_many([{"note": "a"}, {"mood": "calm"}]),
            lambda: model.insert_many([{}, {}]),
            lambda: model.insert({"mood": "wild"}),
            lambda: model.col("mood").eq("wild"),
            lambda: model.col("note").eq(None),
        ):
            with self.assertRaises(p.ConstructionError):
                operation()
        for operation in (
            lambda: p.Column("i32", nullable=1),
            lambda: p.Column("i32", default="yes"),
            lambda: p.Column("i32", generated=True),
            lambda: p.Column(p.TypeName("Mood", schema="s"), values="calm"),
            lambda: p.Column("i32", invented=True),
            lambda: p.Column("i32", generated="sometimes"),
            lambda: p.Column(5),
            lambda: p.Model("t", [("id", p.Column("i32"))]),
            lambda: p.Model("t", {"id": "i32"}),
            lambda: model.key([1]),
            lambda: model.insert([("id", 1)]),
            lambda: p.ModelQuery(p.Insert(model.table)),
            lambda: p.ModelRows(model, "SELECT 1", ("id",)),
            lambda: p.ModelInsert(model, p.Select(p.literal(1)), ()),
            lambda: p.ModelInsertMany(model, p.Select(p.literal(1)), ()),
            lambda: p.ModelUpdate(model, p.Insert(model.table)),
            lambda: p.ModelDelete(model, p.Update(model.table)),
            lambda: model.insert(["id"]),
            lambda: model.insert_many({"id": 1}),
            lambda: model.find().where("id = 1"),
            lambda: model.find().order_by(model.col("id")),
        ):
            with self.assertRaises(TypeError):
                operation()
        for operation in (
            lambda: p.Column(p.TypeName("Mood", schema="s"), values=[]),
            lambda: p.Column(p.TypeName("Mood", schema="s"), values=["a", "a"]),
        ):
            with self.assertRaises(p.ConstructionError):
                operation()
        for kind in ("u64", "char", "made-up"):
            with (
                self.subTest(kind=kind),
                self.assertRaises(p.UnsupportedCapabilityError),
            ):
                p.Column(kind)
        for value in (True, 2**31, "1", None, p.Value(1, "i64")):
            with self.subTest(value=value), self.assertRaises(p.ConstructionError):
                model.insert({"id": value})
        with self.assertRaises(p.ConstructionError):
            model.insert({"mood": p.Value("calm", p.TypeName("Mood", schema="wrong"))})

    # [spec:pgorm:req:bindings.model-reads+1/test]
    def test_queries_keep_native_sql_and_parameter_tags(self):
        model = accounts().as_("a")
        query = (
            model.select("id", "display_name")
            .where(model.col("id") >= 2)
            .order_by(model.col("id").desc())
            .limit(3)
        )
        native = p.Select(
            p.col("id", table="a"), p.col("display name", table="a").as_("display_name")
        ).from_(model.table)
        native = (
            native.where(p.col("id", table="a") >= p.Value(2, "i32"))
            .order_by(p.col("id", table="a").desc())
            .limit(3)
        )
        self.assertEqual(query.inspect().sql, native.inspect().sql)
        self.assertEqual(query.inspect().params, native.inspect().params)
        self.assertNotIn("WHERE", model.find().inspect().sql)
        self.assertEqual(
            model.col("id").is_in([1, 2]).inspect().params,
            [p.Value(1, "i32"), p.Value(2, "i32")],
        )
        ids = p.Select(model.table.col("id")).from_(model.table)
        for method in ("is_in", "is_not_in"):
            self.assertEqual(
                getattr(model.col("id"), method)(ids).inspect().sql,
                getattr(model.table.col("id"), method)(ids).inspect().sql,
            )
        self.assertEqual(
            model.col("id").is_not_in((1,)).inspect().params, [p.Value(1, "i32")]
        )
        quantified = p.any_(p.Value.array("i32", [1, 2]))
        self.assertEqual(
            model.col("id").eq(quantified).inspect().sql,
            (model.table.col("id") == quantified).inspect().sql,
        )
        with self.assertRaises(TypeError):
            model.col("id").is_in("12")
        self.assertIn("IS NULL", model.col("note").is_null().inspect().sql)
        for value in (model.col("id"), query, accounts().delete()):
            with self.assertRaises(p.ConstructionError):
                bool(value)
        self.assertEqual(
            (model.col("id") == model.col("display_name")).inspect().sql,
            (p.col("id", table="a") == p.col("display name", table="a")).inspect().sql,
        )

    # [spec:pgorm:req:bindings.column-operators+3/test]
    def test_columns_are_expressions_converting_every_value_operator(self):
        model = accounts().as_("a")
        column, plain = model.col("id"), model.table.col("id")
        self.assertIsInstance(column, p.Expr)
        typed = p.Value(2, "i32"), p.Value(5, "i32")
        for method in (
            "is_distinct_from", "is_not_distinct_from", "contains", "contained_by",
            "overlaps", "strictly_left_of", "strictly_right_of", "not_extend_right_of",
            "not_extend_left_of", "adjacent_to",
        ):
            with self.subTest(method=method):
                built = getattr(column, method)(2).inspect()
                expected = getattr(plain, method)(typed[0]).inspect()
                self.assertEqual((built.sql, built.params), (expected.sql, expected.params))
        for method in ("between", "not_between"):
            for symmetric in (False, True):
                with self.subTest(method=method, symmetric=symmetric):
                    built = getattr(column, method)(2, 5, symmetric=symmetric).inspect()
                    expected = getattr(plain, method)(*typed, symmetric=symmetric).inspect()
                    self.assertEqual((built.sql, built.params), (expected.sql, expected.params))
        mood = model.col("mood").is_distinct_from("calm").inspect()
        self.assertIn('CAST($1::text AS python_models."Mood")', mood.sql)
        tags = model.col("tags").contains(["a"]).inspect()
        self.assertEqual(tags.params, [p.Value.array("text", ["a"])])
        for operation in (
            lambda: column.between(1, None),
            lambda: column.contains(p.Value(2, "i64")),
            lambda: column.adjacent_to("2"),
            lambda: column.is_distinct_from(p.any_(p.Value.array("i32", [1]))),
        ):
            with self.assertRaises(p.ConstructionError):
                operation()
        self.assertEqual(
            column.between(plain, model.col("id")).inspect().sql,
            plain.between(plain, plain).inspect().sql,
        )
        self.assertEqual((column + 1).inspect().params, (plain + 1).inspect().params)
        self.assertEqual(
            model.col("display_name").like(p.LikeExpr("N%")).inspect().sql,
            model.table.col("display name").like(p.LikeExpr("N%")).inspect().sql,
        )
        self.assertEqual(column.cast(p.TypeName("int8")).inspect().sql, plain.cast(p.TypeName("int8")).inspect().sql)
        self.assertEqual(
            p.Select(column.as_("key")).from_(model.table).inspect().sql,
            p.Select(plain.as_("key")).from_(model.table).inspect().sql,
        )
        self.assertEqual(type(column.expr()), p.Expr)
        self.assertEqual(
            (column.expr() == 1).inspect().params, (plain == 1).inspect().params
        )
        with self.assertRaises(p.ConstructionError):
            column == None  # noqa: E711
        with self.assertRaises(FrozenInstanceError):
            column.field_name = "other"
        with self.assertRaises(TypeError):
            p.ModelColumn("id", "i32", p.col("id"))
        with self.assertRaises(TypeError):
            p.ModelColumn("id", p.Column("i32"), "id")

    # [spec:pgorm:req:bindings.model-writes+3/test]
    def test_crud_preserves_omission_and_write_guards(self):
        model = accounts()
        omitted = model.insert({"id": 1}).inspect()
        explicit = model.insert({"id": 1, "note": None}).inspect()
        self.assertNotIn('"note"', omitted.sql)
        self.assertIn('"note"', explicit.sql)
        self.assertTrue(explicit.params[1].is_null)
        self.assertEqual(
            model.insert({}).inspect().sql,
            p.Insert(model.table).default_values().inspect().sql,
        )
        null_json = model.insert({"data": None}).inspect().params[0]
        json_null = model.insert({"data": p.Value.json(None)}).inspect().params[0]
        self.assertTrue(null_json.is_null)
        self.assertFalse(json_null.is_null)
        for query in (model.update({"note": None}), model.delete()):
            with self.assertRaises(p.ConstructionError):
                query.inspect()
            query.all_rows().inspect()
            query.where(model.key({"id": 1})).inspect()
        first = model.insert({"id": 1, "display_name": "A"}).inspect()
        second = model.insert({"display_name": "A", "id": 1}).inspect()
        self.assertEqual((first.sql, first.params), (second.sql, second.params))


class ModelDatabase(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        await self.pool.execute(p.RawSQL("CREATE SCHEMA python_models"))
        await self.pool.execute(
            p.RawSQL("CREATE TYPE python_models.\"Mood\" AS ENUM ('calm', 'busy')")
        )
        await self.pool.execute(
            p.RawSQL(
                'CREATE TABLE python_models."accounts ""log" (id integer GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, "display name" text NOT NULL DEFAULT \'default\', note text DEFAULT \'omitted\', mood python_models."Mood" NOT NULL DEFAULT \'calm\', data jsonb, tags text[])'
            )
        )
        self.model = accounts()

    async def asyncTearDown(self):
        await self.pool.execute(p.RawSQL("DROP SCHEMA python_models CASCADE"))
        await self.pool.close()

    # [spec:pgorm:req:bindings.model-writes+3/test]
    # [spec:pgorm:req:bindings.model-records+1/test]
    async def test_crud_returns_declared_field_records(self):
        with patch(
            "subprocess.run",
            side_effect=AssertionError("runtime descriptor compiled a module"),
        ):
            row = (
                await self.model.insert(
                    {"display_name": "Nora", "mood": "busy", "tags": ["a", None]}
                )
                .returning()
                .one(self.pool)
            )
            self.assertIsInstance(row, p.ModelRecord)
            self.assertEqual(row["display_name"], "Nora")
            self.assertEqual(row["note"], "omitted")
            self.assertEqual(row["tags"], ["a", None])
            self.assertEqual(
                row.tagged("mood").type_name, p.TypeName("Mood", schema="python_models")
            )
            self.assertEqual(row.tagged("tags").element_type, "text")
            key = self.model.key({"id": row["id"]})
            updated = (
                await self.model.update({"note": None})
                .where(key)
                .returning("id", "note")
                .one(self.pool)
            )
            self.assertEqual(dict(updated), {"id": row["id"], "note": None})
            selected = await self.model.find().where(key).one(self.pool)
            self.assertEqual(selected["display_name"], "Nora")
            self.assertEqual(
                await self.model.delete().where(key).execute(self.pool), 1
            )
            self.assertIsNone(await self.model.find().where(key).optional(self.pool))
        row["tags"].append("local mutation")
        self.assertEqual(row["tags"], ["a", None])

    # [spec:pgorm:req:bindings.column-operators+3/test]
    async def test_value_operators_bind_the_declared_kind(self):
        for name, mood, tags in (("A", "calm", ["a", "b"]), ("B", "busy", ["b"]), ("C", "busy", None)):
            await self.model.insert(
                {"display_name": name, "mood": mood, "tags": tags}
            ).execute(self.pool)

        async def names(predicate):
            query = self.model.select("display_name").where(predicate)
            rows = await query.order_by(self.model.col("id").asc()).all(self.pool)
            return [row["display_name"] for row in rows]

        column = self.model.col
        self.assertEqual(await names(column("id").between(2, 3)), ["B", "C"])
        self.assertEqual(await names(column("id").not_between(3, 1, symmetric=True)), [])
        self.assertEqual(await names(column("mood").is_distinct_from("calm")), ["B", "C"])
        self.assertEqual(await names(column("tags").is_not_distinct_from(["b"])), ["B"])
        self.assertEqual(await names(column("tags").contains(["b"])), ["A", "B"])
        self.assertEqual(await names(column("tags").contained_by(["b", "c"])), ["B"])
        self.assertEqual(await names(column("tags").overlaps(["a"])), ["A"])
        self.assertEqual(await names(column("display_name").like("_")), ["A", "B", "C"])
        self.assertEqual(await names(column("id") % 2 == 0), ["B"])

    # [spec:pgorm:req:bindings.model-columns+2/test]
    async def test_quantifiers_over_values_bind_the_declared_kind(self):
        for name, mood in (("A", "calm"), ("B", "busy"), ("C", "busy")):
            await self.model.insert({"display_name": name, "mood": mood}).execute(self.pool)

        async def names(predicate):
            query = self.model.select("display_name").where(predicate)
            rows = await query.order_by(self.model.col("id").asc()).all(self.pool)
            return [row["display_name"] for row in rows]

        column = self.model.col
        busy = column("mood") == p.any_(["busy"])
        self.assertIn('= ANY(CAST($1::text[] AS python_models."Mood"[]))', busy.inspect().sql)
        self.assertEqual(await names(busy), ["B", "C"])
        self.assertEqual(await names(column("mood") != p.all_(["busy"])), ["A"])
        self.assertEqual(await names(column("mood") == p.any_([])), [])
        self.assertEqual(await names(column("id") == p.any_([1, 3])), ["A", "C"])
        self.assertEqual((column("id") == p.any_([1])).inspect().params, [p.Value.array("i32", [1])])
        for refused in (
            lambda: column("mood") == p.any_(["wild"]),
            lambda: column("id") == p.any_([1, None]),
        ):
            with self.assertRaises(p.ConstructionError):
                refused()
        rows = p.Select(p.literal("calm").cast(p.TypeName("Mood", schema="python_models")))
        self.assertEqual(await names(column("mood") == p.any_(rows)), ["A"])

    # [spec:pgorm:req:bindings.model-columns+2/test]
    async def test_columns_compare_through_the_declared_kind(self):
        for name, mood in (("A", "calm"), ("B", "busy"), ("C", "busy")):
            await self.model.insert({"display_name": name, "mood": mood}).execute(self.pool)

        async def names(predicate):
            query = self.model.select("display_name").where(predicate)
            rows = await query.order_by(self.model.col("id").asc()).all(self.pool)
            return [row["display_name"] for row in rows]

        column = self.model.col
        aliased = self.model.as_("a")
        for model, qualifier in ((self.model, 'accounts "log'), (aliased, "a")):
            with self.subTest(qualifier=qualifier):
                qualified = p.col("display name", table=qualifier)
                self.assertEqual(
                    model.col("display_name").is_null().inspect().sql,
                    qualified.is_null().inspect().sql,
                )
                self.assertIn(qualified.inspect().sql.removeprefix("SELECT "), model.find().inspect().sql)
        self.assertEqual(await names(column("mood") == "busy"), ["B", "C"])
        self.assertEqual(await names(column("mood").ne("busy")), ["A"])
        self.assertIn('CAST($1::text AS python_models."Mood")', column("mood").eq("busy").inspect().sql)
        self.assertEqual(await names(column("id") < 2), ["A"])
        self.assertEqual(await names(column("id") <= 2), ["A", "B"])
        self.assertEqual(await names(column("id") > 2), ["C"])
        self.assertEqual(await names(column("id").gte(2)), ["B", "C"])
        self.assertEqual(column("id").lt(2).inspect().params, [p.Value(2, "i32")])
        self.assertEqual(await names(column("mood").is_in(["calm"])), ["A"])
        self.assertEqual(await names(column("mood").is_not_in(("calm",))), ["B", "C"])
        self.assertIn('CAST($1::text AS python_models."Mood")', column("mood").is_in(["calm"]).inspect().sql)
        self.assertEqual(await names(column("display_name") == p.any_(p.Value.array("text", ["A", "C"]))), ["A", "C"])
        self.assertEqual(await names(column("display_name") != p.all_(p.Value.array("text", ["A", "C"]))), ["B"])
        self.assertEqual(await names(column("id") == column("id")), ["A", "B", "C"])
        self.assertEqual(
            (column("id") == column("display_name")).inspect().sql,
            (p.col("id", table='accounts "log') == p.col("display name", table='accounts "log')).inspect().sql,
        )
        self.assertEqual(await names(column("id") == p.literal(2)), ["B"])
        self.assertEqual(await names(column("id") == p.Value(3, "i32")), ["C"])
        self.assertEqual(await names(column("note").is_null()), [])
        for refused in (
            lambda: column("id") == p.Value(3, "i64"),
            lambda: column("mood") == "wild",
            lambda: column("id") == None,  # noqa: E711
            lambda: column("id").is_in([1, None]),
        ):
            with self.assertRaises(p.ConstructionError):
                refused()

    # [spec:pgorm:req:bindings.model-writes+3/test]
    async def test_insert_row_count_types_and_batches(self):
        one = self.model.insert({"display_name": "A"})
        many = self.model.insert_many([{"display_name": "B"}])
        self.assertIs(type(one), p.ModelInsert)
        self.assertIs(type(many), p.ModelInsertMany)
        self.assertFalse(hasattr(one, "returning_upserts"))
        self.assertFalse(hasattr(many, "returning_upsert"))
        for write, terminal in ((one, "returning_upserts"), (many, "returning_upsert")):
            with self.subTest(terminal=terminal), self.assertRaises(TypeError):
                write.inspect(terminal=terminal)
        added = one.add({"display_name": "C"})
        self.assertIs(type(added), p.ModelInsertMany)
        self.assertEqual(
            added.inspect().sql,
            self.model.insert_many([{"display_name": "A"}, {"display_name": "C"}]).inspect().sql,
        )
        self.assertEqual(many.add_many([]).inspect().sql, many.inspect().sql)
        self.assertEqual(
            self.model.insert_many([]).add({"display_name": "D"}).inspect().sql,
            self.model.insert_many([{"display_name": "D"}]).inspect().sql,
        )
        with self.assertRaisesRegex(p.ConstructionError, "sets the same fields"):
            one.add({"display_name": "x", "note": "y"})
        with self.assertRaisesRegex(p.ConstructionError, "one at a time"):
            self.model.insert({}).add({})
        with self.assertRaises(TypeError):
            one.add_many({"display_name": "x"})
        renamed = p.Conflict.on("id").update("display name")
        kept = self.model.insert({"id": 1, "display_name": "A"}).on_conflict(renamed).add(
            {"id": 2, "display_name": "B"}
        )
        self.assertIn('ON CONFLICT ("id") DO UPDATE', kept.inspect().sql)
        written = await kept.returning_upserts(self.pool)
        self.assertEqual([type(row) for row in written], [p.Upserted.Inserted, p.Upserted.Inserted])
        again = self.model.insert({"id": 1, "display_name": "AA"}).on_conflict(renamed).add_many(
            [{"id": 3, "display_name": "C"}]
        )
        rows = await again.returning_upserts(self.pool)
        self.assertEqual(
            [
                (type(row), (row.model if hasattr(row, "model") else row.change.new)["display_name"])
                for row in rows
            ],
            [(p.Upserted.Updated, "AA"), (p.Upserted.Inserted, "C")],
        )
        self.assertEqual(
            await self.model.insert({"id": 10, "display_name": "E"}).add({"id": 11, "display_name": "F"}).execute(self.pool),
            2,
        )
        self.assertEqual(await self.model.find().count(self.pool), 5)

    # [spec:pgorm:req:bindings.model-writes+3/test]
    async def test_json_null_and_defaults_stay_distinct(self):
        omitted = await self.model.insert({}).returning().one(self.pool)
        sql_null = (
            await self.model.insert({"note": None, "data": None})
            .returning()
            .one(self.pool)
        )
        json_null = (
            await self.model.insert({"data": p.Value.json(None)})
            .returning()
            .one(self.pool)
        )
        self.assertEqual(omitted["note"], "omitted")
        self.assertIsNone(sql_null["note"])
        self.assertTrue(sql_null.tagged("data").is_null)
        self.assertFalse(json_null.tagged("data").is_null)
        self.assertIsNone(json_null["data"])
        with self.assertRaises(p.DecodeError):
            await self.model.find().optional(self.pool)
        self.assertEqual(len(await self.model.find().all(self.pool)), 3)
        self.assertEqual(
            len(
                await self.model.find()
                .order_by(self.model.col("id").asc())
                .offset(1)
                .limit(1)
                .all(self.pool)
            ),
            1,
        )

    # [spec:pgorm:req:bindings.model-records+1/test]
    async def test_declared_types_validate_decoded_columns(self):
        await self.model.insert({}).execute(self.pool)
        for columns in (
            {"id": p.Column("i64")},
            {"note": p.Column("i32")},
            {"data": p.Column("json")},
            {"mood": p.Column(p.TypeName("Mood", schema="wrong"))},
        ):
            declaration = p.Model(self.model.table, columns)
            with self.subTest(columns=columns), self.assertRaises(p.DecodeError):
                await declaration.find().one(self.pool)
        self.assertTrue(await self.pool.ping())
        record = await self.pool.one(p.Select(p.literal(1).as_("wrong")))
        with self.assertRaises(p.DecodeError):
            p.ModelRecord(self.model, record, ("id",))

    # [spec:pgorm:req:bindings.model-reads+1/test]
    async def test_alias_joins_and_composite_keys(self):
        for name in ("A", "B"):
            await self.model.insert({"display_name": name}).execute(self.pool)
        left, right = self.model.as_("a"), self.model.as_("b")
        same = left.has_one(right, from_="id", to="id")
        rows = (
            await left.select("display_name")
            .join(same)
            .order_by(left.col("id").asc())
            .all(self.pool)
        )
        self.assertEqual([row["display_name"] for row in rows], ["A", "B"])
        await self.pool.execute(
            p.RawSQL(
                "CREATE TABLE python_models.pairs (a integer, b text, PRIMARY KEY (a, b))"
            )
        )
        pairs = p.Model(
            "pairs",
            {
                "a": p.Column("i32", primary_key=True),
                "b": p.Column("text", primary_key=True),
            },
            schema="python_models",
        )
        await pairs.insert({"a": 1, "b": "first"}).execute(self.pool)
        await pairs.insert({"a": 1, "b": "second"}).execute(self.pool)
        row = (
            await pairs.find().where(pairs.key({"b": "second", "a": 1})).one(self.pool)
        )
        self.assertEqual(dict(row), {"a": 1, "b": "second"})

    # [spec:pgorm:req:bindings.model-records+1/test]
    async def test_declared_wire_kinds_keep_exact_tags(self):
        cases = [
            ("bool", "boolean", True),
            ("i8", 'pg_catalog."char"', 7),
            ("i16", "smallint", 8),
            ("i32", "integer", 9),
            ("i64", "bigint", 10),
            ("u32", "oid", 11),
            ("f32", "real", 1.5),
            ("f64", "double precision", 2.5),
            ("text", "text", "東京"),
            ("bytes", "bytea", b"\x00\xff"),
            ("decimal", "numeric", Decimal("123.4500")),
            ("uuid", "uuid", UUID("01234567-89ab-cdef-0123-456789abcdef")),
            ("json", "jsonb", {"ok": [True, None]}),
            ("date", "date", dt.date(2026, 9, 9)),
            ("time", "time", dt.time(12, 34, 56, 789)),
            ("datetime", "timestamp", dt.datetime(2026, 9, 9, 12, 34)),
            (
                "datetime_utc",
                "timestamptz",
                dt.datetime(2026, 9, 9, tzinfo=dt.timezone.utc),
            ),
            ("ipnetwork", "inet", "192.0.2.1/24"),
            ("mac_address", "macaddr", bytes.fromhex("001122334455")),
            ("int4range", "int4range", p.Range(1, 5)),
            ("numrange", "numrange", p.Range(Decimal("0.50"), None, "(]")),
            (
                "tstzmultirange",
                "tstzmultirange",
                p.Multirange([p.Range(dt.datetime(2026, 9, 9, tzinfo=dt.timezone.utc), None)]),
            ),
        ]
        ddl = ", ".join(f'"{kind}" {sql_type}' for kind, sql_type, _ in cases)
        await self.pool.execute(p.RawSQL(f"CREATE TABLE python_models.wire ({ddl})"))
        model = p.Model(
            "wire",
            {kind: p.Column(kind) for kind, _, _ in cases},
            schema="python_models",
        )
        row = (
            await model.insert({kind: value for kind, _, value in cases})
            .returning()
            .one(self.pool)
        )
        for kind, _, value in cases:
            with self.subTest(kind=kind):
                self.assertEqual(row.tagged(kind), p.Value(value, kind))
        await self.pool.execute(
            p.RawSQL('CREATE TABLE python_models.enums (moods python_models."Mood"[])')
        )
        mood = p.TypeName("Mood", schema="python_models")
        enums = p.Model(
            "enums",
            {"moods": p.Column(mood, array=True, nullable=True)},
            schema="python_models",
        )
        labels = ["calm", None, "busy"]
        row = await enums.insert({"moods": labels}).returning().one(self.pool)
        self.assertEqual(row.tagged("moods"), p.Value.array(mood, labels))
        with self.assertRaises(p.ConstructionError):
            enums.insert(
                {"moods": p.Value.array(p.TypeName("Mood", schema="wrong"), labels)}
            )

    # [spec:pgorm:req:bindings.model-records+1/test]
    # [spec:pgorm:req:bindings.created-ranges+5/test]
    async def test_created_range_columns_keep_their_type(self):
        await self.pool.execute(
            p.RawSQL("CREATE TYPE python_models.floatrange AS RANGE (SUBTYPE = float8)")
        )
        await self.pool.execute(
            p.RawSQL("CREATE TABLE python_models.spans (span python_models.floatrange)")
        )
        floatrange = p.CreatedRange("floatrange", "f64", schema="python_models")
        spans = p.Model("spans", {"span": p.Column(floatrange)}, schema="python_models")
        row = await spans.insert({"span": p.Range(1.5, None)}).returning().one(self.pool)
        self.assertEqual(row.tagged("span"), p.Value(p.Range(1.5, None), floatrange))
        found = await spans.find().where(spans.col("span") == p.Range(1.5, None)).one(self.pool)
        self.assertEqual(found["span"], p.Range(1.5, None))
        with self.assertRaises(p.ConstructionError):
            p.Column(p.CreatedRange("floatrange", "f64"))
        await self.pool.execute(
            p.RawSQL("CREATE TABLE python_models.shelves (id integer PRIMARY KEY, spans python_models.floatrange[])")
        )
        shelves = p.Model(
            "shelves",
            {"id": p.Column("i32", primary_key=True), "spans": p.Column(floatrange, array=True, nullable=True)},
            schema="python_models",
        )
        stocked = [p.Range(1.5, None), None, p.Range.empty()]
        row = await shelves.insert({"id": 1, "spans": stocked}).returning().one(self.pool)
        self.assertEqual(row["spans"], stocked)
        self.assertEqual(row.tagged("spans"), p.Value.array(floatrange, stocked))
        await shelves.insert({"id": 2, "spans": []}).execute(self.pool)
        await shelves.insert({"id": 3, "spans": None}).execute(self.pool)
        found = await shelves.find().where(shelves.col("spans") == []).one(self.pool)
        self.assertEqual(found["id"], 2)
        found = await shelves.find().where(shelves.col("spans") == stocked).one(self.pool)
        self.assertEqual(found["id"], 1)
        with self.assertRaises(p.ConstructionError):
            shelves.insert({"id": 4, "spans": [p.Range(1, 2)]})

    # [spec:pgorm:req:bindings.models+5/test]
    # [spec:pgorm:req:bindings.model-records+1/test]
    # [spec:pgorm:req:bindings.created-ranges+5/test]
    async def test_created_multirange_columns_read_and_write(self):
        await self.pool.execute(
            p.RawSQL(
                "CREATE TYPE python_models.floatrange AS RANGE "
                "(SUBTYPE = float8, MULTIRANGE_TYPE_NAME = python_models.floatmultirange)"
            )
        )
        await self.pool.execute(
            p.RawSQL(
                "CREATE TABLE python_models.slots "
                "(id integer PRIMARY KEY, open python_models.floatmultirange, "
                "week python_models.floatmultirange[])"
            )
        )
        kind = p.CreatedMultirange("floatmultirange", "f64", schema="python_models")
        slots = p.Model(
            "slots",
            {
                "id": p.Column("i32", primary_key=True),
                "open": p.Column(kind, nullable=True),
                "week": p.Column(kind, array=True, nullable=True),
            },
            schema="python_models",
        )
        hours = p.Multirange([p.Range(9.0, 12.5), p.Range(13.5, 17.0)])
        week = [hours, None, p.Multirange([])]
        written = await slots.insert({"id": 1, "open": hours, "week": week}).returning().one(self.pool)
        self.assertEqual(written["open"], hours)
        self.assertEqual(written.tagged("open"), p.Value(hours, kind))
        # An array of one is read cast to text[], each text read as the declared kind.
        self.assertEqual(written["week"], week)
        self.assertEqual(written.tagged("week"), p.Value.array(kind, week))
        await slots.insert({"id": 2, "open": None}).execute(self.pool)
        # The text is read through the declared subtype, so text that subtype
        # does not read is a DecodeError, never a value of another type.
        wrong = p.Model(
            "slots",
            {
                "id": p.Column("i32", primary_key=True),
                "open": p.Column(
                    p.CreatedMultirange("floatmultirange", "i64", schema="python_models"),
                    nullable=True,
                ),
            },
            schema="python_models",
        )
        with self.assertRaises(p.DecodeError):
            await wrong.find_by_key({"id": 1}).one(self.pool)
        found = await slots.find().where(slots.col("open") == hours).one(self.pool)
        self.assertEqual((found["id"], found["open"]), (1, hours))
        self.assertIsNone((await slots.find_by_key({"id": 2}).one(self.pool))["open"])
        later = p.Multirange([p.Range(18.0, None)])
        change = await (
            slots.update({"open": later}).where(slots.col("id") == 1).returning_change(self.pool)
        )
        self.assertEqual((change.old["open"], change.new["open"]), (hours, later))
        (row,) = await slots.graph().where(slots.col("id") == 1).all(self.pool)
        self.assertEqual(row["open"], later)
        self.assertEqual(
            slots.describe()["fields"]["open"]["kind"],
            'multirange "python_models"."floatmultirange" of f64',
        )
        self.assertEqual(
            slots.describe()["fields"]["week"]["kind"],
            'multirange "python_models"."floatmultirange" of f64[]',
        )
        with self.assertRaises(p.ConstructionError):
            p.Column(p.CreatedMultirange("floatmultirange", "f64"))
