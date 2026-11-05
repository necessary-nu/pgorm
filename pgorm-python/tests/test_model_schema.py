"""A declared model's own DDL, written by pgorm's Schema from the TableShape it describes."""

from decimal import Decimal
import os
import unittest

import pgorm as p
from pgorm import schema as s


def bakery():
    return p.Model(
        "bakery",
        {
            "id": p.Column("i32", primary_key=True, auto_increment=True),
            "name": p.Column("text", column_type="varchar"),
        },
        schema="model_ddl",
    )


def cake(bakery_model):
    flavor = p.TypeName("flavor", schema="model_ddl")
    money = s.DataType("numeric", precision=10, scale=2)
    model = p.Model(
        "cake",
        {
            "id": p.Column("i32", primary_key=True, auto_increment=True),
            "name": p.Column("text", column_type=s.DataType("varchar", length=40), indexed=True),
            "flavor": p.Column(flavor, values=["lemon", "plain"], default_value="plain"),
            "price": p.Column("decimal", column_type=money, comment="in euros"),
            "doubled": p.Column("decimal", column_type=money, generated_stored=p.col("price") * 2),
            "code": p.Column("uuid", default_expr=p.call("uuidv7")),
            "tags": p.Column("text", array=True, column_type="varchar", nullable=True),
            "bakeryId": p.Column("i32"),
        },
        schema="model_ddl",
        comment="one row per cake",
        rename_all="snake_case",
        unique_keys=[{"fields": ("bakeryId", "name"), "name": "one_name_per_bakery"}],
    )
    return model, model.belongs_to(bakery_model, from_="bakeryId", to="id", on_delete="cascade")


class ModelSchemaConstruction(unittest.TestCase):
    # [spec:pgorm:req:bindings.schema-from-model/test]    every part of a declaration is written as an entity's
    # [spec:pgorm:req:bindings.relations+2/test]    a belongs_to relation's actions reach its foreign key
    def test_every_option_is_written(self):
        model, to_bakery = cake(bakery())
        schema = s.from_model(model, relations=[to_bakery])
        self.assertIsInstance(schema, s.EntitySchema)
        self.assertEqual(
            schema.table.inspect().sql,
            'CREATE TABLE "model_ddl"."cake" ( "id" serial NOT NULL, "name" varchar(40) NOT NULL, '
            '"flavor" model_ddl.flavor NOT NULL DEFAULT CAST(\'plain\' AS model_ddl.flavor), '
            '"price" decimal(10, 2) NOT NULL, '
            '"doubled" decimal(10, 2) NOT NULL GENERATED ALWAYS AS ("price" * 2) STORED, '
            '"code" uuid NOT NULL DEFAULT UUIDV7(), "tags" varchar[], '
            '"bakery_id" integer NOT NULL, PRIMARY KEY ("id"), '
            'CONSTRAINT "one_name_per_bakery" UNIQUE ("bakery_id", "name"), '
            'CONSTRAINT "fk-cake-bakery_id" FOREIGN KEY ("bakery_id") '
            'REFERENCES "model_ddl"."bakery" ("id") ON DELETE CASCADE )',
        )
        self.assertEqual(
            [statement.inspect().sql for statement in schema.enums],
            ["""CREATE TYPE "model_ddl"."flavor" AS ENUM ('lemon', 'plain')"""],
        )
        self.assertEqual(
            [statement.inspect().sql for statement in schema.indexes],
            ['CREATE INDEX "idx-cake-name" ON "model_ddl"."cake" ("name")'],
        )
        self.assertEqual(
            [statement.inspect().sql for statement in schema.comments],
            [
                """COMMENT ON TABLE "model_ddl"."cake" IS 'one row per cake'""",
                """COMMENT ON COLUMN "model_ddl"."cake"."price" IS 'in euros'""",
            ],
        )
        self.assertEqual(model.columns["bakeryId"].name, "bakery_id")
        self.assertEqual(model.comment, "one row per cake")
        self.assertEqual(model.describe()["fields"]["doubled"]["generated"], "always")
        self.assertEqual(to_bakery.on_delete, "cascade")
        self.assertIsNone(to_bakery.fk_name)
        self.assertEqual(
            p.capabilities()["operations"]["schema.from_model"]["registration_required"], False
        )

    # [spec:pgorm:req:bindings.schema-from-model/test]    keys: composite, identity, temporal, named foreign keys
    def test_keys(self):
        booking = p.Model(
            "booking",
            {
                "room": p.Column("i32", primary_key=True),
                "id": p.Column("i64", primary_key=True, generated="always"),
                "during": p.Column("tstzrange", primary_key=True, without_overlaps=True),
            },
        )
        self.assertEqual(
            s.from_model(booking).table.inspect().sql,
            'CREATE TABLE "booking" ( "room" integer NOT NULL, '
            '"id" bigint NOT NULL GENERATED ALWAYS AS IDENTITY, "during" tstzrange NOT NULL, '
            'CONSTRAINT "pk-booking" PRIMARY KEY ("room", "id", "during" WITHOUT OVERLAPS) )',
        )
        room = p.Model("room", {"id": p.Column("i32", primary_key=True)})
        named = booking.belongs_to(
            room, from_="room", to="id", on_update="restrict", fk_name="booking_room"
        )
        self.assertIn(
            'CONSTRAINT "booking_room" FOREIGN KEY ("room") REFERENCES "room" ("id") ON UPDATE RESTRICT',
            s.from_model(booking, relations=(named,)).table.inspect().sql,
        )
        keyless = p.Model("log", {"line": p.Column("text"), "at": p.Column("date", nullable=True)})
        self.assertEqual(
            s.from_model(keyless).table.inspect().sql,
            'CREATE TABLE "log" ( "line" text NOT NULL, "at" date )',
        )

    # [spec:pgorm:req:bindings.schema-from-model/test]    rename_all names columns by the derive's conversion
    def test_rename_all_is_the_derives(self):
        for style, column in [
            ("camelCase", "bakeMinutes"),
            ("PascalCase", "BakeMinutes"),
            ("kebab-case", "bake-minutes"),
            ("SCREAMING_SNAKE_CASE", "BAKE_MINUTES"),
            ("SCREAMING-KEBAB-CASE", "BAKE-MINUTES"),
            ("title_case", "Bake Minutes"),
            ("mixed_case", "bakeMinutes"),
        ]:
            with self.subTest(style=style):
                model = p.Model(
                    "recipe",
                    {"bake_minutes": p.Column("i32"), "kept": p.Column("text", name="as_is")},
                    rename_all=style,
                )
                self.assertEqual(model.columns["bake_minutes"].name, column)
                self.assertEqual(model.columns["kept"].name, "as_is")
        with self.assertRaises(TypeError):
            p.Model("recipe", {"id": p.Column("i32")}, rename_all="Camel")  # type: ignore[arg-type]

    # [spec:pgorm:req:bindings.schema-from-model/test]    what the options cannot say is refused as the model is declared
    def test_refusals(self):
        refused = [
            lambda: p.Column(p.TypeName("flavor", schema="public"), column_type="text"),
            lambda: p.Model("t", {"c": p.Column("text", array=True, column_type=s.DataType("text").array())}),
            lambda: p.Column("i32", default=True, default_value=1),
            lambda: p.Column("i32", default_value=1, default_expr=p.col("x")),
            lambda: p.Column("i32", generated="by_default", generated_stored=p.col("x")),
            lambda: p.Column("i32", primary_key=True, auto_increment=True, generated="always"),
            lambda: p.Column("text", primary_key=True, auto_increment=True),
            lambda: p.Column("i32", auto_increment=True),
            lambda: p.Column("i32", primary_key=True, without_overlaps=True),
            lambda: p.Column("i32", default_value=p.Value.null("i32")),
            lambda: p.Column("i32", default_value="one"),
            lambda: p.Model(
                "t",
                {
                    "a": p.Column("i32", primary_key=True, auto_increment=True),
                    "b": p.Column("i32", primary_key=True),
                },
            ),
            lambda: p.Model(
                "t",
                {
                    "a": p.Column("tstzrange", primary_key=True, without_overlaps=True),
                    "b": p.Column("i32", primary_key=True),
                },
            ),
            lambda: p.Model(
                "t", {"a": p.Column("tstzrange", primary_key=True, without_overlaps=True)}
            ),
            lambda: p.Model(
                "t",
                {"a": p.Column("i32", generated_virtual=p.col("b")), "b": p.Column("i32")},
                unique_keys=[{"fields": "a"}],
            ),
        ]
        for index, refusal in enumerate(refused):
            with self.subTest(index=index), self.assertRaises(p.ConstructionError):
                refusal()
        for wrong in [
            lambda: p.Column("i32", default_expr=1),  # type: ignore[arg-type]
            lambda: p.Column("i32", generated_stored="price"),  # type: ignore[arg-type]
            lambda: p.Column("i32", comment=3),  # type: ignore[arg-type]
            lambda: p.Model("t", {"a": p.Column("i32")}, comment=3),  # type: ignore[arg-type]
        ]:
            with self.assertRaises(TypeError):
                wrong()

    # [spec:pgorm:req:bindings.schema-from-model/test]    the schema refuses relations it holds no key of, and unstated defaults
    # [spec:pgorm:req:bindings.relations+2/test]    only belongs_to takes a foreign key's options
    def test_schema_refusals(self):
        model, to_bakery = cake(bakery())
        owner = bakery()
        cakes = owner.has_many(model, from_="id", to="bakeryId")
        with self.assertRaises(p.ConstructionError):
            s.from_model(model, relations=[cakes])
        with self.assertRaises(p.ConstructionError):
            s.from_model(owner, relations=[to_bakery])
        with self.assertRaises(p.ConstructionError):
            s.from_model(model.as_("c"))
        with self.assertRaises(TypeError):
            s.from_model(model, relations=[model])  # type: ignore[list-item]
        with self.assertRaises(TypeError):
            s.from_entity(model)  # type: ignore[arg-type]
        unstated = p.Model("t", {"id": p.Column("i32", primary_key=True, default=True)})
        with self.assertRaises(p.ConstructionError) as refusal:
            s.from_model(unstated)
        self.assertIn("has a default its description does not state", str(refusal.exception))
        with self.assertRaises(TypeError):
            owner.has_many(model, from_="id", to="bakeryId", on_delete="cascade")  # type: ignore[call-arg]
        with self.assertRaises(TypeError):
            model.belongs_to(owner, from_="bakeryId", to="id", on_delete="drop")  # type: ignore[arg-type]

    # [spec:pgorm:req:bindings.schema-from-model/test]    a disjoint model's tables take the same column lowering
    def test_disjoint_tables_take_column_types_and_defaults(self):
        base = p.Model(
            "vehicle",
            {
                "id": p.Column("i32", primary_key=True),
                "kind": p.Column(p.TypeName("vehicle_kind", schema="public"), values=["car"]),
                "label": p.Column("text", column_type="varchar", default_value="unnamed"),
            },
        )
        car = p.Model("car", {"id": p.Column("i32", primary_key=True), "doors": p.Column("i16")})
        disjoint = p.Disjoint("vehicle", base, {"car": car}, schema_name="public")
        tables = [statement.inspect().sql for statement in s.create_disjoint(disjoint)]
        self.assertIn(""""label" varchar NOT NULL DEFAULT 'unnamed'""", tables[1])


class ModelSchemaDatabase(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        await self.pool.execute(p.RawSQL("CREATE SCHEMA model_ddl"))

    async def asyncTearDown(self):
        await self.pool.execute(p.RawSQL("DROP SCHEMA model_ddl CASCADE"))
        await self.pool.close()

    # [spec:pgorm:req:bindings.schema-from-model/test]    the statements run, and the tables hold what the models declare
    async def test_a_models_schema_creates_its_table(self):
        owner = bakery()
        model, to_bakery = cake(owner)
        cake_schema = s.from_model(model, relations=[to_bakery])
        for statement in [
            *cake_schema.enums,
            s.from_model(owner).table,
            cake_schema.table,
            *cake_schema.indexes,
            *cake_schema.comments,
        ]:
            await self.pool.execute(statement)
        made = await owner.insert({"name": "Crumb"}).returning().one(self.pool)
        row = await model.insert(
            {"name": "Lemon Drizzle", "price": Decimal("3.50"), "bakeryId": made["id"]}
        ).returning().one(self.pool)
        self.assertEqual((row["flavor"], row["doubled"]), ("plain", Decimal("7.00")))
        self.assertEqual(row["code"].version, 7)
        with self.assertRaises(p.ForeignKeyViolationError):
            await model.insert({"name": "Orphan", "price": Decimal(1), "bakeryId": 99}).execute(self.pool)
        await owner.delete().all_rows().execute(self.pool)
        self.assertEqual(await model.find().count(self.pool), 0)
        comment = await self.pool.one(
            p.RawSQL("SELECT obj_description('model_ddl.cake'::regclass, 'pg_class') AS c")
        )
        self.assertEqual(comment["c"], "one row per cake")


if __name__ == "__main__":
    unittest.main()
