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
one hop from one model to another in one direction, and so is a path of more
than one: `Model.related` declares a path through a junction, as a Rust
`Related` with a `via` does, and `Model.linked` a chain of relations, as a
Rust `Linked` type does. Every read names the relation or the path it
follows.

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

from bakery import Baker, Bakery, Cake, bakery_cakes, cake_related_bakers


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        sunrise = await Bakery.find_by_key({"id": 1}).one(pool)
        cakes = await bakery_cakes.find(sunrise).order_by(Cake.col("price").asc()).all(pool)
        assert [cake["name"] for cake in cakes] == ["Lemon Drizzle", "Chocolate Fudge", "Chocolate Torte"]

        # A cake's bakers, through the cake_baker junction.
        lemon_drizzle = cakes[0]
        bakers = await cake_related_bakers.find(lemon_drizzle).order_by(Baker.col("name").asc()).all(pool)
        assert [baker["name"] for baker in bakers] == ["Ada", "Bo"]


asyncio.run(main())
```

`bakery.py` declares the cake's bakers as the path the Rust cake's
`Related<baker::Entity>` declares: `Cake.related(cake_baker_baker,
via=cake_bakers)`, `via` the relation from the cake to its junction rows and
then the relation from a junction row to its baker, which reaches one row.
Its `find` joins through the junction and keys the cake by its primary key,
as Rust's `find_related` does:

```python
from decimal import Decimal

from bakery import cake_related_bakers

lemon_drizzle = {
    "id": 2, "name": "Lemon Drizzle", "flavor": "lemon", "price": Decimal("3.20"),
    "gluten_free": True, "bakery_id": 1,
}
assert cake_related_bakers.find(lemon_drizzle).inspect().sql == (
    'SELECT "baker"."id", "baker"."name", "baker"."hired_on", "baker"."bakery_id" FROM "baker" '
    'INNER JOIN "cake_baker" ON "cake_baker"."baker_id" = "baker"."id" '
    'INNER JOIN "cake" ON "cake"."id" = "cake_baker"."cake_id" WHERE "cake"."id" = $1'
)
```

A relation with an `on_condition` ([Models and
entities](03-entities.md#relations)) is read whole the same way: its `find`
joins the model it starts at back, so the condition can read both ends.

## Longer paths

A path of more than one relation is a `Linked` chain, which a model's
`linked(*relations)` makes: the relations from one model to another, in
order, each starting where the one before it ends. `find(row)` reads along it
from one record, as Rust's `find_linked` does. Here a baker's customers are
the people who ordered a cake the baker bakes:

```python
import asyncio
import os

from pgorm import Pool

from bakery import Baker, baker_cake_bakers, cake_baker_cake, cake_order_lines, order_customer, order_line_order

# From a baker to the customers who ordered what they bake.
baked_for = Baker.linked(baker_cake_bakers, cake_baker_cake, cake_order_lines, order_line_order, order_customer)


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        cy = await Baker.find_by_key({"id": 3}).one(pool)
        customers = await baked_for.find(cy).all(pool)
        assert [customer["name"] for customer in customers] == ["Lee"]


asyncio.run(main())
```

Each hop joins its table under an alias of its own, `r0`, `r1` and so on,
walking back from the far end, so a chain may pass through one table twice.
A filter or an ordering on a hop's fields names the hop: `hop(i)` is the model
read at hop `i` under that alias, as Rust's `LinkedAlias::hop` names it, and
`last_hop()` the starting model's, as `last_hop_alias` names it.
`find_linked()` reads every row the chain reaches, from any starting row.
Here Python differs from Rust, whose `hop` and `last_hop_alias` give the
alias's name; a Python hop is the model under it, whose `col(field)` is
qualified by it:

```python
from bakery import Baker, Cake, baker_cake_bakers, cake_baker_cake

bakes = Baker.linked(baker_cake_bakers, cake_baker_cake)
assert bakes.last_hop().qualifier == "r1"
assert bakes.find({"id": 1}).inspect().sql == (
    'SELECT "cake"."id", "cake"."name", "cake"."flavor", "cake"."price", '
    '"cake"."gluten_free", "cake"."bakery_id" FROM "cake" '
    'INNER JOIN "cake_baker" AS "r0" ON "r0"."cake_id" = "cake"."id" '
    'INNER JOIN "baker" AS "r1" ON "r1"."id" = "r0"."baker_id" WHERE "r1"."id" = $1'
)
adas = bakes.find_linked().where(bakes.last_hop().col("name") == "Ada").order_by(Cake.col("id").asc())
assert adas.inspect().sql.endswith('WHERE "r1"."name" = $1 ORDER BY "cake"."id" ASC')
```

A path that a related path already describes needs no chain of its own:
Rust's `RelatedLink` is that chain, and a Python related path has its own
`find`. A model query's `join` can also go on from a second visit to one
table: `relation.from_alias(name)`, as Rust's `RelationDef::from_alias`,
starts the join from the table answering to `name`.

## Loading related rows for many records

Reading each record's related rows with `find` runs one query per record.
`relation.load(db, rows)` reads them for a whole list in one query, and
returns one entry per input row, in the input's order:

- along a `has_many` relation, such as a bakery's cakes, a `list` of records
  per row, an empty one where nothing matched;
- along a `belongs_to` or `has_one` relation, such as a cake's bakery, one
  record per row, or `None` where nothing matched or the row's key holds a
  `None`;
- along a related path, such as a cake's bakers, a `list` of records per row,
  as Rust's `load_many_via` reads it.

A relation's `cardinality`, `"one"` or `"many"`, says which. Here Python
differs from Rust: Rust has a loader for each shape, `load_one` and
`load_many`, where Python's one `load` follows the relation's kind. Each
takes a query of the related model as its third argument, to filter and order
what is loaded, as Rust's loaders take a `Select`:

```python
import asyncio
import os

from pgorm import Pool

from bakery import Baker, Bakery, Cake, bakery_cakes, cake_bakery, cake_related_bakers


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        bakeries = await Bakery.find().order_by(Bakery.col("id").asc()).all(pool)

        # Each bakery's gluten-free cakes, cheapest first.
        gluten_free = Cake.find().where(Cake.col("gluten_free").eq(True)).order_by(Cake.col("price").asc())
        cakes = await bakery_cakes.load(pool, bakeries, gluten_free)
        names = [[cake["name"] for cake in menu] for menu in cakes]
        assert names == [["Lemon Drizzle", "Chocolate Torte"], ["Vanilla Sponge"]]

        # Each of Sunrise's cakes, with its bakery and its bakers.
        sunrise_cakes = cakes[0]
        their_bakery = await cake_bakery.load(pool, sunrise_cakes)
        assert all(bakery is not None and bakery["name"] == "Sunrise" for bakery in their_bakery)

        bakers = await cake_related_bakers.load(pool, sunrise_cakes, Baker.find().order_by(Baker.col("name").asc()))
        assert [[baker["name"] for baker in row] for row in bakers] == [["Ada", "Bo"], ["Bo"]]


asyncio.run(main())
```

A loader reads the relation whole, its `on_condition` included: it joins the
model the relation starts at back under `pgorm_loader_src`, as Rust's does,
and an ordering on its query orders every record's list. Two things are
errors rather than silent answers, both a `DecodeError`: two different rows
for one record along a `belongs_to` or `has_one` relation, which contradicts
a relation to at most one row, and a related row whose key matches in SQL but
not as a Python value.

## Reading rows with their related rows

The graph reads rows of one model, the *root*, with rows of related models
joined beside them, in one query. Start it with `graph()` where you would
write `find()`, and add each related model as a *slot*, along a relation or a
path from the root or from a model the graph already reads. A slot is
required or optional:

| Method | Joins | Decodes as |
| --- | --- | --- |
| `related_one(path)` | a relation or a related path, `INNER JOIN` | a record of the path's far end |
| `related_maybe(path)` | the same path, `LEFT JOIN` | a record, or `None` where the join matched nothing |
| `join_one(relation)` | one relation, `INNER JOIN` | a record of the relation's far end |
| `join_maybe(relation)` | one relation, `LEFT JOIN` | a record, or `None` where the join matched nothing |

A row is a tuple of the root's record and one item per slot, in the order the
slots were added; a graph with no slot gives the root's records alone. Here
Python differs from Rust: the Rust compiler knows the tuple's type from
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
            .related_one(order_customer)
            .related_one(order_bakery)
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
as a junction. `related_maybe` and `related_one` add a related path's
junction themselves, `related_one` by an `INNER JOIN` and `related_maybe` by
a `LEFT JOIN`; written by hand, the same many-to-many read is a `via` and a
slot. A `via` is always a `LEFT JOIN`, so a missing middle row does not drop
a root row by itself:

```python
from bakery import Cake, cake_baker_baker, cake_bakers, cake_related_bakers

by_hand = Cake.graph().via(cake_bakers).join_maybe(cake_baker_baker)
described = Cake.graph().related_maybe(cake_related_bakers)
assert by_hand.inspect().sql == described.inspect().sql
assert described.inspect().sql == (
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

A path that passes through one table twice binds the second visit to an
alias: `via`, `join_one` and `join_maybe` each take `alias=`, and
`relation.from_alias(name)`, as Rust's `from_alias` does, makes the next
relation start from the table answering to `name`, a `via` hop included.
Otherwise a relation starts from the first table of its model the graph
reads, or from the decoded source `from_=` names by its index. Here each cake
reads the other cakes one of its bakers bakes: from the cake through
`cake_baker` to the baker, and back through `cake_baker` a second time, under
an alias, to the other cakes:

```python
import asyncio
import os

from pgorm import Pool

from bakery import Cake, baker_cake_bakers, cake_baker_baker, cake_baker_cake, cake_bakers


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        sibling = Cake.as_("sibling")
        siblings = await (
            Cake.graph()
            .via(cake_bakers)
            .via(cake_baker_baker)
            .via(baker_cake_bakers, alias="shared")
            .join_maybe(cake_baker_cake.from_alias("shared"), alias="sibling")
            .where(sibling.col("id") != Cake.col("id"))
            .order_by(sibling.col("name").asc())
            .all_grouped(pool)
        )
        lemon_drizzle = next(others for cake, others in siblings if cake["id"] == 2)
        assert [cake["name"] for cake in lemon_drizzle] == ["Chocolate Fudge", "Chocolate Torte"]


asyncio.run(main())
```

A filter or an ordering on an aliased table names it through the aliased
model, `Cake.as_("sibling").col(..)`, as above, or through `col(source,
field)`.

### Narrowing a join

A `where` lands in the query's `WHERE`, where it applies to the joined row as
a whole: under a `LEFT JOIN`, a root whose slot did not match has `NULL`s
there, fails the condition, and drops out, as if the join were `INNER`. To
narrow which rows *match* instead, keeping the unmatched roots,
`join_maybe_filtered(relation, condition)` adds the condition to the join's
`ON`, as Rust's `join_maybe_filtered` does. Its function receives the join's
two ends, each the model as the join names its table, so it qualifies its
fields through them:

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
        carrot = await (
            Bakery.graph()
            .join_maybe_filtered(bakery_cakes, lambda bakery, cake: cake.col("flavor") == "carrot")
            .all_grouped(pool)
        )
        assert [(bakery["name"], len(cakes)) for bakery, cakes in carrot] == [("Sunrise", 0), ("Moonlight", 1)]


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
added, so rows that share an ordering value still page correctly.
`cursor_on(source, *fields)` orders by fields of slot `source` instead,
counting the first slot as `1`, as Rust's `cursor_by_on` does, and breaks
ties by that slot's primary key, then the root's and every other slot's.
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

        # By the bakery's name, its id and then the cake's breaking ties.
        by_bakery = Cake.graph().join_one(cake_bakery).cursor_on(1, "name")
        assert by_bakery.first(2).inspect().sql.endswith(
            'ORDER BY "bakery"."name" ASC, "bakery"."id" ASC, "cake"."id" ASC LIMIT $1'
        )
        moonlight = cast(Rows, await by_bakery.first(2).all(pool))
        assert [(cake["name"], bakery["name"]) for cake, bakery in moonlight] == [
            ("Carrot Cake", "Moonlight"),
            ("Vanilla Sponge", "Moonlight"),
        ]


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

`cursor(*columns)` is `SelectGraph::cursor_by` on root columns, and
`cursor_on(source, *columns)` `cursor_by_on` on a slot's, with every decoded
source's primary key added as tiebreaks: `after` and `before` take the order
columns' values, and `after_with` and `before_with` those or the whole key,
the order columns first, then the ordered source's other key columns and
each other source's, in order. `stream`, `all_grouped`, `paginate` and
`count` are `SelectGraph`'s own, `all_grouped` on a graph of one `Opt` slot
only, as in Rust.
