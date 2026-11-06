# Types and mypy

The package is typed. A type checker reads its declarations from the
installed wheel, so a program checked with mypy in strict mode finds a wrong
argument, a method a builder does not have, or a result used as the wrong
type before anything runs. This chapter covers what the declarations say,
what they leave to run time, and how to check a program against them.

The Rust guide has no chapter like this one: in Rust the compiler holds a
program to pgorm's types. In Python a type checker does that job, and only
where the program asks it to.

## The package's type information

The wheel carries its types inside the package. There is no separate stubs
package to install. It holds:

- `py.typed`, the marker that tells a type checker to read the package's own
  types;
- a `.pyi` declaration for every class and function of the native extension,
  `pgorm._native`;
- typed Python modules for what the package writes in Python: pools,
  connections and transactions, records and streams, COPY, the listener,
  models, schema and the pipeline.

```python
from importlib.resources import files

import pgorm

package = files(pgorm)
assert package.joinpath("py.typed").is_file()
assert package.joinpath("_native.pyi").is_file()
```

The declarations use Python 3.14's typing, as the package needs CPython 3.14
to run. Point the type checker at the interpreter the wheel is installed in,
so it reads that installation and checks against 3.14:

```sh
uv tool run --from mypy@latest mypy --strict --python-executable .venv/bin/python app.py
```

With mypy installed in the same environment, `.venv/bin/python -m mypy
--strict app.py` does the same. pgorm's own checks run the latest mypy
release in strict mode against an installed wheel, over the package itself,
the examples in this guide that run on the standalone wheel, and the
application example at the end of this chapter.

The declarations mark positional-only parameters with `/` and keyword-only
options with `*`, as the native code takes them. A class whose objects come
only from pgorm, such as `Record`, `Compiled` or `Expr`, declares a
constructor that takes `Never`. An `Expr` comes from `col`, `literal` and the
other functions, a `Compiled` from `inspect()`, a DDL statement such as
`CreateTable` from its function (`create_table`), a conflict action from
`Conflict`'s functions and a MERGE action from `MergeAction`'s. Calling the
class is an error for mypy, and a `TypeError` at run time.

## What an operation returns

Building a statement is synchronous. A builder method returns a new builder
and leaves the one it was called on as it was, so a query can be kept and
refined in several ways:

```python
from bakery import Cake

every_cake = Cake.find()
gluten_free = every_cake.where(Cake.col("gluten_free").eq(True))

assert "WHERE" not in every_cake.inspect().sql
assert gluten_free.inspect().sql.endswith('WHERE "cake"."gluten_free" = $1')
```

Running one is asynchronous. A terminal returns an awaitable, and what the
`await` gives is typed:

| Operation | Gives, once awaited |
| --- | --- |
| `execute(query)` on a pool, connection or transaction | `int`, the rows affected |
| `query(query)` | `list[Record]` |
| `one(query)` | `Record`; `DecodeError` unless exactly one row exists |
| `optional(query)` | `Record \| None`; `DecodeError` if more than one row exists |
| `stream(query)` on a pool or connection | `RowStream`, an `AsyncIterator[Record]` and async context manager |
| `copy_in(statement, source)` on a pool, connection or transaction | `int`, the rows written |
| `copy_out(statement)` on a pool, connection or transaction | `CopyOutStream`, an `AsyncIterator[bytes]` and async context manager |
| `copy_in_rows(statement, rows)` on a pool, connection or transaction | `int`, the rows written; `statement` is a `CopyFrom[Literal["binary"]]` |
| `copy_out_rows(statement)` on a pool, connection or transaction | `CopyRowStream`, an `AsyncIterator[tuple[object, ...]]` and async context manager; `statement` is a `CopyTo[Literal["binary"]]` |
| `pool.listener(capacity=1024)` | `Listener`, an `AsyncIterator[Notification]` and async context manager |
| `notify(channel, payload)` on a pool, connection or transaction | `None` |
| `Model.find().all(db)`, `.one(db)`, `.optional(db)` | `list[ModelRecord]`, `ModelRecord`, `ModelRecord \| None` |
| `Model.find().count(db)` | `int` |
| `Model.find().stream(db)` on a pool or connection | `RowStream[ModelRecord]` |
| a model write's `execute(db)` | `int`, the rows written |
| a model write's `returning(...)` | `ModelRows`, whose `all`, `one` and `optional` give `ModelRecord`s |
| `Model.update(values)...returning_change(db)`, `returning_changes(db)` | `Change[ModelRecord]`, `list[Change[ModelRecord]]` |
| `Model.insert(values)...returning_upsert(db)` | `Upserted.Inserted[ModelRecord] \| Upserted.Updated[ModelRecord] \| None` |
| `Entity.find().all(db)` for a registered entity | `list[EntityModel]` |
| `Pipeline.all(db)`, `.one(db)`, `.optional(db)` | `list[Record]`, `Record`, `Record \| None` |

Here `db` is a pool, a connection or a transaction. Two calls give no
awaitable but an async context manager, used with `async with` directly:
`pool.connection()`, which lends a connection for the block, and
`transaction()`, on a pool, a connection or a transaction, which opens a
transaction or a savepoint for the block. The `Transaction` the block
receives is not a context manager itself, and has no `commit` or `rollback`:
the block's exit decides its outcome.

A model's writes are each a class of their own, with only their own
methods:

| Write | Its methods |
| --- | --- |
| `Model.insert(values)`: `ModelInsert` | `on_conflict`, `add`, `add_many`, `returning_upsert`, `returning`, `execute`, `inspect` |
| `Model.insert_many(rows)`, or an insert's `add(values)` or `add_many(rows)`: `ModelInsertMany` | `on_conflict`, `add`, `add_many`, `returning_upserts`, `returning`, `execute`, `inspect` |
| `Model.update(values)`: `ModelUpdate` | `where`, `all_rows`, `returning_change`, `returning_changes`, `returning`, `execute`, `inspect` |
| `Model.delete()`: `ModelDelete` | `where`, `all_rows`, `returning`, `execute`, `inspect` |

`assert_type` shows what mypy infers, and does nothing when the program
runs:

```python
import asyncio
import os
from decimal import Decimal
from typing import assert_type

from pgorm import Change, ModelRecord, Pool, Record, RowStream, select

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        names = select(Cake.col("name")).from_(Cake.table).order_by(Cake.col("name").asc())

        rows = await pool.query(names)
        assert_type(rows, list[Record])
        assert rows[0]["name"] == "Carrot Cake"

        first = await pool.optional(names.limit(1))
        assert_type(first, Record | None)
        assert first is not None and first["name"] == "Carrot Cake"

        changed = await Cake.update({"price": Decimal("4.75")}).where(Cake.key({"id": 1})).execute(pool)
        assert_type(changed, int)
        assert changed == 1

        raised = await Cake.update({"price": Decimal("5.00")}).where(Cake.key({"id": 1})).returning_change(pool)
        assert_type(raised, Change[ModelRecord])
        # Change, unparameterized, is any write's change: a model's, an entity's or a view's.
        change: Change = raised
        assert (change.old["price"], change.new["price"]) == (Decimal("4.75"), Decimal("5.00"))

        cakes = await Cake.find().where(Cake.col("bakery_id") == 2).all(pool)
        assert_type(cakes, list[ModelRecord])
        assert len(cakes) == 2

        async with await pool.stream(names) as stream:
            assert_type(stream, RowStream)
            streamed = [row["name"] async for row in stream]
        assert streamed == [row["name"] for row in rows]


asyncio.run(main())
```

A stream holds its connection until it ends. Use it with `async with`, or
close it, whenever iteration can stop before the last row. A stream from a
pool holds a connection of its own: reaching the last row returns it to the
pool, and closing the stream before then discards it. A stream from a
connection reserves that connection until it ends.

A transaction reserves its connection for its block, and a savepoint its
parent transaction until the savepoint's block ends. A transaction has no
`stream`: rows from inside one come back whole, through `query`.

## Records are dynamic

A `Record` is a mapping from column name to value, and a `ModelRecord` from
field name to value. Their values are typed `Any`. A type checker cannot know
which columns a statement projects, or what a model declared at run time
holds, so it does not guess:

```python
import asyncio
import os
from typing import Any, assert_type

from pgorm import Pool

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        cake = await Cake.find().where(Cake.key({"id": 2})).one(pool)
        name = cake["name"]
        assert_type(name, Any)
        assert isinstance(name, str) and name == "Lemon Drizzle"

        # The value with its PostgreSQL type, which Python's int does not carry.
        assert cake.tagged("bakery_id").kind == "i32"


asyncio.run(main())
```

An optional result still has to be checked for `None` before it is read, as
`first is not None` does above. Python's `int` says nothing of a column's
width or range: pgorm checks those when a value is converted, and a `Value`
with an explicit kind (`Value(1, "i16")`) keeps a width, a typed NULL, an
array's element type or an enum's qualified name where an inferred one
cannot. [Values and types](13-values.md) covers how values convert.

## Generated concrete models

A model declared in Python is dynamic in the way its records are. For
properties with types of their own, declare the entities in Rust and
register them into an application wheel. [Registering Rust
entities](24-registrations.md) walks through the build: code generation
writes, beside the registration crate, a Python module of concrete views and
the declarations for them. Each entity has a model view with one property per
column, typed as its Rust field is (`int`, `str`, `Decimal`, `date`,
`datetime`, `str | None` for an `Option<String>`), and an active view with a
typed setter per column.

```python registered
import asyncio
import os
from decimal import Decimal
from typing import assert_type

from pgorm import Pool
from pgorm.rust_bakery import Cake, CakeActive, CakeModel, Customer, CustomerModel


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        cakes = await Cake.find().where(Cake.col("gluten_free") == True).order_by(Cake.col("name").asc()).all(pool)
        assert_type(cakes, list[CakeModel])
        assert [cake.name for cake in cakes] == ["Chocolate Torte", "Lemon Drizzle", "Vanilla Sponge"]

        torte = cakes[0]
        assert_type(torte.price, Decimal)
        assert torte.price == Decimal("6.00")

        cheaper = torte.into_active().set_price(Decimal("5.50"))
        assert_type(cheaper, CakeActive)
        saved = await cheaper.update(pool)
        assert_type(saved, CakeModel)
        assert saved.price == Decimal("5.50")

        kim = await Customer.find().where(Customer.col("id") == 1).one(pool)
        assert_type(kim, CustomerModel)
        assert_type(kim.notes, str | None)
        assert kim.notes is None


asyncio.run(main())
```

A setter takes its property's type or an explicit `Value`, so
`set_price("cheap")` is an error for mypy. A column whose Rust type has no
Python counterpart, such as a custom representation or JSON, stays `Any`.
Importing the generated module checks that it was generated for the
installed extension: its version, its features and its registrations. Against
any other it raises `UnsupportedCapabilityError` rather than load
declarations that would not match what runs.

## Types that follow a statement's shape

Some builders carry what they are building in their type, so that mypy
refuses a call the runtime would refuse too.

### A write by its kind and its row count

An insert has no `where`, and an update no `on_conflict`: each write has
only its own statement's methods. An insert is also typed by its row count,
as Rust's `Insert` is. `insert` gives a `ModelInsert`, a statement of one
row, and `insert_many`, `add` and `add_many` give a `ModelInsertMany`. Each
has its own upsert terminal, `returning_upsert` for one row and
`returning_upserts` for a batch, and its `inspect` takes the name of that
terminal alone, a `Literal`:

```python
from decimal import Decimal
from typing import assert_type

from pgorm import ModelInsert, ModelInsertMany

from bakery import Cake

scone = {"name": "Lemon Scone", "flavor": "lemon", "price": Decimal("2.50"),
         "gluten_free": False, "bakery_id": 1}
bun = {"name": "Vanilla Bun", "flavor": "vanilla", "price": Decimal("1.80"),
       "gluten_free": False, "bakery_id": 2}

one = Cake.insert(scone)
assert_type(one, ModelInsert)
batch = one.add(bun)
assert_type(batch, ModelInsertMany)

assert "RETURNING" in one.inspect(terminal="returning_upsert").sql
assert "RETURNING" in batch.inspect(terminal="returning_upserts").sql
```

### A COPY by its format

A COPY statement's format is a type parameter. `CopyFrom(table, "csv")` is a
`CopyFrom[Literal["csv"]]`, and each option exists only on the formats that
have it: `quote` and `escape` on csv, `header` and `delimiter` on text and
csv, nothing of the kind on binary. `copy_in_rows` and `copy_out_rows`, which
carry rows of values, take a binary statement alone:

```python
from typing import Literal, assert_type

from pgorm import CopyFrom, CopyTo

from bakery import Cake

csv = CopyFrom(Cake.table, "csv").header().delimiter(";")
assert_type(csv, CopyFrom[Literal["csv"]])
assert csv.inspect().sql == """COPY "cake" FROM STDIN WITH (FORMAT csv, DELIMITER ';', HEADER true)"""

rows = CopyTo(Cake.table, "binary")
assert_type(rows, CopyTo[Literal["binary"]])
assert rows.inspect().sql == 'COPY "cake" TO STDOUT WITH (FORMAT binary)'
```

### A MERGE action by the row it takes

A MERGE action carries the kind of row it acts on.
`MergeAction.delete()` is a `MergeAction[Literal["target"]]`,
`insert_defaults()` a `MergeAction[Literal["source"]]`, and `do_nothing()`
both. `when_matched` and `when_not_matched_by_source` take an update or a
target action, and `when_not_matched` an insert or a source action. The
type parameter exists for the type checker alone, so name it in an
annotation, as below, rather than in an expression the program evaluates:

```python
from typing import Literal

from pgorm import MergeAction, Table, merge

target, staged = Table("cake"), Table("staged_cake")
remove: MergeAction[Literal["target"]] = MergeAction.delete()

statement = (
    merge(target, staged, target.col("id") == staged.col("id"))
    .when_matched(remove)
    .when_not_matched(MergeAction.insert_defaults())
)
assert statement.inspect().sql == (
    'MERGE INTO "cake" USING "staged_cake" ON "cake"."id" = "staged_cake"."id" '
    "WHEN MATCHED THEN DELETE WHEN NOT MATCHED THEN INSERT DEFAULT VALUES"
)
```

### A disjoint model with no kind column

A [disjoint model](15-disjoint.md) whose base table holds each row's kind is
a `Disjoint[Literal["kinded"]]`, the default. One declared with
`unkinded=True`, whose base has no kind column, is a
`Disjoint[Literal["unkinded"]]`. It is read-only: its `insert`, `update` and
`change_kind`, and the schema functions `create_disjoint`,
`add_disjoint_kind` and `remove_disjoint_kind_table`, take a kinded one
alone. A registered disjoint enum is a `DisjointEntity`, which a generated
module types as kinded or unkinded in the same way; `disjoint_entity(name)`
on its own gives one typed `Any`.

```python
from typing import Literal, assert_type

from pgorm import Column, Disjoint, Model, TypeName

PAYMENT_KIND = TypeName("payment_kind", schema="public")

Payment = Model("payment", {
    "id": Column("i32", primary_key=True, default=True),
    "kind": Column(PAYMENT_KIND, values=["card", "cash"]),
})
CardPayment = Model("card_payment", {
    "id": Column("i32", primary_key=True),
    "last_digits": Column("text"),
})
LegacyPayment = Model("legacy_payment", {"id": Column("i32", primary_key=True)})

payments = Disjoint("payment", Payment, {"card": CardPayment, "cash": None}, schema_name="public")
assert_type(payments, Disjoint[Literal["kinded"]])
assert payments.kind_field == "kind"

legacy = Disjoint("legacy_payment", LegacyPayment, {"card": CardPayment, "cash": None}, unkinded=True)
assert_type(legacy, Disjoint[Literal["unkinded"]])
assert legacy.kind_field is None
```

### Closed choices

An option that takes one of a fixed set of words is typed as their
`Literal`: a transaction's `mode` and `isolation`, a pool's `tls` and
`recycle`, a foreign key's actions, a COPY's format, a generated column's
kind and the rest. A word outside the set is an error for mypy and a
`TypeError` at run time. A choice the package spells as a class, such as
`Join.Left` or `Direction.Asc`, is typed by its class. A schema column's
type is a `Literal` of the built-in type names too, or a `TypeName` for any
other type; a name the package does not know is an
`UnsupportedCapabilityError` at run time.

## What mypy refuses

Each program below fails `mypy --strict` with the message shown, and each
says what happens if it runs regardless. They are shown as text, since a
program in this guide has to pass mypy. Given a `pool`, the bakery's `Cake`
and the examples above:

A string is not a statement. SQL text goes through `RawSQL`, with its values
apart ([Raw SQL](10-raw-sql.md)); at run time this is a `TypeError`.

```text
await pool.execute("UPDATE cake SET price = 5")

error: Argument 1 to "execute" of "Pool" has incompatible type "str"; expected "Select | Insert | Update | Delete | Merge | RawSQL | Compiled | Pipeline | DDL"  [arg-type]
```

A record is not the value inside it. Nothing stops this at run time: the
program goes on holding a `Record` where it expects an `int`.

```text
count: int = await pool.one(select(Cake.col("id")).from_(Cake.table))

error: Incompatible types in assignment (expression has type "Record", variable has type "int")  [assignment]
```

A write has only its own kind's methods, and an insert only its row count's
upsert terminal; at run time the first two are `AttributeError`s and the
third a `TypeError`:

```text
insert = Cake.insert(scone)
insert.where(Cake.col("id") == 1)
await insert.returning_upserts(pool)
insert.inspect(terminal="returning_upserts")

error: "ModelInsert" has no attribute "where"  [attr-defined]
error: "ModelInsert" has no attribute "returning_upserts"; maybe "returning_upsert"?  [attr-defined]
error: Argument "terminal" to "inspect" of "ModelInsert" has incompatible type "Literal['returning_upserts']"; expected "Literal['returning_upsert'] | None"  [arg-type]
```

A COPY option its format lacks, a row COPY in a text format, and a format
that does not exist; at run time a `ConstructionError` and two `TypeError`s:

```text
CopyFrom(Cake.table, "binary").header()
await pool.copy_in_rows(CopyFrom(Cake.table, "csv"), [[1]])
CopyFrom(Cake.table, "yaml")

error: Invalid self argument "CopyFrom[Literal['binary']]" to attribute function "header" with type "Callable[[_DelimitedFrom], _DelimitedFrom]"  [misc]
error: Argument 2 to "CopyFrom" has incompatible type "Literal['csv']"; expected "Literal['binary']"  [arg-type]
error: Value of type variable "_Format" of "CopyFrom" cannot be "Literal['yaml']"  [type-var]
```

A MERGE arm given an action for the other kind of row; at run time a
`ConstructionError`:

```text
merge(target, staged, target.col("id") == staged.col("id")).when_matched(MergeAction.insert_defaults())

error: Argument 1 to "when_matched" of "PendingMerge" has incompatible type "MergeAction[Literal['source']]"; expected "MergeUpdate | MergeAction[Literal['target']]"  [arg-type]
```

A transaction ended by hand, and an isolation level PostgreSQL does not
have; at run time an `AttributeError` and a `TypeError`:

```text
async with pool.transaction() as transaction:
    await transaction.commit()
pool.transaction(isolation="snapshot")

error: "Transaction" has no attribute "commit"  [attr-defined]
error: Argument "isolation" to "transaction" of "Pool" has incompatible type "Literal['snapshot']"; expected "Literal['read_uncommitted', 'read_committed', 'repeatable_read', 'serializable'] | None"  [arg-type]
```

A write to an unkinded disjoint model, or its schema; at run time a
`ConstructionError` naming it read-only:

```text
legacy.insert({"id": 1}, "cash")
create_disjoint(legacy)

error: Invalid self argument "Disjoint[Literal['unkinded']]" to attribute function "insert" with type "Callable[[Disjoint[Literal['kinded']], Mapping[str, Any], str, Mapping[str, Any] | None], DisjointWrite[tuple[ModelRecord, str, ModelRecord | None]]]"  [misc]
error: Argument 1 to "create_disjoint" has incompatible type "Disjoint[Literal['unkinded']]"; expected "Disjoint[Literal['kinded']] | DisjointEntity[Literal['kinded'], Any] | WritableDisjointView[Any, Any, Any, Any]"  [arg-type]
```

A class pgorm makes, called directly; at run time a `TypeError`:

```text
Compiled()

error: Too few arguments for "Compiled"  [call-arg]
```

Exceptions are not part of a signature: Python's typing does not declare
them. [Errors](09-errors.md) lists what each failure raises.

## A typed application

[`examples/application.py`](../examples/application.py) is a whole program
that uses only `import pgorm` and its `schema` and `pipeline` modules, and
checks its types with `assert_type` as it goes. It creates two tables with
names of its own, inserts and updates rows through a model, joins the
tables, commits a transaction while rolling back a savepoint inside it,
takes one table's rows apart from the other's with `EXCEPT`, merges one table
into the other and reads the rows the `MERGE` returned, selects through a
pipeline, streams rows and deletes one. Its `finally` blocks drop its tables
again. It needs a database on which its account can create and drop tables,
and reads that database's URL from `PGORM_TEST_DSN`.

From the repository root, with the wheel installed in `.venv`:

```sh
export PGORM_TEST_DSN="$DATABASE_URL"
.venv/bin/python -I pgorm-python/examples/application.py
uv tool run --from mypy@latest mypy --strict \
  --python-executable .venv/bin/python pgorm-python/examples/application.py
```

It prints what it did as JSON: the two joined rows, the merged row with its
`merge_action` of `UPDATE`, the streamed ids `[1, 2]`, one deleted row, and
the transport, `in-process`.
