# Values and types

Every column holds a value of some PostgreSQL type, and every field of a
model declares the *kind* that stands for it: a name such as `"i32"` or
`"decimal"`, which decides the Python type of its values. This chapter goes
through the kinds pgorm carries: what each one is in Python, what it is in
PostgreSQL, and anything that travels between the two in a way you should
know about. It is the Python dialect of the Rust guide's [Values and
types](../../docs/guide/12-values.md).

## The types at a glance

A field's kind decides what its column holds, as [Models and
entities](03-entities.md) showed. The same table answers both directions: a
value written to a field is converted through its kind, and a column read
back decodes to that kind's Python type.

| PostgreSQL | Kind | Python |
| --- | --- | --- |
| `boolean` | `bool` | `bool` |
| `smallint`, `integer`, `bigint` | `i16`, `i32`, `i64` | `int` |
| `real`, `double precision` | `f32`, `f64` | `float` |
| `numeric` | `decimal` | `decimal.Decimal` |
| `text`, `varchar`, `char(n)`, `name` | `text` | `str` |
| `bytea` | `bytes` | `bytes` |
| `date`, `time`, `timestamp` | `date`, `time`, `datetime` | `datetime.date`, `datetime.time`, a naive `datetime.datetime` |
| `timestamptz` | `datetime_utc` | an aware `datetime.datetime` in UTC |
| `interval` | `interval` | `pgorm.Interval` |
| `uuid` | `uuid` | `uuid.UUID` |
| `json`, `jsonb` | `json` | `dict`, `list`, `str`, `int`, `float`, `bool` or `None` |
| an array | the element's kind, with `array=True` | `list` |
| an enum | its `TypeName` | `str`, the label |
| a built-in range or multirange | its type's name, such as `daterange` | `pgorm.Range`, `pgorm.Multirange` |
| a range type the schema creates | its `CreatedRange` or `CreatedMultirange` | `pgorm.Range`, `pgorm.Multirange` |
| `inet`, `cidr`; `macaddr` | `ipnetwork`; `mac_address` | `str`; six `bytes` |
| pgvector's `vector` | `vector` | `list` of `float` |
| `"char"`, `oid` | `i8`, `u32` | `int` |

A field declared `nullable=True` holds `None` as well, which is SQL `NULL`.
The last row is PostgreSQL's internal types, which the system catalogues
use. Every Python type in the table is the standard library's, but for
pgorm's own `Interval`, `Range` and `Multirange`. A column typed by a domain
takes the kind of the type the domain is over, and a `money` column has no
kind of its own; both are covered below.

A field's kind has to be the one its column decodes as. An `integer` column
is `i32`, and declaring it `i64` raises `DecodeError` when a row is read,
even though every value would fit. A column of a type no kind covers, such
as `timetz`, `bit` or a composite row, raises `DecodeError` too, a `NULL` of
it included: pgorm never hands back such a value as text, a rounded number or
`None`.

Every row of the bakery shows some of these:

```python
import asyncio
import os
from datetime import date, datetime, timezone
from decimal import Decimal

from pgorm import Pool

from bakery import FLAVOR, Baker, Cake, Order


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        fudge = await Cake.find_by_key({"id": 1}).one(pool)
        assert fudge["price"] == Decimal("4.50")
        assert fudge["flavor"] == "chocolate"
        assert fudge["gluten_free"] is False

        # tagged() gives a field as a Value, which keeps its kind.
        assert fudge.tagged("id").kind == "i32"
        assert fudge.tagged("flavor").type_name == FLAVOR

        ada = await Baker.find_by_key({"id": 1}).one(pool)
        assert ada["hired_on"] == date(2019, 3, 1)

        kims = await Order.find_by_key({"id": 1}).one(pool)
        assert kims["placed_at"] == datetime(2026, 11, 2, 9, 30, tzinfo=timezone.utc)


asyncio.run(main())
```

A value that reaches a statement with no field to give it a kind, such as one
of [raw SQL](10-raw-sql.md)'s values or the operand of a plain expression,
takes the one kind its Python type leaves. `bool` is `bool`, never an
integer; `int` is `i64`; `float` is `f64`; `str` is `text`; `bytes`,
`Decimal`, `UUID`, `date`, `time`, a naive `datetime` and `Interval` are
their kinds; a `dict` is `json`; and a `list` is an array of the one kind its
items other than `None` take, each `None` an SQL `NULL` element. What its
type leaves open is refused with `ConstructionError`, asking for a kind:
`None`, an empty or all-`None` list, a list of several kinds or of lists, a
`tuple` (which reads as a row), an aware `datetime`, a range, and any other
object. Nothing is converted to text on the way, and an `int` too large for
`i64` is refused rather than promoted.

```python
import datetime as dt
from decimal import Decimal

import pgorm
from pgorm import Interval, Value

assert Value(True).kind == "bool"
assert Value(42).kind == "i64"
assert Value(4.5).kind == "f64"
assert Value("Lemon Drizzle").kind == "text"
assert Value(Decimal("3.20")).kind == "decimal"
assert Value(dt.date(2026, 11, 6)).kind == "date"
assert Value(Interval(days=3)).kind == "interval"
assert Value({"apron": "L"}).kind == "json"
assert Value(["morning", None]).element_type == "text"

ambiguous_values: list[object] = [
    None, [], (1, 2), dt.datetime(2026, 11, 6, tzinfo=dt.timezone.utc), 2**63,
]
for ambiguous in ambiguous_values:
    try:
        Value(ambiguous)
    except pgorm.ConstructionError:
        pass
    else:
        raise AssertionError(f"{ambiguous!r} needs a kind")
```

The server then decides the type of each placeholder, and pgorm writes the
value as that type, converting only between numbers: an integer to any
integer type it fits, to a float or to `numeric`, and a float to `real`
(rounding as PostgreSQL's own cast does), to `double precision` or to
`numeric`. Anything else is refused before the statement runs, naming the
parameter: `SELECT $1` with the value `1` is refused, since the server takes
an untyped `$1` as `text`. [The query builder](11-query-builder.md) shows how
a cast tells the server a placeholder's type.

## The `Value` container

When pgorm builds a statement it carries each parameter as a `pgorm.Value`:
the Rust library's own value, held natively, which keeps what a plain Python
value cannot say. It holds the kind, so integer widths, enum types and the
two kinds of timestamp stay apart, and a `NULL` keeps its kind. Every value
a statement binds becomes one, and `value.value` reads the Python value back
out:

```python
from decimal import Decimal

import pgorm
from pgorm import Value

price = Value(Decimal("4.50"))
assert price.kind == "decimal"
assert price.value == Decimal("4.50")

# A NULL keeps its kind.
missing = Value.null("text")
assert missing.is_null and missing.value is None
assert missing == Value(None, "text")
assert missing != Value.null("i32")

# A kind picks the width, and holds the value to it.
small = Value(42, "i16")
assert small != Value(42)
try:
    Value(40_000, "i16")
except pgorm.ConstructionError:
    pass
else:
    raise AssertionError("40000 does not fit a smallint")
```

Here Python differs from Rust in shape. Rust's `Value` is an enum with a
variant per kind, and `ValueType` reads a variant back into a Rust type; a
Python `Value` holds that same Rust value, its `kind` naming the variant, and
`value` reads it back. The JavaScript binding names the kinds alike. A
statement's `inspect()` returns its values beside its SQL, each a `Value`, as
[the query builder](11-query-builder.md) chapter shows, and a model's write
converts each field it is given into one.

`Value(data, kind)` takes a kind by its name: `bool`, `i8`, `i16`, `i32`,
`i64`, `u32`, `u64`, `f32`, `f64`, `text`, `char`, `bytes`, `decimal`,
`uuid`, `json`, `date`, `time`, `datetime`, `datetime_utc`, `interval`,
`ipnetwork`, `mac_address`, `vector`, and the twelve built-in range and
multirange types by name. An enum's kind is its `TypeName`, and a range type
the schema creates is its `CreatedRange` or `CreatedMultirange`. A name pgorm
has no kind for raises `ConstructionError`, and a kind that is neither a name
nor one of those objects raises `TypeError`. Passing a `Value` as the data
gives a copy of it, and naming a different kind beside it raises
`ConstructionError`. `pgorm.capabilities()["value_types"]` lists the kinds,
and `pgorm.capabilities()["model_policy"]["scalar_kinds"]` the ones a model
field can declare.

Two of the kinds are not column kinds. `u64` and `char` (one Unicode
character) are values a statement can bind but no row decodes as, so a model
field cannot declare them; PostgreSQL has no unsigned 64-bit integer, and a
`u64` is no claim that it has. `u32` is the kind of `oid`, and `i8` of the
internal `"char"`.

A value is immutable and can be bound any number of times. `value.value` is
a Python value of its own: a list or dictionary it returns can be changed
without changing the value. Two values are equal when their kinds and their
Rust values are, so `Value(1, "i32")` differs from `Value(1, "i64")`, an
enum's label from the same label of another type, and a float compares by its
bits: `NaN` equals a `NaN` with the same payload, and `0.0` differs from
`-0.0`. A value never equals a plain Python value, and it is not hashable.

Three constructors say what a plain value cannot. `Value.null(kind)`, the same
as `Value(None, kind)`, is the `NULL` of a kind. `Value.array(kind, items)`
is an array of a named element kind, which keeps its kind when it is empty
or `NULL`; each item is a Python value, a `Value` of the element kind, or
`None` for a `NULL` element, and an item of another kind or a nested list is
refused. `Value.json(data)` is a JSON document, so that JSON's `null` stays
apart from SQL `NULL`:

```python
from pgorm import Value

from bakery import FLAVOR

assert Value.array("i32", []) != Value.array("text", [])
assert Value.array("i32", None).is_null
tags = Value.array("text", ["morning", None])
assert tags.value == ["morning", None]
items = tags.items()
assert items is not None and [item.is_null for item in items] == [False, True]

# JSON's null is a value; SQL NULL is the absence of one.
assert Value.json(None) != Value.null("json")
assert not Value.json(None).is_null

# An enum's label carries its type, and an array of labels its element type.
lemon = Value("lemon", FLAVOR)
assert lemon.kind == "enum" and lemon.type_name == FLAVOR
assert Value.array(FLAVOR, ["lemon", None]).element_type == FLAVOR

# snapshot() writes a value out as JSON-ready data, every tag in it.
assert Value(42, "i16").snapshot() == {
    "version": 1, "type": {"kind": "i16"}, "sql_null": False, "data": "42",
}
assert lemon.snapshot()["type"] == {"kind": "enum", "name": "flavor", "schema": "public"}
```

A value's `kind` is its kind's name, or `"enum"`, `"array"`,
`"created_range"` or `"created_multirange"` for the kinds that carry more.
`is_null` says whether it is SQL `NULL`. `type_name` is the schema-qualified
`TypeName` of an enum or a created range or multirange, an array's being its
element's; `created_type` is a created range's or multirange's kind,
`element_type` an array's element kind, and `items()` an array's elements as
values. A `TypeName`'s name and schema are each 1 to 63 bytes of UTF-8 with no
NUL, kept as written, case and punctuation included. Making a `TypeName` or a
`Value` creates no type in the database.

`snapshot()` is for looking at a value, in a test or a log line: a
dictionary with `version` 1, the `type` with its tags, `sql_null`, and the
`data`. Integers and decimals are written as text, floats and a vector's
elements as the hexadecimal bits of the IEEE number (so infinities, `-0.0`
and `NaN` payloads survive JSON), bytes and a hardware address as a list of
numbers, a UUID or a network address as its text, a date or a time as its
ISO text, an array as the snapshot of each element, and an interval as its
three fields as text. It is not SQL, and not a format to read values back
from.

A row's `tagged(name)` gives one column as a `Value`, as the first example
did: a nullable `integer` and a nullable `bigint` both read as `None` in a
row, and their tagged values still say `i32` and `i64`. Neither direction
fails silently: data a kind cannot hold exactly raises `ConstructionError` as
the value is made, and a column with no exact Python form raises
`DecodeError` as the row is read.

## Dates and times

Each PostgreSQL type has the Python type that holds what it does:

- `date` is a calendar date, `datetime.date`.
- `time` is a time of day, `datetime.time`, with no time zone.
- `datetime` is a date and time with no time zone, `timestamp`, a naive
  `datetime.datetime`.
- `datetime_utc` is an instant, `timestamptz`, an aware `datetime.datetime`
  in UTC. PostgreSQL stores the instant and no zone, and so does pgorm.

The bakery's bakers work shifts. A shift falls on a date, starts at a time of
day, and records the instant the baker clocked in:

```python
import asyncio
import os
from datetime import date, datetime, time, timezone

from pgorm import Column, Model, Pool
from pgorm import schema as s

Shift = Model("shift", {
    "id": Column("i32", primary_key=True, default=True),
    "baker_id": Column("i32"),
    "on": Column("date"),
    "starts": Column("time"),
    "planned": Column("datetime"),
    "clocked_in": Column("datetime_utc", nullable=True),
})


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await pool.execute(
            s.create_table("shift")
            .column(s.ColumnDef("id", "integer").not_null().auto_increment())
            .column(s.ColumnDef("baker_id", "integer").not_null())
            .column(s.ColumnDef("on", "date").not_null())
            .column(s.ColumnDef("starts", "time").not_null())
            .column(s.ColumnDef("planned", "timestamp").not_null())
            .column(s.ColumnDef("clocked_in", "timestamptz"))
            .primary_key("id")
        )

        early = await Shift.insert({
            "baker_id": 1,
            "on": date(2026, 11, 6),
            "starts": time(5, 30),
            "planned": datetime(2026, 11, 6, 5, 30),
            "clocked_in": datetime(2026, 11, 6, 4, 28, tzinfo=timezone.utc),
        }).returning().one(pool)

        assert early["on"] == date(2026, 11, 6)
        assert early["starts"].isoformat() == "05:30:00"
        assert early["clocked_in"] == datetime(2026, 11, 6, 4, 28, tzinfo=timezone.utc)

        # Dates compare as dates.
        this_week = await (
            Shift.find()
            .where(Shift.col("on").between(date(2026, 11, 2), date(2026, 11, 8)))
            .count(pool)
        )
        assert this_week == 1


asyncio.run(main())
```

A `timestamptz` comes back as the instant it was, in UTC, whatever the
session's time zone: an aware `datetime` whose `tzinfo` is
`datetime.timezone.utc`. To show it in a zone, convert it in Python with
`astimezone`, where the zone rules are explicit. The other way, a
`datetime_utc` takes only a `datetime` whose offset is zero, since the value
has nowhere to keep another; convert one at another offset first. A
`zoneinfo` time that falls twice on the night the clocks go back is resolved
by its `fold` during that conversion, so the two instants it can mean stay
two:

```python
import asyncio
import os
from datetime import datetime, timezone
from zoneinfo import ZoneInfo

import pgorm
from pgorm import Pool, RawSQL

from bakery import Order

OSLO = ZoneInfo("Europe/Oslo")


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        async with pool.connection() as connection:
            await connection.execute(RawSQL("SET TIME ZONE 'Europe/Oslo'"))
            kims = await Order.find_by_key({"id": 1}).one(connection)

        placed = kims["placed_at"]
        assert placed == datetime(2026, 11, 2, 9, 30, tzinfo=timezone.utc)
        assert placed.astimezone(OSLO).hour == 10

        # Half past ten in Oslo is refused as it is, and found once in UTC.
        local = datetime(2026, 11, 2, 10, 30, tzinfo=OSLO)
        try:
            Order.col("placed_at").eq(local)
        except pgorm.ConstructionError:
            pass
        else:
            raise AssertionError("a datetime_utc takes a zero offset")
        found = await (
            Order.find()
            .where(Order.col("placed_at").eq(local.astimezone(timezone.utc)))
            .count(pool)
        )
        assert found == 1


asyncio.run(main())
```

PostgreSQL and Python both keep time to the microsecond, and every temporal
value travels exactly. What one side can hold and the other cannot raises an
error. A date or timestamp outside Python's years 1 to 9999, a `timestamptz`
later than 22:00 UTC on 30 December 9999 (where pgorm's Rust instant ends),
`infinity` and `-infinity`, and the time `24:00:00` raise `DecodeError` when
read. A `time` with a `tzinfo`, an offset of a fraction of a second, a naive
`time` or `datetime` with `fold=1`, and an instant past pgorm's end raise
`ConstructionError`. There is no kind for `timetz`, nor for
`datetime.timedelta`, which [Intervals](#intervals) covers.

## Numbers

Integers and floats map one to one: each width has its kind, and a Python
`int` or `float` crosses as it is. `numeric` maps to `Decimal`, which is
exact: `4.50` is stored as `4.50` and read back as `4.50`, with its scale.
Arithmetic the database does on a `numeric` stays exact, so a sum of prices
is the sum to the cent:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Pool, Table, select

from bakery import OrderLine


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        lines = await pool.query(
            select((OrderLine.col("price") * OrderLine.col("quantity")).as_("total"))
            .from_(Table("order_line"))
            .where(OrderLine.col("order_id").eq(1))
            .order_by(OrderLine.col("id").asc())
        )

        totals = [line["total"] for line in lines]
        assert totals == [Decimal("9.00"), Decimal("3.20")]
        assert str(sum(totals)) == "12.20"


asyncio.run(main())
```

Here Python differs from Rust: a model's query reads the model's fields, so
a computed column is read with the [query builder](11-query-builder.md)'s
`select`, as here.

A value is held to its kind's range. An `i32` field given `2**31` raises
`ConstructionError`, a `bool` is never taken for an integer, and an `int`
with no kind is an `i64`. An `f32` takes only a float it holds exactly:
`1.5` is one, and `0.1` is refused, since a `real` cannot hold it unchanged.
Round it yourself first, with `struct`, if a rounded value is what you mean.
`f64` keeps every float, infinities, `NaN` and `-0.0` included, and a `real`
read back widens exactly, but for a `NaN` whose payload a Python float cannot
carry, which raises `DecodeError`.

A `Decimal` never passes through a float or the active `decimal` context. It
holds up to 28 digits after the point, in a 96-bit coefficient, as Rust's
`Decimal` does. A trailing zero and a negative zero keep their sign and scale;
an exponent above zero is written out, so `Decimal("1E+2")` is `100`:

```python
from decimal import Decimal

import pgorm
from pgorm import Value

assert Value(Decimal("4.50")).value.as_tuple() == Decimal("4.50").as_tuple()
assert str(Value(Decimal("1E+2")).value) == "100"
assert Value(1.5, "f32").value == 1.5

for unrepresentable, kind in [
    (Decimal("NaN"), "decimal"),
    (Decimal("1E-29"), "decimal"),
    (Decimal("79228162514264337593543950336"), "decimal"),
    (0.1, "decimal"),
    (0.1, "f32"),
    (2**31, "i32"),
    (True, "i32"),
]:
    try:
        Value(unrepresentable, kind)
    except pgorm.ConstructionError:
        pass
    else:
        raise AssertionError(f"{unrepresentable!r} is not a {kind}")
```

A `numeric` that a `Decimal` cannot hold exactly, `NaN`, or an infinity
raises `DecodeError` when read. Give a `numeric` column a precision and
scale within those limits, such as `numeric(10, 2)` for prices, so every
value it holds reads back.

## Money

PostgreSQL's `money` stores an amount in the currency of the server's
`lc_monetary` setting. pgorm reads and writes it through `numeric`: the
column is read cast to `numeric`, each value is written cast to `money`, and
the server rounds a value to the currency's places.

A model declares the column `column_type="money"` and its field the
`"decimal"` kind, as a Rust entity declares `column_type = ColumnType::Money`
on a `Decimal` field, and the model's reads, writes and comparisons take
pgorm's casts for `money`:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Column, Model, Pool
from pgorm import schema as s

TipJar = Model("tip_jar", {
    "id": Column("i32", primary_key=True, auto_increment=True),
    "amount": Column("decimal", column_type="money"),
})


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await pool.execute(s.from_model(TipJar).table)

        tips = await TipJar.insert({"amount": Decimal("12.345")}).returning().one(pool)
        # Rounded to the currency's two places.
        assert tips["amount"] == Decimal("12.35")

        query = TipJar.find().where(TipJar.col("amount") > Decimal(10))
        assert query.inspect().sql == (
            'SELECT "tip_jar"."id", CAST("tip_jar"."amount" AS numeric) AS "amount" '
            'FROM "tip_jar" WHERE "tip_jar"."amount" > CAST($1::numeric AS money)'
        )
        assert [row["amount"] for row in await query.all(pool)] == [Decimal("12.35")]


asyncio.run(main())
```

A statement written with the [query builder](11-query-builder.md) takes the
same casts by hand: `col("amount").cast(TypeName("numeric"))` to read and
`bind(amount).cast(TypeName("money"))` to write. A Rust entity with a
`money` column, reached through a [registration](24-registrations.md), reads
it so too, and a binary [`COPY`](17-copy.md) of rows reads a `money` column
as a `Decimal`.

## JSON

A `json` field holds any JSON value, as the Python values the `json` module
reads: a `dict`, a `list`, a `str`, an `int`, a `float`, a `bool` or `None`.
The kind covers both column types. A `json` column keeps the text as written;
a `jsonb` column stores it decomposed, which is what PostgreSQL indexes and
searches. A field's `contains` is `@>`, whether a `jsonb` document holds the
given JSON.

When the JSON has a fixed shape, give it a type of its own, and convert at
the edge. Here Python differs from Rust: a Rust struct that derives
`JsonValueType` is stored as its JSON and read back as the struct, and a
Python model converts nothing but the JSON itself, so the program turns its
dataclass into a dictionary and back:

```python
import asyncio
import os
from dataclasses import asdict, dataclass

from pgorm import Column, Model, Pool, Value
from pgorm import schema as s


@dataclass
class Contact:
    phone: str
    languages: list[str]


Profile = Model("profile", {
    "baker_id": Column("i32", primary_key=True),
    "contact": Column("json"),
    "extra": Column("json"),
})


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await pool.execute(
            s.create_table("profile")
            .column(s.ColumnDef("baker_id", "integer").not_null())
            .column(s.ColumnDef("contact", "jsonb").not_null())
            .column(s.ColumnDef("extra", "json").not_null())
            .primary_key("baker_id")
        )

        ada = Contact(phone="+47 400 00 001", languages=["nb", "en"])
        bo = Contact(phone="+47 400 00 002", languages=["nb"])
        await Profile.insert_many([
            {"baker_id": 1, "contact": asdict(ada), "extra": {"apron": "L"}},
            # JSON's null is a value, so the NOT NULL column takes it.
            {"baker_id": 2, "contact": asdict(bo), "extra": Value.json(None)},
        ]).execute(pool)

        # `@>` asks whether the jsonb holds the given JSON.
        english = await Profile.find().where(
            Profile.col("contact").contains({"languages": ["en"]})
        ).all(pool)
        assert [Contact(**row["contact"]) for row in english] == [ada]

        bos = await Profile.find_by_key({"baker_id": 2}).one(pool)
        assert bos["extra"] is None
        assert not bos.tagged("extra").is_null


asyncio.run(main())
```

`None` given to a `json` field is SQL `NULL`, which only a nullable field
takes; JSON's `null` is `Value.json(None)`. Read back, both are `None`, and
the tagged value tells them apart, as the example's last line does. Inside a
document, `None` is always JSON's `null`; in an array of `json`,
`Value.json(None)` is an element holding JSON's `null`, and `None` a `NULL`
element.

JSON takes `None`, `bool`, `int`, a finite `float`, `str`, lists, tuples
(written as arrays) and dictionaries with string keys, each of exactly that
type. An integer must fit a signed or an unsigned 64-bit number. A document
that contains itself, or nests deeper than 64 levels, is refused. A
`Decimal`, a `UUID` or a `datetime` inside a document is refused too: choose
its JSON encoding, a string or a number, and convert it first.

A document read back follows the same number model: an integer within 64
bits is an `int`, any other number a `float`, and a number whose text a
`float` would change, such as `0.30000000000000001` or an integer beyond 64
bits, raises `DecodeError` rather than read back as another number. Read back
into Python, a `json` column's document keeps neither its key order nor its
whitespace.

## Arrays

A field declared `array=True` is an array column of its kind's type, for
every kind. `bytes` is not an array: it is `bytea`. An array compares as a
whole with `eq`, and the array operators ask about its elements: `contains`
is `@>` (holds every element of the value), `contained_by` is `<@`, and
`overlaps` is `&&` (shares an element). An array of an enum is an array of
the enum's type, and each label is checked against the field's `values`.

The other direction, a scalar column against a list of values, compares the
column with `any_(values)`, which binds the whole list as one array
parameter, so a statement has the same SQL however long the list is. `is_in`
binds one parameter per value instead.

```python
import asyncio
import os

from pgorm import Column, Model, Pool, TypeName, any_
from pgorm import schema as s

from bakery import FLAVOR, FLAVORS

Menu = Model("menu", {
    "id": Column("i32", primary_key=True, default=True),
    "name": Column("text"),
    "tags": Column("text", array=True),
    "flavors": Column(FLAVOR, array=True, values=FLAVORS),
})


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await pool.execute(
            s.create_table("menu")
            .column(s.ColumnDef("id", "integer").not_null().auto_increment())
            .column(s.ColumnDef("name", "varchar").not_null())
            .column(s.ColumnDef("tags", s.DataType("text").array()).not_null())
            .column(s.ColumnDef("flavors", s.DataType(TypeName("flavor")).array()).not_null())
            .primary_key("id")
        )

        await Menu.insert_many([
            {"name": "Breakfast", "tags": ["morning", "coffee"], "flavors": ["lemon", "vanilla"]},
            {"name": "Party", "tags": ["evening"], "flavors": ["chocolate"]},
        ]).execute(pool)

        with_coffee = await Menu.select("name").where(
            Menu.col("tags").contains(["coffee"])
        ).all(pool)
        assert [menu["name"] for menu in with_coffee] == ["Breakfast"]

        chocolate = await Menu.select("name").where(
            Menu.col("flavors").overlaps(["chocolate", "carrot"])
        ).all(pool)
        assert [menu["name"] for menu in chocolate] == ["Party"]

        named = Menu.select("name").where(Menu.col("name").eq(any_(["Party", "Picnic"])))
        assert named.inspect().sql == 'SELECT "menu"."name" FROM "menu" WHERE "menu"."name" = ANY($1)'
        assert [menu["name"] for menu in await named.all(pool)] == ["Party"]


asyncio.run(main())
```

Here Python differs from Rust in spelling. Rust's `includes` is `contains`,
its `eq_any(list)` is `eq(any_(list))`, and `ne_all(list)` is
`ne(all_(list))`.

An array's elements may be `NULL`: a list may hold `None`, read back as
`None`, and `[]`, the empty array, stays apart from a `NULL` array.
PostgreSQL arrays carry no dimension in their type, but pgorm reads one
dimension starting at 1: a list is a list. An array of more dimensions, or
one whose index starts elsewhere, raises `DecodeError`.

## Enums

An enum field's kind is the enum's `TypeName`, and its values are the labels,
as strings ([Models and entities](03-entities.md) declares the bakery's
`flavor`). pgorm writes each label cast to the enum's type, so a comparison
resolves:

```python
from bakery import Cake

compiled = Cake.select("name").where(Cake.col("flavor").eq("lemon")).inspect()
assert compiled.sql == (
    'SELECT "cake"."name" FROM "cake" '
    'WHERE "cake"."flavor" = CAST($1::text AS public.flavor)'
)
assert [value.value for value in compiled.params] == ["lemon"]
```

Here Python differs from Rust in two ways. A model's enum `TypeName` names
its schema, `TypeName("flavor", schema="public")`, since a row read back
names the type with its schema, so the type is written qualified everywhere,
as a Rust enum with `schema_name = ".."` is. And a model reads an enum
column as it is, where Rust reads it cast to `text`; the driver gives back
the label either way. A field that lists its enum's `values` refuses any
other label with `ConstructionError` as the statement is built, and a row
holding one raises `DecodeError`.

An enum stored as integers or as text, which Rust declares with `rs_type`
and `db_type`, is in Python a field of the kind it is stored as, `i32` or
`text`, with no cast. A Python `enum.Enum` member is not a value, a
`StrEnum` included: pass its `.value`.

## UUIDs

A `uuid` field is a `uuid` column, its values `uuid.UUID`. A key the database
generates is a column whose default is `uuidv7()`, PostgreSQL 18's
time-ordered UUIDs, which index well because each is greater than the last:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Column, Model, Pool, call
from pgorm import schema as s

Voucher = Model("voucher", {
    "code": Column("uuid", primary_key=True, default=True),
    "value": Column("decimal"),
})


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await pool.execute(
            s.create_table("voucher")
            .column(s.ColumnDef("code", "uuid").not_null().default(call("uuidv7")))
            .column(s.ColumnDef("value", "numeric").not_null())
            .primary_key("code")
        )

        async with pool.connection() as connection:
            first = await Voucher.insert({"value": Decimal(10)}).returning("code").one(connection)
            second = await Voucher.insert({"value": Decimal(10)}).returning("code").one(connection)

        assert first["code"].version == 7
        assert first["code"] < second["code"]


asyncio.run(main())
```

## Intervals

A `pgorm.Interval` is PostgreSQL's `interval`: a number of months, of days
and of microseconds, kept apart because a month has no fixed number of days
and a day no fixed number of microseconds across a daylight-saving change.
`Interval(months=0, days=0, microseconds=0)` builds one, each field an exact
`int` with its own sign, `months` and `days` within 32 bits and
`microseconds` within 64. pgorm never turns one field into another, so
`Interval(months=1)` and `Interval(days=30)` are two different values,
though PostgreSQL's `=` calls them equal:

```python
import asyncio
import os

from pgorm import Column, Interval, Model, Pool
from pgorm import schema as s

HOUR = 3_600_000_000

Recipe = Model("recipe", {
    "id": Column("i32", primary_key=True, default=True),
    "name": Column("text"),
    "proving": Column("interval"),
    "keeps_for": Column("interval"),
})


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await pool.execute(
            s.create_table("recipe")
            .column(s.ColumnDef("id", "integer").not_null().auto_increment())
            .column(s.ColumnDef("name", "varchar").not_null())
            .column(s.ColumnDef("proving", "interval").not_null())
            .column(s.ColumnDef("keeps_for", "interval").not_null())
            .primary_key("id")
        )

        sourdough = await Recipe.insert({
            "name": "Sourdough",
            "proving": Interval(microseconds=14 * HOUR),
            "keeps_for": Interval(days=3),
        }).returning().one(pool)

        assert sourdough["proving"] == Interval(microseconds=14 * HOUR)
        assert str(sourdough["keeps_for"]) == "+3 days"
        assert Interval(months=1) != Interval(days=30)

        slow = await (
            Recipe.find()
            .where(Recipe.col("proving") > Interval(microseconds=12 * HOUR))
            .count(pool)
        )
        assert slow == 1


asyncio.run(main())
```

An `Interval` is immutable and hashable, and its `months`, `days` and
`microseconds` read its fields. `str()` gives text PostgreSQL reads back as
the same value, and the interval with every field at its largest, and the one
with every field at its smallest, are PostgreSQL's `infinity` and
`-infinity`. A `datetime.timedelta` is not an interval, and is refused: it
has no months, and it keeps only its days signed, so `-1 hour` would arrive
as `-1 day +23 hours`. Build an `Interval` instead.

An interval column restricted to some fields, such as `interval HOUR TO
MINUTE`, is declared `column_type=pgorm.schema.DataType("interval",
fields="hour_to_minute")`, the fields in snake_case and a `precision` only
where the last field is seconds (`fields="day_to_second", precision=3`), as
Rust's `ColumnType::Interval(IntervalSpec::Fields(..))`; the same `DataType`
goes to `ColumnDef`. Its values read as any interval's do.

## Ranges and multiranges

A range is a span of values of one type, such as the dates a seasonal cake is
on sale. PostgreSQL has six built-in range types, and pgorm carries each as a
`pgorm.Range`, whose bounds are values of a kind:

| Kind | PostgreSQL | Bounds |
| --- | --- | --- |
| `int4range`, `int8range` | `int4range`, `int8range` | `i32`, `i64` |
| `numrange` | `numrange` | `decimal` |
| `daterange` | `daterange` | `date` |
| `tsrange`, `tstzrange` | `tsrange`, `tstzrange` | `datetime`, `datetime_utc` |

A range is either empty or has two bounds, each included, excluded or absent.
`Range(lower, upper, bounds="[)")` builds one: `[` and `]` include a bound,
`(` and `)` exclude it, and `None` is no bound, whose side includes nothing,
so its bracket is always `(` or `)`. `Range()` holds every value, and
`Range.empty()` none. A `pgorm.Multirange` is a set of ranges,
`Multirange(ranges)`, and is a sequence of them; each range kind has its
multirange, `datemultirange` for dates. The [Arrays](#arrays) operators work
on ranges too (`contains`, `contained_by`, `overlaps`), beside
`strictly_left_of`, `strictly_right_of`, `not_extend_right_of`,
`not_extend_left_of` and `adjacent_to`:

```python
import asyncio
import os
from datetime import date

from pgorm import Column, Model, Pool, Range, TypeName
from pgorm import schema as s

Season = Model("season", {
    "id": Column("i32", primary_key=True, default=True),
    "cake": Column("text"),
    "on_sale": Column("daterange"),
    "batch_sizes": Column("int4range"),
})


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await pool.execute(
            s.create_table("season")
            .column(s.ColumnDef("id", "integer").not_null().auto_increment())
            .column(s.ColumnDef("cake", "varchar").not_null())
            .column(s.ColumnDef("on_sale", "daterange").not_null())
            .column(s.ColumnDef("batch_sizes", "int4range").not_null())
            .primary_key("id")
        )

        gingerbread = await Season.insert({
            "cake": "Gingerbread",
            "on_sale": Range(date(2026, 11, 15), date(2027, 1, 7)),
            "batch_sizes": Range(12, 48, "[]"),
        }).returning().one(pool)

        # A discrete range comes back in PostgreSQL's canonical form: [12,49).
        assert gingerbread["batch_sizes"] == Range(12, 49)

        # What is on sale over the Christmas week?
        christmas = Range(date(2026, 12, 21), date(2026, 12, 28))
        on_sale = await Season.select("cake").where(
            Season.col("on_sale").overlaps(christmas)
        ).all(pool)
        assert [season["cake"] for season in on_sale] == ["Gingerbread"]


asyncio.run(main())
```

Here Python differs from Rust: the standard library's `a..b` and `a..=b` are
Rust's spelling of a range's bounds, and Python names them as a string. A
range's kind is never guessed from its bounds. A model's field gives it, and
anywhere else it is a `Value(range, "int4range")` of a named kind; a bound
that kind's element cannot hold is refused as a scalar of it is, and a
`numrange` bound read back is held to the exactness a `numeric` column is.

A range's `lower`, `upper` and `bounds` read it back, beside `is_empty`,
`lower_inc`, `upper_inc`, `lower_inf` and `upper_inf`. Ranges compare by
their bounds as written, so `Range(1, 5, "[]")` differs from `Range(1, 6)`
until the server has made it canonical. The empty range is a value of its
own and never a pair of bounds: `[5,5)` reads back as `Range.empty()`. A
multirange comes back sorted, with overlapping and adjacent ranges merged:

```python
from pgorm import Multirange, Range, Value

assert Range(None, 5, "[]").bounds == "(]"
assert Range() != Range.empty()
assert Range(1, 5).lower_inc and not Range(1, 5).upper_inc

weeks = Multirange([Range(5, 8), Range(1, 3), Range(2, 4)])
assert len(weeks) == 3 and weeks[0] == Range(5, 8)
value = Value(weeks, "int4multirange")
assert value.snapshot()["data"][0] == {"lower": "5", "upper": "8", "bounds": "[)"}
```

Written to a column, the same multirange reads back as
`Multirange([Range(1, 4), Range(5, 8)])`. A range's snapshot is
`{"empty": true}` or its two bounds, each written as its element kind is and
`None` for no bound, beside its brackets; a multirange's is a list of them.

## Range types the schema creates

A schema can create a range type of its own, over a type no built-in range
covers, such as `double precision`. Its kind is a `CreatedRange(name,
subtype, schema=...)`, which names the type and the kind of its bounds, and
a model's field declares it with the type's schema. The type itself is
created with `pgorm.schema`'s `create_type(..).as_range(..)`, one of the
[DDL builders](14-schema.md#the-ddl-builders).

The bakery's ovens each hold a span of temperatures:

```python
import asyncio
import os

from pgorm import Column, CreatedRange, Model, Pool, Range
from pgorm import schema as s

TEMPERATURE = CreatedRange("temperature", "f64", schema="public")

Oven = Model("oven", {
    "id": Column("i32", primary_key=True, default=True),
    "name": Column("text"),
    "heat": Column(TEMPERATURE),
})


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        create_type = s.create_type("temperature").as_range("double")
        assert create_type.inspect().sql == (
            'CREATE TYPE "temperature" AS RANGE (SUBTYPE = double precision)'
        )
        await pool.execute(create_type)
        await pool.execute(
            s.create_table("oven")
            .column(s.ColumnDef("id", "integer").not_null().auto_increment())
            .column(s.ColumnDef("name", "varchar").not_null())
            .column(s.ColumnDef("heat", TEMPERATURE).not_null())
            .primary_key("id")
        )

        deck = await Oven.insert({
            "name": "Deck oven",
            "heat": Range(180.0, 250.5),
        }).returning().one(pool)
        assert deck["heat"] == Range(180.0, 250.5)

        hot_enough = Oven.find().where(Oven.col("heat").overlaps(Range(240.0, 260.0)))
        assert hot_enough.inspect().sql.endswith(
            'WHERE "oven"."heat" && CAST($1::text AS public.temperature)'
        )
        assert await hot_enough.count(pool) == 1


asyncio.run(main())
```

Here Python differs from Rust: a Rust program names the type once, in a
newtype over `Range<T>` that derives `CreatedRange`, and uses the newtype as
the field; a Python program names it in the kind. The subtype is one of the
twelve a created range can range over: `i16`, `i32`, `i64`, `f32`, `f64`,
`decimal`, `text`, `date`, `time`, `datetime`, `datetime_utc` and `uuid`, and
each bound converts with that kind's limits.

A value of a created range type travels as its text, cast to the type by
name, as the example's SQL shows, and is read back from its binary form
through its subtype's own codec. A `str` in the type's text form, such as
`"[1.5,2.5)"`, is taken as well, read as PostgreSQL's range input reads it,
so whitespace inside the brackets is part of a bound. `literal(value)` writes
it into the SQL instead, as an escaped string cast to the type. Its
snapshot's `type` is `{"kind": "created_range", "name": ..., "schema": ...,
"subtype": ...}`, with the text as its `data`.

A column of one reads back as its own kind, named as the server names the
column's type, even when its subtype is `integer`: a range type `slot` over
`integer` reads as `CreatedRange("slot", "i32", schema=...)`, not as an
`int4range`, so the value writes back to its column. A range type pgorm
creates is continuous whatever its subtype, since its canonical function
would have to be written in C, so `[1,5]` reads back as written.

The multirange PostgreSQL creates beside it is a `CreatedMultirange`, named
by its own name: `temperature_multirange` here, or `floatmultirange` beside a
`floatrange`. The driver reports a column of one with no subtype to read its
binary form by, so a model reads such a field cast to `text` and parses the
text as its subtype, as Rust's entities do (text the subtype does not read
raises `DecodeError`). A query that reads one as it is, or an array of one,
raises `DecodeError`: select it cast to `text`, and give the text to
`Value(text, CreatedMultirange(...))`. `Value.array` of a created range or
multirange kind is an array of it, cast to the type's array where it is
written, and a model field of either kind can be an array, one of
multiranges read cast to `text[]`. The [pipeline](12-pipeline.md), like
SQL/JSON's `JsonDefault`, refuses a created range's value, as it refuses a
qualified enum's, having no place for its cast.

## Domains

A domain is a type with constraints: `CREATE DOMAIN grams AS integer CHECK
(VALUE > 0)`. Its values travel as the type it is over, so the field is that
type's kind, and the domain's checks then hold every write. `pgorm.schema`'s
`create_domain` makes one, and `domain_value()` is the `VALUE` its check is
written over:

```python
import asyncio
import os

import pgorm
from pgorm import Column, Model, Pool, TypeName, domain_value
from pgorm import schema as s

Ingredient = Model("ingredient", {
    "id": Column("i32", primary_key=True, default=True),
    "name": Column("text"),
    "per_batch": Column("i32"),
})


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        grams = s.create_domain("grams", "integer").check(domain_value() > 0)
        assert grams.inspect().sql == 'CREATE DOMAIN "grams" AS integer CHECK (VALUE > 0)'
        await pool.execute(grams)

        create = (
            s.create_table("ingredient")
            .column(s.ColumnDef("id", "integer").not_null().auto_increment())
            .column(s.ColumnDef("name", "varchar").not_null())
            .column(s.ColumnDef("per_batch", TypeName("grams")).not_null())
            .primary_key("id")
        )
        assert create.inspect().sql == (
            'CREATE TABLE "ingredient" ( "id" serial NOT NULL, "name" varchar NOT NULL, '
            '"per_batch" grams NOT NULL, PRIMARY KEY ("id") )'
        )
        await pool.execute(create)

        flour = await Ingredient.insert({"name": "Flour", "per_batch": 500}).returning().one(pool)
        assert flour["per_batch"] == 500

        try:
            await Ingredient.insert({"name": "Air", "per_batch": 0}).execute(pool)
        except pgorm.CheckViolationError as refused:
            assert refused.sqlstate == "23514"
        else:
            raise AssertionError("the domain refuses zero grams")


asyncio.run(main())
```

A model whose table it writes says the column is the domain with
`column_type=pgorm.schema.DataType.domain("grams", "integer")`, as a Rust
entity says `column_type = ColumnType::Domain { name, schema, base }`; the
field keeps the kind the domain is over. A domain over a domain decodes as
the type at the bottom.

A domain over an enum is read and written as the enum, so its field's kind is
the enum's `TypeName`, and its `column_type` the domain. PostgreSQL resolves
no comparison operator for such a domain, so the model compares its column
cast to the enum, as a Rust entity does, reads it as its text and writes each
label cast to the domain:

```python
from pgorm import Column, Model, TypeName
from pgorm import schema as s

FLAVOR = TypeName("flavor", schema="public")
SPECIAL = TypeName("special_flavor", schema="public")

Special = Model("special", {
    "id": Column("i32", primary_key=True, auto_increment=True),
    "flavor": Column(FLAVOR, values=["lemon", "vanilla"], column_type=s.DataType.domain(SPECIAL, FLAVOR)),
})

lemon = Special.find().where(Special.col("flavor") == "lemon").inspect()
assert lemon.sql == (
    'SELECT "special"."id", CAST("special"."flavor" AS text) AS "flavor" FROM "special" '
    'WHERE CAST("special"."flavor" AS public.flavor) = CAST($1::text AS public.flavor)'
)
```

An enum column's `column_type` must be a domain over that enum, any other a
`ConstructionError`. PostgreSQL cannot build a foreign key on such a column,
and [Schema generation](14-schema.md) refuses a relation over one when it
can tell.

## Network addresses, bytes and vectors

`ipnetwork` is an `inet` (an address, with an optional network mask) or a
`cidr`, and `mac_address` a `macaddr`. `bytes` is `bytea`, raw bytes.
`vector` is the `vector` type of the
[pgvector](https://github.com/pgvector/pgvector) extension, for embeddings,
and needs the extension installed in the database.

```python
import asyncio
import ipaddress
import os

from pgorm import Column, Model, Pool
from pgorm import schema as s

Till = Model("till", {
    "id": Column("i32", primary_key=True, default=True),
    "address": Column("ipnetwork"),
    "hardware": Column("mac_address"),
    "receipt_logo": Column("bytes"),
})


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await pool.execute(
            s.create_table("till")
            .column(s.ColumnDef("id", "integer").not_null().auto_increment())
            .column(s.ColumnDef("address", "inet").not_null())
            .column(s.ColumnDef("hardware", "macaddr").not_null())
            .column(s.ColumnDef("receipt_logo", "bytea").not_null())
            .primary_key("id")
        )

        front = await Till.insert({
            "address": "10.0.4.17/24",
            "hardware": bytes.fromhex("02005e100001"),
            "receipt_logo": b"\x89PNG",
        }).returning().one(pool)

        assert front["address"] == "10.0.4.17/24"
        assert ipaddress.ip_interface(front["address"]).network == ipaddress.ip_network("10.0.4.0/24")
        assert front["hardware"].hex(":") == "02:00:5e:10:00:01"
        assert front["receipt_logo"] == b"\x89PNG"


asyncio.run(main())
```

Here Python differs from Rust: Rust has `IpNetwork` and `MacAddress` types,
and Python carries an address as its text and a hardware address as its six
bytes. An address is written as an address with an optional `/` and prefix
length, keeping its host bits, and always reads back with its prefix length,
so `10.0.4.17` reads back as `10.0.4.17/32`; the standard `ipaddress` module
reads the text. A hardware address is exactly six bytes, and text such as
`"02:00:5e:10:00:01"` is refused.

A vector is a list (or tuple) of floats, each one a `real` holds exactly, and
reads back as a list. Its snapshot writes each element's bits:

```python
import pgorm
from pgorm import Value

embedding = Value([0.5, -1.25, 3.0], "vector")
assert embedding.value == [0.5, -1.25, 3.0]
assert embedding.snapshot()["data"] == ["3f000000", "bfa00000", "40400000"]

try:
    Value([0.1], "vector")
except pgorm.ConstructionError:
    pass
else:
    raise AssertionError("a real cannot hold 0.1 exactly")
```

## Newtypes

A Rust newtype over any of these types derives `ValueType` and then goes
wherever its inner type goes: as a field, as a bound value, as a decoded
column. A model's column says the same with two functions: `decode`, which
turns each value a record reads into the program's own, and `encode`, its
inverse, which turns the program's value into one of the kind before every
write and comparison converts it. A value `decode` refuses, by raising,
makes the read a `DecodeError`, never a record holding the value, which is how
a JSON column's documents are checked as they are read, as a Rust
`JsonValueType` field's are. `None` passes through neither. An enum stored as
numbers, Rust's `rs_type = i16, db_type = ColumnType::SmallInteger`, is a
column of that kind decoded as a Python `IntEnum`:

```python
import asyncio
import enum
import os
from dataclasses import dataclass
from typing import Any

import pgorm
from pgorm import Column, Model, Pool
from pgorm import schema as s


class Shelf(enum.IntEnum):
    TOP = 1
    BOTTOM = 2


@dataclass(frozen=True)
class Contact:
    name: str
    phone: str

    @staticmethod
    def read(document: Any) -> "Contact":
        return Contact(str(document["name"]), str(document["phone"]))

    def json(self) -> dict[str, str]:
        return {"name": self.name, "phone": self.phone}


Display = Model("display", {
    "id": Column("i32", primary_key=True),
    "shelf": Column("i16", decode=Shelf, encode=int),
    "supplier": Column("json", nullable=True, decode=Contact.read, encode=Contact.json),
})


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await pool.execute(s.from_model(Display).table)
        row = await Display.insert({
            "id": 1, "shelf": Shelf.TOP, "supplier": Contact("Flour & Co", "555-0100"),
        }).returning().one(pool)
        assert row["shelf"] is Shelf.TOP
        assert row["supplier"] == Contact("Flour & Co", "555-0100")

        tops = await Display.find().where(Display.col("shelf") == Shelf.TOP).all(pool)
        assert [top["id"] for top in tops] == [1]

        # A document without a phone is no Contact, and is refused as it is read.
        await pool.execute(pgorm.RawSQL(
            "INSERT INTO display (id, shelf, supplier) VALUES (2, 2, '{\"name\": \"Nobody\"}')"
        ))
        try:
            await Display.find_by_key({"id": 2}).one(pool)
        except pgorm.DecodeError:
            pass
        else:
            raise AssertionError("a contact without a phone was read")


asyncio.run(main())
```

Rust's `display` is the program's own spelling of the enum, its `str()`. A
type that only exists for the type checker needs neither function:
`typing.NewType` makes values of the inner type itself, so a `CakeId` is an
`int` wherever pgorm sees it. A subclass of a kind's type is refused as the
value is made, where no `encode` turns it into one. A Rust entity's newtype
fields reach a Python program through a [registration](24-registrations.md),
which reads and writes them with the Rust conversions in between.

## Custom reads and writes

Sometimes a column needs SQL around it on its way in or out, because pgorm
has no kind that reads it directly. A Rust field says so with five
attributes, and a model's column takes the same five: `select_as` and
`save_as` read the column cast to a type and write each value cast to one,
and `select_with` and `save_with` read and write through functions from an
expression to an expression, the value bound before `save_with` sees it,
with `save_array_with` beside `save_with` for the array `any_(..)` and
`all_(..)` compare the column with. As in Rust, `save_with` and
`save_array_with` come together, and either alone is a `ConstructionError`.
A comparison's value takes the column's write too, and so does a list
`any_(..)` or `all_(..)` ranges over, bound as one array: a `save_as`
column's array is cast to the array of its type, a `money` column's to
`money[]`, a domain over an enum's to the enum's array, as Rust's
`save_array_as` casts it. A type is a built-in type's name, a `TypeName`, or
a `DataType`, whose length or precision the cast writes: `bit(7)`.

The bakery keeps the days of the week it opens as a `bit(7)`, Monday first,
and the program reads and writes it as an integer. No kind reads `bit`, so
the column is read cast to `bigint`, and each value written cast to
`bit(7)`, as the Rust entity's `select_as = "bigint", save_as = "bit(7)"`:

```python
import asyncio
import os

from pgorm import Column, Model, Pool, any_
from pgorm import schema as s

DAYS = s.DataType("bit", length=7)

Opening = Model("opening", {
    "bakery_id": Column("i32", primary_key=True),
    "days": Column("i64", column_type=DAYS, select_as="bigint", save_as=DAYS),
})


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await pool.execute(s.from_model(Opening).table)

        # Monday to Saturday.
        sunrise = await Opening.insert({"bakery_id": 1, "days": 0b111_1110}).returning().one(pool)
        assert sunrise["days"] == 0b111_1110

        every_day = Opening.find().where(Opening.col("days") == 0b111_1111)
        assert every_day.inspect().sql == (
            'SELECT "opening"."bakery_id", CAST("opening"."days" AS bigint) AS "days" '
            'FROM "opening" WHERE "opening"."days" = CAST($1::int8 AS bit(7))'
        )
        assert await every_day.all(pool) == []

        some_days = Opening.find().where(Opening.col("days") == any_([0b111_1110, 0b000_0011]))
        assert some_days.inspect().sql.endswith(
            'WHERE "opening"."days" = ANY(CAST($1::int8[] AS bit(7)[]))'
        )
        assert [row["bakery_id"] for row in await some_days.all(pool)] == [1]


asyncio.run(main())
```

A statement written with the [query builder](11-query-builder.md) writes the
casts itself: `expr.cast(type)` is `select_as`, `bind(value).cast(type)` is
`save_as`, and a function of its own that returns an expression is
`select_with` or `save_with`. A cast there takes a `TypeName`, which has no
length, so a value bound for `bit(7)` goes through `varbit`, where a model's
`save_as` names `bit(7)` itself, and an array `any_(..)` compares with is
the statement's to cast as well. A type whose name needs quoting is a `TypeName`
like any other, and an array of it is `cast(type, array=True)`. Here is
Rust's shelf label, read as `text` and compared with a list of labels as one
array:

```python
from pgorm import Table, TypeName, Value, any_, bind, col, select

LABEL = TypeName("Shelf Label")
label = col("label", table="shelf")

query = (
    select(col("id", table="shelf"), label.cast(TypeName("text")).as_("label"))
    .from_(Table("shelf"))
    .where(label.eq(any_(bind(Value.array("text", ["top", "middle"])).cast(LABEL, array=True))))
)
assert query.inspect().sql == (
    'SELECT "shelf"."id", CAST("shelf"."label" AS text) AS "label" FROM "shelf" '
    'WHERE "shelf"."label" = ANY(CAST($1::text[] AS "Shelf Label"[]))'
)
```
