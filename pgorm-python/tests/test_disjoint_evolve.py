"""A disjoint model's kind changes, written by pgorm's DisjointShape from the
declaration as it will be: the steps and gaps each change gives, its
refusals, and each change run on a live server."""

import os
import unittest
from unittest.mock import patch

import pgorm as p
import pgorm.schema as ps

SCHEMA = "python_disjoint_evolve"


def kind_type():
    return p.TypeName("vehicle_kind", schema=SCHEMA)


def table(name, **columns):
    return p.Model(name, {"id": p.Column("i32", primary_key=True), **columns}, schema=SCHEMA)


def vehicles(**kinds):
    """The declaration whose kinds are ``kinds``, in order, each a kind
    table's columns or ``None`` for a kind with none."""
    base = p.Model(
        "vehicle",
        {
            "id": p.Column("i32", primary_key=True, generated="by_default"),
            "kind": p.Column(kind_type(), values=list(kinds)),
            "name": p.Column("text"),
        },
        schema=SCHEMA,
    )
    return p.Disjoint(
        "vehicle",
        base,
        {value: None if columns is None else table(value, **columns) for value, columns in kinds.items()},
        schema_name=SCHEMA,
    )


CAR = {"seats": p.Column("i16")}
TRUCK = {"axles": p.Column("i16")}
VAN = {"doors": p.Column("i16")}
BIKE = {"gears": p.Column("i16")}


class KindChangeDeclaration(unittest.TestCase):
    # [spec:pgorm:req:bindings.disjoint-evolve/test]    each change gives its steps as pgorm's shape writes them, data sending nothing: a kind with a table added in two steps, a kind table added with a gap naming the kind and its table, and a kind table dropped in one step
    def test_changes_give_the_shape_steps(self):
        p.require_capability("disjoint.evolve")
        with patch("socket.socket", side_effect=AssertionError("a change connected")):
            added = ps.add_disjoint_kind(vehicles(car=CAR, van=VAN, bike=None), "van")
            tabled = ps.add_disjoint_kind_table(vehicles(car=CAR, bike=BIKE), "bike")
            dropped = ps.remove_disjoint_kind_table(
                vehicles(car=CAR, bike=None), "bike", p.Table("bike", schema=SCHEMA)
            )
        self.assertIsInstance(added, ps.DisjointChange)
        self.assertEqual(
            [[statement.inspect().sql for statement in step.statements] for step in added.steps][0],
            [
                f'ALTER TYPE "{SCHEMA}"."vehicle_kind" ADD VALUE \'van\' AFTER \'car\'',
            ],
        )
        self.assertEqual(
            [type(statement).__name__ for statement in added.steps[1].statements],
            ["CreateTable", "AlterTable"],
        )
        self.assertEqual([step.gap for step in added.steps], [None, None])
        gap = tabled.steps[0].gap
        self.assertIsInstance(gap, ps.DisjointGap)
        self.assertEqual((gap.kind, gap.table.schema, gap.table.name), ("bike", SCHEMA, "bike"))
        self.assertIsNone(tabled.steps[1].gap)
        self.assertIn("VALIDATE CONSTRAINT", tabled.steps[1].statements[0].inspect().sql)
        self.assertEqual(len(dropped.steps), 1)
        self.assertEqual(
            dropped.steps[0].statements[-1].inspect().sql, f'DROP TABLE "{SCHEMA}"."bike"'
        )
        self.assertTrue(all(isinstance(s, ps.DDL) for step in added.steps for s in step.statements))

    # [spec:pgorm:req:bindings.disjoint-evolve/test]    a change the declaration does not describe is a ConstructionError as the function is called, an unkinded model's a ConstructionError naming it read-only, and an argument of the wrong type a TypeError
    def test_changes_refuse_what_the_shape_refuses(self):
        two = vehicles(car=CAR, bike=None)
        refused = {
            "not declared": lambda: ps.add_disjoint_kind(two, "van"),
            "recreating not declared": lambda: ps.add_disjoint_kind_recreating(two, "van"),
            "still declared": lambda: ps.remove_disjoint_kind(two, "bike"),
            "declared without a table": lambda: ps.add_disjoint_kind_table(two, "bike"),
            "declared with a table": lambda: ps.remove_disjoint_kind_table(two, "car", "car"),
        }
        for case, change in refused.items():
            with self.subTest(case=case), self.assertRaises(p.ConstructionError):
                change()
        with self.assertRaises(p.ConstructionError) as raised:
            ps.add_disjoint_kind(two, "van")
        self.assertIn("`van` is no kind of the disjoint entity", str(raised.exception))
        base = p.Model("shop", {"id": p.Column("i32", primary_key=True)}, schema=SCHEMA)
        unkinded = p.Disjoint("shop", base, {"web": table("web"), "dormant": None}, unkinded=True)
        with self.assertRaises(p.ConstructionError) as raised:
            ps.add_disjoint_kind(unkinded, "web")
        self.assertIn("read-only", str(raised.exception))
        for wrong in (
            lambda: ps.add_disjoint_kind(two.base, "car"),
            lambda: ps.add_disjoint_kind(two, 1),
            lambda: ps.remove_disjoint_kind_table(two, "bike", 1),
        ):
            with self.assertRaises(TypeError):
                wrong()


class KindChangeDatabase(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=2)
        await self.pool.execute(p.RawSQL(f"CREATE SCHEMA {SCHEMA}"))
        model = vehicles(car=CAR, truck=TRUCK, bike=None)
        for statement in ps.create_disjoint(model):
            await self.pool.execute(statement)
        await model.insert({"name": "mini"}, "car", {"seats": 4}).returning_model(self.pool)
        await model.insert({"name": "lorry"}, "truck", {"axles": 3}).returning_model(self.pool)
        await model.insert({"name": "bmx"}, "bike").returning_model(self.pool)

    async def asyncTearDown(self):
        await self.pool.execute(p.RawSQL(f"DROP SCHEMA {SCHEMA} CASCADE"))
        await self.pool.close()

    async def run_step(self, step):
        async with self.pool.transaction() as tx:
            for statement in step.statements:
                await tx.execute(statement)
        self.pool.clear_statement_caches()

    async def run_change(self, change):
        for step in change.steps:
            await self.run_step(step)

    async def kinds(self):
        rows = await self.pool.query(
            p.RawSQL(f"SELECT enum_range(NULL::{SCHEMA}.vehicle_kind)::text AS kinds")
        )
        return rows[0]["kinds"]

    @staticmethod
    def plain(row):
        base, kind, record = row
        return (base["name"], kind, None if record is None else dict(record))

    async def read(self, model):
        rows = await model.find().order_by(model.base.col("id").asc()).all(self.pool)
        return [self.plain(row) for row in rows]

    # [spec:pgorm:req:bindings.disjoint-evolve/test]    a kind added in two steps and one added by recreating the kind type each run, the declaration as it will be then reading every row and writing the new kinds
    async def test_kinds_are_added_through_both_routes(self):
        with_van = vehicles(car=CAR, truck=TRUCK, van=VAN, bike=None)
        await self.run_change(ps.add_disjoint_kind(with_van, "van"))
        await with_van.insert({"name": "kombi"}, "van", {"doors": 3}).returning_model(self.pool)
        with_boat = vehicles(car=CAR, truck=TRUCK, van=VAN, bike=None, boat=None)
        change = ps.add_disjoint_kind_recreating(with_boat, "boat")
        self.assertEqual(len(change.steps), 1)
        await self.run_change(change)
        await with_boat.insert({"name": "dinghy"}, "boat").returning_model(self.pool)
        self.assertEqual(await self.kinds(), "{car,truck,van,bike,boat}")
        self.assertEqual(
            await self.read(with_boat),
            [
                ("mini", "car", {"id": 1, "seats": 4}),
                ("lorry", "truck", {"id": 2, "axles": 3}),
                ("bmx", "bike", None),
                ("kombi", "van", {"id": 4, "doors": 3}),
                ("dinghy", "boat", None),
            ],
        )

    # [spec:pgorm:req:bindings.disjoint-evolve/test]    a kind table is added across its gap, the program writing the rows its base rows lack between the steps, and the second step refused while one is missing; a kind table dropped turns the kind into one with no table, its rows lost
    async def test_kind_tables_are_added_and_removed(self):
        bikes = vehicles(car=CAR, truck=TRUCK, bike=BIKE)
        change = ps.add_disjoint_kind_table(bikes, "bike")
        first, second = change.steps
        await self.run_step(first)
        with self.assertRaises(p.ForeignKeyViolationError):
            await self.run_step(second)
        gap = first.gap
        await self.pool.execute(
            p.RawSQL(
                f'INSERT INTO "{gap.table.schema}"."{gap.table.name}" (id, gears) '
                f"SELECT id, 21 FROM {SCHEMA}.vehicle WHERE kind = '{gap.kind}'"
            )
        )
        await self.run_step(second)
        self.assertEqual((await self.read(bikes))[2], ("bmx", "bike", {"id": 3, "gears": 21}))
        no_trucks = vehicles(car=CAR, truck=None, bike=BIKE)
        await self.run_change(
            ps.remove_disjoint_kind_table(no_trucks, "truck", p.Table("truck", schema=SCHEMA))
        )
        self.assertEqual((await self.read(no_trucks))[1], ("lorry", "truck", None))
        tables = await self.pool.query(
            p.RawSQL(f"SELECT count(*) AS n FROM pg_tables WHERE schemaname = '{SCHEMA}' AND tablename = 'truck'")
        )
        self.assertEqual(tables[0]["n"], 0)

    # [spec:pgorm:req:bindings.disjoint-evolve/test]    a kind removed while a base row has it is refused by the server's check (23514) and changes nothing; once no row has it, the kind type is recreated without it
    async def test_a_kind_is_removed_once_unused(self):
        without_bikes = vehicles(car=CAR, truck=TRUCK)
        change = ps.remove_disjoint_kind(without_bikes, "bike")
        with self.assertRaises(p.CheckViolationError) as raised:
            await self.run_change(change)
        self.assertEqual(raised.exception.sqlstate, "23514")
        self.assertEqual(await self.kinds(), "{car,truck,bike}")
        await self.pool.execute(p.RawSQL(f"DELETE FROM {SCHEMA}.vehicle WHERE kind = 'bike'"))
        await self.run_change(change)
        self.assertEqual(await self.kinds(), "{car,truck}")
        self.assertEqual([kind for _, kind, _ in await self.read(without_bikes)], ["car", "truck"])
