#![allow(unused_imports, dead_code)]

//! An entity relation whose foreign key is `NOT ENFORCED` or deferrable,
//! against a live PostgreSQL server.
//!
//! The relation carries the two attributes its key is declared with, so the
//! schema generated from the entity creates the key the entity describes. What
//! only a server settles is what each means — a `NOT ENFORCED` key admits a
//! row that references nothing, a deferred one waits for the commit — and
//! what the ORM's readers make of a row that references nothing: the
//! relation's loaders and joins read it as having no related row.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::{ConnectionTrait, LoaderTrait, Schema, TransactionTrait, entity::prelude::*};
use pretty_assertions::assert_eq;
use tokio_postgres::error::SqlState;

mod shelf {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "shelf")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub label: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {
        #[pgorm(has_many = "super::loose_book::Entity")]
        LooseBook,
    }

    impl Related<super::loose_book::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::LooseBook.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// A book whose shelf the server never checks: its key is `NOT ENFORCED`.
mod loose_book {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "loose_book")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub shelf_id: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = "super::shelf::Entity",
            from = "Column::ShelfId",
            to = "super::shelf::Column::Id",
            enforcement = "NotEnforced"
        )]
        Shelf,
    }

    impl Related<super::shelf::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Shelf.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// A book whose shelf is checked at commit: its key is deferred.
mod later_book {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "later_book")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub shelf_id: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = "super::shelf::Entity",
            from = "Column::ShelfId",
            to = "super::shelf::Column::Id",
            deferrability = "DeferrableInitiallyDeferred"
        )]
        Shelf,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// A book whose shelf is checked as each statement ends: the plain key, the
/// control.
mod tight_book {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "tight_book")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub shelf_id: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = "super::shelf::Entity",
            from = "Column::ShelfId",
            to = "super::shelf::Column::Id"
        )]
        Shelf,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

/// Each foreign key of `table` as the catalogue holds it: whether it is
/// enforced, deferrable and initially deferred.
async fn key_of(db: &DatabaseConnection, table: &str) -> Result<(bool, bool, bool), Error> {
    let row = db
        .query_one(
            "SELECT conenforced, condeferrable, condeferred FROM pg_constraint \
             WHERE conrelid = $1::text::regclass AND contype = 'f'",
            &[&table],
        )
        .await?;
    Ok((row.get(0), row.get(1), row.get(2)))
}

async fn create<E: EntityTrait>(db: &DatabaseConnection, entity: E) -> Result<String, Error> {
    let create = Schema::new().create_table_from_entity(entity)?.to_string();
    db.batch_execute(&create).await?;
    Ok(create)
}

/// A relation declared `NOT ENFORCED` generates a key the server records and
/// never checks, so a book may name a shelf that does not exist, where the
/// plain key refuses it (`23503`). The relation's readers take such a book as
/// having no shelf: `load_one` answers `None` for it, a graph's optional slot
/// pairs it with `None` and a required slot leaves it out, as the shelf's
/// `load_many` and `find_related` do.
// [spec:pgorm:def:entity.relation.def+12/test]    against a live server: a NOT ENFORCED relation
// creates the key unchecked, and its readers take an orphan as having no related row
// [spec:pgorm:req:entity.relation.fk+5/test]
#[pgorm_macros::test]
async fn a_not_enforced_relation_admits_orphans() -> Result<(), Error> {
    let ctx = TestContext::new("relation_not_enforced").await;
    let db = ctx.db.get().await?;
    create(&db, shelf::Entity).await?;
    let loose = create(&db, loose_book::Entity).await?;
    create(&db, tight_book::Entity).await?;
    assert!(loose.contains("NOT ENFORCED"), "{loose}");
    assert_eq!(key_of(&db, "loose_book").await?, (false, false, false));
    assert_eq!(key_of(&db, "tight_book").await?, (true, false, false));

    let shelf = shelf::ActiveModel {
        id: set(1),
        label: set("near"),
    }
    .insert(&db)
    .await?;
    let refused = db
        .execute("INSERT INTO tight_book VALUES (1, 99)", &[])
        .await
        .expect_err("an orphan under the plain key");
    refused_with(&refused, &SqlState::FOREIGN_KEY_VIOLATION);
    let books = Insert::many(
        [(1, 1), (2, 99)].map(|(id, shelf_id)| loose_book::ActiveModel {
            id: set(id),
            shelf_id: set(shelf_id),
        }),
    )
    .exec_returning_models(&db)
    .await?;

    assert_eq!(
        books.load_one(shelf::Entity, &db).await?,
        [Some(shelf.clone()), None]
    );
    assert_eq!(
        loose_book::Entity::graph()
            .join_maybe::<shelf::Entity>(loose_book::Relation::Shelf.def())
            .order_by_asc(loose_book::Column::Id)
            .all(&db)
            .await?,
        [
            (books[0].clone(), Some(shelf.clone())),
            (books[1].clone(), None)
        ]
    );
    assert_eq!(
        loose_book::Entity::graph()
            .join_one::<shelf::Entity>(loose_book::Relation::Shelf.def())
            .all(&db)
            .await?,
        [(books[0].clone(), shelf.clone())]
    );
    assert_eq!(
        vec![shelf.clone()]
            .load_many(loose_book::Entity, &db)
            .await?,
        [vec![books[0].clone()]]
    );
    assert_eq!(
        shelf.find_related(loose_book::Entity).all(&db).await?,
        [books[0].clone()]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A relation declared `DEFERRABLE INITIALLY DEFERRED` generates a key
/// checked at commit: a transaction may write a book before its shelf, and a
/// book whose shelf never comes is refused at commit (`23503`), where the
/// plain key refuses the first insert.
// [spec:pgorm:def:entity.relation.def+12/test]    against a live server: a deferred relation's
// key is checked at commit
// [spec:pgorm:req:entity.relation.fk+5/test]
#[pgorm_macros::test]
async fn a_deferred_relation_waits_for_commit() -> Result<(), Error> {
    let ctx = TestContext::new("relation_deferred").await;
    let mut db = ctx.db.get().await?;
    create(&db, shelf::Entity).await?;
    let later = create(&db, later_book::Entity).await?;
    assert!(later.contains("DEFERRABLE INITIALLY DEFERRED"), "{later}");
    assert_eq!(key_of(&db, "later_book").await?, (true, true, true));

    db.transaction(async |tx| {
        tx.execute("INSERT INTO later_book VALUES (1, 7)", &[])
            .await?;
        tx.execute("INSERT INTO shelf VALUES (7, 'late')", &[])
            .await
    })
    .await
    .expect("the shelf arrives before the commit");

    let refused = commit_refused(
        db.transaction(async |tx| {
            tx.execute("INSERT INTO later_book VALUES (2, 8)", &[])
                .await
        })
        .await,
    );
    refused_with(&refused, &SqlState::FOREIGN_KEY_VIOLATION);

    drop(db);
    ctx.delete().await;
    Ok(())
}
