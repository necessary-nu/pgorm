# Errors

Every call in the package that can fail raises an exception, and the
exceptions pgorm raises derive from one class, `pgorm.PgormError`, an
`Exception`, apart from three described below. An error says what failed in one of three places: the database refused a
statement, the driver refused a value on its way in or out, or pgorm refused
something before sending anything.

Here Python differs from Rust. A Rust call returns a `Result` whose error is
one enum, `pgorm::Error`, and a program matches its variants. A Python call
raises an instance of one of a family of classes, and a program catches the
classes it cares about:

```text
PgormError
├── DatabaseError                  the server refused a statement
│   ├── UniqueViolationError       23505
│   ├── ForeignKeyViolationError   23503
│   ├── CheckViolationError        23514
│   ├── NotNullViolationError      23502
│   └── ExclusionViolationError    23P01
├── ConnectionError                the server could not be reached, or the connection broke
├── ConstructionError              a value or statement pgorm cannot send
│   └── UnsupportedCapabilityError an operation this build does not have
├── DecodeError                    a result that cannot become what was asked for
├── TimeoutError                   no connection came free within the pool's acquire_timeout
├── LifecycleError                 a resource used after it closed, or while another operation holds it
└── InternalError                  a failure no input should cause
```

Three exceptions fall outside the family. An argument of a type the call does
not take raises Python's `TypeError`, a listener capacity that is an `int`
but not a positive one raises `ValueError`, and a cancelled operation raises
asyncio's `CancelledError`. Each is covered below.

`pgorm.ConnectionError` and `pgorm.TimeoutError` are pgorm's own classes, not
Python's built-in exceptions of the same names, and neither derives from
them. Refer to them as `pgorm.ConnectionError` and `pgorm.TimeoutError`, or
import them under those names on purpose: an `except ConnectionError` that
means the built-in one does not catch pgorm's.

## What the database refused

A statement the server rejects raises `DatabaseError`, holding everything
the server reported. Each of these attributes is a `str`, or `None` when the
server sent no such field:

| Attribute | Holds |
| --- | --- |
| `sqlstate` | the SQLSTATE code, such as `42P01` |
| `message` | the server's message, which `str(error)` gives too |
| `severity` | `ERROR`, `FATAL` or `PANIC` |
| `detail`, `hint` | the server's detail and hint lines |
| `schema`, `table`, `column`, `datatype`, `constraint` | the objects the error names |

The refusals a program most often has to answer are constraint violations: a
duplicate email address, an order line for a cake that does not exist. Each
raises a subclass of `DatabaseError` named for it:

| Class | SQLSTATE | Raised by |
| --- | --- | --- |
| `UniqueViolationError` | `23505` | a duplicate in a unique key or primary key |
| `ForeignKeyViolationError` | `23503` | a reference to a row that does not exist, or a delete of a row still referenced |
| `CheckViolationError` | `23514` | a table's or a domain's `CHECK` |
| `NotNullViolationError` | `23502` | a `NULL` in a `NOT NULL` column or domain |
| `ExclusionViolationError` | `23P01` | an `EXCLUDE` constraint, or a key `WITHOUT OVERLAPS` |

Here Python differs from Rust: Rust returns one `Error::Postgres` and
classifies it with `sql_error`, whose `SqlError` variants carry a
`ConstraintViolation`. Python raises the class itself, and the names the
server reported are the error's own attributes. A table's constraint names
its `table` and `constraint`; a `NOT NULL` column names its `table` and
`column` and no constraint; a domain's names the domain as `datatype`.

```python
import asyncio
import os
from decimal import Decimal

from pgorm import (
    ForeignKeyViolationError,
    NotNullViolationError,
    Pool,
    RawSQL,
    UniqueViolationError,
)

from bakery import Cake, Customer


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # Lee's address is taken.
        try:
            await Customer.insert({
                "name": "Lee Two",
                "email": "lee@example.com",
                "notes": None,
            }).execute(pool)
        except UniqueViolationError as error:
            assert (error.table, error.constraint) == ("customer", "customer_email_key")
            assert error.detail == "Key (email)=(lee@example.com) already exists."
        else:
            raise AssertionError("the email is unique")

        # There is no bakery 99.
        try:
            await Cake.insert({
                "name": "Ghost Cake",
                "flavor": "vanilla",
                "price": Decimal("1.00"),
                "gluten_free": False,
                "bakery_id": 99,
            }).execute(pool)
        except ForeignKeyViolationError as error:
            assert error.constraint == "fk-cake-bakery_id"
        else:
            raise AssertionError("the bakery does not exist")

        # A cake left without a name.
        try:
            await pool.execute(RawSQL(
                "INSERT INTO cake (flavor, price, gluten_free, bakery_id) VALUES ($1, $2, $3, $4)",
                ["vanilla", Decimal("1.00"), False, 1],
            ))
        except NotNullViolationError as error:
            assert (error.table, error.column) == ("cake", "name")
            assert error.constraint is None
        else:
            raise AssertionError("a cake has a name")


asyncio.run(main())
```

The last insert is raw SQL because a model refuses it first: `Cake.insert`
without a name raises `ConstructionError` before anything is sent, since the
model declares `name` as neither nullable nor defaulted (see [What pgorm
refused](#what-pgorm-refused)).

A check and an exclusion constraint answer the same way. The bakery's ovens
are booked by time, and an exclusion constraint keeps two bookings of one
oven from overlapping. A `RawSQL` holds one statement, so the setup runs
statement by statement:

```python
import asyncio
import os
from datetime import datetime
from decimal import Decimal

from pgorm import CheckViolationError, ExclusionViolationError, Pool, RawSQL, Value

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        for statement in (
            "ALTER TABLE cake ADD CONSTRAINT cake_price_positive CHECK (price > 0)",
            "CREATE EXTENSION IF NOT EXISTS btree_gist",
            "CREATE TABLE oven_booking ("
            " oven integer NOT NULL,"
            " during tstzrange NOT NULL,"
            " EXCLUDE USING gist (oven WITH =, during WITH &&))",
        ):
            await pool.execute(RawSQL(statement))

        try:
            await Cake.update({"price": Decimal("0")}).where(Cake.col("id") == 1).execute(pool)
        except CheckViolationError as error:
            assert error.constraint == "cake_price_positive"
        else:
            raise AssertionError("a cake costs something")

        def at(time: str) -> Value:
            # A timezone-aware datetime is bound with its kind named.
            return Value(datetime.fromisoformat(f"2026-11-04T{time}:00+00:00"), "datetime_utc")

        book = "INSERT INTO oven_booking (oven, during) VALUES ($1, tstzrange($2, $3))"
        await pool.execute(RawSQL(book, [1, at("06:00"), at("08:00")]))
        try:
            await pool.execute(RawSQL(book, [1, at("07:00"), at("09:00")]))
        except ExclusionViolationError as error:
            assert error.table == "oven_booking"
        else:
            raise AssertionError("oven 1 is booked from six to eight")


asyncio.run(main())
```

Any other refusal is a plain `DatabaseError`, read off its `sqlstate`.
PostgreSQL's documentation lists the codes in its appendix *PostgreSQL Error
Codes*:

```python
import asyncio
import os

from pgorm import DatabaseError, Pool, RawSQL


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        try:
            await pool.query(RawSQL("SELECT * FROM pastry"))
        except DatabaseError as error:
            # 42P01 is undefined_table.
            assert error.sqlstate == "42P01"
            assert str(error) == error.message == 'relation "pastry" does not exist'
            assert type(error) is DatabaseError
        else:
            raise AssertionError("there is no pastry table")


asyncio.run(main())
```

Rust's `is_retryable` answers whether an error is one PostgreSQL expects the
client to retry the whole transaction after, a serialization failure
(`40001`) or a deadlock (`40P01`). It is not available in Python yet: compare
`sqlstate` with those two codes. [Transactions](08-transactions.md) shows
the retrying.

A `DatabaseError` pgorm raises without an answer from the server, which some
of pgorm's own refusals are, has every one of these attributes `None`;
`str(error)` is its text either way.

## What the driver refused

Some failures happen on the client, without the server refusing anything. A
value can be refused on its way in, because the parameter's type cannot hold
it, or on its way out, because Python, or the field it is read into, cannot
hold the column's value.

A value refused on its way in raises `ConstructionError`, with pgorm's text
for the refusal: which `$N` parameter was refused, and why. Nothing was sent.
So does a statement given another number of values than its placeholders
take, its text saying how many it takes and how many it was given. A value
refused on its way out raises `DecodeError`:

```python
import asyncio
import os

from pgorm import Column, ConstructionError, DecodeError, Model, Pool, RawSQL


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # The parameter is a smallint, and 100,000 is not one.
        try:
            await pool.one(RawSQL("SELECT $1::smallint AS n", [100_000]))
        except ConstructionError as error:
            assert str(error) == (
                "parameter $1 cannot be bound: value `100000` is out of range for Postgres type `int2`"
            )
        else:
            raise AssertionError("100,000 does not bind as a smallint")

        # Two placeholders, and one value.
        try:
            await pool.one(RawSQL("SELECT $1::text AS a, $2::text AS b", ["lemon"]))
        except ConstructionError as error:
            assert str(error) == "the statement takes 2 parameters, and 1 was given"
        else:
            raise AssertionError("a value is missing")

        # A model that says a customer's name is an integer cannot read it.
        Mislabelled = Model("customer", {
            "id": Column("i32", primary_key=True),
            "name": Column("i32"),
        })
        try:
            await Mislabelled.find().all(pool)
        except DecodeError as error:
            assert str(error) == "value type differs from the declared model column"
        else:
            raise AssertionError("text does not decode as an integer")

        # A NULL for a field the model does not declare nullable: Kim has no notes.
        Strict = Model("customer", {
            "id": Column("i32", primary_key=True),
            "notes": Column("text"),
        })
        try:
            await Strict.find().all(pool)
        except DecodeError as error:
            assert str(error) == "non-nullable model column received SQL NULL"
        else:
            raise AssertionError("a NULL is not text")

        # A numeric NaN, which pgorm's decimal cannot hold.
        try:
            await pool.one(RawSQL("SELECT 'NaN'::numeric AS price"))
        except DecodeError:
            pass
        else:
            raise AssertionError("NaN is not a decimal pgorm reads")


asyncio.run(main())
```

Here Python differs from Rust. Rust reports these refusals as
`Error::Postgres` with no server code, and classifies them with
`bind_refusal`, `parameter_count_refusal` and `decode_refusal`, whose
`BindRefusal`, `ParameterCountRefusal` and `DecodeRefusal` carry the
parameter's number, the two counts and what was being read as fields. Python
raises `ConstructionError` and `DecodeError`, with the refusal's text and no
such fields. A Python record reads each column as its own type, so a raw
query never asks for a type the column does not have: the decoding a program
chooses is a model's declaration, and a model whose field does not match its
column, or receives a `NULL` it does not declare, raises `DecodeError`. Rust
reports that `NULL` as `Error::Type`, which Python's `DecodeError` covers too.
[Values and types](13-values.md) lists what each PostgreSQL type reads as,
and what cannot be read.

## What pgorm refused

The rest are pgorm's own refusals. Most say that a call could not do what it
promised, before or instead of sending a statement. Rust's `Error` variants
arrive in Python as these classes:

| Class | Raised when | Rust's variants |
| --- | --- | --- |
| `DecodeError` | `one` found no row or several, `optional` several; a value of the wrong type or a `NULL` where none is declared; JSON or a value that does not convert; a registered entity's write that reads back no row | `RecordNotFound`, `RecordNotInserted`, `Type`, `Json`, `Conversion`, `UnpackInsertId`, `Verify` |
| `ConstructionError` | a write that names no row or sets nothing, a field that is required and left out, a builder that refused its input, a hook's refusal | `PrimaryKeyNotSet`, `NothingToSet`, `AttrNotSet`, `QueryBuilder`, `Custom`, and a `Listen` channel name that is too long |
| `DatabaseError` with `sqlstate` `25P02` | a transaction's block exited normally after catching a failed statement's error, so it was rolled back ([Transactions](08-transactions.md#a-caught-error-still-aborts)) | `TransactionAborted` |
| `ConnectionError` | the pool could not reach the server, a connection broke, or a listener fell behind | `Pool`, and a `Listen` overflow |
| `TimeoutError` | no connection came free within the pool's `acquire_timeout` | `Pool`, when a wait times out |
| `LifecycleError` | a pool, connection, transaction, stream or listener used after it closed, while another operation holds it, or from another event loop | `Pool` for a closed pool; Rust's borrows make the rest compile errors |
| `UnsupportedCapabilityError` | an operation this build does not have, or a registration it does not hold | none |
| `InternalError` | pgorm or the binding failed in a way no input should cause | none |

Here Python differs from Rust: `optional` is Rust's `one_opt`, answering
`None` for no row, and a model write checks its own fields as it is built, so
many of these refusals come from building the statement, not from running it:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import ConstructionError, DecodeError, Pool

from bakery import Cake, Customer


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # No cake 99.
        try:
            await Cake.find_by_key({"id": 99}).one(pool)
        except DecodeError as error:
            assert str(error) == "expected exactly one row, received 0"
        else:
            raise AssertionError("there is no cake 99")
        assert await Cake.find_by_key({"id": 99}).optional(pool) is None

        # An update says which rows it changes.
        try:
            await Customer.update({"name": "Kim"}).execute(pool)
        except ConstructionError as error:
            assert str(error) == "update requires where or explicit all_rows"
        else:
            raise AssertionError("an update names its rows")

    # An update that sets nothing is refused as it is built.
    try:
        Cake.update({})
    except ConstructionError as error:
        assert str(error) == "model update requires at least one assigned field"
    else:
        raise AssertionError("an update sets something")

    # So is a cake left without its name.
    try:
        Cake.insert({"flavor": "vanilla", "price": Decimal("1.00"), "gluten_free": False, "bakery_id": 1})
    except ConstructionError as error:
        assert str(error) == "name is required: it is neither nullable, defaulted nor generated"
    else:
        raise AssertionError("a cake has a name")


asyncio.run(main())
```

A registered Rust entity's `ActiveModel`, which [Registering Rust
entities](24-registrations.md) covers, refuses as Rust's does: a one-row
update or delete whose key is not set, and a hook's refusal, raise
`ConstructionError`.

`UnsupportedCapabilityError` is a `ConstructionError`: it refuses something
the installed build cannot do, before anything is sent. A wheel that
registers no Rust entities raises it for `pgorm.entity(..)`, and
`pgorm.require_capability` raises it for an operation the build lacks:

```python
import pgorm

try:
    pgorm.require_capability("no_such_operation")
except pgorm.UnsupportedCapabilityError as error:
    assert isinstance(error, pgorm.ConstructionError)
    assert str(error) == 'this pgorm build does not support operation "no_such_operation"'
else:
    raise AssertionError("no build has that operation")
```

`InternalError` means a bug in pgorm or the binding, never a fault in what the
program passed. A panic in pgorm's native work, which every awaitable the
binding returns runs on its runtime, raises `InternalError`, its message
`pgorm panicked:` and the panic's own. A panic in a synchronous call, such as
a builder's, raises PyO3's `PanicException` instead, since PyO3 catches it
before the binding can; it derives from `BaseException`, so an
`except Exception` meant for bad input does not swallow it either.

### When the pool cannot connect

A pool that cannot open a connection reports it when one is asked for, since
building the pool connects to nothing. A pool's `async with` block asks for
one as it starts, so the error comes from there. Credentials never appear in
an error: the password, and the user name, are replaced by `[redacted]`
wherever the server's message, or pgorm's, would show them.

Here Python differs from Rust: Rust reports every failure to connect as
`Error::Pool`. Python reports a refusal the server sent while connecting as
the `DatabaseError` it is, with its SQLSTATE, such as `3D000` for a database
that does not exist or `28P01` for a wrong password. A server that cannot be
reached at all, or a connection that breaks, raises `ConnectionError`, whose
text says only that the connection failed:

```python
import asyncio
import os
from urllib.parse import urlsplit, urlunsplit

import pgorm
from pgorm import DatabaseError, Pool


def changed(url: str, *, path: str | None = None, password: str | None = None,
            port: int | None = None) -> str:
    """The URL with its database, password or port replaced."""
    parts = urlsplit(url)
    netloc = f"{parts.username}:{password or parts.password}@{parts.hostname}:{port or parts.port}"
    return urlunsplit((parts.scheme, netloc, path or parts.path, parts.query, parts.fragment))


async def main() -> None:
    url = os.environ["DATABASE_URL"]

    try:
        async with Pool(changed(url, path="/no_such_bakery")):
            pass
    except DatabaseError as error:
        assert error.sqlstate == "3D000"
        assert error.message == 'database "no_such_bakery" does not exist'
    else:
        raise AssertionError("there is no such database")

    secret = "not-the-password"
    try:
        async with Pool(changed(url, password=secret)):
            pass
    except DatabaseError as error:
        assert error.sqlstate == "28P01"
        assert error.message == 'password authentication failed for user "[redacted]"'
        assert secret not in repr(error)
    else:
        raise AssertionError("the password is wrong")

    try:
        async with Pool(changed(url, port=1), connect_timeout=2):
            pass
    except pgorm.ConnectionError as error:
        assert str(error) == "PostgreSQL connection or protocol failure"
    else:
        raise AssertionError("nothing listens on port 1")


asyncio.run(main())
```

## Arguments of the wrong type

An argument of a type a call does not take raises Python's own `TypeError`,
before anything is sent: SQL text where a statement goes (raw SQL is a
`RawSQL`, never a bare `str`), a channel name that is not a `str`, an option
outside its choices, or a keyword the call does not know. A value of the
right type that pgorm cannot send is a `ConstructionError` instead. mypy
reports the first kind before the program runs, which is why the lines below
tell it to look away:

```python
import asyncio
import os

from pgorm import ConstructionError, Pool


async def main() -> None:
    url = os.environ["DATABASE_URL"]

    # "clean" is not one of the recycling methods.
    try:
        Pool(url, recycle="clean")  # type: ignore[arg-type]
    except TypeError as error:
        assert str(error) == "recycle is 'verified' or 'fast'"
    else:
        raise AssertionError("recycle takes 'verified' or 'fast'")

    # A misspelt option is never left out silently.
    try:
        Pool(url, max_sise=4)  # type: ignore[call-arg]
    except TypeError:
        pass
    else:
        raise AssertionError("there is no max_sise")

    # An int, but not one the pool can use.
    try:
        Pool(url, max_size=0)
    except ConstructionError as error:
        assert str(error) == "max_size is outside the supported range"
    else:
        raise AssertionError("a pool holds at least one connection")

    # SQL text is not a statement.
    async with Pool(url) as pool:
        try:
            await pool.execute("SELECT 1")  # type: ignore[arg-type]
        except TypeError as error:
            assert str(error) == "execution requires a native statement, Compiled object or explicit RawSQL"
        else:
            raise AssertionError("raw SQL is a RawSQL")


asyncio.run(main())
```

The first three pools are refused as they are built, before they could
connect, and the statement before it is sent. A listener's capacity is the
one argument with a third answer: one that is an `int` but not a positive one
raises `ValueError` ([LISTEN and NOTIFY](18-listen-notify.md)).

## Matching errors

Catch the classes from the most specific to the most general: a handler for
`DatabaseError` also catches the five violation classes, and one for
`PgormError` catches everything pgorm raises. Within a class, `match` on
`sqlstate` picks out the codes a program answers:

```python
import asyncio
import os

from pgorm import DatabaseError, Pool, UniqueViolationError

from bakery import Customer


async def sign_up(pool: Pool, name: str, email: str) -> str:
    """Sign a customer up, saying why not when the database refuses."""
    try:
        await Customer.insert({"name": name, "email": email, "notes": None}).execute(pool)
    except UniqueViolationError as error:
        if error.constraint == "customer_email_key":
            return f"{email} already has an account"
        raise
    except DatabaseError as error:
        match error.sqlstate:
            case "40001" | "40P01":
                return "try again"
            case _:
                raise
    return f"welcome, {name}"


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        assert await sign_up(pool, "Mo", "mo@example.com") == "welcome, Mo"
        assert await sign_up(pool, "Lee", "lee@example.com") == "lee@example.com already has an account"


asyncio.run(main())
```

Leave cancellation alone. A cancelled task raises asyncio's `CancelledError`,
which `pgorm.CancelledError` names too, and which is a `BaseException`, not
a `PgormError`, so an `except PgormError` never swallows it and an
`except Exception` does not either. Let it propagate, as asyncio expects;
[Python and asyncio](02-asyncio.md) covers what it does to a pool's work.

A failure inside a transaction's block reaches the code around the block as
itself, after the rollback, with no wrapper of the kind Rust's
`TransactionError` is; [Transactions](08-transactions.md) covers what a
failed `COMMIT` or `ROLLBACK` raises.
