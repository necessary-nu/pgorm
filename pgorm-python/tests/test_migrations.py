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
    # [spec:pgorm:req:bindings.migrations+1/test]    a file reads as pgorm-migration reads it: name, digest, directive
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

    # [spec:pgorm:req:bindings.migrations+1/test]    a migration is a SqlMigration or an object with a name and an up
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

    # [spec:pgorm:req:bindings.migrations+1/test]    files and Python migrations run in order, once, through the runner
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
            [s.MigrationWithStatus(migration.name, s.MigrationStatus.Applied) for migration in pending],
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

    # [spec:pgorm:req:bindings.migrations+1/test]    a migration's exception rolls the batch back and is raised as itself
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

    # [spec:pgorm:req:bindings.migrations+1/test]    up's transaction takes savepoints and statements, commits_after commits
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

    # [spec:pgorm:req:bindings.migrations+1/test]    the runner clears the pool's statement caches after a batch
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

    def ahead(self):
        """The two migrations a newer build of a program has, applied by it."""
        schema = self.schema
        return (
            s.SqlMigration(
                "m20261101_000001_table_a",
                f'CREATE SCHEMA "{schema}"; CREATE TABLE "{schema}".table_a (id integer)',
            ),
            s.SqlMigration("m20261101_000002_table_b", f'CREATE TABLE "{schema}".table_b (id integer)'),
        )

    async def ledger_rows(self):
        return await self.scalar(f'SELECT count(*) AS value FROM "{self.schema}_ops".ledger')

    # [spec:pgorm:req:bindings.migrations.unknown-applied/test]    an older build that warns goes on, applying
    # nothing, and each read logs each unknown migration once, as a warning of the runner's words
    async def test_an_older_build_that_warns_goes_on(self):
        a, b = self.ahead()
        await self.migrator([a, b]).up(self.pool)
        older = self.migrator([a], unknown_migrations="warn")
        warning = (
            "WARNING:pgorm.migration:The database records migration 'm20261101_000002_table_b' as applied, "
            "but this migrator does not know it, so the database is ahead of this build"
        )

        with self.assertLogs("pgorm.migration", "WARNING") as logged:
            await older.up(self.pool)
        self.assertEqual(logged.output, [warning])
        self.assertEqual(await self.ledger_rows(), 2)

        with self.assertLogs("pgorm.migration", "INFO") as logged:
            await older.status(self.pool)
        self.assertEqual(logged.output, [
            "INFO:pgorm.migration:Checking migration status",
            f"INFO:pgorm.migration:Migration '{a.name}'... Applied",
            warning,
        ])

        with self.assertLogs("pgorm.migration", "WARNING") as logged:
            self.assertEqual(
                await older.get_migration_with_status(self.pool), [s.MigrationWithStatus(a.name, s.MigrationStatus.Applied)]
            )
            self.assertEqual(await older.get_pending_migrations(self.pool), [])
            self.assertEqual(len(await older.get_applied_migrations(self.pool)), 1)
        self.assertEqual(logged.output, [warning, warning, warning])

    # [spec:pgorm:req:bindings.migrations.unknown-applied/test]    refusing is the default, and the setting is
    # "refuse" or "warn" and nothing else
    async def test_an_older_build_refuses_by_default(self):
        a, b = self.ahead()
        await self.migrator([a, b]).up(self.pool)
        missing = (
            f"Migration file of version '{b.name}' is missing, this migration has been applied but its file is missing"
        )
        for older in (self.migrator([a]), self.migrator([a], unknown_migrations="refuse")):
            for read in (older.up, older.status, older.get_pending_migrations):
                with self.assertRaises(p.ConstructionError) as refused:
                    await read(self.pool)
                self.assertIn(missing, str(refused.exception))
        for setting in ("ignore", None, 1):
            with self.assertRaisesRegex(TypeError, "unknown_migrations is one of \"refuse\", \"warn\""):
                self.migrator([a], unknown_migrations=setting)

    # [spec:pgorm:req:bindings.migrations.unknown-applied/test]    a pending migration beside an unknown one is
    # refused, naming both, whatever steps bounds
    async def test_pending_beside_unknown_is_refused_under_warn(self):
        a, b = self.ahead()
        await self.migrator([a, b]).up(self.pool)
        c = s.SqlMigration("m20261101_000003_table_c", f'CREATE TABLE "{self.schema}".table_c (id integer)')
        diverged = self.migrator([a, c], unknown_migrations="warn")
        for steps in (None, 1, 0):
            with self.assertRaises(p.ConstructionError) as refused:
                await diverged.up(self.pool, steps)
            message = str(refused.exception)
            self.assertIn(f"'{b.name}'", message)
            self.assertIn(f"'{c.name}'", message)
            self.assertIn("out of order", message)
        self.assertEqual(await self.ledger_rows(), 2)
        with self.assertLogs("pgorm.migration", "WARNING"):
            pending = await diverged.get_pending_migrations(self.pool)
        self.assertEqual(pending, [s.MigrationWithStatus(c.name, s.MigrationStatus.Pending)])

    # [spec:pgorm:req:bindings.migrations.unknown-applied/test]    a changed checksum is refused under "warn" too
    async def test_a_changed_checksum_is_refused_under_warn(self):
        a, b = self.ahead()
        await self.migrator([a, b]).up(self.pool)
        edited = self.migrator([s.SqlMigration(a.name, a.sql + ";")], unknown_migrations="warn")
        for read in (edited.up, edited.status, edited.get_migration_with_status):
            with self.assertRaises(p.ConstructionError) as refused:
                await read(self.pool)
            self.assertIn(f"Migration '{a.name}' was applied with checksum", str(refused.exception))
            self.assertNotIn(b.name, str(refused.exception))


if __name__ == "__main__":
    unittest.main()
