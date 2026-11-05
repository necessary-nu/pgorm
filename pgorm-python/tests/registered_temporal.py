"""A temporal key and a relation's PERIOD, deferral and NOT ENFORCED, in the application wheel."""

from datetime import date
import os
import unittest

import pgorm as p
from pgorm import schema as s


def days(start, end):
    """``[start, end)`` in 2026, each a ``(month, day)`` pair."""
    return p.Range(date(2026, *start), date(2026, *end))


# [spec:pgorm:req:bindings.entity-reads+3/test]
# [spec:pgorm:req:bindings.entity-graphs+3/test]
class RegisteredTemporal(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1)
        self.room, self.stay = p.entity("app.Room"), p.entity("app.Stay")
        self.account = p.entity("app.Account")
        await self.pool.execute(p.RawSQL("CREATE SCHEMA python_entities"))
        # The room key's scalar part needs btree_gist's operator class; in the
        # dropped schema, the extension goes with it.
        await self.pool.execute(p.RawSQL("CREATE EXTENSION IF NOT EXISTS btree_gist SCHEMA python_entities"))
        for entity in (self.account, self.room, self.stay):
            generated = s.from_entity(entity)
            for enum in generated.enums:
                await self.pool.execute(enum)
            await self.pool.execute(generated.table)
        async with self.pool.connection() as connection:
            for identity, start, end, rate in ((1, (1, 1), (2, 1), 100), (1, (2, 1), (3, 1), 120), (2, (1, 1), (2, 1), 80)):
                await self.version(identity, days(start, end), rate).insert(connection)

    async def asyncTearDown(self):
        await self.pool.execute(p.RawSQL("DROP SCHEMA python_entities CASCADE"))
        await self.pool.close()

    def version(self, identity, valid_at, rate):
        return self.room.active().set("id", identity).set("valid_at", valid_at).set("rate", rate)

    def booked(self, identity, room, guest, during):
        return self.stay.active().set("id", identity).set("room_id", room).set("guest_id", guest).set("during", during)

    async def rates(self, connection):
        query = self.room.find().order_by(self.room.col("id").expr().asc(), self.room.col("valid_at").expr().asc())
        return [(row["id"], row["valid_at"], row["rate"]) for row in await query.all(connection)]

    async def test_describe_reports_the_temporal_key_and_relations(self):
        room = self.room.describe()
        self.assertEqual(room["primary_key"], ["id", "valid_at"])
        self.assertTrue(room["primary_key_without_overlaps"])
        self.assertEqual(room["relations"], [])
        self.assertFalse(self.account.describe()["primary_key_without_overlaps"])
        kinds = {column["name"]: column["kind"] for column in room["columns"]}
        self.assertEqual(kinds["valid_at"], "daterange")
        relations = {relation["name"]: relation for relation in self.stay.describe()["relations"]}
        self.assertEqual(relations["Room"], {
            "name": "Room", "type": "has_one",
            "from": {"schema": "python_entities", "table": "stays"},
            "to": {"schema": "python_entities", "table": "rooms"},
            "columns": [["room_id", "id"]], "period": ["during", "valid_at"],
            "condition_type": "all", "on_condition": False,
            "enforcement": None, "deferrability": "deferrable_initially_deferred",
        })
        self.assertEqual(relations["Guest"]["period"], None)
        self.assertEqual(relations["Guest"]["enforcement"], "not_enforced")
        self.assertEqual(relations["Guest"]["deferrability"], None)
        ddl = s.from_entity(self.stay).table.inspect().sql
        self.assertIn('REFERENCES "python_entities"."rooms" ("id", PERIOD "valid_at")', ddl)
        self.assertIn("DEFERRABLE INITIALLY DEFERRED", ddl)
        self.assertIn("NOT ENFORCED", ddl)

    async def test_version_written_by_whole_key(self):
        january, february = days((1, 1), (2, 1)), days((2, 1), (3, 1))
        async with self.pool.connection() as connection:
            key = (self.room.col("id") == 1) & (self.room.col("valid_at") == february)
            found = await self.room.find().where(key).one(connection)
            self.assertEqual(found["rate"], 120)
            # An update by key names one version: equality on the period too,
            # so January's rate for room 1 is the only row it changes.
            jan = await self.room.find().where((self.room.col("id") == 1) & (self.room.col("valid_at") == january)).one(connection)
            changed = await jan.into_active().set("rate", 105).update(connection)
            self.assertEqual((changed["valid_at"], changed["rate"]), (january, 105))
            self.assertEqual(await self.rates(connection), [(1, january, 105), (1, february, 120), (2, january, 80)])
            self.assertEqual(await found.into_active().delete(connection), 1)
            self.assertEqual(await self.rates(connection), [(1, january, 105), (2, january, 80)])
            with self.assertRaises(p.DatabaseError) as caught:
                await self.version(1, days((1, 15), (2, 15)), 90).insert(connection)
            self.assertEqual(caught.exception.sqlstate, "23P01")
            # A period that only touches January's end overlaps nothing.
            await self.version(1, days((2, 1), (2, 15)), 90).insert(connection)

    async def test_a_period_relation_joins_every_overlapping_version(self):
        async with self.pool.connection() as connection:
            await self.booked(1, 1, 7, days((1, 20), (2, 10))).insert(connection)
            await self.booked(2, 2, 7, days((1, 5), (1, 9))).insert(connection)
            query = p.graph("app.StayRooms").find(aliases=["Room version"])
            self.assertIn('"during" && "Room version"."valid_at"', query.inspect().sql)
            rows = await query.order_by(query.col(0, "id").asc(), query.col(1, "valid_at").asc()).all(connection)
            self.assertEqual([(stay["id"], room["rate"]) for stay, room in rows], [(1, 100), (1, 120), (2, 80)])

    async def test_a_deferred_key_is_checked_at_commit(self):
        async with self.pool.transaction() as tx:
            await self.booked(3, 3, 7, days((3, 1), (3, 5))).insert(tx)
            await self.version(3, days((3, 1), (4, 1)), 60).insert(tx)
        with self.assertRaises(p.DatabaseError) as caught:
            async with self.pool.transaction() as tx:
                await self.booked(4, 4, 7, days((3, 1), (3, 5))).insert(tx)
        self.assertEqual(caught.exception.sqlstate, "23503")
        async with self.pool.connection() as connection:
            stays = await self.stay.find().all(connection)
            self.assertEqual([row["id"] for row in stays], [3])

    async def test_unenforced_key_admits_an_orphan(self):
        async with self.pool.connection() as connection:
            await self.account.active().set("id", 7).set("display name", "Guest").set("note", None).insert(connection)
            await self.booked(1, 1, 7, days((1, 2), (1, 3))).insert(connection)
            await self.booked(2, 1, 99, days((1, 4), (1, 5))).insert(connection)
            optional = p.graph("app.StayGuest").find(aliases=["guest"])
            rows = await optional.order_by(optional.col(0, "id").asc()).all(connection)
            self.assertEqual([(stay["id"], None if guest is None else guest["id"]) for stay, guest in rows],
                             [(1, 7), (2, None)])
            required = p.graph("app.StayGuestRequired").find(aliases=["guest"])
            self.assertEqual([stay["id"] for stay, _ in await required.all(connection)], [1])


if __name__ == "__main__":
    unittest.main()
