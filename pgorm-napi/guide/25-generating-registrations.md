# Generating registrations

*This chapter is JavaScript's own: the Rust guide has no counterpart of it.*
[Registered Rust entities](24-registered-entities.md) builds an
application's module by hand. `pgorm-napi/codegen` builds it from a
description of what to register, and writes a TypeScript module typing each
registration from what the built library describes: records keyed by SQL
column with each column's type, nullability and an enum's labels, and graph
rows and source-tuple rows as tuples of those records. It is the JavaScript
counterpart of pgorm-python's [code generator](../../pgorm-python/CODEGEN.md):
a build, then generation. Running queries over the generated module needs
neither.

## Describing the application

The application's entity crate holds ordinary Rust: entities, and functions
building each `SelectGraph` from the aliases JavaScript chooses, whether
written by hand or by `pgorm-codegen` from a schema ([Generating
entities](23-codegen.md)). Beside it, an `application.json` names what to
register:

```json
{
  "schema_version": 1,
  "module": "app",
  "entity_crate": ".",
  "entities": [
    {"name": "app.Account", "rust": "account::Entity", "typescript": "Account"},
    {"name": "app.Note", "rust": "note::Entity", "typescript": "Note"}
  ],
  "graphs": [
    {"name": "app.AccountNotes", "rust": "optional", "typescript": "AccountNotes"}
  ],
  "sources": [
    {"name": "app.AccountWithNote", "rust": ["account::Entity", "note::Entity"], "typescript": "AccountWithNote"}
  ],
  "disjoints": [
    {"name": "app.Shipments", "rust": "shipping::shipment::Shipment", "typescript": "Shipments"}
  ]
}
```

`entity_crate` is relative to the file. A Rust path names modules and an item
of that crate, with no expression, generic or keyword; a graph's `rust` names
the function that builds it. Registration names are unique per kind, each
Rust entity and disjoint enum is registered once, a source tuple lists one to
six of the described entities, and every `typescript` export is a public
JavaScript identifier no other export or the module takes. Anything else is
a `CodegenError` before a file is written.

A disjoint enum's base and kind entities are among the described entities,
and emission refuses one that is not. Its row type is a union over its kinds,
`[ShipmentRecord, "packet", PacketRecord] | [ShipmentRecord, "letter", null] |
..`, which narrows by the kind; a kinded enum is a `DisjointEntity` whose
writes take its entities' ActiveModels and its kinds, and an unkinded one an
`UnkindedDisjointEntity` with no write.

## Scaffold, build, emit

```sh
node pgorm-napi/codegen/cli.js scaffold application.json --pgorm-source "$PGORM_SOURCE" --output app-module
node pgorm-napi/codegen/cli.js build app-module            # --release for an optimised library
node pgorm-napi/codegen/cli.js emit app-module
```

- **scaffold** writes a crate registering the described entities, graphs and
  tuples in its own `#[neon::main]`, depending on the checkout's pgorm-napi
  without its `standalone-module` feature, its lockfile seeded from
  pgorm-napi's so the crates they share resolve to the audited versions; a
  copy of the package's ES module under `lib/`; and the resolved description
  as `application.json`. It refuses a destination that exists.
- **build** compiles the crate and places its library as
  `lib/pgorm_napi.node`, the addon the copied `index.js` loads. Keep the
  project's `Cargo.lock` for reproducible rebuilds.
- **emit** loads the built module and writes `lib/<module>.js` and
  `lib/<module>.d.ts`, laid out as deno fmt and prettier lay them out, within
  80 columns, so they read as written by hand. Emitting again from the same
  library writes the same bytes.

## Using the generated module

The generated module exports each registration under its `typescript` name,
typed. This sketch needs a generated module, which the guide's tests do not
build; [`tests/codegen-entities/tests/consumer.ts`](../tests/codegen-entities/tests/consumer.ts)
is the same use of the repository's fixture, which `node
pgorm-napi/checks/codegen.js` type-checks and runs:

```typescript
import { Account, AccountNotes, AccountWithNote } from "./app-module/lib/app.js";
import { pipeline as pl } from "./app-module/lib/index.js";

const busy = await Account.find().where(Account.col("mood").eq("busy")).all(pool);  // AccountRecord[]
busy[0]?.["display name"];                                                           // string
const saved = await Account.active().set("id", 3).set("display name", "Cy").insert(pool);
for (const [account, note] of await AccountNotes.find({ aliases: ["n"] }).all(pool)) {
  note?.body;                                                                         // NoteRecord | null
}
const pairs = await pl.from(Account).selectSources(AccountWithNote).all(pool);      // [AccountRecord | null, NoteRecord | null][]
```

For each entity the declarations export `NameRecord`, its columns by SQL
name typed as the library decodes them, `| null` where the column is
nullable, an enum's labels as a union, and an array's items `| null` unless
the entity's Rust field is a `Vec` of a type that is no `Option`; and
`NameInput`, what each column takes. The export is typed as an `Entity` over
both, so `col(..)` comparisons, ActiveModel `set`s, records and the version
terminals are all typed. A graph exports `NameRow`, the root's record for a
slotless shape and otherwise a tuple whose `Opt` slots are `| null`; a source
tuple exports `NameRow`, a tuple of `Record | null`. A column of a type the
package has no kind for reads as `PlainValue` and takes a `Value`.

## When the entities change

The generated module records a fingerprint of the addon's version and of
every registration it names, as the library describes them, and checks it as
it loads. A library rebuilt from changed entities, hooks aside, refuses to
load behind declarations it no longer matches, with a `ConstructionError`
asking to regenerate. Rebuild and regenerate whenever the entity crate
changes. The fingerprint covers each disjoint enum's `describe()` too.

## Checking the workflow

```sh
node pgorm-napi/checks/codegen.js
```

The check scaffolds `tests/codegen-entities` under `target/napi-codegen`,
refuses to scaffold over it, builds and emits twice to check that emission is
deterministic, and type-checks the fixture's consumer with `deno check`, its
`@ts-expect-error` lines holding the declarations to what they refuse. It
then loads a copy of the module with another fingerprint in both runtimes and
requires the refusal, and runs the fixture's live suite under `node --test`
and `deno test`. It reads `PGORM_TEST_DSN`, or the server `DATABASE_URL`
names.
