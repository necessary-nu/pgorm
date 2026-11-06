"""pgorm-migration's runner, run from Python: SQL files read as the program
runs and migrations written in Python, applied through Rust's ``Migrations``."""

import asyncio
import os
import tempfile
import unittest
from pathlib import Path

import pgorm as p
from pgorm import schema as s

SQL = Path(__file__).resolve().parents[2] / "pgorm-migration" / "tests" / "sql"
FROZEN = SQL / "frozen"
NOTE = SQL / "m20260101_000001_create_note.sql"
# The digest pgorm-migration's own tests hold NOTE's text to.
NOTE_DIGEST = "9ae0994f5653f1500a920aee8881629d9aafeead971627b2f54bc0ace1b1bb17"


class Written:
    """A migration written in Python."""

    def __init__(self, name, sql, *, checksum=None, commits_after=False):
        self.name = name
        self.sql = sql
        self.checksum = checksum
        self.commits_after = commits_after
        self.runs = 0

    async def up(self, tx):
        self.runs += 1
        # Several statements run as one script, as a SQL file's migration runs.
        await tx.batch_execute(";\n".join(self.sql))


class SqlMigrations(unittest.TestCase):
    # [spec:pgorm:req:bindings.migrations/test]    a file reads as pgorm-migration reads it: name, digest, directive
    def test_a_file_reads_as_the_runner_does(self):
        note = s.SqlMigration.read(NOTE)
        self.assertEqual(note.name, "m20260101_000001_create_note")
        self.assertEqual(note.checksum, NOTE_DIGEST)
        self.assertFalse(note.commits_after)
        self.assertIn("CREATE TABLE note (", note.sql)

        frozen = s.SqlMigration.read_dir(FROZEN)
        self.assertEqual(
            [migration.name for migration in frozen],
            ["m20240401_000001_create_fleet", "m20240401_000002_add_van_1", "m20240401_000002_add_van_2"],
        )
        self.assertEqual([migration.commits_after for migration in frozen], [False, True, False])

        built = s.SqlMigration("m20260101_000001_create_note", note.sql)
        self.assertEqual(built.checksum, NOTE_DIGEST)

        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / "m1.sql").write_bytes(note.sql.replace("\n", "\r\n").encode())
            (Path(directory) / "notes.txt").write_text("not a migration")
            [crlf] = s.SqlMigration.read_dir(directory)
            self.assertEqual(crlf.checksum, NOTE_DIGEST)
            with self.assertRaises(OSError):
                s.SqlMigration.read(Path(directory) / "missing.sql")
        with self.assertRaises(p.ConstructionError):
            s.SqlMigration.read("/")
        with self.assertRaisesRegex(p.ConstructionError, "PostgreSQL rejected this SQL: syntax error"):
            s.SqlMigration.read(SQL / "broken" / "m20260101_000001_create_note.sql")

    # [spec:pgorm:req:bindings.migrations/test]    a migration is a SqlMigration or an object with a name and an up
    # [spec:pgorm:req:python.truth-testing+5/test]    a migrator, a SQL migration and a status are no booleans
    def test_a_migrator_refuses_what_is_no_migration(self):
        with self.assertRaises(TypeError):
            s.Migrations([object()])
        with self.assertRaises(TypeError):
            s.Migrations([Written(1, [])])
        with self.assertRaises(TypeError):
            s.Migrations([Written("m1", [], checksum=1)])
        migrations = s.Migrations([s.SqlMigration.read(NOTE)])
        for value in (migrations, s.SqlMigration.read(NOTE), s.MigrationStatus.Pending):
            with self.assertRaises(p.ConstructionError):
                bool(value)


class Running(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.pool = p.Pool(os.environ["PGORM_TEST_DSN"], max_size=3)
        self.schema = f"migrations_{os.getpid()}_{id(self)}"

    async def asyncTearDown(self):
        await self.pool.execute(
            s.drop_schema([self.schema, f"{self.schema}_ops"], if_exists=True, behavior="cascade")
        )
        await asyncio.wait_for(self.pool.close(), 5)

    def migrator(self, migrations, **options):
        return s.Migrations(
            migrations, migration_table_schema=f"{self.schema}_ops", migration_table_name="ledger", **options
        )

    async def scalar(self, sql):
        return (await self.pool.one(p.RawSQL(sql)))["value"]

    # [spec:pgorm:req:bindings.migrations/test]    files and Python migrations run in order, once, through the runner
    async def test_a_frozen_history_and_python_migrations_replay(self):
        schema = self.schema
        mood = p.TypeName("mood", schema=schema)
        note = p.Table("note", schema=schema)
        frozen = [
            s.migration_file("m20260101_000001_create_note", [
                s.create_schema(schema),
                s.create_type(mood).as_enum().values(["calm"]),
                s.create_table(note).column(s.ColumnDef("id", "integer").not_null()).column(
                    s.ColumnDef("mood", mood).not_null()
                ),
            ]),
            s.migration_file("m20260102_000001_add_glad", [s.alter_type(mood).add_value("glad")], commits_after=True),
        ]
        seed = Written(
            "m20260103_000001_seed_note",
            [f"INSERT INTO \"{schema}\".note VALUES (1, 'glad')"],
            checksum="1",
        )
        with tempfile.TemporaryDirectory() as directory:
            for file in frozen:
                self.assertTrue(file.freeze(directory))
            files = s.SqlMigration.read_dir(directory)
        self.assertEqual([file.name for file in files], [file.name for file in frozen])
        self.assertEqual([file.sql for file in files], [file.text for file in frozen])
        migrations = self.migrator([*files, seed])
        pending = await migrations.get_pending_migrations(self.pool)
        self.assertEqual([migration.name for migration in pending], [
            "m20260101_000001_create_note", "m20260102_000001_add_glad", "m20260103_000001_seed_note",
        ])
        self.assertTrue(all(migration.status is s.MigrationStatus.Pending for migration in pending))

        await migrations.up(self.pool)
        await migrations.up(self.pool)
        self.assertEqual(seed.runs, 1)
        self.assertEqual(
            await migrations.get_applied_migrations(self.pool),
            [s.Migration(migration.name, s.MigrationStatus.Applied) for migration in pending],
        )
        self.assertEqual(await migrations.get_pending_migrations(self.pool), [])
        self.assertEqual(await self.scalar(f'SELECT count(*) AS value FROM "{schema}_ops".ledger'), 3)
        self.assertEqual(
            await self.scalar(f"SELECT checksum AS value FROM \"{schema}_ops\".ledger WHERE version = '{seed.name}'"),
            "1",
        )
        self.assertEqual(
            await self.scalar(f"SELECT checksum AS value FROM \"{schema}_ops\".ledger WHERE version = '{files[0].name}'"),
            files[0].checksum,
        )
        self.assertEqual(await self.scalar(f'SELECT mood::text AS value FROM "{schema}".note'), "glad")

        edited = Written(seed.name, seed.sql, checksum="2")
        with self.assertRaises(p.ConstructionError) as refused:
            await self.migrator([*files, edited]).up(self.pool)
        self.assertIn("was applied with checksum '1'", str(refused.exception))
        self.assertEqual(edited.runs, 0)

        with self.assertLogs("pgorm.migration", "INFO") as logged:
            await migrations.status(self.pool)
        self.assertIn(f"INFO:pgorm.migration:Migration '{seed.name}'... Applied", logged.output)

    # [spec:pgorm:req:bindings.migrations/test]    a migration's exception rolls the batch back and is raised as itself
    async def test_a_raising_migration_rolls_the_batch_back(self):
        class Refused(Exception):
            pass

        class Raising:
            name = "m20260101_000002_raise"

            async def up(self, tx):
                raise Refused("no")

        first = Written("m20260101_000001_create", [f'CREATE SCHEMA "{self.schema}"'])
        migrations = self.migrator([first, Raising()])
        with self.assertRaises(Refused):
            await migrations.up(self.pool)
        self.assertEqual(await self.scalar(
            f"SELECT count(*) AS value FROM pg_namespace WHERE nspname = '{self.schema}'"
        ), 0)

        with self.assertRaises(p.ConstructionError):
            await self.migrator([first, Written(first.name, [])]).up(self.pool)

    # [spec:pgorm:req:bindings.migrations/test]    up's transaction takes savepoints and statements, commits_after commits
    async def test_the_lent_transaction_is_a_transaction(self):
        schema = self.schema
        held = []

        class Tolerant:
            name = "m20260101_000001_tolerant"
            commits_after = True

            async def up(self, tx):
                held.append(tx)
                with self.assertRaises(p.DatabaseError):
                    async with tx.transaction() as savepoint:
                        await savepoint.execute(p.RawSQL("CREATE EXTENSION pgorm_no_such_extension"))
                await tx.execute(s.create_schema(schema))
                await tx.execute(s.create_type(p.TypeName("mood", schema=schema)).as_enum().values(["calm"]))
                await tx.execute(s.alter_type(p.TypeName("mood", schema=schema)).add_value("glad"))

        class Uses:
            name = "m20260101_000002_uses"

            async def up(self, tx):
                await tx.execute(p.RawSQL(f"CREATE TABLE \"{schema}\".feeling (mood \"{schema}\".mood DEFAULT 'glad')"))

        tolerant = Tolerant()
        tolerant.assertRaises = self.assertRaises
        await self.migrator([tolerant, Uses()]).up(self.pool)
        self.assertEqual(await self.scalar(
            f"SELECT count(*) AS value FROM pg_tables WHERE schemaname = '{schema}' AND tablename = 'feeling'"
        ), 1)
        with self.assertRaises(p.LifecycleError):
            await held[0].execute(p.RawSQL("SELECT 1"))

    # [spec:pgorm:req:bindings.migrations/test]    the runner clears the pool's statement caches after a batch
    async def test_up_clears_the_pools_statement_caches(self):
        schema = self.schema
        async with p.Pool(os.environ["PGORM_TEST_DSN"], max_size=1) as pool:
            await pool.execute(s.create_schema(schema))
            await pool.execute(p.RawSQL(f"CREATE DOMAIN \"{schema}\".ribbon AS text DEFAULT 'red'"))
            await pool.execute(p.RawSQL(f'CREATE TABLE "{schema}".box (id serial, ribbon "{schema}".ribbon)'))
            pack = p.RawSQL(f'INSERT INTO "{schema}".box DEFAULT VALUES RETURNING ribbon')
            self.assertEqual((await pool.one(pack))["ribbon"], "red")
            gold = Written("m20260101_000001_gold", [f"ALTER DOMAIN \"{schema}\".ribbon SET DEFAULT 'gold'"])
            await self.migrator([gold], statement_cache_channel=None).up(pool)
            self.assertEqual((await pool.one(pack))["ribbon"], "gold")


if __name__ == "__main__":
    unittest.main()
