# Schema generation

A program's tables have to exist before it can use them. pgorm writes the
DDL that creates them in two ways: from entities a Rust crate declared, and
from DDL builders for everything else, such as a model's table, a view, a
sequence or a change to an existing table. Both live in `pgorm.schema`, and
both only build statements. Building one sends nothing, and neither does
importing a module or declaring a model. You run each statement yourself,
usually in a [migration](22-migrations.md), with `execute`.

Here Python differs from Rust. In Rust a statement becomes its text with
`to_string()`, and `batch_execute` runs the text. In Python `execute` takes
the statement itself, on a pool, a connection or a transaction, and
`inspect().sql` shows its text.

Schema generation writes; it does not read. pgorm never inspects a database
to compare it with your models or entities.

## Tables from entities

Rust's `Schema` turns an entity into the statements that create its table.
Python reaches it for a [registered Rust entity](24-registrations.md) through
`pgorm.schema.from_entity`, which calls those Rust methods for the entity and
returns an `EntitySchema`:

- `table`, the `CREATE TABLE`, with its primary key, its unique keys and a
  foreign key for each `belongs_to` relation;
- `enums`, a `CREATE TYPE .. AS ENUM` for each PostgreSQL enum its columns
  use;
- `indexes`, a `CREATE INDEX` for each `indexed` column;
- `comments`, a `COMMENT ON` for each comment.

Each read of one of them gives new statement objects. Types come before the
tables that use them, and a table before the tables whose foreign keys point
at it, so run the enums first, then the table, then its indexes and
comments, and create the entities in the order their foreign keys need. Two
entities that use one enum each list it, so create it once.

`pgorm.entities()` lists the names of the entities the installed wheel
registers. The guide's registration build registers the bakery's Rust
entities as `rust_bakery.Bakery`, `rust_bakery.Cake` and the rest. This is
what Rust writes for the cake:

```python registered
import pgorm
from pgorm import schema as s

from bakery import statements

cake = s.from_entity(pgorm.entity("rust_bakery.Cake"))

assert [statement.inspect().sql for statement in cake.enums] == [
    """CREATE TYPE "flavor" AS ENUM ('chocolate', 'vanilla', 'lemon', 'carrot')"""
]
assert cake.table.inspect().sql == (
    'CREATE TABLE "cake" ( "id" serial NOT NULL, "name" varchar NOT NULL, '
    '"flavor" flavor NOT NULL, "price" decimal(10, 2) NOT NULL, "gluten_free" bool NOT NULL, '
    '"bakery_id" integer NOT NULL, PRIMARY KEY ("id"), '
    'CONSTRAINT "fk-cake-bakery_id" FOREIGN KEY ("bakery_id") REFERENCES "bakery" ("id") )'
)
assert len(cake.indexes) == 0 and len(cake.comments) == 0

# bakery.py writes the same table with the DDL builders.
assert cake.table.inspect().sql == statements()[3].inspect().sql
```

The names generation gives are fixed: a composite primary key is
`pk-{table}`, a foreign key `fk-{table}-{columns}`, an index
`idx-{table}-{column}`. A relation's `fk_name` names its foreign key itself.
A derived name longer than PostgreSQL's 63 bytes is shortened to 47 bytes and
a hash, so two long names never collide.

`from_entity` refuses an entity Rust cannot write a table for, with a
`ConstructionError` carrying Rust's message: a unique key with no columns,
which only a hand-written `EntityName` can describe, and a `belongs_to`
relation with a column typed as a domain over an enum on either side, a
foreign key PostgreSQL cannot build. Rust's refusal is `Error::Type`.

Generating a table from a model declared in Python is not available in
Python yet. `from_entity` takes only a registered entity, and refuses a
`pgorm.Model` with a `TypeError`. A model's table is written with the [DDL
builders](#the-ddl-builders) instead, column by column, as
[`bakery.py`](bakery.py)'s `statements()` writes the bakery's. The one
exception is a [disjoint model](15-disjoint.md), whose tables
`create_disjoint` writes from its models. Here is the cake's table, written
to match what Rust generates. `bakery.create` ran the same statement on the
database every example starts from, so its foreign key already holds there:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import ForeignKeyViolationError, Pool, TypeName
from pgorm import schema as s

from bakery import Cake, statements

cake = (
    s.create_table("cake")
    .column(s.ColumnDef("id", "integer").not_null().auto_increment())
    .column(s.ColumnDef("name", "varchar").not_null())
    .column(s.ColumnDef("flavor", TypeName("flavor")).not_null())
    .column(s.ColumnDef("price", s.DataType("numeric", precision=10, scale=2)).not_null())
    .column(s.ColumnDef("gluten_free", "boolean").not_null())
    .column(s.ColumnDef("bakery_id", "integer").not_null())
    .primary_key("id")
    .foreign_key("bakery_id", "bakery", "id", name="fk-cake-bakery_id")
)
assert cake.inspect().sql == (
    'CREATE TABLE "cake" ( "id" serial NOT NULL, "name" varchar NOT NULL, '
    '"flavor" flavor NOT NULL, "price" decimal(10, 2) NOT NULL, "gluten_free" bool NOT NULL, '
    '"bakery_id" integer NOT NULL, PRIMARY KEY ("id"), '
    'CONSTRAINT "fk-cake-bakery_id" FOREIGN KEY ("bakery_id") REFERENCES "bakery" ("id") )'
)
assert cake.inspect().sql == statements()[3].inspect().sql


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # A cake needs its bakery.
        try:
            await Cake.insert({
                "name": "Nowhere Cake",
                "flavor": "vanilla",
                "price": Decimal("1.00"),
                "gluten_free": False,
                "bakery_id": 99,
            }).execute(pool)
        except ForeignKeyViolationError as error:
            assert error.constraint == "fk-cake-bakery_id"
        else:
            raise AssertionError("a cake was written without its bakery")


asyncio.run(main())
```

Generated statements describe a table as it is when they are generated, so a
migration history freezes them into migration files when a change is made,
rather than generating them again as each migration runs:
`pgorm.schema.migration_file(name, statements)` does that, and
[Generated DDL in a migration](22-migrations.md#generated-ddl-in-a-migration)
shows it.

## Enum types

A registered entity's `enums` come from its columns: one `CREATE TYPE .. AS
ENUM` for each enum a column holds, an array of an enum included, each
written once however many of its columns use it.

Rust's `create_enum_from_active_enum` writes one `ActiveEnum`'s type
directly. Python has no `ActiveEnum`: a model's enum column names its type
and lists its labels (`Column(TypeName("flavor", schema="public"),
values=FLAVORS)`), and the type is written with `create_type`, given the
same labels, as `bakery.py` writes `flavor`. Labels are data: an empty label
and one holding a quote are written as escaped literals. A label longer than
PostgreSQL's 63 bytes, or holding a NUL, cannot be stored, and is refused as
the statement is built:

```python
from pgorm import ConstructionError
from pgorm import schema as s

from bakery import FLAVORS

flavor = s.create_type("flavor").as_enum().values(FLAVORS)
assert flavor.inspect().sql == (
    """CREATE TYPE "flavor" AS ENUM ('chocolate', 'vanilla', 'lemon', 'carrot')"""
)

shelf = s.create_type("shelf").values(["top", "", "O'Brien's"])
assert shelf.inspect().sql == """CREATE TYPE "shelf" AS ENUM ('top', '', E'O\\'Brien\\'s')"""

try:
    s.create_type("shelf").values(["x" * 64])
except ConstructionError:
    pass
else:
    raise AssertionError("a 64-byte label was taken")
```

PostgreSQL has no `CREATE TYPE IF NOT EXISTS`, so creating a type twice is
an error; run each enum's statement once. Adding a value later is
`ALTER TYPE .. ADD VALUE`, which [`alter_type`](#types) writes.

## Indexes and comments

A Rust entity's column marked `indexed` gets an index of its own, and a
`comment` on its `Model` or a field becomes a `COMMENT ON`. A registered
entity's `indexes` and `comments` hold them. Both are statements apart from
the `CREATE TABLE`, and run after it.

A model declared in Python has no `indexed` or `comment` option, so it
generates neither; that is not available in Python yet. `create_index`,
`comment_on_table` and `comment_on_column` write the same statements, and an
index can take the name generation would give it. Here the bakery records how
each cake is made:

```python
import asyncio
import os

from pgorm import Pool, RawSQL
from pgorm import schema as s

recipe = (
    s.create_table("recipe")
    .column(s.ColumnDef("id", "integer").not_null().auto_increment())
    .column(s.ColumnDef("cake_id", "integer").not_null())
    .column(s.ColumnDef("bake_minutes", "integer").not_null())
    .primary_key("id")
)
index = s.create_index("recipe", "cake_id", name="idx-recipe-cake_id")
comments = [
    s.comment_on_table("recipe", "How each cake is made"),
    s.comment_on_column("recipe", "bake_minutes", "Minutes in the oven"),
]
assert index.inspect().sql == 'CREATE INDEX "idx-recipe-cake_id" ON "recipe" ("cake_id")'
assert [comment.inspect().sql for comment in comments] == [
    """COMMENT ON TABLE "recipe" IS 'How each cake is made'""",
    """COMMENT ON COLUMN "recipe"."bake_minutes" IS 'Minutes in the oven'""",
]


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        for statement in [recipe, index, *comments]:
            await pool.execute(statement)
        row = await pool.one(RawSQL(
            "SELECT obj_description('recipe'::regclass, 'pg_class') AS comment"
        ))
        assert row["comment"] == "How each cake is made"


asyncio.run(main())
```

An index over several columns, a unique index, a partial index or one of
another method is the [index builder](#indexes)'s. A unique key over several
columns is part of the `CREATE TABLE`, written by its `unique`; a model
declares the same key with `unique_keys`, which [Models and
entities](03-entities.md) shows.

## The DDL builders

`pgorm.schema` has a builder for each DDL statement PostgreSQL has a use for
here: tables, indexes, foreign keys, types, domains, sequences, views,
materialized views, schemas, extensions and comments. They are Rust's
pgorm-query builders, as the [query builder](11-query-builder.md)'s
statements are. Each statement is made by its function: `create_table`,
`alter_table`, `create_index`, `create_type`, `create_domain`,
`create_sequence`, `create_view` and the rest. Their classes, `CreateTable`,
`CreateIndex` and so on, are exported for annotations and `isinstance`, and
calling one is a `TypeError`. Every complete statement is a `DDL`, and every
`execute` takes one.

A builder's methods return a new statement and leave the one they were
called on as it was, so a statement can be shared and extended. An `ALTER`
that names its object and no action yet, such as `alter_table("cake")`, is
not a statement: it has no `inspect()`, `execute` refuses it, and its first
action gives the statement.

Here Python differs from Rust. Rust names a table or a column with a typed
identifier, an entity, its `Column`, or an enum deriving `SqlName`. In
Python a name is a string. A table, a view or a sequence is a `str` or a
`pgorm.Table`, whose `schema=` qualifies it; a type or a domain a `str` or a
`pgorm.TypeName`. Where a statement takes several names, such as a key's
columns or the objects a drop drops, it takes one name or a list of them.
Every name is quoted as it is written, so no name can break out of its
place:

```python
from pgorm import ConstructionError, Table
from pgorm import schema as s

drop = s.drop_table(["delivery", Table("courier_run", schema="archive")], if_exists=True)
assert drop.inspect().sql == 'DROP TABLE IF EXISTS "delivery", "archive"."courier_run"'
assert drop.inspect().params == []

odd = s.create_table('Odd "name"').column(s.ColumnDef("due at", "date"))
assert odd.inspect().sql == 'CREATE TABLE "Odd ""name""" ( "due at" date )'

try:
    s.drop_table([])
except ConstructionError:
    pass
else:
    raise AssertionError("a drop of nothing was built")
```

A choice from a closed set is a string, spelled in snake_case:
`behavior="cascade"`, `on_delete="set_null"`,
`deferrability="deferrable_initially_deferred"`. An argument of the wrong
type is a `TypeError`, a string outside its set among them. A value that
cannot become what pgorm writes, such as an empty list of names, an aliased
`Table` or an integer outside `bigint`, is a `ConstructionError`. Like the
query builder's statements, a statement refuses to be tested as a Python
boolean, so `if statement:` fails where it is written.

PostgreSQL takes no parameters in DDL, so a statement writes its values,
such as a default or a check's constant, inline as escaped literals, and
`inspect().params` is always empty. Python never pastes a value or a name
into DDL text itself: Rust's renderer writes each one.
`pgorm.capabilities()["schema_policy"]` records these rules as data, and its
`unsupported` list names what is not built: schema introspection, automatic
migrations, raw column options and concurrent indexes.

Rust has two `Schema`s: `pgorm::Schema` generates DDL from entities, and
`pgorm_query::Schema` builds `CREATE SCHEMA`. In Python they are
`from_entity` and `create_schema`, side by side in `pgorm.schema`.

### Tables

`create_table` takes the table's name, and then its columns, keys, foreign
keys and checks. `ColumnDef(name, kind)` describes a column: its type, then
`not_null`, `default`, `identity`, `generated` and the rest, its clauses
written in the order they are added.

A column's type is a built-in name such as `"integer"`, `"text"` or
`"timestamptz"` (`pgorm.capabilities()["schema_policy"]["column_types"]`
lists them); a `DataType`, for a length, a precision or an array
(`DataType("numeric", precision=10, scale=2)`, `DataType("text").array()`);
a `TypeName` naming a type of the schema's own, such as an enum, a domain or
a composite; or a `CreatedRange` or `CreatedMultirange` for a range type a
schema created ([Values and types](13-values.md)). A type a column can be
created with is not always one a record can decode:
`capabilities()["result_policy"]` says which are. Here the bakery starts
delivering orders:

```python
import asyncio
import os

from pgorm import CheckViolationError, Pool, RawSQL, col
from pgorm import schema as s

create = (
    s.create_table("delivery")
    .column(s.ColumnDef("id", "integer").not_null().identity("always"))
    .column(s.ColumnDef("order_id", "integer").not_null())
    .column(s.ColumnDef("address", "text").not_null())
    .column(s.ColumnDef("due_at", "timestamptz").not_null())
    .column(s.ColumnDef("fragile", "boolean").not_null().default(False))
    .primary_key("id")
    .foreign_key("order_id", "order", "id", on_delete="cascade")
    .check(col("address").ne(""))
)
assert create.inspect().sql == " ".join([
    'CREATE TABLE "delivery" (',
    '"id" integer NOT NULL GENERATED ALWAYS AS IDENTITY,',
    '"order_id" integer NOT NULL,',
    '"address" text NOT NULL,',
    '"due_at" timestamp with time zone NOT NULL,',
    '"fragile" bool NOT NULL DEFAULT FALSE,',
    'PRIMARY KEY ("id"),',
    'FOREIGN KEY ("order_id") REFERENCES "order" ("id") ON DELETE CASCADE,',
    """CHECK ("address" <> '')""",
    ')',
])

# A column added later, and the table renamed.
alter = s.alter_table("delivery").add_column(s.ColumnDef("notes", "text"))
assert alter.inspect().sql == 'ALTER TABLE "delivery" ADD COLUMN "notes" text'
rename = s.rename_table("delivery", "courier_run")
assert rename.inspect().sql == 'ALTER TABLE "delivery" RENAME TO "courier_run"'
drop = s.drop_table("courier_run", if_exists=True)
assert drop.inspect().sql == 'DROP TABLE IF EXISTS "courier_run"'


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await pool.execute(create)
        # The check holds: a delivery needs an address.
        try:
            await pool.execute(RawSQL(
                'INSERT INTO "delivery" ("order_id", "address", "due_at") VALUES ($1, $2, now())',
                [1, ""],
            ))
        except CheckViolationError:
            pass
        else:
            raise AssertionError("a delivery was written without an address")
        await pool.execute(alter)
        await pool.execute(rename)
        await pool.execute(drop)


asyncio.run(main())
```

`identity("always")` or `identity("by_default")` is `GENERATED .. AS
IDENTITY`, the form PostgreSQL recommends over `auto_increment()`'s
`serial`. It takes a sequence's options: `start_with`, `increment_by`,
`min_value`, `max_value`, `cache` and `cycle`, a bound given as `None` being
its `NO` form. A generated column always names its kind,
`.generated(expression, "stored")` or `.generated(expression, "virtual")`,
since PostgreSQL 17 requires `STORED` and 18 reads a bare one as `VIRTUAL`.
Write the columns inside a check or a generated expression unqualified, as
`col("address")`.

`not_null(name=None, no_inherit=False)` is the column's one `NOT NULL`,
which PostgreSQL 18 records as a constraint by name, the one given or one it
derives, and which a table inheriting this one takes unless `no_inherit` is
set. `check(condition, name=None, enforcement=None, no_inherit=False)` adds
a `CHECK`; `enforcement="not_enforced"` is PostgreSQL 18's `NOT ENFORCED`, a
check recorded and never checked. `collate(name, schema=None)` is the
column's collation, written after its type: `ColumnDef("code",
"text").collate("C")` compares and sorts the column bytewise. The server
refuses a collation that does not exist, and one on a type without
collations.

The table's keys are its own. `primary_key(columns)` declares its one
primary key, which a later call replaces, and `unique(columns,
nulls_not_distinct=False)` adds a unique key. Either takes `name=`,
`include=[..]` for columns its index carries, `deferrability=` and
`without_overlaps=column`, PostgreSQL 18's temporal key: the column, a range
or a multirange, is written last as `column WITHOUT OVERLAPS`, and two rows
may then share the other columns only while their periods do not overlap.
Its index needs the [`btree_gist`](#extensions-and-comments) extension for
the other columns. `foreign_key(columns, references, ref_columns)` pairs its
columns in order with as many of the referenced table's, and takes `name=`,
`on_delete=` and `on_update=` (`"no_action"`, `"restrict"`, `"cascade"`,
`"set_null"`, `"set_default"`), `deferrability=`, `enforcement=` and
PostgreSQL 18's `period=(column, referenced_column)`, written last on both
sides. `CreateTable` also takes `if_not_exists()`.

`alter_table(table)` takes one action or several, each written in order in
one `ALTER TABLE`: `add_column`, `modify_column`, `drop_column`,
`add_primary_key`, `add_unique` and `add_foreign_key` (with the options the
`CREATE TABLE` keys take), `add_check`, `add_not_null`, `drop_constraint`,
which drops a key, a foreign key, a check or a `NOT NULL` by name,
`validate_constraint`, `alter_constraint`, `set_expression` and
`drop_expression`. A foreign key, a check and a `NOT NULL` added with
`not_valid=True` leave the rows already there unchecked until
`validate_constraint` checks them, while new rows are held to them at once:

```python
import asyncio
import os

from pgorm import CheckViolationError, Pool, col
from pgorm import schema as s

positive = s.alter_table("cake").add_check(col("price") > 0, name="ck-cake-price", not_valid=True)
assert positive.inspect().sql == (
    'ALTER TABLE "cake" ADD CONSTRAINT "ck-cake-price" CHECK ("price" > 0) NOT VALID'
)
validate = s.alter_table("cake").validate_constraint("ck-cake-price")
assert validate.inspect().sql == 'ALTER TABLE "cake" VALIDATE CONSTRAINT "ck-cake-price"'

contact = (
    s.alter_table("customer")
    .add_column(s.ColumnDef("phone", "text"))
    .modify_column(s.ColumnDef("name", "text"))
)
assert contact.inspect().sql == (
    'ALTER TABLE "customer" ADD COLUMN "phone" text, ALTER COLUMN "name" TYPE text'
)


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await pool.execute(positive)
        await pool.execute(validate)
        try:
            dear = s.alter_table("cake").add_check(col("price") > 5, name="ck-cake-dear")
            await pool.execute(dear)
        except CheckViolationError as error:
            assert error.constraint == "ck-cake-dear"
        else:
            raise AssertionError("a check four cakes fail was added")
        await pool.execute(contact)


asyncio.run(main())
```

`modify_column(column)` writes each part of the column it is given: a new
type, with its collation, `SET` or `DROP NOT NULL`, a named `NOT NULL`,
`SET DEFAULT`, a check or an identity added. What no such action writes is
refused rather than dropped: a generated expression, which `set_expression`
changes, the rows already written taking the new one, and `drop_expression`
removes, leaving a stored column plain with its values; the `serial` family;
a collation without its type; and a column that changes nothing. A
`ColumnDef` given to `modify_column` may leave out its type, to change the
rest alone. `no_inherit=True` keeps a check or a `NOT NULL` from tables that
inherit this one, which a partitioned table refuses. `alter_constraint(name,
"inherit")` or `"no_inherit"` changes whether a `NOT NULL` passes to
inheriting tables, and `"enforced"` or `"not_enforced"` whether a foreign
key, or on PostgreSQL 19 a check, is enforced; enforcing one checks the rows
already there.

Beside it, `rename_column(table, name, new_name)`,
`rename_constraint(table, name, new_name)`, which renames a key's index with
it, `truncate_table(table)` and `drop_table(tables, if_exists=False,
behavior=None)` each give a statement. Database validation and privileges
still apply.

### Foreign keys and their timing

A foreign key is checked at the end of each statement unless it says
otherwise. `deferrability="deferrable_initially_deferred"` moves the check
to commit, which lets a transaction write rows in an order no single
statement could, such as two rows that point at each other. PostgreSQL 18
also takes `enforcement="not_enforced"`: a key that is recorded, for the
people and tools reading the schema, but never checked.

A Rust entity's relation carries the same two settings, as `deferrability =
..` and `enforcement = ..`, and `from_entity` writes them. A key added to an
existing table is `alter_table(..).add_foreign_key(..)`:

```python
import asyncio
import os
from datetime import date

from pgorm import Pool, RawSQL
from pgorm import schema as s

from bakery import Baker

# Each bakery names its head baker, and each baker their bakery.
column = s.alter_table("bakery").add_column(s.ColumnDef("head_baker_id", "integer"))
head_baker = s.alter_table("bakery").add_foreign_key(
    "head_baker_id", "baker", "id", deferrability="deferrable_initially_deferred"
)
assert head_baker.inspect().sql == (
    'ALTER TABLE "bakery" ADD FOREIGN KEY ("head_baker_id") REFERENCES "baker" ("id") '
    "DEFERRABLE INITIALLY DEFERRED"
)


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await pool.execute(column)
        await pool.execute(head_baker)

        # Point the bakery at a baker who does not exist yet: the key is
        # checked at commit, by which time the baker does.
        async with pool.transaction() as tx:
            await tx.execute(RawSQL('UPDATE "bakery" SET "head_baker_id" = 4 WHERE "id" = 1'))
            await Baker.insert({
                "name": "Di", "hired_on": date(2026, 11, 1), "bakery_id": 1,
            }).execute(tx)

        row = await pool.one(RawSQL('SELECT "head_baker_id" FROM "bakery" WHERE "id" = 1'))
        assert row["head_baker_id"] == 4


asyncio.run(main())
```

### Indexes

`create_index(table, on, name=None)` is an index over its first entry, a
column's name or an expression; `column(on)` adds another. Each entry takes
`order="asc"` or `"desc"` and an `operator_class`. `unique()`,
`nulls_not_distinct()`, which PostgreSQL defines for a unique index alone
and so makes the index unique, `include([..])`, a covering index's extra
columns, `using(method)`, any access method the server has, such as `gin`
or `hash`, `where(predicate)`, a partial index's predicate, and
`if_not_exists()` say what kind of index it is:

```python
import asyncio
import os

from pgorm import Pool, RawSQL, col
from pgorm import schema as s

# One cake of each name per bakery.
unique = s.create_index("cake", "bakery_id", name="uq-cake-bakery-name").column("name").unique()
assert unique.inspect().sql == (
    'CREATE UNIQUE INDEX "uq-cake-bakery-name" ON "cake" ("bakery_id", "name")'
)

# Only the gluten-free cakes, which the shop's filter asks for most.
partial = (
    s.create_index("cake", "price", name="idx-cake-gluten-free-price")
    .where(col("gluten_free").eq(True))
)
assert partial.inspect().sql == (
    'CREATE INDEX "idx-cake-gluten-free-price" ON "cake" ("price") WHERE "gluten_free" = TRUE'
)

drop = s.drop_index("cake", "idx-cake-gluten-free-price")
assert drop.inspect().sql == 'DROP INDEX "idx-cake-gluten-free-price"'


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        for statement in (unique, partial, drop):
            await pool.execute(statement)
        rows = await pool.query(RawSQL(
            "SELECT indexname FROM pg_indexes WHERE tablename = 'cake' ORDER BY indexname"
        ))
        assert [row["indexname"] for row in rows] == ["cake_pkey", "uq-cake-bakery-name"]


asyncio.run(main())
```

An expression is indexed as written, `create_index("customer",
call("lower", col("email")), name="idx-customer-email")`. `CONCURRENTLY` is
not offered, since PostgreSQL refuses it inside a transaction.
`drop_index(table, name, if_exists=False)` qualifies the index's name with
its table's schema.

### Types

`create_type` creates an enum (`as_enum` and `values`), a composite
(`as_composite` and `attribute`), or a range (`as_range`, which [Values and
types](13-values.md) uses). What a type is, is one choice: `values` makes it
an enum and `attribute` a composite without saying so first. `alter_type`
adds an enum value, before or after another, renames one, or renames the
type:

```python
import asyncio
import os

from pgorm import Pool, RawSQL
from pgorm import schema as s

box_size = s.create_type("box_size").as_enum().values(["small", "large"])
assert box_size.inspect().sql == """CREATE TYPE "box_size" AS ENUM ('small', 'large')"""

medium = s.alter_type("box_size").add_value("medium", before="large")
assert medium.inspect().sql == """ALTER TYPE "box_size" ADD VALUE 'medium' BEFORE 'large'"""

address = (
    s.create_type("address").as_composite().attribute("street", "text").attribute("city", "text")
)
assert address.inspect().sql == 'CREATE TYPE "address" AS ("street" text, "city" text)'


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        for statement in (box_size, medium, address):
            await pool.execute(statement)
        row = await pool.one(RawSQL("SELECT enum_range(NULL::box_size)::text AS labels"))
        assert row["labels"] == "{small,medium,large}"


asyncio.run(main())
```

`alter_type(name)` names the type, and each of its actions gives the
statement: `add_value(label, before=None, after=None)`, `rename_to(name)`,
`rename_value(label, new_label)` and `rename_attribute(name, new_name)`, and
for a composite `add_attribute`, `drop_attribute(name, if_exists=False)` and
`alter_attribute`, which take more of their kind. A composite's change, and
an attribute renamed, take `behavior("cascade")` to carry it into tables
typed by the composite. `as_range(subtype, collation=None,
subtype_opclass=None, subtype_diff=None, multirange_type_name=None)` writes
a range's options, and `drop_type(names, if_exists=False, behavior=None)`
drops types. A collation is a name or `{"name": .., "schema": ..}`.

A value added with `ADD VALUE` cannot be used in the transaction that adds
it, so commit the change before a statement uses the value
([Migrations](22-migrations.md)).

Here Python differs from Rust. A connection remembers the types it has read,
and a model reads an enum column as the enum, not as its text as Rust's
entities do. After a label of an existing enum is renamed, or a type is
renamed, a connection that read the type before reads it no more: its rows
are a `DecodeError`, and clearing the [statement caches](19-statement-caches.md)
does not help. Close the program's pools and open new ones before reading the
type again. Returning a connection to its pool does not refresh what it
remembers.

### Domains, sequences and schemas

`create_domain(name, base)` makes a domain, a type over a base type with a
default, a `NOT NULL` and checks of its own; a column of the domain names it
as a `TypeName`, as a column of an enum does. `create_sequence` makes a
counter of its own, beyond the ones an identity column keeps.
`create_schema` makes a schema, the namespace a `Table(.., schema=..)`, a
`TypeName` or a sequence is qualified by:

```python
import asyncio
import os

from pgorm import Pool, RawSQL, Table, domain_value
from pgorm import schema as s

archive = s.create_schema("archive", if_not_exists=True)
assert archive.inspect().sql == 'CREATE SCHEMA IF NOT EXISTS "archive"'

receipt_no = (
    s.create_sequence(Table("receipt_no", schema="archive"))
    .as_type("bigint")
    .options(start_with=1000)
)
assert receipt_no.inspect().sql == 'CREATE SEQUENCE "archive"."receipt_no" AS bigint START WITH 1000'

percent = s.create_domain("percent", "smallint").check(domain_value().between(0, 100))
assert percent.inspect().sql == 'CREATE DOMAIN "percent" AS smallint CHECK (VALUE BETWEEN 0 AND 100)'


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        for statement in (archive, receipt_no, percent):
            await pool.execute(statement)
        row = await pool.one(RawSQL("SELECT nextval('archive.receipt_no') AS next"))
        assert row["next"] == 1000


asyncio.run(main())
```

A domain's check is an expression over `domain_value()`, the keyword
`VALUE`, which PostgreSQL reads as the value tested. `create_domain` also
takes `collate(name, schema=None)`, for a base type with collations,
`default(value)`, the pair `not_null()` and `null()`, each the later call
winning, and `check(condition, name=None)`, each check added in order.
`alter_domain(name)` names the domain, and each action gives the whole
statement, PostgreSQL taking one action per `ALTER DOMAIN`:
`set_default(value)`, `drop_default()`, `set_not_null()`, `drop_not_null()`,
`add_check(condition, name=None, not_valid=False)`, `drop_constraint(name,
if_exists=False, behavior=None)`, `rename_constraint(name, new_name)`,
`validate_constraint(name)` and `rename_to(name)`. `drop_domain(names,
if_exists=False, behavior=None)` drops domains; a domain a column is typed
with is refused unless `behavior="cascade"`, which drops the column too.

A value outside a domain's check or a null under its `NOT NULL` is refused
whether it is written to a column of the domain or already there when
`add_check`, `set_not_null` or `validate_constraint` runs, and
`not_valid=True` leaves the values already there for `validate_constraint`.
An insert a connection prepared before `set_default` or `drop_default` keeps
the default it was planned with, since PostgreSQL's plan cache does not track
a domain's default: call `pool.clear_statement_caches()` after the change
([Statement caches and DDL](19-statement-caches.md)).

`create_sequence(name)` takes `if_not_exists()`, `as_type("smallint" |
"integer" | "bigint")`, `options(..)`, the identity column's options, each
call merged into those set, and `owned_by(table, column)`, or
`owned_by(None)` for `OWNED BY NONE`. `alter_sequence(name)` names the
sequence, and its first clause, one of those three or `restart(value=None)`,
gives a statement that takes more, and `if_exists()`.
`rename_sequence(name, new_name)` and `drop_sequence(names, if_exists=False,
behavior=None)` give statements.

A schema is named by one identifier, never qualified itself.
`create_schema(name, if_not_exists=False)` refuses a schema that exists
unless `if_not_exists`, and PostgreSQL keeps names starting `pg_` for
itself. `drop_schema(names, if_exists=False, behavior=None)` refuses a schema
that holds anything unless `behavior="cascade"`, which drops what it holds as
well. `rename_schema(name, new_name)` renames one, its contents moving with
it. `AUTHORIZATION` and `OWNER TO`, which name a role, are not built.

### Views and tables from a query

A view is a named query. `create_view` takes its name and the query, a
`Select` built with the [query builder](11-query-builder.md);
`create_materialized_view` also stores the query's rows until it is
refreshed, and `create_table_as` makes an ordinary table from them once.
PostgreSQL stores a view's definition as SQL text, so the query's values are
written into it as literals. Each refuses a query PostgreSQL would not take
in that place, such as a `limit_with_ties` without an `order_by`, with a
`ConstructionError` as it is built:

```python
import asyncio
import os

from pgorm import Pool, RawSQL, Select, Table, call, col
from pgorm import schema as s

gluten_free = s.create_view(
    "gluten_free_cake",
    Select(col("id"), col("name")).from_(Table("cake")).where(col("gluten_free").eq(True)),
).or_replace()
assert gluten_free.inspect().sql == (
    'CREATE OR REPLACE VIEW "gluten_free_cake" AS '
    'SELECT "id", "name" FROM "cake" WHERE "gluten_free" = TRUE'
)

sold = s.create_materialized_view(
    "cakes_sold",
    Select(col("cake_id"), call("sum", col("quantity")))
    .from_(Table("order_line"))
    .group_by(col("cake_id")),
).columns(["cake_id", "sold"])
refresh = s.refresh_materialized_view("cakes_sold")


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        for statement in (gluten_free, sold, refresh):
            await pool.execute(statement)
        fudge = await pool.one(RawSQL("SELECT sold FROM cakes_sold WHERE cake_id = 1"))
        assert fudge["sold"] == 2
        count = await pool.one(RawSQL("SELECT count(*) AS n FROM gluten_free_cake"))
        assert count["n"] == 3


asyncio.run(main())
```

`columns([..])` names the leading columns the query returns. A view also
takes `or_replace()`, which may add columns after the view's own and
replaces its options, `check_option("local")` or `"cascaded"`,
`security_barrier()` and `security_invoker()`. Rows written through an
automatically updatable view reach its table, and with a check option a row
the view would not show is refused, `"cascaded"` consulting the views
beneath it too. `security_barrier` evaluates the view's conditions before a
caller's, and `security_invoker` reads the tables beneath with the querying
role's privileges.

`create_recursive_view(name, columns, query)` names its columns and takes a
view's methods but `check_option`, which PostgreSQL refuses there. Its query
is a starting term joined by `union` or `union_all` to a term that reads the
view by its bare name, as PostgreSQL defines it as a `WITH RECURSIVE` query
of that name. `drop_view(names, if_exists=False, behavior=None)` drops views.

`create_materialized_view` and `create_table_as` each take
`if_not_exists()`, `columns([..])` and `with_no_data()`, which leaves the
rows out until a refresh. `refresh_materialized_view(name,
concurrently=False, with_no_data=False)` replaces a materialized view's
rows; a concurrent refresh needs a unique index on the view and rows in it,
and cannot be combined with `with_no_data`. `drop_materialized_view` drops
them. Neither has `or_replace`, and a view has no `if_not_exists`.

### Extensions and comments

`create_extension` installs an extension the server has available, such as
`btree_gist`, which a temporal key over a scalar column needs.
`comment_on_table` and `comment_on_column` write `COMMENT ON` for any table:

```python
import asyncio
import os

from pgorm import Pool, RawSQL
from pgorm import schema as s

btree_gist = s.create_extension("btree_gist", if_not_exists=True)
assert btree_gist.inspect().sql == 'CREATE EXTENSION IF NOT EXISTS "btree_gist"'

note = s.comment_on_column("cake", "price", "In kroner, VAT included")
assert note.inspect().sql == """COMMENT ON COLUMN "cake"."price" IS 'In kroner, VAT included'"""


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await pool.execute(btree_gist)
        await pool.execute(note)
        row = await pool.one(RawSQL(
            "SELECT col_description('cake'::regclass, 4) AS comment"
        ))
        assert row["comment"] == "In kroner, VAT included"


asyncio.run(main())
```

`create_extension(name, if_not_exists=False, schema=None, version=None,
cascade=False)` writes the version as a literal, and `drop_extension(name,
if_exists=False, behavior=None)` drops one.
