"""Public graph behavior in the downstream application wheel."""

import asyncio
import json
from pathlib import Path
import os
import unittest
import pgorm as p
from pgorm import schema as s
import registered_entities as fixture


def identities(rows):
    return [(root["id"], None if note is None else note["id"]) for root, note in rows]


class GraphTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        await fixture.RegisteredEntities.asyncSetUp(self)
        async with self.pool.connection() as connection:
            for identity in (1, 2, 3):
                active = fixture.RegisteredEntities.active(self, identity)
                if identity == 3:
                    active = active.set("mood", "busy")
                await active.insert(connection)
            await connection.execute(p.RawSQL("INSERT INTO python_entities.notes VALUES (11,1,'first'),(12,1,'second'),(21,2,'third')"))
        self.graph = p.graph("app.AccountNotes")

    async def asyncTearDown(self):
        await fixture.RegisteredEntities.asyncTearDown(self)

    # [spec:pgorm:req:bindings.entity-graphs+4/test]
    async def test_required_optional_and_bare_models(self):
        async with self.pool.connection() as connection:
            query = self.graph.find()
            rows = await query.order_by(query.col(0, "id").asc(), query.col(1, "id").asc()).all(connection)
            self.assertEqual(identities(rows), [(1, 11), (1, 12), (2, 21), (3, None)])
            self.assertIsInstance(rows[0][0], p.EntityModel)
            self.assertEqual(rows[0][1].entity_name, "app.Note")
            self.assertEqual(rows[0][1].into_active().get("body").state, p.ActiveState.Unchanged)
            required = p.graph("app.RequiredNotes").find()
            self.assertEqual(len(await required.all(connection)), 3)
            mixed = p.graph("app.MixedNotes").find()
            self.assertEqual(len(await mixed.all(connection)), 5)
            self.assertTrue(all(len(row) == 3 for row in await mixed.all(connection)))
            root = p.graph("app.AccountOnly").find()
            self.assertIsInstance(await root.optional(connection), p.EntityModel)
            for size, name in ((4, "FourSources"), (5, "FiveSources"), (6, "SixSources"), (7, "SevenSources")):
                query = p.graph("app." + name).find()
                row = await query.where(query.col(0, "id") == 3).optional(connection)
                self.assertEqual(len(row), size)
                self.assertEqual(row[1:], (None,) * (size - 1))

    # [spec:pgorm:req:bindings.entity-graphs+4/test]
    async def test_manifest_declares_shapes(self):
        manifest = p.capabilities()
        self.assertEqual(manifest["graph_policy"]["source_arities"], list(range(1, 8)))
        graphs = {graph["name"]: graph for graph in manifest["registrations"]["graphs"]}
        self.assertEqual(len(graphs), 12)
        self.assertEqual(p.graphs(), sorted(graphs))
        self.assertEqual([source["slot"] for source in graphs["app.MixedNotes"]["sources"]], ["root", "Req", "Opt"])
        self.assertEqual(graphs["app.AccountNotes"]["sources"][1]["entity"], "app.Note")
        self.assertIn("pgorm::query::graph::SelectGraph", graphs["app.AccountNotes"]["rust_shape"])
        self.assertEqual(self.graph.describe(), graphs["app.AccountNotes"])

    # [spec:pgorm:req:bindings.entity-graphs+4/test]
    async def test_aliases_filters_and_reuse_match_projection(self):
        alias = 'notes "β"'
        query = self.graph.find(aliases=[alias])
        changed = query.where(query.col(1, "body") == "second")
        self.assertIn('"notes ""β"""', changed.inspect().sql)
        self.assertEqual(changed.inspect().params[0].value, "second")
        self.assertEqual(query.inspect().params, [])
        async with self.pool.connection() as connection:
            row = await changed.optional(connection)
            self.assertEqual((row[0]["id"], row[1]["id"]), (1, 12))
            self.assertEqual(len(await query.all(connection)), 4)
            absent = query.where(query.col(0, "id") == 100)
            self.assertEqual(await absent.all(connection), [])
            self.assertIsNone(await absent.optional(connection))
        self.assertNotEqual(changed.inspect().sql, changed.inspect(terminal="optional").sql)

    # [spec:pgorm:req:bindings.entity-graphs+4/test]
    async def test_cursor_boundaries_keep_join_tiebreaks(self):
        query = self.graph.find(aliases=["n"])
        cursor = query.cursor("id")
        cases = [
            ("first", cursor.first(2), [(1, 11), (1, 12)]),
            ("full_after", cursor.after_with(1, 11).first(2), [(1, 12), (2, 21)]),
            ("primary_after", cursor.after(1), [(2, 21), (3, None)]),
            ("before_last", cursor.before_with(2, 21).last(1), [(1, 12)]),
            ("descending", cursor.desc().first(2), [(3, None), (2, 21)]),
            ("replace_window", cursor.first(1).last(2), [(2, 21), (3, None)]),
            ("empty", cursor.first(0), []),
            ("enum", query.cursor("mood").after_with("calm", 1, 11).first(2), [(1, 12), (2, 21)]),
        ]
        report = {}
        async with self.pool.connection() as connection:
            for name, selected, expected in cases:
                with self.subTest(name=name):
                    report[name] = identities(await selected.all(connection))
                    self.assertEqual(report[name], expected)
        if path := os.environ.get("PGORM_GRAPH_REPORT"):
            Path(path).write_text(json.dumps(report, indent=2) + "\n")

    # [spec:pgorm:req:bindings.entity-graphs+4/test]
    async def test_present_decode_failure_is_not_absent(self):
        query = self.graph.find()
        async with self.pool.connection() as connection:
            await connection.execute(p.RawSQL("ALTER TABLE python_entities.notes ALTER COLUMN body DROP NOT NULL"))
            await connection.execute(p.RawSQL("UPDATE python_entities.notes SET body = NULL WHERE id = 11"))
            broken = query.where(query.col(1, "id") == 11)
            for terminal in (broken.all, broken.optional, broken.cursor("id").all):
                with self.assertRaises(p.DecodeError):
                    await terminal(connection)
            self.assertEqual((await query.where(query.col(0, "id") == 3).optional(connection))[1], None)

    # [spec:pgorm:req:bindings.entity-graphs+4/test]
    async def test_composite_root_cursor_ties_on_whole_key(self):
        membership = p.entity("app.Membership")
        async with self.pool.connection() as connection:
            await connection.execute(s.from_entity(membership).table)
            for tenant, identity, role in ((1, 1, "a"), (1, 2, "a"), (2, 1, "a"), (2, 2, "b")):
                await membership.active().set("tenant_id", tenant).set("id", identity).set("role", role).insert(connection)
            cursor = p.graph("app.MembershipOnly").find().cursor("role")
            cases = [
                ("first", cursor.first(3), [(1, 1), (1, 2), (2, 1)]),
                ("after_whole_key", cursor.after_with("a", 1, 2).first(2), [(2, 1), (2, 2)]),
                ("before_whole_key", cursor.before_with("a", 2, 1).last(2), [(1, 1), (1, 2)]),
                ("primary_after", cursor.after("a"), [(2, 2)]),
            ]
            for name, selected, expected in cases:
                with self.subTest(name=name):
                    rows = await selected.all(connection)
                    self.assertEqual([(row["tenant_id"], row["id"]) for row in rows], expected)
        for values in (("a", 1), ("a", 1, 2, 3)):
            with self.subTest(values=values), self.assertRaises(p.ConstructionError):
                cursor.after_with(*values)
        rows = await cursor.after_with("a").all(self.pool)
        self.assertEqual([(row["tenant_id"], row["id"]) for row in rows], [(2, 2)])

    # [spec:pgorm:req:bindings.entity-graphs+4/test]
    async def test_invalid_shapes_aliases_and_boundaries_fail(self):
        with self.assertRaises(p.UnsupportedCapabilityError):
            p.graph("runtime-created-tuple")
        for capability in ("graph.stream", "graph.cursor.inspect", "graph.cursor_by_on", "graph.all_grouped"):
            p.require_capability(capability)
        for aliases in ([], ["one", "two"], ["accounts"], [""], ["nul\0"]):
            with self.subTest(aliases=aliases), self.assertRaises(p.ConstructionError):
                self.graph.find(aliases=aliases)
        for aliases in ("one", 1):
            with self.subTest(aliases=aliases), self.assertRaises(TypeError):
                self.graph.find(aliases=aliases)
        query = self.graph.find()
        for source in (-1, 2):
            with self.subTest(source=source), self.assertRaises(p.ConstructionError):
                query.col(source, "id")
        for source in (True, 1.5):
            with self.subTest(source=source), self.assertRaises(TypeError):
                query.col(source, "id")
        with self.assertRaises(p.ConstructionError):
            query.col(0, "missing")
        cursor = query.cursor("id")
        for values in ((), (1, 2, 3)):
            with self.assertRaises(p.ConstructionError):
                cursor.after_with(*values)
        for value in (True, 2**31, "one"):
            with self.assertRaises(p.ConstructionError):
                cursor.after(value)
        with self.assertRaises(p.ConstructionError):
            cursor.first(-1)
        for value in (True, None, 1.5):
            with self.assertRaises(TypeError):
                cursor.first(value)
        for value in (query, cursor):
            with self.assertRaises(p.ConstructionError):
                bool(value)

    # [spec:pgorm:req:bindings.entity-graphs+4/test]
    async def test_cancellation_releases_a_blocked_graph(self):
        async with p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1) as blocker:
            async with blocker.connection() as lock, self.pool.connection() as connection:
                await lock.execute(p.RawSQL("BEGIN"))
                try:
                    await lock.execute(p.RawSQL("LOCK python_entities.accounts IN ACCESS EXCLUSIVE MODE"))
                    future = self.graph.find().all(connection)
                    await asyncio.sleep(0.05)
                    future.cancel()
                    with self.assertRaises(asyncio.CancelledError):
                        await asyncio.wait_for(future, 1)
                    await asyncio.wait_for(connection.close(), 1)
                finally:
                    await lock.execute(p.RawSQL("ROLLBACK"))
        self.assertTrue(await self.pool.ping())


if __name__ == "__main__":
    unittest.main()
