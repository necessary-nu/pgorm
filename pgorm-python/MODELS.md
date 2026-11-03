# Runtime model descriptors

`Model` and `Column` declare application metadata entirely in Python. They use
the installed native statement builders and PostgreSQL Record decoder. Creating
a descriptor or constructing another query requires no compilation, HTTP
adapter or database access. Declarations do not run DDL. They do not instantiate
Rust derives, `EntityTrait` implementations or ActiveModel hooks; those are
available through [compiled entity registrations](ENTITIES.md).

```python
from pgorm import Column, Model, TypeName

Accounts = Model("accounts", {
    "id": Column("i32", primary_key=True, generated="by_default"),
    "display_name": Column("text", name="display name", default=True),
    "note": Column("text", nullable=True),
    "mood": Column(TypeName("Mood", schema="application"), values=["calm", "busy"]),
    "tags": Column("text", array=True, nullable=True),
}, schema="application")
Posts = Model("posts", {
    "id": Column("i64", primary_key=True),
    "account_id": Column("i32"),
    "title": Column("text"),
}, schema="application")
```

Mapping keys are the Python record identities. `Column.name` is the actual SQL
column name and defaults to its mapping key. Table, schema, physical column and
output names use native identifier validation and Rust quoting. Duplicate
physical columns, duplicate selected field identities and unknown fields fail
before execution. Descriptors copy their input mapping and remain immutable.

Beside its kind and `name`, a column declares whether it is `nullable`, an
`array`, part of the `primary_key`, filled by a `default` when an insert
leaves it out, or `generated` by the server — `"always"`, which no write sets,
or `"by_default"`, an identity an insert may leave out — and an enum column
its `values`, the labels every value it reads or writes is one of. A
non-boolean flag, a `generated` or `values` of the wrong type and an unknown
`generated` are a `TypeError`; an empty or repeated label list, a nullable key,
a generated column with a default and labels on a column that is no enum a
`ConstructionError`. An unknown option is Python's own `TypeError`.

A `Column` is a declaration and has no methods: a model converts each value
through its field's column where it takes one, and `Value(data, kind)` is the
conversion a program calls. `model.describe()` reports the declaration as data:
the table's `schema`, `table` and `alias`, the `primary_key` fields, and under
`fields` each field's SQL `column`, its `kind` (`i64`, `text[]`,
`enum "app"."mood"`, `range "app"."floatrange" of f64`), `nullable`,
`primary_key`, `default`, `generated` and an enum's `values`: the keys
JavaScript's `describe()` reports, spelled in snake_case. Neither a `Column` nor
`col(field)` describes itself.

## Query and write

```python
async def example(connection):
    created = await Accounts.insert({
        "display_name": "Nora",
        "note": None,
        "mood": "calm",
    }).returning().one(connection)

    key = Accounts.key({"id": created["id"]})
    changed = await (
        Accounts.update({"display_name": "Updated"})
        .where(key).returning_change(connection)
    )
    rows = await (
        Accounts.select("id", "display_name")
        .where(Accounts.col("id") >= created["id"])
        .order_by(Accounts.col("id").asc()).limit(20).all(connection)
    )
    deleted = await Accounts.delete().where(key).execute(connection)
    return changed.old, changed.new, rows, deleted
```

`find()` selects every declared field, `select(*fields)` a subset and
`find_by_key(mapping)` the row of a primary key. A query projects each field's
column qualified by the table's alias or else its name, never its schema, as
pgorm qualifies an entity's columns. Queries provide `where`, `order_by`,
`limit`, `offset` and `join(relation, *, kind=Join.Inner, alias=None)`, which
joins a relation's far end — from the query's model or a table it joined,
`Join.Inner` or `Join.Left` — without reading its columns. Reusing a query
preserves its original state. `as_(alias)` produces another descriptor whose
native expressions use that alias, including self-joins:

```python
left, right = Accounts.as_("a"), Accounts.as_("b")
query = left.select("id").join(left.has_one(right, from_="id", to="id"))
```

`all`, `one` and `optional` run on a pool, a connection or a transaction;
`count` counts the rows as pgorm's paginator does, `SELECT COUNT(*) AS
"num_items" FROM (..) AS "sub_query"` with the limit, offset and ordering
dropped, and `inspect(terminal="count")` shows that statement. `await
query.stream(db)` on a pool or a connection gives a `RowStream` of records.

`col(field)` validates the field and is itself an `Expr`, so it goes wherever
an expression goes. Every operator that takes a value converts it through the
field's declared native Value tag: `eq`, `ne` and the ordering comparisons,
`is_in` and `is_not_in`, `between` and `not_between`, `is_distinct_from` and
`is_not_distinct_from`, `contains`, `contained_by`, `overlaps` and the range
position operators. A comparison takes `any_(..)` or `all_(..)` as written,
membership a list or tuple of values or a `Select`'s rows, and every operator
an expression as written. Every other `Expr` method — `+`, `like`, `cast`,
`as_`, `asc`/`desc` — reads the column as written and binds its operands as an
`Expr`'s do. `col(field).expr()` gives the plain expression, whose operators
bind as an `Expr`'s do. `None` is refused as a compared value: use
`is_null()`.

`key(mapping)` constructs a native conjunction over all declared primary-key
fields, in declaration order. It requires every key field and rejects extras.
Keys may be composite. Nullable primary-key declarations are rejected. A key
declaration alone neither creates a constraint nor checks the database schema.

`insert(mapping)` and `insert_many(rows)` return a `ModelInsert`,
`update(mapping)` a `ModelUpdate` and `delete()` a `ModelDelete`, each with
only its own statement's methods, so a type checker refuses an insert's `where`
or a delete's `returning_change` as it refuses any attribute a class lacks.
All three take `returning`, `execute` and `inspect`; an insert also takes
`on_conflict` and the upsert terminals, an update `where`, `all_rows()` and the
change terminals, and a delete `where` and `all_rows()`. `execute(connection)`
returns affected rows. `returning(*fields)` selects declared output fields and
returns a row-producing `ModelRows`; no arguments means all fields. Updates and
deletes require `where(predicate)` or explicit `all_rows()`, using the native
write guard. An empty update is an
error. An insert that sets no field requests the Rust builder's default row; an
insert must set every field neither nullable, defaulted nor generated, and no
write sets a field generated always. Every row of `insert_many` sets the same
fields; a batch of none sends nothing, `execute` giving 0 and the returning
terminals no rows. Inserts require an unaliased table and take
`on_conflict(action)` as an `Insert` does.

`returning_change` and `returning_changes` on an update, and
`returning_upsert` and `returning_upserts` on an insert, read each written
row's two versions as pgorm's `exec_returning_change(s)` and
`exec_returning_upsert(s)` do: a `Change` of `old` and `new` records, and an
upsert `Inserted(model)` or `Updated(change)`; a row an insert's conflict
clause held back is None from `returning_upsert` and left out of
`returning_upserts`. `inspect(terminal=...)` shows the statement each runs: an
insert's `"returning_upsert"` or `"returning_upserts"`, an update's
`"returning_change"` or `"returning_changes"`, any other a `TypeError`.

Missing mapping entries stay omitted, allowing database defaults on INSERT and
leaving fields unchanged on UPDATE. `None` is an explicit SQL NULL and requires
a nullable column. For JSON, `Value.json(None)` is JSON null, including in a
non-nullable JSON column. Plain lists/dicts in a JSON field are JSON values. An
expression is written as it stands. Values are copied into native owned Values
at construction; later mutation of the input mapping or containers cannot
change the query.

## Relations, graphs, cursors and pages

```python
author = Posts.belongs_to(Accounts, from_="account_id", to="id")
posts = Accounts.has_many(Posts, from_="id", to="account_id")

async def related(connection):
    accounts = await Accounts.find().order_by(Accounts.col("id").asc()).all(connection)
    per_account = await posts.load(connection, accounts)
    rows = await Posts.graph().join_one(author).order_by(Posts.col("id").asc()).all(connection)
    grouped = await Accounts.graph().join_maybe(posts).all_grouped(connection)
    page = await Accounts.find().cursor("display_name", "id").first(20).all(connection)
    pages = Accounts.find().order_by(Accounts.col("id").asc()).paginate(20)
    return per_account, rows, grouped, page, await pages.num_items_and_pages(connection)
```

`belongs_to`, `has_one` and `has_many` pair fields of each model — one, or
equal-length lists — in order; a join along a relation is the condition
pgorm's `join_condition` writes, each pair equal. `relation.find(row)` reads
the far end's rows matching a record, none when one of its key fields is None.
`relation.load(db, rows)` reads the far end of every row in one query: a list
per row for `has_many`, otherwise a record per row or None where the row's key
holds a None; a key nothing matches, or a `has_one` key two rows match, is a
`DecodeError`.

`graph()` reads a model's rows with the rows its relations reach, written as
pgorm's `SelectGraph` writes it. `join_one` is an INNER JOIN decoded as a
record, `join_maybe` a LEFT JOIN decoded as a record or None, and `via` a LEFT
JOIN of a table no record reads; each takes `alias=` and `from_=`, the decoded
source the relation starts at. Every decoded source is projected under its
own prefix, `s0_` for the root and `s{i}_` for the i-th slot, bounded to
PostgreSQL's identifier limit by pgorm's own `result_column_name`. A graph
with no slot decodes as the root's record, otherwise as a tuple of each
source's; `col(source, field)` qualifies a field as its source is named.
`all_grouped` reads a graph of one slot as each root with its slot's records.

`cursor(*fields)` on a query or a graph pages by keyset as pgorm's `Cursor`
does, a graph's root key and each slot's key as tiebreaks: `after` and
`before` take the order fields' values, `after_with` and `before_with` those
or the whole key's, `first(n)` and `last(n)` the window and `asc()` and
`desc()` the direction. `paginate(page_size)` reads page `n` with a limit of
`page_size` and an offset of `page_size × n`, as pgorm's `Paginator` does;
`fetch_page`, `num_items`, `num_pages`, `num_items_and_pages` and the
asynchronous iterator `pages` run it.

## Disjoint models

A disjoint model stores a value of one of several kinds across tables: a base
model whose every row has a kind, and a model for each kind that carries data
of its own, keyed by the base row's key. It is pgorm's `DisjointShape`, the
description `#[derive(Disjoint)]` gives a Rust enum, so every statement — the
read, the writes and the schema — is the one the Rust declaration of the same
tables sends.

```python
import pgorm as p
import pgorm.schema

kind = p.TypeName("vehicle_kind", schema="app")
Vehicles = p.Model("vehicle", {
    "id": p.Column("i32", primary_key=True, generated="by_default"),
    "kind": p.Column(kind, values=["car", "truck", "bike"]),
    "name": p.Column("text"),
}, schema="app")
Cars = p.Model("car", {"id": p.Column("i32", primary_key=True), "seats": p.Column("i16")}, schema="app")
Trucks = p.Model("truck", {"vehicle_id": p.Column("i32", primary_key=True), "axles": p.Column("i16")}, schema="app")

Fleet = p.Disjoint("vehicle", Vehicles, {"car": Cars, "truck": Trucks, "bike": None}, schema_name="app")

async def fleet(pool):
    for statement in pgorm.schema.create_disjoint(Fleet):
        await pool.execute(statement)
    base, kind, car = await Fleet.insert({"name": "mini"}, "car", {"seats": 4}).returning_model(pool)
    roomy = await Fleet.find().filter_kind("car", Cars.col("seats") >= 4).all(pool)
    await Fleet.update({"id": base["id"], "name": "cooper"}, "car", {"seats": 5}).returning_model(pool)
    await Fleet.change_kind({"id": base["id"]}, "bike").returning_model(pool)
    return roomy
```

`Disjoint(name, base, kinds, *, kind="kind", enum_name=None, schema_name=None)`
names the base's kind field and the kind type, `{name}_kind` unless
`enum_name` names another; a model's enum column names its schema, so
`schema_name` names the kind type's. What the Rust declaration refuses — no
kinds, one model for two kinds, a kind model keyed by other types than the
base, a kind field missing, nullable, generated or of another type, a kind
model whose key the database fills — is a `ConstructionError` raised as the
model is declared.

`find()` and `find_by_key(key)` read each base row with its kind's row as a
tuple `(base record, kind, kind record or None)`; `where`, `order_by`,
`limit` and `offset` are a query's, `kind(k)` and `kinds([..])` narrow by
kind, and `filter_kind(k, predicate)` to rows of one kind whose row passes
the predicate. The terminals are `all`, `one` and `stream`, beside `count`,
`paginate(size)` and `cursor(*fields)` over base fields. `insert(base, kind,
values)`, `update(base, kind, values)` and `change_kind(key, kind, values)`
build one statement each, its parts joined by data-modifying `WITH` clauses,
whose terminal `returning_model(db)` reads back the written row. A write that
touches no row — a key no row has, a row of another kind, a change to the
row's own kind — changes nothing and is a `DecodeError`; a base kind field or
a kind row's key given a value is refused before anything is sent.
`pgorm.schema.create_disjoint(model)` gives the kind type, the tables and the
constraints holding each base row to exactly its kind's table; a column
declared `default=True` has no `DEFAULT` to write, so its table is refused.

Tables an existing schema holds with no kind column — a base, and tables each
keyed by a foreign key from its primary key onto the base's — are declared
`unkinded=True`, and read by which kind's table has a row for each base row,
`None` naming at most one kind of a row with none:

```python
Shops = p.Disjoint("shop", Shop, {"web_shop": WebShop, "storefront": Storefront, "dormant": None}, unkinded=True)
online = await Shops.find().kind("web_shop").all(pool)
```

Nothing there holds a base row to one kind's table, so such a model is
read-only: it is a `Disjoint[Literal["unkinded"]]`, whose `insert`, `update`,
`change_kind` and `create_disjoint` a type checker refuses, and each refuses
with a `ConstructionError` at run time. `kind`, `enum_name` and `schema_name`
are refused beside `unkinded`, and so are a second kind without a model and no
kind with one. A base row with rows in two kinds' tables, or in none where
every kind has a model, is a `DecodeError`.

## Records and types

`all`, `one` and `optional` accept a Pool, an acquired Connection or a
Transaction. They return `ModelRecord` views over detached native Records,
keyed by declared Python identities. Each field is read from the result column
its statement projected it under — its SQL column, or a graph source's
prefixed name, which `record.columns` lists. `record.model` identifies the
descriptor, `record.native` retains PostgreSQL field metadata and
`record.tagged(field)` preserves native tags. Mutable values returned by record
lookup are independent copies.

Result columns, native Value types, enum qualifications, nullability and an
enum's listed labels must match the declaration. A mismatch raises
`DecodeError`; it is never interpreted as an absent row. Dynamic `one`
requires exactly one row and `optional` at most one, so any other count
raises `DecodeError`. Use explicit `limit(1)` to select a first row. Errors do not
imply rollback of a write already executed.

Supported scalar kinds match the native Record decoder: `bool`, `i8`, `i16`,
`i32`, `i64`, `u32`, `f32`, `f64`, `text`, `bytes`, `decimal`, `uuid`, `json`,
`date`, `time`, `datetime`, `datetime_utc`, `ipnetwork`, `mac_address` and `vector`,
and the range kinds `int4range` through `tstzrange` and `int4multirange` through
`tstzmultirange`, whose values are `pgorm.Range` and `pgorm.Multirange`. A range
type a schema created is a schema-qualified `CreatedRange`, since a decoded row
names the type with its schema, and its multirange a schema-qualified
`CreatedMultirange`. The driver reports such a multirange with no subtype to
read its binary form by, so a model projects its column cast to `text`, as
pgorm's entities do, and reads that text as a `pgorm.Multirange` of the declared
subtype: text the subtype does not read raises `DecodeError`. Arrays of either
are refused.
Here `i8` corresponds to PostgreSQL's internal `"char"` and `u32` to `oid`.
`vector` requires the PostgreSQL vector extension. One-dimensional arrays use
`array=True`; nullable elements retain native Value semantics. Enums require a
schema-qualified `TypeName`, also for array elements.

Declarations require the canonical decoded tags. For example, PostgreSQL
`integer` produces `i32`; declaring it as `i64` is an error even if its value
fits. PostgreSQL `timestamptz` produces UTC datetimes. The Value-only variants
`u64` and Unicode `char` are not model column kinds because the Record decoder
does not produce those tags. The full value
conversion limits remain in [VALUES.md](VALUES.md). The capability manifest
records the model kinds and policies explicitly.

`query.statement` and `write.statement` expose the actual immutable native
builders. `inspect()` uses their Rust SQL/parameter output. Descriptor methods
lower to `NamedTable`, `Expr`/`SimpleExpr`, `Value`, `SelectStatement`,
`InsertStatement`, `UpdateStatement`, `DeleteStatement` and the existing native
Record execution path. Python contains no SQL renderer or optimizer.
`tests/test_models_parity.py` holds every model statement to the SQL and values
pgorm's own entities, `SelectGraph`, `Cursor` and `Paginator` build for the
same tables, the golden file pgorm-napi's Rust parity test writes.

Run the installed-wheel descriptor suite with the existing PostgreSQL fixture:

```sh
target/python-dev/bin/python pgorm-python/tests/with_postgres.py \
  target/python-check/bin/python -m unittest discover \
  -s pgorm-python/tests -p 'test_mode*.py' -v
```
