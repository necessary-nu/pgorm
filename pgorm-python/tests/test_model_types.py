"""A declared model's columns read, written and compared through pgorm's casts
for their types, and converted through the program's own functions."""

from decimal import Decimal
import enum
import os
from typing import Any
import unittest

import pgorm as p
from pgorm import pipeline as pl
from pgorm import schema as s

FLAVOR = p.TypeName("flavor", schema="model_types")
FANCY = p.TypeName("fancy", schema="model_types")


class Shelf(enum.IntEnum):
    TOP = 1
    BOTTOM = 2


def tip_jar():
    return p.Model("tip_jar", {
        "id": p.Column("i32", primary_key=True, auto_increment=True),
        "amount": p.Column("decimal", column_type="money"),
    }, schema="model_types")


def special():
    return p.Model("special", {
        "id": p.Column("i32", primary_key=True, auto_increment=True),
        "flavor": p.Column(FLAVOR, values=["lemon", "plain"], column_type=s.DataType.domain(FANCY, FLAVOR)),
    }, schema="model_types")


def opening():
    bits = s.DataType("bit", length=7)
    return p.Model("opening", {
        "bakery_id": p.Column("i32", primary_key=True),
        "days": p.Column("i64", column_type=bits, select_as="bigint", save_as=bits),
    }, schema="model_types")


def shelving():
    return p.Model("shelving", {
        "id": p.Column("i32", primary_key=True),
        "shelf": p.Column("i16", decode=Shelf, encode=int),
        "contact": p.Column("json", nullable=True, decode=lambda doc: (doc["name"], doc["phone"]),
                            encode=lambda pair: {"name": pair[0], "phone": pair[1]}),
    }, schema="model_types")


class ColumnCastConstruction(unittest.TestCase):
    # [spec:pgorm:req:bindings.model-casts/test]    a money column reads as numeric and writes and compares as money
    def test_money(self):
        tips = tip_jar()
        self.assertEqual(
            tips.find().where(tips.col("amount") > Decimal(10)).inspect().sql,
            'SELECT "tip_jar"."id", CAST("tip_jar"."amount" AS numeric) AS "amount" '
            'FROM "model_types"."tip_jar" WHERE "tip_jar"."amount" > CAST($1::numeric AS money)',
        )
        self.assertEqual(
            tips.insert({"amount": Decimal("1.5")}).inspect().sql,
            'INSERT INTO "model_types"."tip_jar" ("amount") VALUES (CAST($1::numeric AS money))',
        )
        self.assertIn('"amount" money NOT NULL', s.from_model(tips).table.inspect().sql)

    # [spec:pgorm:req:bindings.model-casts/test]    a domain over an enum is read as its text and compared as the enum
    def test_domain_over_enum(self):
        specials = special()
        self.assertEqual(
            specials.find().where(specials.col("flavor") == "lemon").inspect().sql,
            'SELECT "special"."id", CAST("special"."flavor" AS text) AS "flavor" '
            'FROM "model_types"."special" WHERE CAST("special"."flavor" AS model_types.flavor) '
            '= CAST($1::text AS model_types.flavor)',
        )
        self.assertEqual(
            specials.insert({"flavor": "plain"}).inspect().sql,
            'INSERT INTO "model_types"."special" ("flavor") VALUES (CAST($1::text AS model_types.fancy))',
        )

    # [spec:pgorm:req:bindings.model-casts/test]    select_as and save_as cast to a type with modifiers
    def test_casts_to_modified_types(self):
        days = opening()
        self.assertEqual(
            days.find().inspect().sql,
            'SELECT "opening"."bakery_id", CAST("opening"."days" AS bigint) AS "days" '
            'FROM "model_types"."opening"',
        )
        self.assertEqual(
            days.insert({"bakery_id": 1, "days": 0b1111110}).inspect().sql,
            'INSERT INTO "model_types"."opening" ("bakery_id", "days") VALUES ($1, CAST($2::int8 AS bit(7)))',
        )
        read = p.Model("t", {"c": p.Column("text", select_with=lambda c: p.call("upper", c))})
        self.assertEqual(read.find().inspect().sql, 'SELECT UPPER("t"."c") AS "c" FROM "t"')

    # [spec:pgorm:req:bindings.model-casts/test]    an interval restricted to fields
    def test_interval_fields(self):
        slot = p.Model("slot", {
            "span": p.Column("interval", column_type=s.DataType("interval", fields="hour_to_minute")),
            "exact": p.Column("interval", column_type=s.DataType("interval", fields="day_to_second", precision=3)),
        })
        self.assertEqual(
            s.from_model(slot).table.inspect().sql,
            'CREATE TABLE "slot" ( "span" interval HOUR TO MINUTE NOT NULL, '
            '"exact" interval DAY TO SECOND(3) NOT NULL )',
        )
        with self.assertRaises(p.ConstructionError):
            s.DataType("interval", fields="hour_to_minute", precision=3)
        with self.assertRaises(TypeError):
            s.DataType("interval", fields="fortnight")  # type: ignore[arg-type]

    # [spec:pgorm:req:bindings.model-casts/test]    what the casts cannot say is refused as the model is declared
    def test_refusals(self):
        refused = [
            lambda: p.Column(FLAVOR, column_type="text"),
            lambda: p.Column(FLAVOR, column_type=s.DataType.domain(FANCY, "integer")),
            lambda: p.Column("text", select_as="text", select_with=lambda c: c),
            lambda: p.Column("text", save_as="text", save_with=lambda v: v),
            lambda: p.Column("interval", select_as=s.DataType("interval", fields="hour")),
        ]
        for index, refusal in enumerate(refused):
            with self.subTest(index=index), self.assertRaises(p.ConstructionError):
                refusal()
        with self.assertRaises(TypeError):
            p.Column("text", select_with="upper")  # type: ignore[arg-type]
        with self.assertRaises(TypeError):
            p.Column("text", decode=3)  # type: ignore[arg-type]

    # [spec:pgorm:req:bindings.model-array-casts/test]    a list any_ and all_ range over is written as the column writes its array
    def test_quantified_lists_take_the_columns_array_cast(self):
        tips, specials = tip_jar(), special()
        self.assertTrue(
            tips.find().where(tips.col("amount") == p.any_([Decimal(1), Decimal(2)])).inspect().sql
            .endswith('WHERE "tip_jar"."amount" = ANY(CAST($1::numeric[] AS money[]))')
        )
        self.assertTrue(
            specials.find().where(specials.col("flavor") != p.all_(["plain"])).inspect().sql
            .endswith(
                'WHERE CAST("special"."flavor" AS model_types.flavor) '
                '<> ALL(CAST($1::text[] AS model_types.flavor[]))'
            )
        )
        coded = p.Model("coded", {"code": p.Column("text", save_as="varchar")})
        self.assertTrue(
            coded.find().where(coded.col("code") == p.any_(["a"])).inspect().sql
            .endswith('WHERE "coded"."code" = ANY(CAST($1::text[] AS varchar[]))')
        )
        upper = p.Model("upper", {"code": p.Column(
            "text",
            save_with=lambda value: p.call("upper", value),
            save_array_with=lambda array: array.cast(p.TypeName("citext"), array=True),
        )})
        self.assertTrue(
            upper.find().where(upper.col("code") == p.any_(["a"])).inspect().sql
            .endswith('WHERE "upper"."code" = ANY(CAST($1::text[] AS citext[]))')
        )
        plain = p.Model("plain", {"code": p.Column("text")})
        self.assertTrue(
            plain.find().where(plain.col("code") == p.any_(["a"])).inspect().sql
            .endswith('WHERE "plain"."code" = ANY($1)')
        )
        for refused in (
            lambda: p.Column("text", save_with=lambda value: value),
            lambda: p.Column("text", save_array_with=lambda array: array),
            lambda: p.Column("text", save_as="varchar", save_array_with=lambda array: array),
        ):
            with self.subTest(refused=refused), self.assertRaises(p.ConstructionError):
                refused()
        with self.assertRaises(TypeError):
            p.Column("text", save_with=lambda v: v, save_array_with="upper")  # type: ignore[arg-type]
        no_expression: Any = lambda array: "upper"
        bad = p.Model("bad", {"code": p.Column(
            "text", save_with=lambda value: value, save_array_with=no_expression,
        )})
        with self.assertRaises(TypeError):
            bad.find().where(bad.col("code") == p.any_(["a"]))

    # [spec:pgorm:req:bindings.pipeline-models+1/test]    a cast column compares in a pipeline as the model's own comparisons do, and its typed reads read it cast
    def test_pipeline_reads_and_compares_through_casts(self):
        tips, specials, days = tip_jar(), special(), opening()
        compared = pl.from_(tips).filter(pl.col(tips, "amount") > Decimal(10))
        self.assertEqual(
            compared.inspect().sql,
            "SELECT * FROM model_types.tip_jar WHERE amount > CAST($1::numeric AS money)",
        )
        self.assertEqual(
            compared.into_model(tips).inspect().sql,
            "SELECT id, CAST(amount AS numeric) AS amount FROM model_types.tip_jar "
            "WHERE amount > CAST($1::numeric AS money)",
        )
        self.assertEqual(
            pl.from_(specials).filter(pl.col(specials, "flavor") == "lemon").select_sources(specials)
            .inspect().sql,
            "SELECT id AS s0_id, CAST(flavor AS text) AS s0_flavor FROM model_types.special "
            "WHERE CAST(flavor AS model_types.flavor) = CAST($1::text AS model_types.flavor)",
        )
        self.assertEqual(
            pl.from_(days).into_model(days).inspect().sql,
            "SELECT bakery_id, CAST(days AS bigint) AS days FROM model_types.opening",
        )
        plain = shelving()
        self.assertEqual(pl.from_(plain).into_model(plain).inspect().sql, "SELECT * FROM model_types.shelving")

    # [spec:pgorm:req:bindings.model-conversions/test]    encode converts what a write and a comparison take
    def test_encode(self):
        shelves = shelving()
        query = shelves.find().where(shelves.col("shelf") == Shelf.TOP).inspect()
        self.assertEqual([value.value for value in query.params], [1])


class ColumnCastDatabase(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        await self.pool.execute(p.RawSQL("CREATE SCHEMA model_types"))
        await self.pool.execute(s.create_type(FLAVOR).values(["lemon", "plain"]))
        await self.pool.execute(s.create_domain(FANCY, FLAVOR))
        for model in (tip_jar(), special(), opening(), shelving()):
            await self.pool.execute(s.from_model(model).table)

    async def asyncTearDown(self):
        await self.pool.execute(p.RawSQL("DROP SCHEMA model_types CASCADE"))
        await self.pool.close()

    # [spec:pgorm:req:bindings.model-casts/test]    the casts run: money rounds, a domain over an enum compares, bit(7) round-trips
    async def test_casts_run(self):
        tips, specials, days = tip_jar(), special(), opening()
        tip = await tips.insert({"amount": Decimal("12.345")}).returning().one(self.pool)
        self.assertEqual(tip["amount"], Decimal("12.35"))
        self.assertEqual(await tips.find().where(tips.col("amount") > Decimal(10)).count(self.pool), 1)
        await specials.insert_many([{"flavor": "lemon"}, {"flavor": "plain"}]).execute(self.pool)
        lemons = await specials.find().where(specials.col("flavor") == "lemon").all(self.pool)
        self.assertEqual([row["flavor"] for row in lemons], ["lemon"])
        self.assertEqual(lemons[0].tagged("flavor").type_name, FLAVOR)
        week = await days.insert({"bakery_id": 1, "days": 0b1111110}).returning().one(self.pool)
        self.assertEqual(week["days"], 0b1111110)
        cursor = days.find().cursor("days").after(0b0000001)
        self.assertEqual([row["bakery_id"] for row in await cursor.all(self.pool)], [1])

    # [spec:pgorm:req:bindings.model-array-casts/test]    a list compared with a money or domain-over-enum column runs
    # [spec:pgorm:req:bindings.pipeline-models+1/test]    a pipeline over cast columns compares and reads them as the model does
    async def test_cast_columns_run_in_lists_and_pipelines(self):
        tips, specials = tip_jar(), special()
        await tips.insert_many([{"amount": Decimal("12.50")}, {"amount": Decimal("3")}]).execute(self.pool)
        await specials.insert_many([{"flavor": "lemon"}, {"flavor": "plain"}]).execute(self.pool)
        listed = tips.find().where(tips.col("amount") == p.any_([Decimal("3"), Decimal("4")]))
        self.assertEqual([row["amount"] for row in await listed.all(self.pool)], [Decimal("3.00")])
        lemons = specials.find().where(specials.col("flavor") == p.any_(["lemon"]))
        self.assertEqual([row["flavor"] for row in await lemons.all(self.pool)], ["lemon"])
        big = pl.from_(tips).filter(pl.col(tips, "amount") > Decimal(10))
        self.assertEqual([row["amount"] for row in await big.into_model(tips).all(self.pool)], [Decimal("12.50")])
        [(tip,)] = await big.select_sources(tips).all(self.pool)
        self.assertEqual(tip["amount"], Decimal("12.50"))
        lemon = pl.from_(specials).filter(pl.col(specials, "flavor") == "lemon")
        self.assertEqual([row["flavor"] for row in await lemon.into_model(specials).all(self.pool)], ["lemon"])
        self.assertEqual((await lemon.into_model(specials).one(self.pool))["flavor"], "lemon")
        # The pipeline's own terminals read the columns as the server sends them.
        with self.assertRaises(p.DecodeError):
            await pl.from_(tips).all(self.pool)

    # [spec:pgorm:req:bindings.model-conversions/test]    decode reads the program's type and refuses what it cannot read
    async def test_conversions_run(self):
        shelves = shelving()
        row = await shelves.insert(
            {"id": 1, "shelf": Shelf.BOTTOM, "contact": ("Kim", "555")}
        ).returning().one(self.pool)
        self.assertIs(row["shelf"], Shelf.BOTTOM)
        self.assertEqual(row["contact"], ("Kim", "555"))
        await shelves.insert({"id": 2, "shelf": Shelf.TOP, "contact": None}).execute(self.pool)
        tops = await shelves.find().where(shelves.col("shelf") == Shelf.TOP).all(self.pool)
        self.assertEqual([(row["id"], row["contact"]) for row in tops], [(2, None)])
        await self.pool.execute(p.RawSQL(
            "INSERT INTO model_types.shelving (id, shelf, contact) VALUES (3, 9, '{\"name\": \"x\"}')"
        ))
        with self.assertRaises(p.DecodeError):
            await shelves.find().where(shelves.col("id") == 3).one(self.pool)


if __name__ == "__main__":
    unittest.main()
