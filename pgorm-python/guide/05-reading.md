# Reading rows

Every read starts from a model's `find()`, which builds a `ModelQuery`: a
`SELECT` of every field of the table, each row decoded into a record of the
model. The builder methods narrow it, order it, choose other fields or join
other tables, and a terminal runs it. This chapter reads the bakery's cakes,
bakers and orders. It is the Python dialect of the Rust guide's [Reading
rows](../../docs/guide/04-reading.md).

A query is a value. Each builder method returns a new query and leaves the one
it was called on as it was, so a query can be kept and refined in several
directions. Here Python differs from Rust only in spelling: a Rust `Select` is
consumed by each method and cloned to be reused.

## Finding rows

A model query has three terminals that read rows, and `count`, which
[Grouping and counting](#grouping-and-counting) covers:

| Terminal | Returns |
| --- | --- |
| `all` | every row, as a `list` of records |
| `one` | the only row; no row, or more than one, raises `DecodeError` |
| `optional` | the only row, or `None` when there is none; more than one raises `DecodeError` |

Each runs on a pool, a connection or a transaction. `find_by_key` filters on
the primary key, and takes the key as a mapping of every key field, so a
composite key names each of its parts:

```python
import asyncio
import os

from pgorm import DecodeError, Pool

from bakery import Cake, CakeBaker


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        cakes = await Cake.find().all(pool)
        assert len(cakes) == 5

        torte = await Cake.find_by_key({"id": 5}).one(pool)
        assert torte["name"] == "Chocolate Torte"

        try:
            await Cake.find_by_key({"id": 99}).one(pool)
        except DecodeError as error:
            assert str(error) == "expected exactly one row, received 0"
        else:
            raise AssertionError("cake 99 does not exist")
        assert await Cake.find_by_key({"id": 99}).optional(pool) is None

        # Bo bakes the Lemon Drizzle.
        link = await CakeBaker.find_by_key({"cake_id": 2, "baker_id": 2}).optional(pool)
        assert link is not None


asyncio.run(main())
```

Here Python differs from Rust. Rust's `one` and `one_opt` add `LIMIT 1` to
the query and take the first row of however many it finds, `one` failing with
`RecordNotFound` only when there is none. A model query's `one` and `optional`
add nothing to the query and check how many rows came back: exactly one for
`one`, at most one for `optional`. A query that can find several rows, such as
"the dearest cake", says so with `limit(1)`:

```python
import asyncio
import os

from pgorm import DecodeError, Pool

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        by_price = Cake.find().order_by(Cake.col("price").desc())
        try:
            await by_price.one(pool)
        except DecodeError as error:
            assert str(error) == "expected exactly one row, received 5"
        else:
            raise AssertionError("five cakes are not one")

        dearest = await by_price.limit(1).one(pool)
        assert dearest["name"] == "Chocolate Torte"


asyncio.run(main())
```

Rust's own reading is there in Python too, for an entity a Rust crate declares
and a Python program registers ([Registering Rust
entities](24-registrations.md)). A registered entity's `one` and `optional` are
Rust's `one` and `one_opt`: they narrow the query's limit to one row, `LIMIT
1` even where an earlier limit was larger and `LIMIT 0` where it was zero, and
`inspect(terminal="one")` shows that statement. Its rows are typed models,
read by attribute:

```python registered
import asyncio
import os

from pgorm import DecodeError, Pool
from pgorm.rust_bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        by_price = Cake.find().order_by(Cake.col("price").desc())
        compiled = by_price.inspect(terminal="one")
        assert compiled.sql == (
            'SELECT "cake"."id", "cake"."name", CAST("cake"."flavor" AS text), '
            '"cake"."price", "cake"."gluten_free", "cake"."bakery_id" FROM "cake" '
            'ORDER BY "cake"."price" DESC LIMIT $1'
        )
        assert [value.value for value in compiled.params] == [1]

        dearest = await by_price.one(pool)
        assert dearest.name == "Chocolate Torte"

        missing = Cake.find().where(Cake.col("id") == 99)
        assert await missing.optional(pool) is None
        try:
            await missing.one(pool)
        except DecodeError:
            pass
        else:
            raise AssertionError("cake 99 does not exist")


asyncio.run(main())
```

## Filtering

`where` adds a condition to the `WHERE` clause, and several `where` calls
must all hold. A condition is usually built from a column: `Cake.col("price")`
is the model's `price` field, a `ModelColumn`, whose methods and Python's
operators build conditions:

| Methods | SQL |
| --- | --- |
| `eq`, `ne`, `gt`, `gte`, `lt`, `lte`, or `==`, `!=`, `>`, `>=`, `<`, `<=` | `=`, `<>`, `>`, `>=`, `<`, `<=` |
| `between`, `not_between` (`symmetric=True` for `BETWEEN SYMMETRIC`) | `BETWEEN a AND b` |
| `is_in`, `is_not_in` over a list or tuple | `IN (..)` against a list of values |
| `== any_(values)`, `!= all_(values)` | `= ANY($1)`, `<> ALL($1)` against one array parameter |
| `is_in`, `is_not_in` over a `Select` | `IN (SELECT ..)` |
| `is_null`, `is_not_null` | `IS NULL`, `IS NOT NULL` |
| `is_distinct_from`, `is_not_distinct_from` | `IS [NOT] DISTINCT FROM`, which compares `NULL` as a value |
| `like`, `not_like`, `ilike`, `not_ilike` | `LIKE` and `ILIKE` against a pattern you write |
| `starts_with`, `ends_with`, `contains_text` | a prefix, suffix or substring test, in which `%` and `_` are plain characters |
| any comparison against another column or an expression | the comparison, the operand written as it stands |
| `contains`, `contained_by`, `overlaps`, ... | the range and array operators ([Values and types](13-values.md)) |

Each value is converted through the field's declared kind and bound as a
parameter. A value compared with an enum column is cast to the enum's type, a
label the column does not list is refused before anything is sent, and `None`
is refused too: a comparison with `NULL` is never true, so test it with
`is_null()`. `any_(..)` and `all_(..)` are the exception: their list is bound
as it stands, one array, which PostgreSQL reads as an array of the column's
type.

Here Python differs from Rust in four places. Rust's value methods take
values only, and comparing two columns is `eq_col`, comparing with an
expression `eq_expr`; in Python the same methods take either, and convert and
cast a value but write a column or an expression as it stands. Rust's
`eq_any` and `ne_all` convert the list to an array of the column's kind and
cast it as `eq` casts a value; a model column's `any_` and `all_` leave that
to PostgreSQL, as above. Rust's
`starts_with`, `ends_with` and `contains` write `LIKE` with `%` and `_`
escaped; Python's `starts_with` writes PostgreSQL's `starts_with(..)`,
`ends_with` a comparison with `right(..)`, and `contains_text` a test on
`strpos(..)`, which find the same rows. And a column's `contains` in Python is
range and array containment, `@>`; the text test is `contains_text`.

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Pool

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        chocolate_under_five = (
            Cake.find()
            .where(Cake.col("flavor") == "chocolate")
            .where(Cake.col("price") < Decimal("5.00"))
        )
        compiled = chocolate_under_five.inspect()
        assert compiled.sql == (
            'SELECT "cake"."id", "cake"."name", "cake"."flavor", "cake"."price", '
            '"cake"."gluten_free", "cake"."bakery_id" FROM "cake" '
            'WHERE "cake"."flavor" = CAST($1::text AS public.flavor) AND "cake"."price" < $2'
        )
        assert [value.value for value in compiled.params] == ["chocolate", Decimal("5.00")]
        found = await chocolate_under_five.all(pool)
        assert [cake["name"] for cake in found] == ["Chocolate Fudge"]

        mid_priced = await (
            Cake.find()
            .where(Cake.col("price").between(Decimal("3.00"), Decimal("4.00")))
            .count(pool)
        )
        assert mid_priced == 2

        named = await (
            Cake.find()
            .where(Cake.col("name").starts_with("Chocolate"))
            .where(Cake.col("id").is_in([1, 2, 3]))
            .all(pool)
        )
        assert len(named) == 1


asyncio.run(main())
```

The enum's cast names its schema, `public.flavor`, because
[`bakery.py`](bakery.py) declares the column's type as
`TypeName("flavor", schema="public")`. Rust's entity names the type as
`flavor`, which finds the same type. A column itself is qualified by its
table's name, or by the alias the model is under, never by its schema, as
Rust qualifies an entity's columns.

A model column is itself an expression, so it goes wherever an expression
goes, and every other expression method on it, `+`, `cast` or `as_` among
them, reads it as written. `Cake.col("price").expr()` gives the plain
expression, whose comparisons bind a value as any expression's do, with no
conversion through the field.

### Any and all

`Condition.any(..)` joins conditions with `OR`, and `Condition.all(..)` with
`AND`. Each takes any number of conditions, adds more through `add`, nests
inside the other, and goes to `where` like a single condition. Between two
conditions, `|` is `OR`, `&` is `AND` and `~` is `NOT`; Python's precedence
binds them tighter than a comparison, so each comparison needs its
parentheses.

`add_option` adds a condition that may be absent, as Rust's does: given
`None` it leaves the condition as it is, which suits a search form whose
fields are optional. An empty `Condition.all()` always holds and an empty
`Condition.any()` never does.

```python
from decimal import Decimal

from pgorm import Condition

from bakery import Cake

lemon_or_gluten_free = Cake.select("name").where(
    Condition.any(Cake.col("flavor") == "lemon", Cake.col("gluten_free") == True)
)
assert lemon_or_gluten_free.inspect().sql == (
    'SELECT "cake"."name" FROM "cake" '
    'WHERE "cake"."flavor" = CAST($1::text AS public.flavor) OR "cake"."gluten_free" = $2'
)

# The same condition written with the operators.
with_operators = Cake.select("name").where(
    (Cake.col("flavor") == "lemon") | (Cake.col("gluten_free") == True)
)
assert with_operators.inspect().sql == lemon_or_gluten_free.inspect().sql

# A search with an optional upper price: no condition at all when it is absent.
max_price: Decimal | None = None
search = Cake.select("name").where(
    Condition.all(Cake.col("bakery_id") == 1).add_option(
        None if max_price is None else Cake.col("price") <= max_price
    )
)
assert search.inspect().sql == 'SELECT "cake"."name" FROM "cake" WHERE "cake"."bakery_id" = $1'
```

## Ordering, limits and offsets

`order_by` orders by one or more orderings, in the order they are given and
then in the order later calls add them. A column's or an expression's `asc()`
and `desc()` make an ordering, and their `nulls=Nulls.First` or
`nulls=Nulls.Last` says where `NULL`s go. `limit` and `offset` cut the result,
and `distinct()` and `distinct_on(*columns)` drop repeated rows.

```python
import asyncio
import os

from pgorm import Nulls, Pool

from bakery import Cake, Customer


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        two_dearest = (
            Cake.find()
            .order_by(Cake.col("price").desc(), Cake.col("name").asc())
            .limit(2)
        )
        names = [cake["name"] for cake in await two_dearest.all(pool)]
        assert names == ["Chocolate Torte", "Chocolate Fudge"]

        third = await (
            Cake.find().order_by(Cake.col("price").desc()).offset(2).limit(1).one(pool)
        )
        assert third["name"] == "Carrot Cake"

        # Kim has no notes; NULLS FIRST puts Kim first.
        by_notes = Customer.find().order_by(Customer.col("notes").asc(nulls=Nulls.First))
        assert by_notes.inspect().sql.endswith('ORDER BY "customer"."notes" ASC NULLS FIRST')
        assert [customer["name"] for customer in await by_notes.all(pool)] == ["Kim", "Lee"]


asyncio.run(main())
```

`distinct_on` keeps the first row of each run of rows equal on its columns,
so the ordering decides which: here the dearest cake of each flavor. Its
columns are the queried model's or a joined table's, as Rust's are, and the
statement builder's `Select` has the same `distinct()` and `distinct_on`:

```python
import asyncio
import os

from pgorm import Pool

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        dearest = (
            Cake.select("flavor", "name")
            .distinct_on(Cake.col("flavor"))
            .order_by(Cake.col("flavor").asc(), Cake.col("price").desc())
        )
        assert dearest.inspect().sql == (
            'SELECT DISTINCT ON ("cake"."flavor") "cake"."flavor", "cake"."name" FROM "cake" '
            'ORDER BY "cake"."flavor" ASC, "cake"."price" DESC'
        )
        names = [cake["name"] for cake in await dearest.all(pool)]
        assert names == ["Chocolate Torte", "Vanilla Sponge", "Lemon Drizzle", "Carrot Cake"]


asyncio.run(main())
```

A query read in pages is better served by [Pages, cursors and
streams](06-pagination.md) than by `offset`.

## Choosing columns

`select(*fields)` starts a query that reads only the fields it names, where
`find()` reads them all. A list computed at run time goes in with `*`, as any
Python argument list does. The rows are still the model's records, holding
just the fields chosen, in the order they were named.

A query also becomes a *projection* of items the program chooses, as Rust's
`select` and `select_only` make one. `select_only()` clears the fields, and
`column`, `columns`, `column_as` and `expr_as` add items: a column of the
queried model or of a table the query joins, read under its field's name or
the name `column_as` gives it and decoded as its field declares; or an
expression under a name, decoded as the `Column` given beside it declares. A
model query takes the item methods directly too, its fields staying the first
items. A projection reads rows once one of three methods says what each row
becomes, as in Rust:

- `into_tuple()` reads the items' values in order into a tuple.
- `into_values(*names)` reads the named items' values, in the order named.
- `into_model(target)` calls `target` with each item's value as the keyword
  argument of its name: a dataclass whose fields are named for the items, as
  a `FromQueryResult` struct is, or `dict`.

Each gives a `ModelSelector`, Rust's `Selector`, whose `all`, `one`,
`optional`, `stream`, `count`, `cursor` and `paginate` read as a model
query's do. A model query also takes `into_tuple`, `into_values` and
`into_model` directly, reading its fields.

Here Python differs from Rust in where an expression's type is stated. Rust
states it in the type the rows decode into, `Vec<(Flavor, i64)>`; Python
states it beside the expression, `Column("i64")` for a `COUNT`, which
PostgreSQL types `bigint`. A row is held to its items as a record is to its
model: a column of another type, a `NULL` an item does not declare
`nullable`, or a label it does not list, is a `DecodeError`. A Rust column's
`count()` is `call("count", column)` here, and `"sum"`, `"min"`, `"max"`,
`"avg"` and `"count_distinct"` are called the same way.

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Column, Pool, call

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        names = await Cake.select("name").order_by(Cake.col("id").asc()).into_tuple().all(pool)
        assert names[0] == ("Chocolate Fudge",)

        priced = await (
            Cake.select("name", "price")
            .where(Cake.col("gluten_free") == True)
            .order_by(Cake.col("price").asc())
            .into_tuple()
            .all(pool)
        )
        assert priced[0] == ("Vanilla Sponge", Decimal("2.90"))

        flavors = (
            Cake.select("flavor")
            .expr_as(call("count", Cake.col("id")), "cakes", Column("i64"))
            .group_by(Cake.col("flavor"))
            .order_by(Cake.col("flavor").asc())
        )
        assert flavors.inspect().sql == (
            'SELECT "cake"."flavor", COUNT("cake"."id") AS "cakes" FROM "cake" '
            'GROUP BY "cake"."flavor" ORDER BY "cake"."flavor" ASC'
        )
        counted = await flavors.into_tuple().all(pool)
        assert counted[0] == ("chocolate", 2)
        by_name = await flavors.into_values("cakes", "flavor").all(pool)
        assert by_name[0] == (2, "chocolate")


asyncio.run(main())
```

`select_only()` with `column` and `columns` builds a list computed at run
time, as Rust's does. The statement builder's `Select`
([Beyond the model builders](#beyond-the-model-builders)) takes a model's
`table` and its columns too, for what a projection does not cover, and its
rows are plain records keyed by result column, which the next section
describes. [The query builder](11-query-builder.md) covers it in full.

## Decoding into your own types

Every row comes back as a record, a read-only mapping that owns its data and
outlives the connection it came over. A model's rows are `ModelRecord`s,
keyed by field in the order the query selects them. Any other statement's rows are
`Record`s, keyed by result column in the statement's order. Both read like a
dictionary: `row["name"]`, `get`, `keys`, `values`, `items`, iteration and
`dict(row)` give ordinary Python values. `row.tagged("name")` gives one value
as a `Value`, which keeps what a Python value cannot: the column's kind, so
SQL `NULL` stays apart from JSON `null` and an `int4` from an `int8`. A
`Record`'s `fields`, and a model record's `native.fields`, describe each
column as a `Field`: its `name`, its `index`, its PostgreSQL `type_name` and
`type_oid`, and the `table_oid` and `column_id` it was read from when the
server reports them. [Values and types](13-values.md) lists how each
PostgreSQL type reads.

A model record holds each field to its declaration: a column of another type,
a `NULL` in a field that is not nullable, or an enum label the field does not
list raises `DecodeError` rather than giving a value of the wrong kind. A
nullable field reads `NULL` as `None`. So does a `Record`: the columns of a
`LEFT JOIN` that matched nothing read `None`, and `tagged` keeps each one's
type. Two result columns of one name raise
`DecodeError` too, rather than one key for both: give each projection a name
of its own with `as_`.

Rust decodes a row straight into a struct of your own: one deriving
`FromQueryResult`, which reads columns by name, or one deriving
`PartialModel`, which also says which columns and expressions to select.
Python's counterparts are a projection's `into_model` and a `PartialModel`:

- `into_model(target)` reads each row by name into `target`, called with each
  item's value as a keyword argument: a dataclass whose fields are named for
  the items, as a `FromQueryResult` struct's are.
- `PartialModel(model, fields)` declares what to select and how to read it.
  Each field is one of the model's columns, `Cake.col("gluten_free")`, which
  is Rust's `from_col`, a field named for its column being Rust's plain
  field; an expression, `FromExpr(expr, Column(kind))`, Rust's `from_expr`,
  decoded as the `Column` declares; or nothing, `Skip(value)`, Rust's `skip`.
  A model query's `into_partial_model(shape)` selects what the shape declares,
  the query's conditions and ordering kept, and decodes each row into a
  read-only mapping of the shape's fields, in declaration order.

Here Python differs from Rust in what a skipped field holds. Rust takes the
type's `Default`; Python has no default of a type to take, so `Skip` holds
the value it is given in every row.

A second model of the same table, declaring only the fields a view of it
needs, reads a shape of plain columns too, and writes as well: a field named
differently from its column says so with `Column(kind, name="column")`, and
the model's queries select the column under the field's name.

```python
import asyncio
import os
from dataclasses import dataclass
from decimal import Decimal

from pgorm import Column, FromExpr, Model, PartialModel, Pool, Skip, call

from bakery import Cake

# A line of the price list: the cake table's name and price, the price read
# as "euros".
PriceList = Model("cake", {
    "name": Column("text"),
    "euros": Column("decimal", name="price"),
    "bakery_id": Column("i32"),
})


@dataclass(frozen=True)
class PriceTag:
    name: str
    euros: Decimal


# A cake on the menu board, its name shouted.
MenuLine = PartialModel(Cake, {
    "shout": FromExpr(call("upper", Cake.col("name")), Column("text")),
    "free_from_gluten": Cake.col("gluten_free"),
    "featured": Skip(False),
})


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        tags = await (
            Cake.select("name")
            .column_as(Cake.col("price"), "euros")
            .where(Cake.col("bakery_id") == 2)
            .order_by(Cake.col("name").asc())
            .into_model(PriceTag)
            .all(pool)
        )
        assert tags[0] == PriceTag(name="Carrot Cake", euros=Decimal("3.80"))

        # The same price list from the second model: its records go into the
        # dataclass as keyword arguments.
        moonlight = (
            PriceList.select("name", "euros")
            .where(PriceList.col("bakery_id") == 2)
            .order_by(PriceList.col("name").asc())
        )
        assert moonlight.inspect().sql == (
            'SELECT "cake"."name", "cake"."price" AS "euros" FROM "cake" '
            'WHERE "cake"."bakery_id" = $1 ORDER BY "cake"."name" ASC'
        )
        assert [PriceTag(**row) for row in await moonlight.all(pool)] == tags

        lemon = Cake.find().where(Cake.col("id") == 2).into_partial_model(MenuLine)
        assert lemon.inspect().sql == (
            'SELECT UPPER("cake"."name") AS "shout", "cake"."gluten_free" AS "free_from_gluten" '
            'FROM "cake" WHERE "cake"."id" = $1'
        )
        line = await lemon.one(pool)
        assert dict(line) == {"shout": "LEMON DRIZZLE", "free_from_gluten": True, "featured": False}


asyncio.run(main())
```

Declaring such a model creates nothing in the database and checks nothing
against it: each field names a column the table must have, and a mistake
shows when a query runs. It is a model like any other, so it writes too,
touching only the fields it declares ([Writing rows](04-writing.md)).

## Joins

A join follows a relation the models declare ([Models and
entities](03-entities.md)). `join(relation)` joins the table at the far end of
a relation that starts at the queried model, or at a table the query already
joined: `INNER JOIN` by default, and `kind=Join.Left`, `Join.Right` or
`Join.Full` for the others, as Rust's `inner_join_rel`, `left_join_rel` and
`right_join_rel` do. `join_rev(relation)` follows a relation declared on the
other model from its far end, as Rust's `_rev` forms do: from the bakery,
`join_rev(cake_bakery)` joins the cakes along the cake's `belongs_to`.
`alias=` joins the table under another name, for a table joined twice. A
condition or an ordering may then name the joined model's columns.

A join adds rows to the query, not columns: the records stay the queried
model's until a projection chooses columns of the joined table too. A row a
right or full join reads with no match holds `NULL` in the queried model's
columns, which its records refuse where a field is not nullable, so such a
join usually goes with a projection. Reading each row together with its
related row, each as a record of its own model, is the graph's work, which
[Relations](07-relations.md) covers.

```python
import asyncio
import os

from pgorm import Pool

from bakery import Bakery, Cake, cake_bakery


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        menu = (
            Cake.find()
            .join(cake_bakery)
            .select_only()
            .columns(Bakery.col("city"), Cake.col("name"))
            .where(Cake.col("gluten_free") == True)
            .order_by(Bakery.col("city").asc(), Cake.col("name").asc())
        )
        assert menu.inspect().sql == (
            'SELECT "bakery"."city", "cake"."name" FROM "cake" '
            'INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" '
            'WHERE "cake"."gluten_free" = $1 ORDER BY "bakery"."city" ASC, "cake"."name" ASC'
        )
        rows = await menu.into_tuple().all(pool)
        assert rows == [
            ("Bergen", "Vanilla Sponge"),
            ("Oslo", "Chocolate Torte"),
            ("Oslo", "Lemon Drizzle"),
        ]

        # The bakeries that sell a chocolate cake, read from the bakery's side.
        chocolate_shops = (
            Bakery.find()
            .join_rev(cake_bakery)
            .where(Cake.col("name").starts_with("Chocolate"))
            .distinct()
        )
        assert chocolate_shops.inspect().sql == (
            'SELECT DISTINCT "bakery"."id", "bakery"."name", "bakery"."city" FROM "bakery" '
            'INNER JOIN "cake" ON "cake"."bakery_id" = "bakery"."id" '
            'WHERE STARTS_WITH("cake"."name", $1)'
        )
        assert [bakery["name"] for bakery in await chocolate_shops.all(pool)] == ["Sunrise"]


asyncio.run(main())
```

A model joined to itself needs a second name for the table. `as_(alias)` gives
the model under an alias, whose columns are qualified by it, and a relation to
the aliased model joins the table under that alias. Here each cake meets the
other cakes of its flavor:

```python
import asyncio
import os

from pgorm import Pool

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        twin = Cake.as_("twin")
        same_flavor = Cake.has_many(twin, from_="flavor", to="flavor")
        query = (
            Cake.select("name")
            .join(same_flavor)
            .where(twin.col("id") != Cake.col("id"))
            .order_by(Cake.col("name").asc())
        )
        assert query.inspect().sql == (
            'SELECT "cake"."name" FROM "cake" '
            'INNER JOIN "cake" AS "twin" ON "cake"."flavor" = "twin"."flavor" '
            'WHERE "twin"."id" <> "cake"."id" ORDER BY "cake"."name" ASC'
        )
        names = [cake["name"] for cake in await query.all(pool)]
        assert names == ["Chocolate Fudge", "Chocolate Torte"]


asyncio.run(main())
```

## Grouping and counting

`count` on any model query counts its rows. It runs `SELECT COUNT(*) AS
"num_items" FROM (..) AS "sub_query"` over the query with its limit, offset
and ordering dropped, as Rust's paginator counts, and
`inspect(terminal="count")` shows that statement.

`group_by` groups the rows and `having` filters the groups, as Rust's do,
and an aggregate, `call("count", ..)`, `"sum"`, `"min"`, `"max"`, `"avg"` or
`"count_distinct"`, summarizes each group as an item `expr_as` names and
declares ([Choosing columns](#choosing-columns)). A grouped model query's
records stay the model's, which suits a query grouped by its primary key.

```python
import asyncio
import os

from pgorm import Column, Pool, call

from bakery import Bakery, Cake, cake_bakery


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        cakes = call("count", Cake.col("id"))
        busy = (
            Cake.find()
            .join(cake_bakery)
            .select_only()
            .column(Bakery.col("name"))
            .expr_as(cakes, "cakes", Column("i64"))
            .group_by(Bakery.col("name"))
            .having(cakes > 2)
        )
        assert busy.inspect().sql == (
            'SELECT "bakery"."name", COUNT("cake"."id") AS "cakes" FROM "cake" '
            'INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" '
            'GROUP BY "bakery"."name" HAVING COUNT("cake"."id") > $1'
        )
        assert await busy.into_tuple().all(pool) == [("Sunrise", 3)]

        gluten_free = Cake.find().where(Cake.col("gluten_free") == True)
        assert gluten_free.inspect(terminal="count").sql == (
            'SELECT COUNT(*) AS "num_items" FROM (SELECT "cake"."id", "cake"."name", '
            '"cake"."flavor", "cake"."price", "cake"."gluten_free", "cake"."bakery_id" '
            'FROM "cake" WHERE "cake"."gluten_free" = $1) AS "sub_query"'
        )
        assert await gluten_free.count(pool) == 3


asyncio.run(main())
```

## Locking rows

Inside a [transaction](08-transactions.md), a `SELECT .. FOR UPDATE` reads
rows so that no other transaction can change them until this one ends, and
`FOR SHARE` reads them so that none can delete or update them. `NOWAIT`
refuses to wait for a row another transaction holds, and `SKIP LOCKED` passes
over such rows, which suits a queue of work that several workers take from.

A model query's `lock(strength, wait=.., of=..)` is the statement
builder's: `LockStrength.Update`, `NoKeyUpdate`, `Share` or `KeyShare`, and
`LockWait.Nowait` or `LockWait.SkipLocked`. Here Python differs from Rust only
in spelling: Rust's `lock_exclusive()` is `lock(LockStrength.Update)`, its
`lock_shared()` is `lock(LockStrength.Share)`, and its `lock_with_behavior`
is `lock` with `wait`. The locked rows still decode as the model's records.

```python
import asyncio
import os

from pgorm import LockStrength, LockWait, Pool

from bakery import Order


async def main() -> None:
    next_order = (
        Order.select("id")
        .order_by(Order.col("placed_at").asc())
        .limit(1)
        .lock(LockStrength.Update, wait=LockWait.SkipLocked)
    )
    assert next_order.inspect().sql == (
        'SELECT "order"."id" FROM "order" ORDER BY "order"."placed_at" ASC '
        'LIMIT $1 FOR UPDATE SKIP LOCKED'
    )

    async with Pool(os.environ["DATABASE_URL"]) as pool:
        async with pool.transaction() as tx:
            taken = await next_order.optional(tx)
            assert taken is not None and taken["id"] == 1


asyncio.run(main())
```

## Beyond the model builders

A model query covers the everyday reads. A model is also a table and its
columns, which the [query builder](11-query-builder.md) takes as they are, as
the examples above use them: common table expressions, window functions,
lateral joins and grouping sets are written there. Here Python differs from
Rust only in where the statement starts: Rust's `QueryTrait::query` hands a
`Select`'s statement to the query builder, and in Python the statement is
written with the builder from the model's `table` and columns. SQL written by
hand runs as `RawSQL` ([Raw SQL](10-raw-sql.md)), and [the
pipeline](12-pipeline.md) composes queries in another style.

A statement built there, or SQL written by hand, still decodes into a model's
records. `ModelRecord(model, row, fields)` reads a `Record` as the model's
record of the fields named, every field when the list is empty. The row's
columns must be exactly those fields' columns, in order, each of its field's
kind, or it raises `DecodeError`:

```python
import asyncio
import os

from pgorm import DecodeError, ModelRecord, Pool, RawSQL

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        rows = await pool.query(RawSQL("SELECT * FROM cake WHERE bakery_id = $1 ORDER BY id", [2]))
        cakes = [ModelRecord(Cake, row, ()) for row in rows]
        assert [cake["name"] for cake in cakes] == ["Carrot Cake", "Vanilla Sponge"]
        assert cakes[0].model is Cake

        priced = await pool.one(RawSQL("SELECT name, price FROM cake WHERE id = $1", [1]))
        assert dict(ModelRecord(Cake, priced, ["name", "price"]))["name"] == "Chocolate Fudge"

        # An int8 where the model declares an i32 is refused, not converted.
        wide = await pool.one(RawSQL("SELECT id::int8 AS id FROM cake WHERE id = $1", [1]))
        try:
            ModelRecord(Cake, wide, ["id"])
        except DecodeError as error:
            assert str(error) == "value type differs from the declared model column"
        else:
            raise AssertionError("an int8 is not an i32")


asyncio.run(main())
```
