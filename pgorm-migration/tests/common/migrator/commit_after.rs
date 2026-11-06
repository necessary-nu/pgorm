use super::domain_default::{CREATED, DEFAULTED, INSERT, REDEFAULTED, Script};
use pgorm_migration::prelude::*;

const MOOD_CREATED: Script = Script {
    name: "m20240301_000001_create_mood",
    sql: "CREATE TYPE mood AS ENUM ('happy', 'sad'); \
          CREATE TABLE feeling (id integer PRIMARY KEY, mood mood NOT NULL)",
    commits_after: false,
};

/// A value PostgreSQL refuses to use before the transaction adding it
/// commits, which this migration asks for.
const MOOD_ADDED: Script = Script {
    name: "m20240301_000002_add_meh",
    sql: "ALTER TYPE mood ADD VALUE 'meh' AFTER 'happy'",
    commits_after: true,
};

const MOOD_USED: Script = Script {
    name: "m20240301_000003_feel_meh",
    sql: "INSERT INTO feeling (id, mood) VALUES (1, 'meh')",
    commits_after: false,
};

/// The enum alone, so that a later batch adds a value to a type created
/// before it: PostgreSQL lets a transaction use a value it added to a type it
/// created.
pub struct MoodCreated;

#[async_trait::async_trait]
impl MigratorTrait for MoodCreated {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(MOOD_CREATED)]
    }
}

/// An enum, a value added to it, and a row using the value: the batch commits
/// after the value is added, so the row can use it.
pub struct Moods;

#[async_trait::async_trait]
impl MigratorTrait for Moods {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(MOOD_CREATED),
            Box::new(MOOD_ADDED),
            Box::new(MOOD_USED),
        ]
    }
}

/// [`Moods`] with no migration asking to be committed, so the row uses the
/// value in the transaction that added it.
pub struct Unsplit;

#[async_trait::async_trait]
impl MigratorTrait for Unsplit {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(MOOD_CREATED),
            Box::new(Script {
                commits_after: false,
                ..MOOD_ADDED
            }),
            Box::new(MOOD_USED),
        ]
    }
}

/// [`Moods`] up to the commit, then a migration that fails.
pub struct FailingAfterCommit;

#[async_trait::async_trait]
impl MigratorTrait for FailingAfterCommit {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(MOOD_CREATED),
            Box::new(MOOD_ADDED),
            Box::new(Script {
                name: "m20240301_000003_feel_nothing",
                sql: "INSERT INTO feeling (id, mood) VALUES (1, 'numb')",
                commits_after: false,
            }),
        ]
    }
}

/// Writes a line through the cached [`INSERT`], failing unless the line takes
/// the quantity domain's default as [`REDEFAULTED`] sets it: a statement
/// prepared before that migration committed writes the old one.
pub struct Written;

impl MigrationName for Written {
    fn name(&self) -> &str {
        "m20240201_000004_write_line"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Written {
    async fn up(&self, tx: &mut DatabaseTransaction<'_>) -> Result<(), Error> {
        let qty: i32 = tx.query_one(INSERT, &[&10]).await?.get(0);
        if qty == 3 {
            Ok(())
        } else {
            Err(Error::Custom(format!(
                "the line was written with the quantity {qty}, the default its insert was \
                 prepared under"
            )))
        }
    }
}

/// The domain's default changed again in a migration that asks to be
/// committed, then a line written by an insert the connection prepared before.
pub struct Redefaulted;

#[async_trait::async_trait]
impl MigratorTrait for Redefaulted {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(CREATED),
            Box::new(DEFAULTED),
            Box::new(Script {
                commits_after: true,
                ..REDEFAULTED
            }),
            Box::new(Written),
        ]
    }
}
