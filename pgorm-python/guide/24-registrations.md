# Registering Rust entities

A Python program declares its tables as models ([Models and
entities](03-entities.md)). An application that already describes its tables
in Rust, as entities with relations, hooks and graphs, can use those Rust
types from Python instead of declaring the tables again. It *registers* them:
it compiles them into the package's native module, under names a Python
program looks them up by. This chapter has no Rust counterpart. It says what
a registration gives a program, how to use one, and how to build a wheel that
holds them.

## What a registration gives

A registered entity is the Rust entity itself, not a copy of its description.
A query on it builds the entity's own `Select<E>`, so its SQL is the SQL the
Rust program sends; a row comes back as the entity's `Model`, decoded by its
own `FromQueryResult`; and a write goes through its `ActiveModel`, with the
`ActiveModelBehavior` hooks the Rust crate wrote. Beside entities, a crate can
register the `SelectGraph` shapes it builds, the source tuples a pipeline
selects, and its disjoint enums and one-of references. Everything runs in the
Python process, over PostgreSQL's own protocol.

Register when the Rust crate is where the tables are described: an application
with a Rust service and a Python one over the same database, or an entity
whose hooks keep a rule that a Python write must keep too. A program written
only in Python needs none of this, and declares models.

Here Python differs from Rust: a Rust program can instantiate any generic
type over any entity, and Python cannot. Every entity, graph shape and source
tuple a Python program uses is one a Rust crate compiled into the module
ahead of time. The package from [Getting started](01-getting-started.md), the
*standalone* wheel, registers nothing:

```python
import pgorm
from pgorm import pipeline

assert pgorm.entities() == []
assert pgorm.graphs() == []
assert pipeline.source_tuples() == []
assert pgorm.disjoint_entities() == []
assert pgorm.one_of_entities() == []
assert pgorm.capabilities()["registrations"]["entities"] == []

try:
    pgorm.entity("rust_bakery.Cake")
except pgorm.UnsupportedCapabilityError as error:
    assert isinstance(error, pgorm.ConstructionError)
    assert str(error) == "entity is not registered in this native build"
else:
    raise AssertionError("the standalone wheel registers no entity")
```

A wheel that registers entities is an *application wheel*: one native module
holding pgorm's whole Python API and the application's registrations. It
supplies the `pgorm` distribution, so it replaces the standalone wheel in the
environment it is installed into. Two application extensions cannot be
stacked in one process, each with its own copy of pgorm. The last sections of
this chapter build one, by hand or with `pgorm.codegen`.

The examples in this chapter use the guide's application wheel, which
registers the Rust guide's seven bakery entities from the
`pgorm-guide-support` crate as `rust_bakery.Bakery`, `rust_bakery.Baker`,
`rust_bakery.Cake` and so on, over the same tables [`bakery.py`](bakery.py)
creates.

## The generic API

`pgorm.entity(name)` gives the entity registered under `name`, and
`pgorm.entities()` lists every registered name, sorted. A name the module
does not register raises `UnsupportedCapabilityError`, which is a
`ConstructionError`. The entity has the shape of a model: `find()` starts a
query, `col(name)` names a column by its SQL name, and the terminals `all`,
`one` and `optional` run the query on a pool, a connection or a transaction.
A row is an `EntityModel`, read like a dictionary by SQL column name:

```python registered
import asyncio
import os

import pgorm
from pgorm import Pool


async def main() -> None:
    assert pgorm.entities() == [
        "rust_bakery.Baker", "rust_bakery.Bakery", "rust_bakery.Cake", "rust_bakery.CakeBaker",
        "rust_bakery.Customer", "rust_bakery.Order", "rust_bakery.OrderLine",
    ]
    cake = pgorm.entity("rust_bakery.Cake")
    query = cake.find().where(cake.col("gluten_free").eq(True)).order_by(cake.col("name").asc())

    # The Rust entity's own SELECT: it reads its enum column through its text.
    compiled = query.inspect()
    assert compiled.sql == (
        'SELECT "cake"."id", "cake"."name", CAST("cake"."flavor" AS text), "cake"."price", '
        '"cake"."gluten_free", "cake"."bakery_id" FROM "cake" '
        'WHERE "cake"."gluten_free" = $1 ORDER BY "cake"."name" ASC'
    )
    assert [value.value for value in compiled.params] == [True]

    async with Pool(os.environ["DATABASE_URL"]) as pool:
        cakes = await query.all(pool)
        assert [model["name"] for model in cakes] == [
            "Chocolate Torte", "Lemon Drizzle", "Vanilla Sponge",
        ]
        first = await query.optional(pool)
        assert first is not None
        assert dict(first)["flavor"] == "chocolate"
        assert first.entity_name == "rust_bakery.Cake"


asyncio.run(main())
```

`all` returns every row in a list. `one` and `optional` are Rust's `one` and
`one_opt`: they narrow the query to one row, `LIMIT 1` even when an earlier
`limit` was larger and `LIMIT 0` when it was zero. `one` raises `DecodeError`
when there is no row, and `optional` returns `None`.
`inspect(terminal="one")` or `inspect(terminal="optional")` shows the SQL
with that limit. These are the Rust selector's rules; the `one` of a pool, a
connection or a transaction, which runs a statement you built, has its own
([Reading rows](05-reading.md)). A query also counts, pages, takes a keyset
cursor and streams, joins a relation and reads some of its columns, [as
below](#counts-pages-cursors-and-streams).

Here Python differs from Rust: every terminal and write returns an
awaitable, and runs on a `Pool` as well as on a `Connection` or a
`Transaction`. On a pool it runs on a connection the pool lends it alone, as
a pool's statements do. A connection runs one operation at a time, so await
each one before starting the next on it. Cancelling an operation, through its
task or `asyncio.wait_for`, discards its connection when the operation's
outcome is uncertain, and does not promise that a write already sent was
rolled back ([Python and asyncio](02-asyncio.md)). Closing the connection or
the pool also cancels a Rust hook that is still running.

A query, like every builder here, is a value: `where`, `order_by`, `limit`
and `offset` each return a new one. Testing a query or a write for truth, as
`if query:` does, raises `ConstructionError` rather than answering.

### Counts, pages, cursors and streams

A query has the rest of Rust's `Select<E>` reads. `count(db)` is
`PaginatorTrait::count`, counting the rows with the limit, offset and
ordering dropped, and `inspect(terminal="count")` shows that statement.
`paginate(page_size)` is `PaginatorTrait::paginate`: its paginator holds no
database, so each of `fetch_page(db, n)`, `num_items(db)`, `num_pages(db)` and
`num_items_and_pages(db)` takes the one it runs on, `pages(db)` reads each
page in turn until one is empty, and `inspect(n)` or `inspect("count")` shows
a statement, as a model query's paginator does ([Pages, cursors and
streams](06-pagination.md)). `cursor(*columns)` is `Select::cursor_by` over
those columns in order: `after(*values)` and `before(*values)` take one value
per column, converted and cast as a comparison's value is, `first`, `last`,
`asc` and `desc` are the cursor's own, `inspect()` shows its statement, and
`all(db)` runs it. `stream(db)` reads the models one at a time from a pool's
connection of its own or a connection, each decoded as it arrives, as
`Select::stream` does:

```python registered
import asyncio
import os

import pgorm
from pgorm import Pool


async def main() -> None:
    cake = pgorm.entity("rust_bakery.Cake")
    by_price = cake.find().order_by(cake.col("price").asc())

    async with Pool(os.environ["DATABASE_URL"]) as pool:
        assert await by_price.count(pool) == 5

        pages = by_price.paginate(2)
        assert await pages.num_items_and_pages(pool) == (5, 3)
        second = await pages.fetch_page(pool, 1)
        assert [model["name"] for model in second] == ["Carrot Cake", "Chocolate Fudge"]

        cursor = cake.find().cursor("price", "id")
        first = await cursor.first(2).all(pool)
        assert [model["name"] for model in first] == ["Vanilla Sponge", "Lemon Drizzle"]
        last = first[-1]
        following = await cursor.after(last["price"], last["id"]).first(2).all(pool)
        assert [model["name"] for model in following] == ["Carrot Cake", "Chocolate Fudge"]

        async with await by_price.stream(pool) as models:
            names = [model["name"] async for model in models]
        assert names[:2] == ["Vanilla Sponge", "Lemon Drizzle"]


asyncio.run(main())
```

`join(relation, kind=Join.Inner)` is `QuerySelect::join` of a relation the
entity's Rust `Relation` enum declares: `entity.relation(name)` names the
variant, a name it does not declare raising `ConstructionError`, and the
joined entity's columns go in the conditions and orderings. The rows stay the
query's own models. `select(*columns)` is `select_only` with `columns`: the
query reads only those columns, each through its `select_as`, and its rows
are `Record`s, as a statement's are, rather than models; its `all`, `one`,
`optional`, `stream` and `inspect` are a query's. Here Python differs from
Rust, which decodes such a projection into a type of the program's own,
`into_tuple` or a `PartialModel`, which a binding cannot name at run time:

```python registered
import asyncio
import os

import pgorm
from pgorm import Pool


async def main() -> None:
    cake = pgorm.entity("rust_bakery.Cake")
    bakery = pgorm.entity("rust_bakery.Bakery")
    in_bergen = (
        cake.find()
        .join(cake.relation("Bakery"))
        .where(bakery.col("city") == "Bergen")
        .order_by(cake.col("name").asc())
    )
    assert in_bergen.inspect().sql == (
        'SELECT "cake"."id", "cake"."name", CAST("cake"."flavor" AS text), "cake"."price", '
        '"cake"."gluten_free", "cake"."bakery_id" FROM "cake" '
        'INNER JOIN "bakery" ON "cake"."bakery_id" = "bakery"."id" '
        'WHERE "bakery"."city" = $1 ORDER BY "cake"."name" ASC'
    )

    async with Pool(os.environ["DATABASE_URL"]) as pool:
        assert [model["name"] for model in await in_bergen.all(pool)] == ["Carrot Cake", "Vanilla Sponge"]

        names = in_bergen.select("name", "flavor")
        assert [dict(row) for row in await names.all(pool)] == [
            {"name": "Carrot Cake", "flavor": "carrot"},
            {"name": "Vanilla Sponge", "flavor": "vanilla"},
        ]


asyncio.run(main())
```

### Columns

A column's comparisons are the entity's `ColumnTrait`: `eq`, `ne`, `gt`,
`gte`, `lt` and `lte`, and the operators that spell them. The value converts
to the column's declared type and is written through the column's own cast,
so an enum's label is cast to its type, `CAST($1::text AS flavor)` for the
cake's flavor. `None` is refused; test for NULL with `is_null()`.
`between` and `not_between` (`symmetric` or not), `is_distinct_from` and
`is_not_distinct_from`, and the range tests `contains`, `contained_by` and
`overlaps` and the range positions convert and cast their values the same
way. `any_(..)` or `all_(..)` over a list compares with one array of the
column's type, cast as Rust's `eq_any` casts it.

`is_in` and `is_not_in` test membership in a list or tuple of values, each
converted the same way, or in a `Select`'s rows. A column whose type is a
domain over an enum is tested as the enum, the only way PostgreSQL compares
one, and a subquery's column is cast beside it.

An expression given instead of a value is compared as written, and so is a
list holding one. The column is itself an `Expr`, so it goes wherever an
expression goes, and every other expression method reads it as written.
`col(name).expr()` gives the plain expression, whose comparisons bind as an
`Expr`'s do rather than through the column's cast.

### Describing a registration

`describe()` reports what the Rust entity declares, as data:

```python registered
import pgorm

cake = pgorm.entity("rust_bakery.Cake")
info = cake.describe()

assert (info["name"], info["schema"], info["table"]) == ("rust_bakery.Cake", None, "cake")
assert info["primary_key"] == ["id"]
assert info["rust"]["entity"] == "pgorm_guide_support::cake::Entity"
assert info["rust"]["active_model"] == "pgorm_guide_support::cake::ActiveModel"

columns = {column["name"]: column for column in info["columns"]}
assert columns["price"]["rust_type"] == "Decimal"
assert columns["price"]["kind"] == "decimal"
assert columns["flavor"]["rust_type"] == "Flavor"
assert columns["flavor"]["kind"] == 'enum "flavor"'
assert columns["flavor"]["values"] == ["chocolate", "vanilla", "lemon", "carrot"]
assert columns["id"]["primary_key"] and not columns["id"]["nullable"]

relations = {relation["name"]: relation for relation in info["relations"]}
assert relations["OrderLine"]["type"] == "has_many"
assert relations["OrderLine"]["columns"] == [["id", "cake_id"]]
```

The keys are the ones JavaScript's `describe()` reports, each in snake_case:

- `name`, the registration's name, and the table's `schema` and `table`.
- `columns`, one entry per column: its SQL `name`; the `json_key` its Rust
  field serializes under; its `sql_type`; the `rust_type` the model reads it
  as; `nullable`; `primary_key`; the `kind` its values bind and read as
  (`i32`, `text[]`, `enum "app"."mood"`, `range "app"."floatrange" of f64`,
  or `None` for a type that takes an explicit `Value`); and an enum's
  `values`.
- `primary_key`, the key's columns, and `primary_key_without_overlaps`, true
  when the key's last column is a period declared `WITHOUT OVERLAPS`. The
  server then holds a row per period, none overlapping, but a lookup, an
  update and a delete by the key still compare the period for equality, so
  each names exactly one version. Which version covers a moment is a filter
  on the period, not a key lookup.
- `unique_keys`, each unique key the entity's `unique(..)` declares: its
  `name` (`None` where PostgreSQL names it), its `columns` and
  `nulls_not_distinct`.
- `relations`, one entry per variant of the entity's `Relation` enum: its
  `type`, its `from` and `to` tables, and what a join by it reads, namely the
  `columns` paired for equality, the `period` pair a temporal foreign key
  matches for overlap, `on_condition`, true where a Rust closure adds a
  condition of its own (the closure is Rust code and is not reported), and
  `condition_type`, `"all"` where the join needs the columns and that
  condition both and `"any"` where either suffices. Its `enforcement`
  (`"not_enforced"` for a key the server never checks) and `deferrability`
  (`"deferrable_initially_deferred"` for one checked at commit), each `None`
  where the relation does not say, decide what the server holds the rows to;
  a join by the relation reads the same either way.
- `rust`, the Rust `entity`, `model`, `active_model` and `column` type names.

`pgorm.capabilities()["registrations"]` lists every registration as its
`describe()` reports it, under `entities`, `graphs`, `sources`, `disjoints`
and `one_ofs`.

### Writing through the ActiveModel

`active()` gives a new `ActiveModel`, built by the entity's
`ActiveModelBehavior::new`, so the defaults the Rust crate sets there are
set. Each column of it holds an `ActiveValue`, whose `state` is one of
`ActiveState.NotSet`, `ActiveState.Set` and `ActiveState.Unchanged`, and
whose `value` is the `Value` it holds, `None` only for `NotSet` (an SQL NULL
is a `Value` of its own). `set(column, value)`, `not_set(column)` and
`reset(column)`, which marks an unchanged column set, each return a new
`ActiveModel` and leave the one they were called on as it was. `insert`,
`update` and `delete` write it, and each works on a copy, so an
`ActiveModel` can be written again; it does not mark itself saved.

A model read back converts with `into_active()`, the Rust conversion, which
leaves each column `Unchanged`; set the ones to change. `with_value(column,
value)` gives a copy of a model with one value changed, through Rust's
`ModelTrait::set`, and writes nothing.

```python registered
import asyncio
import os
from decimal import Decimal

import pgorm
from pgorm import ActiveState, Pool


async def main() -> None:
    cake = pgorm.entity("rust_bakery.Cake")
    draft = (
        cake.active()
        .set("name", "Lemon Scone")
        .set("flavor", "lemon")
        .set("price", Decimal("2.50"))
        .set("gluten_free", False)
        .set("bakery_id", 1)
    )
    assert draft.get("id").state == ActiveState.NotSet
    assert draft.get("name").state == ActiveState.Set

    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # The id is not set, so the database chooses it.
        scone = await draft.insert(pool)
        assert scone["id"] == 6
        assert scone["price"] == Decimal("2.50")

        loaded = scone.into_active()
        assert loaded.get("price").state == ActiveState.Unchanged
        cheaper = await loaded.set("price", Decimal("2.20")).update(pool)
        assert cheaper["price"] == Decimal("2.20")
        # Each write returns a new model; the one read before is unchanged.
        assert scone["price"] == Decimal("2.50")

        assert await cheaper.into_active().delete(pool) == 1
        assert await cake.find().where(cake.col("id").eq(6)).optional(pool) is None


asyncio.run(main())
```

A plain Python value given to `set` converts as the column's declared SQL
type says, the hint the `kind` in `describe()` names; the Rust setter makes
the final check. A `Value` tagged with an explicit kind keeps that kind, which
is how to write a column whose Rust field has a custom type the hint cannot
reach. An enum value tagged with a qualified type must name the column's own.
A column of a range type the schema created (a `CreatedRange` field in Rust)
takes a plain `pgorm.Range` or `pgorm.Multirange`, converted with the
subtype's limits, and reads back as the range, tagged with the created type.

A row decodes through the entity's own `Model::from_query_result`, a custom
Rust decoder included, and then converts each Rust value to Python. `dict(model)`
gives the Python values, and `model.tagged(column)` the `Value` with its Rust
tag, an enum's qualified type included. The stricter rules a `Record` from a
statement you built follows about PostgreSQL's wire types
([Values and types](13-values.md)) apply to records, not to a registered
entity's models, whose decoder is the application's.

### Hooks

An entity whose `Model` says `#[pgorm(hooks)]` implements
`ActiveModelBehavior` itself, as [the Rust guide's
Entities](../../docs/guide/02-entities.md) teaches. `insert`, `update` and
`delete` on a registered entity's `ActiveModel` run those hooks, the same
code a Rust program's writes run:

```rust
use pgorm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, Entity)]
#[pgorm(table_name = "special", hooks)]
pub struct Model {
    #[pgorm(primary_key)]
    pub id: i32,
    pub name: String,
    pub note: String,
}

#[derive(Copy, Clone, Debug, Relation)]
pub enum Relation {}

#[async_trait::async_trait]
impl ActiveModelBehavior for ActiveModel {
    fn new() -> Self {
        Self {
            note: set("Ask at the counter"),
            ..ActiveModelTrait::default()
        }
    }

    async fn before_save<C>(self, _db: &C, _insert: bool) -> Result<Self, Error>
    where
        C: ConnectionTrait,
    {
        if self.name.as_ref().is_empty() {
            return Err(Error::Custom("a special needs a name".to_owned()));
        }
        Ok(self)
    }
}
```

Registered as `app.Special`, `pgorm.entity("app.Special").active()` starts
with `note` already `Set` to `"Ask at the counter"`, because `active()` calls
`new`. An insert whose `name` is empty is refused by `before_save` before
anything is sent: the hook's `Error::Custom` raises `ConstructionError`
carrying its message, `a special needs a name`. An error of any other kind a
hook returns keeps its own class, a `DatabaseError` from a statement the hook
ran among them. A `before_save` or `before_delete` that fails stops the write.
An `after_save` or `after_delete` that fails raises too, though the write it
follows has already run, and outside a transaction has committed. The
binding adds no transaction and no retry around a write: run it in a
[transaction](08-transactions.md) to make the write and its hooks one unit.

The guide's application wheel registers the bakery's entities, which have no
hooks, so their `active()` starts with every column `NotSet`.

### Writes that return both versions, and the insert terminals

`update`, `update_many`, `insert` and `insert_many` on the entity build the
Rust statements of [Writing rows](04-writing.md), with Rust's terminals:

| Python | Rust |
| --- | --- |
| `entity.update(active).returning_change(db)` | `Update::one(active)?.exec_returning_change` |
| `entity.update_many().set(..).where(..).returning_changes(db)` | `Update::many(entity).col_expr(..).filter(..).exec_returning_changes` |
| `entity.update_many().set(..).all_rows().returning_changes(db)` | `Update::many(entity).col_expr(..).exec_returning_changes` |
| `entity.insert(active).execute(db)` | `Insert::one(active).exec` |
| `entity.insert(active).returning_pk(db)` | `Insert::one(active).exec_returning_pk` |
| `entity.insert(active).returning_model(db)` | `Insert::one(active).exec_returning_model` |
| `entity.insert(active).on_conflict(..).returning_upsert(db)` | `Insert::one(active).on_conflict(..).exec_returning_upsert` |
| `entity.insert_many(actives).execute(db)` | `Insert::many(actives).exec` |
| `entity.insert_many(actives).returning_pks(db)` | `Insert::many(actives).exec_returning_pks` |
| `entity.insert_many(actives).returning_models(db)` | `Insert::many(actives).exec_returning_models` |
| `entity.insert_many(actives).on_conflict(..).returning_upserts(db)` | `Insert::many(actives).on_conflict(..).exec_returning_upserts` |
| `insert.add(active)`, `insert.add_many(actives)` | `Insert::add`, `Insert::add_many` |
| `entity.insert(active).on_conflict_do_nothing().execute(db)` | `Insert::one(active).on_conflict_do_nothing().exec` |
| `entity.insert(active).on_empty_do_nothing().returning_model(db)` | `Insert::one(active).on_empty_do_nothing().exec_returning_model` |
| `entity.insert_many(actives).on_conflict_do_nothing().returning_pks(db)` | `Insert::many(actives).on_conflict_do_nothing().exec_returning_pks` |

An insert is typed by its row count, as Rust's is: `insert` gives an
`EntityInsert`, and `insert_many`, `add` and `add_many` an
`EntityInsertMany`, which keeps the insert's conflict clause. The
`ActiveModel`s of one insert must set the same columns, or building it raises
`ConstructionError`. An insert of no models writes nothing, sends nothing and
answers `0` or an empty list. A key is the value of a one-column key, or a
tuple of a composite key's values in its columns' order.

A `Change` holds `old` and `new`, the row as it was and as the statement left
it, read in one statement through PostgreSQL 18's `old` and `new` in
`RETURNING`.
An upsert answers `Upserted.Inserted(model)` for a row it inserted, or
`Upserted.Updated(change)` for the row its `ON CONFLICT DO UPDATE` updated,
and `into_model()` gives the row as the statement left it either way.
`returning_upsert` answers `None`, and `returning_upserts` leaves the row
out, when the conflict clause did not write it (`DO NOTHING`, or a `DO
UPDATE` whose `WHERE` held it back); `returning_pk` and `returning_model`
raise `DecodeError` for such a row, and `returning_pks` and
`returning_models` leave it out. The conflict clause is the statement
builders' `Conflict` ([Writing rows](04-writing.md)).

`on_conflict_do_nothing()` adds `ON CONFLICT (primary key) DO NOTHING`, as
Rust's does, and turns an `EntityInsert` into an `EntityTryInsert`, or an
`EntityInsertMany` into an `EntityTryInsertMany`: Rust's `TryInsert`.
`on_empty_do_nothing()` makes one too, keeping the insert's own conflict
clause. A try-insert has `on_conflict`, which replaces its clause, `add` and
`add_many`, and Rust's `TryInsert` terminals: `execute`, `returning_pk` and
`returning_model` for one row, `execute`, `returning_pks` and
`returning_models` for a batch, and no upsert terminal. Each answers with the
`TryInsertResult` a declared model's try-insert answers with
([Writing rows](04-writing.md)): `TryInsertResult.Inserted` holding what the
terminal wrote, `TryInsertResult.Conflicted()` when the conflict clause held
every row back, or `TryInsertResult.Empty()` when there was nothing to write,
a batch of no `ActiveModel`s or one that sets no column, and nothing was
sent. An entity view's try-insert reads the rows it wrote as the view's
models:

```python registered
import asyncio
import os

from pgorm import Pool, TryInsertResult
from pgorm.rust_bakery import CakeBaker


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # Ada already bakes the Chocolate Fudge.
        ada = CakeBaker.active().set_cake_id(1).set_baker_id(1)
        again = CakeBaker.insert(ada).on_conflict_do_nothing()
        assert await again.execute(pool) == TryInsertResult.Conflicted()

        # Bo starts baking it too.
        bo = CakeBaker.active().set_cake_id(1).set_baker_id(2)
        match await CakeBaker.insert(bo).on_conflict_do_nothing().returning_model(pool):
            case TryInsertResult.Inserted(row):
                assert (row.cake_id, row.baker_id) == (1, 2)
            case other:
                raise AssertionError(f"Bo is new to it: {other!r}")

        nothing = await CakeBaker.insert_many([]).on_empty_do_nothing().returning_pks(pool)
        assert nothing == TryInsertResult.Empty()


asyncio.run(main())
```

These are statement terminals, as they are in Rust, so they run no hooks:
an update returning its change does not do what a `before_save` would.
`returning_change` with nothing set sends no `UPDATE`: it returns the row its
key reads as both versions, and raises `DecodeError` for a key that matches
no row.
`update_many()` writes only once `where(..)` or `all_rows()` has said which
rows, and raises `ConstructionError` before sending anything otherwise, or
when nothing is set. Its `set(column, value)` converts the value by the
column's declared type and writes it through the column's cast, or takes an
`Expr` as written. Rust names the two versions apart in `RETURNING`, so a
table called `old` or `new` still reports them in order.

A record, an `ActiveModel` or a column of another registration given to an
entity's method is an argument of the wrong type, and raises `TypeError`.

## Building an application wheel by hand

An application wheel is built from a Rust crate of the application's own: a
`cdylib` whose `#[pymodule]` collects the registrations into a
`pgorm_python::entities::Registry` and installs them with pgorm's whole API
into one module, `pgorm._native`:

```rust
use pyo3::prelude::*;

#[pymodule(gil_used = true)]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    let mut registry = pgorm_python::entities::Registry::default();
    registry.entity::<pgorm_guide_support::bakery::Entity>("rust_bakery.Bakery")?;
    registry.entity::<pgorm_guide_support::cake::Entity>("rust_bakery.Cake")?;
    pgorm_python::install(module, registry)
}
```

`Registry::entity` registers any entity whose types are `Send`, `Sync` and
`'static`, and whose `Model` converts into its `ActiveModel`. A name
registered twice, or a Rust entity registered under two names, is refused
there, before the entry is added. `install` fails on a module that already
has a registry.

`Registry::entity` registers `E::default()`, and every statement of the
registration names that value's table. An entity whose names are computed at
run time, such as a schema per tenant, is registered once for each value it
serves with `Registry::entity_in`, and each registration reads and writes its
own value's table, as Rust's `Entity::find_in(entity)` and the other `_in`
constructors do:

```rust
registry.entity::<note::Entity>("app.Note")?;
registry.entity_in("app.AcmeNote", note::Entity { tenant: "acme" })?;
```

Python picks the tenant by the name it asks for, `p.entity("app.AcmeNote")`,
since a Python program has no Rust value to hand over itself. A Rust type is
registered once for each table its values name, so the same type and table
under two names is the duplicate refused above. A graph or a source tuple
over the type reads the metadata of its first registration.

The crate is a library named `_native`, of crate types `cdylib` and `rlib`.
It depends on the checkout's `pgorm-python` with its default
`standalone-module` feature off, so that the module initializer is the
crate's own; on the same checkout's `pgorm`; on the entity crate; and on the
`pyo3` release `pgorm.capabilities()["binding"]["version"]` names. An
`extension-module` feature enables PyO3's. pgorm-python has a Cargo workspace
and lockfile of its own, outside the repository's, and the application crate
is a workspace of its own too:

```toml
[package]
name = "bakery-python"
version = "0.2.0"
edition = "2024"
publish = false

[workspace]

[lib]
name = "_native"
crate-type = ["cdylib", "rlib"]

[features]
extension-module = ["pyo3/extension-module"]

[dependencies]
pgorm-python = { path = "../pgorm/pgorm-python", default-features = false }
pgorm = { path = "../pgorm" }
pgorm-guide-support = { path = "../pgorm/pgorm-guide-support" }
pyo3 = "0.29.3"
```

The crate's version must be the Python package's, `pgorm.__version__`
(0.2.0 here), since the wheel takes it as the version of the `pgorm`
distribution it supplies. Copy the same checkout's `pgorm-python/python/pgorm`
directory, the package's Python half, to `python/pgorm` beside the manifest,
and build with maturin, configured as:

```toml
[build-system]
requires = ["maturin>=1.15,<2"]
build-backend = "maturin"

[project]
name = "pgorm"
dynamic = ["version"]
requires-python = ">=3.14,<3.15"

[tool.maturin]
bindings = "pyo3"
python-source = "python"
module-name = "pgorm._native"
features = ["extension-module"]
```

Build the wheel with CPython 3.14 as [Getting
started](01-getting-started.md) builds the standalone one, pointing
`--manifest-path` at this crate, and install it in place of the standalone
wheel. A module built this way gives the generic API. The typed views of
the next section are generated.

## Building an application wheel with pgorm.codegen

`pgorm.codegen` writes the registration crate for you, from a description of
what to register, and generates a Python module of typed views over the
registrations, with matching stubs. It takes two builds. The first compiles
the registrations, so that the generator can read what the Rust types
declare; the second packages the generated module with them. Repeat them
when the Rust entities, the registrations or the graph shapes change. A
program that only builds new queries over the installed types needs neither.

### Describing the application

The description is a file, `application.json`, written next to the entity
crate's `Cargo.toml`. This is the guide's, which registers the bakery's
seven entities from `pgorm-guide-support`:

```json application.json
{
  "schema_version": 1,
  "module": "rust_bakery",
  "entity_crate": ".",
  "entities": [
    {"name": "rust_bakery.Bakery", "rust": "bakery::Entity", "python": "Bakery"},
    {"name": "rust_bakery.Baker", "rust": "baker::Entity", "python": "Baker"},
    {"name": "rust_bakery.Cake", "rust": "cake::Entity", "python": "Cake"},
    {"name": "rust_bakery.CakeBaker", "rust": "cake_baker::Entity", "python": "CakeBaker"},
    {"name": "rust_bakery.Customer", "rust": "customer::Entity", "python": "Customer"},
    {"name": "rust_bakery.Order", "rust": "order::Entity", "python": "Order"},
    {"name": "rust_bakery.OrderLine", "rust": "order_line::Entity", "python": "OrderLine"}
  ]
}
```

- `schema_version` is `1`, the only version there is.
- `module` names the generated module, `pgorm.rust_bakery` here. It defaults
  to `app`.
- `entity_crate` is the entity crate's directory, relative to the file: `.`
  for a file beside the crate's `Cargo.toml`.
- `entities` lists the entities to register, each with its registration
  `name`, its `rust` path within the entity crate, and the `python` name the
  module exports it under. An entry may add `fields`, a map from a column's
  SQL name to the attribute its typed model reads it as, such as
  `{"display name": "display_name"}`.
- `graphs` and `disjoints` list graph shapes and disjoint enums the same way,
  as the sections on them show.

The description is checked before anything is written. A Rust path names
items of the entity crate (`bakery::Entity`), with no generic arguments, no
expressions and no `crate::` or `self::`. Each Rust entity is registered
once. Registration names, and the Python names the module exports, must be
unique, and an export must be a public Python identifier that collides with
nothing the module imports, no builtin and no method of a model; a column
whose name would make such an attribute needs a `fields` alias. A
description that fails a check raises `CodegenError`, a `ValueError`.

Register an entity crate's ordinary Rust definitions: the generated crate
owns the application's one `#[pymodule]`, and its module is the
application's whole `pgorm`.

### Scaffold, probe build, emit, final build

Start in an environment with the standalone wheel of the same pgorm checkout
installed, maturin 1.15 or later, and CPython 3.14. From the checkout's root,
with `application.json` written beside the guide support crate's
`Cargo.toml`:

```sh
# 1. Write the registration crate, the Python package and the build metadata.
.venv/bin/python -m pgorm.codegen scaffold pgorm-guide-support/application.json \
  --pgorm-source . --output rust-bakery

# 2. The probe build: compile the registrations and install them, starting
#    from the dependency versions pgorm-python's own build is locked to.
cp pgorm-python/Cargo.lock rust-bakery/Cargo.lock
.venv/bin/maturin build --manifest-path rust-bakery/Cargo.toml \
  --interpreter .venv/bin/python --out rust-bakery-probe
uv venv --python 3.14 rust-bakery-probe-env
uv pip install --no-index --python rust-bakery-probe-env/bin/python rust-bakery-probe/pgorm-*.whl

# 3. Emit the typed module from what the installed registrations describe.
rust-bakery-probe-env/bin/python -I -m pgorm.codegen emit rust-bakery

# 4. The final build: the registrations and the typed module, in one wheel.
.venv/bin/maturin build --manifest-path rust-bakery/Cargo.toml \
  --interpreter .venv/bin/python --out rust-bakery-dist --locked
uv venv --python 3.14 rust-bakery-env
uv pip install --no-index --python rust-bakery-env/bin/python rust-bakery-dist/pgorm-*.whl
```

`scaffold` writes a new project: its `Cargo.toml` and `pyproject.toml`, a
`src/lib.rs` registering exactly what the description lists, a copy of the
installed `pgorm` package's Python half, and the validated description with
the versions it was made for. It runs no build and touches no database. It
refuses a destination that exists, and refuses to run when the checkout's
pgorm and pgorm-python versions are not the installed wheel's. Run from an
application wheel, it leaves that wheel's generated module out of the copy.

`emit` reads the registrations of the wheel installed in the environment that
runs it, so it runs in the probe build's environment, and refuses a wheel
that is not the scaffold's build. It writes `python/pgorm/rust_bakery.py`, its
stub `rust_bakery.pyi` and a marker naming the module, and the same build
gives the same bytes each time. The module and its stub are laid out as
black lays out Python, importing only what they use, so they read as
written by hand and need no formatter run over them.

The probe build completes `Cargo.lock` with what the application adds, the
entity crate and its own dependencies. Keep it, and build with `--locked`
from then on, so that a rebuild resolves the same dependencies. The project's
paths to the checkout and the entity crate are build inputs; the installed
module reads none of them.

The two commands are also functions, for a build script:
`pgorm.codegen.scaffold(config, destination, *, pgorm_source, base=".")`
takes the description as data, with `base` the directory its `entity_crate`
is relative to, and returns the project's path, and
`pgorm.codegen.emit(project)` returns the generated module's path. Paths are
strings or `os.PathLike[str]`.

## The generated typed views

The generated module exports three names per entity: for `Cake`, the model
`CakeModel`, the active model `CakeActive`, and the entity view `Cake`. They
wrap the generic API's objects and add types: a model's fields are
read-only properties of their Python types, and an active model has a
`set_<field>` method per field. Each method returns a new view, and the SQL,
the hooks and the outcomes are the registered entity's:

```python registered
import asyncio
import os
from decimal import Decimal

from pgorm import Pool, Value
from pgorm.rust_bakery import Cake, CakeActive, CakeModel


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        query = Cake.find().where(Cake.col("bakery_id").eq(2)).order_by(Cake.col("price").asc())
        moonlight = await query.all(pool)
        assert [(cake.name, cake.price) for cake in moonlight] == [
            ("Vanilla Sponge", Decimal("2.90")),
            ("Carrot Cake", Decimal("3.80")),
        ]
        assert isinstance(moonlight[0], CakeModel)
        # A property and a lookup by SQL name read the same value.
        assert moonlight[0]["name"] == moonlight[0].name

        draft: CakeActive = (
            Cake.active()
            .set_name("Lemon Scone")
            .set_flavor("lemon")
            .set_price(Decimal("2.50"))
            .set_gluten_free(False)
            .set_bakery_id(1)
        )
        bakery_id = draft.get("bakery_id").value
        assert isinstance(bakery_id, Value) and bakery_id.kind == "i32"

        scone = await draft.insert(pool)
        assert (scone.id, scone.flavor, scone.price) == (6, "lemon", Decimal("2.50"))
        renamed = await scone.into_active().set_name("Lemon Bun").update(pool)
        assert (renamed.name, scone.name) == ("Lemon Bun", "Lemon Scone")


asyncio.run(main())
```

A property is named for its column's SQL name, or, where that is not a
Python identifier, for the column's JSON key; a `fields` alias overrides
both. A model is still a mapping by SQL name, so `model["display name"]` and
`model.display_name` read the same value. `.native` gives the wrapped
`EntityModel` or `ActiveModel`, and `.tagged(column)` the `Value` with its
tag. The generic `set` and `with_value` remain, with the generic API's
conversion rules.

`set_<field>` tags its value with the kind of the Rust field's own type
before calling the Rust setter, so `set_bakery_id(1)` binds the `i32` the
field is, not the `i64` a bare `Value(1)` would infer. The generic `set`
takes its hint from the column's SQL type instead, which says the same for
the bakery's columns; the two differ where a column's SQL type does not name
its Rust field's. A `Value` passed to a `set_<field>` goes to the setter as
it is. A view refuses a native object of another registration with
`TypeError`.

A field's Python type comes from its Rust type, as the Rust derives report it:

| Rust field | Python type |
| --- | --- |
| `bool` | `bool` |
| `i8`, `i16`, `i32`, `i64`, `u32`, `u64` | `int`, bound at the field's width |
| `f32`, `f64` | `float` |
| `String`, `char` | `str` |
| `Decimal` | `Decimal` |
| `Uuid` | `UUID` |
| `Date`, `Time`, `DateTime` | `date`, `time`, `datetime` |
| `DateTimeWithTimeZone`, `Timestamp` | `datetime`, bound as a UTC timestamp |
| `Interval` | `pgorm.Interval`, whatever the column's field qualifier |
| `IpNetwork` | `str` |
| `MacAddress`, `Vec<u8>` | `bytes` |
| `Vector` | `list[float]` |
| `Vec<T>` | `list[T]`, one dimension |
| `Option<T>` | `T \| None` |
| an `ActiveEnum` | `str`, its label, bound under the enum's qualified type |
| JSON | `Any` |
| anything else | `Any` |

A column whose type is a domain types and binds as the domain's base does: a
domain over an integer is an `int`, one over an enum the enum's `str`, one
over an array a `list`. A field of a custom Rust type, or one whose decoder is
written by hand, is `Any`, and takes an explicit `Value` where the column's
SQL type is no guide.

The entity view's writes have the generic API's terminals, typed:
`returning_change` gives a `pgorm.Change` of the typed models,
`returning_upsert` a `pgorm.Upserted.Inserted` or `Upserted.Updated` holding
them, the classes every write answers with, and `returning_models` a list of
the typed models:

```python registered
import asyncio
import os

from pgorm import Conflict, Pool, Upserted
from pgorm.rust_bakery import Customer


async def main() -> None:
    async with Pool(os.environ["DATABASE_URL"]) as pool:
        # Kim's email is taken, so the insert updates her name instead.
        kim = Customer.active().set_name("Kimberly").set_email("kim@example.com").set_notes(None)
        renamed = Conflict.on("email").update("name")
        match await Customer.insert(kim).on_conflict(renamed).returning_upsert(pool):
            case Upserted.Updated(change):
                assert (change.old.name, change.new.name) == ("Kim", "Kimberly")
            case other:
                raise AssertionError(f"expected an update, got {other!r}")

        sam = Customer.active().set_name("Sam").set_email("sam@example.com").set_notes(None)
        match await Customer.insert(sam).on_conflict(renamed).returning_upsert(pool):
            case Upserted.Inserted(model):
                # The insert that updated Kim drew id 3 from the sequence too.
                assert (model.id, model.name) == (4, "Sam")
            case other:
                raise AssertionError(f"expected an insert, got {other!r}")


asyncio.run(main())
```

## Schema from a registered entity

`pgorm.schema.from_entity(entity)` writes a registered entity's DDL as Rust's
`Schema` does ([Schema generation](14-schema.md)): an `EntitySchema` whose
`table` is the `CREATE TABLE`, `enums` the `CREATE TYPE`s of the enums it
uses, `indexes` the indexes its columns declare, and `comments` the
comments on the table and its columns. Nothing is sent; execute the
statements, in that order, where they are wanted:

```python registered
import pgorm
from pgorm import schema

generated = schema.from_entity(pgorm.entity("rust_bakery.Cake"))

assert generated.table.inspect().sql == (
    'CREATE TABLE "cake" ( "id" serial NOT NULL, "name" varchar NOT NULL, '
    '"flavor" flavor NOT NULL, "price" decimal(10, 2) NOT NULL, "gluten_free" bool NOT NULL, '
    '"bakery_id" integer NOT NULL, PRIMARY KEY ("id"), '
    'CONSTRAINT "fk-cake-bakery_id" FOREIGN KEY ("bakery_id") REFERENCES "bakery" ("id") )'
)
assert [create.inspect().sql for create in generated.enums] == [
    "CREATE TYPE \"flavor\" AS ENUM ('chocolate', 'vanilla', 'lemon', 'carrot')"
]
assert generated.indexes == []
assert generated.comments == []
```

A belongs-to relation over a column typed as a domain over an enum, which
PostgreSQL cannot key, is refused with `ConstructionError` naming it, as Rust
refuses it with `Error::Type`.

## Registered graphs and keyset cursors

A graph ([Relations](07-relations.md)) is a `SelectGraph<E, S>`: a root
entity and a tuple of slots, each `Req<F>`, an inner join that always holds
a model, or `Opt<F>`, a left join that may hold none. Rust writes the whole
projection, each source's columns under its own prefix, and decodes each
row's models. A registered graph keeps all of that: Python chooses the
aliases, the filters, the ordering and the cursor, but not the entities or
the slots, which are the Rust type's.

### Registering a graph

A graph is registered with a *factory*, a function that builds it from the
aliases Python chooses. It receives one alias per joined slot, in the
tuple's order, each already checked, and must pass each to the matching
`join_one_as` or `join_maybe_as`; the root keeps its table's name. Here is
each bakery with each of its cakes, or with none:

```rust
use pgorm::pgorm_query::Name;
use pgorm::{EntityTrait, Opt, RelationTrait, SelectGraph};
use pgorm_guide_support::{bakery, cake};

/// Each bakery with each of its cakes, or with none.
pub fn bakery_cakes(aliases: &[String]) -> SelectGraph<bakery::Entity, (Opt<cake::Entity>,)> {
    bakery::Entity::graph()
        .join_maybe_as::<cake::Entity>(bakery::Relation::Cake.def(), Name::runtime(&aliases[0]))
}
```

It registers after every entity it decodes:

```rust
registry.entity::<bakery::Entity>("rust_bakery.Bakery")?;
registry.entity::<cake::Entity>("rust_bakery.Cake")?;
registry.graph("rust_bakery.BakeryCakes", bakery_cakes)?;
```

The relation the factory joins by is the application's, its `via` hops and
its conditions included. A relation matching on a `PERIOD`, PostgreSQL 18's
temporal foreign key, joins by equality on its key columns and by overlap
(`&&`) on the two periods, so a row reaches every version of its target
whose period overlaps its own, one row per version. A relation the server
does not enforce may name a row that does not exist: an `Opt` slot then
holds `None`, and a `Req` slot drops the root row, as for any join that
matches nothing.

A graph has no slots or one to six of them, so one to seven sources. A
graph whose entities are not registered, and a name registered twice, are
refused as they are registered. `pgorm.graphs()` lists the registered names,
and `pgorm.graph(name)` gives one, raising `UnsupportedCapabilityError` for
a name the module does not register. Its `describe()` reports its `name`, its
`rust_shape`, the Rust `SelectGraph` type, and its `sources`, the root and
then each slot, each with its `index`, the `entity` it decodes as and its
`slot`: `root`, `Req` or `Opt`.

The guide's application wheel registers no graph, so what follows describes
`rust_bakery.BakeryCakes` as a wheel that registered it would run it.

### Reading a graph

With `cakes = pgorm.graph("rust_bakery.BakeryCakes")`, `cakes.find(aliases=["c"])`
starts a query whose one joined slot is aliased `c`; without `aliases`, the
joined slots are `g1`, `g2` and so on, changed where one would equal the
root's table. `query.col(source, column)` names a
column of one source, source `0` being the root, checking both, and builds
an expression qualified by that source's alias, so build expressions from
the query whose aliases they name. Its comparisons follow the plain
expression rules; use an explicit `Value` or a cast where a value's type
matters. `where` and `order_by` return new queries. To change the aliases,
call `find` again.

`query.all(db)` returns every row: a joined graph's row is a tuple of its
sources' models in order, `(bakery, cake)` here, with `None` for an `Opt`
slot that matched nothing; a graph with no slots returns bare models.
`query.optional(db)` adds `LIMIT 1` and returns the first row or `None`. An
absent `Opt` source is not the same as no row, and a source present in the
row whose model cannot be decoded raises `DecodeError` rather than reading
as absent. `inspect()` gives the SQL and values `all` sends, and
`inspect(terminal="optional")` adds the limit. Graph reads run on a pool, a
connection or a transaction, as an entity's do.

### Keyset cursors

`query.cursor("id")` is Rust's `SelectGraph::cursor_by` on one column of the
root, and it pages through the graph by key ([Pages, cursors and
streams](06-pagination.md)). Rust completes the order with each source's
primary key, the root's and then every slot's, so the key of a row is the
order column's value, the rest of the root's key, and each joined source's
key in order. `after(value)` and `before(value)` bound the order column
alone; `after_with(*values)` and `before_with(*values)` bound the whole key,
in that order, converted by the registered columns' types, enum casts
included. A boundary of the wrong length or type is refused before anything
is sent. `first(n)` and `last(n)` set the window, replacing an earlier one,
`asc()` and `desc()` the direction, and `all(db)` runs it. Each method
returns a new cursor, and the cursor's order replaces the query's, as in
Rust. The rows of a `last` page come back in the cursor's order, and a window
of zero rows returns none.

An `Opt` slot that matched nothing has a NULL key, and PostgreSQL's
comparisons with NULL select nothing, so a boundary inside such a row's key
continues nowhere: bound the order column alone to step past it. A boundary
on the order column alone also skips the remaining joined rows of the same
root, so choose the boundary that matches where the page ended.

`cursor(*columns)` orders by up to six root columns, and
`cursor_on(source, *columns)` is Rust's `cursor_by_on`, ordering by up to six
columns of slot `source`, the first slot being `1`: the whole key is then the
order columns, the rest of that slot's key, the root's, and the other slots'
in order. `after` and `before` take one value per order column, and
`after_with` and `before_with` those or the whole key. `inspect()` shows a
cursor's statement. A graph has no more than seven sources because Rust's
`SelectGraph` holds no more than six slots.

`query.stream(db)` reads the rows one at a time, each decoded by `GraphRow`
as Rust's `SelectGraph::stream` decodes it, from a pool's connection of its
own or a connection. `query.all_grouped(db)` is `all_grouped`, which Rust
has only on a graph of one `Opt` slot: each root once with a list of its
slot's models, an empty one where none matched; any other shape raises
`ConstructionError`. `count(db)` and `paginate(page_size)` count and page the
graph's rows as an entity's query does, a page boundary falling between rows
of the join.

### Graphs in pgorm.codegen

A graph factory the entity crate exports is described under `graphs`, by its
registration name, the Rust path of the factory and the Python name:

```json
"graphs": [
  {"name": "rust_bakery.BakeryCakes", "rust": "graphs::bakery_cakes", "python": "BakeryCakes"}
]
```

The generated module exports a view, `BakeryCakes`, and a row type,
`BakeryCakesRow`, here `tuple[BakeryModel, CakeModel | None]`: a `Req` slot
is its typed model, an `Opt` slot its model or `None`, and a graph with no
slots returns its root's model alone. The view's queries and cursors are the
registered graph's, and wrap the decoded models without building any SQL of
their own. Emission refuses a graph whose sources the description does not
export.

## Disjoint enums, one-of references and source tuples

### Disjoint enums

An application's `#[derive(Disjoint)]` enum ([Disjoint enums](15-disjoint.md))
registers after its base's entity and each kind's:

```rust
registry.entity::<vehicle::Entity>("app.Vehicle")?;
registry.entity::<van::Entity>("app.Van")?;
registry.disjoint::<Vehicle>("app.Vehicles")?;
```

`pgorm.disjoint_entity(name)` gives the registration, a `DisjointEntity`, and
`pgorm.disjoint_entities()` lists the names. It reads and writes as a
disjoint model does: `find()`, `find_by_key(key)`, the kind filters, pages
and cursors. A row is a tuple of the base's model, the kind, and the kind's
model or `None`, each the Rust model it was read as. A write takes the
entities' `ActiveModel`s: `vehicles.insert(base, "van", van)` with `base`
from `pgorm.entity("app.Vehicle").active()` and `van` from the van entity's,
and `returning_model(db)` sends it and reads the row back. Setting the kind
column, or a kind row's key, is refused with `ConstructionError` before
anything is sent, and an `ActiveModel` of another entity, given for the base
or a kind, is a `TypeError`. The statements are the ones Rust's
`Vehicle::find()`, `Vehicle::insert` and `Schema::create_disjoint` build,
from the same `DisjointShape`.

An enum declared `#[pgorm(base = .., unkinded)]` registers the same way and
is read-only, as its Rust declaration is: its kinds are its variants' names
in snake_case, its `describe()` names no kind field or kind type, and every
write and `create_disjoint` is refused with `ConstructionError`.

In `pgorm.codegen`, an enum is described under `disjoints`, its base and kind
entities among the described entities:

```json
"disjoints": [{"name": "app.Vehicles", "rust": "vehicle::Vehicle", "python": "Vehicles"}]
```

The module exports its view and a `VehiclesRow`, a union over the kinds,
`tuple[VehicleModel, Literal['van'], VanModel] | tuple[VehicleModel,
Literal['bike'], None]`, each record the entity's typed model. A kinded
enum's view is a `WritableDisjointView`, whose `insert`, `update` and
`change_kind` take the typed active models and a kind of the enum, and which
`pgorm.schema.create_disjoint` takes; an unkinded enum's is a read-only
`DisjointView`. Emission refuses an enum whose base or kind entity is not
exported, and the import check compares each enum's `describe()` as it
compares an entity's.

### One-of references

A `#[derive(OneOf)]` enum ([One-of references](16-one-of.md)) registers
after its entity and every target's, with `registry.one_of::<Labelled>("app.Labelled")?`.
`pgorm.one_of_entity(name)` gives it and `pgorm.one_of_entities()` lists the
names. Its targets are named as its variants are, in snake_case, and its
records are the entities' own models; `set(active, value, key)` returns the
`ActiveModel` with the reference set to that target's key and every other
target's column NULL, and `set(active, None)` sets them all NULL where the
reference is optional. `load(db, records)` reads the record each reference
names, as a one-of model's does. `pgorm.codegen` does not describe one-of
enums yet: a wheel that registers one is built by hand.

### Pipeline source tuples

The [pipeline](12-pipeline.md) selects whole entities from a join through a
registered *source tuple*, the entities its rows decode as, in order:

```rust
registry.sources::<(bakery::Entity, cake::Entity)>("rust_bakery.BakeryAndCake")?;
```

`pgorm.pipeline.source_tuples()` lists the registered names, and
`pgorm.pipeline.sources(name)` gives one, a `SourceSelection`. A pipeline's
`select_sources(selection, qualifiers=[...])` projects each source's columns
from the source its qualifier names, and its `all`, `one` and `optional` give
tuples of the registered models, with `None` for a source a right or full
join left absent. `(None,)` is a row whose one source is absent, not the
absence of a row. A tuple holds one to six entities, each registered before
it. `pgorm.codegen` does not describe source tuples yet: a wheel that
registers one is built by hand.

## Checks at import

The generated module records, as it is written, what it was generated for:
the package's and pgorm's versions, the registry's ABI version, the features
pgorm was built with, and what each entity, graph and disjoint enum it wraps
describes. As it is imported, it compares that with the native module loaded
in the process, and raises `UnsupportedCapabilityError` on any difference,
before a query is built. So a typed module copied beside another build of
the registrations does not import.

The check covers what the registrations describe, not the code inside them:
a change to a hook's body describes the same. Rebuild and redeploy the
application wheel whenever its Rust code changes.
