# Registered Rust entities

*This chapter is JavaScript's own: the Rust guide has no counterpart of it.*
An application that already has pgorm entities in Rust can use them from
JavaScript as they are: their `Select<E>`, their models, their ActiveModels
with the application's `ActiveModelBehavior` hooks, the `SelectGraph` shapes
it builds from them, and pgorm's schema generation over them. This is how
JavaScript reaches what a declared model does not have: hooks, an
`ActiveModel`'s set, unchanged and reset columns, `IntoActiveModel`, and
`CREATE TABLE` from an entity.

JavaScript cannot instantiate a Rust generic, so the application registers
its concrete types and builds one native module carrying both the package's
API and its registrations. The package's own module registers nothing; the
models JavaScript declares ([Entities](03-entities.md)) need no registration
at all. Python registers entities the same way, through its own
[application module](../../pgorm-python/guide/24-registrations.md).

## Building the application's module

The application's crate depends on the checkout's `pgorm-napi` without its
default `standalone-module` feature, on the same checkout's `pgorm`, and on
Neon, and builds a library of its own:

```toml
[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
pgorm-napi = { path = "../pgorm/pgorm-napi", default-features = false }
pgorm = { path = "../pgorm" }
neon = { version = "1.1.2", default-features = false, features = ["napi-6"] }
```

Its `#[neon::main]` fills a `Registry` and installs it:

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
    registry.sources::<(account::Entity, note::Entity)>("app.AccountWithNote")?;
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

Registration takes an `E: EntityTrait + Send + Sync + 'static` whose model
converts into its ActiveModel. A name or a Rust entity registered twice, a
name outside 1–255 bytes, and a graph whose source entities are not
registered first are refused with a `RegistrationError`. A graph's factory
receives one alias per joined slot, in tuple order, and joins each slot under
its alias; the root keeps its table's name. Rust's slotless shape and tuples
of one to six `Req` or `Opt` slots are registrable, and so are source tuples
of one to six entity types.

Build the crate's library and copy it, beside `pgorm-napi/lib`'s `.js` and
`.d.ts` files, to `lib/pgorm_napi.node`: that directory is the module the
application imports, its `index.js` loading the application's library
instead of the package's own. [Generating
registrations](25-generating-registrations.md) does all of this from a
description of what to register, and writes TypeScript declarations typing
each registration.

The examples in this chapter run against the repository's application
fixture, `pgorm-napi/tests/application-binding`, whose `app.Account` is an
account with an enum column, a column named `display name`, and hooks, and
whose `app.Note` is a note an account has many of. `node
pgorm-napi/checks/entities.js` builds the fixture's module and runs these
examples against it, in Node.js and Deno, each in an empty database; the
package's own suite, whose module registers nothing, skips them.

## The schema

[`fromEntity`](../lib/schema.d.ts) is pgorm's own `Schema` for a registered
entity: its `table` is `create_table_from_entity`, a `CreateTable`, and its
`enums`, `indexes` and `comments` are `create_enum_from_entity`,
`create_index_from_entity` and `create_comments_from_entity`. Each is a DDL
statement as the [DDL builders](14-schema.md) make one, and nothing runs it
but a terminal. Run the enums first, then the table, then the rest; create
an enum two entities share once, and a table a foreign key references before
the table holding the key:

<!-- example: application -->
```ts
import assert from "node:assert/strict";
import { createSchema, entities, entity, fromEntity } from "@necessary/pgorm";

assert.ok(entities().includes("app.Account"));
const Account = entity("app.Account");
const Note = entity("app.Note");

await pool.execute(createSchema("napi_entities"));
for (const registered of [Account, Note]) {
  const generated = fromEntity(registered);
  for (const statement of generated.enums) await pool.execute(statement);
  await pool.execute(generated.table);
  for (const statement of [...generated.indexes, ...generated.comments]) await pool.execute(statement);
}

assert.match(fromEntity(Account).table.inspect().sql, /^CREATE TABLE "napi_entities"\."accounts"/);
const comment = await pool.one(
  "SELECT obj_description('napi_entities.accounts'::regclass, 'pg_class') AS comment",
);
assert.equal(comment.comment, "Accounts' native schema");
```

A table pgorm refuses is a `ConstructionError` carrying its message: a
`belongs_to` over a column typed as a domain over an enum, on either side, is
one, since PostgreSQL builds no foreign key there (`42804`).

## Reading

`entity(name)` gives the registration. `find()` is `E::find()`, and
`where`, `orderBy`, `limit` and `offset` are `Select<E>`'s; `all`, `one` and
`optional` are its terminals, `one` and `optional` narrowing the limit to one
row as Rust does, and `one` finding nothing a `DecodeError`:

<!-- example: application -->
```ts
import assert from "node:assert/strict";
import { createSchema, entity, fromEntity } from "@necessary/pgorm";

const Account = entity("app.Account");
await pool.execute(createSchema("napi_entities"));
for (const statement of fromEntity(Account).enums) await pool.execute(statement);
await pool.execute(fromEntity(Account).table);
await pool.execute(
  `INSERT INTO napi_entities.accounts VALUES (1, 'Ann', NULL, 1, 'calm'), (2, 'Bob', 'b', 1, 'busy'), (3, 'Cy', NULL, 1, 'calm')`,
);

const calm = Account.find().where(Account.col("mood").eq("calm")).orderBy(Account.col("id").desc());
assert.match(calm.inspect().sql, /WHERE "accounts"\."mood" = CAST\(\$1::text AS napi_entities\.mood\)/);

const accounts = await calm.all(pool);
assert.deepEqual(accounts.map((account) => account["display name"]), ["Cy", "Ann"]);
const first = await calm.optional(pool);
assert.equal(first?.id, 3);
```

A record is a frozen plain object keyed by SQL column name, `display name`
included, as Rust's model serializes it. The module keeps the Rust model
behind it, so `intoActive(record)` is the real `IntoActiveModel`,
`withValue(record, column, value)` is `ModelTrait::set` on a copy, and
`tagged(record, column)` gives a column as a `Value` of its declared kind.

`col(name)` names a column as the entity's `ColumnTrait` does, and its
comparisons are that trait's: a value converts to the column's declared kind
and is written through its `save_as`, so an enum's label is cast to its type
and a `money` column's bound to `money`; `any(..)` and `all(..)` over a list
are the trait's `compare_any` and `compare_all`; `between`,
`isDistinctFrom`, `contains`, the range positions, `isIn` and `isNotIn` are
the trait's too, a list converted value by value. An expression is compared
as written, and `null` is refused.

`describe()` reports the registration's `name`, `schema` and `table`; each
column's SQL `name`, the `jsonKey` its Rust field serializes under,
`sqlType`, `rustType`, `nullable`, `primaryKey`, `kind` and an enum's
`values`; the `primaryKey` and whether it ends `WITHOUT OVERLAPS`; the
`uniqueKeys`; every relation with its columns, period, condition type,
whether an `on_condition` adds a condition, enforcement and deferrability;
and under `rust` the Rust types behind it. These are the keys Python's
`describe()` reports, each spelled in camelCase.

### Counts, pages, cursors, streams, joins and projections

A query has the rest of Rust's `Select<E>` reads. `count(db)` is
`PaginatorTrait::count`, and `inspect("count")` shows its statement.
`paginate(pageSize)` is `PaginatorTrait::paginate`: its paginator holds no
database and no current page, so `fetchPage(db, n)`, `numItems(db)`,
`numPages(db)` and `numItemsAndPages(db)` each take the one they run on, and
`pages(db)` is the loop over every page, as a model query's paginator's is
([Pages, cursors and streams](06-pagination.md)). `cursor(...columns)` is
`Select::cursor_by` over those columns in order, its `after(...values)` and
`before(...values)` taking one value per column, converted and cast as a
comparison's value is, and `inspect()` showing its statement.
`stream(db)` reads the records one at a time, each decoded as
`Select::stream` decodes a row, from a pool's connection of its own or a
connection.

`join(relation, { kind })` is `QuerySelect::join` of a relation the entity's
Rust `Relation` enum declares, named by `entity.relation(name)`; the joined
entity's columns go in the conditions, and the rows stay the query's own
records. `select(...columns)` is `select_only` with `columns`, each read
through its `select_as`: its rows are plain records of those columns, as a
statement's are. Rust decodes such a projection into a type of the
program's own, which JavaScript cannot name at run time:

<!-- example: application -->
```ts
import assert from "node:assert/strict";
import { createSchema, entity, fromEntity } from "@necessary/pgorm";

const Account = entity("app.Account");
const Note = entity("app.Note");
await pool.execute(createSchema("napi_entities"));
for (const registered of [Account, Note]) {
  for (const statement of fromEntity(registered).enums) await pool.execute(statement);
  await pool.execute(fromEntity(registered).table);
}
await pool.execute(
  `INSERT INTO napi_entities.accounts VALUES (1, 'Ann', NULL, 1, 'calm'), (2, 'Bob', NULL, 1, 'busy'), (3, 'Cy', NULL, 1, 'calm')`,
);
await pool.execute(`INSERT INTO napi_entities.notes VALUES (10, 1, 'a1'), (11, 2, 'b1')`);

const byId = Account.find().orderBy(Account.col("id").asc());
assert.equal(await byId.count(pool), 3);
const pages = byId.paginate(2);
assert.deepEqual(await pages.numItemsAndPages(pool), { items: 3, pages: 2 });
assert.deepEqual((await pages.fetchPage(pool, 1)).map((account) => account.id), [3]);

const cursor = Account.find().cursor("mood", "id");
assert.deepEqual((await cursor.first(2).all(pool)).map((account) => account.id), [1, 3]);
assert.deepEqual((await cursor.after("calm", 3).all(pool)).map((account) => account.id), [2]);

const streamed = [];
for await (const account of byId.stream(pool)) streamed.push(account["display name"]);
assert.deepEqual(streamed, ["Ann", "Bob", "Cy"]);

const withB1 = Account.find().join(Account.relation("Note")).where(Note.col("body").eq("b1"));
assert.deepEqual((await withB1.all(pool)).map((account) => account.id), [2]);
assert.deepEqual(await byId.select("display name", "mood").all(pool), [
  { "display name": "Ann", mood: "calm" },
  { "display name": "Bob", mood: "busy" },
  { "display name": "Cy", mood: "calm" },
]);
```

## Writing, hooks included

`active()` is `ActiveModelBehavior::new`, the application's defaults
included. `set`, `notSet` and `reset` return a new ActiveModel, and `get`
reports a column as `notSet`, `set` or `unchanged` with its value. `insert`,
`update` and `delete` are `ActiveModelTrait`'s, with the application's hooks
around them; a hook's refusal is a `ConstructionError` carrying its message.
The fixture's account appends `|before` to a new account's name, bumps its
version on each update, and refuses an account named `reject_before`:

<!-- example: application -->
```ts
import assert from "node:assert/strict";
import { ConstructionError, createSchema, entity, fromEntity } from "@necessary/pgorm";

const Account = entity("app.Account");
await pool.execute(createSchema("napi_entities"));
for (const statement of fromEntity(Account).enums) await pool.execute(statement);
await pool.execute(fromEntity(Account).table);

const draft = Account.active().set("id", 1).set("display name", "Ann");
// `new` set the version and the mood.
assert.equal(draft.get("version").state, "set");

const ann = await draft.insert(pool);
assert.equal(ann["display name"], "Ann|before");
assert.equal(ann.version, 1);

const changed = await Account.intoActive(ann).set("note", "seen").update(pool);
assert.equal(changed.version, 2);
assert.equal(Account.intoActive(changed).get("note").state, "unchanged");

await assert.rejects(
  Account.active().set("id", 2).set("display name", "reject_before").insert(pool),
  (error: unknown) => error instanceof ConstructionError && /before_save refused/.test(error.message),
);
assert.equal(await Account.intoActive(changed).delete(pool), 1);
```

A record or ActiveModel of another registration is an argument of the wrong
type, a `TypeError`.

### Writes that return both versions

The entity's statement builders are reachable too, with the terminals that
return a row's two versions. They are statement terminals in Rust, so no
hook runs around them:

| JavaScript | Rust |
| --- | --- |
| `entity.update(active).returningChange(db)` | `Update::one(active)?.exec_returning_change` |
| `entity.updateMany().set(..).where(..).returningChanges(db)` | `Update::many(entity).col_expr(..).filter(..).exec_returning_changes` |
| `entity.insert(active).onConflict(..).returningUpsert(db)` | `Insert::one(active).on_conflict(..).exec_returning_upsert` |
| `entity.insertMany(actives).onConflict(..).returningUpserts(db)` | `Insert::many(actives).on_conflict(..).exec_returning_upserts` |
| `entity.insert(active).execute(db)` | `Insert::one(active).exec` |
| `entity.insert(active).returningPk(db)` | `Insert::one(active).exec_returning_pk` |
| `entity.insert(active).returningModel(db)` | `Insert::one(active).exec_returning_model` |
| `entity.insertMany(actives).execute(db)` | `Insert::many(actives).exec` |
| `entity.insertMany(actives).returningPks(db)` | `Insert::many(actives).exec_returning_pks` |
| `entity.insertMany(actives).returningModels(db)` | `Insert::many(actives).exec_returning_models` |
| `insert.add(active)`, `insert.addMany(actives)` | `Insert::add`, `Insert::add_many` |

An insert is typed by its row count, as Rust's is: `insert` gives an
`EntityInsert` and `insertMany` an `EntityInsertMany`, and `add` or
`addMany` on either gives the `EntityInsertMany` of its ActiveModels and
theirs, its conflict clause kept. The ActiveModels of one insert set the same
columns; a batch that does not is a `ConstructionError` as it is built. A
change is `{ old, new }`; an upsert `{ kind: "inserted", new }` or `{ kind:
"updated", old, new }`. A row the conflict clause held back is `null` from
`returningUpsert`, a `DecodeError` from `returningPk` and `returningModel`,
and left out of `returningUpserts`, `returningPks` and `returningModels`.
`updateMany` needs `where` or `allRows()`. A key is the value of a one-column
key, or a frozen array of a composite key's in its columns' order.

<!-- example: application -->
```ts
import assert from "node:assert/strict";
import { Conflict, createSchema, entity, fromEntity } from "@necessary/pgorm";

const Account = entity("app.Account");
await pool.execute(createSchema("napi_entities"));
for (const statement of fromEntity(Account).enums) await pool.execute(statement);
await pool.execute(fromEntity(Account).table);

const ids = await Account.insertMany([
  Account.active().set("id", 1).set("display name", "Ann"),
  Account.active().set("id", 2).set("display name", "Bob"),
]).returningPks(pool);
assert.deepEqual(ids, [1, 2]);

const raised = await Account.updateMany()
  .set("version", 5)
  .where(Account.col("id").gte(2))
  .returningChanges(pool);
assert.deepEqual(raised.map(({ old, new: now }) => [old.version, now.version]), [[1, 5]]);

const upserted = await Account.insert(Account.active().set("id", 1).set("display name", "Anne"))
  .onConflict(Conflict.on("id").update("display name"))
  .returningUpsert(pool);
assert.equal(upserted?.kind, "updated");
assert.equal(upserted?.new["display name"], "Anne");
```

## Graphs

A registered graph is the application's `SelectGraph`, each slot under the
alias JavaScript gives or `g1`, `g2`, .. by default. Rust writes the
projection and decodes each row through `GraphRow`: a slotless graph's row
is the root's record, otherwise a tuple of every source's, an `Opt` slot
`null` where it matched nothing. `col(source, column)` qualifies a column as
the query names it, `0` being the root. `cursor(...columns)` is
`SelectGraph::cursor_by` on up to six root columns, and
`cursorOn(source, ...columns)` `cursor_by_on` on up to six columns of a slot,
the first slot being `1`: `before` and `after` bound the order columns, and
`beforeWith` and `afterWith` those or the whole key, the order columns, the
ordered source's other key columns, then each other source's, the root's
first; `inspect()` shows a cursor's statement. `stream(db)`, `count(db)` and
`paginate(pageSize)` read, count and page the graph's rows as an entity's
query does, and `allGrouped(db)` is `all_grouped`, which Rust has only on a
graph of one `Opt` slot: each root once with an array of its slot's
records:

<!-- example: application -->
```ts
import assert from "node:assert/strict";
import { createSchema, entity, type EntityRecord, fromEntity, graph } from "@necessary/pgorm";

await pool.execute(createSchema("napi_entities"));
for (const registered of [entity("app.Account"), entity("app.Note")]) {
  for (const statement of fromEntity(registered).enums) await pool.execute(statement);
  await pool.execute(fromEntity(registered).table);
}
await pool.execute(`INSERT INTO napi_entities.accounts VALUES (1, 'Ann', NULL, 1, 'calm'), (2, 'Bob', NULL, 1, 'busy')`);
await pool.execute(`INSERT INTO napi_entities.notes VALUES (10, 1, 'a1'), (11, 1, 'a2')`);

const notes = graph("app.AccountNotes").find({ aliases: ["n"] });
// Without generated declarations, a row is typed as any graph's might be.
const rows = (await notes.orderBy(notes.col(0, "id").asc(), notes.col(1, "id").asc()).all(pool)) as [
  EntityRecord,
  EntityRecord | null,
][];
assert.deepEqual(
  rows.map(([account, note]) => [account["display name"], note?.body ?? null]),
  [["Ann", "a1"], ["Ann", "a2"], ["Bob", null]],
);

const page = await notes.cursor("id").first(2).all(pool);
assert.equal(page.length, 2);

const byBody = await notes.cursorOn(1, "body").after("a1").all(pool);
assert.deepEqual(byBody.map((row) => (row as [EntityRecord, EntityRecord | null])[1]?.body), ["a2"]);

const grouped = await graph("app.AccountNotes").find().allGrouped(pool);
assert.deepEqual(grouped.map(([account, list]) => [account.id, list.length]), [[1, 2], [2, 0]]);
assert.equal(await notes.count(pool), 3);
```

Without declarations of its own, the module types a graph's row as any
graph's might be, a record or a tuple; [Generating
registrations](25-generating-registrations.md) writes declarations that type
each registered graph's rows, and each entity's records, exactly.

A graph's `describe()` reports its `name`, its `rustShape`, the Rust
`SelectGraph` type, and its `sources`, the root and then each slot, each with
its `index`, the `entity` it decodes as and its `slot` (`root`, `Req` or
`Opt`).

## Pipelines over registered entities

A registered entity is a [pipeline](12-pipeline.md) source wherever a
relation is, read through its own `IntoSource`. A registered tuple of one to
six entity types is a pipeline's last stage, `selectSources`, its rows
decoded into the entities' models as pgorm's `select_sources` decodes them,
each source projected under the qualifier given for it or its table's name,
and one the join left empty `null`:

<!-- example: application -->
```ts
import assert from "node:assert/strict";
import { createSchema, entity, fromEntity, pipeline as pl } from "@necessary/pgorm";

const Account = entity("app.Account");
const Note = entity("app.Note");
await pool.execute(createSchema("napi_entities"));
for (const registered of [Account, Note]) {
  for (const statement of fromEntity(registered).enums) await pool.execute(statement);
  await pool.execute(fromEntity(registered).table);
}
await pool.execute(`INSERT INTO napi_entities.accounts VALUES (1, 'Ann', NULL, 1, 'calm'), (2, 'Bob', NULL, 1, 'busy')`);
await pool.execute(`INSERT INTO napi_entities.notes VALUES (10, 1, 'a1')`);

const rows = await pl.from(Account)
  .join(pl.source(Note).named("n"), pl.col("accounts", "id").eq(pl.col("n", "account_id")), { kind: "left" })
  .sort(pl.col("accounts", Account.col("id")))
  .selectSources(pl.sources("app.AccountWithNote"), { qualifiers: ["accounts", "n"] })
  .all(pool);
assert.deepEqual(
  rows.map(([account, note]) => [account?.["display name"], note?.body ?? null]),
  [["Ann", "a1"], ["Bob", null]],
);
```

A registered column a pipeline reads is `pl.col(qualifier,
Entity.col(name))`: the entity's own column under the qualifier its relation
is read by, compared as the entity types it, so a column whose domain is over
an enum compares as the enum. `pl.col(qualifier, "name")` only qualifies. A
pipeline reshaped before `selectSources`, by a `select` or an aggregate, is
refused with the `ConstructionError` pgorm's refusal names, before anything
is sent. A source tuple's `describe()` reports its `name`, its `rustShape`
and its `entities` in order.

## Disjoint enums and one-of references

A `#[derive(Disjoint)]` enum registers beside its entities, its base and each
kind's entity registered first, and a `#[derive(OneOf)]` enum once its entity
and its targets' are:

```rust
registry.entity::<shipment::Entity>("app.Shipment")?;
registry.entity::<packet::Entity>("app.Packet")?;
registry.entity::<pallet::Entity>("app.Pallet")?;
registry.disjoint::<shipment::Shipment>("app.Shipments")?;
registry.entity::<label::Entity>("app.Label")?;
registry.one_of::<label::Labelled>("app.Labelled")?;
```

`disjointEntity(name)` gives a registered disjoint enum, and
`disjointEntities()` lists them. Its reads are a disjoint model's
([Disjoint enums](15-disjoint.md)), each row the base entity's record, the
kind, and the kind entity's record or `null`; its writes take the entities'
ActiveModels, and `createDisjoint` its schema:

<!-- example: application -->
```ts
import assert from "node:assert/strict";
import { createDisjoint, createSchema, disjointEntity, entity } from "@necessary/pgorm";

const Shipments = disjointEntity("app.Shipments");
await pool.execute(createSchema("napi_shipments"));
for (const statement of createDisjoint(Shipments)) await pool.execute(statement);

const [base, kind, packet] = await Shipments.insert(
  entity("app.Shipment").active().set("label", "box"),
  "packet",
  entity("app.Packet").active().set("weight_g", 500),
).returningModel(pool);
assert.equal(kind, "packet");
assert.equal(base.label, "box");
assert.equal(packet?.weight_g, 500);
```

The kind column set, and a kind row's key set, are refused with a
`ConstructionError` before anything is sent; an ActiveModel of another
entity given for the base or a kind is a `TypeError`. An enum declared
`#[pgorm(base = .., unkinded)]` registers the same way and is read-only, as
its Rust declaration is: an `UnkindedDisjointEntity`, whose writes and
`createDisjoint` refuse with a `ConstructionError`.

`oneOfEntity(name)` gives a registered one-of enum, its targets named as its
variants in snake_case, and its `set(active, value, key)` writes the
entity's ActiveModel; `createOneOf` gives its `CHECK` ([One-of
references](16-one-of.md)).

## Where an operation runs

Every operation on a registration runs on a pool, a connection or a
transaction, with what a statement of SQL text gets: a busy connection
refuses it, and an `AbortSignal` aborts it and discards the connection whose
state it left unknown ([Promises, cancellation and cleanup](02-promises.md)).
