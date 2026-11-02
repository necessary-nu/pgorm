#![allow(unused_imports, dead_code)]

//! The write terminals that read a written row's two versions, against a
//! live PostgreSQL 18 server: an update's row before and after it, and an
//! upsert's answer to whether it inserted each row or updated the one there.
//!
//! Each case holds the versions a terminal returns to the rows the table held
//! before and after the statement, so a version decoded from the other one,
//! or from a relation that took the `old` keyword, fails.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{Expr, OnConflict};
use pgorm::{Change, ConnectionTrait, Schema, Upserted, entity::prelude::*};
use pretty_assertions::assert_eq;
use tokio_postgres::error::SqlState;

mod stock {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, EnumIter, ActiveEnum)]
    #[pgorm(rs_type = String, enum_name = "shelf")]
    pub enum Shelf {
        #[pgorm(string_value = "front")]
        Front,
        #[pgorm(string_value = "back")]
        Back,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "stock")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub count: i32,
        pub shelf: Shelf,
        /// 61 bytes: under either version's prefix the result column's name
        /// reaches PostgreSQL's 63-byte bound and is spelled bounded.
        pub a_count_of_the_items_on_the_shelf_that_were_counted_last_time: Option<i32>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// A table called `old`, which a bare `old."col"` in its `RETURNING` list
/// would resolve to in place of the row before the write.
mod old {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "old")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub count: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// A table called by the name the terminals give the row before the write.
mod pgorm_old {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "pgorm_old")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub count: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

use stock::Shelf;

fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

fn item(id: i32, count: i32, shelf: Shelf) -> stock::Model {
    stock::Model {
        id,
        count,
        shelf,
        a_count_of_the_items_on_the_shelf_that_were_counted_last_time: Some(count - 1),
    }
}

impl stock::Model {
    fn with_last_count(self, last: i32) -> Self {
        Self {
            a_count_of_the_items_on_the_shelf_that_were_counted_last_time: Some(last),
            ..self
        }
    }
}

/// `stock` as its entity generates it, holding `rows`.
async fn stocked(db: &DatabaseConnection, rows: &[stock::Model]) -> Result<(), Error> {
    let schema = Schema::new();
    for create in schema.create_enum_from_entity(stock::Entity) {
        db.batch_execute(&create.to_string()).await?;
    }
    db.batch_execute(&schema.create_table_from_entity(stock::Entity)?.to_string())
        .await?;
    Insert::many(rows.iter().cloned().map(IntoActiveModel::into_active_model))
        .exec(db)
        .await?;
    Ok(())
}

/// An update's terminals return each row it changed before and after: the
/// many-row one every row its `WHERE` matched, the by-key one its row. The
/// enum column comes back through its read cast, and the column whose
/// prefixed name reaches the identifier bound decodes under both prefixes.
/// With nothing to set, the by-key terminal sends nothing and reads its row as
/// both versions, the many-row one refuses (`NothingToSet`), and a key no row
/// has is `RecordNotFound`.
// [spec:pgorm:sem:exec.crud.versions/test]    against a live server: an update's changes come back
// as the rows before and after it
#[pgorm_macros::test]
async fn an_update_returns_each_row_before_and_after() -> Result<(), Error> {
    let ctx = TestContext::new("write_versions_update").await;
    let db = ctx.db.get().await?;
    let rows = [
        item(1, 10, Shelf::Front),
        item(2, 20, Shelf::Front),
        item(3, 30, Shelf::Back),
    ];
    stocked(&db, &rows).await?;

    let mut changes = pgorm::Update::many(stock::Entity)
        .col_expr(stock::Column::Count, Expr::col(stock::Column::Count).add(5))
        .col_expr(
            stock::Column::Shelf,
            Expr::val("back").cast_as(Name::runtime("shelf")),
        )
        .filter(stock::Column::Id.lte(2))
        .exec_returning_changes(&db)
        .await?;
    changes.sort_by_key(|change| change.new.id);
    let moved = |row: &stock::Model| stock::Model {
        count: row.count + 5,
        shelf: Shelf::Back,
        ..row.clone()
    };
    assert_eq!(
        changes,
        [
            Change {
                old: rows[0].clone(),
                new: moved(&rows[0]),
            },
            Change {
                old: rows[1].clone(),
                new: moved(&rows[1]),
            },
        ]
    );

    let mut third = rows[2].clone().into_active_model();
    third.count = set(31);
    let change = pgorm::Update::one(third.clone())?
        .exec_returning_change(&db)
        .await?;
    assert_eq!(
        change,
        Change {
            old: rows[2].clone(),
            new: stock::Model {
                count: 31,
                ..rows[2].clone()
            },
        }
    );

    let unchanged = stock::Entity::find_by_id(3).one(&db).await?;
    assert_eq!(
        pgorm::Update::one(unchanged.clone().into_active_model())?
            .exec_returning_change(&db)
            .await?,
        Change {
            old: unchanged.clone(),
            new: unchanged,
        }
    );
    let nothing = pgorm::Update::many(stock::Entity)
        .filter(stock::Column::Id.eq(1))
        .exec_returning_changes(&db)
        .await;
    assert!(matches!(nothing, Err(Error::NothingToSet)), "{nothing:?}");
    let mut missing = third;
    missing.id = set(99);
    let missing = pgorm::Update::one(missing)?
        .exec_returning_change(&db)
        .await;
    assert!(matches!(missing, Err(Error::RecordNotFound)), "{missing:?}");

    drop(db);
    ctx.delete().await;
    Ok(())
}

fn upsert_count() -> OnConflict {
    OnConflict::column(stock::Column::Id)
        .update_column(stock::Column::Count)
        .into()
}

/// An upsert's terminals say, for each row it wrote, whether it inserted the
/// row or updated the one its key conflicted with, which comes back before
/// and after. A row `DO NOTHING` skipped, or that the `DO UPDATE`'s `WHERE`
/// left alone, is not among the answers, and the one-row terminal answers
/// `None` for it; without a conflict clause every row is inserted.
// [spec:pgorm:sem:exec.crud.versions/test]    against a live server: an upsert tells an inserted row
// from an updated one, and returns nothing for a row it did not write
#[pgorm_macros::test]
async fn an_upsert_says_what_it_did_per_row() -> Result<(), Error> {
    let ctx = TestContext::new("write_versions_upsert").await;
    let db = ctx.db.get().await?;
    let rows = [item(1, 10, Shelf::Front), item(2, 20, Shelf::Back)];
    stocked(&db, &rows).await?;

    let written = Insert::many([
        item(1, 11, Shelf::Back).into_active_model(),
        item(3, 30, Shelf::Front).into_active_model(),
    ])
    .on_conflict(upsert_count())
    .exec_returning_upserts(&db)
    .await?;
    assert_eq!(
        written,
        [
            Upserted::Updated(Change {
                old: rows[0].clone(),
                new: item(1, 11, Shelf::Front).with_last_count(9),
            }),
            Upserted::Inserted(item(3, 30, Shelf::Front)),
        ]
    );

    let skipped = Insert::many([
        item(2, 99, Shelf::Front).into_active_model(),
        item(4, 40, Shelf::Back).into_active_model(),
    ])
    .on_conflict(OnConflict::column(stock::Column::Id).do_nothing())
    .exec_returning_upserts(&db)
    .await?;
    assert_eq!(skipped, [Upserted::Inserted(item(4, 40, Shelf::Back))]);
    let one = Insert::one(item(2, 99, Shelf::Front).into_active_model())
        .on_conflict(OnConflict::column(stock::Column::Id).do_nothing())
        .exec_returning_upsert(&db)
        .await?;
    assert_eq!(one, None);

    let filtered = OnConflict::column(stock::Column::Id)
        .update_column(stock::Column::Count)
        .and_where(Expr::col((stock::Entity, stock::Column::Count)).gt(100));
    let left_alone = Insert::one(item(2, 99, Shelf::Front).into_active_model())
        .on_conflict(filtered)
        .exec_returning_upsert(&db)
        .await?;
    assert_eq!(left_alone, None);

    let plain = Insert::one(item(5, 50, Shelf::Front).into_active_model())
        .exec_returning_upsert(&db)
        .await?;
    assert_eq!(plain, Some(Upserted::Inserted(item(5, 50, Shelf::Front))));
    assert_eq!(
        plain.map(Upserted::into_model),
        Some(item(5, 50, Shelf::Front))
    );
    let none: Vec<stock::ActiveModel> = Vec::new();
    assert_eq!(Insert::many(none).exec_returning_upserts(&db).await?, []);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// The terminals rename both versions, so an entity whose table is called
/// `old` — which a bare `old."col"` would read in place of the row before the
/// write, answering the new row twice — returns its old row. An entity whose
/// table is called by the rename itself is refused (`42712`) rather than
/// answered wrong.
// [spec:pgorm:sem:exec.crud.versions/test]    against a live server: a table called `old` reads its
// old row, and one called by the rename is refused
#[pgorm_macros::test]
async fn a_table_called_old_keeps_its_old_row() -> Result<(), Error> {
    let ctx = TestContext::new("write_versions_old_table").await;
    let db = ctx.db.get().await?;
    for create in [
        Schema::new().create_table_from_entity(old::Entity)?,
        Schema::new().create_table_from_entity(pgorm_old::Entity)?,
    ] {
        db.batch_execute(&create.to_string()).await?;
    }
    db.batch_execute("INSERT INTO old VALUES (1, 1); INSERT INTO pgorm_old VALUES (1, 1)")
        .await?;

    let changes = pgorm::Update::many(old::Entity)
        .col_expr(old::Column::Count, Expr::val(2).into())
        .exec_returning_changes(&db)
        .await?;
    assert_eq!(
        changes,
        [Change {
            old: old::Model { id: 1, count: 1 },
            new: old::Model { id: 1, count: 2 },
        }]
    );
    let upserted = Insert::one(old::ActiveModel {
        id: set(1),
        count: set(3),
    })
    .on_conflict(OnConflict::column(old::Column::Id).update_column(old::Column::Count))
    .exec_returning_upsert(&db)
    .await?;
    assert_eq!(
        upserted,
        Some(Upserted::Updated(Change {
            old: old::Model { id: 1, count: 2 },
            new: old::Model { id: 1, count: 3 },
        }))
    );

    let refused = pgorm::Update::many(pgorm_old::Entity)
        .col_expr(pgorm_old::Column::Count, Expr::val(2).into())
        .exec_returning_changes(&db)
        .await
        .expect_err("a table called by the rename");
    refused_with(&refused, &SqlState::DUPLICATE_ALIAS);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// An upsert racing another transaction's uncommitted insert of its key waits
/// for it, and once it commits updates the committed row, as `ON CONFLICT`
/// arbitrates: the answer is `Updated`, read off the old row, with no
/// need of the `xmax` the answer used to be guessed from.
// [spec:pgorm:sem:exec.crud.versions/test]    against a live server: an upsert that waited on a
// concurrent insert of its key reports the update it made
#[pgorm_macros::test]
async fn a_raced_upsert_reports_its_update() -> Result<(), Error> {
    let ctx = TestContext::new("write_versions_race").await;
    let mut first = ctx.db.get().await?;
    stocked(&first, &[]).await?;

    let second = ctx.db.get().await?;
    let observer = ctx.db.get().await?;
    // The racing upsert starts while this transaction holds the key, and the
    // transaction commits once it is seen waiting on the lock.
    let racing = first
        .transaction(async |tx| {
            Insert::one(item(7, 70, Shelf::Front).into_active_model())
                .exec(&tx)
                .await?;
            let racing = tokio::spawn(async move {
                Insert::one(item(7, 71, Shelf::Back).into_active_model())
                    .on_conflict(upsert_count())
                    .exec_returning_upsert(&second)
                    .await
            });
            while !racing.is_finished() {
                let waiting: i64 = observer
                    .query_one(
                        "SELECT count(*) FROM pg_stat_activity \
                         WHERE datname = current_database() AND wait_event_type = 'Lock'",
                        &[],
                    )
                    .await?
                    .get(0);
                if waiting > 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
            Ok::<_, Error>(racing)
        })
        .await
        .expect("the first insert commits");

    let raced = racing.await.expect("the racing upsert's task")?;
    assert_eq!(
        raced,
        Some(Upserted::Updated(Change {
            old: item(7, 70, Shelf::Front),
            new: item(7, 71, Shelf::Front).with_last_count(69),
        }))
    );

    drop(first);
    drop(observer);
    ctx.delete().await;
    Ok(())
}
