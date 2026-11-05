# Raw SQL

The query builders cover most statements, but not all of them, and some SQL
is clearer written out. pgorm runs SQL you write as readily as SQL it builds:
the statement is text, its values travel beside it as parameters, and the
rows come back as the same records a built query returns.

This chapter runs SQL text with `RawSQL`, reads its rows, decodes them into
models, and says what Python has in place of the Rust guide's compile-time
checks, the `sql!` and `prql!` macros.

## Running a statement

`RawSQL(sql, values)` holds a statement: its SQL text, with its parameters
written `$1`, `$2` and so on, and the values for them, a list or a tuple. It
is a value like a built query, and the pool, a connection and a transaction
run it with the same methods:

| Method | Runs the statement and returns |
| --- | --- |
| `execute` | the number of rows it changed |
| `one` | its one row, and a `DecodeError` for any other number |
| `optional` | its row, or `None` for none, and a `DecodeError` for more than one |
| `query` | all of its rows, as a list |
| `stream` | a stream of its rows, read as they arrive (on a pool or a connection) |

Here Python differs from Rust. Rust's methods take the SQL and a slice of
parameters as two arguments; Python puts the two in one `RawSQL`, so a
statement written once can be kept, passed around and run anywhere a built
query can. A plain `str` is not a statement: passing one where a statement
goes raises `TypeError`, so SQL text runs only where the program wrapped it
in a `RawSQL` on purpose.

Each value is read by its Python type, as every value pgorm binds is: an
`int` is an `i64`, a `Decimal` a `decimal`, a `str` `text`, and
`Value(7, "i16")` the kind it names. [Values and types](13-values.md) has the
whole table. The server gives each parameter the type its place in the
statement asks for, and the value is converted to that type as it is bound:
an `int` beside an `integer` column is sent as an `integer`, and one too
large for it raises `ConstructionError`.

A row is a `Record`: a read-only mapping from column name to value, in column
order. `row["name"]` reads a column by name, `row.values()` reads them all by
position, `row.fields` describes each column, its name and its type among
them, and `row.tagged("name")` gives one column as a `Value` carrying its
kind:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Pool, RawSQL


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        raised = await pool.execute(
            RawSQL("UPDATE cake SET price = price + $1 WHERE bakery_id = $2", [Decimal("0.50"), 2])
        )
        assert raised == 2

        row = await pool.one(RawSQL("SELECT name, price FROM cake WHERE id = $1", [3]))
        assert (row["name"], row["price"]) == ("Carrot Cake", Decimal("4.30"))
        assert row.values() == ("Carrot Cake", Decimal("4.30"))
        assert [field.type_name.name for field in row.fields] == ["varchar", "numeric"]

        none = await pool.optional(RawSQL("SELECT name FROM cake WHERE id = $1", [99]))
        assert none is None


asyncio.run(main())
```

Every statement is prepared on its connection once, and reused the next time
the same text runs there, so a statement written once and run often is parsed
and planned once per connection. [Statement caches and
DDL](19-statement-caches.md) covers what that means after a schema change.

Here Python differs from Rust, which also has `batch_execute` for a script of
`;`-separated statements sent as it is. Python has no such method yet: every
statement it sends is prepared, and PostgreSQL refuses to prepare more than
one command at a time, so a script is refused as it runs. Run each statement
as a `RawSQL` of its own:

```python
import asyncio
import os

from pgorm import DatabaseError, Pool, RawSQL


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        script = "CREATE TABLE oven (id integer PRIMARY KEY); CREATE TABLE tray (id integer PRIMARY KEY)"
        try:
            await pool.execute(RawSQL(script))
        except DatabaseError as error:
            assert error.sqlstate == "42601"
        else:
            raise AssertionError("a script of two statements is refused")

        for statement in script.split("; "):
            await pool.execute(RawSQL(statement))
        row = await pool.one(RawSQL("SELECT count(*) AS tables FROM pg_tables WHERE tablename IN ('oven', 'tray')"))
        assert row["tables"] == 2


asyncio.run(main())
```

A parameter takes its type from where it stands in the statement. Beside a
column it takes the column's, which is the common case. Where nothing around
it says, the server reads it as `text`, and a number cannot be sent as text:
`SELECT $1 AS n` with the value `3` raises `ConstructionError` as it runs.
Write the type into the SQL, `SELECT $1::integer AS n`, and the value binds.

`stream` reads the rows as they arrive instead of collecting them, for a
result too large to hold at once. It returns a `RowStream`, an async iterator
that holds its connection until the last row is read or the stream is closed,
so use it as an `async with` block when the loop might stop early.
[Pages, cursors and streams](06-pagination.md) covers streams in full:

```python
import asyncio
import os

from pgorm import Pool, RawSQL


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        query = RawSQL("SELECT name FROM cake WHERE gluten_free = $1 ORDER BY name", [True])
        async with await pool.stream(query) as rows:
            names = [row["name"] async for row in rows]
        assert names == ["Chocolate Torte", "Lemon Drizzle", "Vanilla Sponge"]


asyncio.run(main())
```

A transaction runs `execute`, `query`, `one` and `optional`. Here Python
differs from Rust, whose `query_raw` streams inside a transaction too: a
Python transaction has no `stream` yet, and its connection is held by the
transaction until the block ends, so inside one, read the rows with
`query`.

## Decoding rows into models

A raw statement's rows are records, read by column name, so they need no
decode step to be read. Rust decodes such rows into a struct of its own
choosing; in Python, build one from the record, by name from the mapping or
by position from its values.

To read a row as a model's record, `ModelRecord(model, row, fields)` decodes
it: the row's columns must be the fields named, in that order, or every
field of the model in its declared order when `fields` is empty, and each
column is checked against the kind its field declares. A row that does not
match raises `DecodeError`.

pgorm adds nothing to SQL you wrote, so `one` and `optional` add no `LIMIT`:
a statement that returns more than one row is an error there, and a query for
the first of many says `LIMIT 1` itself.

```python
import asyncio
import os
from dataclasses import dataclass
from decimal import Decimal

from pgorm import DecodeError, ModelRecord, Pool, RawSQL

from bakery import Cake


@dataclass
class Menu:
    bakery: str
    cakes: int


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        rows = await pool.query(RawSQL(
            "SELECT bakery.name AS bakery, count(cake.id) AS cakes"
            " FROM bakery JOIN cake ON cake.bakery_id = bakery.id"
            " WHERE cake.price < $1"
            " GROUP BY bakery.name ORDER BY bakery.name",
            [Decimal("5.00")],
        ))
        menus = [Menu(**row) for row in rows]
        assert menus == [Menu("Moonlight", 2), Menu("Sunrise", 2)]

        cheapest = await pool.one(RawSQL("SELECT name, price FROM cake ORDER BY price LIMIT 1"))
        assert tuple(cheapest.values()) == ("Vanilla Sponge", Decimal("2.90"))

        # Five rows, where `one` expects exactly one.
        try:
            await pool.one(RawSQL("SELECT name FROM cake"))
        except DecodeError:
            pass
        else:
            raise AssertionError("one refuses five rows")

        row = await pool.one(RawSQL("SELECT * FROM cake WHERE id = $1", [2]))
        lemon = ModelRecord(Cake, row, ())
        assert lemon["flavor"] == "lemon"
        assert lemon["name"] == "Lemon Drizzle"


asyncio.run(main())
```

Here Python differs from Rust. A Rust entity's model reads a PostgreSQL enum
column from its text, so SQL written for it casts the column,
`flavor::text AS flavor`. A Python model reads the column as it is, so
`SELECT *` decodes into `Cake` with no cast.

Rust starts the same decode from other places too: `Select::from_raw_sql` on
an entity's `find()`, `FromQueryResult::find_by_statement` and the
`SelectorRaw` constructors, each taking the SQL text and its values in the
one shape its connection methods take them. Python has
neither yet: a model's queries are built, not written, and `ModelRecord` is
how a written statement's row becomes a model's record. A registered Rust
entity's model, an `EntityModel`, cannot be decoded from a raw statement's
row in Python yet.

## Checking a decode target against a statement

Rust's `verify` asks PostgreSQL to describe a statement's result without
running it, and checks a decode target's columns against that description,
so a struct naming a column the statement does not return fails before any
row arrives. That is not available in Python yet. A model's record is checked
as each row is decoded, so a mismatch surfaces with the first row, and a
statement that happens to return no rows does not show it. A test that runs
the statement over data it returns rows for is the check Python has.

## Statements the query builder built

A statement built with [the query builder](11-query-builder.md) runs as it
is: `await pool.execute(statement)`. Rust renders it to the same
`(sql, values)` pair a raw statement is, and wraps each value to run it.
Python needs neither step, and `inspect()` shows the SQL and values that
running it sends. What `inspect()` returns, a `Compiled`, runs too, and its
SQL and values make a `RawSQL` when a statement the builder began needs
editing as text:

```python
import asyncio
import os

from pgorm import Pool, RawSQL, Table, update

cake = Table("cake")


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        rename = update(cake).set("name", "Lemon Drizzle Loaf").where(cake.col("id") == 2)
        compiled = rename.inspect()
        assert compiled.sql == 'UPDATE "cake" SET "name" = $1 WHERE "cake"."id" = $2'
        assert [value.value for value in compiled.params] == ["Lemon Drizzle Loaf", 2]

        assert await pool.execute(rename) == 1
        assert await pool.execute(compiled) == 1
        assert await pool.execute(RawSQL(compiled.sql, compiled.params)) == 1


asyncio.run(main())
```

## Values written into the SQL

A statement sends its values as parameters, never as text, and that is the
form to run. For a log line or a test, `inline_sql()` returns the statement
with each value written in as an escaped literal. It is pgorm's own
substitution, done in Rust: it finds the placeholders as PostgreSQL's lexer
would, leaving those inside quoted strings, dollar-quoted bodies and comments
alone, and writes each value as the query builder writes a literal. A
placeholder that names no value, or a value no placeholder names, raises
`ConstructionError`. `inspect()` leaves the template as it was written:

```python
from pgorm import ConstructionError, RawSQL

query = RawSQL("SELECT $1::int8, $2::text", [42, "O'Brien"])
assert query.inspect().sql == "SELECT $1::int8, $2::text"
assert query.inline_sql() == "SELECT 42::int8, E'O\\'Brien'::text"

try:
    RawSQL("SELECT $2", [1]).inline_sql()
except ConstructionError:
    pass
else:
    raise AssertionError("$2 names no value")
```

A built statement has no inlined form of its own; Rust's `to_string` is one.
Pass its `inspect()` to `RawSQL` and call `inline_sql()` on that.

There is no fallback from the builders to raw SQL: an operation a builder
does not have is not quietly written as text. A `RawSQL` is always a
statement your program chose to write.

## SQL checked before it runs

Rust's `sql!` holds a SQL literal to PostgreSQL's grammar while the crate
compiles, so a typo is a compile error. Python has no compile step, and the
package parses no SQL before sending it: constructing a `RawSQL` refuses a
NUL byte in its text and reads no further, and the server reports a syntax
error when the statement runs, as a `DatabaseError` whose `sqlstate` is
`42601`. The connection stays usable afterwards:

```python
import asyncio
import os

from pgorm import DatabaseError, Pool, RawSQL


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        typo = RawSQL("SELEC name FROM baker WHERE bakery_id = $1", [1])
        try:
            await pool.query(typo)
        except DatabaseError as error:
            assert error.sqlstate == "42601"
            assert error.message == 'syntax error at or near "SELEC"'
        else:
            raise AssertionError("the server refuses the typo")

        fixed = RawSQL("SELECT name FROM baker WHERE bakery_id = $1 ORDER BY name", [1])
        assert [row["name"] for row in await pool.query(fixed)] == ["Ada", "Bo"]


asyncio.run(main())
```

`sql!` is not available in Python, nor is any check of SQL text before it
is sent. `sql!` checks the grammar alone and knows nothing of the schema;
the server checks both when the statement runs. So in Python a test that runs
each statement once is what moves a typo, and a missing table or column,
earlier than production.

## PRQL

Rust's `prql!` compiles a [PRQL](https://prql-lang.org) literal to SQL while
the crate compiles. Python cannot compile PRQL text: no function in the
package takes it. What it has is [the pipeline](12-pipeline.md), the same
PRQL compiler with a builder in front of it, whose steps are Python calls.
Rust's `from cake | filter price < $1 | sort name | select {name, price}` is
this pipeline, whose `filter_with` step binds the price as `$1`:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Pool
from pgorm import pipeline as pl

from bakery import Cake

price, name = pl.col("cake", "price"), pl.col("cake", "name")


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        under = Decimal("4.00")
        cheap = (
            pl.from_(Cake)
            .filter_with(lambda binder: price < binder.bind(under))
            .sort(name)
            .select(name, price)
        )

        compiled = cheap.inspect()
        assert compiled.sql == "SELECT name, price FROM cake WHERE price < $1 ORDER BY name"
        assert [value.value for value in compiled.params] == [under]

        rows = await cheap.all(pool)
        assert [row["name"] for row in rows] == ["Carrot Cake", "Lemon Drizzle", "Vanilla Sponge"]


asyncio.run(main())
```

The pipeline is compiled when it is inspected or run, before anything is
sent, and a pipeline the compiler refuses raises `ConstructionError` there.
That checks the pipeline, not SQL text, and like `prql!` it does not know the
schema: a column that does not exist is the server's to refuse.
