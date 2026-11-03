#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::OnConflict;
use pgorm::{Error, Schema, TryInsertResult, entity::prelude::*};
use pretty_assertions::assert_eq;

/// A manually assigned primary key alongside a unique column to conflict on:
/// the two keys the caller can name a row by, and the shape in which they
/// disagree.
mod manual_key {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "insert_pk_manual")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        #[pgorm(unique)]
        pub email: String,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

async fn create_manual_key_table<C>(db: &C) -> Result<(), Error>
where
    C: ConnectionTrait,
{
    let stmt = Schema::new().create_table_from_entity(manual_key::Entity)?;
    db.execute(&stmt.to_string(), &[]).await?;
    Ok(())
}

// [spec:pgorm:sem:exec.crud.insert+6/test]    the primary key comes from the
// RETURNING row in every case, including the client-supplied-key one that used
// to be answered from a cached tuple
// [spec:pgorm:sem:query.build.insert+5/test]    which is why the builder caches
// no primary key to answer from
#[pgorm_macros::test]
async fn manual_key_comes_from_returning() -> Result<(), Error> {
    let ctx = TestContext::new("insert_returning_pk_manual").await;
    let db = ctx.db.get().await?;
    create_manual_key_table(&db).await?;

    let id = Insert::one(manual_key::ActiveModel {
        id: set(7),
        email: set("first@example.com"),
        name: set("First"),
    })
    .exec_returning_pk(&db)
    .await?;
    assert_eq!(id, 7);

    // The database wrote the row it was told to write, so the key it reports
    // back and the key the caller supplied agree — the interesting case is the
    // one below, where they do not.
    assert_eq!(
        manual_key::Entity::find_by_id(7).one(&db).await?.email,
        "first@example.com"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:exec.crud.insert+6/test]    an `ON CONFLICT DO UPDATE` that
// lands on an existing row reports *that* row's primary key, not the one the
// insert asked for: answering `42` here would name a row that does not exist
// [spec:pgorm:sem:exec.crud.try-insert+4/test]    and the `TryInsert` wrapper
// reports it as `Inserted`, since a row really was written
#[pgorm_macros::test]
async fn upsert_reports_the_conflict_row_key() -> Result<(), Error> {
    let ctx = TestContext::new("insert_returning_pk_upsert").await;
    let db = ctx.db.get().await?;
    create_manual_key_table(&db).await?;

    Insert::one(manual_key::ActiveModel {
        id: set(7),
        email: set("dup@example.com"),
        name: set("Original"),
    })
    .exec(&db)
    .await?;

    // Insert id 42, conflicting on the unique email of row 7. PostgreSQL updates
    // row 7; 42 is never written.
    let id = Insert::one(manual_key::ActiveModel {
        id: set(42),
        email: set("dup@example.com"),
        name: set("Updated"),
    })
    .on_conflict(
        OnConflict::column(manual_key::Column::Email).update_column(manual_key::Column::Name),
    )
    .exec_returning_pk(&db)
    .await?;
    assert_eq!(id, 7, "the key of the row the database wrote");

    assert_eq!(
        manual_key::Entity::find_by_id(7).one(&db).await?.name,
        "Updated"
    );
    assert!(
        manual_key::Entity::find_by_id(42)
            .one_opt(&db)
            .await?
            .is_none()
    );

    // The same through `TryInsert`: a row was written, so it is `Inserted`, and
    // the key it carries is the conflict row's.
    let res = Insert::one(manual_key::ActiveModel {
        id: set(99),
        email: set("dup@example.com"),
        name: set("Updated again"),
    })
    .on_conflict(
        OnConflict::column(manual_key::Column::Email).update_column(manual_key::Column::Name),
    )
    .on_empty_do_nothing()
    .exec_returning_pk(&db)
    .await?;
    assert!(matches!(res, TryInsertResult::Inserted(7)), "got {res:?}");

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:exec.crud.insert+6/test]    an empty RETURNING is
// `RecordNotInserted` for a client-supplied key too, which is what an
// `ON CONFLICT DO NOTHING` that skipped the row yields
// [spec:pgorm:sem:exec.crud.try-insert+4/test]    `TryInsert` reads that as
// `Conflicted`, and an insert with nothing to write as `Empty` without touching
// the database
// [spec:pgorm:sem:query.build.insert.empty-failsafe+5/test]    the empty-insert
// failsafe is unchanged by the key resolution above
#[pgorm_macros::test]
async fn skipped_and_empty_inserts_are_unchanged() -> Result<(), Error> {
    let ctx = TestContext::new("insert_returning_pk_skipped").await;
    let db = ctx.db.get().await?;
    create_manual_key_table(&db).await?;

    let seed = manual_key::ActiveModel {
        id: set(7),
        email: set("dup@example.com"),
        name: set("Original"),
    };
    Insert::one(seed.clone()).exec(&db).await?;

    let err = Insert::one(seed.clone())
        .on_conflict(OnConflict::column(manual_key::Column::Email).do_nothing())
        .exec_returning_pk(&db)
        .await;
    assert_eq!(err.unwrap_err(), Error::RecordNotInserted);

    let res = Insert::one(seed)
        .on_conflict(OnConflict::column(manual_key::Column::Email).do_nothing())
        .on_empty_do_nothing()
        .exec_returning_pk(&db)
        .await?;
    assert!(matches!(res, TryInsertResult::Conflicted), "got {res:?}");

    let res = Insert::many(Vec::<manual_key::ActiveModel>::new())
        .on_empty_do_nothing()
        .exec_returning_pks(&db)
        .await?;
    assert!(matches!(res, TryInsertResult::Empty), "got {res:?}");

    drop(db);
    ctx.delete().await;
    Ok(())
}
