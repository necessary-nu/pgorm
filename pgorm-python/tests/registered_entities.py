"""Run only against the independent application-binding wheel."""

import asyncio
from decimal import Decimal
import os
import unittest

import pgorm as p
from pgorm import pipeline as pl
from pgorm import schema as s
from pgorm._registered.versions import InsertManyView, InsertView, UpdateManyView, UpdateView


class RegisteredEntities(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        self.created = False
        await self.pool.execute(p.RawSQL('CREATE SCHEMA python_entities'))
        self.created = True
        await self.pool.execute(p.RawSQL('CREATE TYPE python_entities."Mood" AS ENUM (\'calm\', \'busy\')'))
        await self.pool.execute(p.RawSQL('CREATE TABLE python_entities.accounts (id integer PRIMARY KEY, "display name" text NOT NULL, note text, version integer NOT NULL, mood python_entities."Mood" NOT NULL)'))
        await self.pool.execute(p.RawSQL('CREATE TABLE python_entities.notes (id integer PRIMARY KEY, account_id integer NOT NULL, body text NOT NULL)'))
        await self.pool.execute(p.RawSQL('CREATE TYPE python_entities.floatrange AS RANGE (SUBTYPE = float8)'))
        await self.pool.execute(p.RawSQL('CREATE TABLE python_entities.bookings (id integer PRIMARY KEY, span python_entities.floatrange NOT NULL, spans python_entities.floatmultirange, spares python_entities.floatrange[], ledgers python_entities.floatmultirange[])'))
        self.account = p.entity("app.Account")
        self.note = p.entity("app.Note")

    async def asyncTearDown(self):
        if self.created:
            await self.pool.execute(p.RawSQL('DROP SCHEMA python_entities CASCADE'))
        await self.pool.close()

    def active(self, identity, name="Nora"):
        return self.account.active().set("id", identity).set("display name", name)

    # [spec:pgorm:req:bindings.entities+5/test]
    async def test_registration_declares_real_types_and_mapping(self):
        registrations = p.capabilities()["registrations"]["entities"]
        self.assertEqual([r["name"] for r in registrations],
                         ["app.Account", "app.Auction", "app.Booking", "app.Label", "app.Listing", "app.Membership",
                          "app.Note", "app.Old", "app.Pallet", "app.Parcel", "app.Room", "app.Sale",
                          "app.Shipment", "app.Stay", "app.Temper", "app.Till"])
        self.assertEqual(p.entities(), [r["name"] for r in registrations])
        info = self.account.describe()
        self.assertTrue(info["rust"]["entity"].endswith("account::Entity"))
        self.assertTrue(info["rust"]["model"].endswith("account::Model"))
        self.assertTrue(info["rust"]["active_model"].endswith("account::ActiveModel"))
        self.assertTrue(info["rust"]["column"].endswith("account::Column"))
        self.assertEqual((info["name"], info["schema"], info["table"]), ("app.Account", "python_entities", "accounts"))
        columns = {column["name"]: column for column in info["columns"]}
        self.assertEqual(columns["display name"]["json_key"], "displayName")
        self.assertEqual(columns["display name"]["rust_type"], "String")
        self.assertTrue(columns["id"]["primary_key"])
        self.assertTrue(columns["note"]["nullable"])
        self.assertEqual([(column["name"], column["kind"], column["values"]) for column in info["columns"]], [
            ("id", "i32", None), ("display name", "text", None), ("note", "text", None), ("version", "i32", None),
            ("mood", 'enum "python_entities"."Mood"', ["calm", "busy"]),
        ])
        self.assertEqual(info["unique_keys"], [])
        # [spec:pgorm:req:bindings.entities+5/test]    a registration's describe() reports the unique keys its entity's unique(..) declares, each its name, its columns by their SQL names in key order and nulls_not_distinct
        self.assertEqual(
            p.entity("app.Label").describe()["unique_keys"],
            [{"name": "labels_text_parcel", "columns": ["text", "parcel_id"], "nulls_not_distinct": True}],
        )
        with self.assertRaises(p.UnsupportedCapabilityError):
            p.entity("invented.Entity")
        with self.assertRaises(p.ConstructionError):
            self.account.col("missing")

    # [spec:pgorm:req:python.capabilities+2/test]
    async def test_manifest_declares_its_reachable_terminals(self):
        terminals = [
            "all", "one", "optional", "active.insert", "active.update", "active.delete",
            "update.returning_change", "update_many.returning_changes",
            "insert.returning_upsert", "insert_many.returning_upserts",
        ]
        operations = p.capabilities()["operations"]
        self.assertEqual([t for t in terminals if "entity." + t not in operations], [])
        active = self.active(1)
        owners = {
            "": self.account.find(), "active": active, "update": self.account.update(active),
            "update_many": self.account.update_many(), "insert": self.account.insert(active),
            "insert_many": self.account.insert_many([active]),
        }
        for terminal in terminals:
            owner, _, method = terminal.rpartition(".")
            with self.subTest(terminal=terminal):
                self.assertTrue(callable(getattr(owners[owner], method)))

    # [spec:pgorm:req:python.truth-testing+3/test]
    async def test_write_terminals_refuse_truth_testing(self):
        active = self.active(1)
        writes = [self.account.update(active), self.account.update_many(),
                  self.account.insert(active), self.account.insert_many([active])]
        typed = [view(write, dict) for view, write in zip(
            (UpdateView, UpdateManyView, InsertView, InsertManyView), writes)]
        for write in writes + typed:
            with self.subTest(write=type(write).__name__), self.assertRaises(p.ConstructionError):
                bool(write)

    # [spec:pgorm:req:bindings.entity-writes/test]
    async def test_active_values_and_hooks_use_rust_states(self):
        initial = self.account.active()
        self.assertEqual(initial.get("id").state, p.ActiveState.NotSet)
        self.assertEqual(initial.get("version").state, p.ActiveState.Set)
        self.assertEqual(initial.get("version").value.value, 1)
        active = self.active(1).set("note", None).set("mood", "busy")
        self.assertEqual(initial.get("id").state, p.ActiveState.NotSet)
        self.assertEqual(active.get("note").state, p.ActiveState.Set)
        self.assertTrue(active.get("note").value.is_null)
        self.assertEqual(active.get("mood").value.type_name, p.TypeName("Mood", schema="python_entities"))
        async with self.pool.connection() as connection:
            model = await active.insert(connection)
            self.assertEqual(dict(model), {"id": 1, "display name": "Nora|before", "note": None, "version": 1, "mood": "busy"})
            loaded = model.into_active()
            self.assertEqual(loaded.get("id").state, p.ActiveState.Unchanged)
            self.assertEqual(loaded.get("note").state, p.ActiveState.Unchanged)
            changed = loaded.set("display name", "Updated").not_set("note")
            updated = await changed.update(connection)
            self.assertEqual(updated["version"], 2)
            self.assertEqual(updated["display name"], "Updated")
            self.assertEqual(model["display name"], "Nora|before")
            self.assertEqual(loaded.reset("id").get("id").state, p.ActiveState.Set)
            self.assertEqual(loaded.get("id").state, p.ActiveState.Unchanged)
            self.assertEqual(await updated.into_active().delete(connection), 1)
            self.assertIsNone(await self.account.find().optional(connection))
        self.assertEqual(model.tagged("mood").type_name, p.TypeName("Mood", schema="python_entities"))

    # [spec:pgorm:req:bindings.entity-writes/test]
    # [spec:pgorm:req:bindings.created-ranges+5/test]
    async def test_created_ranges_round_trip_through_an_entity(self):
        booking = p.entity("app.Booking")
        span = p.CreatedRange("floatrange", "f64", schema="python_entities")
        spans = p.CreatedMultirange("floatmultirange", "f64", schema="python_entities")
        kinds = {column["name"]: column["kind"] for column in booking.describe()["columns"]}
        self.assertEqual(kinds["span"], 'range "python_entities"."floatrange" of f64')
        self.assertEqual(kinds["spans"], 'multirange "python_entities"."floatmultirange" of f64')
        async with self.pool.connection() as connection:
            active = (booking.active().set("id", 1).set("span", p.Range(1.5, 2.5))
                      .set("spans", p.Multirange([p.Range(5.0, 8.0), p.Range(1.0, 3.0)])))
            self.assertEqual(active.get("span").value.created_type, span)
            await active.insert(connection)
            found = await booking.find().where(booking.col("span") == p.Range(1.5, 2.5)).one(connection)
            self.assertEqual(found["span"], p.Range(1.5, 2.5))
            # The server stores a multirange sorted and merged.
            self.assertEqual(found["spans"], p.Multirange([p.Range(1.0, 3.0), p.Range(5.0, 8.0)]))
            self.assertEqual(found.tagged("span").created_type, span)
            self.assertEqual(found.tagged("spans").created_type, spans)
            row = await connection.one(p.RawSQL("SELECT span FROM python_entities.bookings"))
            self.assertEqual(row["span"], p.Range(1.5, 2.5))
            self.assertEqual(row.tagged("span"), found.tagged("span"))
            moved = await found.into_active().set("span", p.Range(None, 0.5, "(]")).set("spans", None).update(connection)
            self.assertEqual((moved["span"], moved["spans"]), (p.Range(None, 0.5, "(]"), None))
            with self.assertRaises(p.ConstructionError):
                booking.active().set("span", p.Value(p.Range(1.0, 2.0), p.CreatedRange("floatrange", "f64")))
            with self.assertRaises(p.ConstructionError):
                booking.active().set("span", p.Range(1, 2))

    # [spec:pgorm:req:bindings.created-ranges+5/test]    a registered column of an array of a created range or
    # multirange takes a list of the kind's values, the empty list and NULL elements included, as the array of
    # their texts cast to the type's array, compares through the column's casts, and reads back as a list, a
    # declared Value of the array's kind included
    async def test_created_range_arrays_bind_as_text_arrays(self):
        booking = p.entity("app.Booking")
        kinds = {column["name"]: column["kind"] for column in booking.describe()["columns"]}
        self.assertEqual(kinds["spares"], 'range "python_entities"."floatrange" of f64[]')
        self.assertEqual(kinds["ledgers"], 'multirange "python_entities"."floatmultirange" of f64[]')
        spares = booking.col("spares")
        async with self.pool.connection() as connection:
            await (booking.active().set("id", 1).set("span", p.Range(1.0, 2.0))
                   .set("spares", [p.Range(1.5, 2.5), p.Range(3.0, 4.0)])
                   .set("ledgers", [p.Multirange([p.Range(5.0, 8.0), p.Range(1.0, 3.0)]), p.Multirange([])])
                   .insert(connection))
            await booking.active().set("id", 2).set("span", p.Range(1.0, 2.0)).set("spares", []).insert(connection)
            found = await booking.find().where(booking.col("id") == 1).one(connection)
            self.assertEqual(found["spares"], [p.Range(1.5, 2.5), p.Range(3.0, 4.0)])
            # The server stores a multirange sorted and merged.
            self.assertEqual(found["ledgers"], [p.Multirange([p.Range(1.0, 3.0), p.Range(5.0, 8.0)]), p.Multirange([])])

            async def ids(predicate):
                return sorted(row["id"] for row in await booking.find().where(predicate).all(connection))

            self.assertIn("= CAST($1::text[] AS python_entities.floatrange[])",
                          (spares == [p.Range(1.5, 2.5)]).inspect().sql)
            self.assertEqual(await ids(spares == [p.Range(1.5, 2.5), p.Range(3.0, 4.0)]), [1])
            self.assertEqual(await ids(spares == []), [2])
            self.assertEqual(await ids(spares != [p.Range(1.5, 2.5), None]), [1, 2])
            self.assertEqual(await ids(spares.contains([p.Range(3.0, 4.0)])), [1])
            moved = await (found.into_active().set("spares", found.tagged("spares")).set("ledgers", None)
                           .update(connection))
            self.assertEqual((moved["spares"], moved["ledgers"]), (found["spares"], None))
        with self.assertRaises(p.ConstructionError):
            booking.active().set("spares", [p.Range(1, 2)])
        with self.assertRaises(p.ConstructionError):
            booking.active().set("spares", found.tagged("ledgers"))
        with self.assertRaises(p.ConstructionError):
            booking.active().set("span", found.tagged("spares"))
        declared = p.Value.array(p.CreatedRange("floatrange", "f64", schema="python_entities"), [p.Range(5.0, 6.0)])
        self.assertEqual(booking.active().set("spares", declared).get("spares").value, declared)
        with self.assertRaises(p.ConstructionError):
            booking.active().set("spares", p.Value.array(p.CreatedRange("floatrange", "f64"), []))

    # [spec:pgorm:req:bindings.entity-reads+3/test]
    # [spec:pgorm:req:bindings.column-operators+3/test]    a registered column is its own expression,
    # every Expr method reading it as written, and describes nothing itself
    async def test_typed_select_reuses_real_entity_projection(self):
        async with self.pool.connection() as connection:
            for identity in (1, 2, 3):
                await self.active(identity, f"account{identity}").insert(connection)
            column = self.account.col("id")
            self.assertIsInstance(column, p.Expr)
            self.assertFalse(hasattr(column, "describe"))
            self.assertEqual(column.desc().expr.inspect().sql, column.expr().inspect().sql)
            self.assertEqual((column + 1).inspect().params[0].kind, "i64")
            self.assertEqual(
                [row["id"] for row in await self.account.find().where(column.between(2, 3)).order_by(column.asc()).all(connection)],
                [2, 3],
            )
            query = self.account.find().where(self.account.col("id") >= 2)
            query = query.order_by(column.desc())
            self.assertEqual([row["id"] for row in await query.all(connection)], [3, 2])
            self.assertEqual((await query.one(connection))["id"], 3)
            self.assertEqual(len(await query.all(connection)), 2)
            self.assertEqual([row["id"] for row in await query.offset(1).limit(1).all(connection)], [2])
            self.assertEqual((await self.account.find().where(self.account.col("mood") == "calm").one(connection))["mood"], "calm")
            none = query.where(self.account.col("id") == 100)
            self.assertEqual(await none.all(connection), [])
            self.assertIsNone(await none.optional(connection))
            with self.assertRaises(p.DecodeError):
                await none.one(connection)
            self.assertNotEqual(query.inspect().sql, query.inspect(terminal="one").sql)
            self.assertEqual(query.inspect().params[0].kind, "i32")

    # [spec:pgorm:req:bindings.entity-reads+3/test]    a registered column is a member of values and of
    # a Select's rows through its ColumnTrait, a column whose domain is over an enum as the enum,
    # where the column's plain expression is refused with 42883
    async def test_column_membership_runs_through_column_trait(self):
        await self.pool.execute(p.RawSQL('CREATE DOMAIN python_entities.temper AS python_entities."Mood"'))
        await self.pool.execute(p.RawSQL(
            "CREATE TABLE python_entities.tempers "
            "(mood python_entities.temper PRIMARY KEY, settles_into python_entities.temper NOT NULL)"))
        await self.pool.execute(p.RawSQL(
            "INSERT INTO python_entities.tempers VALUES ('calm', 'busy'), ('busy', 'busy')"))
        temper = p.entity("app.Temper")
        mood = temper.col("mood")
        tempers = p.Table("tempers", schema="python_entities")
        settled = p.Select(tempers.col("settles_into")).from_(tempers)
        async with self.pool.connection() as connection:
            async def moods(predicate):
                return sorted(row["mood"] for row in await temper.find().where(predicate).all(connection))

            self.assertEqual(await moods(mood.is_in(["busy"])), ["busy"])
            self.assertEqual(await moods(mood.is_not_in(("busy",))), ["calm"])
            self.assertEqual(await moods(mood.is_in(settled)), ["busy"])
            self.assertEqual(await moods(mood.is_not_in(settled)), ["calm"])
            self.assertEqual(await moods(mood.is_in([])), [])
            with self.assertRaises(p.DatabaseError) as refused:
                await temper.find().where(mood.expr().is_in(settled)).all(connection)
            self.assertEqual(refused.exception.sqlstate, "42883")
            for identity in (1, 2, 3):
                await self.active(identity, f"account{identity}").insert(connection)
            chosen = self.account.find().where(self.account.col("id").is_in((1, 3)))
            self.assertEqual(sorted(row["id"] for row in await chosen.all(connection)), [1, 3])
        with self.assertRaises(TypeError):
            mood.is_in("busy")

    # [spec:pgorm:req:bindings.entity-reads+3/test]    a registered column compares with an expression, or
    # any_(..) / all_(..) over one, as written, and a list holding an expression is a member as written
    async def test_expression_operands_compare_as_written(self):
        identifier, version = self.account.col("id"), self.account.col("version")
        self.assertEqual(
            (identifier == version).inspect().sql,
            'SELECT "accounts"."id" = "accounts"."version"',
        )
        self.assertEqual(
            identifier.is_in([p.literal(1), 3]).inspect().sql,
            'SELECT "accounts"."id" IN (1, $1)',
        )
        async with self.pool.connection() as connection:
            for identity in (1, 2, 3):
                await self.active(identity, f"account{identity}").insert(connection)

            async def ids(predicate):
                return sorted(row["id"] for row in await self.account.find().where(predicate).all(connection))

            self.assertEqual(await ids(identifier == version), [1])
            self.assertEqual(await ids(identifier.is_distinct_from(version)), [2, 3])
            self.assertEqual(await ids(identifier > p.literal(1)), [2, 3])
            versions = p.array(p.Select(version).from_(p.Table("accounts", schema="python_entities")))
            self.assertEqual(await ids(identifier == p.any_(versions)), [1])
            self.assertEqual(await ids(identifier == p.any_([1, 3])), [1, 3])
            self.assertEqual(await ids(identifier != p.all_([1, 3])), [2])
            self.assertEqual(await ids(identifier.is_in([p.literal(1), 3])), [1, 3])
            self.assertEqual(await ids(identifier.is_not_in((version, 3))), [2])

    # [spec:pgorm:req:bindings.entities+5/test]    a money column, and an array of it, is a decimal
    # column: described as one, set, compared and read back as a Decimal through the entity's own
    # casts, and compared as money by a pipeline that reads the registered column, where the column
    # named by text takes no Decimal
    async def test_money_columns_are_decimals(self):
        await self.pool.execute(p.RawSQL(
            "CREATE TABLE python_entities.tills (id integer PRIMARY KEY, takings money NOT NULL, floats money[])"))
        till = p.entity("app.Till")
        kinds = [(column["name"], column["kind"]) for column in till.describe()["columns"]]
        self.assertEqual(kinds, [("id", "i32"), ("takings", "decimal"), ("floats", "decimal[]")])
        takings = till.col("takings")
        async with self.pool.connection() as connection:
            written = await (till.active().set("id", 1).set("takings", Decimal("12.345"))
                             .set("floats", [Decimal("1.5"), Decimal("2")]).insert(connection))
            self.assertEqual((written["takings"], written["floats"]),
                             (Decimal("12.35"), [Decimal("1.50"), Decimal("2.00")]))
            self.assertEqual(written.tagged("takings").kind, "decimal")
            await till.active().set("id", 2).set("takings", Decimal("3")).insert(connection)

            async def ids(predicate):
                return sorted(row["id"] for row in await till.find().where(predicate).all(connection))

            self.assertEqual(await ids(takings == Decimal("12.35")), [1])
            self.assertEqual(await ids(takings < Decimal("10")), [2])
            self.assertEqual(await ids(takings.is_in([Decimal("3"), Decimal("12.35")])), [1, 2])
            self.assertEqual(await ids(till.col("floats") == [Decimal("1.5"), Decimal("2")]), [1])

            column = pl.col("tills", takings)
            identifier = pl.col("tills", "id")

            async def piped(query):
                return sorted(row["id"] for row in await query.select(identifier).all(connection))

            self.assertIn("CAST($1::numeric AS money)", pl.from_(till).filter(column > Decimal("5")).inspect().sql)
            self.assertEqual(await piped(pl.from_(till).filter(column > Decimal("5"))), [1])
            self.assertEqual(await piped(pl.from_(till).filter_with(lambda binder: column <= binder.bind(3))), [2])
            with self.assertRaises(p.ConstructionError):
                await piped(pl.from_(till).filter(pl.col("tills", "takings") > Decimal("5")))

    # [spec:pgorm:req:bindings.column-operators+3/test]    a registered column's between, symmetric or
    # not, and its distinctness tests run through its ColumnTrait: a money column's bounds cast to money,
    # an enum's labels to the enum, and a column whose domain is over an enum tested as the enum, where
    # the column's plain expression is refused; an expression bound is taken as written, null never
    async def test_between_and_distinctness_take_the_column_casts(self):
        await self.pool.execute(p.RawSQL(
            "CREATE TABLE python_entities.tills (id integer PRIMARY KEY, takings money NOT NULL, floats money[])"))
        await self.pool.execute(p.RawSQL('CREATE DOMAIN python_entities.temper AS python_entities."Mood"'))
        await self.pool.execute(p.RawSQL(
            "CREATE TABLE python_entities.tempers "
            "(mood python_entities.temper PRIMARY KEY, settles_into python_entities.temper NOT NULL)"))
        await self.pool.execute(p.RawSQL(
            "INSERT INTO python_entities.tempers VALUES ('calm', 'busy'), ('busy', 'busy')"))
        till, temper = p.entity("app.Till"), p.entity("app.Temper")
        takings, temper_mood, mood = till.col("takings"), temper.col("mood"), self.account.col("mood")
        async with self.pool.connection() as connection:
            for identity, amount in ((1, "12.35"), (2, "3"), (3, "7.5")):
                await till.active().set("id", identity).set("takings", Decimal(amount)).insert(connection)
            await self.active(1).insert(connection)
            await self.active(2).set("mood", "busy").insert(connection)

            async def found(entity, predicate, key="id"):
                return sorted(row[key] for row in await entity.find().where(predicate).all(connection))

            low, high = Decimal("3"), Decimal("8")
            self.assertIn("BETWEEN CAST($1::numeric AS money) AND CAST($2::numeric AS money)",
                          till.find().where(takings.between(low, high)).inspect().sql)
            self.assertEqual(await found(till, takings.between(low, high)), [2, 3])
            self.assertEqual(await found(till, takings.not_between(low, high)), [1])
            self.assertEqual(await found(till, takings.between(high, low)), [])
            self.assertEqual(await found(till, takings.between(high, low, symmetric=True)), [2, 3])
            self.assertEqual(await found(till, takings.not_between(high, low, symmetric=True)), [1])
            self.assertEqual(await found(till, takings.is_distinct_from(low)), [1, 3])
            self.assertEqual(await found(till, takings.is_not_distinct_from(low)), [2])
            self.assertEqual(await found(till, takings.between(takings.expr(), takings)), [1, 2, 3])

            self.assertEqual(await found(self.account, mood.between("busy", "busy")), [2])
            self.assertEqual(await found(self.account, mood.not_between("busy", "calm", symmetric=True)), [])
            self.assertEqual(await found(self.account, mood.is_distinct_from("busy")), [1])
            self.assertEqual(await found(self.account, mood.is_not_distinct_from("busy")), [2])

            self.assertEqual(await found(temper, temper_mood.between("calm", "calm"), "mood"), ["calm"])
            self.assertEqual(await found(temper, temper_mood.between("busy", "calm", symmetric=True), "mood"),
                             ["busy", "calm"])
            self.assertEqual(await found(temper, temper_mood.is_distinct_from("calm"), "mood"), ["busy"])
            self.assertEqual(await found(temper, temper_mood.is_not_distinct_from("busy"), "mood"), ["busy"])
            for plain in (temper_mood.expr().between("calm", "busy"), temper_mood.expr().is_distinct_from("calm")):
                with self.assertRaises(p.DatabaseError) as refused:
                    await temper.find().where(plain).all(connection)
                self.assertEqual(refused.exception.sqlstate, "42883")
        with self.assertRaises(p.ConstructionError):
            takings.between(None, high)
        with self.assertRaises(p.ConstructionError):
            takings.is_distinct_from(None)
        with self.assertRaises(p.ConstructionError):
            takings.eq(None)

    # [spec:pgorm:req:bindings.entity-reads+3/test]    any_(..) / all_(..) over a list compares through the
    # column's ColumnTrait, the list one array of its declared kind cast as eq_any's is: money as money[],
    # a column whose domain is over an enum as the enum, where the plain expression is refused
    # [spec:pgorm:req:bindings.quantifiers+3/test]    an empty list is typed by the registered column, = ANY
    # of it false and <> ALL true, and refused as it is used where no registered column types it
    async def test_quantified_lists_take_the_column_casts(self):
        await self.pool.execute(p.RawSQL(
            "CREATE TABLE python_entities.tills (id integer PRIMARY KEY, takings money NOT NULL, floats money[])"))
        await self.pool.execute(p.RawSQL('CREATE DOMAIN python_entities.temper AS python_entities."Mood"'))
        await self.pool.execute(p.RawSQL(
            "CREATE TABLE python_entities.tempers "
            "(mood python_entities.temper PRIMARY KEY, settles_into python_entities.temper NOT NULL)"))
        await self.pool.execute(p.RawSQL(
            "INSERT INTO python_entities.tempers VALUES ('calm', 'busy'), ('busy', 'busy')"))
        till, temper = p.entity("app.Till"), p.entity("app.Temper")
        takings, temper_mood, mood = till.col("takings"), temper.col("mood"), self.account.col("mood")
        self.assertIn("= ANY(CAST($1::numeric[] AS money[]))",
                      (takings == p.any_([Decimal("3")])).inspect().sql)
        async with self.pool.connection() as connection:
            for identity, amount in ((1, "12.35"), (2, "3"), (3, "7.5")):
                await till.active().set("id", identity).set("takings", Decimal(amount)).insert(connection)
            await self.active(1).insert(connection)
            await self.active(2).set("mood", "busy").insert(connection)

            async def found(entity, predicate, key="id"):
                return sorted(row[key] for row in await entity.find().where(predicate).all(connection))

            self.assertEqual(await found(till, takings == p.any_([Decimal("3"), Decimal("12.35")])), [1, 2])
            self.assertEqual(await found(till, takings > p.all_([Decimal("1"), Decimal("5")])), [1, 3])
            self.assertEqual(await found(till, takings == p.any_([])), [])
            self.assertEqual(await found(till, takings != p.all_([])), [1, 2, 3])
            self.assertEqual(await found(self.account, mood != p.all_(["busy"])), [1])
            self.assertEqual(await found(temper, temper_mood == p.any_(["busy"]), "mood"), ["busy"])
            self.assertEqual(await found(temper, temper_mood > p.all_(["calm"]), "mood"), ["busy"])
            with self.assertRaises(p.DatabaseError) as refused:
                await temper.find().where(temper_mood.expr() == p.any_(["busy"])).all(connection)
            self.assertEqual(refused.exception.sqlstate, "42883")
        empty = p.any_([])
        with self.assertRaises(p.ConstructionError):
            p.col("x") == empty
        with self.assertRaises(p.ConstructionError):
            takings.expr() == p.all_([])

    # [spec:pgorm:req:bindings.column-operators+3/test]    a registered range column's containment, overlap
    # and range positions run through its ColumnTrait, a created range written as its text cast to the type,
    # any_/all_ over a list of ranges as the array of their texts cast to the type's array, and an element
    # is no value of the column's kind
    async def test_range_operators_take_the_column_casts(self):
        booking = p.entity("app.Booking")
        span = booking.col("span")
        async with self.pool.connection() as connection:
            for identity, (lower, upper) in ((1, (1.5, 2.5)), (2, (5.0, 6.0))):
                await booking.active().set("id", identity).set("span", p.Range(lower, upper)).insert(connection)

            async def ids(predicate):
                return sorted(row["id"] for row in await booking.find().where(predicate).all(connection))

            self.assertIn("&& CAST($1::text AS python_entities.floatrange)",
                          span.overlaps(p.Range(2.0, 3.0)).inspect().sql)
            self.assertEqual(await ids(span.overlaps(p.Range(2.0, 3.0))), [1])
            self.assertEqual(await ids(span.contains(p.Range(1.6, 1.7))), [1])
            self.assertEqual(await ids(span.contained_by(p.Range(0.0, 10.0))), [1, 2])
            self.assertEqual(await ids(span.strictly_left_of(p.Range(4.0, 4.5))), [1])
            self.assertEqual(await ids(span.strictly_right_of(p.Range(4.0, 4.5))), [2])
            self.assertEqual(await ids(span.not_extend_right_of(p.Range(1.0, 3.0))), [1])
            self.assertEqual(await ids(span.not_extend_left_of(p.Range(4.0, 7.0))), [2])
            self.assertEqual(await ids(span.adjacent_to(p.Range(2.5, 3.0))), [1])
            self.assertIn("= ANY(CAST($1::text[] AS python_entities.floatrange[]))",
                          (span == p.any_([p.Range(1.5, 2.5)])).inspect().sql)
            self.assertEqual(await ids(span == p.any_([p.Range(1.5, 2.5), p.Range(7.0, 8.0)])), [1])
            self.assertEqual(await ids(span.overlaps(p.any_([p.Range(2.0, 3.0), p.Range(5.5, 5.6)]))), [1, 2])
            self.assertEqual(await ids(span.overlaps(p.all_([p.Range(2.0, 3.0), p.Range(1.0, 1.6)]))), [1])
        with self.assertRaises(p.ConstructionError):
            span.contains(1.7)

    # [spec:pgorm:req:bindings.errors+5/test]    a column the entity cannot read as its field's type is a
    # DecodeError naming what the driver was reading and the decoder's reason, as JavaScript reports it
    async def test_decode_refusal_names_column_and_reason(self):
        await self.pool.execute(p.RawSQL(
            "ALTER TABLE python_entities.notes ALTER COLUMN account_id TYPE text USING account_id::text"))
        await self.pool.execute(p.RawSQL("INSERT INTO python_entities.notes VALUES (1, 'one', 'b')"))
        with self.assertRaises(p.DecodeError) as refused:
            await self.note.find().all(self.pool)
        self.assertEqual(
            str(refused.exception),
            "error deserializing column 1: "
            "cannot convert between the Rust type `i32` and the Postgres type `text`",
        )

    # [spec:pgorm:req:bindings.entity-reads+3/test]
    async def test_one_never_widens_a_zero_limit(self):
        async with self.pool.connection() as connection:
            await self.active(1).insert(connection)
            none = self.account.find().limit(0)
            self.assertEqual(await none.all(connection), [])
            self.assertIsNone(await none.optional(connection))
            with self.assertRaises(p.DecodeError):
                await none.one(connection)
            for terminal in ("one", "optional"):
                self.assertTrue(none.inspect(terminal=terminal).sql.endswith("LIMIT $1"))
                self.assertEqual(none.inspect(terminal=terminal).params[-1].value, 0)
            self.assertEqual((await self.account.find().limit(5).one(connection))["id"], 1)
            self.assertEqual(self.account.find().limit(5).inspect(terminal="one").params[-1].value, 1)

    # [spec:pgorm:req:bindings.entity-writes/test]
    async def test_composite_key_entity_writes_by_whole_key(self):
        membership = p.entity("app.Membership")
        described = membership.describe()
        self.assertEqual(described["primary_key"], ["tenant_id", "id"])
        keys = {column["name"]: column["primary_key"] for column in described["columns"]}
        flags = [keys[name] for name in ("tenant_id", "id", "role")]
        self.assertEqual(flags, [True, True, False])
        table = s.from_entity(membership).table
        self.assertIn('"id" integer NOT NULL GENERATED BY DEFAULT AS IDENTITY', table.inspect().sql)
        self.assertIn('PRIMARY KEY ("tenant_id", "id")', table.inspect().sql)
        async with self.pool.connection() as connection:
            await connection.execute(table)
            key = await connection.query(p.RawSQL(
                "SELECT a.attname FROM pg_constraint c, unnest(c.conkey) WITH ORDINALITY AS k(attnum, at) "
                "JOIN pg_attribute a ON a.attnum = k.attnum "
                "WHERE c.conrelid = 'python_entities.memberships'::regclass AND c.contype = 'p' "
                "AND a.attrelid = c.conrelid ORDER BY k.at"
            ))
            self.assertEqual([row["attname"] for row in key], ["tenant_id", "id"])
            # The database generates `id`; a second tenant then holds the same `id`,
            # so a write narrowed by one key column would reach both rows.
            first = await membership.active().set("tenant_id", 1).set("role", "owner").insert(connection)
            shared = first["id"]
            twin = await membership.active().set("tenant_id", 2).set("id", shared).set("role", "guest").insert(connection)
            self.assertEqual((twin["tenant_id"], twin["id"]), (2, shared))
            updated = await first.into_active().set("role", "admin").update(connection)
            self.assertEqual(dict(updated), {"tenant_id": 1, "id": shared, "role": "admin"})
            ordered = membership.find().order_by(membership.col("tenant_id").expr().asc())
            self.assertEqual([dict(row) for row in await ordered.all(connection)], [
                {"tenant_id": 1, "id": shared, "role": "admin"},
                {"tenant_id": 2, "id": shared, "role": "guest"},
            ])
            lookup = membership.find().where((membership.col("tenant_id") == 2) & (membership.col("id") == shared))
            self.assertEqual((await lookup.one(connection))["role"], "guest")
            with self.assertRaisesRegex(p.ConstructionError, "primary key value is not set"):
                await membership.active().set("tenant_id", 2).set("role", "nobody").update(connection)
            self.assertEqual(await twin.into_active().delete(connection), 1)
            self.assertEqual([dict(row) for row in await ordered.all(connection)], [
                {"tenant_id": 1, "id": shared, "role": "admin"},
            ])

    # [spec:pgorm:req:bindings.entity-writes/test]
    async def test_invalid_types_and_foreign_handles_are_rejected(self):
        for column, value in (("id", True), ("id", 2**31), ("id", "1"), ("mood", "unknown"), ("display name", None)):
            with self.subTest(column=column, value=value), self.assertRaises(p.ConstructionError):
                self.active(1).set(column, value)
        with self.assertRaises(p.ConstructionError):
            self.active(1).set("mood", p.Value("calm", p.TypeName("Mood", schema="foreign")))
        for method in (self.active(1).get, self.active(1).not_set, self.active(1).reset):
            with self.assertRaises(p.LifecycleError):
                method(self.note.col("id"))
        with self.assertRaises(p.LifecycleError):
            self.active(1).set(self.note.col("id"), 1)
        with self.assertRaises(p.ConstructionError):
            bool(self.account.find())
        with self.assertRaises(p.ConstructionError):
            bool(self.account.col("id"))
        with self.assertRaises(TypeError):
            self.account.find().limit(True)

    # [spec:pgorm:req:bindings.entity-writes/test]
    async def test_model_setter_preserves_rust_conversion_checks(self):
        async with self.pool.connection() as connection:
            model = await self.active(1).insert(connection)
        changed = model.with_value(self.account.col("display name"), "Local")
        self.assertEqual(changed["display name"], "Local")
        self.assertEqual(model["display name"], "Nora|before")
        with self.assertRaises(p.ConstructionError):
            model.with_value("id", p.Value(1, "i64"))
        with self.assertRaises(p.LifecycleError):
            model.with_value(self.note.col("id"), 2)
        self.assertEqual(changed.into_active().get("display name").state, p.ActiveState.Unchanged)

    # [spec:pgorm:req:bindings.entity-writes/test]    a hook's refusal is a ConstructionError,
    # never a DatabaseError, and the write it refused after the fact stands
    async def test_before_and_after_hooks_keep_write_outcomes(self):
        async with self.pool.connection() as connection:
            with self.assertRaisesRegex(p.ConstructionError, "before_save refused"):
                await self.active(1, "reject_before").insert(connection)
            self.assertIsNone(await self.account.find().optional(connection))
            with self.assertRaisesRegex(p.ConstructionError, "after_save refused") as refused:
                await self.active(2, "reject_after").insert(connection)
            self.assertNotIsInstance(refused.exception, p.DatabaseError)
            self.assertEqual((await self.account.find().one(connection))["id"], 2)
            protected = await self.active(99).insert(connection)
            with self.assertRaisesRegex(p.ConstructionError, "before_delete refused"):
                await protected.into_active().delete(connection)
            self.assertIsNotNone(await self.account.find().where(self.account.col("id") == 99).optional(connection))
            after = await self.active(98).insert(connection)
            with self.assertRaisesRegex(p.ConstructionError, "after_delete refused"):
                await after.into_active().delete(connection)
            self.assertIsNone(await self.account.find().where(self.account.col("id") == 98).optional(connection))

    # [spec:pgorm:req:bindings.entity-reads+3/test]
    # [spec:pgorm:req:bindings.errors+5/test]
    async def test_decode_failure_does_not_become_absence(self):
        async with self.pool.connection() as connection:
            await self.active(1).insert(connection)
            await connection.execute(p.RawSQL('ALTER TABLE python_entities.accounts ALTER COLUMN version TYPE bigint'))
            with self.assertRaises(p.DecodeError):
                await self.account.find().optional(connection)

    # [spec:pgorm:req:bindings.errors+5/test]
    async def test_parameter_failure_preserves_connection(self):
        async with self.pool.connection() as connection:
            with self.assertRaises(p.ConstructionError):
                await connection.one(p.RawSQL("SELECT $1::integer AS n", [p.Value("text", "text")]))
            self.assertTrue(await connection.ping())

    # [spec:pgorm:req:bindings.entities+5/test]
    # [spec:pgorm:req:bindings.cancellation/test]
    async def test_cancellation_interrupts_an_active_rust_hook(self):
        async with self.pool.connection() as connection:
            future = self.active(1, "wait_before").insert(connection)
            await asyncio.sleep(0.05)
            future.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await asyncio.wait_for(future, 1)
            await asyncio.wait_for(connection.close(), 1)
        self.assertEqual(await self.pool.query(p.RawSQL('SELECT id FROM python_entities.accounts')), [])


if __name__ == "__main__":
    unittest.main()
