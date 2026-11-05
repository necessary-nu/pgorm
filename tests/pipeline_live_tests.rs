#![allow(unused_imports, dead_code)]

//! The pipeline API against a live server, with bound parameters.
//!
//! The unit suite holds every emitted string to a golden form and the
//! pg_query oracle; what it cannot prove is that the clause placement means
//! what it says — that a `HAVING` filters groups, that a filter-after-window
//! really applies outside the CTE, that `$N` values arrive in the right
//! slots. Those are semantics, so they are asserted against PostgreSQL with
//! decoded rows.
//!
//! Run the test locally:
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:54329 cargo test --test pipeline_live_tests

pub mod common;

use common::bakery_chain::{customer::Column as C, order::Column as O};
pub use common::{TestContext, bakery_chain::*, setup::*};
pub use jiff::{Timestamp, tz::Offset};
use pgorm::pipeline::{
    AliasName, Expr, ExprOps, IntoSource, JoinSide, Pipeline, alias, by, col, count, count_rows,
    first, last, named_runtime, row_number, sort_by, sum,
};
use pgorm::{ConnectionTrait, Schema, entity::prelude::Uuid, entity::*, set};
use pretty_assertions::assert_eq;
use rust_decimal::Decimal;

/// Names this suite's pipelines introduce: bound once here, referred to by
/// value everywhere below.
const SPENT: AliasName = alias("spent");
const ORDER_COUNT: AliasName = alias("order_count");
const RN: AliasName = alias("rn");
const CUSTOMER_ID: AliasName = alias("customer_id");
const RUNNING: AliasName = alias("running");
const NEXT: AliasName = alias("next_total");
const NEIGHBOURS: AliasName = alias("neighbours");
const MANAGER: AliasName = alias("manager");
const PARENT: AliasName = alias("parent");
const ID: AliasName = alias("id");
const NAME: AliasName = alias("name");
const BODY: AliasName = alias("body");
const MANAGED: AliasName = alias("managed");
const EVERYONE: AliasName = alias("everyone");
const OTHER_LINE: AliasName = alias("other_line");
const ORDER_ID: AliasName = alias("order_id");

/// A table that refers to itself: every employee but the founder reports to
/// another row of this same table.
mod employee {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "employee")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub name: String,
        pub manager_id: Option<i32>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(belongs_to = Entity, from = Column::ManagerId, to = Column::Id)]
        Manager,
    }
}

/// The same shape with the reference left unset on the root row, so a left
/// join has a `NULL` to carry.
mod message {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "message")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub body: String,
        pub parent_id: Option<i32>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {
        #[pgorm(belongs_to = Entity, from = Column::ParentId, to = Column::Id)]
        Parent,
    }
}

struct Seeded {
    alice: i32,
    bob: i32,
    cleo: i32,
}

/// Three customers with 3, 2 and 1 orders: Alice spends 60, Bob 50, Cleo 5.
async fn seed(db: &impl ConnectionTrait) -> Seeded {
    let bakery = bakery::ActiveModel {
        name: set("SeaSide Bakery"),
        profit_margin: set(10.4),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("could not insert bakery");

    let mut ids = Vec::new();
    for (name, totals) in [
        ("Alice", vec![10.00, 20.00, 30.00]),
        ("Bob", vec![25.50, 24.50]),
        ("Cleo", vec![5.00]),
    ] {
        let customer = customer::ActiveModel {
            name: set(name),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("could not insert customer");
        for total in totals {
            order::ActiveModel {
                bakery_id: set(bakery.id),
                customer_id: set(customer.id),
                total: set(rust_dec(total)),
                placed_at: set(Offset::UTC.to_datetime(Timestamp::now())),
                ..Default::default()
            }
            .insert(db)
            .await
            .expect("could not insert order");
        }
        ids.push(customer.id);
    }
    Seeded {
        alice: ids[0],
        bob: ids[1],
        cleo: ids[2],
    }
}

fn parsed_select(sql: &str) -> pg_query::protobuf::SelectStmt {
    let parsed = pg_query::parse(sql, pg_query::ParserOptions::DEFAULT).expect("grammar accepts");
    let node = parsed.protobuf.stmts[0]
        .stmt
        .as_ref()
        .and_then(|stmt| stmt.node.as_ref())
        .expect("statement present");
    match node {
        pg_query::NodeEnum::SelectStmt(select) => (**select).clone(),
        other => panic!("expected SelectStmt, got {other:?}"),
    }
}

fn spending() -> Pipeline {
    Pipeline::from(order::Entity)
        .group(O::CustomerId)
        .aggregate((sum(O::Total).as_(SPENT), count_rows().as_(ORDER_COUNT)))
}

// [spec:pgorm:req:pipeline.surface+4/test]    HAVING placement with a bound
// threshold: the group filter runs against grouped sums, not rows
#[pgorm_macros::test]
async fn having_filters_groups_with_bound_param() {
    let ctx = TestContext::new("pipeline_having_filters_groups").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();
    let seeded = seed(&db).await;

    let pipeline = spending()
        .filter_with(|binder| SPENT.gt(binder.bind(rust_dec(40.0))))
        .sort(SPENT.desc());
    let (sql, _) = pipeline.clone().into_sql().unwrap();
    assert!(parsed_select(&sql).having_clause.is_some());

    let rows: Vec<(i32, Decimal, i64)> = pipeline.into_tuple().unwrap().all(&db).await.unwrap();
    assert_eq!(
        rows,
        vec![
            (seeded.alice, rust_dec(60.00), 3),
            (seeded.bob, rust_dec(50.00), 2),
        ]
    );

    ctx.delete().await;
}

// [spec:pgorm:req:pipeline.surface+4/test]    filter-after-window nests the
// windowed stage in a CTE and filters outside it, with the rank bound
#[pgorm_macros::test]
async fn window_rank_filter_nests_through_cte() {
    let ctx = TestContext::new("pipeline_window_rank_cte").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();
    let seeded = seed(&db).await;

    let pipeline = Pipeline::from(order::Entity)
        .window(
            row_number().as_(RN),
            by(O::CustomerId).sort_by(O::Total.desc()),
        )
        .filter_with(|binder| RN.lte(binder.bind(2_i64)))
        .select((O::CustomerId, O::Total, RN))
        .sort((O::CustomerId, RN));
    let (sql, _) = pipeline.clone().into_sql().unwrap();
    let parsed = parsed_select(&sql);
    assert!(parsed.with_clause.is_some());
    assert!(parsed.where_clause.is_some());

    let rows: Vec<(i32, Decimal, i64)> = pipeline.into_tuple().unwrap().all(&db).await.unwrap();
    assert_eq!(
        rows,
        vec![
            (seeded.alice, rust_dec(30.00), 1),
            (seeded.alice, rust_dec(20.00), 2),
            (seeded.bob, rust_dec(25.50), 1),
            (seeded.bob, rust_dec(24.50), 2),
            (seeded.cleo, rust_dec(5.00), 1),
        ]
    );

    ctx.delete().await;
}

// [spec:pgorm:req:pipeline.surface+4/test]    explicit join condition against
// live rows, with the customer name bound
#[pgorm_macros::test]
async fn join_on_explicit_condition_binds_params() {
    let ctx = TestContext::new("pipeline_join_explicit_condition").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();
    seed(&db).await;

    let rows: Vec<(Decimal, String)> = Pipeline::from(order::Entity)
        .join(JoinSide::Inner, customer::Entity, O::CustomerId.eq(C::Id))
        .filter_with(|binder| C::Name.eq(binder.bind("Alice")))
        .select((O::Total, C::Name))
        .sort(O::Total)
        .into_tuple()
        .unwrap()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(
        rows,
        vec![
            (rust_dec(10.00), "Alice".to_owned()),
            (rust_dec(20.00), "Alice".to_owned()),
            (rust_dec(30.00), "Alice".to_owned()),
        ]
    );

    ctx.delete().await;
}

// [spec:pgorm:req:pipeline.surface+4/test]    an explicit ROWS frame computes
// a running sum per partition on live rows
#[pgorm_macros::test]
async fn rows_frame_computes_running_sum() {
    let ctx = TestContext::new("pipeline_rows_frame_running_sum").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();
    let seeded = seed(&db).await;

    let rows: Vec<(i32, Decimal, Decimal)> = Pipeline::from(order::Entity)
        .window(
            sum(O::Total).as_(RUNNING),
            by(O::CustomerId).sort_by(O::Total).rows(None, Some(0)),
        )
        .filter_with(|binder| O::CustomerId.eq(binder.bind(seeded.alice)))
        .select((O::CustomerId, O::Total, RUNNING))
        .sort(O::Total)
        .into_tuple()
        .unwrap()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(
        rows,
        vec![
            (seeded.alice, rust_dec(10.00), rust_dec(10.00)),
            (seeded.alice, rust_dec(20.00), rust_dec(30.00)),
            (seeded.alice, rust_dec(30.00), rust_dec(60.00)),
        ]
    );

    ctx.delete().await;
}

// [spec:pgorm:sem:pipeline.window-frame/test]    a frame that selects only the
// following row reaches LAST_VALUE, so each row reads its neighbour and the
// partition's last row reads nothing
#[pgorm_macros::test]
async fn a_following_frame_reads_only_the_next_row() {
    let ctx = TestContext::new("pipeline_following_frame_next_row").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();
    let seeded = seed(&db).await;

    let rows: Vec<(Decimal, Option<Decimal>)> = Pipeline::from(order::Entity)
        .window(
            last(O::Total).as_(NEXT),
            by(O::CustomerId).sort_by(O::Total).rows(Some(1), Some(1)),
        )
        .filter_with(|binder| O::CustomerId.eq(binder.bind(seeded.alice)))
        .select((O::Total, NEXT))
        .sort(O::Total)
        .into_tuple()
        .unwrap()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(
        rows,
        vec![
            (rust_dec(10.00), Some(rust_dec(20.00))),
            (rust_dec(20.00), Some(rust_dec(30.00))),
            (rust_dec(30.00), None),
        ]
    );

    ctx.delete().await;
}

// [spec:pgorm:sem:pipeline.window-frame/test]    a frame spanning the current
// row reaches FIRST_VALUE, and the aggregate rendered beside it counts exactly
// the rows that frame holds
#[pgorm_macros::test]
async fn a_spanning_frame_agrees_across_both_paths() {
    let ctx = TestContext::new("pipeline_spanning_frame_both_paths").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();
    let seeded = seed(&db).await;

    let rows: Vec<(Decimal, Decimal, i64)> = Pipeline::from(order::Entity)
        .window(
            (first(O::Total).as_(RUNNING), count_rows().as_(NEIGHBOURS)),
            by(O::CustomerId).sort_by(O::Total).rows(Some(-1), Some(1)),
        )
        .filter_with(|binder| O::CustomerId.eq(binder.bind(seeded.alice)))
        .select((O::Total, RUNNING, NEIGHBOURS))
        .sort(O::Total)
        .into_tuple()
        .unwrap()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(
        rows,
        vec![
            (rust_dec(10.00), rust_dec(10.00), 2),
            (rust_dec(20.00), rust_dec(10.00), 3),
            (rust_dec(30.00), rust_dec(20.00), 2),
        ]
    );

    ctx.delete().await;
}

// [spec:pgorm:req:pipeline.surface+4/test]    fn(Pipeline) -> Pipeline scopes
// compose, each binding its own parameters, and the placeholders stay aligned
#[pgorm_macros::test]
async fn composed_scopes_bind_params_in_order() {
    let ctx = TestContext::new("pipeline_composed_scopes").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();
    let seeded = seed(&db).await;

    fn spent_over(pipeline: Pipeline, threshold: Decimal) -> Pipeline {
        pipeline.filter_with(move |binder| SPENT.gt(binder.bind(threshold)))
    }
    fn fewer_orders_than(pipeline: Pipeline, count: i64) -> Pipeline {
        pipeline.filter_with(move |binder| ORDER_COUNT.lt(binder.bind(count)))
    }

    let pipeline = fewer_orders_than(spent_over(spending(), rust_dec(40.0)), 3);
    let (sql, values) = pipeline.clone().into_sql().unwrap();
    assert!(sql.contains("$1") && sql.contains("$2"), "{sql}");
    assert_eq!(values.0.len(), 2);

    let rows: Vec<(i32, Decimal, i64)> = pipeline.into_tuple().unwrap().all(&db).await.unwrap();
    assert_eq!(rows, vec![(seeded.bob, rust_dec(50.00), 2)]);

    ctx.delete().await;
}

// [spec:pgorm:sem:pipeline.terminal/test]    the model terminal goes straight
// from a pipeline to entity models on a connection
#[pgorm_macros::test]
async fn terminal_decodes_entity_models() {
    let ctx = TestContext::new("pipeline_terminal_models").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();
    seed(&db).await;

    let customers: Vec<customer::Model> = Pipeline::from(customer::Entity)
        .filter_with(|binder| C::Name.ne(binder.bind("Cleo")))
        .sort(C::Name)
        .all(&db)
        .await
        .unwrap();
    assert_eq!(
        customers
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Alice", "Bob"]
    );

    let bob: customer::Model = Pipeline::from(customer::Entity)
        .filter_with(|binder| C::Name.eq(binder.bind("Bob")))
        .one(&db)
        .await
        .unwrap();
    assert_eq!(bob.name, "Bob");

    let nobody: Option<customer::Model> = Pipeline::from(customer::Entity)
        .filter_with(|binder| C::Name.eq(binder.bind("Zed")))
        .one_opt(&db)
        .await
        .unwrap();
    assert!(nobody.is_none());

    ctx.delete().await;
}

// [spec:pgorm:req:pipeline.compose/test]    a union of two filtered
// pipelines, one bound param each: the values interleave with their $N
#[pgorm_macros::test]
async fn union_of_filtered_pipelines_binds_both_params() {
    let ctx = TestContext::new("pipeline_union_two_filtered").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();
    seed(&db).await;

    let small = Pipeline::from(order::Entity)
        .filter_with(|binder| O::Total.lt(binder.bind(rust_dec(6.0))))
        .select(O::Total);
    let pipeline = Pipeline::from(order::Entity)
        .filter_with(|binder| O::Total.gt(binder.bind(rust_dec(25.0))))
        .select(O::Total)
        .append(small)
        .sort(O::Total);
    let (sql, values) = pipeline.clone().into_sql().unwrap();
    assert!(sql.contains("UNION ALL"), "{sql}");
    assert_eq!(values.0.len(), 2);

    let rows: Vec<(Decimal,)> = pipeline.into_tuple().unwrap().all(&db).await.unwrap();
    assert_eq!(
        rows,
        vec![(rust_dec(5.00),), (rust_dec(25.50),), (rust_dec(30.00),),]
    );

    ctx.delete().await;
}

// [spec:pgorm:req:pipeline.compose/test]    top spenders joined back to
// their customers: params bound in the consumer and in the embedded pipeline
#[pgorm_macros::test]
async fn top_spenders_join_binds_across_pipelines() {
    let ctx = TestContext::new("pipeline_top_spenders_join").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();
    seed(&db).await;

    let spenders = spending().filter_with(|binder| SPENT.gt(binder.bind(rust_dec(40.0))));
    let pipeline = Pipeline::from(customer::Entity)
        .filter_with(|binder| C::Name.ne(binder.bind("Zed")))
        .join(JoinSide::Inner, spenders, C::Id.eq(CUSTOMER_ID))
        .select((C::Name, SPENT))
        .sort(SPENT.desc());
    let (sql, values) = pipeline.clone().into_sql().unwrap();
    assert!(parsed_select(&sql).with_clause.is_some());
    assert_eq!(values.0.len(), 2);

    let rows: Vec<(String, Decimal)> = pipeline.into_tuple().unwrap().all(&db).await.unwrap();
    assert_eq!(
        rows,
        vec![
            ("Alice".to_owned(), rust_dec(60.00)),
            ("Bob".to_owned(), rust_dec(50.00)),
        ]
    );

    ctx.delete().await;
}

// [spec:pgorm:req:pipeline.compose/test]    remove drops one matching row
// per row of the removed pipeline, with a param on each side
#[pgorm_macros::test]
async fn remove_pipeline_subtracts_matching_rows() {
    let ctx = TestContext::new("pipeline_remove_matching").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();
    let seeded = seed(&db).await;

    let big = Pipeline::from(order::Entity)
        .filter_with(|binder| O::Total.gt(binder.bind(rust_dec(24.0))))
        .select(O::CustomerId);
    let pipeline = Pipeline::from(order::Entity)
        .filter_with(|binder| O::Total.gt(binder.bind(rust_dec(9.0))))
        .select(O::CustomerId)
        .remove(big)
        .sort(CUSTOMER_ID);
    let (sql, values) = pipeline.clone().into_sql().unwrap();
    assert!(sql.contains("EXCEPT ALL"), "{sql}");
    assert_eq!(values.0.len(), 2);

    let rows: Vec<(i32,)> = pipeline.into_tuple().unwrap().all(&db).await.unwrap();
    assert_eq!(rows, vec![(seeded.alice,), (seeded.alice,)]);

    ctx.delete().await;
}

/// A chain of set operations is evaluated the way it was written.
///
/// SQL binds `INTERSECT` tighter than `UNION`, so a flat
/// `A UNION ALL B INTERSECT ALL C` is the server's
/// `A UNION ALL (B INTERSECT ALL C)`. Here that is every customer twice
/// instead of once — the doubling five campaign items reported, all of them
/// shrinking to this shape. The row count is the whole point, so it is
/// asserted against the server rather than against a rendering.
// [spec:pgorm:req:pipeline.compose/test]
#[pgorm_macros::test]
async fn an_appended_relation_intersects_as_one_whole() {
    let ctx = TestContext::new("pipeline_set_op_association").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();
    let seeded = seed(&db).await;

    // Two copies of every id on the left, one on the right: `INTERSECT ALL`
    // pairs them off and keeps one copy of each.
    let ids = || Pipeline::from(customer::Entity).select(C::Id);
    let pipeline = ids().append(ids()).intersect(ids()).sort(ID);

    let rows: Vec<(i32,)> = pipeline.into_tuple().unwrap().all(&db).await.unwrap();
    let mut expected = vec![(seeded.alice,), (seeded.bob,), (seeded.cleo,)];
    expected.sort_unstable();
    assert_eq!(rows, expected);

    ctx.delete().await;
}

/// A nested source may only read what the relation under it exposes.
///
/// prqlc built a `RelationInstance` for a declared table with an empty
/// `cid_redirects` map — at that point the table is only a `TId` whose output
/// columns do not exist yet, and nothing filled the map in once they did. An
/// ordering inherited from a CTE is phrased in that CTE's interior cids, so
/// translating it through an empty map was the identity and the interior cids
/// leaked outward. Appended to an enclosing SELECT they re-materialised the
/// expression that defined them, naming source columns a rename had replaced —
/// PostgreSQL rejects that with SQLSTATE 42703 (`pipeline-hidden-order`).
///
/// It is a rejection rather than a wrong answer, so the assertion is that the
/// SQL executes at all. The shape is the filed reproducer's: the source nested
/// twice, every column renamed, a derived constant, an ordering over all of
/// them, and a nested projection that keeps only some.
// [spec:pgorm:req:pipeline.compose/test]
#[pgorm_macros::test]
async fn a_nested_source_reads_only_exposed_columns() {
    let ctx = TestContext::new("pipeline_nested_source_scope").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();
    seed(&db).await;

    // The filed reproducer's shape exactly: the source nested twice, every
    // column renamed, a derived constant, an ordering over all of them, then a
    // nested projection that keeps only some — so the dropped sort keys have to
    // be carried through a CTE boundary that no longer names them.
    let origin = pgorm::pgorm_query::Name::runtime("origin");
    let renamed = Pipeline::from(named_runtime(
        named_runtime(Pipeline::from(customer::Entity), origin.clone()),
        origin.clone(),
    ))
    .select((
        col(origin.clone(), ID).as_runtime(pgorm::pgorm_query::Name::runtime("p_id")),
        col(origin.clone(), NAME).as_runtime(pgorm::pgorm_query::Name::runtime("p_name")),
    ))
    .derive(Expr::from(7i64).as_runtime(pgorm::pgorm_query::Name::runtime("nonce")))
    .sort((alias("p_id"), alias("p_name"), alias("nonce")));

    let inner = pgorm::pgorm_query::Name::runtime("nested_inner");
    let narrowed = Pipeline::from(named_runtime(renamed, inner.clone()))
        // p_id and p_name are ordered by, and dropped here.
        .select(
            col(inner.clone(), alias("nonce"))
                .as_runtime(pgorm::pgorm_query::Name::runtime("nonce")),
        );
    let outer = Pipeline::from(named_runtime(
        narrowed,
        pgorm::pgorm_query::Name::runtime("nested_outer"),
    ));

    let rows: Vec<(i32,)> = outer.into_tuple().unwrap().all(&db).await.unwrap();
    assert!(!rows.is_empty(), "the nested source returned nothing");

    ctx.delete().await;
}

/// The other face of the same leak: an ordering that reaches an outer ORDER BY
/// qualified by a relation the final FROM does not carry, which PostgreSQL
/// rejects with SQLSTATE 42P01 (`nested-source-from-entry`).
// [spec:pgorm:req:pipeline.compose/test]
#[pgorm_macros::test]
async fn a_nested_ordering_names_a_relation_in_scope() {
    let ctx = TestContext::new("pipeline_nested_ordering_scope").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();
    seed(&db).await;

    // The same leak reaching an outer ORDER BY instead of an enclosing SELECT:
    // a nested, ordered relation joined to another, where the ordering ends up
    // qualified by a relation the final FROM does not carry.
    let ranked = Pipeline::from(named_runtime(
        named_runtime(
            Pipeline::from(order::Entity),
            pgorm::pgorm_query::Name::runtime("o"),
        ),
        pgorm::pgorm_query::Name::runtime("o"),
    ))
    .select((
        col(pgorm::pgorm_query::Name::runtime("o"), CUSTOMER_ID)
            .as_runtime(pgorm::pgorm_query::Name::runtime("p_customer")),
        col(pgorm::pgorm_query::Name::runtime("o"), alias("total"))
            .as_runtime(pgorm::pgorm_query::Name::runtime("p_total")),
    ))
    .take_range(1i64..=6i64)
    .derive(
        Expr::from(alias("p_customer")).as_runtime(pgorm::pgorm_query::Name::runtime("carried")),
    )
    .sort((alias("p_customer"), alias("p_total"), alias("carried")));
    let held = pgorm::pgorm_query::Name::runtime("held");
    let joined = Pipeline::from(named_runtime(ranked, held.clone())).join(
        JoinSide::Inner,
        named_runtime(
            Pipeline::from(customer::Entity).select((
                C::Id.as_runtime(pgorm::pgorm_query::Name::runtime("c_id")),
                C::Name.as_runtime(pgorm::pgorm_query::Name::runtime("c_name")),
            )),
            pgorm::pgorm_query::Name::runtime("who"),
        ),
        col(held.clone(), alias("carried"))
            .eq(col(pgorm::pgorm_query::Name::runtime("who"), alias("c_id"))),
    );
    let rows: Vec<(String,)> = joined
        .select(col(
            pgorm::pgorm_query::Name::runtime("who"),
            alias("c_name"),
        ))
        .into_tuple()
        .unwrap()
        .all(&db)
        .await
        .unwrap();
    assert!(!rows.is_empty(), "the nested join returned nothing");

    ctx.delete().await;
}

async fn create_entity_table<E: EntityTrait>(db: &impl ConnectionTrait, entity: E) {
    let stmt = Schema::new()
        .create_table_from_entity(entity)
        .expect("the entity's table is generated");
    create_table_without_asserts(db, &stmt)
        .await
        .expect("could not create table");
}

/// Ada founded the company; Grace and Linus report to her, Alan to Grace.
async fn seed_employees(db: &impl ConnectionTrait) {
    let ada = employee::ActiveModel {
        name: set("Ada"),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("could not insert employee");
    let grace = employee::ActiveModel {
        name: set("Grace"),
        manager_id: set(ada.id),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("could not insert employee");
    for (name, manager) in [("Linus", ada.id), ("Alan", grace.id)] {
        employee::ActiveModel {
            name: set(name),
            manager_id: set(manager),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("could not insert employee");
    }
}

/// `COUNT(expr)` and `COUNT(*)` are different questions, and PostgreSQL is
/// asked both here rather than trusted to agree with a golden: of the four
/// employees exactly three report to someone, so counting the reference skips
/// the founder's null and counting rows does not. The two spellings are the
/// two answers, in an `aggregate` and under an explicit frame alike.
// [spec:pgorm:sem:pipeline.count-argument/test]
#[pgorm_macros::test]
async fn counting_a_nullable_column_skips_its_nulls() {
    let ctx = TestContext::new("pipeline_count_nullable").await;
    let db = ctx.db.get().await.unwrap();
    create_entity_table(&db, employee::Entity).await;
    seed_employees(&db).await;

    // The independent reading: the server's own answer to both questions,
    // asked in SQL this suite wrote rather than SQL the pipeline emitted.
    let control = db
        .query_one("SELECT COUNT(manager_id), COUNT(*) FROM employee", &[])
        .await
        .unwrap();
    let (managed, everyone): (i64, i64) = (control.get(0), control.get(1));
    assert_eq!((managed, everyone), (3, 4));

    // Grouped by the same nullable reference, the founder's group is one row
    // that counts as none: the two spellings separate there and nowhere else.
    let per_manager: Vec<(i64, i64)> = db
        .query_all(
            "SELECT COUNT(manager_id) AS m, COUNT(*) AS e FROM employee \
             GROUP BY manager_id ORDER BY m, e",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(per_manager, vec![(0, 1), (1, 1), (2, 2)]);

    let grouped: Vec<(i64, i64)> = Pipeline::from(employee::Entity)
        .group(employee::Column::ManagerId)
        .aggregate((
            count(employee::Column::ManagerId).as_(MANAGED),
            count_rows().as_(EVERYONE),
        ))
        .select((MANAGED, EVERYONE))
        .sort((MANAGED, EVERYONE))
        .into_tuple()
        .unwrap()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(grouped, per_manager);

    // The same distinction under a window wide enough to span the relation:
    // the frame is the whole table, so every row carries both totals.
    let windowed: Vec<(i64, i64)> = Pipeline::from(employee::Entity)
        .window(
            (
                count(employee::Column::ManagerId).as_(MANAGED),
                count_rows().as_(EVERYONE),
            ),
            sort_by(employee::Column::Id).rows(None, None),
        )
        .select((MANAGED, EVERYONE))
        .into_tuple()
        .unwrap()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(windowed, vec![(managed, everyone); 4]);

    ctx.delete().await;
}

// [spec:pgorm:sem:pipeline.self-join+1/test]    the classic employee-manager
// query on live rows: one table, two names, both sides decoded
#[pgorm_macros::test]
async fn self_join_decodes_employee_and_manager() {
    let ctx = TestContext::new("pipeline_self_join_manager").await;
    let db = ctx.db.get().await.unwrap();
    create_entity_table(&db, employee::Entity).await;
    seed_employees(&db).await;

    let pipeline = Pipeline::from(employee::Entity)
        .join(
            JoinSide::Inner,
            employee::Entity.named(MANAGER),
            employee::Column::ManagerId.eq(col(MANAGER, ID)),
        )
        .select((
            employee::Column::Name,
            col(MANAGER, NAME).as_(alias("boss")),
        ))
        .sort(employee::Column::Name);
    let (sql, _) = pipeline.clone().into_sql().unwrap();
    assert!(sql.contains("employee AS manager"), "{sql}");
    assert!(parsed_select(&sql).with_clause.is_none(), "{sql}");

    let rows: Vec<(String, String)> = pipeline.into_tuple().unwrap().all(&db).await.unwrap();
    assert_eq!(
        rows,
        vec![
            ("Alan".to_owned(), "Grace".to_owned()),
            ("Grace".to_owned(), "Ada".to_owned()),
            ("Linus".to_owned(), "Ada".to_owned()),
        ]
    );

    ctx.delete().await;
}

// [spec:pgorm:sem:pipeline.self-join+1/test]    a left self-join over a nullable
// reference keeps the root row, its parent decoding as None
#[pgorm_macros::test]
async fn left_self_join_decodes_a_null_parent() {
    let ctx = TestContext::new("pipeline_self_join_parent").await;
    let db = ctx.db.get().await.unwrap();
    create_entity_table(&db, message::Entity).await;

    let root = message::ActiveModel {
        body: set("root"),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    let reply = message::ActiveModel {
        body: set("reply"),
        parent_id: set(root.id),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    message::ActiveModel {
        body: set("nested"),
        parent_id: set(reply.id),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();

    let pipeline = Pipeline::from(message::Entity)
        .join(
            JoinSide::Left,
            message::Entity.named(PARENT),
            message::Column::ParentId.eq(col(PARENT, ID)),
        )
        .sort(message::Column::Id)
        .select((
            message::Column::Body,
            col(PARENT, BODY).as_(alias("parent_body")),
        ));
    let (sql, _) = pipeline.clone().into_sql().unwrap();
    assert!(sql.contains("LEFT OUTER JOIN message AS parent"), "{sql}");

    let rows: Vec<(String, Option<String>)> =
        pipeline.into_tuple().unwrap().all(&db).await.unwrap();
    assert_eq!(
        rows,
        vec![
            ("root".to_owned(), None),
            ("reply".to_owned(), Some("root".to_owned())),
            ("nested".to_owned(), Some("reply".to_owned())),
        ]
    );

    ctx.delete().await;
}

// [spec:pgorm:sem:pipeline.self-join+1/test]    the same query through an
// embedded pipeline: the far side is renamed before it crosses
#[pgorm_macros::test]
async fn embedded_self_join_renames_before_crossing() {
    let ctx = TestContext::new("pipeline_self_join_embedded").await;
    let db = ctx.db.get().await.unwrap();
    create_entity_table(&db, employee::Entity).await;
    seed_employees(&db).await;

    let boss = alias("boss");
    let managers = Pipeline::from(employee::Entity)
        .filter_with(|binder| employee::Column::Name.ne(binder.bind("Grace")))
        .select((
            employee::Column::Id.as_(alias("manager_pk")),
            employee::Column::Name.as_(boss),
        ));
    let pipeline = Pipeline::from(employee::Entity)
        .join(
            JoinSide::Inner,
            managers.named(MANAGER),
            employee::Column::ManagerId.eq(col(MANAGER, alias("manager_pk"))),
        )
        .select((employee::Column::Name, col(MANAGER, boss)))
        .sort(employee::Column::Name);
    let (sql, values) = pipeline.clone().into_sql().unwrap();
    assert!(parsed_select(&sql).with_clause.is_some(), "{sql}");
    assert_eq!(values.0.len(), 1);

    let rows: Vec<(String, String)> = pipeline.into_tuple().unwrap().all(&db).await.unwrap();
    assert_eq!(
        rows,
        vec![
            ("Grace".to_owned(), "Ada".to_owned()),
            ("Linus".to_owned(), "Ada".to_owned()),
        ]
    );

    ctx.delete().await;
}

// [spec:pgorm:req:pipeline.params+4/test]    a bound derivation the
// optimizer prunes must not leave its value behind: the statement executes
// with exactly the parameters it asks for
#[pgorm_macros::test]
async fn pruned_binding_executes() {
    let ctx = TestContext::new("pipeline_pruned_binding").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();
    seed(&db).await;

    let pipeline = Pipeline::from(customer::Entity)
        .derive_with(|binder| [binder.bind(42_i32).as_(alias("unused"))])
        .select(C::Name)
        .sort(C::Name);
    let (sql, values) = pipeline.clone().into_sql().unwrap();
    assert!(!sql.contains('$'), "{sql}");
    assert!(values.0.is_empty(), "{values:?}");

    let rows: Vec<(String,)> = pipeline.into_tuple().unwrap().all(&db).await.unwrap();
    assert_eq!(
        rows,
        vec![
            ("Alice".to_owned(),),
            ("Bob".to_owned(),),
            ("Cleo".to_owned(),),
        ]
    );

    ctx.delete().await;
}

// [spec:pgorm:req:pipeline.params+4/test]    a surviving placeholder
// renumbered past a pruned one binds the right value on the server
#[pgorm_macros::test]
async fn renumbered_binding_executes() {
    let ctx = TestContext::new("pipeline_renumbered_binding").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();
    seed(&db).await;

    let pipeline = Pipeline::from(customer::Entity)
        .derive_with(|binder| [binder.bind(999_i32).as_(alias("unused"))])
        .filter_with(|binder| C::Name.eq(binder.bind("Alice")))
        .select(C::Name);
    let (sql, values) = pipeline.clone().into_sql().unwrap();
    assert!(sql.contains("$1"), "{sql}");
    assert!(!sql.contains("$2"), "{sql}");
    assert_eq!(values.0.len(), 1);

    let rows: Vec<(String,)> = pipeline.into_tuple().unwrap().all(&db).await.unwrap();
    assert_eq!(rows, vec![("Alice".to_owned(),)]);

    ctx.delete().await;
}

// [spec:pgorm:req:pipeline.params+4/test]    an embedded pipeline's pruned
// binding and the consumer's surviving one: the rebase offsets and the
// census compose, and the joined rows decode
#[pgorm_macros::test]
async fn composed_prune_renumbers_across_the_join() {
    let ctx = TestContext::new("pipeline_composed_prune").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();
    seed(&db).await;

    let total = alias("total");
    let totals = Pipeline::from(order::Entity)
        .derive_with(|binder| [binder.bind(0_i32).as_(alias("unused"))])
        .select((O::CustomerId, O::Total));
    let pipeline = Pipeline::from(customer::Entity)
        .join(JoinSide::Inner, totals, C::Id.eq(CUSTOMER_ID))
        .filter_with(|binder| total.gt(binder.bind(rust_dec(24.0))))
        .select((C::Name, total))
        .sort(total.desc());
    let (sql, values) = pipeline.clone().into_sql().unwrap();
    assert!(sql.contains("$1"), "{sql}");
    assert!(!sql.contains("$2"), "{sql}");
    assert_eq!(values.0.len(), 1);

    let rows: Vec<(String, Decimal)> = pipeline.into_tuple().unwrap().all(&db).await.unwrap();
    assert_eq!(
        rows,
        vec![
            ("Alice".to_owned(), rust_dec(30.00)),
            ("Bob".to_owned(), rust_dec(25.50)),
            ("Bob".to_owned(), rust_dec(24.50)),
        ]
    );

    ctx.delete().await;
}

// [spec:pgorm:sem:pipeline.select-sources+4/test]    two sources whose column
// names collide decode whole models under their own prefixes — the
// _expr_N dissolution, proven by rows rather than by the emitted string
#[pgorm_macros::test]
async fn select_sources_decodes_colliding_columns() {
    let ctx = TestContext::new("pipeline_select_sources_collide").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();
    seed(&db).await;

    let pipeline = Pipeline::from(order::Entity)
        .join(JoinSide::Inner, customer::Entity, O::CustomerId.eq(C::Id))
        .sort(O::Total);
    let (sql, _) = pipeline
        .clone()
        .select_sources((order::Entity, customer::Entity))
        .into_sql()
        .unwrap();
    assert!(!sql.contains("_expr_"), "{sql}");

    let rows: Vec<(Option<order::Model>, Option<customer::Model>)> = pipeline
        .select_sources((order::Entity, customer::Entity))
        .all(&db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 6);
    for (order, customer) in &rows {
        let (order, customer) = (order.as_ref().unwrap(), customer.as_ref().unwrap());
        assert_eq!(order.customer_id, customer.id);
    }
    let named: Vec<(Decimal, &str)> = rows
        .iter()
        .map(|(order, customer)| {
            (
                order.as_ref().unwrap().total,
                customer.as_ref().unwrap().name.as_str(),
            )
        })
        .collect();
    assert_eq!(
        named,
        vec![
            (rust_dec(5.00), "Cleo"),
            (rust_dec(10.00), "Alice"),
            (rust_dec(20.00), "Alice"),
            (rust_dec(24.50), "Bob"),
            (rust_dec(25.50), "Bob"),
            (rust_dec(30.00), "Alice"),
        ]
    );

    ctx.delete().await;
}

// [spec:pgorm:sem:pipeline.select-sources+4/test]    a named restatement
// decodes both occurrences of one table: employee beside manager, whole
// models on each side
#[pgorm_macros::test]
async fn select_sources_named_self_join_decodes_both_sides() {
    let ctx = TestContext::new("pipeline_select_sources_self_join").await;
    let db = ctx.db.get().await.unwrap();
    create_entity_table(&db, employee::Entity).await;
    seed_employees(&db).await;

    let rows: Vec<(Option<employee::Model>, Option<employee::Model>)> =
        Pipeline::from(employee::Entity)
            .join(
                JoinSide::Inner,
                employee::Entity.named(MANAGER),
                employee::Column::ManagerId.eq(col(MANAGER, ID)),
            )
            .sort(employee::Column::Name)
            .select_sources((employee::Entity, employee::Entity.named(MANAGER)))
            .all(&db)
            .await
            .unwrap();

    let named: Vec<(&str, &str)> = rows
        .iter()
        .map(|(employee, manager)| {
            (
                employee.as_ref().unwrap().name.as_str(),
                manager.as_ref().unwrap().name.as_str(),
            )
        })
        .collect();
    assert_eq!(
        named,
        vec![("Alan", "Grace"), ("Grace", "Ada"), ("Linus", "Ada")]
    );
    for (employee, manager) in &rows {
        assert_eq!(
            employee.as_ref().unwrap().manager_id,
            Some(manager.as_ref().unwrap().id)
        );
    }

    ctx.delete().await;
}

// [spec:pgorm:sem:pipeline.select-sources+4/test]    under a right join the
// *left* side is the absent one, and the first listed source decodes None —
// what the all-optional row type exists to carry
#[pgorm_macros::test]
async fn right_join_leaves_the_first_position_none() {
    let ctx = TestContext::new("pipeline_select_sources_right").await;
    let db = ctx.db.get().await.unwrap();
    create_entity_table(&db, message::Entity).await;

    let root = message::ActiveModel {
        body: set("root"),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    let reply = message::ActiveModel {
        body: set("reply"),
        parent_id: set(root.id),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    message::ActiveModel {
        body: set("nested"),
        parent_id: set(reply.id),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();

    let pipeline = Pipeline::from(message::Entity)
        .join(
            JoinSide::Right,
            message::Entity.named(PARENT),
            message::Column::ParentId.eq(col(PARENT, ID)),
        )
        .sort(col(PARENT, ID));
    let (sql, _) = pipeline
        .clone()
        .select_sources((message::Entity, message::Entity.named(PARENT)))
        .into_sql()
        .unwrap();
    assert!(sql.contains("RIGHT"), "{sql}");

    let rows: Vec<(Option<message::Model>, Option<message::Model>)> = pipeline
        .select_sources((message::Entity, message::Entity.named(PARENT)))
        .all(&db)
        .await
        .unwrap();
    let named: Vec<(Option<&str>, Option<&str>)> = rows
        .iter()
        .map(|(child, parent)| {
            (
                child.as_ref().map(|m| m.body.as_str()),
                parent.as_ref().map(|m| m.body.as_str()),
            )
        })
        .collect();
    assert_eq!(
        named,
        vec![
            (Some("reply"), Some("root")),
            (Some("nested"), Some("reply")),
            (None, Some("nested")),
        ]
    );

    ctx.delete().await;
}

// [spec:pgorm:sem:pipeline.select-sources+4/test]    the allowed set composes
// live ahead of the terminal: filter, derive, sort, take and a join, then
// whole models out
#[pgorm_macros::test]
async fn select_sources_composes_with_allowed_stages_live() {
    let ctx = TestContext::new("pipeline_select_sources_allowed").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();
    seed(&db).await;

    let rows: Vec<(Option<order::Model>, Option<customer::Model>)> = Pipeline::from(order::Entity)
        .join(JoinSide::Inner, customer::Entity, O::CustomerId.eq(C::Id))
        .filter_with(|binder| O::Total.gt(binder.bind(rust_dec(9.0))))
        .derive(O::Total.mul(2).as_(alias("doubled")))
        .sort(O::Total.desc())
        .take(2)
        .select_sources((order::Entity, customer::Entity))
        .all(&db)
        .await
        .unwrap();
    let named: Vec<(Decimal, &str)> = rows
        .iter()
        .map(|(order, customer)| {
            (
                order.as_ref().unwrap().total,
                customer.as_ref().unwrap().name.as_str(),
            )
        })
        .collect();
    assert_eq!(
        named,
        vec![(rust_dec(30.00), "Alice"), (rust_dec(25.50), "Bob")]
    );

    ctx.delete().await;
}

// [spec:pgorm:sem:pipeline.select-sources+4/test]    one and one_opt carry the
// terminal's take-1 semantics: first row of the sorted pipeline, RecordNotFound
// or None when nothing matches
#[pgorm_macros::test]
async fn select_sources_one_takes_one() {
    let ctx = TestContext::new("pipeline_select_sources_one").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();
    seed(&db).await;

    let biggest = Pipeline::from(order::Entity)
        .join(JoinSide::Inner, customer::Entity, O::CustomerId.eq(C::Id))
        .sort(O::Total.desc());
    let (order, customer) = biggest
        .clone()
        .select_sources((order::Entity, customer::Entity))
        .one(&db)
        .await
        .unwrap();
    assert_eq!(order.unwrap().total, rust_dec(30.00));
    assert_eq!(customer.unwrap().name, "Alice");

    let none = Pipeline::from(customer::Entity)
        .filter_with(|binder| C::Name.eq(binder.bind("Zed")))
        .select_sources(customer::Entity)
        .one(&db)
        .await;
    assert!(
        matches!(none, Err(pgorm::Error::RecordNotFound)),
        "{none:?}"
    );

    let nobody: Option<Option<customer::Model>> = Pipeline::from(customer::Entity)
        .filter_with(|binder| C::Name.eq(binder.bind("Zed")))
        .select_sources(customer::Entity)
        .one_opt(&db)
        .await
        .unwrap();
    assert!(nobody.is_none());

    ctx.delete().await;
}

// [spec:pgorm:sem:pipeline.select-sources+4/test]    the widest list the
// terminal takes, decoded live: six sources, each position read back from
// its own s{i}_ prefix into its own model — two of them the same table under
// two names, so a crossed prefix decodes cleanly into the wrong row — and a
// left join's unmatched side decoding None in the middle of the list
#[pgorm_macros::test]
async fn select_sources_decodes_six_sources_by_position() {
    let ctx = TestContext::new("pipeline_select_sources_six").await;
    create_tables(&ctx.db).await.unwrap();
    let db = ctx.db.get().await.unwrap();

    // Every row's ids and text are its own, so a position that read a
    // neighbour's prefix would come back holding the neighbour's row. The
    // expected models are what each insert returned: a reading that never
    // passes through the sources projection.
    let bakery = bakery::ActiveModel {
        id: set(501),
        name: set("Harbour Bakery"),
        profit_margin: set(12.5),
    }
    .insert(&db)
    .await
    .unwrap();
    let customer = customer::ActiveModel {
        id: set(301),
        name: set("Cleo"),
        notes: set(Some("collects at noon".to_owned())),
    }
    .insert(&db)
    .await
    .unwrap();
    let order = order::ActiveModel {
        id: set(201),
        total: set(rust_dec(17.25)),
        bakery_id: set(bakery.id),
        customer_id: set(customer.id),
        placed_at: set(Offset::UTC.to_datetime(Timestamp::now())),
    }
    .insert(&db)
    .await
    .unwrap();
    let tart = cake::ActiveModel {
        id: set(401),
        name: set("Lemon Tart"),
        price: set(rust_dec(6.25)),
        bakery_id: set(bakery.id),
        gluten_free: set(true),
        serial: set(Uuid::new_v4()),
    }
    .insert(&db)
    .await
    .unwrap();
    // A cake no bakery claims: the left join to `bakery` has nothing to match.
    let loaf = cake::ActiveModel {
        id: set(402),
        name: set("Market Loaf"),
        price: set(rust_dec(4.75)),
        bakery_id: set(None::<i32>),
        gluten_free: set(false),
        serial: set(Uuid::new_v4()),
    }
    .insert(&db)
    .await
    .unwrap();
    let tart_line = lineitem::ActiveModel {
        id: set(101),
        price: set(rust_dec(6.25)),
        quantity: set(2),
        order_id: set(order.id),
        cake_id: set(tart.id),
    }
    .insert(&db)
    .await
    .unwrap();
    let loaf_line = lineitem::ActiveModel {
        id: set(102),
        price: set(rust_dec(4.75)),
        quantity: set(1),
        order_id: set(order.id),
        cake_id: set(loaf.id),
    }
    .insert(&db)
    .await
    .unwrap();

    let rows = Pipeline::from(order::Entity)
        .join(JoinSide::Inner, customer::Entity, O::CustomerId.eq(C::Id))
        .join(
            JoinSide::Inner,
            lineitem::Entity,
            O::Id.eq(lineitem::Column::OrderId),
        )
        .join(
            JoinSide::Inner,
            cake::Entity,
            lineitem::Column::CakeId.eq(cake::Column::Id),
        )
        .join(
            JoinSide::Left,
            bakery::Entity,
            cake::Column::BakeryId.eq(bakery::Column::Id),
        )
        // Each line of the order beside the order's other line: the same
        // table, read a second time under a name.
        .join(
            JoinSide::Inner,
            lineitem::Entity.named(OTHER_LINE),
            col(OTHER_LINE, ORDER_ID)
                .eq(O::Id)
                .and(col(OTHER_LINE, ID).ne(lineitem::Column::Id)),
        )
        .sort(lineitem::Column::Id)
        .select_sources((
            order::Entity,
            customer::Entity,
            lineitem::Entity,
            cake::Entity,
            bakery::Entity,
            lineitem::Entity.named(OTHER_LINE),
        ))
        .all(&db)
        .await
        .unwrap();

    assert_eq!(
        rows,
        vec![
            (
                Some(order.clone()),
                Some(customer.clone()),
                Some(tart_line.clone()),
                Some(tart),
                Some(bakery),
                Some(loaf_line.clone()),
            ),
            (
                Some(order),
                Some(customer),
                Some(loaf_line),
                Some(loaf),
                None,
                Some(tart_line),
            ),
        ]
    );

    ctx.delete().await;
}

mod cast_probe {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "cast_probe")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        #[pgorm(
            select_as = "text",
            save_as = "numeric",
            column_type = ColumnType::named("numeric")
        )]
        pub amount: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

// [spec:pgorm:sem:pipeline.select-sources+4/test]    a select_as override and
// the enum-array default alike reach the sources projection: a numeric column
// read as text decodes, and a tea[] column reads back under text[]
#[pgorm_macros::test]
async fn select_sources_applies_read_casts() -> Result<(), pgorm::Error> {
    use pgorm::QueryFilter;

    let ctx = TestContext::new("pipeline_select_sources_casts").await;
    let db = ctx.db.get().await?;
    db.batch_execute(
        r#"CREATE TABLE cast_probe (id integer PRIMARY KEY, amount numeric NOT NULL);
           INSERT INTO cast_probe VALUES (1, 12.5);
           CREATE TYPE tea AS ENUM ('EverydayTea', 'BreakfastTea');
           CREATE TABLE tea_pot (id integer PRIMARY KEY, teas tea[] NOT NULL);
           INSERT INTO tea_pot VALUES (1, ARRAY['EverydayTea']::tea[]);"#,
    )
    .await?;

    let expected = cast_probe::Model {
        id: 1,
        amount: "12.5".to_owned(),
    };
    // The graph writer already honours the override; it is the control.
    assert_eq!(
        cast_probe::Entity::graph().all(&db).await?,
        std::slice::from_ref(&expected)
    );
    assert_eq!(
        Pipeline::from(cast_probe::Entity)
            .select_sources(cast_probe::Entity)
            .all(&db)
            .await?,
        [Some(expected)]
    );

    let brewed = Pipeline::from(tea_pot::Entity)
        .select_sources(tea_pot::Entity)
        .all(&db)
        .await?;
    assert_eq!(
        brewed,
        [Some(tea_pot::Model {
            id: 1,
            teas: vec![Tea::EverydayTea],
        })]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

mod tea_pot {
    use pgorm::entity::prelude::*;

    #[derive(Debug, Clone, PartialEq, Eq, ActiveEnum, Copy)]
    #[pgorm(enum_name = "tea")]
    pub enum Tea {
        #[pgorm(string_value = "EverydayTea")]
        EverydayTea,
        #[pgorm(string_value = "BreakfastTea")]
        BreakfastTea,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "tea_pot")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub teas: Vec<Tea>,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}
use tea_pot::Tea;

/// Deduplicating before projecting is a different question from projecting
/// before deduplicating, and the pipeline's stage order is which one is asked.
///
/// Over a relation whose columns are still a star the deduplication used to be
/// rendered `DISTINCT ON (.., *)` — a key PostgreSQL's grammar has no reading
/// of at all — so neither answer came back. The rows here separate the two:
/// four sightings, two of them identical, and two names shared across
/// different ids. Deduplicating whole rows removes only the identical pair;
/// deduplicating the projected name removes far more.
// [spec:pgorm:req:pipeline.compose/test]
#[pgorm_macros::test]
async fn deduplicating_then_projecting_keeps_its_rows() -> Result<(), pgorm::Error> {
    let ctx = TestContext::new("pipeline_distinct_star_projection").await;
    let db = ctx.db.get().await?;
    let sightings = alias("sightings");
    db.batch_execute(
        r#"CREATE TABLE sightings (id integer NOT NULL, name text NOT NULL);
           INSERT INTO sightings VALUES (1, 'ada'), (2, 'ada'), (3, 'grace'), (3, 'grace');"#,
    )
    .await?;

    // The server's own answer to both questions, in SQL this suite wrote.
    async fn control(db: &impl ConnectionTrait, sql: &str) -> Vec<String> {
        let mut names: Vec<String> = db
            .query_all(sql, &[])
            .await
            .expect("the control query runs")
            .iter()
            .map(|row| row.get(0))
            .collect();
        names.sort();
        names
    }
    let whole_rows = control(&db, "SELECT name FROM (SELECT DISTINCT * FROM sightings) t").await;
    let projected = control(&db, "SELECT DISTINCT name FROM sightings").await;
    assert_eq!(whole_rows, ["ada", "ada", "grace"]);
    assert_eq!(projected, ["ada", "grace"]);

    // Deduplicate, then project: the identical pair collapses, the two `ada`
    // rows with different ids do not.
    let mut deduplicated: Vec<String> = Pipeline::from(sightings)
        .distinct()
        .select(col(sightings, NAME))
        .into_tuple::<(String,)>()?
        .all(&db)
        .await?
        .into_iter()
        .map(|(name,)| name)
        .collect();
    deduplicated.sort();
    assert_eq!(deduplicated, whole_rows);

    // Project, then deduplicate: the other question, and the other answer.
    let mut narrowed: Vec<String> = Pipeline::from(sightings)
        .select(col(sightings, NAME))
        .distinct()
        .into_tuple::<(String,)>()?
        .all(&db)
        .await?
        .into_iter()
        .map(|(name,)| name)
        .collect();
    narrowed.sort();
    assert_eq!(narrowed, projected);

    drop(db);
    ctx.delete().await;
    Ok(())
}
