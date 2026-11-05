"""Application builders in an installed wheel; emits evidence for Rust parity."""

import csv
import importlib.metadata
import io
import json
import os
from pathlib import Path
import sys
import unittest
from uuid import uuid4

import pgorm as p


def quoted(name):
    """Fixture DDL only; protected application queries use public builders."""
    return '"' + name.replace('"', '""') + '"'


async def collect(stream):
    async with stream:
        return b"".join([chunk async for chunk in stream])


class DirectBuilderTests(unittest.IsolatedAsyncioTestCase):
    # [spec:pgorm:req:python.direct-builders/test]
    async def test_installed_builders_execute_without_http(self):
        self.assertTrue(Path(p.__file__).resolve().is_relative_to(Path(sys.prefix).resolve()))
        self.assertEqual(p.capabilities()["transport"], "in-process")
        native_files = [str(f) for f in importlib.metadata.files("pgorm")
                        if str(f).endswith((".so", ".pyd"))]
        self.assertEqual(len(native_files), 1)
        schema = os.environ.get("PGORM_DIRECT_SCHEMA", 'Python 雪 "' + uuid4().hex[:8])
        accounts = os.environ.get("PGORM_DIRECT_ACCOUNTS", 'accounts %"' + uuid4().hex[:8])
        events = os.environ.get("PGORM_DIRECT_EVENTS", 'events_\\' + uuid4().hex[:8])
        report = {
            "schema_version": 1, "transport": "in-process", "package_version": p.__version__,
            "native_file": native_files[0], "schema": schema, "accounts": accounts,
            "events": events, "runs": [],
        }
        async with p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1) as pool:
            await pool.execute(p.RawSQL(f"CREATE SCHEMA {quoted(schema)}"))
            try:
                aq = f"{quoted(schema)}.{quoted(accounts)}"
                eq = f"{quoted(schema)}.{quoted(events)}"
                await pool.execute(p.RawSQL(f"CREATE TABLE {aq} (id integer PRIMARY KEY, name text NOT NULL, active boolean NOT NULL, visits bigint NOT NULL)"))
                await pool.execute(p.RawSQL(f"CREATE TABLE {eq} (id integer PRIMARY KEY, account_id integer REFERENCES {aq}(id), kind text NOT NULL, points integer NOT NULL)"))
                for mode in ("bound", "literal"):
                    for variant in (0, 1):
                        await pool.execute(p.RawSQL(f"TRUNCATE {eq}, {aq}"))
                        report["runs"].append(await self.exercise(pool, schema, accounts, events, mode, variant))
            finally:
                await pool.execute(p.RawSQL(f"DROP SCHEMA {quoted(schema)} CASCADE"))
        self.assertEqual(p.__version__, report["package_version"])
        if path := os.environ.get("PGORM_DIRECT_REPORT"):
            Path(path).write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")

    async def exercise(self, pool, schema, accounts, events, mode, variant):
        operand = p.literal if mode == "literal" else p.bind
        term = "click ' %\\雪" if variant == 0 else "x'); DROP TABLE accounts; --"
        threshold = 0 if variant == 0 else 15
        run = {"mode": mode, "variant": variant, "term": term, "threshold": threshold, "cases": {}}
        a = p.Table(accounts, schema=schema)
        e = p.Table(events, schema=schema)

        def record(name, query):
            compiled = query.inspect()
            run["cases"][name] = {"sql": compiled.sql, "params": [v.snapshot() for v in compiled.params]}
            if mode == "literal":
                # Rust's typed limit is a bound u64 even when expression
                # values explicitly take the literal path.
                expected = [p.Value(2, "u64")] if name == "select_join" and variant else []
                self.assertEqual(compiled.params, expected)
            return query

        insert = p.insert(a).columns("id", "name", "active", "visits")
        for values in ((1, "Nora O'Brien", True, 10), (2, "Beta%_\\雪", True, 20), (3, "No event", True, 30)):
            insert = insert.values(*(operand(v) for v in values))
        self.assertEqual(await pool.execute(record("insert_accounts", insert)), 3)
        insert = p.insert(e).columns("id", "account_id", "kind", "points")
        for values in ((10, 1, term, 2), (11, 1, "other", 3), (12, 2, term, 4)):
            insert = insert.values(*(operand(v) for v in values))
        self.assertEqual(await pool.execute(record("insert_events", insert)), 3)

        a, e = a.as_("a"), e.as_("e")
        query = p.select(a.col("id").as_("account_id"), a.col("name"), e.col("id").as_("event_id"), e.col("kind"))
        query = query.from_(a).join(e, a.col("id") == e.col("account_id"), kind=p.Join.Left)
        predicate = p.Condition.all(
            a.col("active") == operand(True),
            p.Condition.any(e.col("kind") == operand(term), e.col("id").is_null()),
            a.col("visits") >= operand(threshold),
        )
        if variant:
            predicate = predicate.add(a.col("id") >= operand(2))
        query = query.where(predicate).order_by(a.col("id").asc(), e.col("id").asc(nulls=p.Nulls.Last))
        if variant:
            query = query.limit(2)
        rows = await pool.query(record("select_join", query))
        expected = [
            {"account_id": 1, "name": "Nora O'Brien", "event_id": 10, "kind": term},
            {"account_id": 2, "name": "Beta%_\\雪", "event_id": 12, "kind": term},
            {"account_id": 3, "name": "No event", "event_id": None, "kind": None},
        ][variant:]
        self.assertEqual([dict(row) for row in rows], expected)
        self.assertEqual(rows[-1].tagged("event_id").kind, "i32")
        self.assertTrue(rows[-1].tagged("event_id").is_null)

        a, e = p.Table(accounts, schema=schema), p.Table(events, schema=schema)
        update = p.update(a).set("visits", p.col("visits") + operand(5 + variant))
        update = update.where(p.col("id") == operand(1)).returning(p.col("id"), p.col("name"), p.col("visits"))
        updated = await pool.one(record("update_returning", update))
        self.assertEqual(dict(updated), {"id": 1, "name": "Nora O'Brien", "visits": 15 + variant})
        delete = record("delete_event", p.delete(e).where(p.col("id") == operand(11)))
        self.assertEqual(await pool.execute(delete), 1)
        self.assertEqual(await pool.execute(delete), 0)
        missing = p.select(p.col("id")).from_(a).where(p.col("id") == operand(999))
        self.assertIsNone(await pool.optional(record("select_missing", missing)))
        target, source = e.as_("e"), a.as_("a")
        merge = (p.merge(target, source, target.col("id") == source.col("id") + operand(10))
                 .when_matched(p.MergeAction.update("points", target.col("points") + operand(1)),
                               condition=target.col("kind") == operand(term))
                 .when_not_matched(p.MergeAction.insert("id", source.col("id") + operand(10))
                                   .set("account_id", source.col("id"))
                                   .set("kind", operand("merged")).set("points", operand(0)))
                 .when_not_matched_by_source(p.MergeAction.delete())
                 .returning_action()
                 .returning(target.col("id"), p.ReturningRow.Old.col("points").as_("before"),
                            p.ReturningRow.New.col("points").as_("after")))
        merged = sorted(tuple(row.values()) for row in await pool.query(record("merge_events", merge)))
        self.assertEqual(merged, [("DELETE", 10, 2, None), ("INSERT", 11, None, 0),
                                  ("INSERT", 13, None, 0), ("UPDATE", 12, 4, 5)])
        final = p.select(p.col("id"), p.col("visits")).from_(a).order_by(p.col("id").asc())
        self.assertEqual([tuple(row.values()) for row in await pool.query(record("select_final", final))],
                         [(1, 15 + variant), (2, 20), (3, 30)])
        copied = await self.copy_cases(pool, record, operand, term, e)
        run["verified"] = {"inserted_accounts": 3, "inserted_events": 3, "selected": len(expected), "updated_visits": 15 + variant, "deleted": [1, 0], "missing": True, "merged": len(merged), "copied": copied}
        return run

    # [spec:pgorm:req:bindings.copy/test]
    async def copy_cases(self, pool, record, operand, term, e):
        """COPY each way in each format, over a table, a SELECT and a DELETE's RETURNING."""
        columns = ("id", "account_id", "kind", "points")
        rows = io.StringIO()
        csv.writer(rows, delimiter=";", lineterminator="\n").writerows(
            [columns, (20, 1, term, 7), (21, 2, "copied", 8)])
        csv_in = p.CopyFrom(e, "csv").columns(*columns).header().delimiter(";")
        copied = {"csv": await pool.copy_in(record("copy_from_csv", csv_in), rows.getvalue().encode())}
        text_in = p.CopyFrom(e, "text").columns(*columns).null_string("NULL")
        copied["text"] = await pool.copy_in(record("copy_from_text", text_in), b"22\tNULL\tplain\t9\n")
        query = (p.select(p.col("id"), p.col("kind")).from_(e).where(p.col("points") >= operand(7))
                 .order_by(p.col("id").asc()))
        csv_out = p.CopyTo(query, "csv").header().force_quote("kind")
        out = await collect(await pool.copy_out(record("copy_to_csv", csv_out)))
        quoted_term = '"' + term.replace('"', '""') + '"'
        self.assertEqual(out.decode(), f'id,kind\n20,{quoted_term}\n21,"copied"\n22,"plain"\n')
        moved = p.delete(e).where(p.col("id") >= operand(20)).returning(*map(p.col, columns))
        data = await collect(await pool.copy_out(record("copy_to_binary", p.CopyTo(moved, "binary"))))
        binary_in = p.CopyFrom(e, "binary").columns(*columns)
        copied["binary"] = await pool.copy_in(record("copy_from_binary", binary_in), data)
        text_out = p.CopyTo(e, "text").null_string("-")
        lines = (await collect(await pool.copy_out(record("copy_to_text", text_out)))).decode().splitlines()
        escaped = term.replace("\\", "\\\\")
        self.assertEqual(sorted(lines), [
            "11\t1\tmerged\t0", f"12\t2\t{escaped}\t5", "13\t3\tmerged\t0",
            f"20\t1\t{escaped}\t7", "21\t2\tcopied\t8", "22\t-\tplain\t9",
        ])
        return copied


def forbid_processes(event, args):
    if event == "subprocess.Popen" or event == "os.system" or event.startswith(("os.exec", "os.spawn")):
        raise RuntimeError("direct-builder queries may not launch compilers or helper processes")


if __name__ == "__main__":
    # The dedicated runner installs the wheel and compiles the Rust oracle first.
    # During this process all builder construction and DB I/O are native calls.
    sys.addaudithook(forbid_processes)
    unittest.main()
