#![allow(unused_imports, dead_code)]

//! Composite primary keys against a live PostgreSQL server: the multi-tenant
//! shape whose second column the database generates, and a small library —
//! shelves, books, authors and the junction between them, every key and
//! foreign key two or three columns wide — through each write and lookup path
//! a composite key takes.

pub mod common;

pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::OnConflict;
use pgorm::{ConnectionTrait, Error, QueryTrait, Schema, TryInsertResult, entity::prelude::*};
use pretty_assertions::assert_eq;
use std::borrow::Cow;
use tokio_postgres::error::SqlState;

/// The tenant is supplied, the row number generated: `id` is an identity
/// inside the key, and nothing else about the table is unusual.
mod ticket {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[pgorm(table_name = "ticket")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub tenant_id: i32,
        #[pgorm(primary_key, identity)]
        pub id: i64,
        pub title: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// The same key with the generated part `BY DEFAULT`: a supplied number is
/// kept, an omitted one drawn from the sequence.
mod imported_ticket {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[pgorm(table_name = "imported_ticket")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub tenant_id: i32,
        #[pgorm(primary_key, identity_by_default)]
        pub id: i32,
        pub title: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// A tenant's shelf, keyed by the tenant and a code the tenant chose: a
/// composite key with a text part, which a lookup names with a borrowed `&str`.
mod shelf {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[pgorm(table_name = "shelf")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub tenant_id: i32,
        #[pgorm(primary_key)]
        pub code: String,
        pub label: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter)]
    pub enum Relation {
        Book,
    }

    impl RelationTrait for Relation {
        fn def(&self) -> RelationDef {
            match self {
                Self::Book => Entity::has_many(super::book::Entity).into(),
            }
        }
    }

    impl Related<super::book::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Book.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// A book on a shelf: its own key is the tenant and a generated number, and it
/// reaches its shelf through a two-column foreign key.
mod book {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[pgorm(table_name = "book")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub tenant_id: i32,
        #[pgorm(primary_key, identity)]
        pub id: i64,
        pub shelf_code: String,
        pub title: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter)]
    pub enum Relation {
        Shelf,
    }

    impl RelationTrait for Relation {
        fn def(&self) -> RelationDef {
            match self {
                Self::Shelf => Entity::belongs_to(super::shelf::Entity)
                    .columns(Column::TenantId, super::shelf::Column::TenantId)
                    .and_columns(Column::ShelfCode, super::shelf::Column::Code)
                    .into(),
            }
        }
    }

    impl Related<super::shelf::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Shelf.def()
        }
    }

    impl Related<super::author::Entity> for Entity {
        fn to() -> RelationDef {
            super::book_author::Relation::Author.def()
        }

        fn via() -> Option<RelationDef> {
            Some(super::book_author::Relation::Book.def().rev())
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// An author, keyed by the tenant and a number the caller supplies.
mod author {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[pgorm(table_name = "author")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub tenant_id: i32,
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter)]
    pub enum Relation {}

    impl RelationTrait for Relation {
        fn def(&self) -> RelationDef {
            match *self {}
        }
    }

    impl Related<super::book::Entity> for Entity {
        fn to() -> RelationDef {
            super::book_author::Relation::Book.def()
        }

        fn via() -> Option<RelationDef> {
            Some(super::book_author::Relation::Author.def().rev())
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// The junction between books and authors: a three-column key, two
/// two-column foreign keys sharing the tenant.
mod book_author {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[pgorm(table_name = "book_author")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub tenant_id: i32,
        #[pgorm(primary_key)]
        pub book_id: i64,
        #[pgorm(primary_key)]
        pub author_id: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter)]
    pub enum Relation {
        Book,
        Author,
    }

    impl RelationTrait for Relation {
        fn def(&self) -> RelationDef {
            match self {
                Self::Book => Entity::belongs_to(super::book::Entity)
                    .columns(Column::TenantId, super::book::Column::TenantId)
                    .and_columns(Column::BookId, super::book::Column::Id)
                    .into(),
                Self::Author => Entity::belongs_to(super::author::Entity)
                    .columns(Column::TenantId, super::author::Column::TenantId)
                    .and_columns(Column::AuthorId, super::author::Column::Id)
                    .into(),
            }
        }
    }

    impl Related<super::book::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Book.def()
        }
    }

    impl Related<super::author::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Author.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

/// Where a tenant keeps a thing: keyed by the tenant and two codes the tenant
/// chose, a three-column key with two text parts, which a lookup names with
/// borrowed parts in more than one position.
mod placement {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[pgorm(table_name = "placement")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub tenant_id: i32,
        #[pgorm(primary_key)]
        pub room: String,
        #[pgorm(primary_key)]
        pub slot: String,
        pub item: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

async fn create_from_entity<C, E>(db: &C, entity: E) -> Result<(), Error>
where
    C: ConnectionTrait,
    E: EntityTrait,
{
    let stmt = Schema::new().create_table_from_entity(entity)?;
    db.execute(&stmt.to_string(), &[]).await?;
    Ok(())
}

fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

fn new_ticket(tenant_id: i32, title: &str) -> ticket::ActiveModel {
    ticket::ActiveModel {
        tenant_id: set(tenant_id),
        title: set(title),
        ..Default::default()
    }
}

// [spec:pgorm:sem:schema.from-entity+9/test]    the generated part of a
// composite key gets its identity in the table schema-gen builds, and nothing
// else in the key does
// [spec:pgorm:sem:exec.crud.insert+6/test]    an insert leaving the generated
// part NotSet names no value for it, and the key it reports is the whole tuple
// the database wrote, generated part included
#[pgorm_macros::test]
async fn generated_key_part_comes_back_in_the_tuple() -> Result<(), Error> {
    let ctx = TestContext::new("composite_key_generated_part").await;
    let db = ctx.db.get().await?;
    create_from_entity(&db, ticket::Entity).await?;

    let insert = Insert::one(new_ticket(7, "first"));
    assert_eq!(
        insert.as_query().to_string(),
        r#"INSERT INTO "ticket" ("tenant_id", "title") VALUES (7, 'first')"#,
        "the generated column is not named"
    );
    assert_eq!(insert.exec_returning_pk(&db).await?, (7, 1));
    assert_eq!(
        Insert::one(new_ticket(8, "second"))
            .exec_returning_pk(&db)
            .await?,
        (8, 2)
    );

    let third = Insert::one(new_ticket(7, "third"))
        .exec_returning_model(&db)
        .await?;
    assert_eq!(
        third,
        ticket::Model {
            tenant_id: 7,
            id: 3,
            title: "third".to_owned(),
        }
    );
    let fourth = new_ticket(8, "fourth").insert(&db).await?;
    assert_eq!((fourth.tenant_id, fourth.id), (8, 4));

    assert_eq!(
        ticket::Entity::find_by_id((7, 3)).one(&db).await?.title,
        "third"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:exec.crud.insert+6/test]    a many-row insert leaving the
// generated part NotSet writes every row with its own generated number, and
// reports every row's whole key tuple
// [spec:pgorm:sem:exec.crud.insert-returning+3/test]    and every row's model
#[pgorm_macros::test]
async fn many_rows_each_get_a_generated_key_part() -> Result<(), Error> {
    let ctx = TestContext::new("composite_key_generated_many").await;
    let db = ctx.db.get().await?;
    create_from_entity(&db, ticket::Entity).await?;

    let keys = Insert::many([new_ticket(1, "a"), new_ticket(2, "b"), new_ticket(1, "c")])
        .exec_returning_pks(&db)
        .await?;
    assert_eq!(keys, [(1, 1), (2, 2), (1, 3)]);

    let models = Insert::many([new_ticket(3, "d"), new_ticket(3, "e")])
        .exec_returning_models(&db)
        .await?;
    let written: Vec<_> = models
        .iter()
        .map(|model| (model.tenant_id, model.id, model.title.as_str()))
        .collect();
    assert_eq!(written, [(3, 4, "d"), (3, 5, "e")]);

    let keys: Vec<(i32, i64)> = ticket::Entity::find()
        .order_by_asc(ticket::Column::Id)
        .all(&db)
        .await?
        .into_iter()
        .map(|row| (row.tenant_id, row.id))
        .collect();
    assert_eq!(keys, [(1, 1), (2, 2), (1, 3), (3, 4), (3, 5)]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:macros.derive.entity-model.primary-key+7/test]    `identity`
// is `GENERATED ALWAYS`: a number the insert supplies is refused by the server
// (428C9), so the generated part cannot be claimed by accident;
// `identity_by_default` keeps a supplied number and generates an omitted one
#[pgorm_macros::test]
async fn identity_forms_answer_a_supplied_key_part() -> Result<(), Error> {
    let ctx = TestContext::new("composite_key_identity_forms").await;
    let db = ctx.db.get().await?;
    create_from_entity(&db, ticket::Entity).await?;
    create_from_entity(&db, imported_ticket::Entity).await?;

    let claimed = Insert::one(ticket::ActiveModel {
        tenant_id: set(1),
        id: set(42),
        title: set("claimed"),
    })
    .exec_returning_pk(&db)
    .await;
    refused_with(
        &claimed.expect_err("ALWAYS refuses a supplied value"),
        &SqlState::GENERATED_ALWAYS,
    );

    let kept = Insert::one(imported_ticket::ActiveModel {
        tenant_id: set(1),
        id: set(42),
        title: set("kept"),
    })
    .exec_returning_pk(&db)
    .await?;
    assert_eq!(kept, (1, 42));
    let drawn = Insert::one(imported_ticket::ActiveModel {
        tenant_id: set(1),
        title: set("drawn"),
        ..Default::default()
    })
    .exec_returning_pk(&db)
    .await?;
    assert_eq!(drawn, (1, 1));

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// Two tenants' shelves, books, authors and authorships: tenant 1 has shelves
/// `a` and `b` (`b` empty), books 1 and 2 on `a`, authors 1 and 2, with book 1
/// by both and book 2 by none; tenant 2 has shelf `a` holding book 3 by its own
/// author 1, so every key part has to be compared for a join to stay inside
/// one tenant.
async fn seed_library<C>(db: &C) -> Result<Library, Error>
where
    C: ConnectionTrait,
{
    for entity_stmt in [
        Schema::new().create_table_from_entity(shelf::Entity)?,
        Schema::new().create_table_from_entity(author::Entity)?,
        Schema::new().create_table_from_entity(book::Entity)?,
        Schema::new().create_table_from_entity(book_author::Entity)?,
    ] {
        db.execute(&entity_stmt.to_string(), &[]).await?;
    }

    let shelves = Insert::many(
        [(1, "a", "Fiction"), (1, "b", "Empty"), (2, "a", "Poetry")].map(
            |(tenant_id, code, label)| shelf::ActiveModel {
                tenant_id: set(tenant_id),
                code: set(code),
                label: set(label),
            },
        ),
    )
    .exec_returning_models(db)
    .await?;
    let authors = Insert::many([(1, 1, "Ann"), (1, 2, "Bo"), (2, 1, "Cy")].map(
        |(tenant_id, id, name)| author::ActiveModel {
            tenant_id: set(tenant_id),
            id: set(id),
            name: set(name),
        },
    ))
    .exec_returning_models(db)
    .await?;
    let books = Insert::many([(1, "a", "One"), (1, "a", "Two"), (2, "a", "Three")].map(
        |(tenant_id, shelf_code, title)| book::ActiveModel {
            tenant_id: set(tenant_id),
            shelf_code: set(shelf_code),
            title: set(title),
            ..Default::default()
        },
    ))
    .exec_returning_models(db)
    .await?;
    Insert::many(
        [(1, 1, 1), (1, 1, 2), (2, 3, 1)].map(|(tenant_id, book_id, author_id)| {
            book_author::ActiveModel {
                tenant_id: set(tenant_id),
                book_id: set(book_id),
                author_id: set(author_id),
            }
        }),
    )
    .exec(db)
    .await?;

    Ok(Library {
        shelves,
        authors,
        books,
    })
}

struct Library {
    shelves: Vec<shelf::Model>,
    authors: Vec<author::Model>,
    books: Vec<book::Model>,
}

// [spec:pgorm:def:entity.traits.primary-key+7/test]    a composite key takes
// its parts borrowed, each converted into the key's part in its position
// [spec:pgorm:req:entity.traits.crud+4/test]    find_by_id and delete_by_id
// filter on every key column, so a key differing only in its tenant names
// another row
#[pgorm_macros::test]
async fn key_lookups_take_borrowed_parts() -> Result<(), Error> {
    let ctx = TestContext::new("composite_key_borrowed_parts").await;
    let db = ctx.db.get().await?;
    let library = seed_library(&db).await?;

    let fiction = shelf::Entity::find_by_id((1, "a")).one(&db).await?;
    assert_eq!(fiction, library.shelves[0]);
    let code = String::from("a");
    assert_eq!(
        shelf::Entity::find_by_id((2, &code)).one(&db).await?.label,
        "Poetry"
    );
    assert_eq!(
        shelf::Entity::find_by_id((2, Cow::Borrowed("b")))
            .one_opt(&db)
            .await?,
        None
    );

    assert_eq!(shelf::Entity::delete_by_id((1, "b")).exec(&db).await?, 1);
    assert_eq!(shelf::Entity::delete_by_id((1, "b")).exec(&db).await?, 0);
    assert_eq!(
        book::Entity::find_by_id((2, 3)).one(&db).await?.title,
        "Three"
    );
    assert_eq!(
        book_author::Entity::find_by_id((1, 1, 2)).one(&db).await?,
        book_author::Model {
            tenant_id: 1,
            book_id: 1,
            author_id: 2,
        }
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:def:entity.traits.primary-key+7/test]    a key of three columns
// takes its text parts borrowed, as &str, &String or Cow, in any position, each
// converted into the key's part in its position
// [spec:pgorm:req:entity.traits.crud+4/test]    find_by_id and delete_by_id
// filter on all three columns, so keys differing in one part name other rows
#[pgorm_macros::test]
async fn three_part_lookups_take_borrowed_parts() -> Result<(), Error> {
    let ctx = TestContext::new("composite_key_three_part_borrowed").await;
    let db = ctx.db.get().await?;
    create_from_entity(&db, placement::Entity).await?;
    let placed = Insert::many(
        [
            (1, "hall", "top", "lamp"),
            (1, "hall", "low", "boots"),
            (1, "attic", "top", "trunk"),
            (2, "hall", "top", "coat"),
        ]
        .map(|(tenant_id, room, slot, item)| placement::ActiveModel {
            tenant_id: set(tenant_id),
            room: set(room),
            slot: set(slot),
            item: set(item),
        }),
    )
    .exec_returning_models(&db)
    .await?;

    assert_eq!(
        placement::Entity::find_by_id((1, "hall", "top"))
            .one(&db)
            .await?,
        placed[0]
    );
    let room = String::from("hall");
    assert_eq!(
        placement::Entity::find_by_id((1, &room, Cow::Borrowed("low")))
            .one(&db)
            .await?
            .item,
        "boots"
    );
    assert_eq!(
        placement::Entity::find_by_id((2, Cow::<str>::Owned(room.clone()), "top"))
            .one(&db)
            .await?
            .item,
        "coat"
    );
    assert_eq!(
        placement::Entity::find_by_id((2, "attic", "top"))
            .one_opt(&db)
            .await?,
        None
    );

    assert_eq!(
        placement::Entity::delete_by_id((1, Cow::Borrowed("attic"), &String::from("top")))
            .exec(&db)
            .await?,
        1
    );
    assert_eq!(
        placement::Entity::delete_by_id((1, "attic", "top"))
            .exec(&db)
            .await?,
        0
    );
    assert_eq!(placement::Entity::find().all(&db).await?.len(), 3);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:exec.crud.insert+6/test]    a batch's keys come back one per
// row written, in the order the models were added — not sorted by key — and an
// empty batch has none
// [spec:pgorm:sem:exec.crud.insert-returning+3/test]    so do its models
// [spec:pgorm:sem:exec.crud.try-insert+4/test]    a batch the conflict clause
// skipped whole is Conflicted; one it skipped in part reports the rows written
#[pgorm_macros::test]
async fn batch_returns_every_key_in_order() -> Result<(), Error> {
    let ctx = TestContext::new("composite_key_batch_keys").await;
    let db = ctx.db.get().await?;
    seed_library(&db).await?;

    let author = |id: i32, name: &str| author::ActiveModel {
        tenant_id: set(1),
        id: set(id),
        name: set(name),
    };
    let keys = Insert::many([author(9, "Di"), author(5, "Ed"), author(7, "Fe")])
        .exec_returning_pks(&db)
        .await?;
    assert_eq!(keys, [(1, 9), (1, 5), (1, 7)]);

    let models = Insert::many([author(8, "Gu"), author(6, "Ha")])
        .exec_returning_models(&db)
        .await?;
    let names: Vec<_> = models
        .iter()
        .map(|model| (model.id, model.name.as_str()))
        .collect();
    assert_eq!(names, [(8, "Gu"), (6, "Ha")]);

    let none: Vec<(i32, i32)> = Insert::many(std::iter::empty::<author::ActiveModel>())
        .exec_returning_pks(&db)
        .await?;
    assert_eq!(none, []);

    let skipped = Insert::many([author(9, "Di"), author(5, "Ed")])
        .on_conflict_do_nothing()
        .exec_returning_pks(&db)
        .await?;
    assert!(
        matches!(skipped, TryInsertResult::Conflicted),
        "{skipped:?}"
    );
    let partial = Insert::many([author(9, "Di"), author(4, "Io")])
        .on_conflict_do_nothing()
        .exec_returning_pks(&db)
        .await?;
    assert!(
        matches!(partial, TryInsertResult::Inserted(ref keys) if keys == &[(1, 4)]),
        "{partial:?}"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:entity.active-model.persistence+2/test]    an ActiveModel
// over a composite key updates and deletes the one row its whole key names, and
// a key with a part NotSet is refused before any SQL
#[pgorm_macros::test]
async fn active_model_writes_need_the_whole_key() -> Result<(), Error> {
    let ctx = TestContext::new("composite_key_active_model").await;
    let db = ctx.db.get().await?;
    let library = seed_library(&db).await?;

    let mut renamed = library.authors[0].clone().into_active();
    renamed.name = set("Anne");
    let updated = renamed.update(&db).await?;
    assert_eq!(
        (updated.tenant_id, updated.id, updated.name.as_str()),
        (1, 1, "Anne")
    );
    assert_eq!(
        author::Entity::find_by_id((2, 1)).one(&db).await?.name,
        "Cy",
        "the other tenant's author 1 is untouched"
    );

    let half_key = author::ActiveModel {
        tenant_id: set(1),
        name: set("Nobody"),
        ..Default::default()
    };
    assert!(matches!(
        half_key.clone().update(&db).await,
        Err(Error::PrimaryKeyNotSet)
    ));
    assert!(matches!(
        half_key.delete(&db).await,
        Err(Error::PrimaryKeyNotSet)
    ));

    let removed = book_author::Entity::find_by_id((1, 1, 2))
        .one(&db)
        .await?
        .into_active()
        .delete(&db)
        .await?;
    assert_eq!(removed, 1);
    assert_eq!(book_author::Entity::find().all(&db).await?.len(), 2);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:exec.crud.insert+6/test]    an upsert arbitrated by the whole
// composite key reports the key and the model of the row it updated
// [spec:pgorm:req:sql.ast.on-conflict+4/test]    the one-call composite target
// arbitrates the two-column key live
// [spec:pgorm:sem:query.build.insert.empty-failsafe+5/test]
// `on_conflict_do_nothing` names every key column, so a duplicate of the whole
// key is Conflicted
#[pgorm_macros::test]
async fn upsert_on_the_whole_key() -> Result<(), Error> {
    let ctx = TestContext::new("composite_key_upsert").await;
    let db = ctx.db.get().await?;
    seed_library(&db).await?;

    let renamed = |name: &str| author::ActiveModel {
        tenant_id: set(2),
        id: set(1),
        name: set(name),
    };
    let whole_key = || {
        OnConflict::columns((author::Column::TenantId, author::Column::Id))
            .update_column(author::Column::Name)
    };

    let model = Insert::one(renamed("Cyd"))
        .on_conflict(whole_key())
        .exec_returning_model(&db)
        .await?;
    assert_eq!(
        (model.tenant_id, model.id, model.name.as_str()),
        (2, 1, "Cyd")
    );
    let key = Insert::one(renamed("Cyrus"))
        .on_conflict(whole_key())
        .exec_returning_pk(&db)
        .await?;
    assert_eq!(key, (2, 1));
    assert_eq!(
        author::Entity::find_by_id((2, 1)).one(&db).await?.name,
        "Cyrus"
    );
    assert_eq!(
        author::Entity::find_by_id((1, 1)).one(&db).await?.name,
        "Ann"
    );

    let res = Insert::one(renamed("Ignored"))
        .on_conflict_do_nothing()
        .exec_returning_pk(&db)
        .await?;
    assert!(matches!(res, TryInsertResult::Conflicted), "{res:?}");

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:query.loader+1/test]    the loaders batch a composite foreign
// key and a composite junction, each bucket keeping to its own tenant, and an
// input with nothing related gets an empty bucket
#[pgorm_macros::test]
async fn loaders_batch_over_composite_keys() -> Result<(), Error> {
    let ctx = TestContext::new("composite_key_loaders").await;
    let db = ctx.db.get().await?;
    let library = seed_library(&db).await?;
    let shelves = &library.shelves;
    let books = &library.books;
    let authors = &library.authors;

    let homes = books.load_one(shelf::Entity, &db).await?;
    assert_eq!(
        homes,
        [
            Some(shelves[0].clone()),
            Some(shelves[0].clone()),
            Some(shelves[2].clone())
        ]
    );

    let mut held = shelves.load_many(book::Entity, &db).await?;
    for bucket in &mut held {
        bucket.sort_by_key(|book| book.id);
    }
    assert_eq!(
        held,
        [
            vec![books[0].clone(), books[1].clone()],
            vec![],
            vec![books[2].clone()]
        ]
    );

    let mut written = books.load_many_via(author::Entity, &db).await?;
    for bucket in &mut written {
        bucket.sort_by_key(|author| author.id);
    }
    assert_eq!(
        written,
        [
            vec![authors[0].clone(), authors[1].clone()],
            vec![],
            vec![authors[2].clone()]
        ]
    );
    let wrote = authors.load_many_via(book::Entity, &db).await?;
    assert_eq!(
        wrote,
        [
            vec![books[0].clone()],
            vec![books[0].clone()],
            vec![books[2].clone()]
        ]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// Shelf to authorship, two hops through the book.
struct ShelfAuthorships;

impl Linked for ShelfAuthorships {
    type FromEntity = shelf::Entity;
    type ToEntity = book_author::Entity;

    fn link(&self) -> Vec<RelationDef> {
        vec![
            shelf::Relation::Book.def(),
            book_author::Relation::Book.def().rev(),
        ]
    }
}

/// Shelf to author, three hops through the book and the authorship.
struct ShelfAuthors;

impl Linked for ShelfAuthors {
    type FromEntity = shelf::Entity;
    type ToEntity = author::Entity;

    fn link(&self) -> Vec<RelationDef> {
        vec![
            shelf::Relation::Book.def(),
            book_author::Relation::Book.def().rev(),
            book_author::Relation::Author.def(),
        ]
    }
}

// [spec:pgorm:def:entity.traits.model+3/test]    find_related walks a composite
// foreign key both ways and find_linked over two and three hops, each join
// comparing every key column, so tenant 2's rows never reach tenant 1's
#[pgorm_macros::test]
async fn related_and_linked_walk_composite_keys() -> Result<(), Error> {
    let ctx = TestContext::new("composite_key_related_linked").await;
    let db = ctx.db.get().await?;
    let library = seed_library(&db).await?;
    let fiction = &library.shelves[0];

    let on_fiction = fiction
        .find_related(book::Entity)
        .order_by_asc(book::Column::Id)
        .all(&db)
        .await?;
    assert_eq!(on_fiction, library.books[..2]);
    assert_eq!(
        library.books[2]
            .find_related(shelf::Entity)
            .all(&db)
            .await?,
        [library.shelves[2].clone()]
    );

    let authorships = fiction
        .find_linked(ShelfAuthorships)
        .order_by_asc(book_author::Column::AuthorId)
        .all(&db)
        .await?;
    let pairs: Vec<_> = authorships
        .iter()
        .map(|row| (row.tenant_id, row.book_id, row.author_id))
        .collect();
    assert_eq!(pairs, [(1, 1, 1), (1, 1, 2)]);

    let fiction_authors = fiction
        .find_linked(ShelfAuthors)
        .order_by_asc(author::Column::Id)
        .all(&db)
        .await?;
    assert_eq!(fiction_authors, library.authors[..2]);
    let poets = library.shelves[2]
        .find_linked(ShelfAuthors)
        .all(&db)
        .await?;
    assert_eq!(poets, [library.authors[2].clone()]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:query.graph.cursor+4/test]    a graph rooted at a composite
// key completes its cursor key with both key columns, so `after_with` resumes
// at arity three from inside a run of equal labels
#[pgorm_macros::test]
async fn composite_root_graph_resumes_mid_run() -> Result<(), Error> {
    let ctx = TestContext::new("composite_key_graph_cursor").await;
    let db = ctx.db.get().await?;
    seed_library(&db).await?;
    Insert::many(
        [(1, "c"), (2, "c"), (3, "c")].map(|(tenant_id, code)| shelf::ActiveModel {
            tenant_id: set(tenant_id),
            code: set(code),
            label: set("Same"),
        }),
    )
    .exec(&db)
    .await?;

    let keys = |rows: Vec<shelf::Model>| {
        rows.into_iter()
            .map(|shelf| (shelf.tenant_id, shelf.code))
            .collect::<Vec<_>>()
    };
    let same = || shelf::Entity::graph().cursor_by(shelf::Column::Label);

    let page = same()
        .after_with(("Same", 1, "c"))
        .first(2)
        .all(&db)
        .await?;
    assert_eq!(keys(page), [(2, "c".to_owned()), (3, "c".to_owned())]);
    let page = same().after("Same").first(2).all(&db).await?;
    assert!(
        keys(page).is_empty(),
        "the order column alone skips the run"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}
