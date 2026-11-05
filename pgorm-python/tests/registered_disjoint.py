"""Run only against the independent application-binding wheel: the
application's `#[derive(Disjoint)]` enum, registered with its entities."""

import os
import unittest

import pgorm as p
from pgorm import schema as s


class RegisteredDisjoint(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=2)
        await self.pool.execute(p.RawSQL("CREATE SCHEMA python_shipments"))
        self.shipments = p.disjoint_entity("app.Shipments")
        for statement in s.create_disjoint(self.shipments):
            await self.pool.execute(statement)
        self.shipment = p.entity("app.Shipment")
        self.parcel = p.entity("app.Parcel")
        self.pallet = p.entity("app.Pallet")

    async def asyncTearDown(self):
        await self.pool.execute(p.RawSQL("DROP SCHEMA python_shipments CASCADE"))
        await self.pool.close()

    def base(self, label, **values):
        active = self.shipment.active().set("label", label)
        for column, value in values.items():
            active = active.set(column, value)
        return active

    @staticmethod
    def plain(row):
        base, kind, record = row
        return (dict(base), kind, None if record is None else dict(record))

    async def seed(self):
        shipments = self.shipments
        boxed = await shipments.insert(
            self.base("box"), "parcel", self.parcel.active().set("weight_g", 500)
        ).returning_model(self.pool)
        note = await shipments.insert(self.base("note"), "letter").returning_model(self.pool)
        crate = await shipments.insert(
            self.base("crate"), "pallet", self.pallet.active().set("volume_l", 40).set("fragile", None)
        ).returning_model(self.pool)
        return boxed, note, crate

    # [spec:pgorm:req:bindings.disjoint+1/test]    a registered disjoint enum is listed, described under the declared keys and reported in the capability manifest beside its base and kind entities; a name the module does not register is refused
    async def test_registration_describes_the_enum(self):
        self.assertEqual(p.disjoint_entities(), ["app.Listings", "app.Shipments"])
        description = self.shipments.describe()
        self.assertEqual(
            description,
            {
                "name": "app.Shipments",
                "base": "app.Shipment",
                "kind_field": "kind",
                "kind_type": {"schema": "python_shipments", "name": "shipment_kind"},
                "kinds": [
                    {"value": "parcel", "model": "app.Parcel"},
                    {"value": "letter", "model": None},
                    {"value": "pallet", "model": "app.Pallet"},
                ],
            },
        )
        self.assertIn(description, p.capabilities()["registrations"]["disjoints"])
        p.require_capability("disjoint_entity")
        with self.assertRaises(p.UnsupportedCapabilityError):
            p.disjoint_entity("app.Missing")
        statements = s.create_disjoint(self.shipments)
        self.assertEqual(
            [type(statement).__name__ for statement in statements],
            ["CreateType", "CreateTable", "CreateTable", "CreateTable", "AlterTable"],
        )
        self.assertIn(
            'CREATE TYPE "python_shipments"."shipment_kind" AS ENUM (\'parcel\', \'letter\', \'pallet\')',
            statements[0].inspect().sql,
        )

    # [spec:pgorm:req:bindings.disjoint+1/test]    a registered disjoint enum's writes take its entities' ActiveModels and return each row as the base's model, the kind and the kind's model, which keep the Rust models they were read as; reads narrow, page, count, stream and read through a cursor on a pool and a connection
    async def test_writes_and_reads_through_registered_entities(self):
        boxed, note, crate = await self.seed()
        self.assertIsInstance(boxed[0], p.EntityModel)
        self.assertEqual(boxed[0].entity_name, "app.Shipment")
        self.assertEqual(boxed[2].entity_name, "app.Parcel")
        self.assertEqual(
            self.plain(boxed),
            ({"id": 1, "kind": "parcel", "label": "box"}, "parcel", {"shipment_id": 1, "weight_g": 500}),
        )
        self.assertEqual(self.plain(note), ({"id": 2, "kind": "letter", "label": "note"}, "letter", None))
        self.assertEqual(
            self.plain(crate),
            ({"id": 3, "kind": "pallet", "label": "crate"}, "pallet", {"id": 3, "volume_l": 40, "fragile": None}),
        )
        self.assertEqual(boxed[0].tagged("kind").type_name, p.TypeName("shipment_kind", schema="python_shipments"))
        everything = [self.plain(row) for row in (boxed, note, crate)]
        ordered = self.shipments.find().order_by(self.shipment.col("id").asc())
        self.assertEqual([self.plain(row) for row in await ordered.all(self.pool)], everything)
        async with self.pool.connection() as connection:
            self.assertEqual(
                self.plain(await self.shipments.find_by_key({"id": 3}).one(connection)), everything[2]
            )
            async with await ordered.stream(connection) as rows:
                self.assertEqual([self.plain(row) async for row in rows], everything)
        self.assertEqual(
            [self.plain(row) for row in await ordered.kinds(["letter", "pallet"]).all(self.pool)],
            everything[1:],
        )
        self.assertEqual(
            [
                self.plain(row)
                for row in await ordered.filter_kind("parcel", self.parcel.col("weight_g").gt(100)).all(self.pool)
            ],
            everything[:1],
        )
        self.assertEqual(await ordered.kind("parcel").count(self.pool), 1)
        self.assertEqual(
            [[self.plain(row) for row in page] async for page in ordered.paginate(2).pages(self.pool)],
            [everything[:2], everything[2:]],
        )
        self.assertEqual(
            [self.plain(row) for row in await self.shipments.find().cursor("id").after(1).first(1).all(self.pool)],
            everything[1:2],
        )
        active = boxed[0].into_active()
        self.assertEqual(active.get("label").value.value, "box")

    # [spec:pgorm:req:bindings.disjoint+1/test]    a registered enum's update and change of kind write through its entities' ActiveModels, a miss being a DecodeError that changes nothing, and an ActiveModel of another registration, the kind column set, a kind row's key set and a row for a kind without a table are refused before anything is sent
    async def test_updates_changes_and_refusals(self):
        await self.seed()
        shipments = self.shipments
        renamed = await shipments.update(
            self.base("big box", id=1), "parcel", self.parcel.active().set("weight_g", 900)
        ).returning_model(self.pool)
        self.assertEqual(
            self.plain(renamed),
            ({"id": 1, "kind": "parcel", "label": "big box"}, "parcel", {"shipment_id": 1, "weight_g": 900}),
        )
        with self.assertRaises(p.DecodeError):
            await shipments.update(self.base("x", id=1), "letter").returning_model(self.pool)
        with self.assertRaises(p.DecodeError):
            await shipments.change_kind({"id": 2}, "letter").returning_model(self.pool)
        moved = await shipments.change_kind(
            {"id": 1}, "pallet", self.pallet.active().set("volume_l", 12)
        ).returning_model(self.pool)
        self.assertEqual(
            self.plain(moved),
            ({"id": 1, "kind": "pallet", "label": "big box"}, "pallet", {"id": 1, "volume_l": 12, "fragile": None}),
        )
        self.assertEqual(await shipments.find().kind("parcel").count(self.pool), 0)
        refused = [
            lambda: shipments.insert(self.parcel.active(), "letter"),
            lambda: shipments.insert(self.base("x", kind="letter"), "letter"),
            lambda: shipments.insert(self.base("x"), "parcel", self.parcel.active().set("shipment_id", 9)),
            lambda: shipments.insert(self.base("x"), "letter", self.parcel.active()),
            lambda: shipments.insert(self.base("x"), "parcel", self.pallet.active()),
            lambda: shipments.update(self.base("x"), "letter"),
            lambda: shipments.find_by_key({"label": "x"}),
        ]
        for index, write in enumerate(refused):
            with self.subTest(index=index), self.assertRaises(p.ConstructionError):
                write()
        with self.assertRaises(TypeError):
            shipments.insert({"label": "x"}, "letter")

    # [spec:pgorm:req:bindings.disjoint-evolve/test]    a registered enum's kind changes are the shape's, Schema's for the enum: a value the database holds and the enum does not is removed live, the kind type recreated with the enum's values in order
    async def test_kind_changes_are_the_enums(self):
        added = s.add_disjoint_kind(self.shipments, "pallet")
        self.assertEqual(
            [statement.inspect().sql for statement in added.steps[0].statements],
            ['ALTER TYPE "python_shipments"."shipment_kind" ADD VALUE \'pallet\' AFTER \'letter\''],
        )
        self.assertEqual(len(added.steps), 2)
        await self.pool.execute(p.RawSQL("ALTER TYPE python_shipments.shipment_kind ADD VALUE 'ghost'"))
        for step in s.remove_disjoint_kind(self.shipments, "ghost").steps:
            async with self.pool.transaction() as tx:
                for statement in step.statements:
                    await tx.execute(statement)
        self.pool.clear_statement_caches()
        kinds = await self.pool.one(
            p.RawSQL("SELECT enum_range(NULL::python_shipments.shipment_kind)::text AS kinds")
        )
        self.assertEqual(kinds["kinds"], "{parcel,letter,pallet}")
        self.assertEqual(len(await self.seed()), 3)


# Listings as an existing schema holds them: no kind column, each kind's
# table keyed by a foreign key onto the listing's.
LISTINGS = (
    "CREATE TABLE python_listings.listings (id integer PRIMARY KEY, title text NOT NULL)",
    "CREATE TABLE python_listings.auctions "
    "(listing_id integer PRIMARY KEY REFERENCES python_listings.listings (id), reserve integer NOT NULL)",
    "CREATE TABLE python_listings.sales "
    "(id integer PRIMARY KEY REFERENCES python_listings.listings (id), price integer NOT NULL)",
    "INSERT INTO python_listings.listings VALUES (1, 'lamp'), (2, 'chair'), (3, 'desk')",
    "INSERT INTO python_listings.auctions VALUES (1, 50)",
    "INSERT INTO python_listings.sales VALUES (2, 80)",
)


class RegisteredUnkinded(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=2)
        await self.pool.execute(p.RawSQL("CREATE SCHEMA python_listings"))
        for statement in LISTINGS:
            await self.pool.execute(p.RawSQL(statement))
        self.listings = p.disjoint_entity("app.Listings")
        self.listing = p.entity("app.Listing")
        self.auction = p.entity("app.Auction")

    async def asyncTearDown(self):
        await self.pool.execute(p.RawSQL("DROP SCHEMA python_listings CASCADE"))
        await self.pool.close()

    @staticmethod
    def plain(row):
        base, kind, record = row
        return (dict(base), kind, None if record is None else dict(record))

    # [spec:pgorm:req:bindings.disjoint-unkinded/test]    a downstream crate's unkinded enum registers through the same registration, describes itself with no kind field or kind type, reads each row as the kind whose table has its row, the unit kind where none does, its kinds named as its variants in snake_case, narrows by presence, and reports a row in two kinds' tables as a DecodeError
    async def test_unkinded_enum_registers_and_reads(self):
        self.assertEqual(
            self.listings.describe(),
            {
                "name": "app.Listings",
                "base": "app.Listing",
                "kind_field": None,
                "kind_type": {"schema": None, "name": None},
                "kinds": [
                    {"value": "auction", "model": "app.Auction"},
                    {"value": "sale", "model": "app.Sale"},
                    {"value": "draft", "model": None},
                ],
            },
        )
        ordered = self.listings.find().order_by(self.listing.col("id").asc())
        everything = [
            ({"id": 1, "title": "lamp"}, "auction", {"listing_id": 1, "reserve": 50}),
            ({"id": 2, "title": "chair"}, "sale", {"id": 2, "price": 80}),
            ({"id": 3, "title": "desk"}, "draft", None),
        ]
        rows = await ordered.all(self.pool)
        self.assertEqual([self.plain(row) for row in rows], everything)
        self.assertEqual(rows[0][2].entity_name, "app.Auction")
        self.assertEqual([self.plain(row) for row in await ordered.kind("draft").all(self.pool)], everything[2:])
        self.assertEqual(
            [self.plain(row) for row in await ordered.filter_kind("auction", self.auction.col("reserve").gt(10)).all(self.pool)],
            everything[:1],
        )
        self.assertEqual(await self.listings.find().kinds(["auction", "sale"]).count(self.pool), 2)
        await self.pool.execute(p.RawSQL("INSERT INTO python_listings.sales VALUES (1, 70)"))
        with self.assertRaises(p.DecodeError) as raised:
            await self.listings.find_by_key({"id": 1}).one(self.pool)
        self.assertIn("of an unkinded disjoint entity has a row in", str(raised.exception))

    # [spec:pgorm:req:bindings.disjoint-unkinded/test]    a registered unkinded enum's writes and schema are refused with a ConstructionError naming its base read-only, before anything is sent
    async def test_unkinded_registration_is_read_only(self):
        listings = self.listings
        base = self.listing.active().set("id", 9).set("title", "rug")
        refused = [
            lambda: listings.insert(base, "draft"),
            lambda: listings.insert(base, "auction", self.auction.active().set("reserve", 1)),
            lambda: listings.update(self.listing.active().set("id", 1).set("title", "x"), "draft"),
            lambda: listings.change_kind({"id": 1}, "draft"),
            lambda: s.create_disjoint(listings),
            lambda: s.add_disjoint_kind(listings, "draft"),
        ]
        for index, write in enumerate(refused):
            with self.subTest(index=index), self.assertRaises(p.ConstructionError) as raised:
                write()
            self.assertIn("is unkinded", str(raised.exception))
        count = await self.pool.one(p.RawSQL("SELECT count(*) AS n FROM python_listings.listings"))
        self.assertEqual(count["n"], 3)


if __name__ == "__main__":
    unittest.main()
