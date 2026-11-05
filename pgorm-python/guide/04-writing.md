# Writing rows

A model writes rows through three statements its methods build: an insert,
from `insert` or `insert_many`, an update, from `update`, and a delete, from
`delete`. Each is a value until a terminal runs it, and each terminal says
what comes back. A write's `statement` is the query builder's `Insert`,
`Update` or `Delete` it runs, for a program that needs what a model's write
does not offer; an insert of no rows has none. Testing a write for truth,
as `if insert:` does, raises `ConstructionError` rather than reading the
statement as true. This chapter writes to the bakery: it adds cakes, changes
their prices, removes them, and signs customers up without creating
duplicates.

## The values a write sets

Rust writes a row through an *active model*: the row's fields, each holding
an `ActiveValue` that says what to do with its column. A Python model's
write takes a mapping from field to value instead, and the mapping says the
same thing:

| In the mapping | Meaning | Rust |
| --- | --- | --- |
| a field with a value | write the value to the column | `Set(v)` |
| a field holding `None` | write SQL NULL, which only a nullable field takes | `Set(None)` |
| a field left out | leave the column out of the statement | `NotSet` |

A column left out of an insert takes its default in the database, which for
the generated `id` is the next number:

```python
from decimal import Decimal

from bakery import Cake

muffin = {
    "name": "Blueberry Muffin",
    "flavor": "vanilla",
    "price": Decimal("2.75"),
    "gluten_free": False,
    "bakery_id": 2,
}
insert = Cake.insert(muffin)
compiled = insert.inspect()
assert compiled.sql == (
    'INSERT INTO "cake" ("name", "flavor", "price", "gluten_free", "bakery_id") '
    'VALUES ($1, CAST($2::text AS public.flavor), $3, $4, $5)'
)
assert [value.value for value in compiled.params] == [
    "Blueberry Muffin", "vanilla", Decimal("2.75"), False, 2,
]

# The values were copied as the insert was built.
muffin["name"] = "Something Else"
assert insert.inspect().params[0].value == "Blueberry Muffin"
```

Each value converts to its field's kind as the write is built, as a
comparison's value does, and one that is not of the kind is a
`ConstructionError` before anything is sent: a `float` for a `"decimal"`
field, a `bool` for an `"i32"` one, a number too large for it, or a label an
enum column does not list. A value that is a `pgorm.Expr` is written as the
expression it is. The fields are written in the order the model declares
them, whatever order the mapping names them in, and the values are copied as
the write is built, so changing the mapping afterwards changes nothing. In a
`"json"` field, `None` is SQL NULL and `pgorm.Value.json(None)` is JSON's
`null`; a list or a dictionary there is a JSON value.

Here Python differs from Rust. Rust's third state, `Unchanged(v)`, a column
that already holds `v` and is not written, has no place in a mapping, which
says only what to write. Nor is there anything like Rust's free function
`set` or `.into()`: the mapping holds the value itself.

A [registered entity](24-registrations.md) writes through its real Rust
`ActiveModel`. `entity.active()` makes one through the entity's `new` hook,
which leaves every column not set unless the hook fills some. `set(column,
value)` sets a column and `not_set(column)` leaves it out again. Each
returns a new active model and leaves the one it was called on as it was, so
an active model can be reused, and it never marks itself saved.
`get(column)` reports a column's `ActiveValue`: its `state`, which is
`ActiveState.NotSet`, `ActiveState.Set` or `ActiveState.Unchanged`, and its
`value`, a `pgorm.Value`. The value is `None` only for a column not set; a
column set to SQL NULL holds a null `Value`. An active model, a model or a
column of another entity given to an entity's write is an argument of the
wrong type, a `TypeError`. Columns are named by their SQL names, and a
generated module's typed view adds a setter for each, `set_name(..)` for
`name`:

```python registered
from decimal import Decimal

from pgorm import ActiveState
from pgorm.rust_bakery import Cake

muffin = (
    Cake.active()
    .set_name("Blueberry Muffin")
    .set_flavor("vanilla")
    .set_price(Decimal("2.75"))
    .set_gluten_free(False)
    .set_bakery_id(2)
)
assert muffin.get("id").state == ActiveState.NotSet
assert muffin.get("name").state == ActiveState.Set
value = muffin.get("name").value
assert value is not None and value.value == "Blueberry Muffin"

# not_set leaves a column out again, in a new active model.
nameless = muffin.not_set("name")
assert nameless.get("name").state == ActiveState.NotSet
assert muffin.get("name").state == ActiveState.Set
```

A model read through a registered entity becomes an active model with
`into_active()`, each of its columns `Unchanged`, so writing it back changes
only the columns then set. `set_if_not_equals(column, value)` sets a
column only when the value differs from the one read, as Rust's does, so
`is_changed()` then says whether there is anything to write; `set` always
sets, even to the value already there. `reset(column)` turns an `Unchanged`
column into a `Set` one, to write it although its value is the same.
`model.with_value(column, value)` gives a copy of a model with one value
changed, and writes nothing.

## Inserting a row

`insert(values)` builds the `INSERT` of one row, Rust's `Insert::one`, and
its terminals say what comes back:

| Terminal | Returns | Rust |
| --- | --- | --- |
| `execute(db)` | the number of rows written, with no `RETURNING` | `exec` |
| `returning("id").one(db)` | a record of the fields named, such as the key | `exec_returning_pk` |
| `returning().one(db)` | the row as a record, every field | `exec_returning_model` |
| `returning_upsert(db)` | whether it inserted or updated, [below](#upserts) | `exec_returning_upsert` |

Here Python differs from Rust. A declared model's insert has no
`returning_pk` or `returning_model`: `returning(*fields)` names the fields to
read back, every field when it names none, and its `one`, `optional` and
`all` decode what the statement returned, as records. The row comes back as
the database stored it, its generated `id` and its defaults included.

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Pool

from bakery import Cake


def cupcake(name: str, price: str) -> dict[str, object]:
    return {
        "name": name,
        "flavor": "chocolate",
        "price": Decimal(price),
        "gluten_free": False,
        "bakery_id": 1,
    }


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        key = await Cake.insert(cupcake("Mocha Cupcake", "3.10")).returning("id").one(pool)
        assert key["id"] == 6

        cherry = await Cake.insert(cupcake("Cherry Cupcake", "3.30")).returning().one(pool)
        assert (cherry["id"], cherry["name"]) == (7, "Cherry Cupcake")

        written = await Cake.insert(cupcake("Plain Cupcake", "2.50")).execute(pool)
        assert written == 1


asyncio.run(main())
```

The statement binds every value as a parameter. An enum label is cast to
its type, since it travels as text, as the first example shows. Here the
SQL differs from Rust's in one place: the cast names the enum's schema,
`public.flavor`, as the model's `TypeName` does.

An insert must set every field that is neither nullable, defaulted nor
generated, and one that leaves such a field out is refused with a
`ConstructionError` before anything is sent, where Rust leaves the column out
and the database refuses the row. An insert that sets no field at all writes
a row of defaults, `INSERT INTO .. VALUES (DEFAULT)`. A model under an alias
does not insert.

Rust's `ActiveModel::insert` writes the row and runs the entity's
[hooks](03-entities.md#hooks). A declared model has no hooks, and its insert
is Rust's `Insert::one`, which runs none either. A registered entity has
both forms. `active.insert(db)` runs its hooks and returns the model the
database stored; `entity.insert(active)` builds the statement, whose terminals are
Rust's own: `execute`, `returning_pk`, which gives the key's value, or a
tuple of a composite key's in its columns' order, `returning_model` and
`returning_upsert`. A row the conflict clause held back is a `DecodeError`
from `returning_pk` and `returning_model`.

## Inserting many rows

`insert_many(rows)` writes a batch in one statement, Rust's `Insert::many`.
`returning(..).all(db)` returns one record per row, in the order the rows
were given, so `returning("id")` gives their keys, and `execute` returns the
count. `add(values)` and `add_many(rows)` grow a batch, and turn a one-row
insert into a batch, keeping its conflict clause.

Every row of a batch must set the same fields, since one `INSERT` has one
column list; a batch that does not is refused with a `ConstructionError`
before anything is sent, and so is a batch of several rows that set no
field: insert those one at a time. A batch of no rows sends nothing:
`execute` returns `0` and the returning terminals no rows.

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Pool

from bakery import Cake


def tart(name: str, flavor: str, price: str) -> dict[str, object]:
    return {
        "name": name,
        "flavor": flavor,
        "price": Decimal(price),
        "gluten_free": True,
        "bakery_id": 2,
    }


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        keys = await Cake.insert_many([
            tart("Lemon Tart", "lemon", "4.20"),
            tart("Chocolate Tart", "chocolate", "4.50"),
        ]).returning("id").all(pool)
        assert [key["id"] for key in keys] == [6, 7]

        tarts = await (
            Cake.insert_many([tart("Vanilla Tart", "vanilla", "4.00")])
            .add(tart("Carrot Tart", "carrot", "4.10"))
            .returning()
            .all(pool)
        )
        assert [row["name"] for row in tarts] == ["Vanilla Tart", "Carrot Tart"]

        assert await Cake.insert_many([]).execute(pool) == 0


asyncio.run(main())
```

An insert is typed by its row count, as Rust's is. `insert` gives a
`ModelInsert` and `insert_many`, `add` and `add_many` a `ModelInsertMany`,
each with only its own terminals: `returning_upsert` on the first and
`returning_upserts` on the second, so a type checker refuses the other. A
registered entity's `insert_many(actives)` has Rust's batch terminals,
`execute`, `returning_pks`, `returning_models` and `returning_upserts`, and
its active models must set the same columns too.

## Updating rows

`update(values)` builds an `UPDATE` that writes the fields `values` sets, and
leaves the others as they are. `where(..)` says which rows it changes, and
`returning(..)` reads them back as they now are.

Here Python differs from Rust. Rust changes a row it has read by turning the
model into an active model, setting the columns that change, and calling
`update`, which finds the row by its primary key. A declared model's record
does not turn into a write: the update names its row with `where`, and
`key(..)` is the condition that matches one primary key:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Pool

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        cheaper = Cake.update({"price": Decimal("3.99")}).where(Cake.key({"id": 1}))
        assert cheaper.inspect().sql == 'UPDATE "cake" SET "price" = $1 WHERE "cake"."id" = $2'

        fudge = await cheaper.returning().one(pool)
        assert str(fudge["price"]) == "3.99"
        assert fudge["name"] == "Chocolate Fudge"


asyncio.run(main())
```

A key that matches no row updates nothing: `execute` returns `0`,
`returning(..).optional` gives `None`, and `returning(..).one` raises
`DecodeError`, where Rust's `Update::one` reports
`Error::RecordNotFound`. `key` requires every field of the key and no other,
so a key with a part missing is a `ConstructionError` before anything is
sent, as Rust's `Error::PrimaryKeyNotSet` is.

A registered entity updates the Rust way. A model it read turns into an
active model with `into_active()`, every column `Unchanged`; `set` changes
one, and the active model's `update(db)` writes only the columns set, finds
the row by its key, runs the entity's hooks, and returns the row as it now
is:

```python registered
import asyncio
import os
from decimal import Decimal

from pgorm import ActiveState, Pool
from pgorm.rust_bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        fudge = await Cake.find().where(Cake.col("id") == 1).one(pool)
        cheaper = fudge.into_active()
        assert cheaper.get("price").state == ActiveState.Unchanged

        # The same name is no change.
        cheaper = cheaper.set_if_not_equals("name", "Chocolate Fudge")
        assert not cheaper.is_changed()

        cheaper = cheaper.set_price(Decimal("3.99"))
        assert cheaper.is_changed()
        assert cheaper.get("price").state == ActiveState.Set
        assert cheaper.get("name").state == ActiveState.Unchanged

        fudge = await cheaper.update(pool)
        assert str(fudge.price) == "3.99"
        assert fudge.name == "Chocolate Fudge"


asyncio.run(main())
```

An update of many rows is the same `update` with a `where` that matches
them, Rust's `Update::many`. A value that is an expression can read the
row's own columns, as Rust's `col_expr` does. `execute` returns the number of
rows changed, and `returning(..).all` the rows themselves:

```python
import asyncio
import os
from decimal import Decimal

from pgorm import Pool

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # Sunrise raises every price by fifty cents.
        raise_ = Cake.update({"price": Cake.col("price") + Decimal("0.50")}).where(
            Cake.col("bakery_id") == 1
        )
        assert raise_.inspect().sql == (
            'UPDATE "cake" SET "price" = "cake"."price" + $1::numeric WHERE "cake"."bakery_id" = $2'
        )
        assert await raise_.execute(pool) == 3

        # Moonlight's cakes all become gluten-free.
        changed = await (
            Cake.update({"gluten_free": True})
            .where(Cake.col("bakery_id") == 2)
            .returning()
            .all(pool)
        )
        assert all(cake["gluten_free"] for cake in changed)
        assert len(changed) == 2


asyncio.run(main())
```

The expression reads the column qualified by its table, `"cake"."price"`,
where Rust's `col_expr` writes `"price"`; PostgreSQL reads both the same.

An update or a delete must say which rows it writes before it runs: a
`where(..)`, or `all_rows()` for every row. One with neither is a
`ConstructionError` as it is inspected or run. `all_rows()` permits a
statement with no condition; it does not remove a condition already given.
Here Python differs from Rust, whose `Update::many` and `Delete::many` with
no filter write every row. The guard asks for the intent, nothing more: a
condition that is always true passes it.

An update that sets nothing is a `ConstructionError` as it is built, as
Rust's `Error::NothingToSet` is. `UPDATE` takes no `ORDER BY` or `LIMIT` in
PostgreSQL, so an update has neither; filter on a subquery instead.

`from_(source)` names another table the update reads, PostgreSQL's
`UPDATE .. FROM`, as Rust's `from` does: another model, whose table it reads
as the model names it, or a table or FROM item of [the query
builder](11-query-builder.md). Its columns go wherever the update's values
and conditions go, and a `RETURNING` list then qualifies the model's own
columns, so the two tables' `id`s cannot be confused. Here every cake sold
in Bergen becomes gluten-free:

```python
import asyncio
import os

from pgorm import Pool

from bakery import Bakery, Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        bergen = (
            Cake.update({"gluten_free": True})
            .from_(Bakery)
            .where(Cake.col("bakery_id") == Bakery.col("id"))
            .where(Bakery.col("city") == "Bergen")
        )
        assert bergen.inspect().sql == (
            'UPDATE "cake" SET "gluten_free" = $1 FROM "bakery" '
            'WHERE "cake"."bakery_id" = "bakery"."id" AND "bakery"."city" = $2'
        )
        changed = await bergen.returning("name").all(pool)
        assert sorted(cake["name"] for cake in changed) == ["Carrot Cake", "Vanilla Sponge"]


asyncio.run(main())
```

A registered entity's `update_many()` takes `set(column, value)`, a value
converted to the column's type or an expression as written, and `where(..)`
or `all_rows()`, and its terminal is `returning_changes`, below.

### Before and after

PostgreSQL 18's `RETURNING` can read a row both as it was and as the update
left it. `returning_change(db)` and `returning_changes(db)` return each row
as a `Change`, its `old` and `new` records. `returning_change` reads exactly
one row, and any other count is a `DecodeError`:

```python
import asyncio
import os

from pgorm import Pool

from bakery import Cake


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        changes = await (
            Cake.update({"price": Cake.col("price") * 2})
            .where(Cake.col("name") == "Vanilla Sponge")
            .returning_changes(pool)
        )
        assert len(changes) == 1
        assert str(changes[0].old["price"]) == "2.90"
        assert str(changes[0].new["price"]) == "5.80"


asyncio.run(main())
```

The count is checked once the statement has run, so an update that
matched two rows has changed both by the time `returning_change` raises; a
`DecodeError` from any write's terminal leaves the write done. Run it in a
[transaction](08-transactions.md) for the error to undo it.

`inspect(terminal="returning_changes")` shows the statement the terminal
runs. Its `RETURNING` list renames the two versions,
`RETURNING WITH (OLD AS "pgorm_old", NEW AS "pgorm_new")`, as Rust's does, so
a table called `old` or `new` still reads them in order.

A registered entity's `update(active).returning_change(db)` and
`update_many().set(..).where(..).returning_changes(db)` are Rust's
`exec_returning_change` and `exec_returning_changes`, each version a model of
the entity, and they run no hooks. With nothing set, `returning_change`
sends no `UPDATE`: it returns the row its key reads as both versions, and a
key matching no row raises `DecodeError`. `returning_changes` with nothing
set is a `ConstructionError`, and so is an `update_many()` with neither
`where(..)` nor `all_rows()`.

## Deleting rows

`delete()` builds a `DELETE`, which needs `where(..)` or `all_rows()` before
it runs, as an update does. `execute` returns the number of rows deleted, and
`returning(..).all` the rows as they were. A delete that matches nothing
returns `0`.

Here Python differs from Rust. A declared model's record does not delete
itself, and there is no `delete_by_id`: a delete by key is `delete()` with
`where(model.key(..))`. A registered entity's model does:
`model.into_active().delete(db)` deletes its row by its key, returns the
count, and runs the entity's `before_delete` and `after_delete` hooks.

```python
import asyncio
import os

from pgorm import Pool, Select

from bakery import Cake, CakeBaker


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # Cy no longer bakes the Vanilla Sponge.
        gone = CakeBaker.delete().where(CakeBaker.key({"cake_id": 4, "baker_id": 3}))
        assert await gone.execute(pool) == 1

        # Bo leaves the Lemon Drizzle to Ada.
        left = await (
            CakeBaker.delete()
            .where(CakeBaker.key({"cake_id": 2, "baker_id": 2}))
            .returning()
            .all(pool)
        )
        assert [dict(row) for row in left] == [{"cake_id": 2, "baker_id": 2}]

        # Moonlight stops selling the cakes nobody bakes.
        baked = Select(CakeBaker.col("cake_id")).from_(CakeBaker.table)
        unbaked = (
            Cake.delete()
            .where(Cake.col("bakery_id") == 2)
            .where(Cake.col("id").is_not_in(baked))
        )
        assert unbaked.inspect().sql == (
            'DELETE FROM "cake" WHERE "cake"."bakery_id" = $1 '
            'AND "cake"."id" NOT IN (SELECT "cake_baker"."cake_id" FROM "cake_baker")'
        )
        assert await unbaked.execute(pool) == 1
        assert await Cake.find().count(pool) == 4


asyncio.run(main())
```

Each `where` adds a condition, and they all must hold. `using(source)` names
another table the delete reads, PostgreSQL's `DELETE .. USING`, as Rust's
`using` does, from another model or a table or FROM item, as an update's
`from_` does:

```python
import asyncio
import os

from pgorm import Pool

from bakery import Baker, CakeBaker


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # Bo hands every cake on to the others.
        handed = CakeBaker.delete().using(Baker).where(CakeBaker.col("baker_id") == Baker.col("id"))
        handed = handed.where(Baker.col("name") == "Bo")
        assert handed.inspect().sql == (
            'DELETE FROM "cake_baker" USING "baker" '
            'WHERE "cake_baker"."baker_id" = "baker"."id" AND "baker"."name" = $1'
        )
        assert await handed.execute(pool) == 2


asyncio.run(main())
```

## Upserts

An upsert inserts a row unless it would conflict with one already there, and
then does something else: updates the row that is there, or skips the new
one. `on_conflict(action)` attaches PostgreSQL's `ON CONFLICT` clause, built
with `pgorm.Conflict`: `Conflict.on(..)` names the columns of the unique key
that may conflict, and `update(..)` the columns whose new value comes from
the row that failed to insert. `Conflict` is the query builder's, so it
names SQL columns, not a model's fields;
[The query builder](11-query-builder.md) covers the rest of it.

The bakery's customers are unique by email. A sign-up from an address already
known updates the name instead of failing. `returning_upsert(db)` says which
happened: an `Inserted` holding the new row as its `model`, or an `Updated`
holding the row before and after as its `change`. It answers `None` when the
conflict clause wrote nothing.

```python
import asyncio
import os

from pgorm import Conflict, ModelInsert, Pool
from pgorm._registered.entities import Inserted, Updated

from bakery import Customer


def sign_up(name: str, email: str) -> ModelInsert:
    return Customer.insert({"name": name, "email": email, "notes": None}).on_conflict(
        Conflict.on("email").update("name")
    )


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        match await sign_up("Kimberly", "kim@example.com").returning_upsert(pool):
            case Updated(change):
                assert (change.old["name"], change.new["name"]) == ("Kim", "Kimberly")
            case other:
                raise AssertionError(f"Kim signed up before: {other!r}")

        match await sign_up("Mo", "mo@example.com").returning_upsert(pool):
            # Kim's upsert drew id 3 before it conflicted, so Mo gets 4.
            case Inserted(mo):
                assert mo["id"] == 4
            case other:
                raise AssertionError(f"Mo is new: {other!r}")


asyncio.run(main())
```

Here Python differs from Rust in where the three classes live. `Inserted`,
`Updated` and `Change` are not exported from `pgorm` itself yet; import them
from `pgorm._registered.entities`, where the generated typed views take
them from. They are generic over the record, so `Updated[ModelRecord]`
annotates a declared model's answer. A registered entity reached through
`pgorm.entity(name)` answers `returning_upsert` with
`pgorm.Upserted.Inserted` or `pgorm.Upserted.Updated`, whose `into_model()`
is the row as the statement left it either way; a generated typed view
answers with the generic classes, holding its typed models.

An insert draws its generated key before it finds the conflict, so a
conflicting row uses up a number of the sequence, as Kim's did.
`returning_upserts(db)` does the same as `returning_upsert` for a batch, row
by row, leaving out a row the conflict clause did not write.
`inspect(terminal="returning_upsert")`, or `"returning_upserts"` on a batch,
shows the statement the terminal runs. The other terminals work under
`ON CONFLICT` too: `returning("id").one(db)` returns the key of the row
written, whether it was inserted or updated.

To skip conflicting rows instead, `on_conflict_do_nothing()` adds
`ON CONFLICT (primary key) DO NOTHING`, as Rust's does, and turns the insert
into a `ModelTryInsert`, or a batch into a `ModelTryInsertMany`, Rust's
`TryInsert`. Its terminals answer with a `TryInsertResult`:
`TryInsertResult.Inserted` holding what was written, `TryInsertResult.Conflicted()`
when every row was skipped, or `TryInsertResult.Empty()` when there was nothing
to write and nothing was sent. `execute(db)` answers with the count, and
`returning(*fields)` with the rows, its `one(db)` for one row and `all(db)`
for a batch. `on_empty_do_nothing()` makes a try-insert too, its statement
unchanged, answering an empty batch, or a row that sets no field, with
`Empty`. `Conflict.on(..).do_nothing()` names another key through
`on_conflict`, and `Conflict.do_nothing()` skips a row that conflicts on any
key.

```python
import asyncio
import os

from pgorm import ModelTryInsert, Pool, TryInsertResult

from bakery import CakeBaker


def bakes(cake_id: int, baker_id: int) -> ModelTryInsert:
    return CakeBaker.insert({"cake_id": cake_id, "baker_id": baker_id}).on_conflict_do_nothing()


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # Ada already bakes the Chocolate Fudge.
        again = bakes(1, 1)
        assert again.inspect().sql == (
            'INSERT INTO "cake_baker" ("cake_id", "baker_id") VALUES ($1, $2) '
            'ON CONFLICT ("cake_id", "baker_id") DO NOTHING'
        )
        assert await again.execute(pool) == TryInsertResult.Conflicted()

        # Bo starts baking the Chocolate Fudge too.
        match await bakes(1, 2).execute(pool):
            case TryInsertResult.Inserted(count):
                assert count == 1
            case other:
                raise AssertionError(f"Bo is new to it: {other!r}")

        empty = await CakeBaker.insert_many([]).on_empty_do_nothing().execute(pool)
        assert empty == TryInsertResult.Empty()


asyncio.run(main())
```

Here Python differs from Rust: a declared model's try-insert has no
`returning_pk` or `returning_model`; `returning(*fields)` reads the fields it
names, as a plain insert's does, and a row the conflict clause skipped is
`Conflicted` there too.

## MERGE

PostgreSQL's `MERGE` reconciles a table with a source of rows in one
statement: it updates the rows that match, inserts the ones that do not, and
can delete. As in Rust, a model has no `MERGE` of its own: pgorm builds it
with the query builder's `pgorm.merge(target, source, on)`, over the tables
the models name, and [The query builder](11-query-builder.md) covers it.
Here a staged list of sign-ups is merged into the customers, updating the
names of those already known and inserting the rest:

```python
import asyncio
import os

from pgorm import Column, MergeAction, Model, Pool, merge
from pgorm import schema as s

from bakery import Customer

SignUp = Model("sign_up", {"name": Column("text"), "email": Column("text")})


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        await pool.execute(
            s.create_table("sign_up")
            .column(s.ColumnDef("name", "varchar").not_null())
            .column(s.ColumnDef("email", "varchar").not_null())
        )
        await SignUp.insert_many([
            {"name": "Kimberly", "email": "kim@example.com"},
            {"name": "Mo", "email": "mo@example.com"},
        ]).execute(pool)

        target, staged = Customer.table.as_("c"), SignUp.table.as_("s")
        sync = (
            merge(target, staged, target.col("email") == staged.col("email"))
            .when_matched(MergeAction.update("name", staged.col("name")))
            .when_not_matched(
                MergeAction.insert("name", staged.col("name")).set("email", staged.col("email"))
            )
            .returning_action()
            .returning(target.col("name"))
        )
        assert sync.inspect().sql == (
            'MERGE INTO "customer" AS "c" USING "sign_up" AS "s" ON "c"."email" = "s"."email" '
            'WHEN MATCHED THEN UPDATE SET "name" = "s"."name" '
            'WHEN NOT MATCHED THEN INSERT ("name", "email") VALUES ("s"."name", "s"."email") '
            'RETURNING merge_action(), "c"."name"'
        )
        rows = await pool.query(sync)
        assert sorted((row["merge_action"], row["name"]) for row in rows) == [
            ("INSERT", "Mo"), ("UPDATE", "Kimberly"),
        ]


asyncio.run(main())
```

A `MERGE` runs as any statement does, through a pool, a connection or a
transaction, and its rows are `Record`s, not a model's records.
