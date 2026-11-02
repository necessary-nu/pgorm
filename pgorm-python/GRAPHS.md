# Compiled typed graphs

`pgorm.graph(name)` selects a compiled `SelectGraph<E, S>` registration. Its
root entity and tuple of `Req<F>` / `Opt<F>` slots are fixed Rust types.
`Req` retains Rust's inner join and required model; `Opt` retains its left join
and optional model. Rust generates the complete prefixed projection and
decodes it through `GraphRow`. Python cannot substitute a different entity
tuple or a hand-written projection.

## Register an application shape

Use the [single-module application wheel](ENTITIES.md) workflow. Register
every source entity, then supply a factory that builds the concrete graph:

```rust
use pgorm::{EntityTrait, Opt};
use pgorm::pgorm_query::Name;

registry.entity::<account::Entity>("app.Account")?;
registry.entity::<note::Entity>("app.Note")?;
registry.graph::<account::Entity, (Opt<note::Entity>,), _>(
    "app.AccountNotes",
    |aliases| account::Entity::graph().join_maybe_as::<note::Entity>(
        account::Entity::has_many(note::Entity).into(),
        Name::runtime(&aliases[0]),
    ),
)?;
```

The application's `note::Entity` supplies its real `Related<account::Entity>`
implementation. The factory receives exactly one validated alias per joined
slot, in tuple order. It must apply these aliases to the corresponding Rust
`join_*_as` calls. The root uses its entity table name. Undecoded `via` hops
and application relation conditions remain part of the compiled factory.
Factories must preserve the declared source identities; their implementation
is application Rust code included in the wheel's tests.

The binding supports Rust's slotless shape and tuples of one through six slots
(one through seven decoded sources). Each source must satisfy the same model
and ActiveModel bounds as an entity registration. Missing source registrations,
duplicate graph names, and unknown Python graph names are explicit errors.
`pgorm.graphs()` lists the installed graph names. A graph's `describe()`, and
`capabilities()["registrations"]["graphs"]` for every installed graph, reports
its `name`, its `rust_shape`, the Rust `SelectGraph` type, and its `sources`,
the root and then each slot, each with its `index`, the `entity` it decodes as
and its `slot` (`root`, `Req` or `Opt`): the keys JavaScript's reports, each
spelled in snake_case. A standalone wheel has none.

## Compose and read

```python
import pgorm as p

AccountNotes = p.graph("app.AccountNotes")
query = AccountNotes.find(aliases=["n"])
query = query.where(query.col(0, "id") >= 10)
query = query.order_by(query.col(0, "id").asc(), query.col(1, "id").asc())

async with pool.connection() as connection:
    rows = await query.all(connection)
    first = await query.optional(connection)
    for account, note in rows:
        print(account["id"], None if note is None else note["body"])
```

Source zero is the root. `col(source, column)` validates the declared source
index and SQL column name, then builds an owned `Expr::col` with the effective
qualifier. It has the normal dynamic expression conversion rules: use an
explicit `Value` or cast when a particular value tag/type is needed. Expressions
contain their qualifier; compose them from the query whose aliases you intend
to reference. Changing filters or ordering returns a new query. Call `find`
again to select different aliases; existing expressions and queries retain
their original names. Default joined aliases are `g1`, `g2`, and so on (adjusted
if a name equals the root's table).

`all` returns every graph row in a list. A joined row is a Python tuple in
declared source order, containing detached native `EntityModel` objects and
`None` for absent optional slots. A slotless row is a bare `EntityModel`.
Models retain their registered columns, value tags and `into_active` behavior.
`optional` adds Rust's `LIMIT 1` and returns the first row or `None` for no row.
An absent optional source is distinct from no graph row. A present source whose
model cannot decode raises `DecodeError`; it does not become absent.

The relation a factory joins by is the application's, attributes included. A
relation matching on a `PERIOD` (`from_period` / `to_period`, PostgreSQL 18's
temporal foreign key) joins by equality on its key columns and by overlap
(`&&`) on the two periods, so a row reaches every version of its target whose
period overlaps its own: one root row per overlapping version. A relation the
server does not enforce (`enforcement = Enforcement::NotEnforced`) may name a row that does
not exist; an optional slot then holds `None` and a required slot drops the
root row, as for any unmatched join. Deferrability changes when the server
checks the key, never what a join reads. The entity's `describe()` reports each
of these on its `relations` (see [ENTITIES.md](ENTITIES.md)).

`inspect()` returns Rust-built SQL and tagged parameters for `all`.
`inspect(terminal="optional")` includes that terminal's limit. Execution uses
the same owned `SelectGraph` builder state and calls its actual Rust terminal.
Graph reads run on a pool, an acquired connection or a transaction and return
native asyncio awaitables, with the same loop ownership, busy-connection
rejection and cancellation/discard policy as entity reads.

## Keyset cursors

```python
cursor = query.cursor("id")
async with pool.connection() as connection:
    page = await cursor.first(20).all(connection)
    # Resume inside a root's matched notes using its full key:
    next_page = await cursor.after_with(account_id, note_id).first(20).all(connection)
```

Cursors call Rust's `SelectGraph::cursor_by` for one root column, followed by
the real `Cursor` methods and `Cursor::all`. Rust installs primary-key
tiebreaks for the root and every decoded slot. `before(value)` and `after(value)`
bound the root order column only. `before_with(*values)` and
`after_with(*values)` bound the whole key: order-column value, remaining root
primary-key values, then each joined source's primary-key values in declaration
order. Input conversion uses those registered column types, including enum
casts. Wrong arity and invalid input types fail before execution.

`first(n)` and `last(n)` replace any earlier window. `asc()` and `desc()` set
the direction. Every operation returns an independent cursor, so the original
can be reused. Cursor ordering replaces the query's earlier ordering, as in
Rust. Last-page rows are returned in the requested logical order after Rust's
reverse fetch. A zero-sized window returns no rows.

An unmatched optional slot has a NULL tiebreak. PostgreSQL comparisons against
NULL do not select a continuation inside that key; Rust's primary-column
boundary is the way to cross unmatched roots. A primary-only boundary also
skips remaining joined rows with the same root value. Choose the boundary that
matches the page's position.

The installed capability contract currently excludes slot-column cursors,
composite order-column cursors, graph streaming/grouping, and cursor SQL
inspection. These operations do not fall back to another query path. Entity
combinations require registrations, and arities beyond seven are unsupported.

## Verification

The [application checker](checks/entities.py) builds and installs the downstream
wheel in a clean environment. It exercises every supported source arity,
required/optional decoding, quoted aliases, invalid boundaries, cancellation,
and entity regression tests. Rust tests compare graph SELECT SQL and parameters.
Eight installed Python cursor result sequences are also compared with
independent Rust `SelectGraph` cursors against the same fixture data. The
checker writes its evidence under `target/python-entities` and reports success
only after these comparisons pass.
