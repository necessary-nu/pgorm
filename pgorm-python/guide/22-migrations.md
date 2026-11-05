# Migrations

A database's schema changes as the program does. A *migration* is one such
change, written once and applied to every database the program runs against:
a developer's, a test server's, production's. In Rust, `pgorm-migration`
runs them. It records each migration it applies in a table of its own, the
*ledger*, so that each database receives each migration exactly once, in the
order the program lists them. Migrations run forward only: a migration has
an `up` and nothing else, and a mistake is corrected by the next migration.

Here Python differs from Rust: the migration runner is not available in
Python yet. pgorm-migration is a Rust crate, and the Python package has no
counterpart of its `MigrationTrait`, its `MigratorTrait`, its ledger, its
`up` and `status`, or its checksums. A Python application has two ways to
change its schema today. It can keep a Rust migration crate beside it and
run it as a step of each deploy, so that the runner's ledger, lock and
checksums keep the Python application's database too. Or it can run its own
DDL, built with `pgorm.schema` ([Schema generation](14-schema.md)), in a
transaction. This
chapter follows the Rust chapter's sections, and at each says what the
runner does and what a Python program can do.

## Adding the crate

pgorm-migration lives in pgorm's repository beside pgorm, and a Rust crate
depends on it the same way:

```toml
[dependencies]
pgorm-migration = { git = "https://github.com/necessary-nu/pgorm" }
```

There is nothing to add on the Python side. The migration crate is a Rust
program of its own, built and run by Cargo, and the Python application talks
to the tables it creates. [A migration crate](#a-migration-crate) at the end
of this chapter sets one up.

## Writing a migration

A Rust migration is a type whose `up` receives the transaction it runs in
and changes the schema through it, and a migrator lists the migrations in the
order they apply. A Python program has no such types, but it has the part
that does the work: DDL statements run in one transaction. `pgorm.schema`
builds each statement, as [Schema generation](14-schema.md) describes, and a
transaction runs them all or none ([Transactions](08-transactions.md)). Here
a program adds a table for the bakery's deliveries:

```python
import asyncio
import os

from pgorm import Column, Model, Pool
from pgorm import schema as s

Delivery = Model("delivery", {
    "id": Column("i32", primary_key=True, default=True),
    "order_id": Column("i32"),
    "address": Column("text"),
})


async def main() -> None:
    create = (
        s.create_table("delivery")
        .column(s.ColumnDef("id", "integer").not_null().auto_increment())
        .column(s.ColumnDef("order_id", "integer").not_null())
        .column(s.ColumnDef("address", "text").not_null())
        .primary_key("id")
        .foreign_key("order_id", "order", "id", name="fk-delivery-order_id")
    )
    assert create.inspect().sql == (
        'CREATE TABLE "delivery" ( "id" serial NOT NULL, "order_id" integer NOT NULL, '
        '"address" text NOT NULL, PRIMARY KEY ("id"), '
        'CONSTRAINT "fk-delivery-order_id" FOREIGN KEY ("order_id") REFERENCES "order" ("id") )'
    )

    async with Pool(os.environ["DATABASE_URL"]) as pool:
        async with pool.transaction() as tx:
            await tx.execute(create)
            await tx.execute(s.create_index("delivery", "order_id", name="delivery_order_id"))

        delivery = await Delivery.insert({"order_id": 1, "address": "Storgata 1, Oslo"}).returning().one(pool)
        assert delivery["id"] == 1


asyncio.run(main())
```

A statement built this way is one command. Here Python differs from Rust:
Rust's `batch_execute` runs a script of several statements through the
simple-query protocol, and Python has no counterpart yet. Every statement a
Python program sends is prepared, and PostgreSQL refuses to prepare two
commands at once, so SQL text of several statements runs one statement at a
time, each a `RawSQL` of its own ([Raw SQL](10-raw-sql.md)).

## Naming migrations

The runner knows a migration by its name, which its ledger row records, so
the name must be unique and must never change. In a Rust migration crate
each migration is a file named for when it was written and what it does,
such as `m20260101_000001_create_bakery.rs`, and `#[derive(MigrationName)]`
names it for the file. The runner refuses a list that names one migration
twice, and reports a name in the ledger with no migration to match as
missing. None of this has a Python counterpart: the names are the Rust
crate's.

## Migrations written as SQL

In Rust, `sql_migration!` makes a migration of a `.sql` file, checks the
file against PostgreSQL's grammar while the crate compiles, and reports the
file's SHA-256 digest as its checksum. The macro is Rust's alone. A Python
program can read a `.sql` file and run its statements in a transaction, one
`RawSQL` each, but nothing checks the file before it runs: a syntax error is
the server's to report when the statement is sent.

## One transaction, one runner at a time

The runner applies every pending migration in one transaction, with each
migration's ledger row, and commits once at the end, so a run that fails
leaves the schema and the ledger as they were. A Python program's DDL gets
the same all or nothing from a transaction, with no ledger to keep: when one
statement fails, the ones before it roll back with it. Here the second statement cannot succeed,
since the bakery already has a `customer` table:

```python
import asyncio
import os

from pgorm import DatabaseError, Pool, RawSQL
from pgorm import schema as s


async def main() -> None:
    notes = (
        s.create_table("customer_note")
        .column(s.ColumnDef("id", "integer").not_null().auto_increment())
        .column(s.ColumnDef("note", "text").not_null())
        .primary_key("id")
    )
    customer_again = s.create_table("customer").column(s.ColumnDef("id", "integer").not_null())

    async with Pool(os.environ["DATABASE_URL"]) as pool:
        try:
            async with pool.transaction() as tx:
                await tx.execute(notes)
                await tx.execute(customer_again)
        except DatabaseError as error:
            assert error.sqlstate == "42P07"  # duplicate_table
        else:
            raise AssertionError("the bakery already has a customer table")

        # The first table was rolled back with the second.
        row = await pool.one(RawSQL("SELECT count(*) AS n FROM pg_tables WHERE tablename = 'customer_note'"))
        assert row["n"] == 0


asyncio.run(main())
```

Before it reads the ledger, the runner takes a PostgreSQL advisory lock
keyed by the ledger's name, so two deploys that start together queue rather
than collide: the second waits until the first commits, then finds nothing
left to apply. A Python program that changes its schema as it starts, in
several replicas at once, can take a lock of its own the same way, as the
first statement of its transaction. `pg_advisory_xact_lock` waits for the
lock and holds it until the transaction ends. Deciding what is left to do
once it holds the lock is the program's own work, since there is no ledger
to ask; here each statement says `IF NOT EXISTS`:

```python
import asyncio
import os

from pgorm import Pool, RawSQL
from pgorm import schema as s

# Any key every replica of the program agrees on.
SCHEMA_LOCK = 20260101

NOTES = (
    s.create_table("customer_note")
    .if_not_exists()
    .column(s.ColumnDef("id", "integer").not_null().auto_increment())
    .column(s.ColumnDef("note", "text").not_null())
    .primary_key("id")
)


async def change_schema(pool: Pool) -> None:
    async with pool.transaction() as tx:
        await tx.execute(RawSQL("SELECT pg_advisory_xact_lock($1)", [SCHEMA_LOCK]))
        await tx.execute(NOTES)


async def main() -> None:
    assert NOTES.inspect().sql == (
        'CREATE TABLE IF NOT EXISTS "customer_note" ( "id" serial NOT NULL, '
        '"note" text NOT NULL, PRIMARY KEY ("id") )'
    )
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # Two replicas starting at once: the second waits for the first.
        await asyncio.gather(change_schema(pool), change_schema(pool))
        row = await pool.one(RawSQL("SELECT count(*) AS n FROM pg_tables WHERE tablename = 'customer_note'"))
        assert row["n"] == 1


asyncio.run(main())
```

### Committing partway

Some changes PostgreSQL will not let one transaction finish. A value that
`ALTER TYPE .. ADD VALUE` adds to an enum cannot be used until the
transaction that added it has committed. A Rust migration making such a
change answers `true` from `commits_after`, and the runner commits after it
and runs the rest in a new transaction. A Python program does the same with
two transactions:

```python
import asyncio
import os

from pgorm import DatabaseError, Pool, RawSQL
from pgorm import schema as s

STRAWBERRY_TART = RawSQL(
    "INSERT INTO cake (name, flavor, price, gluten_free, bakery_id) "
    "VALUES ('Strawberry Tart', 'strawberry', 5.20, false, 1)"
)


async def main() -> None:
    add = s.alter_type("flavor").add_value("strawberry")
    assert add.inspect().sql == "ALTER TYPE \"flavor\" ADD VALUE 'strawberry'"

    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # In one transaction the new value cannot be used yet.
        try:
            async with pool.transaction() as tx:
                await tx.execute(add)
                await tx.execute(STRAWBERRY_TART)
        except DatabaseError as error:
            assert error.sqlstate == "55P04"  # unsafe_new_enum_value_usage
        else:
            raise AssertionError("a new enum value waits for its commit")

        # Committed first, it can.
        async with pool.transaction() as tx:
            await tx.execute(add)
        async with pool.transaction() as tx:
            await tx.execute(STRAWBERRY_TART)

        row = await pool.one(RawSQL("SELECT name FROM cake WHERE flavor = 'strawberry'"))
        assert row["name"] == "Strawberry Tart"


asyncio.run(main())
```

Such a change is no longer all or nothing: a failure in the second
transaction leaves the first committed. [Disjoint enums](15-disjoint.md)
produces changes of this kind, a list of steps, each but the last to be
committed before the next runs.

## The ledger

The runner's ledger is a table named `pgorm_migrations`, with one row per
applied migration: its `version`, the migration's name; `applied_at`, the
Unix time in seconds it was applied; and `checksum`, what the migration
reported as its checksum, or `NULL`. A migrator can rename it with
`migration_table_name` and place it in a schema of its own with
`migration_table_schema`; the runner creates the table, and the schema, the
first time it runs, and never moves one that exists. A database last
migrated by SeaORM keeps its ledger as `seaql_migrations`, which the first
`up` renames to `pgorm_migrations` in place.

Python has no ledger of its own, and nothing in the package writes this one.
It is an ordinary table, though, so a Python program can read it.

## Asking what has run

The runner's `status` and `get_migration_with_status` report each
migration as `Pending` or `Applied`, reading the ledger and nothing else, so
that a readiness probe or a CI step can ask with a role that may not change
the schema. They are not available in Python yet, but the question they
answer is a read of the ledger, which a Python program can make itself.
Declare the ledger as a model, read the names it holds, and compare them
with the migrations the deploy is meant to have applied. Here the example
creates the ledger and writes its rows as the runner would have, then asks:

```python
import asyncio
import os

from pgorm import Column, Model, Pool
from pgorm import schema as s

Ledger = Model("pgorm_migrations", {
    "version": Column("text", primary_key=True),
    "applied_at": Column("i64"),
    "checksum": Column("text", nullable=True),
})

# The migrations this release of the program expects, in order.
EXPECTED = [
    "m20260101_000001_create_bakery",
    "m20260102_000001_index_cake_name",
    "m20260103_000001_create_delivery",
]


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # What the runner leaves behind after applying the first two.
        await pool.execute(
            s.create_table("pgorm_migrations")
            .column(s.ColumnDef("version", "text").not_null())
            .column(s.ColumnDef("applied_at", "bigint").not_null())
            .column(s.ColumnDef("checksum", "text").null())
            .primary_key("version")
        )
        await Ledger.insert_many([
            {"version": EXPECTED[0], "applied_at": 1767225600, "checksum": None},
            {"version": EXPECTED[1], "applied_at": 1767312000, "checksum": "3f1a"},
        ]).execute(pool)

        applied = {row["version"] for row in await Ledger.find().all(pool)}
        pending = [name for name in EXPECTED if name not in applied]
        assert pending == ["m20260103_000001_create_delivery"]


asyncio.run(main())
```

A program that starts only once its schema is current can make this check
at startup and refuse to serve while anything is pending.

## Catching an edited migration

A Rust migration that reports a `checksum` has it stored in its ledger row,
and every later `status` and `up` compares the two, failing with an error
that names the migration when it was edited after it ran. The check is the
runner's, so it is not available in Python yet. The checksums are in the
ledger's `checksum` column for a Python program to read, but what a
migration reports now is known only to the Rust crate.

## Generated DDL in a migration

A Rust migration can create a table from its entity with schema generation,
so the entity is the one place the table is described. Python has the same
generation for a Rust entity it registers: `pgorm.schema.from_entity`, which
[Registering Rust entities](24-registrations.md#schema-from-a-registered-entity)
shows, gives the entity's `CREATE TYPE`, `CREATE TABLE`, index and comment
statements, to run in a transaction as the examples above run theirs. A
Python model is not described as DDL: a program writes its tables' DDL with
the builders, as [`bakery.py`](bakery.py) does for the bakery.

An entity describes the table as it is now, while a migration describes a
change at one moment. DDL generated from an entity applies the entity as it
stands when the DDL runs, so once a change has shipped and the entity
changes again, write the next change as a new step rather than letting the
old one follow the entity.

## Statement caches

pgorm prepares each statement once per pooled connection and reuses it. Most
schema changes make PostgreSQL plan the statement again, but some do not: an
insert prepared before a domain's default changed keeps writing the old
default ([Statement caches and DDL](19-statement-caches.md)). So once a run
that applied a migration commits, the Rust runner clears the statement
caches of the pool it was given, and a Python program that changes the
schema through its own pool does the same with `clear_statement_caches()`:

```python
import asyncio
import os

from pgorm import Pool, RawSQL, TypeName
from pgorm import schema as s

PACK = RawSQL("INSERT INTO box (contents) VALUES ($1) RETURNING ribbon", ["truffles"])


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

        async with pool.transaction() as tx:
            await tx.execute(s.alter_domain("ribbon").set_default("gold"))
        # The insert prepared before the change still writes the old default.
        assert (await pool.one(PACK))["ribbon"] == "red"

        pool.clear_statement_caches()
        assert (await pool.one(PACK))["ribbon"] == "gold"


asyncio.run(main())
```

The runner also sends a notification on the channel
`pgorm.STATEMENT_CACHE_CHANNEL`, `pgorm_statement_caches`, from inside the
run's transaction, so it is delivered exactly when the run commits. A pool
built with `statement_cache_channel=pgorm.STATEMENT_CACHE_CHANNEL` listens
there and clears its caches on every notification, so a Python process's
pool hears the Rust runner of another process migrate the database. Here
the second pool stands in for the runner, sending the notification as it
does:

```python
import asyncio
import os

from pgorm import STATEMENT_CACHE_CHANNEL, Pool, RawSQL, TypeName
from pgorm import schema as s

PACK = RawSQL("INSERT INTO box (contents) VALUES ($1) RETURNING ribbon", ["fudge"])


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

        # What the runner does: change the schema and notify, in one transaction.
        async with runner.transaction() as tx:
            await tx.execute(s.alter_domain("ribbon").set_default("gold"))
            await tx.notify(STATEMENT_CACHE_CHANNEL, "")

        # The clear arrives as a notification, so the shop sees it a moment later.
        async def gold() -> str:
            while (ribbon := (await shop.one(PACK))["ribbon"]) != "gold":
                await asyncio.sleep(0.05)
            return str(ribbon)

        assert await asyncio.wait_for(gold(), timeout=10) == "gold"


asyncio.run(main())
```

A Rust migrator whose `statement_cache_channel` answers `None` sends
nothing, and a pool built without the option does not listen.
[Statement caches and DDL](19-statement-caches.md) covers both in full.

## A migration crate

There is no command-line tool: a program's migrations are a Rust crate of
their own with a small binary that runs them. A starter crate lives in
pgorm's repository at `pgorm-migration/template/migration/`. Its
`src/lib.rs` holds the migrator, one file per migration beside it, and its
`src/main.rs` reads `DATABASE_URL` and runs `up`, `up <n>` or `status`:

```sh
DATABASE_URL=postgres://localhost/bakery cargo run -p migration -- up
DATABASE_URL=postgres://localhost/bakery cargo run -p migration -- status
```

Copy it into the project, write the migrations in Rust as [the Rust
guide's Migrations](../../docs/guide/21-migrations.md) teaches, and run the
binary as a step of each deploy, before the Python application starts. A
Rust program can instead call `Migrator::up(pool, None)` itself at startup;
a Python program cannot, so for a Python application the binary is the way
to run them.
