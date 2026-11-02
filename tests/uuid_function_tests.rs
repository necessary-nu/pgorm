#![allow(unused_imports, dead_code)]

//! PostgreSQL 18's UUID functions against a live server.
//!
//! The render tests in pgorm-query settle that the calls parse. What only a
//! server settles is what they claim: that a key a `uuidv7()` default fills
//! sorts in the order the rows were written, and that the two readers return
//! the version and the minting instant — and nothing, rather than an error,
//! where a UUID carries no such thing.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{Expr, Func, Name, Query, SimpleExpr, Values};
use pgorm::{ConnectionTrait, Schema, entity::prelude::*};
use pretty_assertions::assert_eq;

/// A row keyed by the time it was minted, the key left to the server.
mod token {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "token")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false, default_expr = "Func::uuidv7()")]
        pub id: Uuid,
        pub written: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

async fn create_token_table(db: &DatabaseConnection) -> Result<(), Error> {
    let create = Schema::new()
        .create_table_from_entity(token::Entity)?
        .to_string();
    assert_eq!(
        create,
        r#"CREATE TABLE "token" ( "id" uuid NOT NULL DEFAULT UUIDV7(), "written" integer NOT NULL, PRIMARY KEY ("id") )"#
    );
    db.batch_execute(&create).await
}

async fn server_clock(db: &DatabaseConnection) -> Result<DateTimeWithTimeZone, Error> {
    ("SELECT clock_timestamp()", Values(vec![]))
        .into_tuple::<DateTimeWithTimeZone>()
        .one(db)
        .await
}

/// Rows written one after another, each leaving its key to the `uuidv7()`
/// default, come back in the order they were written when sorted by key alone.
// [spec:pgorm:def:sql.ast.func+11/test]    a v7 default sorts in minting order
// [spec:pgorm:def:entity.prelude+10/test]    an entity's `default_expr` names `Func`
// through the prelude
#[pgorm_macros::test]
async fn a_uuidv7_key_sorts_in_write_order() -> Result<(), Error> {
    let ctx = TestContext::new("uuidv7_key_order").await;
    let db = ctx.db.get().await?;
    create_token_table(&db).await?;

    let mut minted = Vec::new();
    for written in 0..64 {
        let row = token::ActiveModel {
            id: NotSet,
            written: set(written),
        }
        .insert(&db)
        .await?;
        assert_eq!(row.id.get_version_num(), 7, "{}", row.id);
        minted.push(row.id);
    }

    let by_key = token::Entity::find()
        .order_by_asc(token::Column::Id)
        .all(&db)
        .await?;
    assert_eq!(
        by_key.iter().map(|row| row.written).collect::<Vec<_>>(),
        (0..64).collect::<Vec<_>>()
    );
    assert_eq!(by_key.iter().map(|row| row.id).collect::<Vec<_>>(), minted);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// `uuid_extract_version` reads 7 off a key the default minted, and
/// `uuid_extract_timestamp` the millisecond it was minted in, which lies
/// between the server's clock read before the insert and after it.
// [spec:pgorm:def:sql.ast.func+11/test]    the readers return the version and the
// minting instant at millisecond precision
#[pgorm_macros::test]
async fn the_readers_return_the_version_and_the_instant() -> Result<(), Error> {
    let ctx = TestContext::new("uuid_readers").await;
    let db = ctx.db.get().await?;
    create_token_table(&db).await?;

    let before = server_clock(&db).await?;
    token::ActiveModel {
        id: NotSet,
        written: set(1),
    }
    .insert(&db)
    .await?;
    let after = server_clock(&db).await?;

    let read = Query::select()
        .expr(Func::uuid_extract_version(Expr::col(token::Column::Id)))
        .expr(Func::uuid_extract_timestamp(Expr::col(token::Column::Id)))
        .from(token::Entity)
        .build();
    let (version, minted) = read
        .into_tuple::<(i16, DateTimeWithTimeZone)>()
        .one(&db)
        .await?;
    assert_eq!(version, 7);
    assert_eq!(
        minted.subsec_nanosecond() % 1_000_000,
        0,
        "millisecond precision"
    );
    assert!(
        before.as_millisecond() <= minted.as_millisecond() && minted <= after,
        "{before} <= {minted} <= {after}"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A version 4 UUID — `uuidv4()`, like `gen_random_uuid()` — reads as version
/// 4 and carries no instant, so the timestamp reader answers `NULL` rather
/// than failing; a UUID outside the RFC 9562 variant has no version either.
// [spec:pgorm:def:sql.ast.func+11/test]    the readers answer NULL where there is
// nothing to read
#[pgorm_macros::test]
async fn the_readers_answer_null_where_nothing_is_encoded() -> Result<(), Error> {
    let ctx = TestContext::new("uuid_readers_null").await;
    let db = ctx.db.get().await?;

    let random = Query::select()
        .expr(Func::uuid_extract_version(Func::uuidv4()))
        .expr(Func::uuid_extract_timestamp(Func::uuidv4()))
        .expr(Func::uuid_extract_version(Func::gen_random_uuid()))
        .build();
    assert_eq!(
        random
            .into_tuple::<(Option<i16>, Option<DateTimeWithTimeZone>, Option<i16>)>()
            .one(&db)
            .await?,
        (Some(4), None, Some(4))
    );

    let nil = Query::select()
        .expr(Func::uuid_extract_version(Expr::val(Uuid::nil())))
        .expr(Func::uuid_extract_timestamp(Expr::val(Uuid::nil())))
        .build();
    assert_eq!(
        nil.into_tuple::<(Option<i16>, Option<DateTimeWithTimeZone>)>()
            .one(&db)
            .await?,
        (None, None)
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// `uuidv7(shift)` embeds the instant `shift` away from the clock, so a key
/// minted with a shift of minus one day reads a day back, and sorts before one
/// minted unshifted.
// [spec:pgorm:def:sql.ast.func+11/test]    the shifted overload embeds the
// shifted instant
#[pgorm_macros::test]
async fn a_shifted_uuidv7_carries_the_moved_instant() -> Result<(), Error> {
    let ctx = TestContext::new("uuidv7_shifted").await;
    let db = ctx.db.get().await?;

    let day_back = || Func::uuidv7_shifted(Expr::val("-1 day").cast_as(Name::runtime("interval")));
    let before = server_clock(&db).await?;
    let read = Query::select()
        .expr(Func::uuid_extract_timestamp(day_back()))
        .expr(Expr::expr(day_back()).lt(Func::uuidv7()))
        .build();
    let (minted, earlier) = read
        .into_tuple::<(DateTimeWithTimeZone, bool)>()
        .one(&db)
        .await?;
    let after = server_clock(&db).await?;

    let day = jiff::SignedDuration::from_hours(24);
    assert!(
        (before - day).as_millisecond() <= minted.as_millisecond() && minted <= after - day,
        "{before} - 1 day <= {minted} <= {after} - 1 day"
    );
    assert!(earlier);

    drop(db);
    ctx.delete().await;
    Ok(())
}
