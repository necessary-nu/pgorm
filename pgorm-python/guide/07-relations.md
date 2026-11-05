# Relations

A relation says how two tables' rows belong together: a cake belongs to a
bakery, a bakery has many cakes, a cake has many bakers through the
`cake_baker` table. [Models and entities](03-entities.md) declares them with a
model's `belongs_to`, `has_one` and `has_many`, each pairing fields of the two
models, and [`bakery.py`](bakery.py) names each relation for where it starts
and what it reaches: `cake_bakery` goes from a cake to its bakery,
`bakery_cakes` from a bakery to its cakes. This chapter reads along them. It
is the Python dialect of the Rust guide's
[Relations](../../docs/guide/06-relations.md).

Here Python differs from Rust. A Rust entity declares its relations on its
`Relation` enum, and `Related<E>` says how to reach entity `E`, through a
junction table where there is one. A Python relation is a value of its own,
one hop from one model to another in one direction; a path of more hops, a
junction's included, is written as its hops.

pgorm offers three ways to read along a relation, one for each shape of
question:

- one record's related rows: `relation.find(row)`, a query of its own;
- the related rows of a list of records, in one query rather than one per
  record: `relation.load(db, rows)`;
- rows read together with their related rows, in one joined query: the graph,
  `Model.graph()`.

## The related rows of one record

`relation.find(row)` is a query for the rows at the far end of the relation
whose paired fields equal the row's. The row is a record of the model the
relation starts at, or any mapping holding its fields, and a row whose key
field is `None` finds nothing. The query is an ordinary model query, so it can
be filtered and ordered before it runs:

```python
import asyncio
import os

from pgorm import Pool

from bakery import Bakery, Cake, bakery_cakes


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        sunrise = await Bakery.find_by_key({"id": 1}).one(pool)
        cakes = await bakery_cakes.find(sunrise).order_by(Cake.col("price").asc()).all(pool)
        assert [cake["name"] for cake in cakes] == ["Lemon Drizzle", "Chocolate Fudge", "Chocolate Torte"]


asyncio.run(main())
```

Here Python differs from Rust, where a cake's `Related<baker::Entity>` names
the junction in its `via`, so `find_related` joins through it. A Python
relation is one hop, and the bakers of a cake are a query of the bakers that
joins the junction and the cake. `bakery.py` declares the relation from a
junction row to its baker; the join from the baker's side uses the relation
declared from that side, here written beside the query:

```python
from bakery import Baker, Cake, CakeBaker, cake_baker_cake

baker_cake_bakers = Baker.has_many(CakeBaker, from_="id", to="baker_id")

lemon_drizzle_bakers = (
    Baker.find()
    .join(baker_cake_bakers)
    .join(cake_baker_cake)
    .where(Cake.col("id") == 2)
    .order_by(Baker.col("name").asc())
)
assert lemon_drizzle_bakers.inspect().sql == (
    'SELECT "baker"."id", "baker"."name", "baker"."hired_on", "baker"."bakery_id" FROM "baker" '
    'INNER JOIN "cake_baker" ON "baker"."id" = "cake_baker"."baker_id" '
    'INNER JOIN "cake" ON "cake_baker"."cake_id" = "cake"."id" '
    'WHERE "cake"."id" = $1 ORDER BY "baker"."name" ASC'
)
```

Run, it reads Ada and Bo, the two bakers of the Lemon Drizzle.

## Longer paths

A path of more than one relation is a chain of joins on a model query, each
joining a relation from a table the query already reads. Here a baker's
customers are the people who ordered a cake the baker bakes, read from the
customer's side: from each customer through their orders, the orders' lines
and the lines' cakes to the cakes' bakers.

Here Python differs from Rust, where the path is a `Linked` type listing the
relations in order and `find_linked` reads along it from one model:

```python
import asyncio
import os

from pgorm import Pool

from bakery import Baker, CakeBaker, Customer, cake_bakers, customer_orders, order_line_cake, order_lines


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        cy = await Baker.find_by_key({"id": 3}).one(pool)
        baked_for = (
            Customer.find()
            .join(customer_orders)
            .join(order_lines)
            .join(order_line_cake)
            .join(cake_bakers)
            .where(CakeBaker.col("baker_id") == cy["id"])
        )
        customers = await baked_for.all(pool)
        assert [customer["name"] for customer in customers] == ["Lee"]


asyncio.run(main())
```

A customer who ordered two of the baker's cakes is read once for each, as the
joins make them. Rust's chain joins each hop under an alias of its own, `r0`,
`r1` and so on, so it may pass through one table twice. A Python join names
each table by its own name unless given `alias=`, and a relation starts from
the first table of its model the query reads, so a chain that continues from
a second visit to one table is not available in Python yet. Rust's
`RelatedLink`, the chain a `Related` impl describes, has no counterpart
either: Python has no `Related`.

## Loading related rows for many records

Reading each record's related rows with `find` runs one query per record.
`relation.load(db, rows)` reads them for a whole list in one query of the
distinct keys, and returns one entry per input row, in the input's order:

- along a `has_many` relation, such as a bakery's cakes, a `list` of records
  per row, an empty one where nothing matched;
- along a `belongs_to` or `has_one` relation, such as a cake's bakery, one
  record per row, or `None` where the row's key holds a `None`.

A relation's `cardinality`, `"one"` or `"many"`, says which. Two things are
errors rather than silent answers, both a `DecodeError`, along a
`belongs_to` or `has_one` relation: a key that no row at the far end matches,
and a key that two rows there match, which contradicts a relation to at most
one row.

Here Python differs from Rust in three places. Rust has a loader for each
shape, `load_one` and `load_many`, where Python's one `load` follows the
relation's kind. A Rust loader takes the related entity or a `Select` of it,
to filter and order what is loaded; `load` takes no query, and reads the far
end's rows whole in no promised order, so the program filters and sorts each
list itself. And Rust's `load_many_via` follows a relation through a junction
in one query, which Python does in two: load the junction rows, then their
far end.

```python
import asyncio
import os
from typing import Any

from pgorm import Pool

from bakery import Bakery, bakery_cakes, cake_baker_baker, cake_bakers, cake_bakery


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        bakeries = await Bakery.find().order_by(Bakery.col("id").asc()).all(pool)

        # Each bakery's gluten-free cakes, cheapest first.
        menus = await bakery_cakes.load(pool, bakeries)
        gluten_free: list[list[Any]] = [
            sorted((cake for cake in menu if cake["gluten_free"]), key=lambda cake: cake["price"])
            for menu in menus
        ]
        names = [[cake["name"] for cake in menu] for menu in gluten_free]
        assert names == [["Lemon Drizzle", "Chocolate Torte"], ["Vanilla Sponge"]]

        # Each of Sunrise's gluten-free cakes, with its bakery and its bakers.
        sunrise_cakes = gluten_free[0]
        their_bakery = await cake_bakery.load(pool, sunrise_cakes)
        assert all(bakery["name"] == "Sunrise" for bakery in their_bakery)

        # The junction rows of every cake in one query, then their bakers in another.
        links = await cake_bakers.load(pool, sunrise_cakes)
        bakers = iter(await cake_baker_baker.load(pool, [link for row in links for link in row]))
        per_cake = [sorted(next(bakers)["name"] for _ in row) for row in links]
        assert per_cake == [["Ada", "Bo"], ["Bo"]]


asyncio.run(main())
```

## Reading rows with their related rows

The graph reads rows of one model, the *root*, with rows of related models
joined beside them, in one query. Start it with `graph()` where you would
write `find()`, and add each related model as a *slot*, along a relation from
the root or from a model the graph already reads. A slot is required or
optional:

| Method | Joins | Decodes as |
| --- | --- | --- |
| `join_one(relation)` | `INNER JOIN` | a record of the relation's far end |
| `join_maybe(relation)` | `LEFT JOIN` | a record, or `None` where the join matched nothing |

Here Python differs from Rust, whose graph also has `related_one::<F>()` and
`related_maybe::<F>()`, following the path a `Related` impl describes. Python
has no `Related`, so every slot names its relation, and a path through a
junction is written with `via`, below.

A row is a tuple of the root's record and one item per slot, in the order the
slots were added; a graph with no slot gives the root's records alone. Here
Python differs from Rust again: the Rust compiler knows the tuple's type from
the slots, where a Python graph's terminals are typed for every shape a graph
can give, `ModelRecord | tuple[ModelRecord | None, ...]`. A program checked
with mypy states the shape with `cast`, as the examples do, and nothing checks
the cast against the graph.

```python
import asyncio
import os
from typing import cast

from pgorm import ModelRecord, Pool

from bakery import Order, order_bakery, order_customer


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        orders = cast(
            list[tuple[ModelRecord, ModelRecord, ModelRecord]],
            await Order.graph()
            .join_one(order_customer)
            .join_one(order_bakery)
            .order_by(Order.col("placed_at").asc())
            .all(pool),
        )
        summary = [(order["id"], customer["name"], bakery["name"]) for order, customer, bakery in orders]
        assert summary == [(1, "Kim", "Sunrise"), (2, "Lee", "Moonlight")]


asyncio.run(main())
```

A required slot leaves out a root row with nothing to join; an optional one
keeps it and decodes `None`, which it does exactly where every column the slot
reads is `NULL`. A slot row that is present but does not decode raises
`DecodeError`; it never becomes an absent one. A graph with one slot can also
be read grouped: `all_grouped` returns each root once, with its slot's records
collected in a `list`, an empty one when nothing matched. It orders by your
ordering first and the root's primary key after it, so your ordering decides
the order of both the roots and each list, and `inspect(terminal="all_grouped")`
shows that statement.

```python
import asyncio
import os
from typing import cast

from pgorm import ModelRecord, Pool

from bakery import Bakery, Cake, bakery_cakes


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # A third bakery, which sells nothing yet.
        await Bakery.insert({"name": "Noon", "city": "Tromsø"}).execute(pool)

        rows = cast(
            list[tuple[ModelRecord, ModelRecord | None]],
            await Bakery.graph()
            .join_maybe(bakery_cakes)
            .where(Bakery.col("name") != "Sunrise")
            .order_by(Bakery.col("id").asc(), Cake.col("name").asc())
            .all(pool),
        )
        flat = [(bakery["name"], None if cake is None else cake["name"]) for bakery, cake in rows]
        assert flat == [
            ("Moonlight", "Carrot Cake"),
            ("Moonlight", "Vanilla Sponge"),
            ("Noon", None),
        ]

        grouped = await (
            Bakery.graph()
            .join_maybe(bakery_cakes)
            .order_by(Cake.col("price").asc())
            .all_grouped(pool)
        )
        menu = [(bakery["name"], len(cakes)) for bakery, cakes in grouped]
        assert menu == [("Moonlight", 2), ("Sunrise", 3), ("Noon", 0)]


asyncio.run(main())
```

The roots come in the order of their first row. Ordering by price put
Moonlight's Vanilla Sponge first, so Moonlight comes first; Noon has no row
with a price, and PostgreSQL sorts its `NULL` last.

Here Python differs from Rust, whose graph has no `one`, since one root can
make several rows, and whose `one_opt` reads the first row with `LIMIT 1`. A
Python graph has `one` and `optional` with a model query's strict counts:
exactly one row, or at most one, and any other count a `DecodeError`.
`count` counts the graph's rows, and `stream` reads them one at a time, as
[Pages, cursors and streams](06-pagination.md) describes for a model query.

### Junctions and other paths

`via(relation)` joins a table the path runs through without decoding it, such
as a junction, so a many-to-many read is a `via` and a slot. A `via` is always
a `LEFT JOIN`, so a missing middle row does not drop a root row by itself:

```python
from bakery import Cake, cake_baker_baker, cake_bakers

cakes_and_bakers = Cake.graph().via(cake_bakers).join_maybe(cake_baker_baker)
assert cakes_and_bakers.inspect().sql == (
    'SELECT "cake"."id" AS "s0_id", "cake"."name" AS "s0_name", "cake"."flavor" AS "s0_flavor", '
    '"cake"."price" AS "s0_price", "cake"."gluten_free" AS "s0_gluten_free", '
    '"cake"."bakery_id" AS "s0_bakery_id", "baker"."id" AS "s1_id", "baker"."name" AS "s1_name", '
    '"baker"."hired_on" AS "s1_hired_on", "baker"."bakery_id" AS "s1_bakery_id" '
    'FROM "cake" LEFT JOIN "cake_baker" ON "cake"."id" = "cake_baker"."cake_id" '
    'LEFT JOIN "baker" ON "cake_baker"."baker_id" = "baker"."id"'
)
```

This is the SQL Rust's `related_maybe::<baker::Entity>()` writes for the same
read, the flavor aside, which Rust reads cast to `text`. The graph names its
result columns `s0_` for the root and `s1_`, `s2_` and on for each slot, each
name kept within PostgreSQL's 63 bytes as pgorm's own graph keeps it. `via`
hops are not counted. `col(source, field)` gives a field of a decoded source,
`0` the root and `i` the i-th slot, qualified as that source is named. Here
Python differs from Rust, whose graph holds up to six slots; a Python graph
sets no such limit.

A path that passes through one table twice binds the second visit to an alias:
`via`, `join_one` and `join_maybe` each take `alias=`, and `from_=` names the
decoded source a relation starts from, where it would otherwise start from the
first table of its model the graph reads. Here Python differs from Rust, whose
`from_alias` lets the next relation start from any aliased table, a `via` hop
included. Python's `from_` names a decoded source only, so a visit that the
path goes on from is a slot.

Here each cake reads the other cakes one of its bakers bakes: from the cake
through `cake_baker` to the junction rows of the same baker, under the alias
`shared`, decoded so the path can go on from it, and from them to the other
cakes, under the alias `sibling`. A graph of two slots has no `all_grouped`,
so the program groups the rows itself:

```python
import asyncio
import os
from typing import cast

from pgorm import ModelRecord, Pool

from bakery import Cake, CakeBaker, cake_baker_cake, cake_bakers

same_baker = CakeBaker.has_many(CakeBaker, from_="baker_id", to="baker_id")


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        graph = (
            Cake.graph()
            .via(cake_bakers)
            .join_maybe(same_baker, alias="shared")
            .join_maybe(cake_baker_cake, alias="sibling", from_=1)
        )
        graph = graph.where(graph.col(2, "id") != graph.col(0, "id")).order_by(graph.col(2, "name").asc())

        # The WHERE keeps only rows that found a sibling, so neither slot is None.
        rows = cast(
            list[tuple[ModelRecord, ModelRecord, ModelRecord]],
            await graph.all(pool),
        )
        siblings: dict[int, list[str]] = {}
        for cake, _, sibling in rows:
            siblings.setdefault(cake["id"], []).append(sibling["name"])
        assert siblings[2] == ["Chocolate Fudge", "Chocolate Torte"]


asyncio.run(main())
```

A filter or an ordering on an aliased table names it through `col(source,
field)`, as above, or through the aliased model, `Cake.as_("sibling").col(..)`.

### Narrowing a join

A `where` lands in the query's `WHERE`, where it applies to the joined row as
a whole: under a `LEFT JOIN`, a root whose slot did not match has `NULL`s
there, fails the condition, and drops out, as if the join were `INNER`.

Here Python differs from Rust: narrowing which rows *match* instead, keeping
the unmatched roots, is not available in Python yet. Rust's
`join_maybe_filtered` adds the condition to the join's `ON`; a Python relation
joins by its paired fields alone. Every bakery with its carrot cakes is read
with a loader, the lists narrowed in Python:

```python
import asyncio
import os

from pgorm import Pool

from bakery import Bakery, Cake, bakery_cakes


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # A WHERE on the slot drops the bakery without a carrot cake.
        filtered = await (
            Bakery.graph()
            .join_maybe(bakery_cakes)
            .where(Cake.col("flavor") == "carrot")
            .all_grouped(pool)
        )
        assert [(bakery["name"], len(cakes)) for bakery, cakes in filtered] == [("Moonlight", 1)]

        # Every bakery, with its carrot cakes if it has any.
        bakeries = await Bakery.find().order_by(Bakery.col("id").asc()).all(pool)
        menus = await bakery_cakes.load(pool, bakeries)
        carrot = [
            (bakery["name"], len([cake for cake in menu if cake["flavor"] == "carrot"]))
            for bakery, menu in zip(bakeries, menus)
        ]
        assert carrot == [("Sunrise", 0), ("Moonlight", 1)]


asyncio.run(main())
```

### Limits, pages and cursors

`limit` and `offset` count the rows of the join, not the roots: a bakery with
three cakes spends three rows of a limit. Rust therefore refuses
`all_grouped` on a limited graph, whose last entry could hold only part of
its root's rows, and `paginate`, `count` and a cursor, which set the rows
themselves; its compiler rejects each.

Here Python differs from Rust: a Python graph refuses none of them. A limited
graph's `all_grouped` groups the rows the limit let through, so its last root
can come with part of its rows, as Sunrise does here with two of its three
cakes; a paginator's pages and a cursor's window replace the limit, and
`count` drops it. Do not limit a graph you read grouped.

```python
import asyncio
import os

from pgorm import Pool

from bakery import Bakery, bakery_cakes


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        limited = (
            Bakery.graph()
            .join_maybe(bakery_cakes)
            .order_by(Bakery.col("id").asc())
            .limit(2)
        )
        partial = await limited.all_grouped(pool)
        assert [(bakery["name"], len(cakes)) for bakery, cakes in partial] == [("Sunrise", 2)]


asyncio.run(main())
```

To read the first `n` roots with all of their related rows, limit the roots
and load the rest:

```python
import asyncio
import os

from pgorm import Pool

from bakery import Bakery, bakery_cakes


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        first = await Bakery.find().order_by(Bakery.col("id").asc()).limit(1).all(pool)
        cakes = await bakery_cakes.load(pool, first)
        assert (len(first), len(cakes[0])) == (1, 3)


asyncio.run(main())
```

A graph pages as a model query does, by rows of the join, through `paginate`
and `cursor`. `cursor(*fields)` orders by fields of the root, and then breaks
ties by the root's primary key and every slot's, in the order the slots were
added, so rows that share an ordering value still page correctly. Here Python
differs from Rust, whose `cursor_by_on` orders by the columns of one slot;
ordering a graph's cursor by a slot's columns is not available in Python yet.
[Pages, cursors and streams](06-pagination.md#ties-and-secondary-keys) shows
`after` beside `after_with`, which takes the whole ordering.

An optional slot that matched nothing has `NULL`s for its tiebreak, and
PostgreSQL's comparisons with `NULL` select no continuation inside that key.
A boundary of the order fields alone, `after` or `before`, is the way past
such a root; it also passes over the rest of that root's joined rows. Choose
the boundary that matches where the window ended.

```python
import asyncio
import os
from typing import cast

from pgorm import ModelRecord, Pool

from bakery import Cake, cake_bakery

Rows = list[tuple[ModelRecord, ModelRecord]]


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # Every cake with its bakery, two rows at a time, cheapest first.
        cursor = Cake.graph().join_one(cake_bakery).cursor("price")
        assert cursor.first(2).inspect().sql.endswith(
            'ORDER BY "cake"."price" ASC, "cake"."id" ASC, "bakery"."id" ASC LIMIT $1'
        )

        page = cast(Rows, await cursor.first(2).all(pool))
        names = [(cake["name"], bakery["name"]) for cake, bakery in page]
        assert names == [("Vanilla Sponge", "Moonlight"), ("Lemon Drizzle", "Sunrise")]

        last_price = page[1][0]["price"]
        following = cast(Rows, await cursor.after(last_price).first(2).all(pool))
        assert [cake["name"] for cake, _ in following] == ["Carrot Cake", "Chocolate Fudge"]


asyncio.run(main())
```

## Registered graphs

A graph a Rust crate builds, a `SelectGraph` of its entities, can be
registered by name with the crate's entities and read from Python; [Registering
Rust entities](24-registrations.md#registered-graphs-and-keyset-cursors) says
how, and covers what follows in full. A registered graph's root, its slots and
their joins are fixed in Rust, so the application's relations decide what it
joins, their `via` hops and conditions included, and Python chooses the
aliases, the filters, the ordering and the cursor. `pgorm.graphs()` lists the
names a build registers, `pgorm.graph(name)` gives one, and its `describe()`
reports its `name`, its Rust type and its sources.

`find(aliases=[..])` builds the graph with each slot under the alias given,
`g1`, `g2` and so on when none are, and gives a query whose `where` and
`order_by` are `SelectGraph`'s. `col(source, column)` names a column of a
decoded source as the query names it. Its `all` is `SelectGraph::all`, and its
`optional` is Rust's `one_opt`, which reads the first row with `LIMIT 1`. A
row is the root's `EntityModel` when the graph has no slot, and otherwise a
tuple of `EntityModel`s, `None` for an optional slot that matched nothing.

`cursor(column)` is `SelectGraph::cursor_by` on one root column, with the
root's and every slot's primary key added as tiebreaks: `after` and `before`
take that column's value, and `after_with` and `before_with` the whole key,
the column first, then the root's other key columns and each slot's, in
order. Here a registered graph differs from a model's: it has no `stream`,
`all_grouped`, `paginate` or `count`, and its cursor orders by one root column
only.
