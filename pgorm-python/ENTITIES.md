# Compiled application entities

`pgorm.entity(name)` exposes a concrete Rust entity compiled into the installed
native module. Queries own a real `pgorm::Select<E>`; records own `E::Model`;
writes call `E::ActiveModel` and its `ActiveModelBehavior` hooks. All calls run
inside Python's process, using PostgreSQL's native protocol.

`pgorm.entities()`, `pgorm.graphs()` and `pgorm.pipeline.source_tuples()` list
the names the module registers, in order. The standalone wheel has an empty
registry, so each lists none. Runtime `Table`, SELECT and CRUD
builders work without registrations. Applications that already have Rust
entities can build a wheel containing those types and the common Python API.
Python cannot create Rust generic instantiations at runtime.

## Build an application wheel

Use one native module containing both the common bindings and the application
registrations. Disable the binding crate's default `standalone-module` feature
and provide the following initializer in the application's Rust crate:

```rust
use pyo3::prelude::*;

#[pymodule(gil_used = true)]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    let mut registry = pgorm_python::entities::Registry::default();
    registry.entity::<my_entities::account::Entity>("app.Account")?;
    pgorm_python::install(module, registry)
}
```

The library target is named `_native` with `crate-type = ["cdylib", "rlib"]`.
Depend on this checkout's `pgorm-python` with `default-features = false`, the
same checkout's `pgorm`, your entity crate, and the `pyo3` release
`pgorm.capabilities()["binding"]["version"]` names. Define an
`extension-module` feature enabling `pyo3/extension-module`. Registration
requires `E: EntityTrait + Send + Sync + 'static`,
`E::Model: IntoActiveModel<E::ActiveModel> + Sync + 'static`, and
`E::ActiveModel: Send + Sync + 'static`. Duplicate names and duplicate Rust
entity types fail before the entry is added.

Copy the matching `pgorm-python/python/pgorm` facade into the application's
`python/pgorm` directory. Its Maturin configuration is:

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

The application crate version must match the common facade's package version
(currently 0.2.0). Build with Maturin and CPython 3.14, then install that wheel
in the application's environment. It supplies the `pgorm` distribution and
replaces the standalone wheel there. Separate extensions containing duplicate
copies of the binding classes are not a supported registration mechanism.

The independent [application crate](tests/application-binding) and
[build checker](checks/entities.py) are executable examples. The checker
materializes the facade next to the downstream Rust crate, runs Rust parity
tests, builds a wheel, and installs it into a fresh environment:

```sh
target/python-dev/bin/python pgorm-python/tests/with_postgres.py \
  target/python-dev/bin/python pgorm-python/checks/entities.py
```

An existing disposable server can instead be supplied with `PGORM_TEST_DSN`.
The checker uses its own `python_entities` schema; the database account needs
schema creation privileges. The output wheel and success report are written
under `target/python-entities`.

## Query and write

Using the example registration:

```python
import pgorm as p

Account = p.entity("app.Account")
query = Account.find().where(Account.col("id") >= 10)
query = query.order_by(Account.col("id").desc()).limit(20)
print(query.inspect().sql)

async with pool.connection() as connection:
    accounts = await query.all(connection)
    first = await query.optional(connection)
    active = Account.active().set("id", 42).set("display name", "Nora")
    inserted = await active.insert(connection)
    updated = await inserted.into_active().set("note", "Reviewed").update(connection)
    affected = await updated.into_active().delete(connection)
```

A column's `eq`, `ne`, `gt`, `gte`, `lt` and `lte` (and the operators) are
its `ColumnTrait` comparisons: the value converts to the column's declared
kind and is written through its `save_as`, so an enum's label is cast to its
type. `is_in` and `is_not_in` are the trait's membership: `is_in` /
`is_not_in` over a list or tuple of values, each converted the same way, and
`in_subquery` / `not_in_subquery` over a `Select`'s rows, so a column whose
domain is over an enum is a member as the enum — the only way PostgreSQL
compares one — and the subquery's projection is cast beside it:

```python
Temper = p.entity("app.Temper")
settled = p.Select(p.col("settles_into")).from_(p.Table("tempers", schema="python_entities"))
query = Temper.find().where(Temper.col("mood").is_in(settled))
```

The column is itself an `Expr`: every other expression method reads it as
written, and it goes wherever an expression goes. An expression operand, a
list of expressions or `any_(..)` included, is compared as written.
`col(name).expr()` gives its plain expression, whose comparisons bind as an
`Expr`'s do rather than through the column's `ColumnTrait`.

Every entity terminal, ActiveModel write and version terminal runs on a
`Pool`, an acquired `Connection` or a `Transaction`. On a pool it runs on a
connection the pool lends it alone, returned once the operation finishes, as a
pool's statement does. They return native asyncio awaitables; calling a
terminal schedules its operation. Await each operation before reusing a
connection. An outstanding operation can be cancelled through its future or
`asyncio.wait_for`; cancellation discards the connection if the operation's
completion is uncertain. Closing its connection or pool also cancels an
outstanding Rust hook. Cancellation does not guarantee rollback of a submitted
write.

`all` returns a list of detached models. `one` and `optional` are Rust's `one`
and `one_opt`, which narrow the query's limit to one row: `LIMIT 1`, including
when an earlier query limit was larger, and `LIMIT 0` when it was zero, which
finds no row. `one` raises `DecodeError` on absence; `optional` returns
`None`. `inspect(terminal="one")` or `inspect(terminal="optional")` includes
that terminal's limit. A decoding failure remains `DecodeError`. These are the
registered Rust selector's semantics; the `one` of a pool, connection or
transaction has its own strict cardinality contract. Entity streams and
arbitrary partial entity projections are not advertised by this registration
API.

## Writes that return both versions

PostgreSQL 18's `RETURNING` reads a written row as it was and as the write
left it. Four entity terminals return both, through the Rust terminals that
do:

```python
Account = p.entity("app.Account")
change = await Account.update(model.into_active().set("note", "x")).returning_change(connection)
print(change.old["note"], change.new["note"])
changes = await (Account.update_many().set("note", "bulk")
                 .where(Account.col("id") >= 2).returning_changes(connection))
renamed = p.Conflict.on("id").update("display name")
match await Account.insert(active).on_conflict(renamed).returning_upsert(connection):
    case p.Upserted.Inserted(model): ...
    case p.Upserted.Updated(change): ...
    case None: ...  # the conflict clause wrote nothing
rows = await Account.insert_many([a, b]).on_conflict(renamed).returning_upserts(connection)
```

| Python | Rust |
| --- | --- |
| `entity.update(active).returning_change(connection)` | `Update::one(active)?.exec_returning_change` |
| `entity.update_many().set(..).where(..).returning_changes(connection)` | `Update::many(entity).col_expr(..).filter(..).exec_returning_changes` |
| `entity.update_many().set(..).all_rows().returning_changes(connection)` | `Update::many(entity).col_expr(..).exec_returning_changes` |
| `entity.insert(active).on_conflict(..).returning_upsert(connection)` | `Insert::one(active).on_conflict(..).exec_returning_upsert` |
| `entity.insert_many(actives).on_conflict(..).returning_upserts(connection)` | `Insert::many(actives).on_conflict(..).exec_returning_upserts` |

A `Change` holds `old` and `new`, each an `EntityModel`. `Upserted` is
`Upserted.Inserted(model)` for a row the insert wrote or `Upserted.Updated(change)`
for the row its `ON CONFLICT DO UPDATE` updated; `into_model()` is the row as
the statement left it either way. `returning_upsert` answers `None`, and
`returning_upserts` leaves out the row, when the conflict clause did not write
it (`DO NOTHING`, or a `DO UPDATE` whose `WHERE` held it back). The conflict
action is the statement builders' own `Conflict` or `ConflictUpdate`.

These are statement terminals, as they are in Rust, so no `ActiveModelBehavior`
hook runs around them: an `update` returning its change does not bump what a
`before_save` would. With nothing set, `returning_change` sends nothing and
returns the row its key reads as both versions; a key matching no row raises
`DecodeError`, and `returning_changes` with nothing set `ConstructionError`
(Rust's `NothingToSet`). An `update_many()` runs only once `where(..)` or
`all_rows()` has said which rows it writes, as a statement's UPDATE does; with
neither, `returning_changes` raises `ConstructionError` before anything is sent.
`update_many().set(column, value)` converts a value by the
column's declared type and writes it through the column's `save_as`, or takes
an `Expr` as written. An insert of models that set different columns is
refused, and a batch of none writes and answers nothing. Rust renames the two
versions in `RETURNING`, so an entity whose table is called `old` or `new`
still reports them in order. An ActiveModel of another registration raises
`LifecycleError`. A generated module's typed views return `Change`,
`Inserted` and `Updated` from `pgorm._registered.entities` holding its typed
models. Like an entity query, each of these writes and the typed view over it
raises `ConstructionError` on Python truth testing.

## Values, state and hooks

Column lookup and model keys use SQL names. `describe()` reports the
registration's `name`, `schema` and `table`; under `columns`, each column's
SQL `name`, its `json_key`, `sql_type`, the `rust_type` its model reads it as,
`nullable`, `primary_key`, the `kind` its values bind and read as (`i32`,
`text[]`, `enum "app"."mood"`, `range "app"."floatrange" of f64`, or `None`
for a type that takes an explicit `Value`) and an enum's `values`; and under
`rust` the concrete Rust `entity`, `model`, `active_model` and `column` type
names. These are the keys JavaScript's `describe()` reports, each spelled in
snake_case. Plain Python inputs use the declared SQL column type as a
conversion hint. Explicit `Value` tags preserve the caller's chosen Rust value variant;
the real Rust model/ActiveModel setter performs the final conversion checks.
An unusual custom field type may therefore require an explicit tagged value.
Qualified enum input must match the declared column enum identity. A field of a
`CreatedRange` newtype, a range type a schema created, hints its
`CreatedRange` or `CreatedMultirange` kind: a plain `pgorm.Range` or
`pgorm.Multirange` converts with the subtype's limits, a tagged value must name
the same type, and the field reads back as the range, tagged with the kind. The
Rust newtype writes it as its text cast to the type and reads a multirange
through its text, so both read through `find`.

`describe()` also reports `primary_key_without_overlaps`, true when the key's
last column is a period declared `without_overlaps` (`PRIMARY KEY (id, valid_at
WITHOUT OVERLAPS)`): the server holds a row per period, none overlapping, yet a
lookup, an update and a delete by the key still compare the period for
equality, so each names exactly one version. A filter on the whole key reads
one version back; which version covers a moment is a filter on the period, not
a key lookup. `relations` lists every variant of the entity's `Relation` enum
with its `type`, `from` and `to` tables, and what a join by it reads: the
`columns` paired for equality, the `period` pair a temporal foreign key matches
for overlap, `on_condition`, true where a Rust closure adds its own condition
to the join (the closure itself is Rust code and is not reported), and
`condition_type`, `"all"` where the join needs the columns and that condition
both to match and `"any"` where either suffices. Its `enforcement`
(`"not_enforced"` for a key the server never checks) and `deferrability`
(`"deferrable_initially_deferred"` for one checked at commit), each `None`
where the relation does not say, decide what the server holds the key's rows
to; a join by the relation reads the same either way.

`dict(model)` returns Python values, and `model.tagged("id")` retains the Rust
value tag. `model.with_value(column, value)` clones and calls `ModelTrait::set`;
it changes the detached model and performs no database write. `into_active()`
calls the real Rust conversion, usually producing `Unchanged` fields. To write
a changed field, call `set` on that ActiveModel.

`ActiveValue.state` distinguishes `NotSet`, `Set` and `Unchanged`.
`ActiveValue.value` is absent only for `NotSet`; a SQL NULL is an explicit
tagged value. `set`, `not_set` and `reset` return independent ActiveModel
wrappers. `Account.active()` calls the application's `ActiveModelBehavior::new`
and preserves its defaults. Each write operates on a clone, so the Python
wrapper is reusable; it is not a mutable object that marks itself saved.

Insert, update and delete run the actual before/after hooks. A hook's
refusal, the `pgorm::Error::Custom` it returns, raises `ConstructionError`
carrying its message; any other error a hook returns keeps its own class, a
`DatabaseError` from the server among them. A before-hook failure prevents the
subsequent write. An after-hook failure is reported even though the write may
already have committed. The binding adds no implicit
transaction or retries around these methods.

Registered models decode with their actual `Model::from_query_result` and then
use checked Rust-value-to-Python conversion. Their custom Rust decoders retain
their own behavior. The additional PostgreSQL wire exactness policies for
dynamic `Record` results are scoped to that result form; they do not replace an
application's typed decoder. `capabilities()` records this distinction and
lists each registration as its `describe()` reports it.

## Disjoint enums

An application's `#[derive(Disjoint)]` enum registers beside its entities, its
base and each kind's entity registered first:

```rust
registry.entity::<shipment::Entity>("app.Shipment")?;
registry.entity::<parcel::Entity>("app.Parcel")?;
registry.disjoint::<shipment::Shipment>("app.Shipments")?;
```

`pgorm.disjoint_entity(name)` gives the registration and
`pgorm.disjoint_entities()` lists them. Its reads are a disjoint model's
(`MODELS.md`), each row the base entity's record, the kind and the kind
entity's record or None, every record the Rust model it was read as; its
writes take the entities' ActiveModels:

```python
shipments = p.disjoint_entity("app.Shipments")
active = p.entity("app.Shipment").active().set("label", "box")
base, kind, parcel = await shipments.insert(
    active, "parcel", p.entity("app.Parcel").active().set("weight_g", 500)
).returning_model(pool)
```

An ActiveModel of another registration, the kind column set and a kind
row's key set are refused before anything is sent. The registration's
statements are `DisjointShape::of` the enum's, which `Vehicle::find()`,
`Vehicle::insert` and `Schema::create_disjoint` build theirs from.

An enum declared `#[pgorm(base = .., unkinded)]` registers the same way and is
read-only, as its Rust declaration is: its kinds are its variants' names in
snake_case, its `describe()` has no kind field or kind type, and each write and
`create_disjoint` is refused with a `ConstructionError`.

