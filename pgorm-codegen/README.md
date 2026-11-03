# pgorm-codegen

Entity source generation for pgorm. Give it a schema as DDL text and it
writes the entity modules: models, columns, primary keys, relations, and
active enums. The text is read with
[libpg_query](https://github.com/pganalyze/libpg_query), the PostgreSQL
server's own parser, so it is read as PostgreSQL 18 reads it. Library-only;
there is no CLI. Call it from a build script or a small binary of your own.

`entities_from_sql` is the whole pipeline. It returns a `WriterOutput`, whose
`files` are `OutputFile`s holding a name and its source, for you to write;
`EntityWriterOptions` chooses what is generated:

```rust,no_run
use pgorm_codegen::{EntityWriterOptions, WithSerde, entities_from_sql};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ddl = std::fs::read_to_string("schema.sql")?;
    let output = entities_from_sql(
        &ddl,
        EntityWriterOptions {
            with_serde: WithSerde::Both,
            ..Default::default()
        },
    )?;
    for file in output.files {
        std::fs::write(format!("src/entities/{}", file.name), file.content)?;
    }
    Ok(())
}
```

The schema is the `CREATE` statements: tables with their keys and foreign
keys, enum and range types, domains, indexes, comments and schemas, and the
columns and constraints an `ALTER TABLE` adds to a table. Anything else in
the text is refused with an `Error` naming it, so a schema that generates is
one read in full. `pg_dump --schema-only` opens with `SET`s and changes
owners, so its output is refused.

The tables `Schema::create_disjoint` writes for a `#[derive(Disjoint)]` enum
generate as that enum again, in its base's module, beside the base's and the
kinds' entities. Tables keyed by a foreign key from their primary key onto a
base's, with no kind column, generate as an `unkinded`, read-only one where
`unkinded_disjoint` names them:

```rust,no_run
use pgorm_codegen::{EntityWriterOptions, TableIdent, UnkindedDisjoint, entities_from_sql};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ddl = std::fs::read_to_string("schema.sql")?;
    let table = |name: &str| TableIdent {
        table: name.to_owned(),
        schema: None,
    };
    let shops = UnkindedDisjoint {
        base: table("shop"),
        kinds: vec![table("web_shop"), table("storefront")],
        unit: Some("dormant".to_owned()),
    };
    let output = entities_from_sql(
        &ddl,
        EntityWriterOptions {
            unkinded_disjoint: vec![shops],
            ..Default::default()
        },
    )?;
    assert!(output.files.iter().any(|file| file.name == "shop.rs"));
    Ok(())
}
```

The stages are public too: `parse_schema` reads the text into the
`TableCreateStatement`s `EntityTransformer::transform` takes beside the
`EntityWriterContext`, and `EntityWriter::generate` writes the entities, so
statements built in code generate the same way.
