"""Execute concrete generated wrappers from a freshly installed application wheel."""

import copy
from decimal import Decimal
import inspect
import os
from pathlib import Path
import unittest
from unittest.mock import patch

import pgorm as p
from pgorm import app, pipeline as pl, schema as s
from pgorm._registered.compat import check
from pgorm._registered.entities import Change, Inserted, Updated
import registered_entities as fixture


class GeneratedApplication(unittest.IsolatedAsyncioTestCase):
    asyncSetUp = fixture.RegisteredEntities.asyncSetUp
    asyncTearDown = fixture.RegisteredEntities.asyncTearDown

    # [spec:pgorm:req:bindings.codegen/test]
    async def test_generated_models_keep_native_states(self):
        original = (
            app.Account.active()
            .set_id(1)
            .set_display_name("Generated")
            .set_note(None)
            .set_mood("busy")
        )
        self.assertIsInstance(original, app.AccountActive)
        self.assertEqual(original.get("id").value.kind, "i32")
        self.assertEqual(
            original.get("mood").value.type_name,
            p.TypeName("Mood", schema="python_entities"),
        )
        with patch(
            "subprocess.run", side_effect=AssertionError("query compiled a program")
        ):
            async with self.pool.connection() as connection:
                model = await original.insert(connection)
                self.assertIsInstance(model, app.AccountModel)
                self.assertEqual(
                    (model.id, model.display_name, model.note, model.mood),
                    (1, "Generated|before", None, "busy"),
                )
                self.assertEqual(model["display name"], model.display_name)
                self.assertEqual(
                    model.into_active().get("id").state, p.ActiveState.Unchanged
                )
                changed = model.with_value("display name", "Updated")
                self.assertEqual(changed.display_name, "Updated")
                updated = (
                    await model.into_active()
                    .set_display_name("Saved")
                    .update(connection)
                )
                self.assertEqual((updated.display_name, updated.version), ("Saved", 2))
                query = app.Account.find().where(app.Account.col("id") == 1)
                self.assertEqual([row.id for row in await query.all(connection)], [1])
                self.assertIsInstance(await query.one(connection), app.AccountModel)
                self.assertIsInstance(await query.optional(connection), app.AccountModel)
                self.assertEqual(query.inspect().params[0].kind, "i32")
                self.assertEqual(await updated.into_active().delete(connection), 1)
                self.assertIsNone(await query.optional(connection))

    # [spec:pgorm:req:bindings.codegen/test]
    # [spec:pgorm:req:bindings.entity-versions+3/test]
    async def test_generated_views_return_typed_versions(self):
        def active(identity, name):
            return app.Account.active().set_id(identity).set_display_name(name).set_note(None)

        renamed = p.Conflict.on("id").update("display name")
        async with self.pool.connection() as connection:
            first = await app.Account.insert(active(1, "One")).returning_upsert(connection)
            self.assertIsInstance(first, Inserted)
            self.assertIsInstance(first.model, app.AccountModel)
            change = await app.Account.update(
                first.model.into_active().set_note("noted")
            ).returning_change(connection)
            self.assertIsInstance(change, Change)
            self.assertIsInstance(change.new, app.AccountModel)
            self.assertEqual((change.old.note, change.new.note), (None, "noted"))
            changes = await (
                app.Account.update_many()
                .set("note", "bulk")
                .where(app.Account.col("id") == 1)
                .returning_changes(connection)
            )
            self.assertEqual([(c.old.note, c.new.note) for c in changes], [("noted", "bulk")])
            rows = await (
                app.Account.insert_many([active(1, "Again"), active(2, "Two")])
                .on_conflict(renamed)
                .returning_upserts(connection)
            )
            match rows:
                case [Updated(change), Inserted(model)]:
                    self.assertIsInstance(change.old, app.AccountModel)
                    self.assertEqual((change.old.display_name, change.new.display_name), ("One", "Again"))
                    self.assertEqual(model.display_name, "Two")
                case other:
                    self.fail(f"expected an update then an insert, got {other!r}")

    # [spec:pgorm:req:bindings.codegen/test]
    async def test_graph_views_keep_optional_models(self):
        async with self.pool.connection() as connection:
            for identity in (1, 2):
                await (
                    app.Account.active()
                    .set_id(identity)
                    .set_display_name("A")
                    .insert(connection)
                )
            await (
                app.Note.active()
                .set_id(10)
                .set_account_id(1)
                .set_body("B")
                .insert(connection)
            )
            query = app.AccountNotes.find(aliases=['notes "東京"'])
            rows = await query.order_by(query.col(0, "id").asc()).all(connection)
            self.assertEqual(
                [(a.id, None if n is None else n.id) for a, n in rows],
                [(1, 10), (2, None)],
            )
            self.assertIsInstance(rows[0][0], app.AccountModel)
            self.assertIsInstance(rows[0][1], app.NoteModel)
            page = await query.cursor("id").after_with(1, 10).first(1).all(connection)
            self.assertEqual([(a.id, n) for a, n in page], [(2, None)])
            self.assertIsInstance(
                await app.AccountOnly.find().optional(connection), app.AccountModel
            )
            required = await app.RequiredNotes.find().all(connection)
            self.assertEqual([(a.id, n.id) for a, n in required], [(1, 10)])

    # [spec:pgorm:req:bindings.codegen/test]
    async def test_views_reject_foreign_entities(self):
        with self.assertRaises(p.LifecycleError):
            app.AccountActive(p.entity("app.Note").active())
        with self.assertRaises(p.ConstructionError):
            app.Account.active().set_id(p.Value(1, "i64"))
        with self.assertRaises(p.ConstructionError):
            app.Account.active().set_mood("unknown")
        async with self.pool.connection() as connection:
            model = (
                await app.Note.active()
                .set_id(1)
                .set_account_id(1)
                .set_body("B")
                .insert(connection)
            )
            with self.assertRaises(p.LifecycleError):
                app.AccountModel(model.native)


HOUR = 3_600_000_000


class GeneratedIntervals(unittest.IsolatedAsyncioTestCase):
    """The wrappers over the entity pgorm-codegen generates from its interval
    schema, against the table that schema creates."""

    async def asyncSetUp(self):
        schema = Path(__file__).resolve().parents[2] / (
            "pgorm-codegen/tests/sql/interval/schema.sql"
        )
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        await self.pool.execute(p.RawSQL("DROP TABLE IF EXISTS interval_span"))
        await self.pool.execute(p.RawSQL(schema.read_text()))

    async def asyncTearDown(self):
        await self.pool.execute(p.RawSQL("DROP TABLE IF EXISTS interval_span"))
        await self.pool.close()

    # [spec:pgorm:req:python.codegen+3/test]
    async def test_interval_fields_are_pgorm_intervals(self):
        behind = p.Interval(14, -3, -(4 * HOUR + 1_234_567))
        ahead = p.Interval(-1, 2, HOUR + 3_000_004)
        active = (
            app.IntervalSpan.active()
            .set_id(1)
            .set_plain(behind)
            .set_maybe(None)
            .set_precise(behind)
            .set_months(behind)
            .set_days(behind)
            .set_clock(ahead)
            .set_minutes(behind)
            .set_seconds(ahead)
            .set_history([behind, ahead])
            .set_budgets([behind, ahead])
        )
        self.assertEqual(active.get("months").value.kind, "interval")
        async with self.pool.connection() as connection:
            model = await active.insert(connection)
            self.assertIsInstance(model, app.IntervalSpanModel)
            # Each column stores the value as its qualifier makes it: the
            # fields it leaves out dropped, seconds rounded to its precision.
            self.assertEqual(
                (model.plain, model.maybe, model.history),
                (behind, None, [behind, ahead]),
            )
            self.assertEqual(model.precise, p.Interval(14, -3, -(4 * HOUR + 1_235_000)))
            self.assertEqual(model.months, p.Interval(14, 0, 0))
            self.assertEqual(model.days, p.Interval(14, -3, 0))
            self.assertEqual(model.clock, p.Interval(-1, 2, HOUR + 3_000_000))
            self.assertEqual(model.minutes, p.Interval(14, -3, -4 * HOUR))
            self.assertEqual(model.seconds, p.Interval(-1, 2, HOUR + 3_000_000))
            self.assertEqual(model.budgets, [p.Interval(14, 0, 0), p.Interval(-1, 0, 0)])
            cleared = await model.into_active().set_budgets(None).update(connection)
            self.assertIsNone(cleared.budgets)
            found = await app.IntervalSpan.find().where(
                app.IntervalSpan.col("id") == 1
            ).one(connection)
            self.assertIsInstance(found, app.IntervalSpanModel)
            self.assertEqual(found.months, p.Interval(14, 0, 0))
        with self.assertRaises(p.ConstructionError):
            app.IntervalSpan.active().set_plain(p.Value(1, "i64"))


class GeneratedDomains(unittest.IsolatedAsyncioTestCase):
    """The wrappers over the entity pgorm-codegen generates from its domain
    schema, against the table that schema creates: each field is its domain's
    base, and each domain's checks hold what the wrappers write."""

    async def asyncSetUp(self):
        schema = Path(__file__).resolve().parents[2] / (
            "pgorm-codegen/tests/sql/domain/schema.sql"
        )
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        await self.drop()
        await self.pool.execute(p.RawSQL("CREATE SCHEMA billing"))
        # A raw statement is prepared, and a prepared statement is one
        # command, so the file runs a statement at a time.
        for statement in schema.read_text().split(";"):
            if statement.strip():
                await self.pool.execute(p.RawSQL(statement))

    async def drop(self):
        for statement in (
            "DROP TABLE IF EXISTS stock_line",
            "DROP SCHEMA IF EXISTS billing CASCADE",
            "DROP DOMAIN IF EXISTS small_quantity, quantity, code, cheerful, labels, span",
            "DROP TYPE IF EXISTS mood, floatrange",
        ):
            await self.pool.execute(p.RawSQL(statement))

    async def asyncTearDown(self):
        await self.drop()
        await self.pool.close()

    # [spec:pgorm:req:python.codegen+3/test]
    async def test_domain_fields_are_their_bases(self):
        active = (
            app.StockLine.active()
            .set_id(1)
            .set_qty(3)
            .set_small(40)
            .set_code("AB")
            .set_codes(["X", "YZ"])
            .set_mood("glad")
            .set_moods(["calm", "glad"])
            .set_labels(["new"])
            .set_span(None)
            .set_amount(Decimal("19.99"))
        )
        self.assertEqual(active.get("qty").value.kind, "i32")
        async with self.pool.connection() as connection:
            model = await active.insert(connection)
            self.assertIsInstance(model, app.StockLineModel)
            self.assertEqual(
                (model.qty, model.small, model.code, model.codes),
                (3, 40, "AB", ["X", "YZ"]),
            )
            self.assertEqual((model.mood, model.moods), ("glad", ["calm", "glad"]))
            self.assertEqual((model.labels, model.span), (["new"], None))
            self.assertEqual(model.amount, Decimal("19.99"))
            found = await app.StockLine.find().where(
                app.StockLine.col("small") > 10
            ).one(connection)
            self.assertEqual(found.codes, ["X", "YZ"])
            # The mood column's domain is over an enum, which PostgreSQL
            # compares only once both sides are cast to the enum.
            glad = await app.StockLine.find().where(
                app.StockLine.col("mood") == "glad"
            ).all(connection)
            self.assertEqual([line.id for line in glad], [1])
            calmer = await app.StockLine.find().where(
                app.StockLine.col("mood") < "glad"
            ).all(connection)
            self.assertEqual(calmer, [])
            with self.assertRaises(p.DatabaseError) as refused:
                await active.set_id(2).set_small(150).insert(connection)
            self.assertEqual(refused.exception.sqlstate, "23514")


    # [spec:pgorm:req:bindings.pipeline-sources+3/test]    a registered column a pipeline reads compares
    # as the enum its domain is over, under its table and under a source's name, where the
    # column named by text is refused with 42883
    async def test_pipeline_compares_registered_domain_as_enum(self):
        def line(identifier, mood):
            return (
                app.StockLine.active()
                .set_id(identifier)
                .set_qty(1)
                .set_small(1)
                .set_code("A")
                .set_codes([])
                .set_mood(mood)
                .set_moods(None)
                .set_labels(None)
                .set_span(None)
                .set_amount(Decimal("1"))
            )

        stock = p.entity("app.StockLine")
        mood = app.StockLine.col("mood")
        identifier = pl.col("stock_line", "id")
        async with self.pool.connection() as connection:
            for number, label in ((1, "glad"), (2, "calm"), (3, "glad")):
                await line(number, label).insert(connection)

            async def ids(query):
                rows = await query.select(identifier).all(connection)
                return sorted(row["id"] for row in rows)

            typed = pl.col("stock_line", mood)
            glad = pl.from_(stock).filter(typed == pl.literal("glad"))
            self.assertIn("CAST(mood AS mood) = CAST('glad' AS mood)", glad.inspect().sql)
            self.assertEqual(await ids(glad), [1, 3])
            plain = pl.from_(stock).filter(typed == "glad")
            self.assertIn("CAST(mood AS mood) = CAST($1::text AS mood)", plain.inspect().sql)
            self.assertEqual(await ids(plain), [1, 3])
            bound = pl.from_(stock).filter_with(lambda binder: typed < binder.bind("glad"))
            self.assertEqual(await ids(bound), [2])
            peer = pl.source(stock).named("peer")
            joined = (
                pl.from_(stock)
                .join(peer, typed == pl.col("peer", mood))
                .filter(pl.col("peer", "id") == 3)
            )
            self.assertEqual(await ids(joined), [1, 3])
            for value in ("glad", pl.literal("glad")):
                with self.assertRaises(p.DatabaseError) as refused:
                    await ids(pl.from_(stock).filter(pl.col("stock_line", "mood") == value))
                self.assertEqual(refused.exception.sqlstate, "42883")

class GeneratedDisjoints(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=2)
        await self.pool.execute(p.RawSQL("CREATE SCHEMA python_courier"))
        for statement in s.create_disjoint(app.Deliveries):
            await self.pool.execute(statement)
        for statement in (
            "CREATE TABLE python_courier.offers (id integer PRIMARY KEY, title text NOT NULL)",
            "CREATE TABLE python_courier.auctions (offer_id integer PRIMARY KEY, reserve integer NOT NULL)",
            "CREATE TABLE python_courier.sales (id integer PRIMARY KEY, price integer NOT NULL)",
            "INSERT INTO python_courier.offers VALUES (1, 'lamp'), (2, 'desk')",
            "INSERT INTO python_courier.auctions VALUES (1, 50)",
        ):
            await self.pool.execute(p.RawSQL(statement))

    async def asyncTearDown(self):
        await self.pool.execute(p.RawSQL("DROP SCHEMA python_courier CASCADE"))
        await self.pool.close()

    # [spec:pgorm:req:bindings.codegen-disjoint/test]    a generated disjoint view reads each row as the generated models of its base and kind, writes through their generated ActiveModels, pages and narrows as a registration does, and an unkinded enum's view reads and offers no write
    async def test_disjoint_views_read_and_write_generated_models(self):
        deliveries = app.Deliveries
        boxed = await deliveries.insert(
            app.Delivery.active().set_label("box"), "parcel", app.Parcel.active().set_weight_g(500)
        ).returning_model(self.pool)
        base, kind, parcel = boxed
        self.assertIsInstance(base, app.DeliveryModel)
        self.assertEqual((base.label, kind), ("box", "parcel"))
        self.assertIsInstance(parcel, app.ParcelModel)
        self.assertEqual(parcel.weight_g, 500)
        letter = await deliveries.insert(app.Delivery.active().set_label("note"), "letter").returning_model(self.pool)
        self.assertEqual((letter[1], letter[2]), ("letter", None))
        moved = await deliveries.change_kind(
            {"id": base.id}, "pallet", app.Pallet.active().set_volume_l(12).set_fragile(None)
        ).returning_model(self.pool)
        self.assertIsInstance(moved[2], app.PalletModel)
        ordered = deliveries.find().order_by(app.Delivery.col("id").asc())
        rows = await ordered.all(self.pool)
        self.assertEqual([(row[0].id, row[1]) for row in rows], [(1, "pallet"), (2, "letter")])
        self.assertEqual(await ordered.kind("letter").count(self.pool), 1)
        self.assertEqual([[row[1] for row in page] async for page in ordered.paginate(1).pages(self.pool)],
                         [["pallet"], ["letter"]])
        self.assertEqual(deliveries.describe()["kind_field"], "kind")

        offers = app.Offers
        self.assertFalse(hasattr(offers, "insert"))
        read = await offers.find().order_by(app.Offer.col("id").asc()).all(self.pool)
        self.assertIsInstance(read[0][0], app.OfferModel)
        self.assertIsInstance(read[0][2], app.AuctionModel)
        self.assertEqual([(row[0].title, row[1]) for row in read], [("lamp", "auction"), ("desk", "draft")])
        self.assertEqual((await offers.find_by_key({"id": 2}).one(self.pool))[2], None)
        self.assertIsNone(offers.describe()["kind_field"])
        with self.assertRaises(p.ConstructionError):
            s.create_disjoint(offers)


class GeneratedContract(unittest.TestCase):
    # [spec:pgorm:req:bindings.codegen/test]
    # [spec:pgorm:req:bindings.codegen-disjoint/test]    the load-time check compares each registered disjoint enum's describe() as it compares an entity's
    def test_compatibility_fails_before_queries(self):
        for key in ("package_version", "pgorm_version", "registry_abi", "features"):
            expected = copy.deepcopy(app._COMPATIBILITY)
            expected[key] = "mismatch"
            with self.subTest(key=key), self.assertRaises(p.UnsupportedCapabilityError):
                check(expected)
        changed = {"entities": "table", "graphs": "sources", "disjoints": "kinds"}
        for family, key in changed.items():
            expected = copy.deepcopy(app._COMPATIBILITY)
            expected[family][0][key] = "changed"
            with (
                self.subTest(family=family),
                self.assertRaises(p.UnsupportedCapabilityError),
            ):
                check(expected)
        code = inspect.getsource(app)
        code = code.replace(
            "_check(_COMPATIBILITY)",
            "_COMPATIBILITY['registry_abi'] = -1\n_check(_COMPATIBILITY)",
        )
        with self.assertRaises(p.UnsupportedCapabilityError):
            exec(compile(code, "incompatible.py", "exec"), {})

    # [spec:pgorm:req:python.codegen+3/test]
    def test_generated_types_follow_compiled_metadata(self):
        stub = Path(app.__file__).with_suffix(".pyi").read_text()
        self.assertIn("def id(self) -> int:", stub)
        self.assertIn("def display_name(self) -> str:", stub)
        self.assertIn("def note(self) -> str | None:", stub)
        self.assertIn(
            "AccountNotesRow: TypeAlias = tuple[AccountModel, NoteModel | None]", stub
        )
        self.assertIn(
            "RequiredNotesRow: TypeAlias = tuple[AccountModel, NoteModel]", stub
        )
        self.assertIn("AccountOnlyRow: TypeAlias = AccountModel", stub)
        self.assertIn("def months(self) -> p.Interval:", stub)
        self.assertIn("def maybe(self) -> p.Interval | None:", stub)
        self.assertIn("def budgets(self) -> list[p.Interval] | None:", stub)
        self.assertIn(
            "def set_history(self, value: list[p.Interval] | p.Value) -> IntervalSpanActive:",
            stub,
        )
        self.assertIn("def qty(self) -> int | None:", stub)
        self.assertIn("def small(self) -> int:", stub)
        self.assertIn("def codes(self) -> list[str]:", stub)
        self.assertIn("def labels(self) -> list[str] | None:", stub)
        self.assertIn("def amount(self) -> Decimal:", stub)
        self.assertIn(
            "DeliveriesRow: TypeAlias = tuple[DeliveryModel, Literal['parcel'], ParcelModel] "
            "| tuple[DeliveryModel, Literal['letter'], None] "
            "| tuple[DeliveryModel, Literal['pallet'], PalletModel]",
            stub,
        )
        self.assertIn(
            "Deliveries: WritableDisjointView[DeliveriesRow, Literal['parcel', 'letter', 'pallet'], "
            "DeliveryActive, ParcelActive | PalletActive]",
            stub,
        )
        self.assertIn("Offers: DisjointView[OffersRow]", stub)
        described = app.Account.describe()
        columns = {column["name"]: column for column in described["columns"]}
        self.assertEqual(columns["id"]["rust_type"], "i32")
        self.assertEqual(columns["display name"]["json_key"], "displayName")
        self.assertEqual(described["schema"], "python_entities")
