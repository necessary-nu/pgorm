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

Here Python differs from Rust: there is no `add_option`. A condition that may
be absent, such as an optional field of a search form, is left out of the list
with ordinary Python before the list goes to `Condition.all`. An empty
`Condition.all()` always holds and an empty `Condition.any()` never does.

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
conditions = [Cake.col("bakery_id") == 1]
if max_price is not None:
    conditions.append(Cake.col("price") <= max_price)
search = Cake.select("name").where(Condition.all(*conditions))
assert search.inspect().sql == 'SELECT "cake"."name" FROM "cake" WHERE "cake"."bakery_id" = $1'
```

## Ordering, limits and offsets

`order_by` orders by one or more orderings, in the order they are given and
then in the order later calls add them. A column's or an expression's `asc()`
and `desc()` make an ordering, and their `nulls=Nulls.First` or
`nulls=Nulls.Last` says where `NULL`s go. `limit` and `offset` cut the result.

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

Here Python differs from Rust: a model query has no `distinct` or
`distinct_on`. The statement builder's `Select` has `distinct()`
([Beyond the model builders](#beyond-the-model-builders)); `DISTINCT ON` is
not available in Python yet. [Joins](#joins) below shows a subquery that does
the work a `distinct` does after a join.

A query read in pages is better served by [Pages, cursors and
streams](06-pagination.md) than by `offset`.

## Choosing columns

`select(*fields)` starts a query that reads only the fields it names, where
`find()` reads them all. A list computed at run time goes in with `*`, as any
Python argument list does. The rows are still the model's records, holding
just the fields chosen, in the order they were named, and
`tuple(row.values())` reads them as a tuple.

Here Python differs from Rust. In Rust, choosing columns leaves the rows
without the `Model`'s shape, so the query needs `into_tuple`, `into_model` or
`into_values` to say what to decode into. A Python model query never loses its
shape: it can choose only among the model's fields, and decodes them as the
model declares. A computed item, an aggregate, or a column of another table
beside the model's (Rust's `column_as` and `expr_as`) is the statement
builder's, as the second example shows.

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Pool, Select, call

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        names = await Cake.select("name").order_by(Cake.col("id").asc()).all(pool)
        assert dict(names[0]) == {"name": "Chocolate Fudge"}

        priced = await (
            Cake.select("name", "price")
            .where(Cake.col("gluten_free") == True)
            .order_by(Cake.col("price").asc())
            .all(pool)
        )
        assert tuple(priced[0].values()) == ("Vanilla Sponge", Decimal("2.90"))

        flavors = (
            Select(Cake.col("flavor"), call("count", Cake.col("id")).as_("cakes"))
            .from_(Cake.table)
            .group_by(Cake.col("flavor"))
            .order_by(Cake.col("flavor").asc())
        )
        assert flavors.inspect().sql == (
            'SELECT "cake"."flavor", COUNT("cake"."id") AS "cakes" FROM "cake" '
            'GROUP BY "cake"."flavor" ORDER BY "cake"."flavor" ASC'
        )
        counted = [tuple(row.values()) for row in await pool.query(flavors)]
        assert counted[0] == ("chocolate", 2)


asyncio.run(main())
```

A model's `table` and its columns go into the statement builder's `Select`
as they are, and `as_(name)` names an item there. A `Select` runs through the
pool's (or a connection's, or a transaction's) own `query`, `one` and
`optional`, the same strict terminals a model query has, and its rows are
plain records keyed by result column, which the next section describes. [The
query builder](11-query-builder.md) covers it in full.

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

Here Python differs from Rust, which decodes a row straight into a struct of
your own: one deriving `FromQueryResult`, which reads columns by name, or one
deriving `PartialModel`, which also says which columns to select. Python has
neither derive. Its answers are these two:

- A model is what Rust's `PartialModel` is: a declaration of the columns to
  select and how to read them. A second model of the same table, declaring
  only the fields a view of it needs, is the partial model. A field named
  differently from its column says so with `Column(kind, name="column")`, and
  the model's queries select the column under the field's name, so the query
  and the record cannot disagree. A model field is always a column of its
  table: an expression such as `upper(name)`, Rust's `from_expr`, is not
  available as a model field in Python yet.
- Any record goes into a type of your own as keyword arguments, `Tag(**row)`,
  which suits a dataclass whose fields are named for the columns, as a
  `FromQueryResult` struct's are. For a computed column, select it with the
  statement builder under the name the type expects.

```python
import asyncio
import os
from dataclasses import dataclass
from decimal import Decimal

from pgorm import Column, Model, Pool, Select, call

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


@dataclass(frozen=True)
class MenuLine:
    shout: str
    free_from_gluten: bool


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        moonlight = (
            PriceList.select("name", "euros")
            .where(PriceList.col("bakery_id") == 2)
            .order_by(PriceList.col("name").asc())
        )
        assert moonlight.inspect().sql == (
            'SELECT "cake"."name", "cake"."price" AS "euros" FROM "cake" '
            'WHERE "cake"."bakery_id" = $1 ORDER BY "cake"."name" ASC'
        )
        tags = [PriceTag(**row) for row in await moonlight.all(pool)]
        assert tags[0] == PriceTag(name="Carrot Cake", euros=Decimal("3.80"))

        # A computed column comes from the statement builder, named for the field it fills.
        lemon = (
            Select(
                call("upper", Cake.col("name")).as_("shout"),
                Cake.col("gluten_free").as_("free_from_gluten"),
            )
            .from_(Cake.table)
            .where(Cake.col("id") == 2)
        )
        assert lemon.inspect().sql == (
            'SELECT UPPER("cake"."name") AS "shout", "cake"."gluten_free" AS "free_from_gluten" '
            'FROM "cake" WHERE "cake"."id" = $1'
        )
        row = await pool.one(lemon)
        assert MenuLine(**row) == MenuLine(shout="LEMON DRIZZLE", free_from_gluten=True)

        assert row.tagged("shout").kind == "text"
        assert [field.name for field in row.fields] == ["shout", "free_from_gluten"]
        assert row.fields[1].type_name.name == "bool"


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
joined, `INNER JOIN` by default and `LEFT JOIN` with `kind=Join.Left`.
`alias=` joins the table under another name, for a table joined twice. A
condition or an ordering may then name the joined model's columns.

Here Python differs from Rust. Rust's `inner_join_rel`, `left_join_rel` and
`right_join_rel` follow a relation of the queried entity, and their `_rev`
forms follow one declared on the other entity. A Python relation is a value
that goes one way, from the model it was declared on, so a join from the other
side uses a relation declared from that side: [`bakery.py`](bakery.py)
declares both `cake_bakery` and `bakery_cakes` for that reason. A model join is
inner or left; `Join.Right` and `Join.Full` are the statement builder's.

A join adds rows to the query, not columns: the records stay the queried
model's. Here Python differs from Rust too, where `select` can add the joined
table's columns beside the model's. In Python, a joined table's columns beside
the model's are the statement builder's, as the example below reads them, and
reading each row together with its related row, each as a record of its own
model, is the graph's work, which [Relations](07-relations.md) covers.

```python
import asyncio
import os

from pgorm import Pool, Select

from bakery import Bakery, Cake, bakery_cakes, cake_bakery


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        menu = (
            Cake.select("name")
            .join(cake_bakery)
            .where(Cake.col("gluten_free") == True)
            .order_by(Bakery.col("city").asc(), Cake.col("name").asc())
        )
        assert menu.inspect().sql == (
            'SELECT "cake"."name" FROM "cake" '
            'INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" '
            'WHERE "cake"."gluten_free" = $1 ORDER BY "bakery"."city" ASC, "cake"."name" ASC'
        )
        names = [cake["name"] for cake in await menu.all(pool)]
        assert names == ["Vanilla Sponge", "Chocolate Torte", "Lemon Drizzle"]

        # The bakery's city beside each cake's name: the statement builder's.
        with_city = (
            Select(Bakery.col("city"), Cake.col("name"))
            .from_(Cake.table)
            .join(Bakery.table, Cake.col("bakery_id") == Bakery.col("id"))
            .where(Cake.col("gluten_free") == True)
            .order_by(Bakery.col("city").asc(), Cake.col("name").asc())
        )
        assert with_city.inspect().sql == (
            'SELECT "bakery"."city", "cake"."name" FROM "cake" '
            'INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" '
            'WHERE "cake"."gluten_free" = $1 ORDER BY "bakery"."city" ASC, "cake"."name" ASC'
        )
        rows = [tuple(row.values()) for row in await pool.query(with_city)]
        assert rows == [
            ("Bergen", "Vanilla Sponge"),
            ("Oslo", "Chocolate Torte"),
            ("Oslo", "Lemon Drizzle"),
        ]

        # The bakeries that sell a chocolate cake, read from the bakery's side.
        joined = Bakery.find().join(bakery_cakes).where(Cake.col("name").starts_with("Chocolate"))
        assert [bakery["name"] for bakery in await joined.all(pool)] == ["Sunrise", "Sunrise"]

        # Sunrise sells two, so the join reads it twice. A subquery reads each bakery once.
        chocolate = Select(Cake.col("bakery_id")).from_(Cake.table).where(
            Cake.col("name").starts_with("Chocolate")
        )
        shops = await Bakery.find().where(Bakery.col("id").is_in(chocolate)).all(pool)
        assert [bakery["name"] for bakery in shops] == ["Sunrise"]


asyncio.run(main())
```

In Rust, the join followed by `distinct()` reads each bakery once. A Python
model query has no `distinct`, and the `IN (SELECT ..)` above finds the same
bakeries.

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

Here Python differs from Rust: a model query has no `group_by` or `having`,
and a column has no aggregate methods. Grouping is the statement builder's:
`Select` takes `group_by` and `having`, and `call` makes the aggregates,
`call("count", ..)`, `"sum"`, `"min"`, `"max"`, `"avg"` and
`"count_distinct"` among them. The join is written with its `ON` condition,
since a relation joins a model query only.

```python
import asyncio
import os

from pgorm import Pool, Select, call

from bakery import Bakery, Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        cakes = call("count", Cake.col("id"))
        busy = (
            Select(Bakery.col("name"), cakes.as_("cakes"))
            .from_(Cake.table)
            .join(Bakery.table, Cake.col("bakery_id") == Bakery.col("id"))
            .group_by(Bakery.col("name"))
            .having(cakes > 2)
        )
        assert busy.inspect().sql == (
            'SELECT "bakery"."name", COUNT("cake"."id") AS "cakes" FROM "cake" '
            'INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" '
            'GROUP BY "bakery"."name" HAVING COUNT("cake"."id") > $1'
        )
        rows = [tuple(row.values()) for row in await pool.query(busy)]
        assert rows == [("Sunrise", 3)]

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

Here Python differs from Rust: a model query has no lock methods, where
Rust's `Select` has `lock_exclusive`, `lock_shared` and `lock_with_behavior`.
The statement builder's `Select.lock(strength, wait=.., of=..)` writes every
form: `LockStrength.Update`, `NoKeyUpdate`, `Share` and `KeyShare`, and
`LockWait.Nowait` or `LockWait.SkipLocked`.

```python
import asyncio
import os

from pgorm import LockStrength, LockWait, Pool, Select

from bakery import Order


async def main() -> None:
    next_order = (
        Select(Order.col("id"))
        .from_(Order.table)
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
            taken = await tx.optional(next_order)
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
