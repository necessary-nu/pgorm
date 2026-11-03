# Generate an application module

`pgorm.codegen` creates an application wheel containing the common pgorm API,
your compiled Rust entity and graph registrations, and concrete Python models
with matching `.pyi` declarations. Query construction calls the installed
native builders; it does not compile a program or use HTTP.

The workflow has two explicit builds. The first makes Rust's compiled entity
and graph metadata available to the generator. The second packages the
generated wrappers and typing with that same extension. Repeat these steps
when Rust definitions, registrations or selected graph shapes change. Ordinary
queries over the installed types require neither build.

## Describe the application

Your entity crate exports concrete `EntityTrait` implementations. Selected
graph factories accept `&[String]` for joined aliases and return a concrete
`SelectGraph<E, S>`. Apply each supplied alias using `join_one_as` or
`join_maybe_as`, in source order. The root source keeps its registered table
name. See [graph registration](GRAPHS.md) for supported shapes and cursors.

Write `application.json` next to the entity crate's `Cargo.toml`:

```json
{
  "schema_version": 1,
  "module": "app",
  "entity_crate": ".",
  "entities": [
    {"name": "app.Account", "rust": "account::Entity", "python": "Account",
     "fields": {"display name": "display_name"}},
    {"name": "app.Note", "rust": "note::Entity", "python": "Note"}
  ],
  "graphs": [
    {"name": "app.AccountNotes", "rust": "account_notes", "python": "AccountNotes"}
  ]
}
```

`entity_crate` resolves relative to this JSON file. Rust paths resolve within
that crate and must name items; generic expressions and inline Rust are not
accepted. Each concrete entity type has one registration. Registration names
and generated Python exports must be unique. Public Python names are normalized
as Python identifiers; collisions with imports, builtins and model methods
are rejected. Use an explicit field alias to resolve a column-name collision.

The entity crate should expose ordinary Rust definitions. The generated crate
owns the application's single `pgorm._native` initializer. An application
wheel replaces the standalone `pgorm` wheel in its environment; independent
application extension wheels cannot be stacked in the same process.

## Build and generate

Start with the matching standalone pgorm wheel installed, Maturin 1 (1.15 or
later) and CPython 3.14. Set `PGORM_SOURCE` to the repository checkout and `APP_CONFIG` to
your application JSON file. Choose a new output directory:

```sh
python -m pgorm.codegen scaffold "$APP_CONFIG" \
  --pgorm-source "$PGORM_SOURCE" --output generated-binding
maturin build --manifest-path generated-binding/Cargo.toml \
  --interpreter python --out probe-dist
uv venv --python python probe-env
uv pip install --no-index --python probe-env/bin/python probe-dist/*.whl
probe-env/bin/python -I -m pgorm.codegen emit generated-binding
maturin build --manifest-path generated-binding/Cargo.toml \
  --interpreter python --out application-dist --locked
uv venv --python python application-env
uv pip install --no-index --python application-env/bin/python application-dist/*.whl
```

Scaffolding writes Cargo and Maturin manifests, the native registration module,
a copy of the shared Python facade and build metadata. It refuses an existing
destination. Emission writes `python/pgorm/app.py`, `app.pyi` and an application
module marker. Emission can be repeated deterministically after the probe wheel
is installed. The source paths in scaffold metadata are build inputs; the
installed generated module does not read those paths.

Cargo manifests pin pgorm and binding package versions. Preserve the resulting
`Cargo.lock` and use `--locked` for reproducible rebuilds. The generated module
checks package and pgorm versions, registry ABI, features, and compiled entity
and graph metadata at import. A mismatch raises `UnsupportedCapabilityError`
before a query is constructed or executed. Version and metadata checks do not
claim a cryptographic identity for arbitrary downstream hook code; rebuild and
deploy your application wheel whenever that code changes too.

## Use the concrete API

The build commands also have typed Python entry points:
`scaffold(config, destination, *, pgorm_source, base=".") -> pathlib.Path`
and `emit(project) -> pathlib.Path` in `pgorm.codegen`. Path arguments accept
strings or `os.PathLike[str]`. The configuration is validated at runtime;
invalid build descriptions raise `CodegenError` (a `ValueError` subclass).

```python
from pgorm.app import Account, AccountNotes

async def example(connection):
    model = await (
        Account.active().set_id(1).set_display_name("Nora").set_note(None)
        .insert(connection)
    )
    print(model.id, model.display_name, model.note)
    changed = await model.into_active().set_display_name("New name").update(connection)
    rows = await Account.find().where(Account.col("id") == changed.id).all(connection)
    graph = AccountNotes.find(aliases=["related_notes"])
    joined = await graph.cursor("id").first(20).all(connection)
    for account, note in joined:
        print(account.display_name, None if note is None else note.body)
```

For each entity `Account`, the module exports `AccountModel`, `AccountActive`
and an `Account` entity view. Model properties are read-only. Mapping keys remain
the actual SQL column names, so `model["display name"]` and
`model.display_name` address the same value. Without an explicit alias, a valid
SQL column name becomes the property name; otherwise the derived JSON key is
used. Native models remain accessible through `.native`, and `.tagged(column)`
retains exact Value tags and qualified enum identities.

Generated `set_<field>` methods construct native `Value` tags using reflected
standard Rust field spellings, then call the real Rust ActiveModel setter.
An `i32` field receives an `i32` Value even though Python's default integer
inference is `i64`. Explicit `Value` arguments pass through to Rust validation.
Setters, state changes and queries return new views around cloned native
state. Insert/update/delete retain the actual Rust hooks and outcomes.
The generic `.set` and `.with_value` methods keep the underlying native API's
conversion rules; supply an explicit Value for unusual custom representations.

The type source is `FromQueryResult::expected_columns`, generated by the Rust
derives. Known scalar and standard container spellings produce concrete Python
types, including nullable fields, integer widths for binding, bytes, decimals,
UUIDs, temporal values, intervals (`pgorm.Interval`, whatever the column's
field qualifier) and one-dimensional arrays. A column whose type is a domain
types and binds as the domain's base: a domain over an integer is an `int`,
one over an enum the enum's `str`. SQL `ColumnType` remains an
input hint and cannot establish an unknown Rust field type. Custom aliases and
manual decoders without reflection keep explicit `Any`. JSON's value shape is
also `Any`. Rust enum fields expose their Python string representation while
binding and tagged reads preserve the qualified PostgreSQL enum name.

Each graph exports a typed view and a `NameRow` alias. A single-source graph
returns its concrete model. Joined graphs return ordered tuples of concrete
models; each `Opt` slot contributes `Model | None`. Required slots remain
non-optional. The wrapper delegates queries and cursors to the registered
native graph, then wraps the decoded models without rebuilding SQL.

A `#[derive(Disjoint)]` enum is described under `disjoints`, each entry its
`name`, `rust` path and `python` export, its base and kind entities among the
described entities:

```json
"disjoints": [{"name": "app.Shipments", "rust": "shipping::shipment::Shipment", "python": "Shipments"}]
```

Its view and a `NameRow` alias are exported. A row is a union over the kinds,
`tuple[ShipmentModel, Literal['parcel'], ParcelModel] | tuple[ShipmentModel,
Literal['letter'], None] | ...`, each record the entity's generated model. A
kinded enum's view is a `WritableDisjointView` whose `insert`, `update` and
`change_kind` take the base's and the kinds' generated ActiveModels and a kind
of the enum, and which `pgorm.schema.create_disjoint` takes; an unkinded
enum's is a read-only `DisjointView` with neither. Emission refuses an enum
whose base or kind entity is not exported, and the import check compares each
enum's `describe()` as it compares an entity's.

## Verify the workflow

From the repository root, with Maturin installed in `target/python-dev`:

```sh
target/python-dev/bin/python pgorm-python/tests/with_postgres.py \
  target/python-dev/bin/python pgorm-python/checks/codegen.py
```

The checker creates three fresh installations, compiles a downstream entity
crate (two of whose entities are pgorm-codegen's output for a table of interval
columns and a table of domain columns, so the Rust and Python generators are
checked together), emits twice
to check determinism, executes generated CRUD and graph
queries against PostgreSQL, checks import compatibility failures and runs
valid and invalid consumers through the latest mypy release. It also checks that
scaffolding from a final application wheel omits its previous generated module.
Evidence and the final wheel are written to `target/python-codegen`.
