# Generating entities

A database that already exists has its schema written down as DDL: the
`CREATE TABLE` and `CREATE TYPE` statements that make it. In Rust,
`pgorm-codegen` reads that DDL and writes the entity modules that describe
it, one file per table. It is a library with no command-line tool: a build
script or a small binary calls it and writes the files where the program
keeps its entities. It reads the DDL with libpg_query, PostgreSQL's own
parser, as PostgreSQL 18 reads it, and connects to no database.

Here Python differs from Rust: pgorm-codegen is not available in Python yet.
The package cannot read DDL and write the `pgorm.Model` declarations of
[Models and entities](03-entities.md), and nothing reads a live database's
catalogue to write them either; pgorm-codegen itself reads DDL text, not a
database. A Python model is declared by hand.

What a Python application can do is generate Rust entities with
pgorm-codegen, then use them from Python by registering them. `pgorm.codegen`,
a different generator that [Registering Rust entities](24-registrations.md)
covers, takes the Rust entities and writes the registration crate and a
module of typed Python views over them. Together they turn DDL into typed
Python models in two steps, with Rust in between:

1. pgorm-codegen reads the DDL and writes the Rust entities of a crate of
   their own.
2. `pgorm.codegen` registers that crate's entities into an application wheel
   and generates `CakeModel`, `CakeActive` and `Cake` for each.

This chapter follows the Rust chapter, saying at each step what the Rust
tool does and what it means for the Python side. [The Rust guide's
Generating entities](../../docs/guide/22-codegen.md) has every detail of
pgorm-codegen itself.

## Adding the crate

`pgorm-codegen` lives in pgorm's repository and is depended on the same way
as pgorm. Generation happens when the schema changes, not when the program
runs, so it is a dependency of a small tool of your own, or a
build-dependency:

```toml
[dependencies]
pgorm-codegen = { git = "https://github.com/necessary-nu/pgorm" }
```

## From DDL to entity files

`entities_from_sql` takes the DDL text and an `EntityWriterOptions`, and
returns the files to write, each a `name` and its `content`:

- one `<table>.rs` per table, holding its `Model` and `Relation`;
- `mod.rs`, declaring each module, or `lib.rs` instead when the `lib` option
  is set, for a crate of entities alone;
- `prelude.rs`, which re-exports each `Entity` under the table's name in
  CamelCase;
- `pgorm_active_enums.rs`, one `ActiveEnum` per enum type the tables use;
- `pgorm_range_types.rs`, one `CreatedRange` newtype per range type the
  schema creates.

For a Python application, generate a crate of entities alone, with the `lib`
option, so that the registration build can depend on it. A small binary of
your own, outside that crate, reads the schema and writes the files into
the crate's `src`:

```rust
use pgorm_codegen::{EntityWriterOptions, entities_from_sql};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ddl = std::fs::read_to_string("schema.sql")?;
    let options = EntityWriterOptions {
        lib: true,
        ..Default::default()
    };
    let output = entities_from_sql(&ddl, options)?;
    let out = std::path::Path::new("bakery-entities/src");
    std::fs::create_dir_all(out)?;
    for file in output.files {
        std::fs::write(out.join(&file.name), file.content)?;
    }
    Ok(())
}
```

The entity crate's `Cargo.toml` depends on pgorm, as any crate of entities
does. Each run writes every file afresh, `lib.rs` and `prelude.rs`
included, so keep hand-written code out of the generated directory. The
files are Rust laid out as [the Rust guide's entities](../../docs/guide/02-entities.md)
are written, so nothing needs to run over them once they are written.

## Writing the DDL from Python

pgorm-codegen reads `CREATE` statements, each table's keys inside it. A
Python program that keeps its schema as `pgorm.schema` builders, as
[`bakery.py`](bakery.py) does, already writes DDL of that shape: render each
statement and write them, in order, to the file the Rust tool reads.

```python
import bakery

ddl = "".join(statement.inspect().sql + ";\n" for statement in bakery.statements())

assert ddl.startswith("CREATE TYPE \"flavor\" AS ENUM ('chocolate', 'vanilla', 'lemon', 'carrot');\n")
assert (
    'CREATE TABLE "cake" ( "id" serial NOT NULL, "name" varchar NOT NULL, '
    '"flavor" flavor NOT NULL, "price" decimal(10, 2) NOT NULL, "gluten_free" bool NOT NULL, '
    '"bakery_id" integer NOT NULL, PRIMARY KEY ("id"), '
    'CONSTRAINT "fk-cake-bakery_id" FOREIGN KEY ("bakery_id") REFERENCES "bakery" ("id") );\n'
) in ddl
assert ddl.count("CREATE TABLE") == 7
```

The cake's statement is the one Rust's schema generation writes for the Rust
guide's cake entity, character for character, which is the DDL the Rust
chapter's round trip reads back as that entity.

The output of `pg_dump --schema-only` is not that shape: it opens with
`SET`s, changes owners, and adds its keys with `ALTER TABLE` after the
tables, and pgorm-codegen refuses it.

## Options

`EntityWriterOptions` names every option, and its `Default` asks for none.
Most shape only the Rust source: serde derives (`with_serde` and the two
`serde_skip_*` options), `Copy` on enums (`with_copy_enums`), and extra
derives and attributes (`model_extra_derives`, `model_extra_attributes`,
`enum_extra_derives`, `enum_extra_attributes`); a registration reads the
entity's columns and their types, which these leave as they are. `lib`
writes the `lib.rs` a crate of entities needs.
`schema_name` sets the schema an unqualified table is in, which the
registered entity's `describe()` then reports as its `schema`.
`unkinded_disjoint` names table sets to read as an unkinded [disjoint
enum](15-disjoint.md). An option that is not valid Rust is refused before
the schema is read.

## Types

pgorm-codegen gives each column the Rust type [the Rust guide's
Entities](../../docs/guide/02-entities.md) maps it to, and a typed Python
view reads that Rust type as the Python type its values decode as:

| Column | Rust field | Python property |
| --- | --- | --- |
| `varchar`, `text` | `String` | `str` |
| `integer`, `bigint` | `i32`, `i64` | `int` |
| `boolean` | `bool` | `bool` |
| `numeric` | `Decimal` | `Decimal` |
| `date` | `Date` | `date` |
| `timestamptz` | `DateTimeWithTimeZone` | `datetime` |
| `interval` | `Interval` | `pgorm.Interval` |
| `uuid` | `Uuid` | `UUID` |
| an enum type | its `ActiveEnum` | `str` |
| a nullable column | `Option<T>` | `T \| None` |
| an array | `Vec<T>` | `list[T]` |
| a domain | its base's type | its base's type |
| a range | `Range`, or a `CreatedRange` newtype | `Any` |

Where the field's type would not give the column's own type back, the Rust
file states a `column_type`, as for a `numeric(10, 2)`; the registration's
`describe()` reports it as the column's `sql_type`, and its `kind` is what
the column's values bind as. [Registering Rust
entities](24-registrations.md#the-generated-typed-views) lists every Rust
type the views know; a type outside them reads as `Any`.

## What it refuses

pgorm-codegen reads the DDL in full or not at all. It reads what the entity
model can hold: `CREATE TABLE` with its columns, keys, foreign keys, defaults
and generation expressions; `ALTER TABLE` adding a column or a constraint to
a table the text creates; enum and range types; domains; indexes; schemas;
and comments on tables and columns. Anything else, from a `CREATE TRIGGER`
to a `CHECK` on a column, or a column of a type the entity model lacks such
as `point`, is refused with a `TransformError` naming it and the statement
it is in, never skipped. A schema that holds such things keeps them in DDL
of its own, outside the file pgorm-codegen reads.

## From entities and back

Schema generation turns an entity into DDL, and pgorm-codegen turns DDL into
an entity, so each undoes the other. On the Python side,
`pgorm.schema.from_entity` writes a registered Rust entity's DDL
([Registering Rust entities](24-registrations.md#schema-from-a-registered-entity)),
which pgorm-codegen reads back as the same entity. A Python model has no
such pair: neither direction is generated, and its DDL is written with the
builders.

## Disjoint enums and one-of references

The tables Rust's `Schema::create_disjoint` writes for a [disjoint
enum](15-disjoint.md) generate as that enum again: the base's module holds
the `#[derive(Disjoint)]` enum beside the base's `Model`, and each kind's
table is an entity of its own. A table's `CHECK` that a [one-of
reference](16-one-of.md) writes generates its `#[derive(OneOf)]` enum the
same way. `pgorm.schema.create_disjoint` writes a Python disjoint model's
tables with the same code, pgorm's `DisjointShape`.

On the Python side, `pgorm.codegen` registers a generated disjoint enum
under `disjoints` and gives it a typed view. It does not describe one-of
enums yet; a wheel that registers one is built by hand.

## The stages

`entities_from_sql` runs three public stages that Rust can call one at a
time: `parse_schema` reads the text into pgorm-query's `CREATE TABLE`
statements, `EntityTransformer::transform` builds the entities from them,
and `EntityWriter::generate` writes the files. They are Rust's alone; none
is available in Python.

## From generated entities to Python

Once the entity crate exists, describe its entities in an `application.json`
beside its `Cargo.toml`, with each table's module as its Rust path, the
entity crate being a crate of entities alone:

```json
{
  "schema_version": 1,
  "module": "bakery_entities",
  "entity_crate": ".",
  "entities": [
    {"name": "bakery_entities.Bakery", "rust": "bakery::Entity", "python": "Bakery"},
    {"name": "bakery_entities.Cake", "rust": "cake::Entity", "python": "Cake"}
  ]
}
```

and build it as [Registering Rust
entities](24-registrations.md#building-an-application-wheel-with-pgormcodegen)
describes: scaffold, the probe build, emit, and the final build. The wheel
it installs exports `from pgorm.bakery_entities import Cake, CakeModel,
CakeActive`, typed as the table above says. When the schema changes, run
pgorm-codegen again, then the four steps again.
