# Explicit schema construction

`pgorm.schema` owns native Rust DDL builders. Constructing a statement, importing
a module or declaring a `pgorm.Model` performs no database work. Execute each
statement explicitly through `Pool.execute` or `Connection.execute`.

```python
import asyncio
import os
import pgorm as p
from pgorm import schema as s

async def main() -> None:
    table = p.Table("python_schema_example")
    statement = (
        s.create_table(table)
        .column(s.ColumnDef("id", "integer").identity("always"))
        .column(s.ColumnDef("name", s.DataType("varchar", length=80)).not_null())
        .column(s.ColumnDef("created_at", "timestamptz"))
        .primary_key("id")
    )
    print(statement.inspect().sql)
    async with p.Pool(os.environ["PGORM_TEST_DSN"]) as pool:
        await pool.execute(statement)
        try:
            await pool.execute(s.create_index(table, "name", name="example_name_idx"))
            row = await pool.one(
                p.insert(table).columns("name").values("Nora")
                .returning(p.col("id"), p.col("name"))
            )
            print(dict(row))
        finally:
            await pool.execute(s.drop_table(table))

if __name__ == "__main__":
    asyncio.run(main())
```

Every complete statement is a `s.DDL`: `CreateTable`, `AlterTable`,
`CreateIndex`, `CreateType`, `AlterComposite`, `RenameAttribute`,
`CreateDomain`, `CreateSequence`, `AlterSequence`, the view builders and the
plain statements the other functions give all extend it, and every terminal
takes one. Builder methods return new objects. A statement is made by its
function, never by its class: `create_table` gives a `CreateTable`, and
`create_index`, `create_type`, `create_domain`, `create_sequence`,
`create_view`, `create_recursive_view`, `create_materialized_view` and
`create_table_as` give theirs. The classes are exported for annotations and
`isinstance`, and calling one is a `TypeError`. An `ALTER` that names its
object and no action yet — `s.alter_table(table)`, `s.alter_type(name)`,
`s.alter_domain(name)` and `s.alter_sequence(name)` — is a pending object of
its own, no `DDL`: it has no `inspect()`, every terminal refuses it with
`ConstructionError`, and its first action gives the statement.

A table or sequence is named by a `str`, a `p.Identifier` or a `p.Table`
without an alias, whose `schema=` qualifies it; a type or domain by a name or a
`p.TypeName`. Where a statement takes several names — a key's columns, the
objects a drop drops — it takes one name or a non-empty list of them. A closed
choice is a string: a drop's `behavior` is `"cascade"` or `"restrict"`, and the
others are spelled in snake_case (`"not_enforced"`,
`"deferrable_initially_deferred"`, `"set_null"`). An argument of the wrong type
is a `TypeError`, a string outside its closed choice among them, since the
choice's type is the `Literal` of its strings; a value that cannot become what
pgorm writes — an empty list, an aliased `Table`, a label PostgreSQL cannot
store, an integer outside `bigint` — is a `ConstructionError`.

## Columns

`ColumnDef(name, kind)` takes a type, which only a `modify_column` that does
not retype the column leaves out. Its clauses render in the order they are
added. Generated columns always name their kind:
`.generated(expr, "stored")` or `.generated(expr, "virtual")`, since
PostgreSQL 17 requires `STORED` and 18 reads a bare one as `VIRTUAL`.
`.identity("always" | "by_default", **options)` is `GENERATED .. AS
IDENTITY`, the form PostgreSQL recommends over `.auto_increment()`'s serial
family; its options are a sequence's (`start_with=`, `increment_by=`,
`min_value=`, `max_value=`, `cache=`, `cycle=`, a bound's `None` being its
`NO` form). `.not_null(name=None, no_inherit=False)` is the column's one `NOT
NULL` constraint, which PostgreSQL 18 records by name — the one given, or one
it derives — and which a table that inherits this one takes unless
`no_inherit=True`. `.check(condition, name=None, enforcement=None,
no_inherit=False)` adds a `CHECK`; `enforcement="not_enforced"` is PostgreSQL
18's `NOT ENFORCED`, recorded and never checked. `.collate(name, schema=None)`
is the column's one collation, written after its type:
`s.ColumnDef("code", "text").collate("C")` compares and sorts the column's text
bytewise, and a later call replaces an earlier. `modify_column` writes it on
the retype, the only place PostgreSQL changes a column's collation. The server
refuses a collation that does not exist (`42704`) and one on a type without
collations (`42804`). Use unqualified `p.col(...)` expressions inside checks
and generated columns.

## Keys and foreign keys

Keys are the table's: `CreateTable.primary_key(columns)` declares its one
primary key, which a later call replaces, and `CreateTable.unique(columns,
nulls_not_distinct=False)` adds a unique key, so a table cannot be written with
two primary keys. Either takes `name=`, `include=[...]` (columns carried in the
key's index), `deferrability=` (`"not_deferrable"`,
`"deferrable_initially_immediate"` or `"deferrable_initially_deferred"`) and
`without_overlaps=column`, PostgreSQL 18's temporal key: the column, a range or
multirange, is written last as `column WITHOUT OVERLAPS`, and two rows may then
share the other columns only while their periods do not overlap. Its GiST index
needs the `btree_gist` extension for the other columns.

```python
owner = p.Table("owner", schema="app")
pet = (
    s.create_table(p.Table("pet", schema="app"))
    .column(s.ColumnDef("owner", "integer"))
    .column(s.ColumnDef("held", "daterange"))
    .foreign_key("owner", owner, "id", on_delete="cascade", name="pet_owner")
    .foreign_key(["owner"], owner, ["id"], period=("held", "during"))
)
```

`foreign_key(columns, references, ref_columns)` pairs its columns in order with
as many columns of the referenced table, anything else refused, and takes
`name=`, `on_delete=` and `on_update=` (`"no_action"`, `"restrict"`,
`"cascade"`, `"set_null"`, `"set_default"`), `deferrability=`,
`enforcement=` and PostgreSQL 18's `period=(column, referenced_column)`,
written last on both sides. `CreateTable` also takes `check(condition,
name=None, enforcement=None, no_inherit=False)` and `if_not_exists()`.

## Altering a table

`s.alter_table(table)` gives a `PendingAlterTable`; its first action gives an
`AlterTable` that takes more, so one `ALTER TABLE` holds several actions,
applied in order:

```python
change = (
    s.alter_table(p.Table("stock", schema="app"))
    .add_column(s.ColumnDef("label", "text").default("none"))
    .modify_column(s.ColumnDef("code", "bigint").not_null().default(1))
    .drop_column("legacy")
    .add_primary_key("id", name="stock_key")
    .add_foreign_key("supplier", p.Table("supplier", schema="app"), "id", not_valid=True)
)
```

The actions are `add_column(column, if_not_exists=False)`,
`modify_column(column)`, `drop_column(name)`, `add_primary_key`,
`add_unique` and `add_foreign_key` (the keys `CreateTable` declares, with the
same options), `add_check(condition, name=None, enforcement=None,
no_inherit=False, not_valid=False)`, `add_not_null(column, name=None,
no_inherit=False, not_valid=False)`, `drop_constraint(name, if_exists=False,
behavior=None)`, `validate_constraint(name)`, `alter_constraint(name,
change)`, `set_expression(column, expr)` and `drop_expression(column,
if_exists=False)`.

A `modify_column` definition writes each aspect it carries — a retype with its
collation, `SET` or `DROP NOT NULL`, a named `NOT NULL` added, `SET DEFAULT`, a
`CHECK` added, an identity added. An aspect no such action writes is refused
rather than dropped: a generated expression, which `set_expression` changes —
the rows already written take the new one — and `drop_expression` removes,
leaving a stored column plain with its values; the serial family; and a
collation without its type. So is a column that changes nothing.

`not_valid=True`, which a foreign key, a `CHECK` and a `NOT NULL` take, leaves
the rows already there unchecked until `validate_constraint(name)` checks
them, while new rows are held to the constraint at once. `no_inherit=True`
keeps a `CHECK` or a `NOT NULL` from tables that inherit this one; a
partitioned table refuses it. `alter_constraint(name, "inherit" |
"no_inherit")` changes whether a `NOT NULL` passes to inheriting tables, and
`alter_constraint(name, "enforced" | "not_enforced")` whether a foreign key, or
on PostgreSQL 19 a `CHECK`, is enforced; enforcing one checks the rows already
there. `drop_constraint` drops a constraint of any kind by name — a key, a
foreign key, a `CHECK` or a `NOT NULL`.

Beside it, `rename_column(table, name, new_name)`, `rename_constraint(table,
name, new_name)` (a key's index renamed with it), `rename_table(table,
new_name)`, `truncate_table(table)` and `drop_table(tables, if_exists=False,
behavior=None)` each give a statement, and `comment_on_table(table, text)` and
`comment_on_column(table, column, text)` write `COMMENT ON`, the text an
escaped literal. Database validation and privileges still apply; these APIs do
not introspect the database or compute migrations.

## Indexes

`create_index(table, on, name=None, order=None, operator_class=None)` is an
index over its first entry, a column's name or an expression;
`.column(on, order=None, operator_class=None)` appends another, `order` being
`"asc"` or `"desc"`. `.unique()`, `.nulls_not_distinct()` (which PostgreSQL
defines for a unique index alone, so it makes the index unique),
`.if_not_exists()`, `.using(method)` (any access method the server has, by
identifier), `.include([...])` (columns carried without being indexed) and
`.where(predicate)` (a partial index's predicate, ANDed to one already there)
select the rest. `CONCURRENTLY` is not offered: PostgreSQL refuses it inside a
transaction. `drop_index(table, name, if_exists=False)` uses the table's schema
to qualify the index name.

```python
person = p.Table("person", schema="app")
index = (
    s.create_index(person, p.call("lower", p.col("email")), name="person_email",
                   operator_class="text_pattern_ops", order="desc")
    .column("id").unique().include(["nick"]).where(p.col("active") == p.literal(True))
)
```

## Types

`DataType` accepts the closed built-in names listed in
`p.capabilities()["schema_policy"]["column_types"]`. `p.DataType` is the same
class as `s.DataType`, exported beside the statement builders whose
`column_definition` and SQL/JSON `returning` take one too. `length=` applies to char,
varchar, bit, varbit and vector; varbit requires it. `numeric` accepts
`precision=1..1000` with optional `scale=0..1000`. The six built-in range types
and their multiranges are named as PostgreSQL names them, `int4range` through
`tstzmultirange`, the column type a `without_overlaps` key ends with. A
`CreatedRange` or `CreatedMultirange` names a range type a schema created, Rust's
`ColumnType::CreatedRange` / `CreatedMultirange`, written quoted and
schema-qualified as it was given.
A `p.TypeName` references an
existing named type with full schema identity. `.array()` retains that element
type. A type usable in DDL is not necessarily decodable as a dynamic `Record`;
the separate `result_policy` lists result conversion support.

```python
mood = p.TypeName('Mood "name"', schema="application")
enum = s.create_type(mood).values(["calm", "O'Brien", ""])
column = s.ColumnDef("mood", mood).default(p.Value("calm", mood))
array_column = s.ColumnDef("moods", s.DataType(mood).array())
pair = (
    s.create_type(p.TypeName("pair", schema="application"))
    .attribute("a", "integer")
    .attribute("b", "text", collation={"name": "C", "schema": "pg_catalog"})
)
span = s.create_type("floatrange").as_range("double", subtype_diff="float8mi")
```

`create_type(name)` is a shell type until a kind is chosen, and what a type is,
is one slot: `as_enum()` and `values(labels)`, which appends labels and makes
the type an enumeration; `as_composite()` and `attribute(name, kind,
collation=None)`; or `as_range(subtype, collation=None, subtype_opclass=None,
subtype_diff=None, multirange_type_name=None)`. A collation is a name or
`{"name": .., "schema": ..}`. `s.alter_type(name)` gives a `PendingAlterType`
whose `add_value(label, before=None, after=None)`, `rename_to(new_name)` and
`rename_value(label, new_label)` each give a statement,
`rename_attribute(name, new_name)` a `RenameAttribute`, and a composite's
`add_attribute`, `drop_attribute(name, if_exists=False)` and
`alter_attribute` an `AlterComposite` that takes more; both take
`behavior("cascade")` to carry the change into typed tables.
`drop_type(names, if_exists=False, behavior=None)` drops them. Labels are data:
empty labels are valid, and labels must contain no NUL and fit PostgreSQL's
63-byte limit. Creating a named column does not create its type. Create schemas
explicitly before their objects, with `s.create_schema` ([below](#schemas)).

Rust's PostgreSQL driver caches type definitions on physical connections.
After changing existing enum labels or renaming a type, close and recreate
application pools before querying those types. Closing one acquired Python
connection returns it to the pool and does not refresh the driver's cache.
These bindings preserve the native DDL execution behavior; they do not coordinate
live schema changes across application connections.

DDL inspection returns `Compiled` with no parameters: Rust's DDL renderer emits
typed literals for defaults and checks because these statements cannot use
query placeholders. Python does not concatenate identifiers or values into
DDL. Arbitrary column options and concurrent indexes are not exposed.
`schema_policy` records these limits.

Like SQL expressions and statements, every `pgorm.schema` object — the
builders, the pending `ALTER`s, `DDL` and `EntitySchema` — and the `Compiled`
an inspection returns raise `ConstructionError` on Python truth testing, so
`if statement:` or `statement or fallback` fails where it is written rather
than reading every statement as true.

## Sequences, extensions and comments

```python
ticket = p.Table("ticket", schema="app")
statements = [
    s.create_sequence(ticket).as_type("integer").options(start_with=10, increment_by=5, min_value=None),
    s.alter_sequence(ticket).restart(100).owned_by(p.Table("booking", schema="app"), "id"),
    s.rename_sequence(ticket, "voucher"),
    s.create_extension("btree_gist", schema="app", version="1.7", if_not_exists=True),
]
```

`create_sequence(name)` takes `if_not_exists()`, `as_type("smallint" |
"integer" | "bigint")`, `options(**options)` — at least one, merged into those
set, a later one for a clause replacing the earlier — and `owned_by(table,
column)`, or `owned_by(None)` for `OWNED BY NONE`. `s.alter_sequence(name)`
gives a `PendingAlterSequence` whose first clause — the same three, or
`restart(value=None)` — gives an `AlterSequence` that takes more and
`if_exists()`. The options are the identity column's: integers within
`bigint`, a bound's `None` its `NO` form. `drop_sequence(names,
if_exists=False, behavior=None)` and `rename_sequence(name, new_name)` give
statements. `create_extension(name, if_not_exists=False, schema=None,
version=None, cascade=False)` writes the version as a literal, and
`drop_extension(name, if_exists=False, behavior=None)` drops one.

## Schemas

A schema is the namespace a `p.Table(..., schema=...)`, a `p.TypeName` or a
sequence is qualified by. It is named by one identifier, a `str` or a
`p.Identifier`, quoted, and never qualified itself.

```python
statements = [
    s.create_schema("billing", if_not_exists=True),
    s.create_table(p.Table("invoice", schema="billing")).column(s.ColumnDef("id", "integer")),
    s.rename_schema("billing", "ledger"),
    s.drop_schema(["ledger", "archive"], if_exists=True, behavior="cascade"),
]
```

`create_schema(name, if_not_exists=False)` creates one; a schema that exists
is refused (`42P06`) unless `if_not_exists`, and PostgreSQL keeps names
starting `pg_` for itself (`42939`). `drop_schema(names, if_exists=False,
behavior=None)` drops every schema it names. A schema that holds anything is
refused (`2BP01`) unless `behavior="cascade"`, which drops what it holds as
well; a missing one is refused (`3F000`) unless `if_exists`.
`rename_schema(name, new_name)` renames one, its contents moving with it; it
has no `if_exists`, PostgreSQL having none for it. `AUTHORIZATION` and `OWNER
TO`, which name a role, are not built.

## Domains

A domain is a named type over a base type, with a default, a `NOT NULL` and
named `CHECK`s of its own. It is named as a type is: a `str` or a
`p.Identifier`, or a `p.TypeName` carrying its schema. A column whose type is a
domain names it as a `p.TypeName`, exactly as a column of an enum type does.

```python
quantity = p.TypeName("quantity", schema="billing")
statements = [
    s.create_domain(quantity, "integer").default(1).not_null()
        .check(p.domain_value() > 0, name="positive"),
    s.create_table(p.Table("line", schema="billing"))
        .column(s.ColumnDef("qty", quantity)),
    s.alter_domain(quantity).add_check(p.domain_value() < 1000, name="small", not_valid=True),
    s.alter_domain(quantity).validate_constraint("small"),
    s.drop_domain(quantity, behavior="cascade"),
]
```

`create_domain(name, base)` builds as `create_table` does, each method
returning a new builder: the base is any type `ColumnDef` takes, `collate(name,
schema=None)` names a collation for a base type that has one (`42804`
otherwise), `default(value)` and the pair `not_null()` / `null()` are one slot
each, the later call winning, and `check(condition, name=None)` adds a check,
in order. A check's condition is an expression over `p.domain_value()`, the
keyword `VALUE`: inside a domain's check PostgreSQL reads it as the value
tested.

`s.alter_domain(domain)` gives a `PendingAlterDomain`, whose actions each give
the whole statement, PostgreSQL taking one action per `ALTER DOMAIN`:
`set_default(value)`, `drop_default()`, `set_not_null()`, `drop_not_null()`,
`add_check(condition, name=None, not_valid=False)`, `drop_constraint(name,
if_exists=False, behavior=None)`, `rename_constraint(name, new_name)`,
`validate_constraint(name)` and `rename_to(new_name)`. None takes `if_exists`,
PostgreSQL having none for `ALTER DOMAIN`. `drop_domain(names,
if_exists=False, behavior=None)` drops every domain it names; one a column is
typed with is refused (`2BP01`) unless `behavior="cascade"`, which drops the
column too.

A value outside a check (`23514`) or a null under `NOT NULL` (`23502`) is
refused whether it is written to a column of the domain or already there when
`add_check`, `set_not_null` or `validate_constraint` runs; `not_valid=True`
leaves the values already there for `validate_constraint` to check. A name a
type or a constraint already has is refused (`42710`), and a missing domain or
constraint (`42704`) unless the drop says `if_exists`. An insert the connection
prepared before `set_default` or `drop_default` keeps the default it was
planned with: PostgreSQL's plan cache does not track a domain's default. Call
`pool.clear_statement_caches()` after the change, and each connection prepares
the insert again before it is next lent.

## Views, materialized views and tables from a query

A view, a materialized view and `CREATE TABLE AS` wrap a `Select` in a name.
PostgreSQL stores a view's definition as SQL text, so the query's values are
written into it as literals by Rust's renderer, and `inspect().params` is
empty. The name is a relation's, as a table's is; `.columns([...])` appends
names for the leading columns the query returns. The query is read as a CTE
body or a subquery is: nothing can order it once it is a definition, so a
`limit_with_ties` without `order_by`, or a WINDOW clause PostgreSQL would
refuse, raises `ConstructionError` as the builder is constructed.

```python
account = p.Table("account", schema="app")
active = p.Select(p.col("id"), p.col("name")).from_(account).where(p.col("active") == True)
view = s.create_view(p.Table("active_account", schema="app"), active).check_option("cascaded")
snapshot = p.Table("account_snapshot", schema="app")
statements = [
    view,
    s.create_materialized_view(snapshot, active).with_no_data(),
    s.create_index(snapshot, "id", name="account_snapshot_id").unique(),
    s.refresh_materialized_view(snapshot),
    s.refresh_materialized_view(snapshot, concurrently=True),
    s.create_table_as(p.Table("account_copy", schema="app"), active).columns(["account_id"]),
]
```

`create_view(name, query)` takes `or_replace()`, `columns(names)`,
`check_option("local" | "cascaded")`, `security_barrier()` and
`security_invoker()`. Rows written through an automatically updatable view
reach its table; with a check option a row the view would not show is refused
(`44000`), `"cascaded"` consulting the views beneath it too. `or_replace()` may
add columns after the view's own and replaces its options. `security_barrier`
evaluates the view's conditions before a caller's, and `security_invoker` reads
the underlying tables with the querying role's privileges.
`create_recursive_view(name, columns, query)` names one column or a non-empty
list of them and has every method but `check_option`, which PostgreSQL refuses
on a recursive view. Its query is a non-recursive term joined by `union` or
`union_all` to an arm that reads the view (see
[STATEMENTS.md](STATEMENTS.md#set-operations)). PostgreSQL defines the view as
a `WITH RECURSIVE` query named by the view's bare name, so the arm reads it
unqualified; a schema-qualified name looks for a relation that does not exist
yet (`42P01`):

```python
counter = p.Table("counter", schema="app")
step = p.Select(p.col("n") + 1).from_(p.Table("counter")).where(p.col("n") < 4)
counting = s.create_recursive_view(counter, "n", p.Select(p.literal(1).as_("n")).union_all(step))
```

`drop_view(names, if_exists=False, behavior=None)` drops views.

`create_materialized_view(name, query)` stores the query's rows, or none until a
refresh with `with_no_data()`, and takes `if_not_exists()` and `columns(names)`;
`refresh_materialized_view(name, concurrently=False, with_no_data=False)`
replaces them. A concurrent refresh needs a unique index on the view and a
populated view, and cannot be combined with `with_no_data`, which is refused
as the statement is built. `drop_materialized_view(names, ...)` drops them.
`create_table_as(name, query)` takes the same three methods and creates an
ordinary table from the query's columns and, unless `with_no_data()`, its
rows. Neither has `or_replace`, and a view has no `if_not_exists`.

## Disjoint models

`create_disjoint(model)` gives a disjoint model's or registered disjoint
enum's statements in the order they run: the kind type, the base table with
a `UNIQUE` key on its key and kind column, each kind's table with its kind
column and foreign key, and the `ALTER TABLE` adding the base's generated key
columns and foreign keys — `Schema::create_disjoint`'s, from pgorm's
`DisjointShape` (`MODELS.md`). The types the tables' other columns use are
not among them.

## Registered Rust entities

In an application wheel, `s.from_entity(p.entity("app.Account"))` invokes the
real monomorphized `pgorm::Schema` methods for that registered Rust entity. The
returned `EntitySchema` has `.enums` (each a `CreateType`), `.table` (a
`CreateTable`), `.indexes` (each a `CreateIndex`) and `.comments`. Each
property returns detached native statements. Execute enums first, then the
table, then indexes and comments; deduplicate shared enum types when creating
multiple entities. Foreign keys or other dependencies may require an
application-specific creation order. A table Rust refuses to generate raises
`ConstructionError` with Rust's message: an entity with a `belongs_to` over a
column typed as a domain over an enum, on either side, is one, since
PostgreSQL builds no foreign key there (`42804`).

```python
async def create_account_schema(connection: p.Connection) -> None:
    generated = s.from_entity(p.entity("app.Account"))
    for statement in generated.enums:
        await connection.execute(statement)
    await connection.execute(generated.table)
    for statement in generated.indexes:
        await connection.execute(statement)
    for statement in generated.comments:
        await connection.execute(statement)
```

This example requires a registered `app.Account` and its schema to exist.
Runtime Python `Model` descriptors use the explicit builders above.
`operations["schema.from_entity"]["registration_required"]` distinguishes this
path from runtime DDL; `registrations.entities` lists this wheel's entities.
