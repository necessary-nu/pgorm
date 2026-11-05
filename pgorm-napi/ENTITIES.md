# Registered application entities

An application that already has pgorm entities in Rust can use them from
JavaScript as they are: their `Select<E>`, their models, their ActiveModels
with the application's `ActiveModelBehavior` hooks, and the `SelectGraph`
shapes it builds from them. JavaScript cannot instantiate a Rust generic, so
the application registers its concrete types and builds one native module
carrying both the binding's API and its registrations, as pgorm-python's
[application module](../pgorm-python/ENTITIES.md) does. The binding's own
module registers nothing; the models JavaScript declares
([README](README.md#models)) need no registration at all.

## Build the application's module

The application crate depends on this checkout's `pgorm-napi` without its
default `standalone-module` feature, the same checkout's `pgorm`, and Neon,
and exports its own `#[neon::main]`:

```toml
[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
pgorm-napi = { path = "../pgorm/pgorm-napi", default-features = false }
pgorm = { path = "../pgorm" }
neon = { version = "1.1.2", default-features = false, features = ["napi-6"] }
```

```rust
use neon::prelude::*;
use pgorm::{EntityTrait, RelationTrait, pgorm_query::Name};
use pgorm_napi::{RegistrationError, Registry};
use my_entities::{account, note};

fn registry() -> Result<Registry, RegistrationError> {
    let mut registry = Registry::default();
    registry.entity::<account::Entity>("app.Account")?;
    registry.entity::<note::Entity>("app.Note")?;
    registry.graph("app.AccountNotes", |aliases| {
        account::Entity::graph()
            .join_maybe_as::<note::Entity>(account::Relation::Note.def(), Name::runtime(&aliases[0]))
    })?;
    Ok(registry)
}

#[neon::main]
fn main(mut cx: ModuleContext) -> NeonResult<()> {
    match registry() {
        Ok(registry) => pgorm_napi::install(&mut cx, registry),
        Err(error) => cx.throw_error(error.to_string()),
    }
}
```

Registration takes `E: EntityTrait + Send + Sync + 'static` whose model
converts into its ActiveModel. A name or a Rust entity registered twice, a
name outside 1–255 bytes, and a graph whose source entities are not
registered first are refused with a `RegistrationError`. A graph's factory
receives one alias per joined slot, in tuple order, and joins each slot under
its alias; the root keeps its table's name. Rust's slotless shape and tuples
of one to six `Req` or `Opt` slots are registrable.

Build the crate's library and copy it, beside `pgorm-napi/lib`'s `.js` and
`.d.ts` files, to `lib/pgorm_napi.node`: that directory is the module the
application imports, its `index.js` loading the application's library
instead of the binding's own.

The [application fixture](tests/application-binding) and
[`checks/entities.js`](checks/entities.js) are the executable example. The
check runs the fixture's Rust tests — its registry, and the parity of its
entities' statements with `tests/parity.json` — builds its library,
materializes the module under `target/napi-entities`, and runs its suite
there under `node --test` and `deno test`:

```sh
node pgorm-napi/checks/entities.js
```

It reads `PGORM_TEST_DSN`, or the server `DATABASE_URL` names, and makes and
drops a database of its own.

## Read

```js
import { entities, entity } from "./lib/index.js";

entities();                                  // ["app.Account", "app.Note"]
const Account = entity("app.Account");
const query = Account.find().where(Account.col("id").gte(10)).orderBy(Account.col("id").desc()).limit(20);
query.inspect().sql;                         // the SQL `Select<E>` builds
const accounts = await query.all(pool);      // Select::all
const first = await query.optional(pool);    // Select::one_opt, LIMIT 1
```

- `find()` is `E::find()`; `where`, `orderBy`, `limit` and `offset` are
  `Select<E>`'s; `all`, `one` and `optional` are its terminals, `one` and
  `optional` narrowing the limit to one row as Rust does (a limit of zero stays
  zero and finds no row), and `one` finding nothing a `DecodeError`.
- `col(name)` names a column as the entity's `ColumnTrait` does. Its
  comparisons are that trait's: a value converts to the column's declared
  kind and is written through its `save_as`, so an enum's label is cast to
  its type; `any(..)` or `all(..)` over a list is the trait's `compare_any`
  / `compare_all`, the list converted to one array of the column's kind and
  cast as `eq_any`'s is; an expression, and a quantifier over a `Select` or
  an expression, are compared as written; `null` is refused. `between` and
  `notBetween`, with `{ symmetric }` or without, `isDistinctFrom` and
  `isNotDistinctFrom`, and `contains`, `containedBy`, `overlaps` and the
  range positions are the trait's too, their values converted and cast the
  same way, so a `money` column's bounds are cast to `money` and a created
  range is written as its text cast to the type; a bound that is an
  expression takes the whole test as written, an element `contains` tests
  among them. Its
  `isIn` and `isNotIn` are the trait's too, over a list of values converted
  the same way or over a `Select`'s rows (`in_subquery`), so a column whose
  domain is over an enum is a member as the enum, which is the only way
  PostgreSQL compares one; a list holding an expression is a member as
  written.
- A record is a frozen plain object keyed by SQL column name. The module
  keeps the Rust model behind it, so `Account.intoActive(record)` is the real
  `IntoActiveModel`, `Account.withValue(record, column, value)` is
  `ModelTrait::set` on a copy, and `Account.tagged(record, column)` gives a
  column as a `Value` of its declared kind.
- `describe()` reports the registration's `name`, `schema` and `table`; its
  `columns`, each one's SQL `name`, the `jsonKey` its Rust field serializes
  under, `sqlType`, `rustType`, `nullable`, `primaryKey`, `kind` and an enum's
  `values`; the `primaryKey` and whether it ends `WITHOUT OVERLAPS`
  (`primaryKeyWithoutOverlaps`); every relation with its columns, period,
  condition type (`all` or `any`), whether an `on_condition` adds a condition
  of its own, enforcement and deferrability; and under `rust` the Rust types
  behind it. These are the keys Python's `describe()` reports, each spelled in
  camelCase. A column whose type is a
  domain has the kind of the type the domain is over, its values travelling
  as that type's do.

Every operation runs on a pool, a connection or a transaction, with what a
statement of SQL text gets: a busy connection refuses it, an `AbortSignal`
aborts it and discards the connection whose state it left unknown.

## Write

```js
const ann = await Account.active().set("id", 1).set("display name", "Ann").insert(pool);
const changed = await Account.intoActive(ann).set("note", "seen").update(pool);
await Account.intoActive(changed).delete(pool);
```

`active()` is `ActiveModelBehavior::new`, its defaults included.
`get(column)` reports `notSet`, `set` or `unchanged` with the value;
`set`, `notSet` and `reset` return a new ActiveModel. `insert`, `update` and
`delete` are `ActiveModelTrait`'s, the application's hooks around them; a
hook's refusal is a `ConstructionError`. A record or ActiveModel of another
registration is refused.

## Writes that return both versions

```js
const { old, new: now } = await Account.update(Account.intoActive(ann).set("note", "x")).returningChange(pool);
const changes = await Account.updateMany().set("note", "bulk").where(Account.col("id").gte(2)).returningChanges(pool);
const upserted = await Account.insert(active).onConflict(Conflict.on("id").update("display name")).returningUpsert(pool);
const each = await Account.insertMany([a, b]).onConflict(renamed).returningUpserts(pool);
```

| JavaScript | Rust |
| --- | --- |
| `entity.update(active).returningChange(db)` | `Update::one(active)?.exec_returning_change` |
| `entity.updateMany().set(..).where(..).returningChanges(db)` | `Update::many(entity).col_expr(..).filter(..).exec_returning_changes` |
| `entity.insert(active).onConflict(..).returningUpsert(db)` | `Insert::one(active).on_conflict(..).exec_returning_upsert` |
| `entity.insertMany(actives).onConflict(..).returningUpserts(db)` | `Insert::many(actives).on_conflict(..).exec_returning_upserts` |

These are statement terminals in Rust, so no hook runs around them. A change
is `{ old, new }`; an upsert `{ kind: "inserted", new }` or `{ kind:
"updated", old, new }`; a row a conflict clause held back is `null` from
`returningUpsert` and left out of `returningUpserts`. `updateMany` needs
`where` or `allRows()`.

## Schema

```js
import { entity, fromEntity } from "./lib/index.js";

const generated = fromEntity(entity("app.Account"));
for (const statement of generated.enums) await pool.execute(statement);
await pool.execute(generated.table);
for (const statement of [...generated.indexes, ...generated.comments]) await pool.execute(statement);
```

`fromEntity(entity)` is pgorm's own `Schema` for the registered entity: an
`EntitySchema` whose `table` is `create_table_from_entity`, a `CreateTable`,
and whose `enums`, `indexes` and `comments` are `create_enum_from_entity`,
`create_index_from_entity` and `create_comments_from_entity`, as
`CreateType`s, `CreateIndex`es and `DDL`s. Each is a DDL statement as the
module's builders make one, and nothing runs it but a terminal. Run the enums
first, then the table, then the rest; create an enum two entities share once,
and a table a foreign key references before the table holding the key. A
table pgorm refuses is a `ConstructionError` carrying its message: a
`belongs_to` over a column typed as a domain over an enum, on either side, is
one, since PostgreSQL builds no foreign key there (`42804`).

## Graphs

```js
import { graph } from "./lib/index.js";

const notes = graph("app.AccountNotes").find({ aliases: ["n"] });
const rows = await notes.where(notes.col(1, "body").eq("x")).all(pool);  // [account, note | null][]
const page = await notes.cursor("id").afterWith(1, 10).first(20).all(pool);
```

The application's factory builds the `SelectGraph`, each slot under the alias
given or `g1`, `g2`, .. by default. Rust writes the projection and decodes
each row through `GraphRow`: a slotless graph's row is the root's record,
otherwise a tuple of every source's, an `Opt` slot `null` where it matched
nothing. `col(source, column)` qualifies a column as the query names it.
`cursor(column)` is `SelectGraph::cursor_by` on a root column: `before` and
`after` bound that column, `beforeWith` and `afterWith` the whole key — the
column, the root's other key columns, then each slot's. A graph's
`describe()` reports its `name`, its `rustShape`, the Rust `SelectGraph`
type, and its `sources`, the root and then each slot, each with its `index`,
the `entity` it decodes as and its `slot` (`root`, `Req` or `Opt`): the keys
Python's reports, each spelled in camelCase.

## Pipelines over registered entities

A registered entity is a pipeline source wherever a relation is, read through
its own `IntoSource`. A registered tuple of one to six entity types —
`registry.sources::<(account::Entity, note::Entity)>("app.AccountWithNote")` —
is a pipeline's last stage, its rows decoded into the entities' models as
pgorm's `select_sources` decodes them:

```js
import { entity, pipeline as pl } from "./lib/index.js";

const Account = entity("app.Account");
const rows = await pl.from(Account)
  .join(pl.source(entity("app.Note")).named("n"), pl.col("accounts", "id").eq(pl.col("n", "account_id")), { kind: "left" })
  .selectSources(pl.sources("app.AccountWithNote"), { qualifiers: ["accounts", "n"] })
  .all(pool);                                 // [account | null, note | null][]
```

A registered column a pipeline reads is `pl.col(qualifier, Entity.col(name))`:
the entity's own column under the qualifier its relation is read by — its
table's name, or the name of a `pl.source(entity).named(..)` — compared as the
entity types it, so a column whose domain is over an enum compares as the enum.
`pl.col(qualifier, "name")` only qualifies, and such a column needs
`.comparesAs(new TypeName(name, { schema }))` to be compared at all, as a
`money` column needs `.comparesAs("money")` to be compared with a `Decimal`.

Each source is projected under the qualifier given for it, or its table's
name. A row's source that the join left empty is `null`, read through the
entity's absence witness. A pipeline reshaped before the selection — a
`select`, an aggregate — is refused with the `ConstructionError` pgorm's
refusal names, before anything is sent. A source tuple's `describe()` reports
its `name`, its `rustShape`, the Rust tuple of entity types, and its
`entities` in order, the keys Python's reports.

## Disjoint enums

A `#[derive(Disjoint)]` enum registers beside its entities, its base and each
kind's entity registered first:

```rust
registry.entity::<shipment::Entity>("app.Shipment")?;
registry.entity::<packet::Entity>("app.Packet")?;
registry.disjoint::<shipment::Shipment>("app.Shipments")?;
```

`disjointEntity(name)` gives the registration and `disjointEntities()` lists
them. Its reads are a disjoint model's (`README.md`), each row the base
entity's record, the kind and the kind entity's record or `null`; its writes
take the entities' ActiveModels:

```ts
const Shipments = disjointEntity("app.Shipments");
const [base, kind, packet] = await Shipments.insert(
  entity("app.Shipment").active().set("label", "box"),
  "packet",
  entity("app.Packet").active().set("weight_g", 500),
).returningModel(pool);
```

Its statements are `DisjointShape::of` the enum's, which the enum's own
`find`, `insert` and `Schema::create_disjoint` build theirs from, and
`createDisjoint(Shipments)` gives its schema.

An enum declared `#[pgorm(base = .., unkinded)]` registers the same way and is
read-only, as its Rust declaration is: typed as an `UnkindedDisjointEntity`,
its kinds its variants' names in snake_case and its `describe()` without a
kind field or kind type, each write and `createDisjoint` refusing with a
`ConstructionError`.

