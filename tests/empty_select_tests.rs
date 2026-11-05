#![allow(unused_imports, dead_code)]

//! The execution-boundary guard on an empty select list.
//!
//! Run the test locally:
//! cargo test --test empty_select_tests

pub mod common;

pub use common::{TestContext, bakery_chain::*, setup::*};
use pgorm::{
    Error, RuntimeError, SelectGetableTuple, SelectProjected, Selector, entity::prelude::*,
};
use pgorm_query::SelectStatement;
use pretty_assertions::assert_eq;
use std::num::NonZeroU64;

/// Any non-zero size reaches the guard; the paginator never gets far enough to
/// page anything.
const PAGE_SIZE: NonZeroU64 = NonZeroU64::new(10).expect("page size is non-zero");

#[pgorm_macros::test]
async fn main() {
    let ctx = TestContext::new("bakery_chain_empty_select_tests").await;
    create_tables(&ctx.db).await.unwrap();

    let db = ctx.db.get().await.unwrap();
    empty_select_list_is_refused(&db).await;
    raw_builder_select_list_is_refused(&db).await;
    populated_select_list_still_runs(&db).await;

    drop(db);
    ctx.delete().await;
}

/// An `Error::Query` — rather than an `Error::Postgres` carrying `42601` — is the
/// evidence the statement never left the process.
fn assert_empty_select_list(err: Error) {
    let Error::Query(RuntimeError::Internal(message)) = &err else {
        panic!("expected Error::Query(RuntimeError::Internal(..)), got {err:?}");
    };
    assert_eq!(
        message,
        "select list is empty; add at least one column or expression"
    );
    assert_eq!(
        err.to_string(),
        "Query Error: select list is empty; add at least one column or expression"
    );
}

/// `SelectCustom` has no execution path at all, so the guard is reached
/// through the one projection method that can add nothing: an empty iterator.
fn empty_projection() -> SelectProjected<Bakery> {
    Bakery::find()
        .select_only()
        .columns(std::iter::empty::<bakery::Column>())
}

/// `select` reaches the same state in one call: an empty list clears the
/// projection and puts nothing back.
fn empty_projection_via_select() -> SelectProjected<Bakery> {
    Bakery::find().select(Vec::<bakery::Column>::new())
}

// [spec:pgorm:sem:query.build.modifiers+10/test]    every execution path over a
// statement whose projection list is empty returns Error::Query before the
// statement is sent
pub async fn empty_select_list_is_refused(db: &DatabaseConnection) {
    assert_empty_select_list(
        empty_projection()
            .into_model::<bakery::Model>()
            .all(db)
            .await
            .expect_err("all over an empty select list"),
    );

    assert_empty_select_list(
        empty_projection()
            .into_model::<bakery::Model>()
            .one(db)
            .await
            .expect_err("one over an empty select list"),
    );

    assert_empty_select_list(
        empty_projection()
            .into_model::<bakery::Model>()
            .one_opt(db)
            .await
            .expect_err("one_opt over an empty select list"),
    );

    assert_empty_select_list(
        empty_projection()
            .into_model::<bakery::Model>()
            .stream(db)
            .await
            .err()
            .expect("stream over an empty select list"),
    );

    assert_empty_select_list(
        empty_projection()
            .into_tuple::<i32>()
            .all(db)
            .await
            .expect_err("into_tuple over an empty select list"),
    );

    assert_empty_select_list(
        empty_projection_via_select()
            .into_tuple::<i32>()
            .all(db)
            .await
            .expect_err("all over an empty `select` list"),
    );

    assert_empty_select_list(
        empty_projection()
            .into_tuple::<i32>()
            .paginate(db, PAGE_SIZE)
            .fetch()
            .await
            .expect_err("fetch over an empty select list"),
    );

    assert_empty_select_list(
        empty_projection()
            .into_tuple::<i32>()
            .count(db)
            .await
            .expect_err("count over an empty select list"),
    );

    assert_empty_select_list(
        empty_projection()
            .cursor_by(bakery::Column::Id)
            .into_model::<bakery::Model>()
            .first(10)
            .all(db)
            .await
            .expect_err("cursor over an empty select list"),
    );
}

// [spec:pgorm:sem:query.build.modifiers+10/test]    a hand-rolled statement,
// which the typestate never sees, is refused by the same guard
pub async fn raw_builder_select_list_is_refused(db: &DatabaseConnection) {
    assert_empty_select_list(
        Selector::<SelectGetableTuple<i32>>::into_tuple(SelectStatement::new())
            .all(db)
            .await
            .expect_err("raw statement with no projection"),
    );
}

// [spec:pgorm:sem:query.build.modifiers+10/test]    a statement whose projection
// list is non-empty is untouched by the guard
pub async fn populated_select_list_still_runs(db: &DatabaseConnection) {
    Insert::one(bakery::ActiveModel {
        name: set("SeaSide Bakery"),
        profit_margin: set(10.4),
        ..Default::default()
    })
    .exec(db)
    .await
    .expect("insert one bakery");

    let names: Vec<String> = Bakery::find()
        .select(bakery::Column::Name)
        .into_tuple()
        .all(db)
        .await
        .expect("all over a re-populated select list");
    assert_eq!(names, ["SeaSide Bakery".to_owned()]);

    let counted = Bakery::find()
        .select(bakery::Column::Id)
        .into_tuple::<i32>()
        .count(db)
        .await
        .expect("count over a re-populated select list");
    assert_eq!(counted, 1);

    let paged = Bakery::find()
        .select(bakery::Column::Id)
        .into_tuple::<i32>()
        .paginate(db, PAGE_SIZE)
        .fetch()
        .await
        .expect("fetch over a re-populated select list");
    assert_eq!(paged, [1]);

    let cursored = Bakery::find()
        .select([
            bakery::Column::Id,
            bakery::Column::Name,
            bakery::Column::ProfitMargin,
        ])
        .cursor_by(bakery::Column::Id)
        .into_model::<bakery::Model>()
        .first(10)
        .all(db)
        .await
        .expect("cursor over a re-populated select list");
    assert_eq!(cursored.len(), 1);
}
