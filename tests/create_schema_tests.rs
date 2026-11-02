#![allow(unused_imports, dead_code)]

//! `CREATE SCHEMA`, `DROP SCHEMA` and `ALTER SCHEMA ... RENAME TO` against a
//! live PostgreSQL server.
//!
//! The render tests in pgorm-query hold each statement to libpg_query's
//! `CreateSchemaStmt`, `DropStmt` and `RenameStmt`. What only a server settles
//! is what they do: that a schema the builder creates holds the tables, enum
//! types and sequences other builders qualify by its name, and an entity
//! declaring it as its `schema_name` reads and writes there; that a drop is
//! refused while the schema holds anything unless it cascades; that a rename
//! carries the schema's contents with it; that a schema is undone with the
//! transaction that created it; and what the server refuses that no builder
//! value records — a schema that exists, a missing one, and a `pg_` name.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{Name, Schema, Sequence};
use pgorm::{ConnectionTrait, TransactionTrait, entity::prelude::*};
use pretty_assertions::assert_eq;
use tokio_postgres::error::SqlState;

/// An invoice's state: an enum type that lives in `billing`.
mod invoice_state {
    use pgorm::entity::prelude::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter, ActiveEnum)]
    #[pgorm(
        rs_type = String,
        enum_name = "invoice_state",
        schema_name = "billing"
    )]
    pub enum InvoiceState {
        #[pgorm(string_value = "draft")]
        Draft,
        #[pgorm(string_value = "sent")]
        Sent,
    }
}

/// `billing.invoice`, whose state column is `billing.invoice_state`.
mod invoice {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(schema_name = "billing", table_name = "invoice")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub state: super::invoice_state::InvoiceState,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

use invoice_state::InvoiceState;

fn n(name: &str) -> Name {
    Name::runtime(name)
}

fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

/// Every schema in the database that is not PostgreSQL's own, by name.
async fn schemas(db: &impl ConnectionTrait) -> Result<Vec<String>, Error> {
    let rows = db
        .query_all(
            "SELECT nspname::text FROM pg_namespace \
             WHERE nspname NOT LIKE 'pg\\_%' AND nspname <> 'information_schema' \
             ORDER BY nspname COLLATE \"C\"",
            &[],
        )
        .await?;
    Ok(rows.iter().map(|row| row.get(0)).collect())
}

/// `billing`, created by the builder, and inside it the enum type and the
/// table pgorm generates from the entity, and a sequence, each qualified by
/// the schema's name.
async fn billing(db: &impl ConnectionTrait) -> Result<(), Error> {
    let create = Schema::create(n("billing")).to_string();
    assert_eq!(create, r#"CREATE SCHEMA "billing""#);
    db.batch_execute(&create).await?;
    let generator = pgorm::Schema::new();
    for statement in generator.create_enum_from_entity(invoice::Entity) {
        db.batch_execute(&statement.to_string()).await?;
    }
    db.batch_execute(
        &generator
            .create_table_from_entity(invoice::Entity)?
            .to_string(),
    )
    .await?;
    let sequence = Sequence::create((n("billing"), n("invoice_no"))).to_string();
    assert_eq!(sequence, r#"CREATE SEQUENCE "billing"."invoice_no""#);
    db.batch_execute(&sequence).await
}

/// What `billing` holds, by kind and name.
async fn contents(db: &impl ConnectionTrait, schema: &str) -> Result<Vec<String>, Error> {
    let rows = db
        .query_all(
            "SELECT item FROM ( \
             SELECT relkind::text || ' ' || relname::text FROM pg_class c \
             JOIN pg_namespace s ON s.oid = c.relnamespace WHERE s.nspname = $1 \
             UNION ALL \
             SELECT 'enum ' || typname::text FROM pg_type t \
             JOIN pg_namespace s ON s.oid = t.typnamespace \
             WHERE s.nspname = $1 AND t.typtype = 'e' \
             ) AS held (item) ORDER BY item COLLATE \"C\"",
            &[&schema],
        )
        .await?;
    Ok(rows.iter().map(|row| row.get(0)).collect())
}

/// A schema the builder creates holds the enum, table and sequence other
/// builders qualify by its name, an entity naming it reads and writes its
/// table there, and the schema cannot be dropped while it holds them unless
/// the drop cascades.
// [spec:pgorm:req:sql.ddl.schema/test]    a created schema holds a qualified table, enum and
// sequence, an entity whose schema_name is the schema reads and writes there, and a drop is
// refused under RESTRICT, said or unsaid, until it cascades
#[pgorm_macros::test]
async fn a_schema_holds_what_names_it() -> Result<(), Error> {
    let ctx = TestContext::new("schema_holds_qualified_objects").await;
    let db = ctx.db.get().await?;
    billing(&db).await?;
    assert_eq!(
        contents(&db, "billing").await?,
        [
            "S invoice_no",
            "enum invoice_state",
            "i invoice_pkey",
            "r invoice"
        ]
    );

    invoice::ActiveModel {
        id: set(1),
        state: set(InvoiceState::Sent),
    }
    .insert(&db)
    .await?;
    let read = invoice::Entity::find_by_id(1).one(&db).await?;
    assert_eq!(
        read,
        invoice::Model {
            id: 1,
            state: InvoiceState::Sent
        }
    );
    let row = db
        .query_one("SELECT state::text FROM billing.invoice WHERE id = 1", &[])
        .await?;
    assert_eq!(row.get::<_, String>(0), "sent");

    let unsaid = db
        .batch_execute(&Schema::drop(n("billing")).to_string())
        .await
        .expect_err("a drop of a schema that holds a table");
    refused_with(&unsaid, &SqlState::DEPENDENT_OBJECTS_STILL_EXIST);
    let restrict = Schema::drop(n("billing")).restrict().to_string();
    assert_eq!(restrict, r#"DROP SCHEMA "billing" RESTRICT"#);
    let said = db
        .batch_execute(&restrict)
        .await
        .expect_err("a restricted drop of a schema that holds a table");
    refused_with(&said, &SqlState::DEPENDENT_OBJECTS_STILL_EXIST);
    assert_eq!(schemas(&db).await?, ["billing", "public"]);

    let cascade = Schema::drop(n("billing")).cascade().to_string();
    assert_eq!(cascade, r#"DROP SCHEMA "billing" CASCADE"#);
    db.batch_execute(&cascade).await?;
    assert_eq!(schemas(&db).await?, ["public"]);
    assert!(contents(&db, "billing").await?.is_empty());
    let gone = invoice::Entity::find()
        .all(&db)
        .await
        .expect_err("a dropped table");
    refused_with(&gone, &SqlState::UNDEFINED_TABLE);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A create of a schema that exists is refused, `IF NOT EXISTS` leaves it as
/// it was, PostgreSQL keeps `pg_` names for itself, and a quoted name keeps
/// its case and its quote.
// [spec:pgorm:req:sql.ddl.schema/test]    a schema that exists is 42P06 unless IF NOT EXISTS, a
// pg_ name is 42939, and a quoted name is stored exactly as given
#[pgorm_macros::test]
async fn a_create_is_refused_what_exists() -> Result<(), Error> {
    let ctx = TestContext::new("schema_create_refusals").await;
    let db = ctx.db.get().await?;
    db.batch_execute(&Schema::create(n("billing")).to_string())
        .await?;
    let twice = db
        .batch_execute(&Schema::create(n("billing")).to_string())
        .await
        .expect_err("a schema created twice");
    refused_with(&twice, &SqlState::DUPLICATE_SCHEMA);
    db.batch_execute(&Schema::create(n("billing")).if_not_exists().to_string())
        .await?;

    let reserved = db
        .batch_execute(&Schema::create(n("pg_billing")).to_string())
        .await
        .expect_err("a pg_ schema");
    refused_with(&reserved, &SqlState::RESERVED_NAME);
    let reserved = db
        .batch_execute(&Schema::create(n("pg_billing")).if_not_exists().to_string())
        .await
        .expect_err("a pg_ schema under IF NOT EXISTS");
    refused_with(&reserved, &SqlState::RESERVED_NAME);

    let odd = r#"Bill "ing"; DROP SCHEMA public; --"#;
    db.batch_execute(&Schema::create(n(odd)).to_string())
        .await?;
    assert_eq!(schemas(&db).await?, [odd, "billing", "public"]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A rename carries the schema's contents to the new name, so an entity that
/// names the old one stops finding its table; a drop takes several schemas;
/// and a missing schema, a taken name and a `pg_` name are refused.
// [spec:pgorm:req:sql.ddl.schema/test]    a rename moves the schema's objects, a drop names
// several schemas, and a missing schema (3F000), a taken name (42P06) and a pg_ name (42939) are
// refused
#[pgorm_macros::test]
async fn a_rename_moves_what_the_schema_holds() -> Result<(), Error> {
    let ctx = TestContext::new("schema_rename_and_drop").await;
    let db = ctx.db.get().await?;
    billing(&db).await?;
    db.batch_execute(&Schema::create(n("archive")).to_string())
        .await?;

    let rename = Schema::rename(n("billing"), n("ledger")).to_string();
    assert_eq!(rename, r#"ALTER SCHEMA "billing" RENAME TO "ledger""#);
    db.batch_execute(&rename).await?;
    assert_eq!(schemas(&db).await?, ["archive", "ledger", "public"]);
    assert_eq!(contents(&db, "ledger").await?.len(), 4);
    let moved = invoice::Entity::find()
        .all(&db)
        .await
        .expect_err("a renamed schema");
    refused_with(&moved, &SqlState::UNDEFINED_TABLE);

    let missing = db
        .batch_execute(&Schema::rename(n("billing"), n("other")).to_string())
        .await
        .expect_err("a rename of a missing schema");
    refused_with(&missing, &SqlState::INVALID_SCHEMA_NAME);
    let taken = db
        .batch_execute(&Schema::rename(n("ledger"), n("archive")).to_string())
        .await
        .expect_err("a rename onto a taken name");
    refused_with(&taken, &SqlState::DUPLICATE_SCHEMA);
    let reserved = db
        .batch_execute(&Schema::rename(n("ledger"), n("pg_ledger")).to_string())
        .await
        .expect_err("a rename onto a pg_ name");
    refused_with(&reserved, &SqlState::RESERVED_NAME);

    let missing = db
        .batch_execute(&Schema::drop(n("billing")).to_string())
        .await
        .expect_err("a drop of a missing schema");
    refused_with(&missing, &SqlState::INVALID_SCHEMA_NAME);
    let both = Schema::drop(n("billing"))
        .name(n("archive"))
        .name(n("ledger"))
        .if_exists()
        .cascade()
        .to_string();
    assert_eq!(
        both,
        r#"DROP SCHEMA IF EXISTS "billing", "archive", "ledger" CASCADE"#
    );
    db.batch_execute(&both).await?;
    assert_eq!(schemas(&db).await?, ["public"]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A schema is created inside a transaction, and a rollback takes it away
/// with what was created in it.
// [spec:pgorm:req:sql.ddl.schema/test]    a schema created in a transaction is gone when the
// transaction rolls back
#[pgorm_macros::test]
async fn a_schema_rolls_back_with_its_transaction() -> Result<(), Error> {
    let ctx = TestContext::new("schema_rolls_back").await;
    let mut db = ctx.db.get().await?;
    let outcome = db
        .transaction(async |txn| {
            billing(&txn).await?;
            assert_eq!(contents(&txn, "billing").await?.len(), 4);
            roll_back::<()>()
        })
        .await;
    assert_rolled_back(outcome);
    assert_eq!(schemas(&db).await?, ["public"]);

    drop(db);
    ctx.delete().await;
    Ok(())
}
