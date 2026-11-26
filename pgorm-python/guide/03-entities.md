# Models and entities

A *model* is a table, described as a Python value. It holds the table's name
and one `Column` per field, and everything that reads or writes the table
starts from it.

Here Python differs from Rust. A Rust *entity* is a module of types that
derives write at compile time. A Python model is built when the program
runs, by calling `pgorm.Model`: nothing is compiled or generated, and a
model can be declared, inspected and used without a database. A model
describes a table that already exists. Declaring one sends nothing and
creates nothing, and it runs no Rust derive: the hooks an entity's active
model may have are, for a model, functions of the program's own
([Hooks](#hooks)).

A Python program can also use the entities a Rust crate declares, by
registering them into its own wheel. [Registering Rust
entities](24-registrations.md) explains how. This guide calls those
*registered entities*, and says where they differ from declared models.

This chapter declares the bakery's models and goes through what each part
of a declaration can say. The other chapters use these models.

## Declaring a model

Here is the bakery's `cake` table, as [`bakery.py`](bakery.py) declares it:

```python
from pgorm import Column, Model, TypeName
from pgorm import schema as s

from bakery import Bakery, CakeBaker, OrderLine

FLAVOR = TypeName("flavor", schema="public")
FLAVORS = ["chocolate", "vanilla", "lemon", "carrot"]

Cake = Model("cake", {
    "id": Column("i32", primary_key=True, auto_increment=True),
    "name": Column("text", column_type="varchar"),
    "flavor": Column(FLAVOR, values=FLAVORS),
    "price": Column("decimal", column_type=s.DataType("numeric", precision=10, scale=2)),
    "gluten_free": Column("bool"),
    "bakery_id": Column("i32"),
})

# Each relation is named for where it starts and what it reaches.
cake_bakery = Cake.belongs_to(Bakery, from_="bakery_id", to="id")
cake_order_lines = Cake.has_many(OrderLine, from_="id", to="cake_id")
cake_bakers = Cake.has_many(CakeBaker, from_="id", to="cake_id")

assert Cake.table.name == "cake"
assert list(Cake.columns) == ["id", "name", "flavor", "price", "gluten_free", "bakery_id"]
assert Cake.primary_key == ("id",)
```

`Model` takes the table's name and a mapping from each field's name to its
`Column`. The model then stands for the table in every statement:

- The model itself plays the part of Rust's `Entity`. Queries start from it:
  `Cake.find()`.
- `Cake.col("gluten_free")` names a column, where Rust writes
  `Column::GlutenFree`. Filters, orderings and projections name columns
  through it. The field is checked against the declaration, so a misspelt
  one is a `ConstructionError` as the query is built.
- `Cake.primary_key` lists the key's fields, and `Cake.key({"id": 1})` is the
  condition that matches one key, where Rust has the `PrimaryKey` enum.
- A write takes a mapping of the fields it sets, which plays the part of
  Rust's `ActiveModel`: a key present sets its column, a key absent leaves
  the column out ([Writing rows](04-writing.md)).
- A row the model reads is a `ModelRecord`, a read-only mapping keyed by
  field, which plays the part of Rust's `Model`.

Relations are values too. `belongs_to`, `has_one` and `has_many` on a model
each make one, and the program keeps it in a variable; a query joins along
it and a graph decodes along it ([Relations](07-relations.md)). The bakery
has seven models, `Bakery`, `Baker`, `Cake`, `CakeBaker`, `Customer`, `Order`
and `OrderLine`, declared the same way, and the relations between them.
Their full source is in [`bakery.py`](bakery.py), which the examples in
this guide import.

A declaration is checked as it is made. An option `Column` does not have is
Python's own `TypeError`, and so is an option of the wrong type, such as a
`nullable` that is not a `bool`. An option that contradicts another, such as
a nullable primary key, is a `ConstructionError`, and so are two fields
naming one SQL column. A model copies the mapping it is given and cannot be
changed afterwards, so changing the mapping later changes nothing.

## Column kinds

Each `Column` starts with its *kind*: what the column's values are, which
is how pgorm decodes the column and converts the values written to it.

| Kind | Column type |
| --- | --- |
| `"text"` | `text`, `varchar`, `char(n)` or `name` |
| `"i16"`, `"i32"`, `"i64"` | `smallint`, `integer`, `bigint` |
| `"f32"`, `"f64"` | `real`, `double precision` |
| `"bool"` | `boolean` |
| `"decimal"` | `decimal`, which PostgreSQL also spells `numeric` |
| `"date"`, `"time"`, `"datetime"` | `date`, `time`, `timestamp` |
| `"datetime_utc"` | `timestamptz` |
| `"interval"` | `interval` |
| `"uuid"` | `uuid` |
| `"json"` | `json` or `jsonb` |
| `"bytes"` | `bytea` |
| `"ipnetwork"`, `"mac_address"` | `inet` or `cidr`, `macaddr` |
| `"vector"` | `vector`, from the pgvector extension |
| `"i8"`, `"u32"` | `"char"`, `oid` |
| `"int4range"` to `"tstzrange"`, `"int4multirange"` to `"tstzmultirange"` | the built-in range and multirange types |
| a `TypeName`, with `values` | an enum ([below](#enums)) |
| a `CreatedRange` or a `CreatedMultirange` | a range type the schema creates |

`pgorm.capabilities()["model_policy"]["scalar_kinds"]` lists the kind names
the installed package accepts.

Two options qualify the kind. `nullable=True` lets the column hold SQL
NULL, read as `None`, where Rust writes `Option<T>`. `array=True` makes it a
one-dimensional array of the kind, `text[]` for `"text"`, where Rust writes
`Vec<T>`; `nullable` is about the array itself, and its elements may be
`None` either way. [Values and types](13-values.md) says which Python value
each kind reads and writes, and how `pgorm.Value(data, kind)` converts a
value to a kind, as a model does with every value it writes.

Here Python differs from Rust. A kind is what a column decodes as, and a
declaration must give the kind the column really decodes as: an `integer`
column is `"i32"`, and declaring it `"i64"` is a `DecodeError` when it is
read, even though every value would fit. So is a NULL read from a field not
declared `nullable`: a row that does not match its declaration is an error,
never a row read as missing. A `timestamptz` is `"datetime_utc"` and reads as
a UTC `datetime`. A kind the decoder never produces, such as
`"u64"` or `"char"`, is refused with `UnsupportedCapabilityError` as the
column is declared. A kind is not a column type: a `varchar` and a `text`
column are both `"text"`, and the cake's `decimal(10, 2)` price is
`"decimal"`. The type matters only to the table the model writes
([below](#creating-the-table)), which uses the kind's own type unless
`column_type` names another, as Rust's `column_type` does:
`column_type="varchar"`, or a `pgorm.schema.DataType` with a length or a
precision. An array column's `column_type` is its elements' type.

A column is named for its field. `name="..."` names it otherwise, so a
field can stand for a column whose name is not a Python identifier:
`Column("text", name="display name")`. Table, schema and column names, and
the fields' own, may hold any character but NUL, from 1 to 63 bytes, and the
SQL quotes them as pgorm quotes every identifier. `rename_all="..."` on the
`Model` names every column a field does not name itself by a case style, as
Rust's `rename_all` does and by the same conversion: `"camelCase"`,
`"PascalCase"`, `"kebab-case"`, `"snake_case"`, `"SCREAMING_SNAKE_CASE"`,
`"SCREAMING-KEBAB-CASE"`, `"lowercase"`, `"UPPERCASE"`, `"title_case"` or
`"mixed_case"`; any other is a `TypeError`. The record keeps the field's own
name.

## Creating the table

A model also says how its table is created, as a Rust entity does.
`pgorm.schema.from_model(model)` hands the declaration to Rust's schema
generation, which writes the table from it exactly as it writes an
entity's, and returns the same `EntitySchema` a [registered
entity](14-schema.md#tables-from-models-and-entities)'s has: the `CREATE TABLE`, the
`CREATE TYPE` of each enum its columns use, an index for each `indexed`
column and its comments. [Schema generation](14-schema.md) covers it, and
these examples use it to show what a declaration means. The bakery's
customers are unique by email:

```python
from pgorm import Column, Model
from pgorm import schema as s

Customer = Model("customer", {
    "id": Column("i32", primary_key=True, auto_increment=True),
    "name": Column("text", column_type="varchar"),
    "email": Column("text", column_type="varchar"),
    "notes": Column("text", nullable=True),
}, unique_keys=[{"fields": ("email",)}])

create = s.from_model(Customer).table
assert create.inspect().sql == (
    'CREATE TABLE "customer" ( "id" serial NOT NULL, "name" varchar NOT NULL, '
    '"email" varchar NOT NULL, "notes" text, PRIMARY KEY ("id"), UNIQUE ("email") )'
)
assert Customer.describe()["unique_keys"] == [
    {"name": None, "fields": ["email"], "nulls_not_distinct": False}
]
```

The statement is the one the Rust guide's `customer` entity writes. Declaring
a model still sends nothing: `from_model` only builds statements, which the
program runs with `execute`.

## Primary keys

A field marked `primary_key=True` is part of the table's primary key, and a
key of several columns is a field marked so for each. A key column is never
nullable: declaring one `nullable` is a `ConstructionError`.

Here Python differs from Rust. A Rust entity must have a primary key, and a
key of one integer column is generated by the database unless it says
`auto_increment = false`. A Python model may declare no key, as a model of a
view might, though then its `key` and `find_by_key` are refused. And nothing
about a key is assumed: the declaration says how the database fills it.
`auto_increment=True` draws a one-column `"i16"`, `"i32"` or `"i64"` key from
the serial family, as Rust's one-column integer key is by default, so its
table writes it `serial` and an insert leaves it out; the bakery's ids are
declared so. Without it, or one of the `generated` options below, or a
default, an insert must give the key, and the table writes a plain column.

The bakery's `cake_baker` table records which baker bakes which cake, keyed
by the pair:

```python
import asyncio
import os

from pgorm import Column, Model, Pool

from bakery import Baker, Cake

CakeBaker = Model("cake_baker", {
    "cake_id": Column("i32", primary_key=True),
    "baker_id": Column("i32", primary_key=True),
})
cake_baker_cake = CakeBaker.belongs_to(Cake, from_="cake_id", to="id")
cake_baker_baker = CakeBaker.belongs_to(Baker, from_="baker_id", to="id")


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # Ada bakes the Lemon Drizzle.
        found = CakeBaker.find_by_key({"cake_id": 2, "baker_id": 1})
        assert found.inspect().sql == (
            'SELECT "cake_baker"."cake_id", "cake_baker"."baker_id" FROM "cake_baker" '
            'WHERE "cake_baker"."cake_id" = $1 AND "cake_baker"."baker_id" = $2'
        )
        row = await found.one(pool)
        assert dict(row) == {"cake_id": 2, "baker_id": 1}


asyncio.run(main())
```

`find_by_key` takes the key as a mapping of every key field, where Rust's
`find_by_id` takes a tuple. A key missing a field, or naming one that is not
in the key, is a `ConstructionError`.

`generated` says the database generates a column. `generated="always"`
is a column no write sets, such as a `GENERATED ALWAYS AS IDENTITY` key, and
`generated="by_default"` is a `GENERATED BY DEFAULT AS IDENTITY` one, which an
insert may leave out or set. Rust writes these `identity` and
`identity_by_default`. A multi-tenant table keyed by `(tenant_id, id)` marks
`id` this way, and its inserts give the tenant and get the id back:

```python
import asyncio
import os

from pgorm import Column, ConstructionError, Model, Pool
from pgorm import schema as s

Note = Model("note", {
    "tenant_id": Column("i32", primary_key=True),
    "id": Column("i64", primary_key=True, generated="always"),
    "body": Column("text", column_type="varchar"),
})


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        create = s.from_model(Note).table
        assert create.inspect().sql == (
            'CREATE TABLE "note" ( "tenant_id" integer NOT NULL, '
            '"id" bigint NOT NULL GENERATED ALWAYS AS IDENTITY, "body" varchar NOT NULL, '
            'CONSTRAINT "pk-note" PRIMARY KEY ("tenant_id", "id") )'
        )
        await pool.execute(create)

        note = await Note.insert({"tenant_id": 7, "body": "Order more flour"}).returning().one(pool)
        assert (note["tenant_id"], note["id"]) == (7, 1)
        found = await Note.find_by_key({"tenant_id": 7, "id": 1}).one(pool)
        assert found["body"] == "Order more flour"

        try:
            Note.insert({"tenant_id": 7, "id": 2, "body": "Mine"})
        except ConstructionError as error:
            assert str(error) == "id is generated always: the server writes it"
        else:
            raise AssertionError("no write sets a column generated always")


asyncio.run(main())
```

A key whose last field is a range or multirange can be a PostgreSQL 18
temporal key: mark that field `primary_key=True, without_overlaps=True`, as
Rust marks it `primary_key, without_overlaps`, and no two rows with equal
other key fields may hold overlapping ranges. Its table writes the field
last, `WITHOUT OVERLAPS`. A period on any field but the key's last, or in a
key of one field, which PostgreSQL refuses, is a `ConstructionError`. A
lookup by the whole key compares the range for equality, as Rust's does, so
it finds exactly one version of the row.

## Defaults, generated columns and keys

A column can take a default, be computed from the others, carry a comment,
be indexed, or be part of a unique key, each said as Rust's attribute of the
same name says it:

- `default_value=1` gives a constant default, converted through the
  column's kind, and `default_expr=...` an expression, such as
  `call("uuidv7")`. `default=True` says the table has a default the
  declaration does not state, so an insert may leave the column out; a model
  that says so has no `DEFAULT` to write, so `from_model` refuses its table.
- `generated_stored=...` and `generated_virtual=...` make a generated column,
  computed from an expression over the row's other columns, which the
  database stores or computes on read. No write sets one, and every read
  returns it: the model reports it `generated="always"`.
- `unique_keys` on the `Model` lists the table's unique keys, as Rust's
  `unique` and `unique(columns = (..))` do. Each is a mapping: its `fields`,
  one field or several in key order, and an optional `name` and
  `nulls_not_distinct`. A key naming no field, a field the model lacks, or
  one field twice is a `ConstructionError`, and an option it does not have a
  `TypeError`.
- `indexed=True` asks for an index on the column, and `comment="..."` on a
  `Column` or on the `Model` becomes a `COMMENT ON`, both statements of
  their own in the model's schema ([Schema generation](14-schema.md)).

PostgreSQL fills a column one way, so a column given two of a default,
stated or not, an identity, a generation expression and `auto_increment` is
a `ConstructionError`. An insert must set every field that is neither
nullable, defaulted, drawn from the serial family nor generated, and one
that does not is refused with a `ConstructionError` before anything is sent.

The bakery's order lines could carry their total as a stored generated
column, and allow each cake once per order. This example makes the table in
a schema of its own, beside the bakery's own `order_line`:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Column, Model, Pool, col
from pgorm import schema as s

MONEY = s.DataType("numeric", precision=10, scale=2)

OrderLine = Model("order_line", {
    "id": Column("i32", primary_key=True, auto_increment=True),
    "order_id": Column("i32"),
    "cake_id": Column("i32"),
    "quantity": Column("i32", default_value=1),
    "price": Column("decimal", column_type=MONEY),
    "total": Column("decimal", column_type=MONEY, generated_stored=col("price") * col("quantity")),
}, schema="draft", unique_keys=[{"fields": ("order_id", "cake_id"), "name": "one_line_per_cake"}])


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        create = s.from_model(OrderLine).table
        assert create.inspect().sql == (
            'CREATE TABLE "draft"."order_line" ( "id" serial NOT NULL, '
            '"order_id" integer NOT NULL, "cake_id" integer NOT NULL, '
            '"quantity" integer NOT NULL DEFAULT 1, "price" decimal(10, 2) NOT NULL, '
            '"total" decimal(10, 2) NOT NULL GENERATED ALWAYS AS ("price" * "quantity") STORED, '
            'PRIMARY KEY ("id"), CONSTRAINT "one_line_per_cake" UNIQUE ("order_id", "cake_id") )'
        )
        await pool.execute(s.create_schema("draft"))
        await pool.execute(create)

        # The quantity is left out, so it takes its default, and the total is computed.
        insert = OrderLine.insert({"order_id": 1, "cake_id": 1, "price": Decimal("4.50")})
        assert insert.inspect().sql == (
            'INSERT INTO "draft"."order_line" ("order_id", "cake_id", "price") VALUES ($1, $2, $3)'
        )
        line = await insert.returning().one(pool)
        assert (line["quantity"], line["total"]) == (1, Decimal("4.50"))

        two = await OrderLine.insert({
            "order_id": 1, "cake_id": 2, "quantity": 2, "price": Decimal("3.20"),
        }).returning("total").one(pool)
        assert two["total"] == Decimal("6.40")


asyncio.run(main())
```

## Relations

Each relation is made by a method of the model it starts from, and names the
model it reaches and the fields that are equal in a related pair of rows:

- `belongs_to(target, from_=..., to=...)` is the side holding the foreign
  key. `from_` names this model's fields and `to` the target's, a list of
  fields for a composite key, paired in order.
- `has_one` and `has_many` are the other side: a row reaches at most one row,
  or any number, of the target.
- `on_condition` narrows a relation with an extra predicate, as Rust's does:
  a function given the relation's two ends, each the model as a join names
  its table, returning a condition. It is ANDed with the fields' equality,
  or ORed with it where `condition_type="any"`, Rust's `ConditionType::Any`.

Here Python differs from Rust. A Rust `has_many` reads its columns from the
other entity's `belongs_to`; a Python relation names its own fields on both
sides, so the bakery declares `bakery_cakes` and `cake_bakery` as two
relations, one for each direction. Rust's `rev` turns a relation around; in
Python, declare the other direction. And Rust's `on_condition` closure is
given the two tables' names; a Python one is given the two models answering
to them, so `bakery.col("city")` is qualified as the join names the bakery.
A `belongs_to` relation is also the table's foreign key:
`from_model(model, relations=[...])` writes one for each `belongs_to`
relation it is given, and the relation takes the key's `on_delete` and
`on_update` actions (`"no_action"`, `"restrict"`, `"cascade"`, `"set_null"`,
`"set_default"`) and its `fk_name`, as Rust's does, as `bakery.py`'s
`cake_baker_cake` deletes a cake's `cake_baker` rows with it. `has_one` and
`has_many` take none of them.

```python
from bakery import Bakery, Cake

cake_in_oslo = Cake.belongs_to(
    Bakery, from_="bakery_id", to="id", on_condition=lambda cake, bakery: bakery.col("city") == "Oslo"
)
assert Cake.find().join(cake_in_oslo).inspect().sql == (
    'SELECT "cake"."id", "cake"."name", "cake"."flavor", "cake"."price", '
    '"cake"."gluten_free", "cake"."bakery_id" FROM "cake" '
    'INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" AND "bakery"."city" = $1'
)
```

The Rust derive implements `Related<E>` for each entity one variant reaches,
so a query can name the related entity alone. A Python model holds no list of
the models it reaches: every join and every load names the relation it
follows, such as `cake_bakery`, so two relations reaching one model need
nothing extra.

A many-to-many relation goes through a junction table. Where Rust writes the
cake's `Related<baker::Entity>` by hand, with `via` the junction's relation to
the cake, Python declares the same path with the model's `related`:
`Cake.related(cake_baker_baker, via=cake_bakers)`, `via` the relation from
the cake to its `cake_baker` rows and then the relation from those to the
baker, which `bakery.py` names `cake_related_bakers` and
[Relations](07-relations.md) reads along. A query can also follow the two
relations itself. These are the cakes Ada bakes:

```python
import asyncio
import os

from pgorm import Pool

from bakery import Baker, Cake, cake_baker_baker, cake_bakers


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        adas = (
            Cake.find()
            .join(cake_bakers)
            .join(cake_baker_baker)
            .where(Baker.col("name") == "Ada")
            .order_by(Cake.col("id").asc())
        )
        assert adas.inspect().sql == (
            'SELECT "cake"."id", "cake"."name", "cake"."flavor", "cake"."price", '
            '"cake"."gluten_free", "cake"."bakery_id" FROM "cake" '
            'INNER JOIN "cake_baker" ON "cake"."id" = "cake_baker"."cake_id" '
            'INNER JOIN "baker" ON "cake_baker"."baker_id" = "baker"."id" '
            'WHERE "baker"."name" = $1 ORDER BY "cake"."id" ASC'
        )
        names = [cake["name"] for cake in await adas.all(pool)]
        assert names == ["Chocolate Fudge", "Lemon Drizzle"]


asyncio.run(main())
```

A join can start from the query's model or from a table it has already
joined, which is how the second join starts from `cake_baker`.
[Relations](07-relations.md) reads related rows along relations, loads them
in batches, and decodes a row with the rows it reaches.

## Enums

A column of a PostgreSQL enum has a `TypeName` for its kind, naming the
type and its schema, and lists the enum's labels in `values`. The cake's
flavor is the type `public.flavor`:

```python
from pgorm import Column, ConstructionError, Model, TypeName

FLAVOR = TypeName("flavor", schema="public")

Cake = Model("cake", {
    "id": Column("i32", primary_key=True, default=True),
    "flavor": Column(FLAVOR, values=["chocolate", "vanilla", "lemon", "carrot"]),
})

assert Cake.columns["flavor"].values == ("chocolate", "vanilla", "lemon", "carrot")
assert Cake.describe()["fields"]["flavor"]["kind"] == 'enum "public"."flavor"'

# A label travels as text, cast to the enum's type.
lemon = Cake.find().where(Cake.col("flavor") == "lemon").inspect()
assert lemon.sql == (
    'SELECT "cake"."id", "cake"."flavor" FROM "cake" '
    'WHERE "cake"."flavor" = CAST($1::text AS public.flavor)'
)
assert [value.value for value in lemon.params] == ["lemon"]

try:
    Cake.col("flavor").eq("mint")
except ConstructionError as error:
    assert str(error) == "'mint' is not one of the column's values"
else:
    raise AssertionError("mint is no flavor")
```

Here Python differs from Rust. A Rust enum is a Rust type, its values the
variants, written and read as labels. A Python enum column's values are the
labels themselves, as `str`. With `values` listed, a label the list does not
hold is refused as it is written, with a `ConstructionError`, and as it is
read, with a `DecodeError`; without `values`, the column takes any label. A
Python `enum.StrEnum` can name the labels in a program, but the column takes
its member's `.value`: the member itself is refused.

The `TypeName` must name the type's schema, `public` for the bakery's,
because a decoded row names an enum with its schema; a `TypeName` without one
is a `ConstructionError` in a `Column`. The Rust enum's attributes map to
Python like this:

- `enum_name` and `schema_name` are the `TypeName`'s name and `schema`.
- `rename_all` and `string_value` have nothing to do, since the labels are
  written as they are.
- `rs_type` and `db_type`, an enum stored as integers or plain text, are a
  column of the kind it is stored as, such as `"i16"`, whose `decode` and
  `encode` turn the stored value into a Python `enum.IntEnum` or `StrEnum`
  member and back ([Values and types](13-values.md#newtypes)).
- `display` is the Python enum's own `str()`, and the labels are already a
  list, `Cake.columns["flavor"].values`, where Rust iterates `Flavor::iter()`.

An array of an enum is `array=True` beside the `TypeName`, which names its
schema there too. [Schema generation](14-schema.md) creates an enum's type,
and [Values and types](13-values.md) covers enum arrays and enums inside
domains.

## Reading a declaration back

A model is data, and `describe()` reports its declaration as data: the
table's `schema`, `table` and `alias`, its `primary_key` fields, its
`unique_keys`, each with its `name`, `fields` and `nulls_not_distinct`, and
under `fields`, for each field, its SQL `column`, its `kind`, and the
`nullable`, `primary_key`, `default`, `generated` and `values` it was
declared with. The kind is spelled as a decoded column's kind is compared:
`"i32"`, `"text[]"` for an array, `'enum "public"."flavor"'` for an enum,
`'range "app"."floatrange" of f64'` for a created range. JavaScript's
`describe()` reports the same keys, spelled in camelCase. A `Column` has no
methods and does not describe itself, and neither does `col(field)`.

```python
from bakery import Customer

described = Customer.describe()
assert (described["schema"], described["table"], described["alias"]) == (None, "customer", None)
assert described["primary_key"] == ["id"]
assert described["fields"]["notes"] == {
    "column": "notes", "kind": "text", "nullable": True, "primary_key": False,
    "default": False, "generated": None, "values": None,
}
assert described["fields"]["id"]["default"] is False
assert Customer.columns["id"].auto_increment
```

A registered entity has `describe()` too, reporting what its Rust
declaration says: the registration's `name`, its `schema` and `table`, its
`primary_key` and `unique_keys` (each a `name`, `columns` and
`nulls_not_distinct`), and `primary_key_without_overlaps`, true for a
temporal key. Under `columns`, each column's SQL `name`, its `json_key`, its
`sql_type`, the `rust_type` its model reads it as, `nullable`, `primary_key`,
the `kind` its values bind and read as, `None` for a type that needs an
explicit `pgorm.Value`, and an enum's `values`. Under `relations`, every
variant of the entity's `Relation` enum: its `name`, its `type`, `"has_one"`
or `"has_many"` (a `belongs_to` is pgorm's `has_one` from the side holding
the key), its `from` and `to` tables, the `columns` it pairs, the `period`
pair a temporal foreign key matches for overlap, whether a Rust
`on_condition` adds a condition of its own (the condition itself is Rust
code and is not reported), its `condition_type`, and its foreign key's
`enforcement` and `deferrability`. Under `rust`, the concrete `entity`,
`model`, `active_model` and `column` type names. A registered entity's
columns are named by their SQL names, not by Python fields.

## Hooks

A declared model takes its hooks as functions, the dialect of Rust's
`ActiveModelBehavior`: `hooks=` maps `before_save(values, insert)`,
`after_save(record, insert)`, `before_delete(values)` and
`after_delete(values)` to functions of the program's own, each optional,
each free to be a coroutine function, which is awaited. They run around the
model's record writes: `Model.active(values)` holds the fields one write
sets, as an active model does, and its `insert`, `update` and `delete` write
one record by its primary key, as Rust's active model writes itself.

`before_save` and `before_delete` give the values to write, and refuse the
write by raising: nothing is sent, and the exception reaches the caller as it
was raised. `after_save` gives the record the write returns. An `after_`
hook that raises reports its error after the row is written, as Rust's does;
run the write in a transaction to undo it. Rust's `new` hook, which fills
the defaults an active model starts from, is the program's own here, as
`before_save`'s `{"note": .., **values}` does below. The statement builders
of [Writing rows](04-writing.md) — `insert`, `insert_many`, `update` and
`delete` on the model, their returning and try-insert terminals, and an
update `from_` or a delete `using` — run no hook, as Rust's `Insert`,
`Update` and `Delete` run none.

```python
import asyncio
import os
from collections.abc import Mapping
from typing import Any

from pgorm import Column, ConstructionError, Model, Pool, RawSQL


def before_save(values: Mapping[str, Any], insert: bool) -> Mapping[str, Any]:
    if not values.get("name"):
        raise ConstructionError("a special needs a name")
    return {"note": "Ask at the counter", **values}


Special = Model("special", {
    "id": Column("i32", primary_key=True, default=True),
    "name": Column("text"),
    "note": Column("text"),
}, hooks={"before_save": before_save})


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await pool.execute(RawSQL(
            "CREATE TABLE special (id serial PRIMARY KEY, name text NOT NULL, note text NOT NULL)"
        ))
        saved = await Special.active({"name": "Saturday cinnamon buns"}).insert(pool)
        assert saved["note"] == "Ask at the counter"

        try:
            await Special.active({"name": ""}).insert(pool)
        except ConstructionError as error:
            assert str(error) == "a special needs a name"
        else:
            raise AssertionError("a special without a name is refused")
        assert Special.describe()["hooks"] == ["before_save"]


asyncio.run(main())
```

A registered entity's hooks are its Rust `ActiveModelBehavior`, written in
the Rust crate as the [Rust guide's
Entities](../../docs/guide/02-entities.md) chapter shows, and they run when
Python writes through the entity's active model. `entity.active()` calls the
hook `new`, so the active model starts from the defaults it sets. An active
model's `insert`, `update` and `delete` run `before_save` or
`before_delete` before the statement and `after_save` or `after_delete`
after it. The statement terminals of [Writing rows](04-writing.md), such as
`entity.insert(active).returning_model(db)` or
`entity.update_many().set(..)`, run none.

A hook's refusal, the `pgorm::Error::Custom` it returns, raises
`ConstructionError` carrying its message; any other error a hook returns
keeps its own class, so a `DatabaseError` from the server stays one. A
`before_` hook that fails prevents the write. An `after_` hook that fails is
reported even though the write may already have committed. The package adds
no transaction and no retry around these writes. Closing the connection or
the pool cancels a hook still running. The bakery's Rust entities declare no
hooks.

## The other derives

A Rust entity is one `Model` and one `Relation`, and derives make the other
shapes a program reads and writes. Python makes most of them with ordinary
Python values:

| Rust derive | In Python |
| --- | --- |
| `PartialModel` | `select(*fields)` reads some of a model's fields as records; a query of expressions is the query builder's, its rows `Record`s ([Reading rows](05-reading.md)) |
| `FromQueryResult` | every statement's rows are `Record`s, read by column name; no class of the program's is decoded into |
| `Model` | a second `Model` of the same table, declaring some of its columns |
| `IntoActiveModel` | any mapping is a write's values, so a form's dictionary is written as it is |
| `ValueType` | no counterpart: a column's values are its kind's; `typing.NewType` keeps them apart for a type checker |
| `JsonValueType` | a `"json"` column takes and gives dictionaries, lists and scalars; converting a class to and from them is the program's |
| `CreatedRange` | `pgorm.CreatedRange` and `pgorm.CreatedMultirange` as a column's kind ([Values and types](13-values.md)) |
| `Disjoint` | `pgorm.Disjoint` ([Disjoint enums](15-disjoint.md)) |
| `OneOf` | `pgorm.OneOf` ([One-of references](16-one-of.md)) |
| `Column` | `pgorm.col(name, table=...)` names any column |
| `Entity`, on a hand-written `Entity` | a model is built at run time ([below](#names-computed-at-run-time)) |
| `SqlName`, `StaticName` | the [query builder](11-query-builder.md) names tables and columns by strings: `pgorm.Table("cake")`, `pgorm.col("name")` |

A model need not declare every column of its table. One that declares some
reads and writes only those, as a further Rust `Model` of an entity does,
and the table's other columns need defaults for its inserts to succeed:

```python
import asyncio
import os

from pgorm import Column, Model, Pool

CakeName = Model("cake", {
    "id": Column("i32", primary_key=True, default=True),
    "name": Column("text"),
})


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        first = await CakeName.find().order_by(CakeName.col("id").asc()).limit(2).all(pool)
        assert [dict(cake) for cake in first] == [
            {"id": 1, "name": "Chocolate Fudge"},
            {"id": 2, "name": "Lemon Drizzle"},
        ]


asyncio.run(main())
```

Rust's `IntoActiveModel` turns a form into an active model with its fields
set and the rest left out. A Python write takes the form's mapping as it is,
and a field the form leaves out takes the column's default. The newsletter's
sign-up form sends two of the subscriber's columns:

```python
import asyncio
import os
from typing import TypedDict

from pgorm import Column, Model, Pool
from pgorm import schema as s

Subscriber = Model("subscriber", {
    "id": Column("i32", primary_key=True, default=True),
    "name": Column("text"),
    "email": Column("text"),
    "weekly": Column("bool", default=True),
}, unique_keys=[{"fields": "email"}])


class SignUp(TypedDict):
    """What the newsletter's sign-up form sends: two of the columns."""

    name: str
    email: str


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await pool.execute(
            s.create_table("subscriber")
            .column(s.ColumnDef("id", "integer").not_null().auto_increment())
            .column(s.ColumnDef("name", "varchar").not_null())
            .column(s.ColumnDef("email", "varchar").not_null())
            .column(s.ColumnDef("weekly", "boolean").not_null().default(True))
            .primary_key("id")
            .unique("email")
        )

        form: SignUp = {"name": "Mo", "email": "mo@example.com"}
        mo = await Subscriber.insert(form).returning().one(pool)
        assert mo["email"] == "mo@example.com"
        # The form left `weekly` out, so it took the column's default.
        assert mo["weekly"] is True


asyncio.run(main())
```

A Rust form's `Option<T>` field sets its column when it is `Some` and leaves
it out when it is `None`, and `Some(None)` sets a nullable column to `NULL`.
A Python mapping says the same with a key absent, which leaves the column
out, and a key holding `None`, which sets it to `NULL`.

A Rust `ValueType` newtype is a type the compiler keeps apart from the one it
wraps. Python has no such derive, since a column's values are its kind's, but
`typing.NewType` gives a type checker the same distinction, and at run time
the value is the integer it wraps. The recipe weighs its flour in grams:

```python
from typing import NewType

from pgorm import Column, Model

Grams = NewType("Grams", int)

Recipe = Model("recipe", {
    "id": Column("i32", primary_key=True, default=True),
    "flour": Column("i32"),
})


def heavier_than(weight: Grams) -> str:
    return Recipe.find().where(Recipe.col("flour") > weight).inspect().sql


assert heavier_than(Grams(500)) == (
    'SELECT "recipe"."id", "recipe"."flour" FROM "recipe" WHERE "recipe"."flour" > $1'
)
```

## Names computed at run time

A Rust entity's table and schema are fixed when it is written, unless its
`Entity` is written by hand to compute them. A Python model is built when the
program runs, so a name only known then needs nothing special: compute it,
and declare the model with it. A deployment that keeps each region in a
schema of its own declares its models once it knows the region:

```python
from pgorm import Column, Model


def cakes_in(region: str) -> Model:
    return Model("cake", {
        "id": Column("i32", primary_key=True, default=True),
        "name": Column("text"),
    }, schema=region)


# The region this deployment serves, known once the program starts.
REGION = "oslo"

Cake = cakes_in(REGION)
assert Cake.find().inspect().sql == 'SELECT "cake"."id", "cake"."name" FROM "oslo"."cake"'
```

`schema=` puts the table in a schema, and `Model` also takes a
`pgorm.Table`, schema included, in place of the name. A query qualifies a
model's columns by the table's name, never its schema, as pgorm qualifies an
entity's.

`as_(alias)` gives the model under another name: a model of the same table
and fields whose columns are qualified by the alias, for a query that reads
a table twice, as a self-join does. An aliased model reads, but it does not
insert: an insert into one is a `ConstructionError`.

```python
from decimal import Decimal

from bakery import Cake

c = Cake.as_("c")
assert c.select("name").where(c.col("price") > Decimal("4.00")).inspect().sql == (
    'SELECT "c"."name" FROM "cake" AS "c" WHERE "c"."price" > $1'
)
```

A registered entity's names are the Rust entity's: one whose schema is
computed decides it in Rust, as the [Rust guide](../../docs/guide/02-entities.md)
shows.

## Scopes

Some tables are read only under a condition the program supplies: a
company's notes by its members, a user's notes by that user. Forgetting the
condition once lets a statement read another company's rows. A *scope*
holds a table's statements to it: every statement on a scoped model is
built from a *context* holding its scopes, and every way to build one
without such a context is refused as the statement is built, before
anything is sent, in the words the [Rust guide's
Scopes](../../docs/guide/02-entities.md#scopes) section shows Rust's compiler
using. pgorm writes the condition; holding the rows to it in the database,
with row-level security or otherwise, stays the program's.

Here Python differs from Rust: Python has no compile time, so where Rust's
compiler stops a statement, Python's builder raises a `ConstructionError` as
the statement is built. A type checker catches part of it before then: a
scoped model's forms taking no context are typed for unscoped models alone,
so strict mypy refuses them.

### Declaring scopes

A scope is a class of the program's own, a subclass of `Scope`; the type
parameter is what its values are. A column holds a scope where it says so
with `scope=`, and the column's kind converts the scope's values as it
converts a value written to it. A column's kind alone never makes it a scope
column:

```python
from pgorm import Column, ConstructionError, Model, Scope


class CompanyId(Scope[int]):
    """A company, the scope everything its members read is held to."""


class UserId(Scope[int]):
    """A user, the scope what one member wrote is held to."""


Note = Model("note", {
    "id": Column("i64", primary_key=True, generated="by_default"),
    "company_id": Column("i64", scope=CompanyId),
    "author_id": Column("i64", scope=UserId),
    "body": Column("text"),
})

assert Note.scopes == ("CompanyId", "UserId")

# A scope column is never NULL: a row is in one value of its scope.
try:
    Column("i64", scope=UserId, nullable=True)
except ConstructionError as refused:
    assert "never NULL" in str(refused)
else:
    raise AssertionError("a nullable scope column is refused")
```

A scope is named by its class, as Rust names one by its type, and a model
holds each scope in one column. A scope column is written when its row is
inserted and never changed after, since a row cannot leave its scope; a
nullable, array or generated one is refused where it is declared. A primary
key can be a scope column too: a company's `id` is a `CompanyId`, so a
member reads their own company's row and no other.

### Contexts

A *context* is what a statement on a scoped model is built under. One scope
value is a context holding itself, and a subclass of `ScopeContext` is a
context holding each of its fields, which a dataclass declares. A field of
`Every[S]`, given as `Every(S)`, holds every value of `S`: the context lifts
that scope on purpose, and the statement gets no condition for it. A member
is signed in to one company as one user, and a company's administrator sees
every one of its users:

```python
from dataclasses import dataclass

from pgorm import Column, Every, Model, Scope, ScopeContext


class CompanyId(Scope[int]):
    pass


class UserId(Scope[int]):
    pass


@dataclass(frozen=True)
class Member(ScopeContext):
    company: CompanyId
    user: UserId


@dataclass(frozen=True)
class CompanyAdmin(ScopeContext):
    company: CompanyId
    users: Every[UserId]


Company = Model("company", {
    "id": Column("i64", primary_key=True, scope=CompanyId),
    "name": Column("text"),
})
Note = Model("note", {
    "id": Column("i64", primary_key=True, generated="by_default"),
    "company_id": Column("i64", scope=CompanyId),
    "author_id": Column("i64", scope=UserId),
    "body": Column("text"),
})

ada = Member(CompanyId(1), UserId(10))
admin = CompanyAdmin(CompanyId(1), Every(UserId))
assert Note.find_for(ada).inspect().sql == (
    'SELECT "note"."id", "note"."company_id", "note"."author_id", "note"."body" '
    'FROM "note" WHERE "note"."company_id" = $1 AND "note"."author_id" = $2'
)
assert Note.find_for(admin).inspect().sql == (
    'SELECT "note"."id", "note"."company_id", "note"."author_id", "note"."body" '
    'FROM "note" WHERE "note"."company_id" = $1'
)
# A scope value alone is a context, and the company's key is its scope.
assert Company.find_for(CompanyId(1)).inspect().sql == (
    'SELECT "company"."id", "company"."name" FROM "company" WHERE "company"."id" = $1'
)
```

A context holds each scope once, as one value or as every value; a context
holding one twice is refused when a statement is built under it. A context is
a scope value, an `Every` or a `ScopeContext`, and anything else is a
`TypeError`.

### Every statement takes its context first

Each statement constructor has a form ending in `_for` that takes the
context first: a model's `select_for`, `find_for`, `find_by_key_for`,
`graph_for`, `insert_for`, `insert_many_for`, `update_for` and `delete_for`;
an insert's `add_for` and `add_many_for`, and a try-insert's; an active
model's `insert_for`, `update_for` and `delete_for`; a query's `join_for` and
`join_rev_for`; a relation's and a related path's `find_for`; a linked
chain's `find_for` and `find_linked_for`; and `pgorm.pipeline.from_for`. An
unscoped model takes any context, and its forms taking none are the ones it
always had.

On a scoped model, the forms taking no context are refused, and so is a
`_for` form given a context missing one of its scopes:

```python
from pgorm import Column, ConstructionError, Model, Scope


class CompanyId(Scope[int]):
    pass


class UserId(Scope[int]):
    pass


Note = Model("note", {
    "id": Column("i64", primary_key=True, generated="by_default"),
    "company_id": Column("i64", scope=CompanyId),
    "author_id": Column("i64", scope=UserId),
    "body": Column("text"),
})

try:
    Note.find()  # type: ignore[misc]
except ConstructionError as refused:
    assert str(refused).startswith(
        "`note` is scoped by `CompanyId` and `UserId`, which a statement taking no "
        "context does not hold"
    )
else:
    raise AssertionError("a scoped model's find() is refused")

try:
    Note.find_for(CompanyId(1))
except ConstructionError as refused:
    assert str(refused) == (
        "`note` is scoped by `CompanyId` and `UserId`, which `CompanyId` does not hold"
    )
else:
    raise AssertionError("a context missing a scope is refused")
```

The refusal names the table, its scopes and the context, by its class, as
Rust's compile error names the entity and the context's type. The
`type: ignore` silences mypy's own refusal of the same call: a `Column` is
generic in the scope it holds, `Never` for none, and a `Model` in its
columns', and `find` is typed for a `Model[Never]` alone, a model whose
columns hold no scope. A loader is given a query
rather than a model: hand a relation's `load` a `find_for` query, since the
model alone is every row and is refused for a scoped one.

### Reading and joining

A read ANDs each scope its context holds the value of into the `WHERE`, so
Ada reads her own notes and the administrator reads the company's. A graph
keeps its context, and each slot's scopes go in its join's `ON`; a query's
`join_for` puts the joined table's there too, and a plain `join` into a
scoped table is refused:

```python
import asyncio
import os
from dataclasses import dataclass
from typing import cast

from pgorm import (
    Column, ConstructionError, DecodeError, Every, Model, ModelRecord, Pool, Scope, ScopeContext,
)
from pgorm import schema as s


class CompanyId(Scope[int]):
    pass


class UserId(Scope[int]):
    pass


@dataclass(frozen=True)
class Member(ScopeContext):
    company: CompanyId
    user: UserId


@dataclass(frozen=True)
class CompanyAdmin(ScopeContext):
    company: CompanyId
    users: Every[UserId]


Company = Model("company", {
    "id": Column("i64", primary_key=True, scope=CompanyId),
    "name": Column("text"),
})
Note = Model("note", {
    "id": Column("i64", primary_key=True, generated="by_default"),
    "company_id": Column("i64", scope=CompanyId),
    "author_id": Column("i64", scope=UserId),
    "body": Column("text"),
})
company_notes = Company.has_many(Note, from_="id", to="company_id")


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        for model in (Company, Note):
            await pool.execute(s.from_model(model).table)
        for identity, name in ((1, "Acme"), (2, "Globex")):
            await Company.insert_for(CompanyId(identity), {"name": name}).execute(pool)
        for company, user, body in ((1, 10, "Ada's plan"), (1, 11, "Bo's draft"), (2, 20, "Cy's memo")):
            writer = Member(CompanyId(company), UserId(user))
            await Note.insert_for(writer, {"body": body}).execute(pool)

        ada = Member(CompanyId(1), UserId(10))
        admin = CompanyAdmin(CompanyId(1), Every(UserId))
        hers = await Note.find_for(ada).all(pool)
        assert [note["body"] for note in hers] == ["Ada's plan"]
        theirs = await Note.find_for(admin).order_by(Note.col("id").asc()).all(pool)
        assert len(theirs) == 2, "Ada's and Bo's notes, and not Globex's"

        # Cy's note belongs to Globex, so Ada finds no note of its key.
        try:
            await Note.find_by_key_for(ada, {"id": 3}).one(pool)
        except DecodeError:
            pass
        else:
            raise AssertionError("a row outside the scopes is no row")

        # A graph keeps its context, and each slot's scopes go in its join's ON.
        acme = cast(
            list[tuple[ModelRecord, ModelRecord | None]],
            await Company.graph_for(ada).join_maybe(company_notes).all(pool),
        )
        authors = [(company["name"], None if note is None else note["author_id"]) for company, note in acme]
        assert authors == [("Acme", 10)]

        joined = await Company.find_for(ada).join_for(ada, company_notes).all(pool)
        assert len(joined) == 1
        try:
            Company.find_for(ada).join(company_notes)
        except ConstructionError as refused:
            assert "join it with a form taking the statement's context, such as `join_for`" in str(refused)
        else:
            raise AssertionError("a plain join into a scoped table is refused")


asyncio.run(main())
```

A pipeline reads a scoped model through `pgorm.pipeline.from_for`, which
filters its rows by each scope value the context holds, binding the value;
the model given to `from_` or as a join's source is refused, and another
scoped model joins the pipeline as a pipeline of its own, `from_for(cx,
other)`.

### Writing

An insert under a context fills each scope column the row leaves out with
the value the context holds, and refuses a row setting one to another value.
Under `Every` of a scope the insert fills nothing: the row sets the column
itself, and a row that does not is refused.

An update or a delete under a context carries the scopes in its `WHERE`,
before any condition of its own, so a row outside them matches nothing: an
active model's `update_for` of such a key is a `DecodeError`, as Rust's
`RecordNotFound` is, and a delete counts `0`. A scope column is written when
its row is inserted and never changed after: an update setting one, and an
upsert whose `ON CONFLICT .. DO UPDATE` names one, are refused before
anything is sent.

```python
import asyncio
import os
from dataclasses import dataclass

from pgorm import Column, ConstructionError, DecodeError, Every, Model, Pool, Scope, ScopeContext
from pgorm import schema as s


class CompanyId(Scope[int]):
    pass


class UserId(Scope[int]):
    pass


@dataclass(frozen=True)
class Member(ScopeContext):
    company: CompanyId
    user: UserId


@dataclass(frozen=True)
class CompanyAdmin(ScopeContext):
    company: CompanyId
    users: Every[UserId]


Note = Model("note", {
    "id": Column("i64", primary_key=True, generated="by_default"),
    "company_id": Column("i64", scope=CompanyId),
    "author_id": Column("i64", scope=UserId),
    "body": Column("text"),
})


def refused(build: object, words: str) -> None:
    assert callable(build)
    try:
        build()
    except ConstructionError as refusal:
        assert words in str(refusal), refusal
    else:
        raise AssertionError(f"refused: {words}")


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await pool.execute(s.from_model(Note).table)
        ada = Member(CompanyId(1), UserId(10))
        bo = Member(CompanyId(1), UserId(11))
        admin = CompanyAdmin(CompanyId(1), Every(UserId))
        await Note.insert_for(bo, {"body": "Bo's draft"}).execute(pool)

        written = await Note.insert_for(ada, {"body": "Ada's plan"}).returning().one(pool)
        assert (written["company_id"], written["author_id"]) == (1, 10)
        refused(
            lambda: Note.insert_for(ada, {"company_id": 2, "body": "for Globex"}),
            "the row sets `note`.`company_id` to a value outside the context's `CompanyId`",
        )
        refused(
            lambda: Note.insert_for(admin, {"body": "whose?"}),
            "the context holds every `UserId`, so the row sets `note`.`author_id` itself",
        )

        # Bo's note is the company's, so the administrator edits it and Ada cannot.
        edit = Note.active({"id": 1, "body": "Bo's draft, reviewed"})
        try:
            await edit.update_for(ada, pool)
        except DecodeError:
            pass
        else:
            raise AssertionError("Ada cannot edit Bo's note")
        assert (await edit.update_for(admin, pool))["body"] == "Bo's draft, reviewed"

        # Handing Bo's note to Ada would move it out of Bo's scope.
        refused(
            lambda: Note.update_for(admin, {"author_id": 10}),
            "`note`.`author_id` is written when its row is inserted and never changed",
        )
        assert await Note.delete_for(ada).where(Note.col("id") == 1).execute(pool) == 0


asyncio.run(main())
```

A scope written by hand, Rust's `ScopeOn` with a condition of its own such
as a message's sender or recipient, is Rust's alone: a declared model's
scopes are its scope columns. A declared disjoint enum or one-of reference
reads its models under no context, so one over a scoped model is refused as
it is declared.

### Registered entities

A Rust entity that declares scopes stays held to them from Python. The
application crate registers each scope type before the entity, and the
entity by `scoped` rather than `entity`:

```rust
registry.scope::<notes::CompanyId>()?;
registry.scope::<notes::UserId>()?;
registry.scoped::<notes::note::Entity>("app.Note")?;
```

A context names a registered scope by its Rust type's name, so a Python
`class CompanyId(Scope[int])` is the context's value of the Rust
`CompanyId`, converted by its `ValueType`. Every statement on the
registration is the Rust entity's own `_for` form under that context:
`find_for`, `insert_for`, `insert_many_for`, `update_for`,
`update_many_for`, an active model's `insert_for`, `update_for` and
`delete_for`, a query's `join_for` and `pipeline.from_for`, its written
scopes included. The forms taking no context are refused as they are built,
as a declared model's are, and a registration's `scopes` lists the scopes
its entity declares. A graph, a source tuple, a disjoint enum and a one-of
reference register over unscoped entities alone. [Registering Rust
entities](24-registrations.md) says how to build such a wheel.

### Raw SQL

`RawSQL`, `find_by_statement` and the statement builders used directly are
outside scopes: pgorm does not read the SQL they hold. A read meant to cross
scopes, such as a report over every company, is written with them.
