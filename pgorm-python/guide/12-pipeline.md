# The pipeline

A pipeline is a query written as a list of steps. Each step takes a relation
and returns a relation: keep these rows, add this column, group by that key,
sort, take the first ten. `pgorm.pipeline` compiles the steps to one
PostgreSQL statement, in the shape of a [PRQL] query. It is pgorm's own
pipeline, the Rust one, built from Python calls: the steps are compiled by
pgorm's PRQL compiler in the process, and the statement runs as any other.

Where each step lands in the SQL follows from where it stands in the list. A
filter before an aggregation becomes `WHERE`, a filter after one becomes
`HAVING`, and a filter after a window function wraps the steps before it in a
`WITH` query, because SQL cannot filter on a window in the same `SELECT`. You
write the steps in the order you think of them, and the compiler writes the
clauses.

Reach for a pipeline for reports and analysis: aggregates you filter on,
rankings, running totals, a relation built from other relations. A model's
`find()` reads its rows into records ([Reading rows](05-reading.md)); the
[query builder](11-query-builder.md) writes SQL clause by clause; a pipeline
sits between them, composing relations rather than clauses.

[PRQL]: https://prql-lang.org

## A first pipeline

`pl.from_(source)` starts a pipeline from a model, a `Table`, a registered
Rust entity or another pipeline, and each method appends one step, returning a new pipeline and
leaving the one before it as it was. `inspect()` compiles it, returning the
SQL and the values it binds:

```python
from pgorm import pipeline as pl

from bakery import Cake

name, price, gluten_free = pl.col(Cake, "name"), pl.col(Cake, "price"), pl.col(Cake, "gluten_free")

dearest = (
    pl.from_(Cake)
    .filter(gluten_free == pl.literal(True))
    .sort(price.desc())
    .take(2)
    .select(name, price)
)

compiled = dearest.inspect()
assert compiled.sql == "SELECT name, price FROM cake WHERE gluten_free = true ORDER BY price DESC LIMIT 2"
assert compiled.params == []
```

A model's column is `pl.col(model, field)`, as Rust's is the entity's own
`Column` variant: the field's column, qualified by the name the model is read
under, its alias or its table's, and written under the column's name in SQL,
which the compiler leaves unqualified where only one relation is read, as
above. It compares as the field's declared kind does. A value compared with
it, in a comparison, `coalesce` or `in_array`, converts through the field's
declaration as the model's own column converts one, so a label the
declaration does not list, or `None`, is a `ConstructionError` before
anything is sent, and it is bound cast as Rust casts a value compared with
an entity column of that kind, an enum's label to its enum:

```python
from pgorm import ConstructionError
from pgorm import pipeline as pl

from bakery import Cake

lemon = pl.from_(Cake).filter(pl.col(Cake, "flavor") == "lemon").select(pl.col(Cake, "name"))
assert lemon.inspect().sql == "SELECT name FROM cake WHERE flavor = CAST($1::text AS public.flavor)"
assert [value.value for value in lemon.inspect().params] == ["lemon"]

try:
    pl.col(Cake, "flavor") == "banana"
except ConstructionError:
    pass
else:
    raise AssertionError("a label the declaration does not list is refused")
```

A constant is written into the SQL with `pl.literal(value)`; [a plain value
is bound](#values-from-the-program) instead, where Rust writes a literal.
`pl.col(table, column)` names a column of a relation no model describes, by
the relation's name and the column's name in SQL, and carries no type
([below](#a-table-meeting-itself)).

Python's operators build the expressions: `==`, `!=`, `<`, `<=`, `>`, `>=`,
or the methods `eq` .. `lte`; `+`, `-`, `*`, `/` and `%`, and unary `-`; and
`&`, `|` and `~` for `and`, `or` and `not`, each comparison in parentheses.
An expression also has `asc()` and `desc()` for a sort, `is_null()` and
`is_not_null()`, `coalesce(value)`, and `in_array([..])` for membership in a
list of values.

A pipeline runs through its terminals. `all`, `one` and `optional` run it on
a pool, a connection or a transaction and return records: `all` every row,
`one` the first, a `DecodeError` when there is none, and `optional` the
first or `None`. `one` and `optional` append a `take(1)` first, as Rust's do,
and `inspect(terminal="one")` shows the SQL they send:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Pool
from pgorm import pipeline as pl

from bakery import Cake

name, price, gluten_free = pl.col(Cake, "name"), pl.col(Cake, "price"), pl.col(Cake, "gluten_free")


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        dearest = (
            pl.from_(Cake)
            .filter(gluten_free == pl.literal(True))
            .sort(price.desc())
            .take(2)
            .select(name, price)
        )
        rows = await dearest.all(pool)
        assert [(row["name"], row["price"]) for row in rows] == [
            ("Chocolate Torte", Decimal("6.00")),
            ("Lemon Drizzle", Decimal("3.20")),
        ]

        cheapest = pl.from_(Cake).sort(price).select(name)
        assert (await cheapest.one(pool))["name"] == "Vanilla Sponge"
        assert "LIMIT 1" in cheapest.inspect(terminal="one").sql
        assert await pl.from_(Cake).filter(price > Decimal(100)).optional(pool) is None


asyncio.run(main())
```

Rust's terminals decode each row into a type of the program's choosing, and
its `into_model` and `into_tuple` stage such a read. A Python pipeline stages
the same reads. `into_model(model)` reads each row as a model's record, each
field from the result column of its SQL name, the result's columns exactly
the model's and each held to its declaration, as the model's own queries
hold them. A column the model reads through a cast — a `money` column, a
domain over an enum, a `select_as` ([Values](13-values.md)) — is read cast,
the pipeline ending in a projection of the model's columns, as Rust's ends in
one for an entity's model; the pipeline's own `all`, `one` and `optional`
read the columns as the server sends them, so such a model is read through
`into_model` or `select_sources`. Given a function instead, such as a
dataclass, `into_model` calls it with each row's columns as keyword
arguments. `into_tuple()` reads each row as a
tuple of its values in column order, by position, so a result with two
columns of one name is read whole. Each gives a `PipelineSelector`, whose
`all`, `one` and `optional` run as the pipeline's own do, and whose
`inspect(terminal=)` shows what each sends. Rust's `all::<M>()` is
`into_model(M).all(db)`:

```python
import asyncio
import os
from dataclasses import dataclass
from decimal import Decimal

from pgorm import Pool
from pgorm import pipeline as pl

from bakery import Cake


@dataclass(frozen=True)
class Priced:
    name: str
    price: Decimal


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        gluten_free = (
            pl.from_(Cake)
            .filter(pl.col(Cake, "gluten_free") == pl.literal(True))
            .sort(pl.col(Cake, "price"))
        )

        cakes = await gluten_free.into_model(Cake).all(pool)
        assert [(cake["name"], cake["flavor"]) for cake in cakes] == [
            ("Vanilla Sponge", "vanilla"),
            ("Lemon Drizzle", "lemon"),
            ("Chocolate Torte", "chocolate"),
        ]

        priced = gluten_free.select(pl.col(Cake, "name"), pl.col(Cake, "price"))
        assert await priced.into_model(Priced).one(pool) == Priced("Vanilla Sponge", Decimal("2.90"))
        assert await priced.into_tuple().all(pool) == [
            ("Vanilla Sponge", Decimal("2.90")),
            ("Lemon Drizzle", Decimal("3.20")),
            ("Chocolate Torte", Decimal("6.00")),
        ]


asyncio.run(main())
```

## The steps

| Step | What it does | SQL it usually becomes |
| --- | --- | --- |
| `pl.from_` | the relation to start from | `FROM` |
| `filter` | keep the rows a condition holds for | `WHERE`, or `HAVING` after an aggregation |
| `derive` | add computed columns, keeping the others | a projection |
| `select` | replace the columns with these | a projection |
| `group` and `aggregate` | one row per key, with aggregates | `GROUP BY` |
| `window` | add columns computed over a window | `OVER (..)` |
| `sort` | order the rows | `ORDER BY` |
| `take`, `take_range` | keep the first rows, or a 1-based range of them | `LIMIT`, `OFFSET` |
| `join` | combine with another relation on a condition | `JOIN` |
| `append`, `intersect`, `remove` | combine with another relation's rows | `UNION ALL`, `INTERSECT ALL`, `EXCEPT ALL` |
| `distinct` | keep one copy of each row | `DISTINCT` |

A step that takes several expressions takes them as arguments:
`.select(name)`, `.select(name, city)`, `.sort(price.desc(), id)`. Here
Python differs from Rust, which takes one bare, an array or a tuple.
`group(*keys)` returns a grouped relation whose only step is
`aggregate(*columns)`; `group()` with no key aggregates the whole relation
into one row. `take_range(start, end)` keeps rows `start` to `end`, both
included, counting from 1.

## Names

A column a step introduces needs a name the later steps can refer to.
`pl.alias(name)` is that name: it is declared once, attached with `as_`, and
used from then on as an expression like any column. Grouping the cakes by
bakery and keeping the bakeries with more than two:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Pool
from pgorm import pipeline as pl

from bakery import Cake

bakery_id, price = pl.col("cake", "bakery_id"), pl.col("cake", "price")


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        cakes, mean = pl.alias("cakes"), pl.alias("mean_price")
        busy = (
            pl.from_(Cake)
            .group(bakery_id)
            .aggregate(pl.count_rows().as_(cakes), pl.average(price).as_(mean))
            .filter(cakes > pl.literal(2))
        )
        assert busy.inspect().sql == (
            "SELECT bakery_id, COUNT(*) AS cakes, AVG(price) AS mean_price FROM cake "
            "GROUP BY bakery_id HAVING COUNT(*) > 2"
        )

        rows = await busy.all(pool)
        assert len(rows) == 1
        assert (rows[0]["bakery_id"], rows[0]["cakes"]) == (1, 3)
        assert round(rows[0]["mean_price"], 2) == Decimal("4.57")


asyncio.run(main())
```

The aggregates are `pl.count_rows()` (`COUNT(*)`), `pl.count(expr)`
(`COUNT(expr)`, the rows it is not null on), `pl.count_distinct`, `pl.sum`,
`pl.min`, `pl.max`, `pl.average` and `pl.stddev`. `pl.case([(condition,
value), ..], otherwise)` writes a `CASE` with a mandatory fallback,
`pl.null()` is the `NULL` literal, and `cast(type)` converts to one of
`"smallint"`, `"integer"`, `"bigint"`, `"real"`, `"double"`, `"numeric"`,
`"text"`, `"boolean"`, `"date"`, `"timestamp"`, `"timestamptz"`,
`"interval"`, `"uuid"`, `"json"` and `"jsonb"`, the types Rust's `CastType`
names.

`derive` adds a column and keeps the rest. Each cake's tier and its flavor as
text, for the second and third cakes by id:

```python
import asyncio
import os

from pgorm import Pool
from pgorm import pipeline as pl

from bakery import Cake


def column(name: str) -> pl.PipelineExpr:
    return pl.col("cake", name)


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        tier, flavor = pl.alias("tier"), pl.alias("flavor_name")
        tiers = (
            pl.from_(Cake)
            .derive(
                pl.case([(column("price") >= pl.literal(4), pl.literal("premium"))], pl.literal("standard")).as_(tier),
                column("flavor").cast("text").as_(flavor),
            )
            .sort(column("id"))
            .take_range(2, 3)
            .select(column("name"), tier, flavor)
        )
        assert tiers.inspect().sql == (
            "WITH table_0 AS (SELECT name, CASE WHEN price >= 4 THEN 'premium' ELSE 'standard' END "
            "AS tier, CAST(flavor AS text) AS flavor_name, id FROM cake ORDER BY id LIMIT 2 OFFSET 1) "
            "SELECT name, tier, flavor_name FROM table_0 ORDER BY id"
        )

        rows = await tiers.all(pool)
        assert [(row["name"], row["tier"], row["flavor_name"]) for row in rows] == [
            ("Lemon Drizzle", "standard", "lemon"),
            ("Carrot Cake", "standard", "carrot"),
        ]


asyncio.run(main())
```

An alias that collides with a name PRQL itself defines, such as `sum`,
`first` or `from`, is refused when the pipeline compiles, because it would
hide the built-in from every later step:

```python
from pgorm import ConstructionError
from pgorm import pipeline as pl

from bakery import Cake

refused = pl.from_(Cake).group(pl.col("cake", "bakery_id")).aggregate(pl.count_rows().as_(pl.alias("sum")))
try:
    refused.inspect()
except ConstructionError as error:
    assert str(error) == "alias `sum` collides with a PRQL built-in name; choose another alias"
else:
    raise AssertionError("the alias `sum` is refused")
```

## Values from the program

A value that comes from the program at run time, such as a price a customer
typed, is bound as a parameter: the statement's text then stays the same
from one value to the next, so a connection prepares it once.
`pl.literal(value)` writes a constant into the SQL instead, quoted so that it
means exactly the value given: `None`, a `bool`, an `int` within a `bigint`,
a finite `float` or a `str`, plain or as a `Value` of that kind.

Here Python differs from Rust. In Rust a literal is written into the SQL and
binding is the step's `_with` form. In Python a plain operand — a `bool`, an
`int`, a `float`, a `str`, a `list`, a `dict` or a `Value` — is bound by the
step that takes it, its kind inferred as every value's is, so an `int` is a
`bigint` and `Value(10, "i32")` keeps the kind it names
([Values and types](13-values.md)). Only `pl.literal` writes one into the SQL.

Each step also has a `_with` form, `filter_with`, `derive_with`,
`select_with`, `group_with`, `aggregate_with`, `window_with`, `sort_with` and
`join_with`, which calls a function with the step's binder.
`binder.bind(value)` records a value and returns its `$N` placeholder, which
the function can use more than once, and it takes any value pgorm can bind, a
`Decimal` or a date among them. The function runs once, as the step is
built, and returns the step's expression, or a list of them:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Pool
from pgorm import pipeline as pl

from bakery import Cake

name, price, gluten_free = pl.col("cake", "name"), pl.col("cake", "price"), pl.col("cake", "gluten_free")


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        budget = Decimal("3.50")
        affordable = (
            pl.from_(Cake)
            .filter_with(lambda binder: price <= binder.bind(budget))
            .sort(price)
            .select(name)
        )
        compiled = affordable.inspect()
        assert compiled.sql == (
            "WITH table_0 AS (SELECT name, price FROM cake WHERE price <= $1) "
            "SELECT name FROM table_0 ORDER BY price"
        )
        assert [value.value for value in compiled.params] == [budget]
        assert [row["name"] for row in await affordable.all(pool)] == ["Vanilla Sponge", "Lemon Drizzle"]

        # A plain operand is bound by the step that takes it.
        plain = pl.from_(Cake).filter(gluten_free == True).select(name)
        assert plain.inspect().sql == "SELECT name FROM cake WHERE gluten_free = $1"
        assert [value.kind for value in plain.inspect().params] == ["bool"]


asyncio.run(main())
```

`None` has no kind to bind, so it is refused as an operand with
`ConstructionError`: write `pl.null()`, `is_null()`, or a `Value.null(kind)`.
A `Value` of an enum type is refused too, since the pipeline binds a value
bare and has no cast for a named type; a plain string compared with an enum
column is read as that enum by the server. A step that binds takes at most 32
expressions; one that binds nothing has no such limit. A partition or an
order in a window takes no value, bound or plain: write a column or a
`pl.literal`.

A bound placeholder belongs to the step that minted it. Here Python differs
from Rust, where the binder's lifetime makes a placeholder that leaves its
closure a compile error. In Python the binder and every expression holding
one of its placeholders end with the function: using one afterwards, in this
pipeline or another, raises `LifecycleError`, and so does mixing the
placeholders of two steps' functions:

```python
from pgorm import LifecycleError
from pgorm import pipeline as pl

from bakery import Cake

price = pl.col("cake", "price")
leaked: list[pl.PipelineExpr] = []


def under_budget(binder: pl.Binder) -> pl.PipelineExpr:
    budget = binder.bind(3)
    leaked.append(budget)
    return price <= budget


first = pl.from_(Cake).filter_with(under_budget)
assert [value.value for value in first.inspect().params] == [3]

try:
    pl.from_(Cake).filter(price > leaked[0])
except LifecycleError:
    pass
else:
    raise AssertionError("a placeholder cannot leave its step")
```

A `take` takes a number, not an expression: PRQL has no bound row count.

## Joins

`join(source, on, kind=Join.Inner)` takes the relation to join, the
condition, and which rows to keep: `Join.Inner`, the default, `Left`,
`Right` or `Full`. A model is a relation wherever one is taken, `join`,
`append`, `intersect` and `remove` included. A column carries the name of its
relation, so a condition across two is qualified as written:

```python
import asyncio
import os

from pgorm import Join, Pool
from pgorm import pipeline as pl

from bakery import Bakery, Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        cake_name = pl.alias("cake")
        in_bergen = (
            pl.from_(Cake)
            .join(Bakery, pl.col(Cake, "bakery_id") == pl.col(Bakery, "id"), kind=Join.Inner)
            .filter(pl.col(Bakery, "city") == pl.literal("Bergen"))
            .select(pl.col(Cake, "name").as_(cake_name), pl.col(Bakery, "name"))
            .sort(cake_name)
        )
        assert in_bergen.inspect().sql == (
            "SELECT cake.name AS cake, bakery.name FROM cake INNER JOIN bakery "
            "ON cake.bakery_id = bakery.id WHERE bakery.city = 'Bergen' ORDER BY cake"
        )

        rows = await in_bergen.all(pool)
        assert [(row["cake"], row["name"]) for row in rows] == [
            ("Carrot Cake", "Moonlight"),
            ("Vanilla Sponge", "Moonlight"),
        ]


asyncio.run(main())
```

A join with no projection after it selects every column of both relations,
and both have an `id` and a `name`. A record maps column names to values, so
a result holding one name twice raises `DecodeError`: select the columns you
want, each under a name of its own where two would meet.

To decode the models themselves rather than columns of your choosing, end
with `select_sources(*models)`, listing the relations the pipeline read: a
model for one read under its table's name, the aliased model for one read
under its alias. It projects each model's fields the way the model's own
queries read them, every source under a prefix of its own, `s0_`, `s1_` and
on, so two sources sharing a column name cannot meet, as Rust's projects each
entity's. Each row is a tuple with one `ModelRecord` per model listed, or
`None` where the row has none of that model's columns, as a right or full
join leaves a side absent; a model listed alone still gives a one-item
tuple. A step that replaces the sources' own columns, such as `select` or an
aggregation, leaves nothing to project per model, and is a
`ConstructionError` before anything is sent:

```python
import asyncio
import os

from pgorm import ConstructionError, Join, Pool
from pgorm import pipeline as pl

from bakery import Bakery, Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # A third bakery, which sells nothing yet.
        await Bakery.insert({"name": "Noon", "city": "Tromsø"}).execute(pool)

        menus = (
            pl.from_(Cake)
            .join(Bakery, pl.col(Cake, "bakery_id") == pl.col(Bakery, "id"), kind=Join.Right)
            .filter(pl.col(Bakery, "city") != "Oslo")
            .sort(pl.col(Bakery, "id"), pl.col(Cake, "id"))
            .select_sources(Cake, Bakery)
        )
        rows = await menus.all(pool)
        names = [(None if cake is None else cake["name"], None if bakery is None else bakery["name"]) for cake, bakery in rows]
        assert names == [
            ("Carrot Cake", "Moonlight"),
            ("Vanilla Sponge", "Moonlight"),
            (None, "Noon"),
        ]

        reshaped = pl.from_(Cake).select(pl.col(Cake, "name")).select_sources(Cake)
        try:
            reshaped.inspect()
        except ConstructionError:
            pass
        else:
            raise AssertionError("select_sources after a select is refused")


asyncio.run(main())
```

The [relational-read graph](07-relations.md) reads related models with
types that say which side may be missing; `select_sources` is for the reads
the graph does not make, such as a right join or models beside a computed
column.

### A table meeting itself

A relation can be read under a name of your own with
`pl.source(relation).named(name)`, which is how a table is joined to itself.
The name is then the relation's only name, and `pl.col(name, column)` writes
its columns. A model read under an alias, `Cake.as_("other")`, is the same
relation under that name, its columns still typed by its fields, as Rust's
`named` keeps an entity's. The pairs of cakes sold by the same bakery:

```python
import asyncio
import os

from pgorm import Pool
from pgorm import pipeline as pl

from bakery import Cake


Other = Cake.as_("other")


def cake(name: str) -> pl.PipelineExpr:
    return pl.col(Cake, name)


def other(name: str) -> pl.PipelineExpr:
    return pl.col(Other, name)


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        pairs = (
            pl.from_(Cake)
            .join(Other, cake("bakery_id") == other("bakery_id"))
            .filter(cake("id") < other("id"))
            .sort(cake("id"), other("id"))
            .select(cake("name").as_("earlier"), other("name").as_("later"))
        )
        rows = await pairs.all(pool)
        assert [(row["earlier"], row["later"]) for row in rows] == [
            ("Chocolate Fudge", "Lemon Drizzle"),
            ("Chocolate Fudge", "Chocolate Torte"),
            ("Lemon Drizzle", "Chocolate Torte"),
            ("Carrot Cake", "Vanilla Sponge"),
        ]


asyncio.run(main())
```

`pl.col(table, column)` writes a column of a table no model describes as
readily, and `pl.from_(Table(name, schema=schema))` reads a table in another
schema, which Rust spells `from_schema`. A relation read by its name, or by
a name `named` gave it, is what `pl.col`'s first argument names.

`pl.col(table, column)` carries no type, so a column whose type PostgreSQL
compares only as another needs saying so. A column whose domain is over an
enum compares only as that enum, and a `money` column compared with a
`Decimal` only as `money`; `column.compares_as(TypeName(name, schema=..))`
casts the column, and each value compared with it, to that type:

```python
from pgorm import Table, TypeName
from pgorm import pipeline as pl

mood = pl.col("lines", "mood").compares_as(TypeName("mood"))
query = pl.from_(Table("lines")).filter(mood == "glad")
assert query.inspect().sql == "SELECT * FROM lines WHERE CAST(mood AS mood) = CAST($1::text AS mood)"
```

A model's column, `pl.col(model, field)`, and a registered Rust entity's,
`pl.col(table, entity.col(name))`, know their types and compare as the
model's or the entity's own column does, so neither needs `compares_as`: a
model's `money` column takes each value cast to `money`, a domain over an
enum compares as the enum, and a `save_as` value is cast to its type.

## Windows

`window(*columns, over=window)` adds columns computed over a window of rows.
`pl.over()` is the whole relation; `.by(*keys)` partitions it, and
`.sort_by(*keys)` orders each partition. The window functions are
`pl.row_number()`, `pl.rank(expr)`, `pl.rank_dense(expr)`, `pl.lag(offset,
expr)`, `pl.lead(offset, expr)`, `pl.first(expr)` and `pl.last(expr)`, and
any aggregate. The dearest cake at each bakery:

```python
import asyncio
import os

from pgorm import Pool
from pgorm import pipeline as pl

from bakery import Cake

bakery_id, name, price = pl.col("cake", "bakery_id"), pl.col("cake", "name"), pl.col("cake", "price")


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        place = pl.alias("place")
        dearest = (
            pl.from_(Cake)
            .window(pl.row_number().as_(place), over=pl.over().by(bakery_id).sort_by(price.desc()))
            .filter(place == pl.literal(1))
            .sort(bakery_id)
            .select(bakery_id, name)
        )
        assert dearest.inspect().sql == (
            "WITH table_0 AS (SELECT bakery_id, name, ROW_NUMBER() OVER (PARTITION BY bakery_id "
            "ORDER BY price DESC) AS _expr_0 FROM cake) SELECT bakery_id, name FROM table_0 "
            "WHERE _expr_0 = 1 ORDER BY bakery_id"
        )

        rows = await dearest.all(pool)
        assert [(row["bakery_id"], row["name"]) for row in rows] == [
            (1, "Chocolate Torte"),
            (2, "Carrot Cake"),
        ]


asyncio.run(main())
```

A window can be narrowed to a frame: `rows(start, end)` counts rows from the
current one, and `range(start, end)` counts values. Each bound is an offset
from the current row, negative before it and positive after, and `None`
leaves that side unbounded. A running total of a bakery's prices, cheapest
first:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Pool
from pgorm import pipeline as pl

from bakery import Cake

bakery_id, name, price = pl.col("cake", "bakery_id"), pl.col("cake", "name"), pl.col("cake", "price")


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        running = pl.alias("running")
        rows = await (
            pl.from_(Cake)
            .filter(bakery_id == pl.literal(1))
            .window(pl.sum(price).as_(running), over=pl.over().by(bakery_id).sort_by(price).rows(None, 0))
            .sort(price)
            .select(name, running)
            .all(pool)
        )
        assert [(row["name"], row["running"]) for row in rows] == [
            ("Lemon Drizzle", Decimal("3.20")),
            ("Chocolate Fudge", Decimal("7.70")),
            ("Chocolate Torte", Decimal("13.70")),
        ]


asyncio.run(main())
```

## Composing pipelines

Because every step maps a relation to a relation, a function from `Pipeline`
to `Pipeline` is a reusable piece of a query, which any pipeline can apply:

```python
from pgorm import pipeline as pl

from bakery import Cake


def gluten_free(cakes: pl.Pipeline) -> pl.Pipeline:
    """The cakes a customer who avoids gluten can eat."""
    return cakes.filter(pl.col("cake", "gluten_free") == pl.literal(True))


query = gluten_free(pl.from_(Cake)).select(pl.col("cake", "name"))
assert query.inspect().sql == "SELECT name FROM cake WHERE gluten_free = true"
```

A whole pipeline is a relation too: it can be the source of another, the
relation a join reads, or the other side of a set operation. Embedding one
takes it whole, values and all; its placeholders are renumbered after the
consumer's, so the merged SQL and values still line up. Each Sunrise cake's
units sold, from the order lines, joined back to the cake's name:

```python
import asyncio
import os

from pgorm import Pool
from pgorm import pipeline as pl

from bakery import Cake, OrderLine


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        sold = pl.alias("sold")
        units = (
            pl.from_(OrderLine)
            .group(pl.col("order_line", "cake_id"))
            .aggregate(pl.sum(pl.col("order_line", "quantity")).as_(sold))
            .filter_with(lambda binder: sold >= binder.bind(1))
        )
        report = (
            pl.from_(Cake)
            .filter_with(lambda binder: pl.col("cake", "bakery_id") == binder.bind(1))
            .join(units, pl.col("cake", "id") == pl.alias("cake_id"))
            .sort(sold.desc())
            .select(pl.col("cake", "name"), sold)
        )
        assert [value.value for value in report.inspect().params] == [1, 1]

        rows = await report.all(pool)
        assert [(row["name"], row["sold"]) for row in rows] == [("Chocolate Fudge", 2), ("Lemon Drizzle", 1)]


asyncio.run(main())
```

An embedded pipeline's columns are referred to by their names, as
`pl.alias("cake_id")` above. Where a name exists on both sides of a join,
`pl.that(name)` names the joined relation's column and `pl.this(name)` the
pipeline's own.

`append` puts another relation's rows after this one's, matching columns by
position, and `distinct` after it makes the `UNION ALL` a `UNION`.
`intersect` keeps the rows both have, and `remove` the rows the other does
not; give both sides the same projection:

```python
import asyncio
import os

from pgorm import Pool
from pgorm import pipeline as pl

from bakery import Bakery, Customer


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        everyone = (
            pl.from_(Bakery)
            .select(pl.col("bakery", "name"))
            .append(pl.from_(Customer).select(pl.col("customer", "name")))
            .distinct()
        )
        assert everyone.inspect().sql == "SELECT name FROM bakery UNION DISTINCT SELECT name FROM customer"

        names = sorted(row["name"] for row in await everyone.all(pool))
        assert names == ["Kim", "Lee", "Moonlight", "Sunrise"]


asyncio.run(main())
```

A step after a combination reads the whole combined relation, as in Rust:
the combination is written as a `WITH` query first and the step reads it, so
`pl.from_(Cake).append(pl.source(Cake)).select(pl.col("cake", "name"))` projects the rows
of both operands, `WITH table_0 AS (SELECT * FROM cake UNION ALL SELECT *
FROM cake) SELECT name FROM table_0`, and a column named against an operand's
table still resolves after `intersect` or `remove`. A `distinct` directly
after `append` still folds into `UNION DISTINCT`, as above.

## When a pipeline fails

What can go wrong with a pipeline is mostly found when it compiles, in
`inspect()` or a terminal, before anything is sent, and comes back as a
`ConstructionError`:

- an alias that collides with a name PRQL defines;
- a table, column or alias name that cannot be written safely as a name in
  SQL: one holding a double quote or a NUL byte, beginning with `$`, or `*`
  (a `$` after the first character is an ordinary character);
- a pipeline the PRQL compiler refuses, such as a column read after a
  `select` that dropped it, with the compiler's own message.

Here Python differs from Rust, where building a pipeline cannot fail and
every error waits for `into_sql`. Some refusals come as a Python step is
built: an operand with no kind to bind, such as `None`; more than 32 bound
expressions in one step; a value in a window's partition or order; and a
placeholder used outside its step, a `LifecycleError`. An argument of the
wrong type, such as a `bool` for `take`, raises `TypeError` there.

```python
from pgorm import ConstructionError
from pgorm import pipeline as pl

from bakery import Cake

dropped = pl.from_(Cake).select(pl.col("cake", "name")).filter(pl.col("cake", "price") > pl.literal(1))
try:
    dropped.inspect()
except ConstructionError as error:
    assert "Unknown name `cake.price`" in str(error)
else:
    raise AssertionError("price was dropped by the select")

try:
    pl.from_(Cake).filter(pl.col("cake", "price") == None)
except ConstructionError:
    pass
else:
    raise AssertionError("None has no kind to bind")
```

A name no step introduced and no source describes is not caught here: the
compiler has no catalog, so it takes the name as a column of the source, and
the server answers for it when the statement runs, with a `DatabaseError`.

## Results

A pipeline is a statement like any other, so the pool's, a connection's and
a transaction's own methods run it too, with their own rules: `query`
returns all of its rows, and `one` and `optional`, which add no `LIMIT`,
refuse more than one row with `DecodeError`, where the pipeline's own `one`
and `optional` take the first. `stream` on a pool or a connection reads its
rows as they arrive ([Pages, cursors and streams](06-pagination.md)):

```python
import asyncio
import os

from pgorm import DecodeError, Pool
from pgorm import pipeline as pl

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        by_price = pl.from_(Cake).sort(pl.col("cake", "price")).select(pl.col("cake", "name"))

        assert len(await pool.query(by_price)) == 5
        try:
            await pool.one(by_price)
        except DecodeError:
            pass
        else:
            raise AssertionError("five rows are not one")

        async with await pool.stream(by_price) as rows:
            names = [row["name"] async for row in rows]
        assert names[:2] == ["Vanilla Sponge", "Lemon Drizzle"]


asyncio.run(main())
```

### Registered source tuples

Rust's `select_sources` projects each source's columns the way the entity's
own queries read them and decodes each row as a tuple of optional models.
Python reaches the same for Rust entities registered into an application
wheel ([Registering Rust entities](24-registrations.md)), when the
registration also registers the tuple of entities to decode, under a name.
`pl.sources(name)` finds that registration, and
`select_sources(selection, qualifiers=[..])` ends a pipeline over those
entities, the qualifiers naming the relation each entity is read by. Its
`all`, `one` and `optional` run on a pool, a connection or a transaction,
and each row is a tuple with one `EntityModel` or `None` per entity, since a
right or full join can leave either side absent; for a single source,
`(None,)` is a row whose source was absent, and `None` from `optional` is no
row. A step that replaces the sources' own columns, such as `select` or an
aggregation, leaves nothing to project per source, and is refused. A
registration holds one to six entities; `pl.source_tuples()` lists the names
registered, and a selection's `describe()` reports its name and its
entities. The bakery's registration in this guide registers no source tuple,
and the standalone wheel none at all, where `pl.sources(name)` raises
`UnsupportedCapabilityError`.

For a query known whole when the program is written, Rust's `prql!` macro
compiles PRQL text at build time instead; Python has no such macro
([Raw SQL](10-raw-sql.md)).
