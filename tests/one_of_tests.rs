#![allow(unused_imports, dead_code)]

//! A `OneOf` reference on a live server: the `CHECK` `create_one_of` writes,
//! refusing a row with two targets and, without a unit variant, one with
//! none; each reference written through `set` and read back as its key; and
//! the targets of a mixed list loaded with one statement per target table,
//! a key the relation finds no row for, and a relation joining another
//! column, refused before anything is sent.
//!
//! Run locally:
//! `DATABASE_URL="postgres://postgres:postgres@localhost:54329" cargo nextest run --test one_of_tests`

pub mod common;
pub use common::TestContext;

use async_trait::async_trait;
use pgorm::metric::{InstrumentedPool, MetricsCollector, QueryContext};
use pgorm::tests_cfg::{
    comment::{self, Commentable, CommentableKey, CommentableModel},
    photo, post,
};
use pgorm::{ConnectionTrait, DatabaseConnection, Error, OneOf, Schema, entity::prelude::*};
use pretty_assertions::assert_eq;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_postgres::error::SqlState;

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

#[derive(Clone, Copy, Debug, OneOf)]
#[pgorm(entity = note::Entity)]
pub enum Noted {
    #[pgorm(column = note::Column::PostId)]
    Post(post::Entity),
    Loose,
}

/// The SQL of every statement a connection sends.
#[derive(Clone, Debug, Default)]
struct Statements(Arc<Mutex<Vec<String>>>);

impl Statements {
    fn push(&self, query: QueryContext<'_>) {
        if let Ok(mut sent) = self.0.lock() {
            sent.push(query.sql().unwrap_or_default().to_owned());
        }
    }

    fn take(&self) -> Vec<String> {
        self.0
            .lock()
            .map(|mut sent| std::mem::take(&mut *sent))
            .unwrap_or_default()
    }
}

#[async_trait]
impl MetricsCollector for Statements {
    async fn record_query_success(
        &self,
        query: QueryContext<'_>,
        _duration: Duration,
        _rows: Option<u64>,
    ) {
        self.push(query);
    }

    async fn record_query_error(&self, query: QueryContext<'_>, _duration: Duration, _: &Error) {
        self.push(query);
    }

    async fn record_connection_acquired(&self, _duration: Duration) {}

    async fn record_connection_error(&self, _duration: Duration, _error: &Error) {}

    async fn record_transaction_begin(&self, _duration: Duration) {}

    async fn record_transaction_commit(&self, _duration: Duration) {}

    async fn record_transaction_rollback(&self, _duration: Duration) {}
}

/// Create the post, photo and comment tables, their foreign keys among them,
/// and `O`'s `CHECK`.
async fn create<O: OneOf, C: ConnectionTrait>(db: &C) -> Result<(), Error> {
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
async fn targets<C: ConnectionTrait>(
    db: &C,
) -> Result<(Vec<post::Model>, Vec<photo::Model>), Error> {
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
async fn comment_on<O: OneOf<Entity = comment::Entity>, C: ConnectionTrait>(
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

async fn refused<C: ConnectionTrait>(db: &C, sql: &str, state: &SqlState) {
    match db.execute(sql, &[]).await {
        Err(Error::Postgres(error)) => assert_eq!(error.code(), Some(state), "{error}"),
        other => panic!("expected {sql} refused with {state:?}, got {other:?}"),
    }
}

// [spec:pgorm:def:disjoint.one-of+3/test]    `create_one_of` adds the named CHECK `num_nonnulls(..) <= 1` where the enum has a unit variant: PostgreSQL refuses a row referencing two targets and accepts one referencing none
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

// [spec:pgorm:def:disjoint.one-of+3/test]    without a unit variant the CHECK is `num_nonnulls(..) = 1`: PostgreSQL refuses a row referencing none as well as one referencing two
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

// [spec:pgorm:def:disjoint.one-of+3/test]    each key written through `set` reads back as itself, the unit variant included, and setting another key on the row's `ActiveModel` sets its column and nulls the old one in the stored row
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

// [spec:pgorm:def:disjoint.one-of+3/test]    loading a mixed list reads each target table once, in one statement for the keys its variant holds, and gives each row its `<Enum>Model` in order, the unit variant for a row referencing none; a list referencing one target reads one table, and an empty list reads none
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

// [spec:pgorm:def:disjoint.one-of+3/test]    a key its target table does not hold, which `load_one` gives as `None`, is an error naming the table, the key and the column; a row against its declaration fails the load as `key` fails it, before anything is sent
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
             through `post_id`, and reading `comment`'s relation to `post` finds no such row"
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

// [spec:pgorm:def:disjoint.one-of+3/test]    a relation to a target joining another column than the variant's is refused as `Error::Type` before anything is sent
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
             `legacy_post_id` to `id`, and loading reads that relation: it must join `post_id` \
             to `post`'s key `id` alone"
        ),
        other => panic!("expected the relation refused, got {other:?}"),
    }
    assert_eq!(statements.take(), Vec::<String>::new());

    drop(db);
    ctx.delete().await;
    Ok(())
}
