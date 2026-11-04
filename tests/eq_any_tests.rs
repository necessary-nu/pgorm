#![allow(unused_imports, dead_code)]

//! The array-parameter predicates against a live server.
//!
//! `eq_any` / `ne_all` claim two things a renderer cannot prove on its own: that
//! one array parameter carries the whole list, and that an empty array already
//! means what an empty `IN` has to be rewritten to mean. Both are semantics, so
//! both are asserted against PostgreSQL.
//!
//! Run the test locally:
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:54329 cargo test --test eq_any_tests

pub mod common;

pub use common::{TestContext, bakery_chain::*, setup::*};
pub use pgorm::entity::*;
pub use pgorm::{ConnectionTrait, Error, QueryFilter, QueryOrder, QuerySelect, set};
use pgorm::{QueryTrait, pgorm_query::Query};

async fn seed(db: &impl ConnectionTrait) -> Vec<i32> {
    let mut ids = Vec::new();
    for name in ["SeaSide Bakery", "Top Bakery", "Corner Bakery"] {
        let bakery = bakery::ActiveModel {
            name: set(name),
            profit_margin: set(10.4),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("could not insert bakery");
        ids.push(bakery.id);
    }
    ids
}

// [spec:pgorm:req:sql.ast.expr.eq-any+1/test]    against a live server:
// a three-element list and an empty one are the same statement with different
// parameter payloads, and both select what the predicate says they should
// [spec:pgorm:def:entity.traits.column+8/test]
#[pgorm_macros::test]
pub async fn eq_any_round_trips_one_array_parameter() {
    let ctx = TestContext::new("eq_any_round_trips_one_array_parameter").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();
    let ids = seed(&db).await;

    let three = Bakery::find()
        .filter(bakery::Column::Id.eq_any(ids.clone()))
        .order_by_asc(bakery::Column::Id)
        .all(&db)
        .await
        .unwrap();
    assert_eq!(
        three.iter().map(|b| b.id).collect::<Vec<_>>(),
        ids,
        "a three-element list selects its three rows"
    );

    // Membership in the empty set holds for nothing, which the empty array says
    // natively — no constant fall-back, and the same SQL as the populated case.
    let none = Bakery::find()
        .filter(bakery::Column::Id.eq_any(Vec::<i32>::new()))
        .all(&db)
        .await
        .unwrap();
    assert_eq!(none.len(), 0);

    assert_eq!(
        Bakery::find()
            .filter(bakery::Column::Id.eq_any(Vec::<i32>::new()))
            .as_query()
            .build()
            .0,
        Bakery::find()
            .filter(bakery::Column::Id.eq_any(ids))
            .as_query()
            .build()
            .0
    );

    drop(db);
    ctx.delete().await;
}

// [spec:pgorm:req:sql.ast.expr.eq-any+1/test]    against a live server:
// `<> ALL` is the complement of `= ANY`, and an empty list is vacuously true —
// the asymmetry `is_in` / `is_not_in` need two different constants to express
#[pgorm_macros::test]
pub async fn ne_all_complements_eq_any_on_the_server() {
    let ctx = TestContext::new("ne_all_complements_eq_any_on_the_server").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();
    let ids = seed(&db).await;

    let excluded = Bakery::find()
        .filter(bakery::Column::Id.ne_all([ids[0]]))
        .order_by_asc(bakery::Column::Id)
        .all(&db)
        .await
        .unwrap();
    assert_eq!(
        excluded.iter().map(|b| b.id).collect::<Vec<_>>(),
        ids[1..].to_vec()
    );

    let all = Bakery::find()
        .filter(bakery::Column::Id.ne_all(Vec::<i32>::new()))
        .all(&db)
        .await
        .unwrap();
    assert_eq!(all.len(), 3, "nothing fails a test applied to nothing");

    drop(db);
    ctx.delete().await;
}

// [spec:pgorm:def:sql.render.value-literals+5/test]    against a live server: an
// empty array literal is typeable only because it carries its element type —
// PostgreSQL rejects a bare `ARRAY []` with "cannot determine type of empty
// array", which no parser oracle can see because the grammar accepts it
#[pgorm_macros::test]
pub async fn empty_array_literal_types_on_the_server() {
    let ctx = TestContext::new("empty_array_literal_types_on_the_server").await;
    let db = ctx.db.get().await.unwrap();

    let inlined = Query::select()
        .expr(pgorm::pgorm_query::Expr::val(1i32).eq_any(Vec::<i32>::new()))
        .to_string();
    assert_eq!(inlined, "SELECT 1 = ANY(ARRAY []::int4[])");

    let row = db.query_one(inlined.as_str(), &[]).await.unwrap();
    let matched: bool = row.get(0);
    assert!(!matched);

    assert!(
        db.query_one("SELECT 1 = ANY(ARRAY [])", &[]).await.is_err(),
        "the untyped spelling is the one PostgreSQL cannot resolve"
    );

    drop(db);
    ctx.delete().await;
}
