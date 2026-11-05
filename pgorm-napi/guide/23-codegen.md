# Generating entities

A database that already exists has its schema written down as DDL: the
`CREATE TABLE` and `CREATE TYPE` statements that make it. In Rust,
`pgorm-codegen` reads that DDL and writes the entity modules that describe
it, one file per table, in the form the Rust guide's Entities chapter
teaches. This chapter is the JavaScript dialect of the [Rust guide's
chapter](../../docs/guide/22-codegen.md) of the same name.

**Not available in JavaScript yet:** generating declarations from a schema.
Rust's `entities_from_sql` reads DDL with libpg_query, PostgreSQL 18's own
parser, and returns a file per table holding its `Model` and `Relation`, with
`mod.rs`, `prelude.rs`, and a file each for the enums and the range types the
tables use, each laid out as a person writes Rust so nothing needs to run over
it afterwards; `EntityWriterOptions` adds serde derives, extra derives and
attributes, a default schema and unkinded disjoint table sets; it refuses
whatever DDL the entity model cannot hold, such as a `pg_dump` preamble or a
`point` column, with an error whose message names it; it reads the tables `Schema::create_disjoint`
writes back as a disjoint enum, and a one-of `CHECK` as a one-of enum; and its
three stages, `parse_schema`, `EntityTransformer::transform` and
`EntityWriter::generate`, can be called one at a time. JavaScript has no
generator of `model(..)` declarations from DDL, nor any of these.

Two things stand nearest. A JavaScript program can use entities Rust
generated: `pgorm-codegen`'s output is ordinary Rust entities, which an
application registers in a module of its own and reaches from JavaScript
([Registered Rust entities](24-registered-entities.md)), and
`pgorm-napi/codegen` then writes that module and a TypeScript declaration of
every registration from what the built library describes ([Generating
registrations](25-generating-registrations.md)). Or it declares its models by
hand, as [Entities](03-entities.md) does, and lets the first read hold each
declaration to the table it names, as the sections below show.

## Declarations written by hand

A model declares each column's kind, and every read checks the result against
the declaration as it decodes: the result's columns are exactly the ones
selected, each of its declared kind, with no NULL where the field is not
nullable and no label an enum field does not list. A declaration that does
not match its table is a `DecodeError` at the first read, never a value read
wrongly. Here the bakery's `order_line` table is declared with its quantity as
an `i64`, where the column is an `integer`:

```ts
import assert from "node:assert/strict";
import { column, DecodeError, model } from "@necessary/pgorm";

const Mistaken = model("order_line", {
  columns: {
    id: column("i32", { primaryKey: true, default: true }),
    orderId: column("i32", { name: "order_id" }),
    cakeId: column("i32", { name: "cake_id" }),
    quantity: column("i64"),
    price: column("decimal"),
  },
});
await assert.rejects(Mistaken.find().all(pool), DecodeError);

// The bakery's own declaration matches the table.
const lines = await OrderLine.find().all(pool);
assert.equal(lines.length, 3);
```

So a test that reads one row of each model, against a database built from
the same DDL the program deploys, holds every declaration to its table, as
Rust's generated entities are held to theirs by being written from it.

`describe()` gives a declaration back as data, the kinds spelt as a result
spells its columns' kinds, which is how such a test can report what it
compared:

```ts
import assert from "node:assert/strict";

const { table, primaryKey, fields } = Cake.describe();
assert.equal(table, "cake");
assert.deepEqual(primaryKey, ["id"]);
assert.equal(fields.glutenFree?.column, "gluten_free");
assert.equal(fields.price?.kind, "decimal");
assert.equal(fields.flavor?.kind, `enum "public"."flavor"`);
```

## Types

Each column of the DDL is declared with the kind [Entities](03-entities.md)
maps it to, the other way round, as Rust's generator maps each column to a
field type:

| Column type | Declared as |
| --- | --- |
| `varchar`, `text`, `char(n)` | `column("text")` |
| `smallint`, `integer`, `bigint` | `column("i16")`, `column("i32")`, `column("i64")` |
| `real`, `double precision` | `column("f32")`, `column("f64")` |
| `numeric`, `decimal` | `column("decimal")` |
| `boolean` | `column("bool")` |
| `date`, `time`, `timestamp` | `column("date")`, `column("time")`, `column("datetime")` |
| `timestamptz` | `column("datetime_utc")` |
| `uuid`, `json`, `jsonb`, `bytea` | `column("uuid")`, `column("json")`, `column("bytes")` |
| an enum | `column(new TypeName(name, { schema }), { values: [..] })` |
| a built-in range | `column("int4range")` and the others, named as PostgreSQL names them |
| a nullable column | `{ nullable: true }` |
| an array | `{ array: true }` |
| a `serial` key, or a column with a default | `{ default: true }` |
| an identity column | `{ generated: "always" }` or `{ generated: "byDefault" }` |
| a domain | the kind of the type the domain is over |

A foreign key is a relation, `belongsTo` on the table holding it and
`hasMany` or `hasOne` on the table it points to ([Relations](07-relations.md)).
[Values and types](13-values.md) covers each kind's values.
