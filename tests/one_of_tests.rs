#![allow(unused_imports, dead_code)]

//! A `OneOf` reference on a live server: the `CHECK` `create_one_of` writes,
//! refusing a row with two targets and, without a unit variant, one with
//! none; each reference written through `set` and read back as its key; and
//! the targets of a mixed list loaded with one statement per target table,
//! a key the relation finds no row for, and a relation joining another
//! column, refused before anything is sent.
//!
//! Run locally:
//! `DATABASE_URL="postgres://postgres:postgres@localhost:54319" cargo nextest run --test it one_of_tests::`

use crate::common;
pub use common::TestContext;

use common::statements::Statements;
use pgorm::metric::InstrumentedPool;
use pgorm::tests_cfg::scoped::{self, CompanyAdmin, CompanyId, Member, UserId};
use pgorm::tests_cfg::{
    comment::{self, Commentable, CommentableKey, CommentableModel},
    photo, post,
};
use pgorm::{
    Connection, ConnectionMut, DatabaseConnection, Error, OneOf, Schema, entity::prelude::*,
};
use pgorm_driver::error::SqlState;
use pretty_assertions::assert_eq;

/// A comment on a post or a photo, never on neither.
#[derive(Clone, Copy, Debug, OneOf)]
#[pgorm(entity = comment::Entity)]
pub enum Attached {
    #[pgorm(column = comment::Column::PostId)]
    Post(post::Entity),
    #[pgorm(column = comment::Column::PhotoId)]
    Photo(photo::Entity),
}

/// A note whose relation to `post` joins `legacy_post_id`, not the column
/// its `OneOf` names.
pub mod note {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "note")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub post_id: Option<i32>,
        pub legacy_post_id: Option<i32>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = pgorm::tests_cfg::post::Entity,
            from = Column::LegacyPostId,
            to = pgorm::tests_cfg::post::Column::Id
        )]
        Post,
    }
}

/// A pin whose relation to `post` carries a condition of its own.
pub mod pin {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "pin")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub post_id: Option<i32>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = pgorm::tests_cfg::post::Entity,
            from = Column::PostId,
            to = pgorm::tests_cfg::post::Column::Id,
            on_condition = pgorm::tests_cfg::post::Column::Title.like("%pinned%")
        )]
        Post,
    }
}

#[derive(Clone, Copy, Debug, OneOf)]
#[pgorm(entity = pin::Entity)]
pub enum Pinned {
    #[pgorm(column = pin::Column::PostId)]
    Post(post::Entity),
    Loose,
}

#[derive(Clone, Copy, Debug, OneOf)]
#[pgorm(entity = note::Entity)]
pub enum Noted {
    #[pgorm(column = note::Column::PostId)]
    Post(post::Entity),
    Loose,
}

/// A bookmark on a message, whose `UserId` scope is written by hand, or on a
/// note, scoped by `CompanyId` and `UserId` columns.
pub mod bookmark {
    use pgorm::entity::prelude::*;
    use pgorm::tests_cfg::scoped::{message, note};

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "bookmark")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub message_id: Option<i64>,
        pub note_id: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(
            belongs_to = message::Entity,
            from = Column::MessageId,
            to = message::Column::Id
        )]
        Message,
        #[pgorm(belongs_to = note::Entity, from = Column::NoteId, to = note::Column::Id)]
        Note,
    }
}

/// What a bookmark holds, its message declared first so that a load reads
/// `message` before `note`.
#[derive(Clone, Copy, Debug, OneOf)]
#[pgorm(entity = bookmark::Entity, model_attrs(derive(PartialEq, Eq)))]
pub enum Bookmarked {
    #[pgorm(column = bookmark::Column::MessageId)]
    Message(scoped::message::Entity),
    #[pgorm(column = bookmark::Column::NoteId)]
    Note(scoped::note::Entity),
}

/// Create the post, photo and comment tables, their foreign keys among them,
/// and `O`'s `CHECK`.
async fn create<O: OneOf, C: ConnectionMut>(db: &C) -> Result<(), Error> {
    let schema = Schema::new();
    for statement in [
        schema.create_table_from_entity(post::Entity)?,
        schema.create_table_from_entity(photo::Entity)?,
        schema.create_table_from_entity(comment::Entity)?,
    ] {
        db.execute(&statement.to_string(), &[]).await?;
    }
    db.execute(&schema.create_one_of::<O>().to_string(), &[])
        .await?;
    Ok(())
}

/// Two posts and two photos, keyed 1 and 2 each.
async fn targets<C: ConnectionMut>(db: &C) -> Result<(Vec<post::Model>, Vec<photo::Model>), Error> {
    let mut posts = Vec::new();
    let mut photos = Vec::new();
    for at in 1..=2 {
        posts.push(
            post::ActiveModel {
                title: set(format!("post {at}")),
                ..Default::default()
            }
            .insert(db)
            .await?,
        );
        photos.push(
            photo::ActiveModel {
                url: set(format!("{at}.png")),
                ..Default::default()
            }
            .insert(db)
            .await?,
        );
    }
    Ok((posts, photos))
}

/// A comment referencing `key`, written through `O::set`.
async fn comment_on<O: OneOf<Entity = comment::Entity>, C: ConnectionMut>(
    db: &C,
    body: &str,
    key: O::Key,
) -> Result<comment::Model, Error> {
    let mut active = comment::ActiveModel {
        body: set(body),
        ..Default::default()
    };
    O::set(&mut active, key)?;
    active.insert(db).await
}

async fn refused<C: ConnectionMut>(db: &C, sql: &str, state: &SqlState) {
    match db.execute(sql, &[]).await {
        Err(Error::Postgres(error)) => assert_eq!(error.code(), Some(state), "{error}"),
        other => panic!("expected {sql} refused with {state:?}, got {other:?}"),
    }
}

// [spec:pgorm:def:disjoint.one-of+4/test]    `create_one_of` adds the named CHECK `num_nonnulls(..) <= 1` where the enum has a unit variant: PostgreSQL refuses a row referencing two targets and accepts one referencing none
#[pgorm_macros::test]
async fn check_with_a_unit_variant_allows_none_live() -> Result<(), Error> {
    let ctx = TestContext::new("one_of_check_unit").await;
    let db = ctx.db.get().await?;
    create::<Commentable, _>(&db).await?;
    targets(&db).await?;

    let constraint = db
        .query_one(
            "SELECT conname::text AS name, pg_get_constraintdef(oid) AS def FROM pg_constraint \
             WHERE conrelid = 'comment'::regclass AND contype = 'c'",
            &[],
        )
        .await?;
    assert_eq!(
        (
            constraint.try_get::<_, String>("name")?,
            constraint.try_get::<_, String>("def")?
        ),
        (
            "ck-comment-commentable".to_owned(),
            "CHECK ((num_nonnulls(post_id, photo_id) <= 1))".to_owned()
        )
    );

    refused(
        &db,
        "INSERT INTO comment (body, post_id, photo_id) VALUES ('both', 1, 1)",
        &SqlState::CHECK_VIOLATION,
    )
    .await;
    db.execute("INSERT INTO comment (body) VALUES ('neither')", &[])
        .await?;
    db.execute(
        "INSERT INTO comment (body, photo_id) VALUES ('one', 2)",
        &[],
    )
    .await?;
    refused(
        &db,
        "INSERT INTO comment (body, post_id) VALUES ('dangling', 9)",
        &SqlState::FOREIGN_KEY_VIOLATION,
    )
    .await;

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:def:disjoint.one-of+4/test]    without a unit variant the CHECK is `num_nonnulls(..) = 1`: PostgreSQL refuses a row referencing none as well as one referencing two
#[pgorm_macros::test]
async fn check_without_a_unit_variant_requires_one_live() -> Result<(), Error> {
    let ctx = TestContext::new("one_of_check_exact").await;
    let db = ctx.db.get().await?;
    create::<Attached, _>(&db).await?;
    targets(&db).await?;

    refused(
        &db,
        "INSERT INTO comment (body) VALUES ('neither')",
        &SqlState::CHECK_VIOLATION,
    )
    .await;
    refused(
        &db,
        "INSERT INTO comment (body, post_id, photo_id) VALUES ('both', 1, 2)",
        &SqlState::CHECK_VIOLATION,
    )
    .await;
    db.execute("INSERT INTO comment (body, post_id) VALUES ('one', 1)", &[])
        .await?;
    let written = comment_on::<Attached, _>(&db, "photo", AttachedKey::Photo(2)).await?;
    assert_eq!(Attached::key(&written)?, AttachedKey::Photo(2));

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:def:disjoint.one-of+4/test]    each key written through `set` reads back as itself, the unit variant included, and setting another key on the row's `ActiveModel` sets its column and nulls the old one in the stored row
#[pgorm_macros::test]
async fn set_and_read_each_variant_live() -> Result<(), Error> {
    let ctx = TestContext::new("one_of_set_read").await;
    let db = ctx.db.get().await?;
    create::<Commentable, _>(&db).await?;
    targets(&db).await?;

    for key in [
        CommentableKey::Post(1),
        CommentableKey::Photo(2),
        CommentableKey::Detached,
    ] {
        let written = comment_on::<Commentable, _>(&db, "on", key.clone()).await?;
        let read = comment::Entity::find_by_id(written.id).one(&db).await?;
        assert_eq!(Commentable::key(&read)?, key);
    }

    let on_post = comment_on::<Commentable, _>(&db, "moved", CommentableKey::Post(2)).await?;
    let mut active = on_post.into_active();
    Commentable::set(&mut active, CommentableKey::Photo(1))?;
    let moved = active.update(&db).await?;
    assert_eq!((moved.post_id, moved.photo_id), (None, Some(1)));
    let stored = db
        .query_one(
            "SELECT post_id, photo_id FROM comment WHERE id = $1",
            &[&moved.id],
        )
        .await?;
    assert_eq!(
        (
            stored.try_get::<_, Option<i32>>("post_id")?,
            stored.try_get::<_, Option<i64>>("photo_id")?
        ),
        (None, Some(1))
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:def:disjoint.one-of+4/test]    loading a mixed list reads each target table once, in one statement for the keys its variant holds, and gives each row its `<Enum>Model` in order, the unit variant for a row referencing none; a list referencing one target reads one table, and an empty list reads none
#[pgorm_macros::test]
async fn load_reads_each_target_once_live() -> Result<(), Error> {
    let ctx = TestContext::new("one_of_load").await;
    let statements = Statements::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), statements.clone());
    let db = pool.get().await?;
    create::<Commentable, _>(&db).await?;
    let (posts, photos) = targets(&db).await?;
    for (body, key) in [
        ("a", CommentableKey::Photo(2)),
        ("b", CommentableKey::Post(1)),
        ("c", CommentableKey::Detached),
        ("d", CommentableKey::Post(2)),
        ("e", CommentableKey::Photo(1)),
        ("f", CommentableKey::Post(1)),
    ] {
        comment_on::<Commentable, _>(&db, body, key).await?;
    }
    let comments = comment::Entity::find()
        .order_by_asc(comment::Column::Id)
        .all(&db)
        .await?;
    statements.take();

    let loaded = Commentable::load(&comments, &db).await?;
    assert_eq!(
        loaded,
        [
            CommentableModel::Photo(photos[1].clone()),
            CommentableModel::Post(posts[0].clone()),
            CommentableModel::Detached,
            CommentableModel::Post(posts[1].clone()),
            CommentableModel::Photo(photos[0].clone()),
            CommentableModel::Post(posts[0].clone()),
        ]
    );
    let sent = statements.take();
    assert_eq!(sent.len(), 2, "{sent:#?}");
    assert!(
        sent[0].contains(r#"FROM "post""#) && !sent[0].contains(r#""photo""#),
        "{}",
        sent[0]
    );
    assert!(
        sent[1].contains(r#"FROM "photo""#) && !sent[1].contains(r#""post""#),
        "{}",
        sent[1]
    );

    let on_posts: Vec<comment::Model> = comments
        .iter()
        .filter(|comment| comment.post_id.is_some())
        .cloned()
        .collect();
    let loaded = Commentable::load(&on_posts, &db).await?;
    assert_eq!(
        loaded,
        [
            CommentableModel::Post(posts[0].clone()),
            CommentableModel::Post(posts[1].clone()),
            CommentableModel::Post(posts[0].clone()),
        ]
    );
    assert_eq!(statements.take().len(), 1);

    assert_eq!(Commentable::load(&[], &db).await?, []);
    assert_eq!(statements.take(), Vec::<String>::new());

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:def:disjoint.one-of+4/test]    a key its target table does not hold is an error naming the table, the key and the column; a row against its declaration fails the load as `key` fails it, before anything is sent
#[pgorm_macros::test]
async fn load_refuses_an_unmatched_key_live() -> Result<(), Error> {
    let ctx = TestContext::new("one_of_unmatched").await;
    let statements = Statements::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), statements.clone());
    let db = pool.get().await?;
    create::<Commentable, _>(&db).await?;
    targets(&db).await?;
    db.execute(
        r#"ALTER TABLE comment DROP CONSTRAINT "fk-comment-post_id""#,
        &[],
    )
    .await?;
    db.execute(
        "INSERT INTO comment (id, body, post_id) VALUES (7, 'dangling', 9)",
        &[],
    )
    .await?;
    let comments = comment::Entity::find().all(&db).await?;

    match Commentable::load(&comments, &db).await {
        Err(Error::Query(message)) => assert_eq!(
            message.to_string(),
            "the `comment` row whose key is (`id` = 7) references the `post` row of key 9 \
             through `post_id`, and `post` holds no such row"
        ),
        other => panic!("expected the unmatched key refused, got {other:?}"),
    }

    statements.take();
    let both = comment::Model {
        id: 8,
        body: "both".to_owned(),
        post_id: Some(1),
        photo_id: Some(1),
    };
    match Commentable::load(&[both], &db).await {
        Err(Error::Query(message)) => assert!(
            message
                .to_string()
                .contains("references more than one target"),
            "{message}"
        ),
        other => panic!("expected a row with two targets refused, got {other:?}"),
    }
    assert_eq!(statements.take(), Vec::<String>::new());

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:def:disjoint.one-of+4/test]    a relation to a target joining another column than the variant's is refused as `Error::Type` before anything is sent
#[pgorm_macros::test]
async fn load_refuses_a_relation_of_another_column_live() -> Result<(), Error> {
    let ctx = TestContext::new("one_of_relation").await;
    let statements = Statements::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), statements.clone());
    let db = pool.get().await?;
    let notes = [note::Model {
        id: 1,
        post_id: Some(1),
        legacy_post_id: None,
    }];

    match Noted::load(&notes, &db).await {
        Err(Error::Type(message)) => assert_eq!(
            message,
            "`note`.`post_id` holds the key of `post`, but `note`'s relation to `post` joins \
             `legacy_post_id` to `id`, and that relation is the foreign key holding the \
             reference: it must join `post_id` to `post`'s key `id` alone"
        ),
        other => panic!("expected the relation refused, got {other:?}"),
    }
    assert_eq!(statements.take(), Vec::<String>::new());

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:def:disjoint.one-of+4/test]    a relation to a target carrying an `on_condition` is refused as `Error::Type` before anything is sent, since the foreign key holding the reference holds its target row whatever the condition says
#[pgorm_macros::test]
async fn load_refuses_a_conditioned_relation_live() -> Result<(), Error> {
    let ctx = TestContext::new("one_of_conditioned").await;
    let statements = Statements::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), statements.clone());
    let db = pool.get().await?;
    let pins = [pin::Model {
        id: 1,
        post_id: Some(1),
    }];

    match Pinned::load(&pins, &db).await {
        Err(Error::Type(message)) => assert_eq!(
            message,
            "`pin`'s relation to `post` carries an on_condition, and the foreign key holding \
             `pin`.`post_id` holds its target row whatever a condition says: the relation a \
             one-of reference's target is loaded by joins `post_id` to `post`'s key alone"
        ),
        other => panic!("expected the relation refused, got {other:?}"),
    }
    assert_eq!(statements.take(), Vec::<String>::new());

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// The scoped tables `tests_cfg::scoped` declares and the bookmarks on them:
/// notes 1 and 2 are Acme's, by users 10 and 11, and note 3 Globex's;
/// messages 1 and 2 pass between users 10 and 11, and message 3 from 11 to
/// 20. Bookmarks 1 to 4 hold note 1, message 2, note 2 and message 3.
async fn bookmarks<C: ConnectionMut>(db: &C) -> Result<Vec<bookmark::Model>, Error> {
    db.batch_execute(
        "CREATE TABLE company (id bigint PRIMARY KEY, name text NOT NULL);
         CREATE TABLE note (
             id bigserial PRIMARY KEY,
             company_id bigint NOT NULL REFERENCES company (id),
             author_id bigint NOT NULL,
             body text NOT NULL
         );
         CREATE TABLE message (
             id bigserial PRIMARY KEY,
             sender_id bigint NOT NULL,
             recipient_id bigint NOT NULL,
             body text NOT NULL
         );
         CREATE TABLE bookmark (
             id integer PRIMARY KEY,
             message_id bigint REFERENCES message (id),
             note_id bigint REFERENCES note (id),
             CHECK (num_nonnulls(message_id, note_id) = 1)
         );
         INSERT INTO company VALUES (1, 'Acme'), (2, 'Globex');
         INSERT INTO note (company_id, author_id, body) VALUES
             (1, 10, 'mine'), (1, 11, 'a colleague''s'), (2, 20, 'another company''s');
         INSERT INTO message (sender_id, recipient_id, body) VALUES
             (10, 11, 'sent'), (11, 10, 'received'), (11, 20, 'not mine');
         INSERT INTO bookmark VALUES (1, NULL, 1), (2, 2, NULL), (3, NULL, 2), (4, 3, NULL);",
    )
    .await?;
    bookmark::Entity::find()
        .order_by_asc(bookmark::Column::Id)
        .all(db)
        .await
}

fn bodies(models: &[BookmarkedModel]) -> Vec<&str> {
    models
        .iter()
        .map(|model| match model {
            BookmarkedModel::Message(message) => message.body.as_str(),
            BookmarkedModel::Note(note) => note.body.as_str(),
        })
        .collect()
}

// [spec:pgorm:sem:scope.read/test]    a one-of `load_for` reads each target under its table's scopes: a member's own note and message load, a reference to a colleague's note or another user's message is the load's absent-key error, and an administrator lifting `UserId` loads every row of the company and every message
#[pgorm_macros::test]
async fn load_for_reads_targets_under_scopes_live() -> Result<(), Error> {
    let ctx = TestContext::new("one_of_scoped").await;
    let statements = Statements::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), statements.clone());
    let db = pool.get().await?;
    let marks = bookmarks(&db).await?;
    let member = Member {
        company: CompanyId(1),
        user: UserId(10),
    };
    statements.take();

    let own = Bookmarked::load_for(&member, &marks[..2], &db).await?;
    assert_eq!(bodies(&own), ["mine", "received"]);
    let sent = statements.take();
    assert_eq!(sent.len(), 2, "{sent:#?}");
    assert!(
        sent[0].contains(r#"FROM "message""#)
            && sent[0].contains(r#""message"."sender_id" = $"#)
            && sent[0].contains(r#""message"."recipient_id" = $"#),
        "{}",
        sent[0]
    );
    assert!(
        sent[1].contains(r#"FROM "note""#)
            && sent[1].contains(r#""note"."company_id" = $"#)
            && sent[1].contains(r#""note"."author_id" = $"#),
        "{}",
        sent[1]
    );

    match Bookmarked::load_for(&member, &marks[2..3], &db).await {
        Err(Error::Query(message)) => assert_eq!(
            message.to_string(),
            "the `bookmark` row whose key is (`id` = 3) references the `note` row of key 2 \
             through `note_id`, and `note` holds no such row"
        ),
        other => panic!("expected a colleague's note read as absent, got {other:?}"),
    }
    match Bookmarked::load_for(&member, &marks[3..], &db).await {
        Err(Error::Query(message)) => assert!(
            message.to_string().contains("the `message` row of key 3"),
            "{message}"
        ),
        other => panic!("expected another user's message read as absent, got {other:?}"),
    }

    let admin = CompanyAdmin {
        company: CompanyId(1),
        users: Every::new(),
    };
    let company = Bookmarked::load_for(&admin, &marks, &db).await?;
    assert_eq!(
        bodies(&company),
        ["mine", "received", "a colleague's", "not mine"]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:scope.refusals+1/test]    a one-of load whose context does not hold a target's scopes is refused with `NotHeld` before any target is read, whichever targets its rows reference, the plain `load` included
#[pgorm_macros::test]
async fn load_for_refuses_unheld_target_scopes_live() -> Result<(), Error> {
    let ctx = TestContext::new("one_of_unscoped").await;
    let statements = Statements::default();
    let pool = InstrumentedPool::new(ctx.db.clone(), statements.clone());
    let db = pool.get().await?;
    let marks = bookmarks(&db).await?;
    statements.take();

    let not_held =
        |loaded: Result<Vec<BookmarkedModel>, Error>, table: &str, scope: &str| match loaded {
            Err(Error::Scope(ScopeError::NotHeld {
                table: refused,
                scope: lacking,
            })) => assert_eq!((refused.as_str(), lacking), (table, scope)),
            other => panic!("expected `{table}` refused for `{scope}`, got {other:?}"),
        };
    not_held(
        Bookmarked::load_for(&UserId(10), &marks, &db).await,
        "note",
        "CompanyId",
    );
    not_held(
        Bookmarked::load_for(&UserId(10), &marks[1..2], &db).await,
        "note",
        "CompanyId",
    );
    not_held(Bookmarked::load(&marks, &db).await, "message", "UserId");
    assert_eq!(statements.take(), Vec::<String>::new());

    drop(db);
    ctx.delete().await;
    Ok(())
}
