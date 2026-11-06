# Migrations

A database's schema changes as the program does. A *migration* is one such
change, written once and applied to every database the program runs against:
a developer's, a test server's, production's. The migration runner,
pgorm-migration's, records each migration it applies in a table of its own,
the *ledger*, so that each database receives each migration exactly once, in
the order the program lists them. A Python program runs its migrations with
that runner: `pgorm.schema.Migrations` is pgorm-migration's migrator built at
run time, and the ledger, the lock, the checksums and everything else this
chapter describes are the Rust runner's own.

Migrations run forward only. A migration has an `up` and nothing else: there
is no `down`, and no call that rolls back, refreshes or resets a database. A
mistake is corrected by the next migration.

## Adding the crate

In Rust, pgorm-migration is a crate of its own. The Python package carries
it: `pgorm.schema` exports `Migrations`, `SqlMigration` and the rest, and
there is nothing more to install.

## Writing a migration

A migration written in Python is an object with a `name` and an `up`. The
runner awaits `up(tx)` with the transaction the migration runs in, an
ordinary `Transaction`: statements, the query terminals, a savepoint
([Transactions](08-transactions.md)) and a COPY ([COPY](17-copy.md)) all run
on it, and `batch_execute(script)` runs SQL text of several statements as one
script, as a SQL file's migration runs ([Raw SQL](10-raw-sql.md)). Only the
runner commits or rolls it back, and once `up` returns the handle is closed.

A *migrator* lists every migration the program owns, in the order they
apply. `Migrations(migrations)` builds one, and its `up(pool)` applies the
migrations the database has not had yet:

```python
import asyncio
import os

from pgorm import Pool, RawSQL, Transaction
from pgorm import schema as s


class CreateDelivery:
    """Creates the bakery's deliveries."""

    name = "m20260201_000001_create_delivery"

    async def up(self, tx: Transaction) -> None:
        await tx.execute(
            s.create_table("delivery")
            .column(s.ColumnDef("id", "integer").not_null().auto_increment())
            .column(s.ColumnDef("order_id", "integer").not_null())
            .column(s.ColumnDef("address", "text").not_null())
            .primary_key("id")
            .foreign_key("order_id", "order", "id", name="fk-delivery-order_id")
        )
        # SQL text of several statements runs as one script.
        await tx.batch_execute(
            "CREATE INDEX delivery_order_index ON delivery (order_id);"
            "COMMENT ON TABLE delivery IS 'What the bakery sends out'"
        )


migrator = s.Migrations([CreateDelivery()])


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await migrator.up(pool)
        await pool.execute(RawSQL("INSERT INTO delivery (order_id, address) VALUES (1, 'Storgata 1, Oslo')"))

        # Running it again applies nothing: the ledger says the table exists.
        await migrator.up(pool)
        applied = await migrator.get_applied_migrations(pool)
        assert [migration.name for migration in applied] == ["m20260201_000001_create_delivery"]


asyncio.run(main())
```

`up` takes the pool, and checks connections out of it as the run needs
them. Its second argument bounds the run: `None`, the default, applies
everything pending, and a number `n` at most `n` migrations.

## Naming migrations

A migration's name is its identity in the ledger, so it must be unique and
must never change. Name each one for when it was written and what it does,
such as `m20260201_000001_create_delivery`; a migration written as SQL takes
the name of its file. The runner refuses a list that names one migration
twice, before it touches the database, with a `ConstructionError`. Renaming
a migration renames it: the old name is still in the ledger with no
migration to match, and the runner reports it as missing rather than
applying the new name a second time.

```python
import asyncio
import os

from pgorm import ConstructionError, Pool, Transaction
from pgorm import schema as s


class Noop:
    def __init__(self, name: str) -> None:
        self.name = name

    async def up(self, tx: Transaction) -> None:
        pass


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        twice = s.Migrations([Noop("m20260201_000001_noop"), Noop("m20260201_000001_noop")])
        try:
            await twice.up(pool)
        except ConstructionError as error:
            assert "Duplicate migration name(s): 'm20260201_000001_noop'" in str(error)
        else:
            raise AssertionError("one name twice is refused")

        await s.Migrations([Noop("m20260201_000001_noop")]).up(pool)
        renamed = s.Migrations([Noop("m20260201_000001_renamed")])
        try:
            await renamed.get_migration_with_status(pool)
        except ConstructionError as error:
            assert "'m20260201_000001_noop' is missing" in str(error)
        else:
            raise AssertionError("a renamed migration is reported missing")


asyncio.run(main())
```

## Migrations written as SQL

A migration can be a `.sql` file instead. `SqlMigration.read(path)` reads
one as Rust's `sql_migration!` does: named for the file's stem, its text run
whole in the batch's transaction, and its checksum the SHA-256 digest of
that text, so an edit made after it ran is caught, as the next sections
show. `SqlMigration(name, sql)` makes one from text the program holds. The
file is read when the program runs, and checked against PostgreSQL's grammar
as it is read, as `sql_migration!` checks one as the crate compiles: a file
with a syntax error is a `ConstructionError` naming it, before any migration
runs. Text given to `SqlMigration(name, sql)` is checked by the server when
it runs.

The guide keeps its frozen migrations in [`migrations/`](migrations), beside
[`bakery.py`](bakery.py):

```python
import asyncio
import os
from pathlib import Path

import bakery
from pgorm import Pool, RawSQL
from pgorm import schema as s

MIGRATIONS = Path(bakery.__file__).parent / "migrations"


async def main() -> None:
    delivery = s.SqlMigration.read(MIGRATIONS / "m20260201_000001_create_delivery.sql")
    assert delivery.name == "m20260201_000001_create_delivery"
    assert delivery.sql.startswith('CREATE TABLE "delivery" (')
    assert len(delivery.checksum) == 64

    index = s.SqlMigration(
        "m20260202_000001_index_delivery_order",
        "CREATE INDEX delivery_order_index ON delivery (order_id);",
    )
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await s.Migrations([delivery, index]).up(pool)
        found = await pool.one(RawSQL(
            "SELECT count(*) AS n FROM pg_indexes WHERE indexname = 'delivery_order_index'"
        ))
        assert found["n"] == 1


asyncio.run(main())
```

## One transaction, one runner at a time

`up` runs every pending migration in one transaction, with each migration's
ledger row, and commits once at the end. If any migration fails, the whole
run rolls back: the schema and the ledger are as they were before it
started. An exception a Python migration raises is raised by `up` as itself.

Before it reads the ledger, the run takes a PostgreSQL advisory lock keyed
by the ledger's name. Two deploys that start together therefore queue
rather than collide: the second waits until the first commits, then finds
nothing left to apply.

```python
import asyncio
import os

from pgorm import DatabaseError, Pool, RawSQL, Transaction
from pgorm import schema as s


class CreateNote:
    name = "m20260201_000001_create_customer_note"

    async def up(self, tx: Transaction) -> None:
        await tx.execute(RawSQL("CREATE TABLE customer_note (id serial PRIMARY KEY, note text NOT NULL)"))


class CreateCustomerAgain:
    """A migration that cannot succeed: the bakery has a customer table."""

    name = "m20260201_000002_create_customer_again"

    async def up(self, tx: Transaction) -> None:
        await tx.execute(RawSQL("CREATE TABLE customer (id serial PRIMARY KEY)"))


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # The second migration fails, so the first is rolled back with it.
        try:
            await s.Migrations([CreateNote(), CreateCustomerAgain()]).up(pool)
        except DatabaseError as error:
            assert error.sqlstate == "42P07"  # duplicate_table
        else:
            raise AssertionError("the bakery already has a customer table")
        row = await pool.one(RawSQL("SELECT count(*) AS n FROM pg_tables WHERE tablename = 'customer_note'"))
        assert row["n"] == 0

        # Two runners at once: one applies the migration, the other waits and
        # finds nothing to do.
        migrator = s.Migrations([CreateNote()])
        await asyncio.gather(migrator.up(pool), migrator.up(pool))
        assert len(await migrator.get_applied_migrations(pool)) == 1


asyncio.run(main())
```

### Committing partway

Some changes PostgreSQL will not let one transaction finish. A value that
`ALTER TYPE .. ADD VALUE` adds to an enum cannot be used until the
transaction that added it has committed. A migration making such a change
answers `True` from `commits_after`, an attribute beside its `name`. `up`
then commits the run so far once that migration and its ledger row are
written, and runs the rest in a new transaction, which takes the lock again.
A SQL file answers `True` when its first line is `-- pgorm: commits-after`,
as the guide's frozen `m20260301_000001_add_strawberry.sql` is.

Such a run is no longer all or nothing. A failure after the commit rolls
back only the transaction it happens in; the migrations committed before it
stay applied, and the next `up` resumes after them.

```python
import asyncio
import os
from pathlib import Path

import bakery
from pgorm import Pool, RawSQL, Transaction
from pgorm import schema as s

MIGRATIONS = Path(bakery.__file__).parent / "migrations"


class AddStrawberryTart:
    name = "m20260301_000002_add_strawberry_tart"

    async def up(self, tx: Transaction) -> None:
        await tx.execute(RawSQL(
            "INSERT INTO cake (name, flavor, price, gluten_free, bakery_id) "
            "VALUES ('Strawberry Tart', 'strawberry', 5.20, false, 1)"
        ))


async def main() -> None:
    strawberry = s.SqlMigration.read(MIGRATIONS / "m20260301_000001_add_strawberry.sql")
    assert strawberry.commits_after

    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await s.Migrations([strawberry, AddStrawberryTart()]).up(pool)
        row = await pool.one(RawSQL("SELECT name FROM cake WHERE flavor = 'strawberry'"))
        assert row["name"] == "Strawberry Tart"


asyncio.run(main())
```

[Disjoint enums](15-disjoint.md) produces changes of this kind: a change to
the kinds is a list of steps, one migration each, every step but the last
answering `True`. Frozen into migration files, each such step opens with the
line `-- pgorm: commits-after` ([Generated DDL in a
migration](#generated-ddl-in-a-migration)).

## The ledger

The ledger is a table named `pgorm_migrations`. It has one row per applied
migration: its `version`, the migration's name; `applied_at`, the Unix time
in seconds it was applied; and `checksum`, what the migration reported as
its checksum, or `NULL`. `up` creates the table the first time it runs.
Nothing in pgorm-migration deletes a row.

By default the ledger sits wherever `search_path` puts an unqualified name,
usually beside the application's tables in `public`. A migrator can place it
in a schema of its own with `migration_table_schema`, which keeps it apart
from triggers, policies and grants that apply to every application table,
and rename it with `migration_table_name`:

```python
import asyncio
import os

from pgorm import Pool, RawSQL, Transaction
from pgorm import schema as s


class CreateDelivery:
    name = "m20260201_000001_create_delivery"

    async def up(self, tx: Transaction) -> None:
        await tx.execute(RawSQL("CREATE TABLE delivery (id serial PRIMARY KEY, address text NOT NULL)"))


migrator = s.Migrations(
    [CreateDelivery()], migration_table_schema="ops", migration_table_name="bakery_migrations"
)


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await migrator.up(pool)
        row = await pool.one(RawSQL("SELECT version, checksum FROM ops.bakery_migrations"))
        assert row["version"] == "m20260201_000001_create_delivery"
        assert row["checksum"] is None


asyncio.run(main())
```

`up` creates the schema if it is missing, and names the table in it
everywhere, so a table of the same name elsewhere on `search_path` is never
taken for it. The runner does not move a ledger that already exists: a
ledger left in its old place reads as absent from the new one, and every
migration would run again. Move it with `ALTER TABLE .. SET SCHEMA` before
deploying a release that names a new place.

### A ledger from SeaORM

A database last migrated by SeaORM keeps its ledger under the name
`seaql_migrations`, with the same `version` and `applied_at` columns. The
first `up` renames that table to `pgorm_migrations` in place, so nothing
runs twice. This applies to the default name with no schema; a migrator that
names or places its own ledger takes the old table over by hand.

```python
import asyncio
import os

from pgorm import Pool, RawSQL, Transaction
from pgorm import schema as s


class CreateDelivery:
    name = "m20220101_000001_create_delivery"

    async def up(self, tx: Transaction) -> None:
        await tx.execute(RawSQL("CREATE TABLE delivery (id serial PRIMARY KEY)"))


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # What SeaORM left behind: the table, and the ledger row saying it exists.
        async with pool.transaction() as tx:
            await tx.execute(RawSQL("CREATE TABLE delivery (id serial PRIMARY KEY)"))
            await tx.execute(RawSQL(
                "CREATE TABLE seaql_migrations (version varchar PRIMARY KEY, applied_at bigint NOT NULL)"
            ))
            await tx.execute(RawSQL(
                "INSERT INTO seaql_migrations VALUES ('m20220101_000001_create_delivery', 1640995200)"
            ))

        # The migration is not run again, which would fail: the table exists.
        await s.Migrations([CreateDelivery()]).up(pool)
        row = await pool.one(RawSQL("SELECT applied_at FROM pgorm_migrations"))
        assert row["applied_at"] == 1640995200


asyncio.run(main())
```

## Asking what has run

`status` logs each migration's name and whether it is `Pending` or
`Applied`, at `INFO` through the `pgorm.migration` logger, or the
`logging.Logger` it is given. `get_migration_with_status`,
`get_pending_migrations` and `get_applied_migrations` answer the same
question as values: each a list of `Migration`s, a migration's `name` and
its `status`, `MigrationStatus.Pending` or `MigrationStatus.Applied`, in the
migrator's order. Each takes a pool, a connection or a transaction and only
reads: against a database never migrated, every migration is pending and no
ledger is created to say so. A readiness probe or a CI step can ask with a
role that has no right to change the schema.

```python
import asyncio
import logging
import os

from pgorm import Pool, RawSQL, Transaction
from pgorm import schema as s


class Create:
    def __init__(self, name: str, table: str) -> None:
        self.name = name
        self.table = table

    async def up(self, tx: Transaction) -> None:
        await tx.execute(RawSQL(f"CREATE TABLE {self.table} (id serial PRIMARY KEY)"))


migrator = s.Migrations([
    Create("m20260201_000001_create_delivery", "delivery"),
    Create("m20260201_000002_create_voucher", "voucher"),
])


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        assert len(await migrator.get_pending_migrations(pool)) == 2

        # Apply one migration of the two.
        await migrator.up(pool, 1)

        status = await migrator.get_migration_with_status(pool)
        assert status == [
            s.Migration("m20260201_000001_create_delivery", s.MigrationStatus.Applied),
            s.Migration("m20260201_000002_create_voucher", s.MigrationStatus.Pending),
        ]

        logging.basicConfig(level=logging.INFO)
        await migrator.status(pool)


asyncio.run(main())
```

## Catching an edited migration

The ledger matches migrations by name, so a migration edited after it ran
would go unnoticed: its name still matches, and a database built afresh
would quietly differ from the ones built before. A migration that reports a
`checksum` closes that gap. `up` stores the checksum in the migration's
ledger row, and every later `status` and `up` compares it with what the
migration reports now, failing with a `ConstructionError` that names the
migration when the two differ.

The checksum is any text the author controls, such as a hash of the
migration's SQL or a version bumped by hand, given as a `checksum`
attribute. It is off by default: a migration that reports none is never
checked, nor is a row written before it reported one. A `SqlMigration`
always reports one.

```python
import asyncio
import os

from pgorm import ConstructionError, Pool, RawSQL, Transaction
from pgorm import schema as s


class CreateDelivery:
    name = "m20260201_000001_create_delivery"

    def __init__(self, revision: str) -> None:
        self.checksum = revision

    async def up(self, tx: Transaction) -> None:
        await tx.execute(RawSQL("CREATE TABLE delivery (id serial PRIMARY KEY)"))


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await s.Migrations([CreateDelivery("1")]).up(pool)

        # The same migration, edited after it ran.
        try:
            await s.Migrations([CreateDelivery("2")]).up(pool)
        except ConstructionError as error:
            assert "'m20260201_000001_create_delivery' was applied with checksum '1'" in str(error)
        else:
            raise AssertionError("the edit is caught")


asyncio.run(main())
```

## Generated DDL in a migration

Schema generation writes a table's DDL from its description, so the
description is the one place the table is described. Python has it for a
Rust entity it registers, through `pgorm.schema.from_entity`, which
[Registering Rust entities](24-registrations.md#schema-from-a-registered-entity)
shows, and for a disjoint enum, through `create_disjoint`. A Python model is
not described as DDL: a program writes its tables' DDL with the builders, as
[`bakery.py`](bakery.py) does for the bakery.

But a description describes the table as it is now, while a migration
describes a change made once. A migration that generated its DDL as it ran
would follow the description as it changed: once the table gained a column,
a database built from nothing would get the column from the first migration
and then again from the migration that added it, and fail.

So generate the DDL once, when the change is made, and freeze it into a
migration written as SQL. `pgorm.schema.migration_file(name, statements)`
holds statements as a file's text, as pgorm's `MigrationFile` does: its
`name`, its `text`, each statement followed by `;` on a line of its own, and
`freeze(directory)`, which writes `<name>.sql` into the migrations directory
if no file of that name is there, answering whether it wrote it. A file
already there is never rewritten: it is the migration databases have run,
its checksum in their ledgers. So the code that freezes each change can live
beside the migrations and run again whenever a change is added, writing the
new file and keeping the old ones, however far the tables have moved on. A
name that is not a plain file name is a `ConstructionError`. Here the guide's
own migrations directory already holds the delivery table, frozen when it
was added, so freezing it again writes nothing:

```python
from pathlib import Path

import bakery
from pgorm import schema as s

MIGRATIONS = Path(bakery.__file__).parent / "migrations"

create = (
    s.create_table("delivery")
    .column(s.ColumnDef("id", "integer").not_null().auto_increment())
    .column(s.ColumnDef("order_id", "integer").not_null())
    .column(s.ColumnDef("address", "text").not_null())
    .primary_key("id")
    .foreign_key("order_id", "order", "id", name="fk-delivery-order_id")
)
file = s.migration_file("m20260201_000001_create_delivery", [create])

assert file.name == "m20260201_000001_create_delivery"
assert file.text == (
    'CREATE TABLE "delivery" ( "id" serial NOT NULL, "order_id" integer NOT NULL, '
    '"address" text NOT NULL, PRIMARY KEY ("id"), '
    'CONSTRAINT "fk-delivery-order_id" FOREIGN KEY ("order_id") REFERENCES "order" ("id") );\n'
)
assert not file.commits_after
assert not file.freeze(MIGRATIONS)

# The file reads back as the migration the runner applies.
assert s.SqlMigration.read(MIGRATIONS / f"{file.name}.sql").sql == file.text
```

The text is exactly what pgorm writes for the same statements, so a Rust
migration crate can equally compile the files a Python program freezes with
`sql_migration!`. A change PostgreSQL will not let one transaction finish is
frozen into several files, a
[disjoint enum](15-disjoint.md#freezing-a-change-into-migrations)'s new kind
into one per step. `migration_file(.., commits_after=True)` opens a file with
the line `-- pgorm: commits-after`, and `commits_after` answers whether it
does, so its batch commits once it has run, as
[Committing partway](#committing-partway) describes.

## A migrator built at run time

Every migrator in Python is built as the program runs, so a migrator's list
can be read from disk. `SqlMigration.read_dir(directory)` reads every `.sql`
file in a directory, in the order of their names, each as `read` reads one.
So a program replays the files it has frozen without naming each, with its
own Python migrations listed among them where they belong. Here the guide's
frozen history, the delivery table and the strawberry flavor, is replayed
on the bakery, then a migration written in Python uses the new flavor:

```python
import asyncio
import os
from pathlib import Path

import bakery
from pgorm import Pool, RawSQL, Transaction
from pgorm import schema as s

MIGRATIONS = Path(bakery.__file__).parent / "migrations"


class AddStrawberryTart:
    name = "m20260301_000002_add_strawberry_tart"

    async def up(self, tx: Transaction) -> None:
        await tx.execute(RawSQL(
            "INSERT INTO cake (name, flavor, price, gluten_free, bakery_id) "
            "VALUES ('Strawberry Tart', 'strawberry', 5.20, false, 1)"
        ))


async def main() -> None:
    frozen = s.SqlMigration.read_dir(MIGRATIONS)
    assert [file.name for file in frozen] == [
        "m20260201_000001_create_delivery",
        "m20260301_000001_add_strawberry",
    ]
    migrator = s.Migrations([*frozen, AddStrawberryTart()])

    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await migrator.up(pool)
        applied = await migrator.get_applied_migrations(pool)
        assert [migration.name for migration in applied] == [
            "m20260201_000001_create_delivery",
            "m20260301_000001_add_strawberry",
            "m20260301_000002_add_strawberry_tart",
        ]
        recorded = await pool.one(RawSQL(
            "SELECT checksum FROM pgorm_migrations WHERE version = 'm20260201_000001_create_delivery'"
        ))
        assert recorded["checksum"] == frozen[0].checksum


asyncio.run(main())
```

## Statement caches

pgorm prepares each statement once per pooled connection and reuses it. Most
schema changes make PostgreSQL plan the statement again, but some do not: an
insert prepared before a domain's default changed keeps writing the old
default ([Statement caches and DDL](19-statement-caches.md)). So once a run
that applied a migration commits, `up` clears the statement caches of the
pool it was given:

```python
import asyncio
import os

from pgorm import Pool, RawSQL, Transaction, TypeName
from pgorm import schema as s

PACK = RawSQL("INSERT INTO box (contents) VALUES ($1) RETURNING ribbon", ["truffles"])


class GoldRibbon:
    name = "m20260201_000001_gold_ribbon"

    async def up(self, tx: Transaction) -> None:
        await tx.execute(s.alter_domain("ribbon").set_default("gold"))


async def main() -> None:
    # One connection, so every statement runs on the same session.
    async with Pool(os.environ["DATABASE_URL"], max_size=1) as pool:
        async with pool.transaction() as tx:
            await tx.execute(s.create_domain("ribbon", "text").default("red"))
            await tx.execute(
                s.create_table("box")
                .column(s.ColumnDef("id", "integer").not_null().auto_increment())
                .column(s.ColumnDef("contents", "text"))
                .column(s.ColumnDef("ribbon", TypeName("ribbon")))
                .primary_key("id")
            )
        assert (await pool.one(PACK))["ribbon"] == "red"

        # The insert prepared before the change would still write the old
        # default, but the run clears the pool's caches as it commits.
        await s.Migrations([GoldRibbon()]).up(pool)
        assert (await pool.one(PACK))["ribbon"] == "gold"


asyncio.run(main())
```

`up` also sends a notification on `statement_cache_channel`,
`pgorm.STATEMENT_CACHE_CHANNEL` (`pgorm_statement_caches`) unless the
migrator is given another, from inside the run's transaction, so it is
delivered exactly when the run commits. A pool built with
`statement_cache_channel=pgorm.STATEMENT_CACHE_CHANNEL` listens there and
clears its caches on every notification, so another process's pool, a Rust
program's or a Python one's, hears this one migrate the database:

```python
import asyncio
import os

from pgorm import STATEMENT_CACHE_CHANNEL, Pool, RawSQL, Transaction, TypeName
from pgorm import schema as s

PACK = RawSQL("INSERT INTO box (contents) VALUES ($1) RETURNING ribbon", ["fudge"])


class GoldRibbon:
    name = "m20260201_000001_gold_ribbon"

    async def up(self, tx: Transaction) -> None:
        await tx.execute(s.alter_domain("ribbon").set_default("gold"))


async def main() -> None:
    url = os.environ["DATABASE_URL"]
    async with Pool(url) as runner, Pool(url, max_size=1, statement_cache_channel=STATEMENT_CACHE_CHANNEL) as shop:
        async with runner.transaction() as tx:
            await tx.execute(s.create_domain("ribbon", "text").default("red"))
            await tx.execute(
                s.create_table("box")
                .column(s.ColumnDef("id", "integer").not_null().auto_increment())
                .column(s.ColumnDef("contents", "text"))
                .column(s.ColumnDef("ribbon", TypeName("ribbon")))
                .primary_key("id")
            )
        assert (await shop.one(PACK))["ribbon"] == "red"

        await s.Migrations([GoldRibbon()]).up(runner)

        # The clear arrives as a notification, so the shop sees it a moment later.
        async def gold() -> str:
            while (ribbon := (await shop.one(PACK))["ribbon"]) != "gold":
                await asyncio.sleep(0.05)
            return str(ribbon)

        assert await asyncio.wait_for(gold(), timeout=10) == "gold"


asyncio.run(main())
```

A migrator built with `statement_cache_channel=None` sends nothing, and a
pool built without the option does not listen.
[Statement caches and DDL](19-statement-caches.md) covers both in full.

## A migration crate

There is no command-line tool. A Python program runs its migrations itself:
as a step of each deploy, a small script that builds the migrator and calls
`up`, or at startup, before it serves. The advisory lock makes that safe when
several replicas start at once.

A program whose migrations are written in Rust keeps them in a crate of
their own instead, with a small binary that runs them. A starter crate lives
in pgorm's repository at `pgorm-migration/template/migration/`. Its
`src/lib.rs` holds the migrator, one file per migration beside it, and its
`src/main.rs` reads `DATABASE_URL` and runs `up`, `up <n>` or `status`:

```sh
DATABASE_URL=postgres://localhost/bakery cargo run -p migration -- up
DATABASE_URL=postgres://localhost/bakery cargo run -p migration -- status
```

Both runners keep the same ledger, so a database can be migrated by either,
and a migration a Python program froze into a file runs in the Rust crate
through `sql_migration!` with the checksum it has here.
