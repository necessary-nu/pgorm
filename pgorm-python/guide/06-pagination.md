# Pages, cursors and streams

A query whose result is too long to read at once can be read a piece at a
time, three ways:

- *Pages* by number, with `paginate`: page 3 of 20, the shape of a list with
  numbered pages. Each page is a `LIMIT`/`OFFSET`, so a deep page costs the
  database every row before it.
- *Cursors*, with `cursor`: the rows after (or before) a given row, the shape
  of an endless feed or a "next" link. Each window is a `WHERE` on the
  ordering columns, so a deep window costs no more than the first, and rows
  written between two reads do not shift it.
- *Streams*, with `stream`: every row of one query, decoded as it arrives
  from the server instead of collected into a `list`.

The examples page through the bakery's five cakes. This chapter is the
Python dialect of the Rust guide's [Pages, cursors and
streams](../../docs/guide/05-pagination.md).

## Pages

`paginate(page_size)` turns a model query, or a [graph](07-relations.md),
into a `Paginator` that reads `page_size` rows a page. Pages are numbered from
zero. `fetch_page(db, n)` reads one; `num_items(db)`, `num_pages(db)` and
`num_items_and_pages(db)` count, each running a `COUNT(*)` over the query, and
the last page is counted even when it is not full. `inspect(n)` shows the
statement reading page `n`, and `inspect("count")` the one counting.

Here Python differs from Rust in four places. Rust takes the page size as a
`NonZeroU64`, so a page size of zero does not compile; Python takes an `int`
and raises `ConstructionError` when the paginator is made from one below 1,
as it does for a page whose offset is past what PostgreSQL's `bigint` holds.
A Rust paginator is made with its connection; a Python one holds no database,
and each terminal takes the pool, connection or transaction to run on.
`num_items_and_pages` gives a tuple of the two counts, where Rust gives a
struct with `number_of_items` and `number_of_pages`. And a page's type is
looser. A page holds the query's own rows, a model query's records or a
graph's tuples of them, and where Rust's page type is the query's, a Python
paginator's terminals, and a cursor's below, are typed for both shapes,
`ModelRecord | tuple[ModelRecord | None, ...]`. A program checked with mypy
narrows a page with `cast`, as the examples do.

Order the query before paginating it. Without an `ORDER BY`, PostgreSQL may
return the rows in a different order for each page, and a row can then appear
on two pages or on none.

```python
import asyncio
import os

from typing import cast

from pgorm import ModelRecord, Pool

from bakery import Cake

PAGE_SIZE = 2


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        pages = Cake.find().order_by(Cake.col("name").asc()).paginate(PAGE_SIZE)

        assert await pages.num_items_and_pages(pool) == (5, 3)

        second = [cake["name"] for cake in cast(list[ModelRecord], await pages.fetch_page(pool, 1))]
        assert second == ["Chocolate Torte", "Lemon Drizzle"]
        assert await pages.fetch_page(pool, 3) == []

        compiled = pages.inspect(1)
        assert compiled.sql.endswith('ORDER BY "cake"."name" ASC LIMIT $1 OFFSET $2')
        assert [value.value for value in compiled.params] == [2, 2]


asyncio.run(main())
```

Here Python differs from Rust again: a Python paginator keeps no current page,
so it has no `fetch`, `advance` or `fetch_and_next`. `pages(db)` is the loop
over every page those make in Rust, and Rust's `into_stream` too: an
asynchronous iterator that reads each page in turn from the first and ends at
the first empty one.

```python
import asyncio
import os
from typing import cast

from pgorm import ConstructionError, ModelRecord, Pool

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        pages = Cake.find().order_by(Cake.col("id").asc()).paginate(2)
        sizes = [len(page) async for page in pages.pages(pool)]
        assert sizes == [2, 2, 1]

        all_pages = [cast(list[ModelRecord], page) async for page in pages.pages(pool)]
        assert [cake["id"] for cake in all_pages[2]] == [5]

    try:
        Cake.find().paginate(0)
    except ConstructionError as error:
        assert str(error) == "a page size is at least 1"
    else:
        raise AssertionError("a page holds at least one row")


asyncio.run(main())
```

A model query of chosen fields, `select(..)`, paginates the same way, and so does a
[graph](07-relations.md) of related rows; a read of a [disjoint
enum](15-disjoint.md) has its own `paginate`. `count` on the query is the
paginator's count without the pages.

## Cursors

`cursor(*fields)` orders a query by one field or several, the cursor's *key*,
and returns a `Cursor`. The key should identify a row, so end it with the
primary key: rows that tie on the key cannot be told apart, and a window that
ends inside a run of them cannot say where it ended. The cursor's ordering
replaces any the query had.

`after(*values)` and `before(*values)` take a row's key values and keep the
rows past it in the cursor's order, each value converted through its field's
kind. `first(n)` takes that many rows from the start of what is left and
`last(n)` from its end, each replacing the other and the query's own limit;
`asc()` and `desc()` set the direction. `all(db)` runs it, and always returns
the rows in the cursor's order, `last` included.

A feed of the cakes by price, cheapest first, reads a window, then the window
after its last row:

```python
import asyncio
import os
from typing import cast

from pgorm import Cursor, ModelRecord, Pool

from bakery import Cake


async def window(cursor: Cursor, pool: Pool) -> list[ModelRecord]:
    return cast(list[ModelRecord], await cursor.all(pool))


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        feed = Cake.find().cursor("price", "id")
        first = await window(feed.first(2), pool)
        assert [cake["name"] for cake in first] == ["Vanilla Sponge", "Lemon Drizzle"]

        # The next window starts after the last row read.
        last_seen = first[-1]
        after = await window(feed.after(last_seen["price"], last_seen["id"]).first(2), pool)
        assert [cake["name"] for cake in after] == ["Carrot Cake", "Chocolate Fudge"]

        # The two dearest, from the far end, still in ascending order.
        dearest = await window(Cake.find().cursor("price", "id").last(2), pool)
        assert [cake["name"] for cake in dearest] == ["Chocolate Fudge", "Chocolate Torte"]


asyncio.run(main())
```

Here Python differs from Rust. A Rust `Cursor` is changed in place by `after`
and `first`, and runs again each time `all` is called with the boundary and
window it holds then. A Python cursor's methods each return a new cursor and
leave the one they were called on as it was, so a feed keeps its first cursor
and derives each window from it, as `feed` above does. And the key's length
fixes the boundary's in both, but where Rust refuses a boundary of the wrong
length at compile time, Python raises `ConstructionError` when it is given.

The cursor writes the ordering and the boundary itself. A boundary over
several columns becomes the comparison that reads them in order, and
`inspect()` shows the statement:

```python
from decimal import Decimal

from pgorm import ConstructionError

from bakery import Cake

feed = Cake.select("name").cursor("price", "id").after(Decimal("3.20"), 2).first(2)
compiled = feed.inspect()
assert compiled.sql == (
    'SELECT "cake"."name" FROM "cake" '
    'WHERE ("cake"."price" = $1 AND "cake"."id" > $2) OR "cake"."price" > $3 '
    'ORDER BY "cake"."price" ASC, "cake"."id" ASC LIMIT $4'
)
assert [value.value for value in compiled.params] == [Decimal("3.20"), 2, Decimal("3.20"), 2]

try:
    Cake.find().cursor("price", "id").after(Decimal("3.20"))
except ConstructionError as error:
    assert str(error) == "a cursor boundary of 1 values does not match 2 key column(s)"
else:
    raise AssertionError("a boundary takes a value for each key column")
```

A cursor over chosen fields, as above, decodes its rows as the query does,
into the model's records of those fields. Here Python differs from Rust, whose
cursor over chosen columns decodes only once `into_model` or
`into_partial_model` says what into. A window of no rows, `first(0)`, reads
none.

### Ties and secondary keys

`add_secondary_order_by(model, field)` appends a column of the queried model
or a joined one to the cursor's ordering, after the key, which is how a cursor
over a join is made total; the model names the table as the query names it,
an aliased model its alias, as Rust's `add_secondary_order_by(table, column)`
names it. `after` and `before` still take the key alone, and so resume past
every row sharing the boundary's key. `after_with` and `before_with` take the
whole ordering, the appended columns included, and resume from exactly one
row:

```python
import asyncio
import os
from typing import cast

from pgorm import ModelRecord, Pool

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # By bakery, then by id within each bakery.
        by_bakery = Cake.find().cursor("bakery_id").add_secondary_order_by(Cake, "id")

        first = cast(list[ModelRecord], await by_bakery.first(2).all(pool))
        assert [cake["id"] for cake in first] == [1, 2]

        # Bakery 1 has a third cake, which `after(1)` would skip.
        rest = cast(list[ModelRecord], await by_bakery.after_with(1, 2).first(2).all(pool))
        assert [cake["id"] for cake in rest] == [5, 3]
        skipped = cast(list[ModelRecord], await by_bakery.after(1).first(2).all(pool))
        assert [cake["id"] for cake in skipped] == [3, 4]


asyncio.run(main())
```

A [graph](07-relations.md)'s cursor is where the two boundaries differ. It
orders by fields of its root and then adds the root's primary key and every
related table's as tiebreaks itself. `after` and `before` take the order
fields alone, and so resume past every row sharing them; `after_with` and
`before_with` take those or the whole ordering, the tiebreaks included, and
resume from exactly one row. Here each bakery is read with its cakes, ordered
by the bakery's id, so a boundary of the bakery alone passes over the rest of
its cakes:

```python
import asyncio
import os
from typing import cast

from pgorm import ModelRecord, Pool

from bakery import Bakery, bakery_cakes

Rows = list[tuple[ModelRecord, ModelRecord]]


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        menu = Bakery.graph().join_one(bakery_cakes).cursor("id")

        first = cast(Rows, await menu.first(2).all(pool))
        assert [(bakery["id"], cake["id"]) for bakery, cake in first] == [(1, 1), (1, 2)]

        # Past Sunrise altogether.
        past = cast(Rows, await menu.after(1).first(2).all(pool))
        assert [(bakery["id"], cake["id"]) for bakery, cake in past] == [(2, 3), (2, 4)]

        # Past exactly the last row read: Sunrise's third cake comes next.
        resumed = cast(Rows, await menu.after_with(1, 2).first(2).all(pool))
        assert [(bakery["id"], cake["id"]) for bakery, cake in resumed] == [(1, 5), (2, 3)]


asyncio.run(main())
```

A graph's `cursor_on(source, *fields)` orders by fields of one of its slots
instead, the first slot `1`, as Rust's `cursor_by_on` does, and
[Relations](07-relations.md#limits-pages-and-cursors) shows one.

## Paginating raw SQL

A query written as SQL paginates and counts too, as Rust's
`SelectorRaw::paginate` and `count` do: `from_statement(statement)` holds a
`RawSQL` whose rows are records, and a model's `find_by_statement` one whose
rows are its records ([Raw SQL](10-raw-sql.md#decoding-rows-into-models)).
Either's `paginate(page_size)` gives the `Paginator` a model query gives,
and `count(db)` counts. Both read the statement as a subquery, so its own
`ORDER BY` still decides the pages. pgorm reads the statement with
PostgreSQL's own parser to do so, and refuses anything but a single `SELECT`
(a `WITH .. SELECT`, a `VALUES` list and a `UNION` count as one) with
`ConstructionError`, as `paginate` is called:

```python
import asyncio
import os
from decimal import Decimal
from typing import cast

from pgorm import ConstructionError, ModelRecord, Pool, RawSQL, Record, from_statement

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        priced = from_statement(RawSQL("SELECT name, price FROM cake WHERE bakery_id = $1 ORDER BY price", [1]))
        pages = priced.paginate(2)
        assert await pages.num_items(pool) == 3
        first = cast(list[Record], await pages.fetch_page(pool, 0))
        assert dict(first[0]) == {"name": "Lemon Drizzle", "price": Decimal("3.20")}
        assert await priced.count(pool) == 3

        cakes = Cake.find_by_statement(RawSQL("SELECT * FROM cake ORDER BY id")).paginate(2)
        third = cast(list[ModelRecord], await cakes.fetch_page(pool, 2))
        assert [cake["name"] for cake in third] == ["Chocolate Torte"]

        try:
            from_statement(RawSQL("DELETE FROM cake")).paginate(2)
        except ConstructionError as error:
            assert "pagination needs a SELECT" in str(error)
        else:
            raise AssertionError("a DELETE has no pages")


asyncio.run(main())
```

## Streams

`stream(db)` runs a query and gives a `RowStream`, an asynchronous iterator
over its rows. Each pull reads one row, decoded as the query decodes it, so a
large result never sits in memory whole, and while nothing pulls, the driver's
bounded buffer and the socket hold the server back. Only one pull runs at a
time. `stream` exists on a model query and on a [graph](07-relations.md), and
`pool.stream(statement)` and `connection.stream(statement)` stream any
statement's rows as plain `Record`s, SQL written by hand included. A stream
is an `async with` block, which closes it however the block ends; `aclose()`
closes one explicitly.

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Pool, RawSQL

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        total = Decimal(0)
        async with await Cake.find().order_by(Cake.col("id").asc()).stream(pool) as cakes:
            async for cake in cakes:
                total += cake["price"]
        assert total == Decimal("20.40")

        moonlight = Cake.select("name").where(Cake.col("bakery_id") == 2).order_by(Cake.col("name").asc())
        async with await moonlight.stream(pool) as rows:
            names = [cake["name"] async for cake in rows]
        assert names == ["Carrot Cake", "Vanilla Sponge"]

        # Any statement streams as plain records.
        bakers = RawSQL("SELECT name FROM baker WHERE bakery_id = $1 ORDER BY name", [1])
        async with await pool.stream(bakers) as records:
            assert [record["name"] async for record in records] == ["Ada", "Bo"]


asyncio.run(main())
```

A `RowStream` is not a server-side cursor and does not bound the work the
server does: it reads the rows of one statement as they arrive. Here Python
differs from Rust, whose `stream_partial_model` streams a `PartialModel`: in
Python a model declared for the fields wanted ([Reading
rows](05-reading.md#decoding-into-your-own-types)) streams as any model does.

### Who holds the connection

Here Python differs from Rust in what a stream holds. A Rust stream holds no
borrow of its connection, only reads from it. A Python stream holds the
connection it reads from until it ends:

- A pool's stream checks out a connection of its own and keeps it until the
  stream ends.
- A connection's stream reserves that connection. Any other statement on it
  meanwhile raises `LifecycleError`, rather than waiting.
- A transaction's stream reserves the transaction, as Rust's `query_raw`
  inside one reads from it. Any other statement on it meanwhile raises
  `LifecycleError`, and the block ending while the stream is open discards
  the transaction.

A stream read to its last row frees its connection, or its transaction, for
reuse. A stream closed before its last row, by `aclose()` or by leaving its
`async with` block, or abandoned to the garbage collector, discards its
connection instead, since the connection is still in the middle of the
statement's rows; a transaction's stream discards the transaction with it,
nothing committed. Here Python
differs from Rust, where dropping a stream early leaves the connection usable
while the server runs the query to its end. In Python a connection's stream
closed early leaves that connection closed, and the pool opens a new one when
it next needs one:

```python
import asyncio
import os

from pgorm import LifecycleError, Pool, RawSQL


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        async with pool.connection() as connection:
            # Read to its end, a stream leaves its connection usable.
            async with await connection.stream(RawSQL("SELECT generate_series(1, 3) AS n")) as rows:
                assert [row["n"] async for row in rows] == [1, 2, 3]
            assert (await connection.one(RawSQL("SELECT 1 AS one")))["one"] == 1

            numbers = RawSQL("SELECT generate_series(1, 1000000) AS n")
            async with await connection.stream(numbers) as rows:
                async for row in rows:
                    try:
                        await connection.one(RawSQL("SELECT 1 AS one"))
                    except LifecycleError:
                        pass
                    else:
                        raise AssertionError("the stream reserves its connection")
                    if row["n"] == 3:
                        break
            # Closed with rows left, the stream discarded its connection.
            assert connection.closed

        assert (await pool.one(RawSQL("SELECT 1 AS one")))["one"] == 1


asyncio.run(main())
```

A row that fails to decode raises `DecodeError` and closes the stream, never
ending it as though the rows had run out. Here Python differs from Rust,
where the failed row is one `Err` in the stream and the rows after it still
come. A database error part-way, such as a division by zero in a later row,
raises `DatabaseError` with its SQLSTATE after the rows before it.

### Cancellation

A stream follows asyncio's cancellation, as every operation does ([Python and
asyncio](02-asyncio.md)). Cancelling the task that waits on a pull closes the
stream and discards its connection, whose state is then unknown, and nothing
runs the statement again. Closing the
pool or the connection a stream reads from interrupts it, even one idle
between pulls, and its next pull raises `LifecycleError`. A stream belongs to
the event loop it was opened on, and refuses truth testing: `if rows:`
raises `ConstructionError`, since a stream's truth would say nothing of
whether rows remain.
