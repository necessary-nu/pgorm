"""Declared models generated from DDL: pgorm-codegen's Python writer behind
``pgorm.codegen.models_from_sql``, the committed module it writes for
pgorm-codegen's fixture schema, and rows written and read through that module
on a database the schema creates."""

from datetime import date, datetime, time, timezone
from decimal import Decimal
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import ModuleType
import unittest
from urllib.parse import urlsplit, urlunsplit
from uuid import UUID, uuid4

import pgorm as p
from pgorm.codegen import models_from_sql

ROOT = Path(__file__).resolve().parents[2]
SCHEMA = (ROOT / "pgorm-codegen/tests/models/schema.sql").read_text()
COMMITTED = Path(__file__).resolve().parent / "models_from_sql" / "models.py"


def committed() -> ModuleType:
    """The committed module, imported as a program imports what it wrote."""
    spec = importlib.util.spec_from_file_location("generated_models", COMMITTED)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules["generated_models"] = module
    spec.loader.exec_module(module)
    return module


class ModelsFromSql(unittest.TestCase):
    # [spec:pgorm:req:bindings.models-from-sql/test]    models_from_sql writes pgorm-codegen's Python module for the DDL, the committed module byte for byte, which imports as declared models
    def test_the_module_is_pgorm_codegens(self):
        self.assertEqual(models_from_sql(SCHEMA), COMMITTED.read_text())
        models = committed()
        self.assertIsInstance(models.Cake, p.Model)
        self.assertEqual(models.Cake.primary_key, ("id",))
        self.assertEqual(models.Sample.describe()["fields"]["flavors"]["kind"], 'enum "public"."flavor"[]')
        self.assertEqual(models.PaymentDisjoint.describe()["kind_type"], {"schema": "public", "name": "payment_kind"})
        self.assertEqual(
            models.ReviewSubject.describe()["targets"],
            [
                {"value": "cake", "field": "cake_id", "target": "cake"},
                {"value": "bakery", "field": "bakery_id", "target": "bakery"},
            ],
        )

    # [spec:pgorm:req:bindings.models-from-sql/test]    schema_name and unkinded_disjoint reach pgorm-codegen, each table a name or a (schema, name) pair; a refusal is a ConstructionError with its message and a wrong argument a TypeError
    def test_options_and_refusals(self):
        ddl = (
            "CREATE TABLE shop (id int PRIMARY KEY); "
            "CREATE TABLE app.web_shop (shop_id int PRIMARY KEY REFERENCES shop (id));"
        )
        source = models_from_sql(
            ddl,
            schema_name="app",
            unkinded_disjoint=[{"base": "shop", "kinds": [("app", "web_shop")], "unit": "dormant"}],
        )
        self.assertIn('schema="app"', source)
        self.assertIn("unkinded=True", source)
        with self.assertRaises(p.ConstructionError) as refused:
            models_from_sql("CREATE TRIGGER t AFTER INSERT ON a EXECUTE FUNCTION f();")
        self.assertEqual(str(refused.exception), "unsupported DDL: CREATE TRIGGER at statement 1")
        with self.assertRaises(p.ConstructionError) as refused:
            models_from_sql("CREATE TABLE a (id int PRIMARY KEY, note text DEFAULT NULL);")
        self.assertIn("defaults to NULL", str(refused.exception))
        for bad in (
            lambda: models_from_sql(b"CREATE TABLE a (id int)"),
            lambda: models_from_sql("", schema_name=1),
            lambda: models_from_sql("", unkinded_disjoint=[{"base": 1, "kinds": []}]),
        ):
            with self.assertRaises(TypeError):
                bad()

    # [spec:pgorm:req:bindings.models-from-sql/test]    python -m pgorm.codegen models writes the module to the file --output names
    def test_the_command_line_writes_the_module(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "models.py"
            subprocess.run(
                [sys.executable, "-m", "pgorm.codegen", "models",
                 ROOT / "pgorm-codegen/tests/models/schema.sql", "--output", output],
                check=True, capture_output=True,
            )
            self.assertEqual(output.read_text(), COMMITTED.read_text())

    # [spec:pgorm:req:bindings.current-keywords/test]    current_date(), current_time() and current_timestamp() are the SQL keywords
    def test_current_keywords(self):
        self.assertEqual(p.current_date().inspect().sql, "SELECT CURRENT_DATE")
        self.assertEqual(p.current_time().inspect().sql, "SELECT CURRENT_TIME")
        self.assertEqual(p.current_timestamp().inspect().sql, "SELECT CURRENT_TIMESTAMP")


class GeneratedModelsLive(unittest.IsolatedAsyncioTestCase):
    """Rows written and read through the committed module, on a database of
    its own made from the fixture schema."""

    async def asyncSetUp(self):
        dsn = os.environ["PGORM_TEST_DSN"]
        self.database = f"models_from_sql_{uuid4().hex[:12]}"
        self.admin = p.Pool(dsn, max_size=1)
        await self.admin.execute(p.RawSQL(f'CREATE DATABASE "{self.database}"'))
        parts = urlsplit(dsn)
        self.pool = p.Pool(urlunsplit(parts._replace(path=f"/{self.database}")), max_size=2)
        await self.pool.batch_execute(SCHEMA)
        self.models = committed()

    async def asyncTearDown(self):
        await self.pool.close()
        await self.admin.execute(p.RawSQL(f'DROP DATABASE "{self.database}" WITH (FORCE)'))
        await self.admin.close()

    # [spec:pgorm:req:bindings.models-from-sql/test]    the generated models write and read rows of every column type they declare, an enum, a domain over an enum and money among them, and follow the relations and the junction path
    async def test_rows_round_trip(self):
        m = self.models
        bakery = await m.Bakery.insert({"name": "Sunrise", "city": "Oslo"}).returning().one(self.pool)
        cake = await m.Cake.insert({
            "name": "Lemon Drizzle", "flavor": "lemon", "special": "lemon",
            "price": Decimal("3.20"), "tip": Decimal("0.50"), "bakery_id": bakery["id"],
        }).returning().one(self.pool)
        self.assertEqual(
            (cake["special"], cake["price"], cake["tip"], cake["gluten_free"]),
            ("lemon", Decimal("3.20"), Decimal("0.50"), False),
        )
        lemon = await m.Cake.find().where(m.Cake.col("special") == "lemon").all(self.pool)
        self.assertEqual([row["id"] for row in lemon], [cake["id"]])
        baker = await m.Baker.insert({"name": "Ada", "email": "ada@example.com"}).returning().one(self.pool)
        await m.CakeBaker.insert({"cake_id": cake["id"], "baker_id": baker["id"]}).execute(self.pool)
        joined = await m.Cake.find().join(m.cake_bakery).where(m.Bakery.col("city") == "Oslo").all(self.pool)
        self.assertEqual([row["name"] for row in joined], ["Lemon Drizzle"])
        bakers = await m.cake_related_baker.find(cake).all(self.pool)
        self.assertEqual([row["name"] for row in bakers], ["Ada"])

        values = {
            "small": 7, "whole": 70, "big": 7_000_000_000, "single": 1.5, "double": 2.25,
            "amount": Decimal("10.5"), "price": Decimal("1.250"), "tip": Decimal("2.00"),
            "ok": True, "body": "text", "label": "a label", "code": "C-1", "initials": "ADA",
            "raw": b"\x00\x01", "doc": {"a": [1, 2]}, "docb": {"b": None},
            "ref": UUID("6f1c4b7e-7a3e-4c55-9f43-3f1f0b6a2c10"),
            "day": date(2026, 11, 6), "at_time": time(9, 30), "at": datetime(2026, 11, 6, 9, 30),
            "at_utc": datetime(2026, 11, 6, 9, 30, tzinfo=timezone.utc),
            "span": p.Interval(days=2), "short_span": p.Interval(microseconds=5_400_000_000),
            "ints": p.Range(1, 5), "bigs": p.Range(1, 9_000_000_000), "nums": p.Range(Decimal("1.5"), Decimal("2.5")),
            "days": p.Range(date(2026, 1, 1), date(2026, 2, 1)),
            "stamps": p.Range(datetime(2026, 1, 1), datetime(2026, 1, 2)),
            "instants": p.Range(datetime(2026, 1, 1, tzinfo=timezone.utc), datetime(2026, 1, 2, tzinfo=timezone.utc)),
            "int_sets": p.Multirange([p.Range(1, 3), p.Range(5, 7)]),
            "day_sets": p.Multirange([p.Range(date(2026, 1, 1), date(2026, 1, 3))]),
            "heat": p.Range(180.0, 250.5), "flavor": "vanilla", "special": "carrot",
            "flavors": ["lemon", None, "chocolate"], "tags": ["x", "y"], "counts": [1, None, 3],
            "weight": 250,
        }
        sample = await m.Sample.insert(values).returning().one(self.pool)
        read = await m.Sample.find_by_key({"id": sample["id"]}).one(self.pool)
        for field, value in values.items():
            with self.subTest(field=field):
                self.assertEqual(read[field], value)
        self.assertEqual((read["quantity"], read["note"], read["doubled"]), (1, "none", 2))
        self.assertIsInstance(read["created"], datetime)

        base, kind, card = await m.PaymentDisjoint.insert(
            {"order_id": 1, "amount": Decimal("12.20")}, "card", {"last_four": "4242"}
        ).returning_model(self.pool)
        self.assertEqual((kind, base["kind"]), ("card", "card"))
        assert card is not None
        self.assertEqual(dict(card), {"payment_id": base["id"], "last_four": "4242"})
        await m.PaymentDisjoint.insert({"order_id": 2, "amount": Decimal("3.80")}, "cash").returning_model(self.pool)
        payments = await m.PaymentDisjoint.find().order_by(m.Payment.col("id").asc()).all(self.pool)
        self.assertEqual([kind for _, kind, _ in payments], ["card", "cash"])

        review = await m.Review.insert(
            {"stars": 5, "body": "Lovely", **m.ReviewSubject.set("cake", cake["id"])}
        ).returning().one(self.pool)
        self.assertEqual(m.ReviewSubject.key(review), ("cake", cake["id"]))
        [(value, target)] = await m.ReviewSubject.load(self.pool, [review])
        self.assertEqual((value, target["name"]), ("cake", "Lemon Drizzle"))
        with self.assertRaises(p.CheckViolationError):
            await m.Review.insert(
                {"stars": 1, "body": "Both", "cake_id": cake["id"], "bakery_id": bakery["id"]}
            ).execute(self.pool)


if __name__ == "__main__":
    unittest.main()
