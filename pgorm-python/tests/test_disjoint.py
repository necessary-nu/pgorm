"""Disjoint models declared over runtime models: declaration and its refusals,
the statements pgorm's shape builds, and the schema, reads and writes on a
live server."""

import os
from typing import Any, cast
import unittest
from unittest.mock import patch

import pgorm as p
import pgorm.schema as ps

SCHEMA = "python_disjoint"


def kind_type():
    return p.TypeName("vehicle_kind", schema=SCHEMA)


def vehicle(**kind):
    return p.Model(
        "vehicle",
        {
            "id": p.Column("i32", primary_key=True, generated="by_default"),
            "kind": p.Column(kind_type(), values=["car", "truck", "bike"], **kind),
            "name": p.Column("text"),
        },
        schema=SCHEMA,
    )


def car(**key):
    return p.Model(
        "car",
        {
            "id": p.Column("i32", primary_key=True, **key),
            "seats": p.Column("i16"),
            "color": p.Column(
                p.TypeName("color", schema=SCHEMA), nullable=True, values=["red", "blue"]
            ),
        },
        schema=SCHEMA,
    )


def truck():
    return p.Model(
        "truck",
        {
            "vehicle_id": p.Column("i32", primary_key=True),
            "axles": p.Column("i16"),
            "payload_kg": p.Column("i32", name="payload"),
        },
        schema=SCHEMA,
    )


def vehicles(base=None, **kinds):
    return p.Disjoint(
        "vehicle",
        vehicle() if base is None else base,
        {"car": kinds.get("car", car()), "truck": truck(), "bike": None},
        schema_name=SCHEMA,
    )


READ = (
    'SELECT "vehicle"."id" AS "s0_id", CAST("vehicle"."kind" AS text) AS "s0_kind", '
    '"vehicle"."name" AS "s0_name", "car"."id" AS "s1_id", "car"."seats" AS "s1_seats", '
    'CAST("car"."color" AS text) AS "s1_color", "truck"."vehicle_id" AS "s2_vehicle_id", '
    '"truck"."axles" AS "s2_axles", "truck"."payload" AS "s2_payload" '
    'FROM "python_disjoint"."vehicle" '
    'LEFT JOIN "python_disjoint"."car" ON "vehicle"."id" = "car"."id" '
    'LEFT JOIN "python_disjoint"."truck" ON "vehicle"."id" = "truck"."vehicle_id"'
)


class DisjointDeclaration(unittest.TestCase):
    # [spec:pgorm:req:bindings.disjoint+2/test]    a disjoint model declared over models sends nothing, describes itself under the declared keys, and builds the statements pgorm's shape builds: the read, its narrowing by key, kind, kinds and a kind's condition, and an insert
    def test_declaration_builds_the_shape_statements(self):
        for operation in (
            "disjoint.declare",
            "disjoint.find",
            "disjoint.insert",
            "disjoint.update",
            "disjoint.change_kind",
            "disjoint.schema",
        ):
            p.require_capability(operation)
        with patch("socket.socket", side_effect=AssertionError("a declaration connected")):
            model = vehicles()
        self.assertEqual(
            model.describe(),
            {
                "name": "vehicle",
                "base": "vehicle",
                "kind_field": "kind",
                "kind_type": {"schema": SCHEMA, "name": "vehicle_kind"},
                "kinds": [
                    {"value": "car", "model": "car"},
                    {"value": "truck", "model": "truck"},
                    {"value": "bike", "model": None},
                ],
            },
        )
        self.assertEqual(model.find().inspect().sql, READ)
        self.assertEqual(
            model.find().inspect(terminal="one").sql, READ + " LIMIT $1"
        )
        keyed = model.find_by_key({"id": 7}).inspect()
        self.assertEqual(keyed.sql, READ + ' WHERE "vehicle"."id" = $1')
        self.assertEqual([value.value for value in keyed.params], [7])
        narrowed = model.find().kind("car").inspect().sql
        self.assertTrue(
            narrowed.endswith(
                'WHERE "vehicle"."kind" = CAST($1::text AS python_disjoint.vehicle_kind)'
            ),
            narrowed,
        )
        either = model.find().kinds(["truck", "bike"]).inspect().sql
        self.assertTrue(
            either.endswith(
                'WHERE "vehicle"."kind" IN (CAST($1::text AS python_disjoint.vehicle_kind), '
                "CAST($2::text AS python_disjoint.vehicle_kind))"
            ),
            either,
        )
        roomy = model.find().filter_kind("car", model.kinds["car"].col("seats") >= 5).inspect()
        self.assertTrue(
            roomy.sql.endswith(
                'WHERE "vehicle"."kind" = CAST($1::text AS python_disjoint.vehicle_kind) '
                'AND "car"."seats" >= $2'
            ),
            roomy.sql,
        )
        inserted = model.insert({"name": "mini"}, "car", {"seats": 4}).inspect().sql
        self.assertEqual(
            inserted,
            'WITH "vehicle" AS (INSERT INTO "python_disjoint"."vehicle" ("kind", "name") '
            "VALUES (CAST($1::text AS python_disjoint.vehicle_kind), $2) "
            'RETURNING "vehicle"."id", "vehicle"."kind", "vehicle"."name") , '
            '"car" AS (INSERT INTO "python_disjoint"."car" ("id", "seats") '
            'SELECT "vehicle"."id", $3 FROM "vehicle" '
            'RETURNING "car"."id", "car"."seats", "car"."color") '
            + READ.replace('FROM "python_disjoint"."vehicle"', 'FROM "vehicle"').replace(
                'LEFT JOIN "python_disjoint"."car"', 'LEFT JOIN "car"'
            ),
        )

    # [spec:pgorm:req:bindings.disjoint+2/test]    what the Rust declaration refuses is a ConstructionError raised as the model is declared: no kinds, one model for two kinds, a kind model keyed by other types, a kind field missing, nullable, generated or of another type, and a kind model whose key the database fills; an argument of the wrong type is a TypeError
    def test_declaration_refuses_what_rust_refuses(self):
        refused = {
            "no kinds": lambda: p.Disjoint("vehicle", vehicle(), {}, schema_name=SCHEMA),
            "two kinds one model": lambda: p.Disjoint(
                "vehicle",
                vehicle(),
                {"car": car(), "truck": car(), "bike": None},
                schema_name=SCHEMA,
            ),
            "key types": lambda: vehicles(
                car=p.Model(
                    "car", {"id": p.Column("i64", primary_key=True)}, schema=SCHEMA
                )
            ),
            "missing kind field": lambda: p.Disjoint(
                "vehicle", vehicle(), {"car": car()}, kind="category", schema_name=SCHEMA
            ),
            "nullable kind field": lambda: vehicles(vehicle(nullable=True)),
            "generated kind field": lambda: vehicles(vehicle(generated="always")),
            "another type": lambda: p.Disjoint(
                "vehicle", vehicle(), {"car": car(), "truck": truck(), "bike": None}
            ),
            "values unlisted": lambda: p.Disjoint(
                "vehicle",
                vehicle(),
                {"car": car(), "boat": None, "bike": None},
                schema_name=SCHEMA,
            ),
            "generated kind key": lambda: vehicles(car=car(generated="by_default")),
            "defaulted kind key": lambda: vehicles(car=car(default=True)),
            "aliased base": lambda: vehicles(vehicle().as_("v")),
        }
        for case, declare in refused.items():
            with self.subTest(case=case), self.assertRaises(p.ConstructionError):
                declare()
        messages = {
            "nullable kind field": "the kind column `python_disjoint.vehicle`.`kind` is nullable",
            "another type": "is `python_disjoint.vehicle_kind` ('car', 'truck', 'bike'), not "
            "the kind type `vehicle_kind` ('car', 'truck', 'bike')",
            "generated kind key": "`python_disjoint.car`.`id`, the key of the kind `car`, is an "
            "identity",
        }
        for case, message in messages.items():
            with self.subTest(case=case), self.assertRaises(p.ConstructionError) as raised:
                refused[case]()
            self.assertIn(message, str(raised.exception))
        for wrong in (
            lambda: p.Disjoint("vehicle", "vehicle", {"bike": None}),
            lambda: p.Disjoint("vehicle", vehicle(), [("bike", None)]),
            lambda: p.Disjoint("vehicle", vehicle(), {1: None}),
            lambda: p.Disjoint(1, vehicle(), {"bike": None}),
            lambda: p.Disjoint("vehicle", vehicle(), {"bike": "car"}),
        ):
            with self.subTest(wrong=wrong), self.assertRaises(TypeError):
                wrong()

    # [spec:pgorm:req:bindings.model-unique-keys+2/test]    a model's unique key named as a name the disjoint schema adds, as one PostgreSQL gives the tables itself, or as another model's unique key, is a ConstructionError raised as the disjoint model is declared, naming both
    def test_declaration_refuses_schema_named_keys(self):
        def keyed(model, name, fields):
            return p.Model(
                model.table.name, dict(model.columns), schema=SCHEMA,
                unique_keys=[{"fields": fields, "name": name}],
            )

        cases = {
            "the base's UNIQUE": (keyed(vehicle(), "uq-vehicle-id-kind", ["name", "kind"]), car()),
            "a kind's foreign key": (vehicle(), keyed(car(), "fk-car-id-kind", ["seats", "id"])),
            "another model's key": (
                keyed(vehicle(), "shared", ["name", "kind"]), keyed(car(), "shared", ["seats", "id"])
            ),
            "PostgreSQL's primary key": (keyed(vehicle(), "vehicle_pkey", ["name", "kind"]), car()),
            "PostgreSQL's not-null": (vehicle(), keyed(car(), "car_seats_not_null", ["seats", "id"])),
        }
        for case, (base, kind) in cases.items():
            with self.subTest(case=case), self.assertRaises(p.ConstructionError) as raised:
                vehicles(base, car=kind)
            self.assertIn("is named as", str(raised.exception))

    # [spec:pgorm:req:bindings.disjoint+2/test]    a write refuses, as a ConstructionError before anything is sent, the kind field given, a kind row's key given, values for a kind without a table, a kind the model lacks and a field it lacks
    def test_writes_refuse_before_sending(self):
        model = vehicles()
        refused = [
            lambda: model.insert({"kind": "car", "name": "x"}, "car", {"seats": 1}),
            lambda: model.insert({"name": "x"}, "car", {"id": 1, "seats": 1}),
            lambda: model.insert({"name": "x"}, "bike", {"seats": 1}),
            lambda: model.insert({"name": "x"}, "boat"),
            lambda: model.insert({"wheels": 3}, "bike"),
            lambda: model.update({"name": "x"}, "bike"),
            lambda: model.update({"id": 1, "kind": "car"}, "car"),
            lambda: model.update({"id": 1}, "car", {"id": 2}),
            lambda: model.change_kind({"id": 1}, "truck", {"vehicle_id": 1}),
            lambda: model.find_by_key({"id": 1, "name": "x"}),
            lambda: model.find().kind("boat"),
        ]
        for index, write in enumerate(refused):
            with self.subTest(index=index), self.assertRaises(p.ConstructionError):
                write()


class DisjointDatabase(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=2)
        await self.pool.execute(p.RawSQL(f"CREATE SCHEMA {SCHEMA}"))
        await self.pool.execute(
            p.RawSQL(f"CREATE TYPE {SCHEMA}.color AS ENUM ('red', 'blue')")
        )
        self.model = vehicles()
        for statement in ps.create_disjoint(self.model):
            await self.pool.execute(statement)

    async def asyncTearDown(self):
        await self.pool.execute(p.RawSQL(f"DROP SCHEMA {SCHEMA} CASCADE"))
        await self.pool.close()

    async def seed(self):
        model = self.model
        mini = await model.insert({"name": "mini"}, "car", {"seats": 4, "color": "red"}).returning_model(self.pool)
        lorry = await model.insert(
            {"name": "lorry"}, "truck", {"axles": 3, "payload_kg": 9000}
        ).returning_model(self.pool)
        bmx = await model.insert({"name": "bmx"}, "bike").returning_model(self.pool)
        return mini, lorry, bmx

    @staticmethod
    def plain(row):
        base, kind, record = row
        return (dict(base), kind, None if record is None else dict(record))

    # [spec:pgorm:req:bindings.disjoint+2/test]    the schema create_disjoint gives runs, and holds every row to its kind: a row inserted by hand for a kind with a table but no row is refused by the server's foreign key
    async def test_schema_runs_and_holds_rows_to_kinds(self):
        statements = ps.create_disjoint(self.model)
        self.assertEqual(
            [type(statement).__name__ for statement in statements],
            ["CreateType", "CreateTable", "CreateTable", "CreateTable", "AlterTable"],
        )
        self.assertTrue(all(isinstance(statement, ps.DDL) for statement in statements))
        with self.assertRaises(p.ForeignKeyViolationError) as raised:
            await self.pool.execute(
                p.RawSQL(f"INSERT INTO {SCHEMA}.vehicle (kind, name) VALUES ('car', 'ghost')")
            )
        self.assertEqual(raised.exception.sqlstate, "23503")
        with self.assertRaises(TypeError):
            ps.create_disjoint(self.model.base)
        undeclared = p.Disjoint(
            "vehicle",
            p.Model(
                "vehicle",
                {
                    "id": p.Column("i32", primary_key=True, default=True),
                    "kind": p.Column(kind_type(), values=["car", "truck", "bike"]),
                },
                schema=SCHEMA,
            ),
            {"car": car(), "truck": truck(), "bike": None},
            schema_name=SCHEMA,
        )
        with self.assertRaises(p.ConstructionError) as raised:
            ps.create_disjoint(undeclared)
        self.assertIn("has a default its description does not state", str(raised.exception))

    # [spec:pgorm:req:bindings.model-unique-keys+2/test]    create_disjoint writes the base's and each kind model's unique keys after its primary key, which the server then holds, nulls equal under a key declared nulls_not_distinct
    async def test_schema_writes_the_models_unique_keys(self):
        fleet = p.Model(
            "fleet",
            {
                "id": p.Column("i32", primary_key=True, generated="by_default"),
                "kind": p.Column(
                    p.TypeName("fleet_kind", schema=SCHEMA), values=["van", "cart"]
                ),
                "code": p.Column("text"),
                "depot": p.Column("text", nullable=True),
            },
            schema=SCHEMA,
            unique_keys=[
                {"fields": ["depot", "code"], "name": "fleet_depot_code", "nulls_not_distinct": True}
            ],
        )
        van = p.Model(
            "van",
            {"id": p.Column("i32", primary_key=True), "plate": p.Column("text")},
            schema=SCHEMA,
            unique_keys=[{"fields": "plate"}],
        )
        fleets = p.Disjoint("fleet", fleet, {"van": van, "cart": None}, schema_name=SCHEMA)
        statements = ps.create_disjoint(fleets)
        tables = [statement.inspect().sql for statement in statements[1:3]]
        self.assertIn(
            'PRIMARY KEY ("id"), CONSTRAINT "fleet_depot_code" UNIQUE NULLS NOT DISTINCT '
            '("depot", "code"), CONSTRAINT "uq-fleet-id-kind" UNIQUE ("id", "kind")',
            tables[0],
        )
        self.assertIn('PRIMARY KEY ("id"), UNIQUE ("plate"), CONSTRAINT "fk-van-id-kind"', tables[1])
        for statement in statements:
            await self.pool.execute(statement)
        await fleets.insert({"code": "a"}, "van", {"plate": "P1"}).returning_model(self.pool)
        with self.assertRaises(p.UniqueViolationError) as raised:
            await fleets.insert({"code": "b"}, "van", {"plate": "P1"}).returning_model(self.pool)
        self.assertEqual(raised.exception.constraint, "van_plate_key")
        with self.assertRaises(p.UniqueViolationError) as raised:
            await fleets.insert({"code": "a"}, "cart").returning_model(self.pool)
        self.assertEqual(raised.exception.constraint, "fleet_depot_code")

    # [spec:pgorm:req:bindings.disjoint+2/test]    an insert of each kind returns what a read of its row gives, each row the base's record, the kind and the kind's record or None, an enum column of a kind's table read as its enum; reads narrow by key, kind, kinds and a kind's condition, order, slice, count, page, stream and read through a keyset cursor
    async def test_inserts_and_reads(self):
        mini, lorry, bmx = await self.seed()
        self.assertIsInstance(mini[0], p.ModelRecord)
        self.assertEqual(
            self.plain(mini),
            ({"id": 1, "kind": "car", "name": "mini"}, "car", {"id": 1, "seats": 4, "color": "red"}),
        )
        self.assertEqual(mini[2].tagged("color").type_name, p.TypeName("color", schema=SCHEMA))
        self.assertEqual(mini[0].tagged("kind").type_name, kind_type())
        self.assertEqual(
            self.plain(lorry),
            (
                {"id": 2, "kind": "truck", "name": "lorry"},
                "truck",
                {"vehicle_id": 2, "axles": 3, "payload_kg": 9000},
            ),
        )
        self.assertEqual(self.plain(bmx), ({"id": 3, "kind": "bike", "name": "bmx"}, "bike", None))

        model = self.model
        name = model.base.col("name")
        ordered = model.find().order_by(model.base.col("id").asc())
        everything = [self.plain(row) for row in (mini, lorry, bmx)]
        self.assertEqual([self.plain(row) for row in await ordered.all(self.pool)], everything)
        self.assertEqual(self.plain(await model.find_by_key({"id": 2}).one(self.pool)), everything[1])
        with self.assertRaises(p.DecodeError):
            await model.find_by_key({"id": 99}).one(self.pool)
        self.assertEqual(
            [self.plain(row) for row in await ordered.kind("truck").all(self.pool)],
            everything[1:2],
        )
        self.assertEqual(
            [self.plain(row) for row in await ordered.kinds(["car", "bike"]).all(self.pool)],
            [everything[0], everything[2]],
        )
        self.assertEqual(
            [
                self.plain(row)
                for row in await ordered.filter_kind("car", model.kinds["car"].col("seats").is_null()).all(self.pool)
            ],
            [],
        )
        self.assertEqual(
            [self.plain(row) for row in await ordered.where(name.eq("bmx")).all(self.pool)],
            everything[2:],
        )
        self.assertEqual(
            [self.plain(row) for row in await ordered.limit(1).offset(1).all(self.pool)],
            everything[1:2],
        )
        self.assertEqual(await ordered.count(self.pool), 3)
        pages = ordered.paginate(2)
        self.assertEqual(await pages.num_items_and_pages(self.pool), (3, 2))
        self.assertEqual(
            [[self.plain(row) for row in page] async for page in pages.pages(self.pool)],
            [everything[:2], everything[2:]],
        )
        async with self.pool.connection() as connection:
            async with await ordered.stream(connection) as rows:
                self.assertEqual([self.plain(row) async for row in rows], everything)
        async with await ordered.stream(self.pool) as rows:
            self.assertEqual([self.plain(row) async for row in rows], everything)
        cursor = model.find().cursor("id")
        self.assertEqual(
            [self.plain(row) for row in await cursor.after(1).first(1).all(self.pool)],
            everything[1:2],
        )
        self.assertEqual(
            [self.plain(row) for row in await cursor.last(2).all(self.pool)],
            everything[1:],
        )
        self.assertEqual(
            [self.plain(row) for row in await cursor.desc().after(3).all(self.pool)],
            [everything[1], everything[0]],
        )
        with self.assertRaises(p.ConstructionError):
            cursor.after(1, 2)

    # [spec:pgorm:req:bindings.disjoint+2/test]    an update writes the base, the kind's row, both or neither, a row of another kind or a key no row has being a DecodeError that changes nothing; a change of kind moves the row between tables and to the kind without one, to its own kind being a DecodeError; a server's refusal keeps its class
    async def test_updates_and_changes_of_kind(self):
        await self.seed()
        model = self.model
        before = [self.plain(row) for row in await model.find().order_by(model.base.col("id").asc()).all(self.pool)]
        renamed = await model.update({"id": 1, "name": "cooper"}, "car", {"seats": 5}).returning_model(self.pool)
        self.assertEqual(
            self.plain(renamed),
            ({"id": 1, "kind": "car", "name": "cooper"}, "car", {"id": 1, "seats": 5, "color": "red"}),
        )
        only_row = await model.update({"id": 2}, "truck", {"axles": 4}).returning_model(self.pool)
        self.assertEqual(only_row[2]["axles"], 4)
        unchanged = await model.update({"id": 3}, "bike").returning_model(self.pool)
        self.assertEqual(self.plain(unchanged), before[2])
        for miss in (
            model.update({"id": 1, "name": "x"}, "truck", {"axles": 9}),
            model.update({"id": 99, "name": "x"}, "bike"),
            model.change_kind({"id": 1}, "car"),
            model.change_kind({"id": 99}, "bike"),
        ):
            with self.subTest(miss=miss.inspect().sql[:40]), self.assertRaises(p.DecodeError):
                await miss.returning_model(self.pool)
        rekinded = await model.change_kind({"id": 1}, "truck", {"axles": 2, "payload_kg": 500}).returning_model(self.pool)
        self.assertEqual(
            self.plain(rekinded),
            (
                {"id": 1, "kind": "truck", "name": "cooper"},
                "truck",
                {"vehicle_id": 1, "axles": 2, "payload_kg": 500},
            ),
        )
        biked = await model.change_kind({"id": 1}, "bike").returning_model(self.pool)
        self.assertEqual(self.plain(biked), ({"id": 1, "kind": "bike", "name": "cooper"}, "bike", None))
        counted = await self.pool.query(p.RawSQL(f"SELECT count(*) AS n FROM {SCHEMA}.car"))
        self.assertEqual([dict(row) for row in counted], [{"n": 0}])
        with self.assertRaises(p.NotNullViolationError):
            await model.insert({"name": "seatless"}, "car").returning_model(self.pool)
        self.assertEqual(await model.find().count(self.pool), 3)

    # [spec:pgorm:req:bindings.disjoint+2/test]    rows of tables written without the constraints, a car with no car row and a bike with a truck row, are each a DecodeError naming the base table, the key and the kinds, never a row read without its row
    async def test_inconsistent_rows_are_decode_errors(self):
        await self.pool.execute(p.RawSQL(f"DROP SCHEMA {SCHEMA} CASCADE"))
        await self.pool.execute(p.RawSQL(f"CREATE SCHEMA {SCHEMA}"))
        for sql in (
            f"CREATE TYPE {SCHEMA}.color AS ENUM ('red', 'blue')",
            f"CREATE TYPE {SCHEMA}.vehicle_kind AS ENUM ('car', 'truck', 'bike')",
            f"CREATE TABLE {SCHEMA}.vehicle (id integer PRIMARY KEY, kind {SCHEMA}.vehicle_kind NOT NULL, name text NOT NULL)",
            f"CREATE TABLE {SCHEMA}.car (id integer PRIMARY KEY, seats smallint NOT NULL, color {SCHEMA}.color)",
            f"CREATE TABLE {SCHEMA}.truck (vehicle_id integer PRIMARY KEY, axles smallint NOT NULL, payload integer NOT NULL)",
            f"INSERT INTO {SCHEMA}.vehicle VALUES (1, 'car', 'ghost'), (2, 'bike', 'hauler')",
            f"INSERT INTO {SCHEMA}.truck VALUES (2, 2, 300)",
        ):
            await self.pool.execute(p.RawSQL(sql))
        model = self.model
        with self.assertRaises(p.DecodeError) as raised:
            await model.find_by_key({"id": 1}).one(self.pool)
        self.assertIn(
            "the `python_disjoint.vehicle` row whose key is (`id` = 1) is of the kind `car` but "
            "has no row in `python_disjoint.car`",
            str(raised.exception),
        )
        with self.assertRaises(p.DecodeError) as raised:
            await model.find_by_key({"id": 2}).one(self.pool)
        self.assertIn("a row in `python_disjoint.truck`, the table of the kind `truck`", str(raised.exception))
        with self.assertRaises(p.DecodeError):
            await model.find().all(self.pool)



SHOPS = "python_unkinded"

# A schema written with no kind column: each kind's table keyed by a foreign
# key from its primary key onto the shop's, a shop of no kind's table dormant.
SHOP_TABLES = (
    f"CREATE TABLE {SHOPS}.shop (id integer PRIMARY KEY, name text NOT NULL)",
    f"CREATE TABLE {SHOPS}.web_shop (shop_id integer PRIMARY KEY REFERENCES {SHOPS}.shop (id), url text NOT NULL)",
    f"CREATE TABLE {SHOPS}.storefront (id integer PRIMARY KEY REFERENCES {SHOPS}.shop (id), street text NOT NULL)",
    f"INSERT INTO {SHOPS}.shop VALUES (1, 'alpha'), (2, 'beta'), (3, 'gamma'), (4, 'delta')",
    f"INSERT INTO {SHOPS}.web_shop VALUES (1, 'https://alpha.example'), (4, 'https://delta.example')",
    f"INSERT INTO {SHOPS}.storefront VALUES (2, 'Main Street')",
)


def shop():
    return p.Model(
        "shop", {"id": p.Column("i32", primary_key=True), "name": p.Column("text")}, schema=SHOPS
    )


def web_shop():
    return p.Model(
        "web_shop",
        {"shop_id": p.Column("i32", primary_key=True), "url": p.Column("text")},
        schema=SHOPS,
    )


def storefront():
    return p.Model(
        "storefront",
        {"id": p.Column("i32", primary_key=True), "street": p.Column("text")},
        schema=SHOPS,
    )


def shops(**kinds):
    return p.Disjoint(
        "shop",
        shop(),
        kinds or {"web_shop": web_shop(), "storefront": storefront(), "dormant": None},
        unkinded=True,
    )


class UnkindedDeclaration(unittest.TestCase):
    # [spec:pgorm:req:bindings.disjoint-unkinded/test]    an unkinded model describes itself with no kind field or kind type, reads and narrows by presence, and refuses what the derive refuses — a second kind without a model, no kind with one, the kinds' refusals — and a kind option beside unkinded, each a ConstructionError raised as it is declared
    def test_unkinded_declaration_reads_by_presence(self):
        with patch("socket.socket", side_effect=AssertionError("a declaration connected")):
            model = shops()
        self.assertEqual(
            model.describe(),
            {
                "name": "shop",
                "base": "shop",
                "kind_field": None,
                "kind_type": {"schema": None, "name": None},
                "kinds": [
                    {"value": "web_shop", "model": "web_shop"},
                    {"value": "storefront", "model": "storefront"},
                    {"value": "dormant", "model": None},
                ],
            },
        )
        self.assertIsNone(model.kind_field)
        self.assertEqual(
            model.find().inspect().sql,
            'SELECT "shop"."id" AS "s0_id", "shop"."name" AS "s0_name", '
            '"web_shop"."shop_id" AS "s1_shop_id", "web_shop"."url" AS "s1_url", '
            '"storefront"."id" AS "s2_id", "storefront"."street" AS "s2_street" '
            'FROM "python_unkinded"."shop" '
            'LEFT JOIN "python_unkinded"."web_shop" ON "shop"."id" = "web_shop"."shop_id" '
            'LEFT JOIN "python_unkinded"."storefront" ON "shop"."id" = "storefront"."id"',
        )
        self.assertTrue(model.find().kind("web_shop").inspect().sql.endswith('WHERE "web_shop"."shop_id" IS NOT NULL'))
        refused = {
            "two kinds without a model": lambda: shops(web_shop=web_shop(), dormant=None, closed=None),
            "no kind with a model": lambda: shops(dormant=None),
            "no kinds": lambda: p.Disjoint("shop", shop(), {}, unkinded=True),
            "one model for two kinds": lambda: shops(web_shop=web_shop(), online=web_shop()),
            "key types": lambda: shops(
                web_shop=p.Model("web_shop", {"shop_id": p.Column("i64", primary_key=True)}, schema=SHOPS)
            ),
            "a kind option": lambda: p.Disjoint(
                "shop", shop(), {"web_shop": web_shop()}, unkinded=True, kind="state"
            ),
            "a kind type option": lambda: p.Disjoint(
                "shop", shop(), {"web_shop": web_shop()}, unkinded=True, schema_name=SHOPS
            ),
        }
        for case, declare in refused.items():
            with self.subTest(case=case), self.assertRaises(p.ConstructionError):
                declare()
        with self.assertRaises(p.ConstructionError) as raised:
            refused["two kinds without a model"]()
        self.assertIn("declares the kinds `dormant` and `closed` without a table", str(raised.exception))

    # [spec:pgorm:req:bindings.disjoint-unkinded/test]    a write and the schema of an unkinded model are refused with a ConstructionError naming its base read-only, before anything is sent
    def test_unkinded_writes_are_refused_read_only(self):
        model = cast(Any, shops())
        writes = [
            lambda: model.insert({"name": "x"}, "dormant"),
            lambda: model.insert({"name": "x"}, "web_shop", {"url": "u"}),
            lambda: model.update({"id": 1, "name": "x"}, "dormant"),
            lambda: model.change_kind({"id": 1}, "dormant"),
            lambda: ps.create_disjoint(model),
        ]
        with patch("socket.socket", side_effect=AssertionError("a refused write connected")):
            for index, write in enumerate(writes):
                with self.subTest(index=index), self.assertRaises(p.ConstructionError) as raised:
                    write()
                self.assertIn("is unkinded", str(raised.exception))
                self.assertIn("so it is read-only", str(raised.exception))


class UnkindedDatabase(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=2)
        await self.pool.execute(p.RawSQL(f"CREATE SCHEMA {SHOPS}"))
        for sql in SHOP_TABLES:
            await self.pool.execute(p.RawSQL(sql))

    async def asyncTearDown(self):
        await self.pool.execute(p.RawSQL(f"DROP SCHEMA {SHOPS} CASCADE"))
        await self.pool.close()

    @staticmethod
    def plain(row):
        base, kind, record = row
        return (dict(base), kind, None if record is None else dict(record))

    # [spec:pgorm:req:bindings.disjoint-unkinded/test]    each base row reads as the kind whose table has its row, one with a row in none as the kind without a model; reads narrow by presence, count, page and run on a connection; a row in two kinds' tables, and one in none where every kind has a model, is a DecodeError naming the base, the key and the kinds
    async def test_unkinded_rows_read_by_presence(self):
        model = shops()
        ordered = model.find().order_by(model.base.col("id").asc())
        everything = [
            ({"id": 1, "name": "alpha"}, "web_shop", {"shop_id": 1, "url": "https://alpha.example"}),
            ({"id": 2, "name": "beta"}, "storefront", {"id": 2, "street": "Main Street"}),
            ({"id": 3, "name": "gamma"}, "dormant", None),
            ({"id": 4, "name": "delta"}, "web_shop", {"shop_id": 4, "url": "https://delta.example"}),
        ]
        self.assertEqual([self.plain(row) for row in await ordered.all(self.pool)], everything)
        async with self.pool.connection() as connection:
            self.assertEqual(self.plain(await model.find_by_key({"id": 3}).one(connection)), everything[2])
        self.assertEqual([self.plain(row) for row in await ordered.kind("web_shop").all(self.pool)],
                         [everything[0], everything[3]])
        self.assertEqual([self.plain(row) for row in await ordered.kind("dormant").all(self.pool)], [everything[2]])
        self.assertEqual(
            [self.plain(row) for row in await ordered.kinds(["storefront", "dormant"]).all(self.pool)],
            everything[1:3],
        )
        streets = storefront()
        self.assertEqual(
            [self.plain(row) for row in await model.find().filter_kind(
                "storefront", streets.col("street") == "Main Street").all(self.pool)],
            [everything[1]],
        )
        self.assertEqual(await model.find().count(self.pool), 4)
        self.assertEqual(
            [[row[0]["id"] for row in page] async for page in ordered.paginate(3).pages(self.pool)],
            [[1, 2, 3], [4]],
        )

        await self.pool.execute(p.RawSQL(f"INSERT INTO {SHOPS}.storefront VALUES (4, 'Side Street')"))
        with self.assertRaises(p.DecodeError) as raised:
            await model.find_by_key({"id": 4}).one(self.pool)
        self.assertIn(
            "the `python_unkinded.shop` row whose key is (`id` = 4) of an unkinded disjoint entity "
            "has a row in `python_unkinded.web_shop`",
            str(raised.exception),
        )
        with self.assertRaises(p.DecodeError):
            await ordered.all(self.pool)
        outlets = p.Disjoint("shop", shop(), {"web_shop": web_shop(), "storefront": storefront()}, unkinded=True)
        with self.assertRaises(p.DecodeError) as raised:
            await outlets.find_by_key({"id": 3}).one(self.pool)
        self.assertIn("has a row in none of its kinds' tables", str(raised.exception))


if __name__ == "__main__":
    unittest.main()
