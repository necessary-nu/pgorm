#![allow(unused_imports, dead_code)]

//! The JSON operator vocabulary against a live server.
//!
//! The renderer can prove the lexemes and the parameter count; it cannot prove
//! what PostgreSQL does with them. Three things are therefore asserted here:
//! that a path or key list travels as one `text[]` parameter and still selects
//! the right rows, that an empty key list carries its vacuous truth without a
//! fall-back, and that `?` is an operator on `jsonb` alone — a distinction
//! invisible to a syntax-only oracle, since every form parses.
//!
//! Run the test locally:
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:54329 cargo test --test json_operator_tests

use pgorm_query::ExprTrait;
pub mod common;

pub use common::{TestContext, setup::*};
use pgorm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, Error, QueryFilter, QueryOrder,
    QuerySelect, QueryTrait, Schema, pgorm_query::Expr, set,
};
use serde_json::json;

mod doc {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "json_doc")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(column_type = ColumnType::JsonBinary)]
        pub body: Json,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

async fn seed(db: &impl ConnectionTrait) -> Result<(), Error> {
    let create = Schema::new()
        .create_table_from_entity(doc::Entity)?
        .to_string();
    db.execute(create.as_str(), &[]).await?;

    for (id, body) in [
        (
            1,
            json!({ "kind": "cake", "meta": { "tier": "gold" }, "tags": ["new"] }),
        ),
        (2, json!({ "kind": "bread", "meta": { "tier": "silver" } })),
        (3, json!({ "other": 1 })),
    ] {
        doc::ActiveModel {
            id: set(id),
            body: set(body),
        }
        .insert(db)
        .await?;
    }

    Ok(())
}

async fn ids(db: &impl ConnectionTrait, filter: pgorm::pgorm_query::SimpleExpr) -> Vec<i32> {
    doc::Entity::find()
        .filter(filter)
        .order_by_asc(doc::Column::Id)
        .all(db)
        .await
        .expect("filtered select failed")
        .into_iter()
        .map(|m| m.id)
        .collect()
}

// [spec:pgorm:req:sql.ast.expr.json+3/test]    against a live server: a path and a
// key list each travel as one `text[]` parameter and select what the operator
// says they should, composed through an entity-level filter
#[pgorm_macros::test]
pub async fn json_path_and_key_operators_round_trip() {
    let ctx = TestContext::new("json_path_and_key_operators_round_trip").await;
    let db = ctx.db.get().await.unwrap();
    seed(&db).await.unwrap();

    assert_eq!(
        ids(&db, doc::Column::Body.into_expr().has_json_key("kind")).await,
        vec![1, 2]
    );
    assert_eq!(
        ids(
            &db,
            doc::Column::Body
                .into_expr()
                .has_any_json_keys(["tags", "other"])
        )
        .await,
        vec![1, 3]
    );
    assert_eq!(
        ids(
            &db,
            doc::Column::Body
                .into_expr()
                .has_all_json_keys(["kind", "meta"])
        )
        .await,
        vec![1, 2]
    );

    // The two-step path is one operator and one parameter, and `#>>` ends in
    // text, so it compares against a Rust string with no cast.
    assert_eq!(
        ids(
            &db,
            Expr::expr(
                doc::Column::Body
                    .into_expr()
                    .cast_json_path(["meta", "tier"])
            )
            .eq("gold")
        )
        .await,
        vec![1]
    );
    assert_eq!(
        ids(
            &db,
            Expr::expr(doc::Column::Body.into_expr().get_json_path(["meta"])).is_not_null()
        )
        .await,
        vec![1, 2]
    );

    // Containment and merge are the shared operators, not JSON-specific ones.
    assert_eq!(
        ids(
            &db,
            doc::Column::Body
                .into_expr()
                .contains(json!({ "kind": "cake" }))
        )
        .await,
        vec![1]
    );

    drop(db);
    ctx.delete().await;
}

// [spec:pgorm:req:sql.ast.expr.json+3/test]    against a live server: `?|` is false
// and `?&` true over an empty key list, so neither needs the constant fall-back
// `sql.ast.expr.in` has to synthesise, and the statement text does not change
#[pgorm_macros::test]
pub async fn empty_key_lists_carry_vacuous_truth() {
    let ctx = TestContext::new("empty_key_lists_carry_vacuous_truth").await;
    let db = ctx.db.get().await.unwrap();
    seed(&db).await.unwrap();

    assert_eq!(
        ids(
            &db,
            doc::Column::Body
                .into_expr()
                .has_any_json_keys(Vec::<String>::new())
        )
        .await,
        Vec::<i32>::new(),
        "no key of none can be present"
    );
    assert_eq!(
        ids(
            &db,
            doc::Column::Body
                .into_expr()
                .has_all_json_keys(Vec::<String>::new())
        )
        .await,
        vec![1, 2, 3],
        "nothing is missing when nothing was asked for"
    );

    // An empty path selects the document itself, so every row survives.
    assert_eq!(
        ids(
            &db,
            Expr::expr(
                doc::Column::Body
                    .into_expr()
                    .get_json_path(Vec::<String>::new())
            )
            .is_not_null()
        )
        .await,
        vec![1, 2, 3]
    );

    assert_eq!(
        doc::Entity::find()
            .filter(
                doc::Column::Body
                    .into_expr()
                    .has_all_json_keys(Vec::<String>::new())
            )
            .as_query()
            .build()
            .0,
        doc::Entity::find()
            .filter(doc::Column::Body.into_expr().has_all_json_keys(["a", "b"]))
            .as_query()
            .build()
            .0
    );

    drop(db);
    ctx.delete().await;
}

// [spec:pgorm:req:sql.ast.expr.json+3/test]    against a live server: the `?` family
// is defined on `jsonb` and nowhere else, so `->>`'s text result has no `?` —
// a rejection the syntax-only oracle cannot see, because both forms parse.
// The same statement also proves the rendered `?` survives the wire: parameters
// are `$N`, so nothing between here and the server reads it as a placeholder
// [spec:pgorm:def:sql.render.operators+8/test]
#[pgorm_macros::test]
pub async fn question_mark_needs_a_jsonb_operand() {
    let ctx = TestContext::new("question_mark_needs_a_jsonb_operand").await;
    let db = ctx.db.get().await.unwrap();
    seed(&db).await.unwrap();

    let query = doc::Entity::find()
        .filter(doc::Column::Body.into_expr().has_json_key("kind"))
        .filter(doc::Column::Id.gt(1));

    let (sql, values) = query.as_query().build();
    assert!(
        sql.contains(" ? $1") && sql.contains("$2"),
        "an operator `?` and numbered placeholders share one statement: {sql}"
    );
    assert_eq!(values.0.len(), 2);

    let found = query
        .all(&db)
        .await
        .expect("the `?` operator reached the server");
    assert_eq!(found.iter().map(|m| m.id).collect::<Vec<_>>(), vec![2]);

    // The cast form returns text, which has no `?` operator at all.
    let rejected = db
        .query_one(r#"SELECT '{"a":1}'::jsonb ->> 'a' ? 'a'"#, &[])
        .await;
    assert!(rejected.is_err(), "`?` is not defined on text");

    let accepted: bool = db
        .query_one(r#"SELECT '{"a":1}'::jsonb -> 'a' IS NOT NULL"#, &[])
        .await
        .unwrap()
        .get(0);
    assert!(accepted);

    drop(db);
    ctx.delete().await;
}
