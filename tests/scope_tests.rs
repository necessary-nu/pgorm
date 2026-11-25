//! Scoped entities: the statements a context builds, the rows they read and
//! write, and what is refused before anything is sent.
#![allow(unused_imports, dead_code)]

pub mod common;

pub use common::{TestContext, setup::*};
use pgorm::entity::prelude::*;
use pgorm::pgorm_query::OnConflict;
use pgorm::pipeline::{Pipeline, PipelineError};
use pgorm::tests_cfg::scoped::{CompanyAdmin, CompanyId, Member, UserId, company, message, note};
use pretty_assertions::assert_eq;

const MEMBER: Member = Member {
    company: CompanyId(1),
    user: UserId(10),
};

const ADMIN: CompanyAdmin = CompanyAdmin {
    company: CompanyId(1),
    users: Every::new(),
};

fn written(body: &str) -> note::ActiveModel {
    note::ActiveModel {
        body: set(body),
        ..Default::default()
    }
}

// [spec:pgorm:sem:scope.read/test]    every scope the context holds the value
// of is ANDed into the WHERE, an `Every` adds nothing, and a key and the
// scopes stand side by side
// [spec:pgorm:def:scope.every/test]    an administrator's `Every<UserId>` puts
// no condition on the read
// [spec:pgorm:def:scope.context/test]    a derived context holds its fields,
// and a scope value alone is a context
#[test]
fn reads_hold_to_the_contexts_scopes() {
    assert_eq!(
        note::Entity::find_for(&MEMBER).build().0,
        [
            r#"SELECT "note"."id", "note"."company_id", "note"."author_id", "note"."body""#,
            r#"FROM "note" WHERE "note"."company_id" = $1 AND "note"."author_id" = $2"#,
        ]
        .join(" ")
    );
    assert_eq!(
        note::Entity::find_for(&ADMIN).build().0,
        [
            r#"SELECT "note"."id", "note"."company_id", "note"."author_id", "note"."body""#,
            r#"FROM "note" WHERE "note"."company_id" = $1"#,
        ]
        .join(" ")
    );
    assert_eq!(
        note::Entity::find_by_id_for(&MEMBER, 5)
            .as_query()
            .to_string(),
        [
            r#"SELECT "note"."id", "note"."company_id", "note"."author_id", "note"."body""#,
            r#"FROM "note" WHERE "note"."company_id" = 1 AND "note"."author_id" = 10"#,
            r#"AND "note"."id" = 5"#,
        ]
        .join(" ")
    );
    assert_eq!(
        company::Entity::find_for(&CompanyId(1))
            .as_query()
            .to_string(),
        r#"SELECT "company"."id", "company"."name" FROM "company" WHERE "company"."id" = 1"#,
        "a scope value alone is a context, and a key can be the scope"
    );
}

// [spec:pgorm:def:scope.written/test]    a hand-written scope's condition is
// its own, qualified by the name the row is read under, and a pipeline, which
// cannot read it, refuses it
#[test]
fn a_written_scope_puts_its_own_condition() {
    assert_eq!(
        Pipeline::from_for(&UserId(10), message::Entity).into_sql(),
        Err(PipelineError::Scope(ScopeError::Unpiped {
            table: "message".to_owned(),
            scope: "UserId",
        }))
    );
    assert_eq!(
        message::Entity::find_for(&UserId(10))
            .as_query()
            .to_string(),
        [
            r#"SELECT "message"."id", "message"."sender_id", "message"."recipient_id", "message"."body""#,
            r#"FROM "message" WHERE "message"."sender_id" = 10 OR "message"."recipient_id" = 10"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:sem:scope.join/test]    a graph's slot, a typed join and a
// relation joined with its context put the joined table's scopes in its ON,
// under the alias it is read by
#[test]
fn joins_scope_the_joined_table_in_on() {
    let graph = company::Entity::graph_for(&MEMBER)
        .join_maybe::<note::Entity>(company::Relation::Note.def())
        .as_query()
        .to_string();
    assert!(
        graph.ends_with(
            r#"FROM "company" LEFT JOIN "note" ON "company"."id" = "note"."company_id" AND ("note"."company_id" = 1 AND "note"."author_id" = 10) WHERE "company"."id" = 1"#
        ),
        "{graph}"
    );
    assert_eq!(
        company::Entity::find_for(&MEMBER)
            .join_as_for(
                &MEMBER,
                JoinType::InnerJoin,
                company::Relation::Note.def(),
                alias("n")
            )
            .as_query()
            .to_string(),
        [
            r#"SELECT "company"."id", "company"."name" FROM "company""#,
            r#"INNER JOIN "note" AS "n" ON "company"."id" = "n"."company_id""#,
            r#"AND ("n"."company_id" = 1 AND "n"."author_id" = 10)"#,
            r#"WHERE "company"."id" = 1"#,
        ]
        .join(" ")
    );
    let related = company::Model {
        id: CompanyId(1),
        name: "Acme".to_owned(),
    }
    .find_related_for(&MEMBER, note::Entity)
    .as_query()
    .to_string();
    assert!(
        related.contains(
            r#"INNER JOIN "company" ON "company"."id" = "note"."company_id" AND "company"."id" = 1"#
        ),
        "{related}"
    );
}

// [spec:pgorm:sem:scope.insert/test]    an insert fills each scope column
// from the context, keeps a row already holding the context's value, and
// under `Every` takes the row's own
// [spec:pgorm:def:scope.every/test]    an insert under `Every` fills nothing
#[test]
fn an_insert_fills_its_scope_columns() {
    assert_eq!(
        Insert::one_for(&MEMBER, written("hello"))
            .as_query()
            .to_string(),
        r#"INSERT INTO "note" ("company_id", "author_id", "body") VALUES (1, 10, 'hello')"#
    );
    assert_eq!(
        Insert::one_for(
            &ADMIN,
            note::ActiveModel {
                author_id: set(UserId(11)),
                ..written("for 11")
            }
        )
        .as_query()
        .to_string(),
        r#"INSERT INTO "note" ("company_id", "author_id", "body") VALUES (1, 11, 'for 11')"#
    );
    assert_eq!(
        Insert::one_for(
            &UserId(10),
            message::ActiveModel {
                recipient_id: set(UserId(11)),
                body: set("hi"),
                ..Default::default()
            }
        )
        .as_query()
        .to_string(),
        r#"INSERT INTO "message" ("sender_id", "recipient_id", "body") VALUES (10, 11, 'hi')"#
    );
}

// [spec:pgorm:sem:scope.write/test]    an update or delete by key carries the
// scopes beside the key, and a bulk one is filtered by them
#[test]
fn a_write_by_key_carries_its_scopes() -> Result<(), Error> {
    assert_eq!(
        Update::one_for(
            &MEMBER,
            note::ActiveModel {
                id: set(5),
                body: set("edited"),
                ..Default::default()
            }
        )?
        .as_query()
        .to_string(),
        [
            r#"UPDATE "note" SET "body" = 'edited' WHERE "note"."id" = 5"#,
            r#"AND "note"."company_id" = 1 AND "note"."author_id" = 10"#,
        ]
        .join(" ")
    );
    assert_eq!(
        Delete::many_for(&ADMIN, note::Entity)
            .filter(note::Column::Body.eq(""))
            .as_query()
            .to_string(),
        r#"DELETE FROM "note" WHERE "note"."company_id" = 1 AND "note"."body" = ''"#
    );
    assert_eq!(
        note::Entity::delete_by_id_for(&MEMBER, 5)
            .as_query()
            .to_string(),
        [
            r#"DELETE FROM "note" WHERE "note"."company_id" = 1 AND "note"."author_id" = 10"#,
            r#"AND "note"."id" = 5"#,
        ]
        .join(" ")
    );
    Ok(())
}

// [spec:pgorm:req:scope.immutable/test]    a scope column set in an update is
// refused, by key or in bulk, and so is one an upsert's DO UPDATE SET names
#[test]
fn a_scope_column_is_never_changed() {
    let moved = Update::one_for(
        &ADMIN,
        note::ActiveModel {
            id: set(5),
            author_id: set(UserId(11)),
            ..Default::default()
        },
    );
    assert!(
        matches!(
            &moved,
            Err(Error::ImmutableColumn { table, column }) if table == "note" && column == "author_id"
        ),
        "{moved:?}"
    );
    assert!(note::Column::CompanyId.def().is_immutable());
    assert!(!note::Column::Body.def().is_immutable());
}

async fn schema(db: &DatabaseConnection) -> Result<(), Error> {
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
         INSERT INTO company VALUES (1, 'Acme'), (2, 'Globex');
         INSERT INTO note (company_id, author_id, body) VALUES
             (1, 10, 'mine'), (1, 11, 'a colleague''s'), (2, 20, 'another company''s');
         INSERT INTO message (sender_id, recipient_id, body) VALUES
             (10, 11, 'sent'), (11, 10, 'received'), (11, 20, 'not mine');",
    )
    .await
}

fn bodies(notes: Vec<note::Model>) -> Vec<String> {
    notes.into_iter().map(|note| note.body).collect()
}

// [spec:pgorm:sem:scope.read/test]    a context reads its rows and no other
// [spec:pgorm:sem:scope.write/test]    another scope's row is not updated or
// deleted, and reads as not found
// [spec:pgorm:sem:scope.insert/test]    an inserted row holds the context's
// values
#[pgorm_macros::test]
async fn a_context_reads_and_writes_its_own_rows() -> Result<(), Error> {
    let ctx = TestContext::new("scope_rows").await;
    let db = ctx.db.get().await?;
    schema(&db).await?;

    let mine = note::Entity::find_for(&MEMBER)
        .order_by_asc(note::Column::Id)
        .all(&db)
        .await?;
    assert_eq!(bodies(mine), ["mine"]);
    let company = note::Entity::find_for(&ADMIN)
        .order_by_asc(note::Column::Id)
        .all(&db)
        .await?;
    assert_eq!(bodies(company), ["mine", "a colleague's"]);
    let messages = message::Entity::find_for(&UserId(10))
        .order_by_asc(message::Column::Id)
        .all(&db)
        .await?;
    assert_eq!(
        messages.iter().map(|m| m.body.as_str()).collect::<Vec<_>>(),
        ["sent", "received"]
    );
    let piped: Vec<note::Model> = Pipeline::from_for(&MEMBER, note::Entity).all(&db).await?;
    assert_eq!(bodies(piped), ["mine"]);

    let inserted = Insert::one_for(&MEMBER, written("new"))
        .exec_returning_model(&db)
        .await?;
    assert_eq!(
        (inserted.company_id, inserted.author_id),
        (CompanyId(1), UserId(10))
    );

    let colleague = note::Entity::find_for(&ADMIN)
        .filter(note::Column::AuthorId.eq(UserId(11)))
        .one(&db)
        .await?;
    let edit = note::ActiveModel {
        id: set(colleague.id),
        body: set("edited"),
        ..Default::default()
    };
    assert!(matches!(
        Update::one_for(&MEMBER, edit.clone())?
            .exec_returning_model(&db)
            .await,
        Err(Error::RecordNotFound)
    ));
    assert_eq!(
        Update::one_for(&ADMIN, edit)?
            .exec_returning_model(&db)
            .await?
            .body,
        "edited"
    );
    assert_eq!(
        note::Entity::delete_by_id_for(&MEMBER, colleague.id)
            .exec(&db)
            .await?,
        0
    );
    let other = note::Entity::find_for(&CompanyAdmin {
        company: CompanyId(2),
        users: Every::new(),
    })
    .one(&db)
    .await?;
    assert!(matches!(
        note::Entity::find_by_id_for(&MEMBER, other.id)
            .one(&db)
            .await,
        Err(Error::RecordNotFound)
    ));
    assert_eq!(
        Update::many_for(&ADMIN, note::Entity)
            .col_expr(note::Column::Body, Expr::value("bulk"))
            .exec(&db)
            .await?,
        3,
        "company 1's three notes, and not company 2's"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:scope.refusals/test]    a join into a scoped table with no
// context, a row naming another scope's value, a row leaving a lifted scope's
// column unset, and a bulk update or upsert changing a scope column are each
// refused when the statement would run, and nothing is sent
// [spec:pgorm:req:scope.immutable/test]
#[pgorm_macros::test]
async fn a_statement_outside_its_scopes_is_refused() -> Result<(), Error> {
    let ctx = TestContext::new("scope_refusals").await;
    let db = ctx.db.get().await?;
    schema(&db).await?;

    let unscoped = company::Entity::find_for(&MEMBER)
        .join(JoinType::InnerJoin, company::Relation::Note.def())
        .all(&db)
        .await;
    assert!(
        matches!(&unscoped, Err(Error::Scope(ScopeError::Unscoped { table, .. })) if table == "note"),
        "{unscoped:?}"
    );

    let elsewhere = Insert::one_for(
        &MEMBER,
        note::ActiveModel {
            company_id: set(CompanyId(2)),
            ..written("elsewhere")
        },
    )
    .exec(&db)
    .await;
    assert!(
        matches!(&elsewhere, Err(Error::Scope(ScopeError::Conflict { column, .. })) if column == "company_id"),
        "{elsewhere:?}"
    );
    let unset = Insert::one_for(&ADMIN, written("whose?")).exec(&db).await;
    assert!(
        matches!(&unset, Err(Error::Scope(ScopeError::Unset { column, .. })) if column == "author_id"),
        "{unset:?}"
    );

    let moved = Update::many_for(&ADMIN, note::Entity)
        .col_expr(note::Column::CompanyId, Expr::value(2))
        .exec(&db)
        .await;
    assert!(
        matches!(&moved, Err(Error::ImmutableColumn { column, .. }) if column == "company_id"),
        "{moved:?}"
    );
    let upsert = Insert::one_for(
        &MEMBER,
        note::ActiveModel {
            id: set(1),
            ..written("again")
        },
    )
    .on_conflict(OnConflict::column(note::Column::Id).update_column(note::Column::AuthorId))
    .exec(&db)
    .await;
    assert!(
        matches!(&upsert, Err(Error::ImmutableColumn { column, .. }) if column == "author_id"),
        "{upsert:?}"
    );

    assert_eq!(
        note::Entity::find_for(&CompanyAdmin {
            company: CompanyId(2),
            users: Every::new(),
        })
        .all(&db)
        .await?
        .len(),
        1,
        "nothing refused reached the table"
    );
    assert_eq!(note::Entity::find_for(&ADMIN).all(&db).await?.len(), 2);

    drop(db);
    ctx.delete().await;
    Ok(())
}
