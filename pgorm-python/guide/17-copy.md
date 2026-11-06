# COPY

`COPY` moves rows between a table and the client in bulk. It is the fastest
way to load many rows into PostgreSQL, or to read many out: the rows stream
over the connection as one data flow, with no statement per row. pgorm runs
it through Rust's `CopyTrait`, and Python reaches it through four methods,
`copy_in`, `copy_out`, `copy_in_rows` and `copy_out_rows`, each on a `Pool`,
a `Connection` and a `Transaction` (and so a savepoint). A pool's COPY runs on
a connection it lends for the COPY alone.

A COPY is described by a statement built like any other:
`CopyFrom(table, format)` for `COPY .. FROM STDIN`, rows sent into a table,
and `CopyTo(source, format)` for `COPY .. TO STDOUT`, rows read out of a
table or a query. Each takes its format, one of PostgreSQL's three, when it
is built:

| Format | The data |
| --- | --- |
| `"text"` | tab-separated lines, `\N` for NULL |
| `"csv"` | comma-separated values with quoting |
| `"binary"` | each field in its type's binary form |

The options a format takes are methods of its statement. A CSV-only option
such as `quote` set on a text COPY raises `ConstructionError` as it is set,
and a binary COPY has no delimiter, header or NULL string to set. A COPY
takes no bound parameters, so `inspect()` gives its one rendering, with any
value in it written as an escaped literal, and no parameters.

The four methods are the only way a COPY runs. Its data flows beside the
statement, which `execute`, `query` and the other statement methods have no
route for, so a `RawSQL` holding a `COPY` given to one of them raises
`ConstructionError` before anything is sent, saying to use `copy_in` or
`copy_out`, as Rust refuses it with `Error::CopyStatement`:

```python
import asyncio
import os

from pgorm import ConstructionError, Pool, RawSQL


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        try:
            await pool.query(RawSQL("COPY cake TO STDOUT"))
        except ConstructionError as refusal:
            assert "copy_in or copy_out" in str(refusal)
        else:
            raise AssertionError("a COPY ran as a statement")


asyncio.run(main())
```

Here Python differs from Rust in spelling. Rust's format is a type,
`CopyText`, `CopyCsv` or `CopyBinary`, so a missing option is a compile
error. Python's is a string, and the stubs carry it as the statement's type
parameter, `CopyFrom[Literal["csv"]]`, so mypy reports an option the format
lacks before the program runs. `CopyFrom` takes a `Table`, schema-qualified
or not, but not one given an alias. A model's table is its `table`
attribute, so `CopyFrom(Cake.table, "text")` copies into the bakery's cakes.

A COPY statement is not a query: `execute`, `query`, `one` and `optional`
refuse one with `TypeError`.

## Loading rows

`copy_in(statement, source)` runs a `COPY .. FROM STDIN` and returns the
number of rows it inserted. The source is the format's own encoding of the
rows, as bytes-like data (`bytes`, `bytearray`, `memoryview` or another
contiguous buffer), or a sync or async iterable of it, split anywhere: a
chunk need not end at a row. The source is pulled only as the COPY takes
data. Small chunks are gathered up to 64 KiB and large ones sliced at 256 KiB,
and at most four wait for the connection, so a large generator streams rather
than piling up in memory.

Here Python differs from Rust: Rust's `copy_in` returns a sink the program
sends chunks into and then `finish`es. Python's takes the source and pulls
it, and returns once the COPY is done.

The bakery's new cakes arrive as tab-separated text. A column list names the
columns each line holds; the others take their defaults, so the database
assigns each cake its id:

```python
import asyncio
import os

from pgorm import CopyFrom, Pool

from bakery import Cake


async def main() -> None:
    copy = CopyFrom(Cake.table, "text").columns("name", "flavor", "price", "gluten_free", "bakery_id")
    assert copy.inspect().sql == (
        'COPY "cake" ("name", "flavor", "price", "gluten_free", "bakery_id") FROM STDIN WITH (FORMAT text)'
    )
    assert copy.inspect().params == []

    async with Pool(os.environ["DATABASE_URL"]) as pool:
        inserted = await pool.copy_in(copy, [
            b"Lemon Tart\tlemon\t4.10\tf\t1\n",
            b"Carrot Muffin\tcarrot\t2.20\tt\t2\n",
        ])
        assert inserted == 2
        assert await Cake.find().count(pool) == 7


asyncio.run(main())
```

A `str` is not bytes, and `copy_in` raises `TypeError` for one, as for an
item a source yields that is not bytes: encode text first. It never encodes
it for you.

A COPY in progress holds its connection: a statement sent while it is open
would wait behind data not yet sent, and so would the `COMMIT` of a
transaction the COPY runs in. In Rust the borrow makes that a compile error.
In Python, on a connection or a transaction, any other work raises
`LifecycleError` until the COPY ends:

```python
import asyncio
import os
from collections.abc import AsyncIterator

import pgorm
from pgorm import CopyFrom, Pool, RawSQL

from bakery import Bakery


async def main() -> None:
    copy = CopyFrom(Bakery.table, "text").columns("name", "city")
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        async with pool.connection() as connection:
            refused = []

            async def bakeries() -> AsyncIterator[bytes]:
                yield b"Noon\tStavanger\n"
                # The COPY still holds the connection.
                try:
                    await connection.one(RawSQL("SELECT count(*) AS bakeries FROM bakery"))
                except pgorm.LifecycleError as error:
                    refused.append(str(error))
                yield b"Dusk\tBodo\n"

            assert await connection.copy_in(copy, bakeries()) == 2
            assert refused == ["connection already has an active operation"]

            # Once the COPY ends, the connection runs statements again.
            row = await connection.one(RawSQL("SELECT count(*) AS bakeries FROM bakery"))
            assert row["bakeries"] == 4


asyncio.run(main())
```

A row the server refuses, such as one that breaks a constraint or a field
that does not parse, is not reported while the data is still being sent,
since the protocol carries no error back to a client that is sending. It is
raised once the source is spent and the COPY finishes, as the
`DatabaseError` it is, and the COPY then inserts nothing.

A source that raises aborts the COPY, and its exception propagates
unchanged. So does an item that is not bytes, and so does cancelling the
task running `copy_in` before the data is all sent. No row is written.
Outside a transaction the connection stays usable; inside one the
transaction is aborted, as by any failed statement, and refuses statements
(`25P02`) until it is rolled back:

```python
import asyncio
import os
from collections.abc import Iterator

from pgorm import CopyFrom, Pool, UniqueViolationError

from bakery import Bakery


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # Bakery 1 exists, so the second line is refused, and the first with it.
        try:
            await pool.copy_in(
                CopyFrom(Bakery.table, "text").columns("id", "name", "city"),
                b"9\tNoon\tStavanger\n1\tDusk\tBodo\n",
            )
        except UniqueViolationError as error:
            assert error.constraint == "bakery_pkey"
        else:
            raise AssertionError("bakery 1 exists")

        # A source that fails part way.
        def bakeries() -> Iterator[bytes]:
            yield b"Noon\tStavanger\n"
            raise ValueError("the file ended early")

        try:
            await pool.copy_in(CopyFrom(Bakery.table, "text").columns("name", "city"), bakeries())
        except ValueError as error:
            assert str(error) == "the file ended early"
        else:
            raise AssertionError("the source raised")

        assert await Bakery.find().count(pool) == 2


asyncio.run(main())
```

Cancelling the task while the COPY finishes, after its data is sent, leaves
the outcome unknown: outside a transaction the rows may already be
committed. The connection, or the whole transaction, is then discarded rather
than used again.

## Reading rows

`copy_out(statement)` starts a `COPY .. TO STDOUT` and returns a
`CopyOutStream`, an async iterator of `bytes` chunks in the statement's
format. A chunk joins the data already received, up to 64 KiB, and need not
end at a row; the binary format is one byte stream. At most four messages
wait unread, so a slow reader holds the server back rather than filling
memory.

A `CopyTo` reads a whole table, or the rows of a `Select`. A `Select` is also
how to read some of a table's columns or rows, and a model query's
`statement` is one. Here the Sunrise bakery's price list is written as CSV
with a header line:

```python
import asyncio
import os

from pgorm import CopyTo, Pool

from bakery import Cake


async def main() -> None:
    price_list = (
        Cake.select("name", "price")
        .where(Cake.col("bakery_id") == 1)
        .order_by(Cake.col("name").asc())
        .statement
    )
    copy = CopyTo(price_list, "csv").header()
    assert copy.inspect().sql == (
        'COPY (SELECT "cake"."name", "cake"."price" FROM "cake" WHERE "cake"."bakery_id" = 1 '
        'ORDER BY "cake"."name" ASC) TO STDOUT WITH (FORMAT csv, HEADER true)'
    )

    async with Pool(os.environ["DATABASE_URL"]) as pool:
        async with await pool.copy_out(copy) as chunks:
            csv = b"".join([chunk async for chunk in chunks]).decode()
        assert csv == "name,price\nChocolate Fudge,4.50\nChocolate Torte,6.00\nLemon Drizzle,3.20\n"


asyncio.run(main())
```

The model query projects its columns qualified by the table, as every model
query does, where Rust's example names them bare; the data is the same.

A write is a source too, when it has a `RETURNING` list: COPY TO copies the
rows it returns. `CopyTo` takes a built `Insert`, `Update`, `Delete` or
`Merge`, and raises `ConstructionError` for one without a `RETURNING` list,
which returns nothing to copy. A `Select` copies as it would run, a tie
limit (`limit_with_ties`) and a `WINDOW` clause included. One PostgreSQL
would refuse, with a tie limit but no `ORDER BY` to measure its ties on, or a
`WINDOW` clause that names a window twice or refers to one it does not
define, raises `ConstructionError` there too. Here Python differs from Rust,
which refuses that source when `copy_out` runs it rather than when the COPY
is built.

Reading to the end releases the connection, or the transaction, for other
work. Here Python differs from Rust too: a Rust stream dropped early leaves
the connection to read and discard the rest of the data. A Python stream
closed before its end, by `aclose()`, by leaving its `async with` block, or by
an exception inside the `async for`, discards the connection, and with it any
transaction, since the server would otherwise keep sending. Use the
`async with` form when reading might stop early. A pool's `copy_out` lends a
connection the stream owns until its end or close, as a pool's `stream` does.

## The text and CSV options

The two text formats share most of their options, and CSV adds a few of its
own:

| Option | COPY FROM | COPY TO |
| --- | --- | --- |
| `columns(*names)`, `where(predicate)`, `freeze()` | every format | none |
| `delimiter(c)`, `null_string(s)`, `header()`, `encoding(e)` | text, CSV | text, CSV |
| `default_string(s)`, `header_match()`, `on_error_ignore(reject_limit, log_verbosity)` | text, CSV | none |
| `quote(c)`, `escape(c)` | CSV | CSV |
| `force_not_null(*columns)`, `force_null(*columns)`, and `force_not_null_all()`, `force_null_all()` | CSV | none |
| `force_quote(*columns)`, `force_quote_all()` | none | CSV |

What each does:

- `delimiter` separates the fields, and `null_string` is the field that
  means NULL; `default_string` is the field that means the column's
  default.
- `header` skips a header line on COPY FROM and writes one on COPY TO;
  `header_match` also checks that the line names the columns.
- `encoding` is the data's character encoding.
- `on_error_ignore` skips a row whose field does not convert rather than
  failing the COPY. Its `reject_limit` bounds how many rows may be skipped
  before the COPY fails after all, and its `log_verbosity` how much the
  server reports about them: `"verbose"` a notice for each row skipped, and
  `"silent"` none.
- `quote` and `escape` are CSV's quoting and escape characters.
- `force_null` and `force_not_null` read a field matching the NULL string as
  NULL even when quoted, or never as NULL, for the columns given;
  `force_quote` quotes every non-NULL field of the columns given on COPY TO.
- `where` keeps only the rows its condition holds for, and `freeze` writes
  the rows frozen, into a table created or emptied in the same transaction.

`delimiter`, `quote` and `escape` take a one-character string. The builders
are immutable, like every other statement: each option returns a new one.

Here Python differs from Rust in spelling: Rust's `on_error_ignore` takes a
`CopyIgnore`, and Python takes its two settings as arguments. Here a price
list from a supplier has one line whose price is not a number, and the COPY
loads the others:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import CopyFrom, Pool

from bakery import Cake


async def main() -> None:
    copy = (
        CopyFrom(Cake.table, "csv")
        .columns("name", "flavor", "price", "gluten_free", "bakery_id")
        .header()
        .quote("'")
        .on_error_ignore()
    )
    assert copy.inspect().sql == (
        'COPY "cake" ("name", "flavor", "price", "gluten_free", "bakery_id") '
        "FROM STDIN WITH (FORMAT csv, HEADER true, QUOTE E'\\'', ON_ERROR ignore)"
    )

    async with Pool(os.environ["DATABASE_URL"]) as pool:
        inserted = await pool.copy_in(copy, (
            b"name,flavor,price,gluten_free,bakery_id\n"
            b"'Vanilla Slice, iced',vanilla,3.40,false,2\n"
            b"Lemon Bun,lemon,ask,false,1\n"
        ))
        # The second line's price did not convert, so it was skipped.
        assert inserted == 1

        slice_ = await Cake.find().where(Cake.col("name") == "Vanilla Slice, iced").one(pool)
        assert slice_["price"] == Decimal("3.40")


asyncio.run(main())
```

## Rows of values

The binary format sends each field in its type's binary form, the form a
bound parameter travels in, so a binary COPY can carry rows of values with no
text to write or parse. The format itself names no column types, so pgorm
asks the server for them before the COPY starts, by preparing (never running)
a `SELECT` of the COPY's columns, or the COPY TO's own query. A description
the server refuses raises its `DatabaseError` and leaves the connection
usable.

`copy_in_rows(statement, rows)` and `copy_out_rows(statement)` do that, then
run the COPY. They take a binary statement alone, typed
`CopyFrom[Literal["binary"]]` and `CopyTo[Literal["binary"]]`; one in
another format raises `TypeError`.

- `copy_in_rows` takes a sync or async iterable of rows, each a sequence
  holding a value for every column in order, pulls them as the COPY takes
  them, a batch at a time, and returns the rows inserted. Give each row as a
  tuple or a list, its values in the columns' order. Rows given as text,
  bytes or anything not iterable, and a row that is not iterable, raise
  `TypeError`.
  `None` is NULL, and every other value is read as a parameter is, a plain
  value by its kind or a `Value` by its own, and written against its
  column's type. A row of another width, or a value its column's type does
  not take, raises `ConstructionError` naming the columns or the column, and
  aborts the COPY, as a failing source does, so nothing is written.
- `copy_out_rows` returns a `CopyRowStream`, an async iterator of tuples,
  each field decoded by its column's type as a query's result column is.
  Its `columns` attribute names the columns. A field no Python value holds
  raises `DecodeError` and closes the stream.

`copy_in_columns(statement)` and `copy_out_columns(statement)` give that
description without running the COPY, for a statement in any format: a list
of `CopyColumn`s in the COPY's order, each with its `name`, its `type` as a
`TypeName` schema-qualified as the server names it, and a `money` column's
`money_places`.

Given no rows, `copy_in_rows(statement)` returns a `CopyRowSink`, Rust's own,
which holds the COPY until it ends. Its `columns` are the described ones.
`send(row)` writes one row, and `send_model(record)` a model's record, each
of the COPY's columns taken from the record's field whose column it is. A
row the columns refuse raises `ConstructionError` before anything of it is
written, and the sink goes on taking rows. `finish()` completes the COPY
and returns the rows written; leaving the sink's `async with` block without
`finish()` abandons the COPY, so nothing is written. A `CopyRowStream`'s
`models(model)` reads its rows as the model's records instead of tuples.

Here every cake at Sunrise is copied out as records and copied back in
twice: once with `send_model`, as new cakes at Sunrise, and once with `send`,
as new cakes at Moonlight, since a record's fields are its own and a row
with another bakery is a row of the program's. The COPY FROM lists every
column but the id, so the database assigns new ids:

```python
import asyncio
import os

from pgorm import CopyFrom, CopyTo, Pool

from bakery import Cake


async def main() -> None:
    sunrise = CopyTo(Cake.find().where(Cake.col("bakery_id") == 1).statement, "binary")
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        stream = await pool.copy_out_rows(sunrise)
        cakes = [cake async for cake in stream.models(Cake)]
        assert len(cakes) == 3

        into = CopyFrom(Cake.table, "binary").columns("name", "flavor", "price", "gluten_free", "bakery_id")
        columns = await pool.copy_in_columns(into)
        assert [column.type.name for column in columns] == ["varchar", "flavor", "numeric", "bool", "int4"]

        async with pool.transaction() as tx:
            async with await tx.copy_in_rows(into) as sink:
                for cake in cakes:
                    await sink.send_model(cake)
                    await sink.send([cake["name"], cake["flavor"], cake["price"], cake["gluten_free"], 2])
                assert await sink.finish() == 6

        at_sunrise = await Cake.find().where(Cake.col("bakery_id") == 1).count(pool)
        at_moonlight = await Cake.find().where(Cake.col("bakery_id") == 2).count(pool)
        assert (at_sunrise, at_moonlight) == (6, 5)


asyncio.run(main())
```

Here Python differs from Rust in one respect: `copy_in_rows(statement, rows)`,
given rows, pulls them as the COPY takes data and aborts the whole COPY at a
row that does not fit, as a failing source does; a sink refuses only that
row.

A row's values are checked against the column types as it is written, and a
row that does not fit stops the COPY with nothing written:

```python
import asyncio
import os

from pgorm import ConstructionError, CopyFrom, CopyTo, Pool

from bakery import Bakery


async def main() -> None:
    bakeries = CopyFrom(Bakery.table, "binary").columns("name", "city")
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # A row of the wrong width aborts the COPY: neither row is written.
        try:
            await pool.copy_in_rows(bakeries, [("Daybreak", "Trondheim"), ("Dusk",)])
        except ConstructionError as error:
            assert str(error) == "a COPY row of 1 values for the COPY's 2 columns (name, city)"
        else:
            raise AssertionError("a bakery has a name and a city")

        assert await pool.copy_in_rows(bakeries, [("Daybreak", "Trondheim")]) == 1

        async with await pool.copy_out_rows(CopyTo(Bakery.table, "binary")) as rows:
            every = [row async for row in rows]
        assert every[2] == (3, "Daybreak", "Trondheim")


asyncio.run(main())
```

Each field reads as its column's type reads in a query
([Values and types](13-values.md)): the text types and an enum's label as
`str`, a `numeric` as a `Decimal`, an array as a list, and so on. A `numeric`
field reads exactly, its scale included, and one a `Decimal` cannot hold
exactly is refused rather than rounded. A `money` column reads and writes as
a `Decimal` in its currency's units. A query cannot read a `money` column at
all, and raises `DecodeError` for one: cast it to `numeric` there.

A row read out writes back in as it is. A value that says less than its
column is written as the column's type: a JSON number or list into a `jsonb`
column, an empty list into an array column, a `Range` into a range column.
Here every cake is copied into a table of the same columns:

```python
import asyncio
import os

from pgorm import CopyFrom, CopyTo, Pool, RawSQL, Table


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await pool.execute(RawSQL("CREATE TABLE cake_copy (LIKE cake)"))
        cakes = [row async for row in await pool.copy_out_rows(CopyTo(Table("cake"), "binary"))]
        assert await pool.copy_in_rows(CopyFrom(Table("cake_copy"), "binary"), cakes) == len(cakes)
        row = await pool.one(RawSQL(
            "SELECT count(*) AS n FROM (SELECT * FROM cake EXCEPT ALL SELECT * FROM cake_copy) AS differ"
        ))
        assert row["n"] == 0


asyncio.run(main())
```

## Encoding rows yourself

`CopyRowEncoder` and `CopyRowDecoder` are the binary format's two sides,
Rust's own, over a list of `CopyColumn`s, for code that moves the bytes
itself: writing a COPY file to disk, or reading one produced elsewhere. A
program builds a column of a built-in type by its name, as `pg_type` spells
it (`"int4"`, `"varchar"`, `"text[]"`); another type comes from a
description. The encoder gives the format's `header()` and `trailer()` and
each row's `encode(row)` as `bytes`, a row checked as `send` checks one. The
decoder takes the data with `push(chunk)`, in chunks split anywhere, gives
each row with `next_row()` once all of it has arrived, `None` until then,
and `finish()` raises `DecodeError` for data that ends part way through a
row:

```python
import asyncio
import os

from pgorm import CopyColumn, CopyFrom, CopyRowDecoder, CopyRowEncoder, CopyTo, Pool

from bakery import Bakery


async def main() -> None:
    columns = [CopyColumn("name", "varchar"), CopyColumn("city", "varchar")]
    encoder = CopyRowEncoder(columns)
    data = (
        CopyRowEncoder.header()
        + encoder.encode(["Noon", "Stavanger"])
        + encoder.encode(["Night Owl", "Tromsø"])
        + CopyRowEncoder.trailer()
    )
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        into = CopyFrom(Bakery.table, "binary").columns("name", "city")
        assert await pool.copy_in(into, data) == 2

        out = CopyTo(Bakery.select("name", "city").statement, "binary")
        decoder = CopyRowDecoder(columns)
        rows = []
        async with await pool.copy_out(out) as chunks:
            async for chunk in chunks:
                decoder.push(chunk)
                while (row := decoder.next_row()) is not None:
                    rows.append(row)
        decoder.finish()
        assert rows[3] == ("Night Owl", "Tromsø")


asyncio.run(main())
```

The bytes also move as any COPY's do. A binary `copy_out` gives the whole
file, header and trailer included, ready to keep, and `copy_in` loads such a
file back:

```python
import asyncio
import os

from pgorm import CopyFrom, CopyTo, Pool

from bakery import Bakery


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        out = CopyTo(Bakery.select("name", "city").statement, "binary")
        async with await pool.copy_out(out) as chunks:
            data = b"".join([chunk async for chunk in chunks])
        # PostgreSQL's binary COPY signature.
        assert data.startswith(b"PGCOPY\n\xff\r\n\x00")

        into = CopyFrom(Bakery.table, "binary").columns("name", "city")
        assert await pool.copy_in(into, data) == 2
        assert await Bakery.find().count(pool) == 4


asyncio.run(main())
```

## When the server refuses a COPY

A COPY the server refuses at its start, such as one naming a table or column
that does not exist, raises its `DatabaseError` from `copy_in` or `copy_out`.
The two directions leave the connection differently:

- A refused COPY TO leaves the connection as it was.
- A refused COPY FROM closes its connection, and with it any transaction
  open on it, which ends without committing. The driver has by then sent a
  message whose answer nothing is waiting for, and that answer would reach
  the next request on the connection, leaving every later answer one request
  out of step. Closing is the only safe way out. Every later call on the
  connection raises `LifecycleError`, and the pool opens a fresh connection
  rather than lend it again.

A generated column takes no data, and PostgreSQL refuses a column list that
names one, with SQLSTATE `42P10`, at the COPY's start. pgorm does not check
the list first, since that would cost a catalogue read before every COPY.
List only the columns the data supplies, or give no list, which leaves the
generated columns out and lets the server compute them. A COPY TO a table
leaves them out too; a `Select` naming them reads them. The description
`copy_in_rows` asks for does not catch one either, since a `SELECT` reads a
generated column: the COPY's start refuses it, and closes the connection.

```python
import asyncio
import os

import pgorm
from pgorm import CopyFrom, DatabaseError, Pool, RawSQL, Table


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        async with pool.connection() as connection:
            try:
                await connection.copy_in(CopyFrom(Table("pastry"), "text"), b"")
            except DatabaseError as error:
                assert error.sqlstate == "42P01"
            else:
                raise AssertionError("there is no pastry table")

            # The connection is closed now.
            assert connection.closed
            try:
                await connection.one(RawSQL("SELECT 1 AS one"))
            except pgorm.LifecycleError as error:
                assert str(error) == "connection is closed"
            else:
                raise AssertionError("the connection was closed")

        # The pool does not lend it again.
        row = await pool.one(RawSQL("SELECT 1 AS one"))
        assert row["one"] == 1


asyncio.run(main())
```
