#![allow(unused_imports, dead_code)]

//! The foreign keys `Schema::create_table_from_entity` writes, and refuses to
//! write, over columns whose types are domains, held against what a live
//! PostgreSQL server keys.
//!
//! The server implements no foreign key with a column on either side typed
//! as a domain over an enum (`42804`), so generation refuses such a relation
//! and names it. Every pairing the server does key is written: an enum with
//! itself, a domain over an integer with the integer or itself, and a domain
//! over an array of an enum with itself.
//!
//! Run locally:
//! `DATABASE_URL="postgres://postgres:postgres@localhost:54329" cargo nextest run --test schema_foreign_key_tests`

pub mod common;
pub use common::TestContext;

use pgorm::pgorm_query::{
    ColumnType, ForeignKeyCreateStatement, Name, Table, TableCreateStatement,
};
use pgorm::{ConnectionTrait, Error, Schema, entity::prelude::*};
use pretty_assertions::assert_eq;
use std::sync::Arc;
use tokio_postgres::error::SqlState;

/// The enum, the two domains over it, the domain over its array and the
/// domain over an integer the fixtures' columns are typed with.
const TYPES: &str = r#"
    CREATE TYPE "mood" AS ENUM ('sad', 'calm', 'glad');
    CREATE DOMAIN "cheerful" AS "mood";
    CREATE DOMAIN "beaming" AS "cheerful";
    CREATE DOMAIN "moods" AS "mood"[];
    CREATE DOMAIN "tally" AS integer;
"#;

mod mood {
    use pgorm::entity::prelude::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, ActiveEnum)]
    #[pgorm(enum_name = "mood")]
    pub enum Mood {
        #[pgorm(string_value = "sad")]
        Sad,
        #[pgorm(string_value = "calm")]
        Calm,
        #[pgorm(string_value = "glad")]
        Glad,
    }
}

use mood::Mood;

/// The domain `cheerful`, over the enum `mood`.
pub fn cheerful() -> ColumnType {
    ColumnType::Domain {
        name: Name::runtime("cheerful"),
        schema: None,
        base: Arc::new(Mood::db_type().get_column_type().clone()),
    }
}

/// The table every fixture references: one unique column of each type.
mod anchor {
    use super::Mood;
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "anchor")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        #[pgorm(unique)]
        pub as_mood: Mood,
        #[pgorm(
            unique,
            column_type = ColumnType::Domain { name: Name::runtime("cheerful"), schema: None, base: Arc::new(Mood::db_type().get_column_type().clone()) }
        )]
        pub as_cheerful: Mood,
        #[pgorm(unique)]
        pub as_count: i32,
        #[pgorm(
            unique,
            column_type = ColumnType::Domain { name: Name::runtime("tally"), schema: None, base: Arc::new(ColumnType::Integer) }
        )]
        pub as_tally: i32,
        #[pgorm(
            unique,
            column_type = ColumnType::Domain { name: Name::runtime("moods"), schema: None, base: Arc::new(ColumnType::Array(Arc::new(Mood::db_type().get_column_type().clone()))) }
        )]
        pub as_moods: Vec<Mood>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

/// Every pairing the server keys: an enum with itself, a domain over an
/// integer with the integer and the integer with it, and a domain over an
/// array of an enum with itself.
mod keyed {
    use super::Mood;
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "keyed")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub mood: Mood,
        #[pgorm(
            column_type = ColumnType::Domain { name: Name::runtime("tally"), schema: None, base: Arc::new(ColumnType::Integer) }
        )]
        pub tally: i32,
        #[pgorm(
            column_type = ColumnType::Domain { name: Name::runtime("tally"), schema: None, base: Arc::new(ColumnType::Integer) }
        )]
        pub counted: i32,
        pub plain: i32,
        #[pgorm(
            column_type = ColumnType::Domain { name: Name::runtime("moods"), schema: None, base: Arc::new(ColumnType::Array(Arc::new(Mood::db_type().get_column_type().clone()))) }
        )]
        pub moods: Vec<Mood>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = super::anchor::Entity,
            from = Column::Mood,
            to = super::anchor::Column::AsMood
        )]
        Mood,
        #[pgorm(
            belongs_to = super::anchor::Entity,
            from = Column::Counted,
            to = super::anchor::Column::AsCount
        )]
        TallyToCount,
        #[pgorm(
            belongs_to = super::anchor::Entity,
            from = Column::Plain,
            to = super::anchor::Column::AsTally
        )]
        CountToTally,
        #[pgorm(
            belongs_to = super::anchor::Entity,
            from = Column::Tally,
            to = super::anchor::Column::AsTally
        )]
        Tally,
        #[pgorm(
            belongs_to = super::anchor::Entity,
            from = Column::Moods,
            to = super::anchor::Column::AsMoods
        )]
        Moods,
    }
}

/// A domain over an enum keyed to the same domain.
mod domain_to_domain {
    use super::Mood;
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "domain_to_domain")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        #[pgorm(
            column_type = ColumnType::Domain { name: Name::runtime("cheerful"), schema: None, base: Arc::new(Mood::db_type().get_column_type().clone()) }
        )]
        pub mood: Mood,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = super::anchor::Entity,
            from = Column::Mood,
            to = super::anchor::Column::AsCheerful
        )]
        Anchor,
    }
}

/// A domain over an enum keyed to the enum itself.
mod domain_to_enum {
    use super::Mood;
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "domain_to_enum")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        #[pgorm(
            column_type = ColumnType::Domain { name: Name::runtime("cheerful"), schema: None, base: Arc::new(Mood::db_type().get_column_type().clone()) }
        )]
        pub mood: Mood,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = super::anchor::Entity,
            from = Column::Mood,
            to = super::anchor::Column::AsMood
        )]
        Anchor,
    }
}

/// An enum keyed to a domain over it: the referencing side is the plain enum.
mod enum_to_domain {
    use super::Mood;
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "enum_to_domain")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub mood: Mood,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = super::anchor::Entity,
            from = Column::Mood,
            to = super::anchor::Column::AsCheerful
        )]
        Anchor,
    }
}

/// A domain over a domain over an enum keyed to the inner domain.
mod nested_to_domain {
    use super::Mood;
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "nested_to_domain")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        #[pgorm(
            column_type = ColumnType::Domain { name: Name::runtime("beaming"), schema: None, base: Arc::new(super::cheerful()) }
        )]
        pub mood: Mood,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = super::anchor::Entity,
            from = Column::Mood,
            to = super::anchor::Column::AsCheerful
        )]
        Anchor,
    }
}

/// The refusal generation answers for `entity`, whose one relation PostgreSQL
/// cannot key.
#[track_caller]
fn refusal<E: EntityTrait>(entity: E) -> String {
    match Schema::new().create_table_from_entity(entity) {
        Err(Error::Type(message)) => message,
        other => panic!("expected Error::Type, got {other:?}"),
    }
}

/// `entity`'s table with every column and no constraint: what the server is
/// handed a refused relation's foreign key against.
fn unkeyed_table<E: EntityTrait>(entity: E) -> TableCreateStatement {
    let schema = Schema::new();
    let mut table = Table::create(entity.table_ref());
    for column in E::Column::iter() {
        table.col(schema.get_column_def::<E>(column));
    }
    table.take()
}

// [spec:pgorm:sem:schema.from-entity+12/test]    a belongs-to relation with a column typed as a
// domain over an enum on either side is refused as Error::Type naming the entity, the relation
// and the pair, whichever side the domain is on and however deep it nests
#[test]
fn domain_enum_keys_are_refused_by_name() {
    let refused = [
        (
            refusal(domain_to_domain::Entity),
            "domain_to_domain",
            "as_cheerful",
        ),
        (refusal(domain_to_enum::Entity), "domain_to_enum", "as_mood"),
        (
            refusal(enum_to_domain::Entity),
            "enum_to_domain",
            "as_cheerful",
        ),
        (
            refusal(nested_to_domain::Entity),
            "nested_to_domain",
            "as_cheerful",
        ),
    ];
    for (message, table, to) in refused {
        for part in [table, "Anchor", "mood", "anchor", to, "42804"] {
            assert!(message.contains(part), "`{part}` is named: {message}");
        }
    }
    assert_eq!(
        refusal(domain_to_domain::Entity),
        "relation `Anchor` of `domain_to_domain` cannot be a foreign key: \
         `domain_to_domain`.`mood` references `anchor`.`as_cheerful`, and PostgreSQL implements \
         no foreign key with a column typed as a domain over an enum on either side (42804)"
    );
}

// [spec:pgorm:sem:schema.from-entity+12/test]    generation refuses exactly the foreign keys the
// server refuses: each refused relation's key is answered 42804 when sent by hand, and an entity
// whose every pairing the server keys is created with all of its keys, live
#[pgorm_macros::test]
async fn domain_key_refusals_match_the_server() -> Result<(), Error> {
    let ctx = TestContext::new("schema_fk_domain_refusals").await;
    let db = ctx.db.get().await?;
    db.batch_execute(TYPES).await?;
    let schema = Schema::new();
    db.batch_execute(&schema.create_table_from_entity(anchor::Entity)?.to_string())
        .await?;

    let refused = [
        (
            unkeyed_table(domain_to_domain::Entity),
            ForeignKeyCreateStatement::from(domain_to_domain::Relation::Anchor.def()),
        ),
        (
            unkeyed_table(domain_to_enum::Entity),
            ForeignKeyCreateStatement::from(domain_to_enum::Relation::Anchor.def()),
        ),
        (
            unkeyed_table(enum_to_domain::Entity),
            ForeignKeyCreateStatement::from(enum_to_domain::Relation::Anchor.def()),
        ),
        (
            unkeyed_table(nested_to_domain::Entity),
            ForeignKeyCreateStatement::from(nested_to_domain::Relation::Anchor.def()),
        ),
    ];
    for (table, key) in refused {
        db.batch_execute(&table.to_string()).await?;
        match db.batch_execute(&key.to_string()).await {
            Err(Error::Postgres(error)) => {
                assert_eq!(error.code(), Some(&SqlState::DATATYPE_MISMATCH), "{error}")
            }
            other => panic!("expected the server to refuse {key}, got {other:?}"),
        }
    }

    db.batch_execute(&schema.create_table_from_entity(keyed::Entity)?.to_string())
        .await?;
    let keys: i64 = db
        .query_one(
            "SELECT count(*) FROM pg_constraint WHERE conrelid = 'keyed'::regclass AND contype = 'f'",
            &[],
        )
        .await?
        .try_get(0)?;
    assert_eq!(keys, 5);

    drop(db);
    ctx.delete().await;
    Ok(())
}
