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

from bakery import Bakery, CakeBaker, OrderLine

FLAVOR = TypeName("flavor", schema="public")
FLAVORS = ["chocolate", "vanilla", "lemon", "carrot"]

Cake = Model("cake", {
    "id": Column("i32", primary_key=True, default=True),
    "name": Column("text"),
    "flavor": Column(FLAVOR, values=FLAVORS),
    "price": Column("decimal"),
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
column is declared. And since a model creates no table, it does not say the
column's exact type: a `varchar` and a `text` column are both `"text"`, and
the cake's `decimal(10, 2)` price is `"decimal"`. There is no `column_type`.

A column is named for its field. `name="..."` names it otherwise, so a
field can stand for a column whose name is not a Python identifier:
`Column("text", name="display name")`. Table, schema and column names, and
the fields' own, may hold any character but NUL, from 1 to 63 bytes, and the
SQL quotes them as pgorm quotes every identifier. The mapping is ordinary
Python, so a program that wants every column renamed by a case style, as
Rust's `rename_all` does, builds the mapping with that rename; there is no
option for it.

A Rust entity's declaration also says how its table is created, and Rust's
schema generation turns it into `CREATE TABLE`. A declared model does not
write its own `CREATE TABLE`: that is not available in Python yet. Its table
is made by the schema builders, as `bakery.py` makes the bakery's, or by a
migration. [Schema generation](14-schema.md) covers the builders, and also
generating a registered entity's table. These examples use the builders to
show the table a declaration describes. The bakery's customers are unique by
email:

```python
from pgorm import Column, Model
from pgorm import schema as s

Customer = Model("customer", {
    "id": Column("i32", primary_key=True, default=True),
    "name": Column("text"),
    "email": Column("text"),
    "notes": Column("text", nullable=True),
}, unique_keys=[{"fields": ("email",)}])

create = (
    s.create_table("customer")
    .column(s.ColumnDef("id", "integer").not_null().auto_increment())
    .column(s.ColumnDef("name", "varchar").not_null())
    .column(s.ColumnDef("email", "varchar").not_null())
    .column(s.ColumnDef("notes", "text"))
    .primary_key("id")
    .unique("email")
)
assert create.inspect().sql == (
    'CREATE TABLE "customer" ( "id" serial NOT NULL, "name" varchar NOT NULL, '
    '"email" varchar NOT NULL, "notes" text, PRIMARY KEY ("id"), UNIQUE ("email") )'
)
assert Customer.describe()["unique_keys"] == [
    {"name": None, "fields": ["email"], "nulls_not_distinct": False}
]
```

## Primary keys

A field marked `primary_key=True` is part of the table's primary key, and a
key of several columns is a field marked so for each. A key column is never
nullable: declaring one `nullable` is a `ConstructionError`.

Here Python differs from Rust. A Rust entity must have a primary key, and a
key of one integer column is generated by the database unless it says
`auto_increment = false`. A Python model may declare no key, as a model of a
view might, though then its `key` and `find_by_key` are refused. And nothing
about a key is assumed: the declaration says how the database fills it.
`default=True` says the table has a default for it, as the bakery's `serial`
ids do, so an insert may leave it out. Without that, or one of the
`generated` options below, an insert must give the key.

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
    "body": Column("text"),
})


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        create = (
            s.create_table("note")
            .column(s.ColumnDef("tenant_id", "integer").not_null())
            .column(s.ColumnDef("id", "bigint").not_null().identity("always"))
            .column(s.ColumnDef("body", "varchar").not_null())
            .primary_key(["tenant_id", "id"], name="pk-note")
        )
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

A Rust key whose last column is a range can be a PostgreSQL 18 temporal key,
marked `without_overlaps`. A Python model has no option for it: declare the
range field `primary_key=True` like the rest of the key, and create the
table with the schema builders' `primary_key(..., without_overlaps=...)`.
A lookup by the whole key compares the range for equality, as Rust's does,
so it finds exactly one version of the row.

## Defaults, generated columns and keys

A column can take a default, be computed from the others, or be part of a
unique key:

- `default=True` says the table has a default for the column, so an insert
  may leave it out and the database fills it. The model does not know what
  the default is: the table holds it. Rust's `default_value` and
  `default_expr` write the default into the table; in Python, the schema
  builders' `ColumnDef.default(...)` does.
- `generated="always"` also marks a generated column, computed from the
  row's other columns, which the database stores or computes on read. No
  write sets one, and every read returns it. Rust's `generated_stored` and
  `generated_virtual` write the expression; in Python, `ColumnDef.generated`
  does.
- `unique_keys` on the `Model` lists the table's unique keys, as Rust's
  `unique` and `unique(columns = (..))` do. Each is a mapping: its `fields`,
  one field or several in key order, and an optional `name` and
  `nulls_not_distinct`. A key naming no field, a field the model lacks, or
  one field twice is a `ConstructionError`, and an option it does not have a
  `TypeError`.

A declared model writes no schema, so its unique keys are a description of
the table: `describe()` reports them, and a [disjoint
enum](15-disjoint.md)'s schema writes them. Nothing in a declaration asks
for an index or a comment, as Rust's `indexed` and `comment` do: the schema
builders' `create_index`, `comment_on_table` and `comment_on_column` make
them.

A column declared both `default=True` and `generated` is a
`ConstructionError`, since a generated column has no default of its own. An
insert must set every field that is neither nullable, defaulted nor
generated, and one that does not is refused with a `ConstructionError`
before anything is sent.

The bakery's order lines could carry their total as a stored generated
column, and allow each cake once per order. This example makes the table in
a schema of its own, beside the bakery's own `order_line`:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Column, Model, Pool, col
from pgorm import schema as s

OrderLine = Model("order_line", {
    "id": Column("i32", primary_key=True, default=True),
    "order_id": Column("i32"),
    "cake_id": Column("i32"),
    "quantity": Column("i32", default=True),
    "price": Column("decimal"),
    "total": Column("decimal", generated="always"),
}, schema="draft", unique_keys=[{"fields": ("order_id", "cake_id"), "name": "one_line_per_cake"}])


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        money = s.DataType("numeric", precision=10, scale=2)
        create = (
            s.create_table(OrderLine.table)
            .column(s.ColumnDef("id", "integer").not_null().auto_increment())
            .column(s.ColumnDef("order_id", "integer").not_null())
            .column(s.ColumnDef("cake_id", "integer").not_null())
            .column(s.ColumnDef("quantity", "integer").not_null().default(1))
            .column(s.ColumnDef("price", money).not_null())
            .column(
                s.ColumnDef("total", money).not_null()
                .generated(col("price") * col("quantity"), "stored")
            )
            .primary_key("id")
            .unique(["order_id", "cake_id"], name="one_line_per_cake")
        )
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
`on_delete`, `on_update` and `fk_name` belong to the table's foreign key,
which the schema builders' `foreign_key(..., on_delete=..., name=...)`
writes, as `bakery.py` does.

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
- `rs_type` and `db_type`, an enum stored as integers or plain text, have no
  Python counterpart: declare the column by the kind it is stored as, such as
  `"i16"`, and map the values in the program.
- `display` has no counterpart, and the labels are already a list,
  `Cake.columns["flavor"].values`, where Rust iterates `Flavor::iter()`.

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
assert described["fields"]["id"]["default"] is True
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
| `EntityTrait` | a model is built at run time ([below](#names-computed-at-run-time)) |
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
