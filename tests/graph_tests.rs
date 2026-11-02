//! Live coverage for the N-ary relational read.
//!
//! Three decoded sources with same-named columns and absent middles, a
//! required slot decoding without `Option`, the same table entering twice
//! under an alias with a call-site `ON`, a junction folded in by
//! `related_maybe` and as a required path by `related_one`, a `via` hop
//! re-tightened by a required slot, a path through one table twice under
//! `via_as` hops, the terminals landing on the ordinary selector machinery,
//! and `limit` / `offset` counting rows rather than roots.
//!
//! Run locally:
//! `DATABASE_URL="postgres://postgres:postgres@localhost:54329" cargo test --test graph_tests`

#![allow(unused_imports, dead_code)]

pub mod common;

use futures::TryStreamExt;
use pgorm::tests_cfg::{cake, cake_filling, filling, fruit, vendor};
use pgorm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, Error, Linked, LinkedAlias,
    LoaderTrait, Opt, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, RelationTrait,
    SelectGraph, alias,
};
use pgorm_query::{Expr, IntoCondition};
use pretty_assertions::assert_eq;
use std::num::NonZeroU64;
use tokio_postgres::error::SqlState;

pub use common::TestContext;

/// The fixture: three cakes, one of which has no junction row at all, one
/// whose filling has no vendor, and a cake with two fruits.
const SCHEMA: &str = r#"
    CREATE TABLE "cake" ("id" int PRIMARY KEY, "name" text NOT NULL);
    CREATE TABLE "vendor" ("id" int PRIMARY KEY, "name" text NOT NULL);
    CREATE TABLE "filling" ("id" int PRIMARY KEY, "name" text NOT NULL, "vendor_id" int);
    CREATE TABLE "cake_filling" (
        "cake_id" int, "filling_id" int, PRIMARY KEY ("cake_id", "filling_id")
    );
    CREATE TABLE "fruit" ("id" int PRIMARY KEY, "name" text NOT NULL, "cake_id" int);
    INSERT INTO "cake" VALUES (1, 'Cheesecake'), (2, 'Lonely'), (3, 'Mudcake');
    INSERT INTO "vendor" VALUES (7, 'Sweet Supplies');
    INSERT INTO "filling" VALUES (5, 'Cream', 7), (6, 'Orphanite', NULL);
    INSERT INTO "cake_filling" VALUES (1, 5), (3, 6);
    INSERT INTO "fruit" VALUES (10, 'Cherry', 1), (11, 'Peach', 1);
"#;

async fn schema(db: &DatabaseConnection) -> Result<(), Error> {
    db.batch_execute(SCHEMA).await
}

fn cheesecake() -> cake::Model {
    cake::Model {
        id: 1,
        name: "Cheesecake".to_owned(),
    }
}

fn lonely() -> cake::Model {
    cake::Model {
        id: 2,
        name: "Lonely".to_owned(),
    }
}

fn mudcake() -> cake::Model {
    cake::Model {
        id: 3,
        name: "Mudcake".to_owned(),
    }
}

fn cream() -> filling::Model {
    filling::Model {
        id: 5,
        name: "Cream".to_owned(),
        vendor_id: Some(7),
        ignored_attr: 0,
    }
}

fn orphanite() -> filling::Model {
    filling::Model {
        id: 6,
        name: "Orphanite".to_owned(),
        vendor_id: None,
        ignored_attr: 0,
    }
}

fn sweet_supplies() -> vendor::Model {
    vendor::Model {
        id: 7,
        name: "Sweet Supplies".to_owned(),
    }
}

fn cherry() -> fruit::Model {
    fruit::Model {
        id: 10,
        name: "Cherry".to_owned(),
        cake_id: Some(1),
    }
}

fn peach() -> fruit::Model {
    fruit::Model {
        id: 11,
        name: "Peach".to_owned(),
        cake_id: Some(1),
    }
}

/// A page size written as a literal; the `NonZeroU64` is the check.
fn page_size(size: u64) -> NonZeroU64 {
    NonZeroU64::new(size).expect("page size is non-zero")
}

// [spec:pgorm:def:query.graph+1/test]    a graph declared as a root plus joined
// sources executes as one statement whose rows carry every declared source
// [spec:pgorm:sem:query.graph.writer+4/test]    three sources each carrying a
// `name` and an `id` column decode without collision, because each is
// projected under its own `s{i}_` prefix
// [spec:pgorm:sem:query.graph.decode+1/test]    an unmatched LEFT JOIN reads as
// `None` through the absence witness, at the middle of a chain and at its tail
#[pgorm_macros::test]
async fn graph_three_sources() -> Result<(), Error> {
    let ctx = TestContext::new("graph_three_sources").await;
    let db = ctx.db.get().await?;
    schema(&db).await?;

    let rows = cake::Entity::graph()
        .via(cake_filling::Relation::Cake.def().rev())
        .join_maybe::<filling::Entity>(cake_filling::Relation::Filling.def())
        .join_maybe::<vendor::Entity>(filling::Relation::Vendor.def())
        .order_by_asc(cake::Column::Id)
        .all(&db)
        .await?;

    assert_eq!(
        rows,
        [
            (cheesecake(), Some(cream()), Some(sweet_supplies())),
            // No junction row at all: the middle is absent, and so is the tail.
            (lonely(), None, None),
            // A present middle whose own foreign key is NULL.
            (mudcake(), Some(orphanite()), None),
        ]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:query.graph.slots+2/test]    a required slot joins INNER and
// decodes as a bare `Model` — "absent" is not a value the caller unwraps —
// while the same relation as an optional slot keeps the unmatched roots
#[pgorm_macros::test]
async fn graph_required_slot() -> Result<(), Error> {
    let ctx = TestContext::new("graph_required_slot").await;
    let db = ctx.db.get().await?;
    schema(&db).await?;

    let required: Vec<(cake::Model, fruit::Model)> = cake::Entity::graph()
        .join_one::<fruit::Entity>(cake::Relation::Fruit.def())
        .order_by_asc(cake::Column::Id)
        .order_by_asc(fruit::Column::Id)
        .all(&db)
        .await?;

    assert_eq!(
        required,
        [(cheesecake(), cherry()), (cheesecake(), peach())]
    );

    let optional: Vec<(cake::Model, Option<fruit::Model>)> = cake::Entity::graph()
        .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
        .order_by_asc(cake::Column::Id)
        .order_by_asc(fruit::Column::Id)
        .all(&db)
        .await?;

    assert_eq!(
        optional,
        [
            (cheesecake(), Some(cherry())),
            (cheesecake(), Some(peach())),
            (lonely(), None),
            (mudcake(), None),
        ]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:query.graph.aliases+1/test]    the same table enters the graph
// twice under a caller-bound alias, which is the slot's identifier in the ON
// clause and in the projection alike, so both copies decode independently
#[pgorm_macros::test]
async fn graph_same_table_twice() -> Result<(), Error> {
    let ctx = TestContext::new("graph_same_table_twice").await;
    let db = ctx.db.get().await?;
    schema(&db).await?;

    let rows = cake::Entity::graph()
        .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
        .join_maybe_as::<fruit::Entity>(
            cake::Relation::Fruit.def().on_condition(|_left, right| {
                Expr::col((right, fruit::Column::Name))
                    .like("Ch%")
                    .into_condition()
            }),
            alias("starts_with_ch"),
        )
        .order_by_asc(cake::Column::Id)
        .order_by_asc(fruit::Column::Id)
        .all(&db)
        .await?;

    assert_eq!(
        rows,
        [
            // The aliased copy is narrowed by the call-site ON, so the second
            // slot repeats Cherry against both of the first slot's rows.
            (cheesecake(), Some(cherry()), Some(cherry())),
            (cheesecake(), Some(peach()), Some(cherry())),
            (lonely(), None, None),
            (mudcake(), None, None),
        ]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:req:query.graph.aliases+1/test]    `join_maybe_filtered` puts the
// call-site predicate in ON, where unmatched roots survive decoding `None`,
// rather than in WHERE, where the join silently tightens to INNER
#[pgorm_macros::test]
async fn graph_filtered_join_keeps_unmatched_roots() -> Result<(), Error> {
    let ctx = TestContext::new("graph_filtered_join").await;
    let db = ctx.db.get().await?;
    schema(&db).await?;

    let in_on = cake::Entity::graph()
        .join_maybe_filtered::<fruit::Entity, _>(cake::Relation::Fruit.def(), |_left, right| {
            Expr::col((right, fruit::Column::Name))
                .like("Ch%")
                .into_condition()
        })
        .order_by_asc(cake::Column::Id)
        .all(&db)
        .await?;

    assert_eq!(
        in_on,
        [
            (cheesecake(), Some(cherry())),
            (lonely(), None),
            (mudcake(), None),
        ]
    );

    // The same predicate through `filter` lands in WHERE, and the NULLs of an
    // unmatched row fail it.
    let in_where = cake::Entity::graph()
        .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
        .filter(fruit::Column::Name.like("Ch%"))
        .order_by_asc(cake::Column::Id)
        .all(&db)
        .await?;

    assert_eq!(in_where, [(cheesecake(), Some(cherry()))]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:query.graph.slots+2/test]    `related_maybe` folds the whole
// described path in — junction hop included — and a required slot joined
// through a `via` hop re-tightens the chain to INNER semantics end to end
#[pgorm_macros::test]
async fn graph_related_and_via() -> Result<(), Error> {
    let ctx = TestContext::new("graph_related_and_via").await;
    let db = ctx.db.get().await?;
    schema(&db).await?;

    let folded = cake::Entity::graph()
        .related_maybe::<filling::Entity>()
        .order_by_asc(cake::Column::Id)
        .all(&db)
        .await?;

    assert_eq!(
        folded,
        [
            (cheesecake(), Some(cream())),
            (lonely(), None),
            (mudcake(), Some(orphanite())),
        ]
    );

    // `via` alone is LEFT, so it cannot erase a root; the required slot behind
    // it does, because its ON references the middle's columns.
    let retightened: Vec<(cake::Model, filling::Model)> = cake::Entity::graph()
        .via(cake_filling::Relation::Cake.def().rev())
        .join_one::<filling::Entity>(cake_filling::Relation::Filling.def())
        .order_by_asc(cake::Column::Id)
        .all(&db)
        .await?;

    assert_eq!(
        retightened,
        [(cheesecake(), cream()), (mudcake(), orphanite())]
    );

    // A via-then-join_one chain that also re-tightens at its tail: only the
    // filling with a vendor survives.
    let tail: Vec<(cake::Model, vendor::Model)> = cake::Entity::graph()
        .via(cake_filling::Relation::Cake.def().rev())
        .via(cake_filling::Relation::Filling.def())
        .join_one::<vendor::Entity>(filling::Relation::Vendor.def())
        .order_by_asc(cake::Column::Id)
        .all(&db)
        .await?;

    assert_eq!(tail, [(cheesecake(), sweet_supplies())]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:query.graph.slots+2/test]    `related_one` folds the described
// path in as a required source — a direct relation and a junction path alike —
// so a root row with no related row drops out and the rest decode bare
#[pgorm_macros::test]
async fn graph_related_one_drops_unmatched_roots() -> Result<(), Error> {
    let ctx = TestContext::new("graph_related_one").await;
    let db = ctx.db.get().await?;
    schema(&db).await?;

    // Lonely and Mudcake have no fruit.
    let direct: Vec<(cake::Model, fruit::Model)> = cake::Entity::graph()
        .related_one::<fruit::Entity>()
        .order_by_asc(cake::Column::Id)
        .order_by_asc(fruit::Column::Id)
        .all(&db)
        .await?;

    assert_eq!(direct, [(cheesecake(), cherry()), (cheesecake(), peach())]);

    // Lonely has no junction row, so the junction hop is where it drops out.
    let junction: Vec<(cake::Model, filling::Model)> = cake::Entity::graph()
        .related_one::<filling::Entity>()
        .order_by_asc(cake::Column::Id)
        .all(&db)
        .await?;

    assert_eq!(
        junction,
        [(cheesecake(), cream()), (mudcake(), orphanite())]
    );

    // A required slot behind an optional one: the optional slot keeps
    // nothing alive that the required path refuses.
    let both: Vec<(cake::Model, Option<fruit::Model>, filling::Model)> = cake::Entity::graph()
        .related_maybe::<fruit::Entity>()
        .related_one::<filling::Entity>()
        .order_by_asc(cake::Column::Id)
        .order_by_asc(fruit::Column::Id)
        .all(&db)
        .await?;

    assert_eq!(
        both,
        [
            (cheesecake(), Some(cherry()), cream()),
            (cheesecake(), Some(peach()), cream()),
            (mudcake(), None, orphanite()),
        ]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:query.graph.cursor+4/test]    a graph's cursor orders on the
// root and tiebreaks on the declared slot's primary key, so a page that ends
// inside a run of rows sharing a root resumes exactly through `after_with` —
// where the order-column boundary can only skip the rest of the run — and the
// keyset's arity check is the machinery's, unmoved
#[pgorm_macros::test]
async fn graph_cursor_tie_straddles_a_page_boundary() -> Result<(), Error> {
    let ctx = TestContext::new("graph_cursor_tie").await;
    let db = ctx.db.get().await?;
    schema(&db).await?;

    // In key order the rows run (1, Cherry), (1, Peach), (2, -), (3, -): the
    // first two share a root, so a page of one ends mid-run.
    let cursor = || {
        cake::Entity::graph()
            .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
            .cursor_by(cake::Column::Id)
    };

    assert_eq!(
        cursor().first(1).all(&db).await?,
        [(cheesecake(), Some(cherry()))]
    );

    // The whole key of that last row resumes from it exactly.
    assert_eq!(
        cursor().after_with((1, 10)).first(1).all(&db).await?,
        [(cheesecake(), Some(peach()))]
    );

    // The order-column boundary can say no more than "past every row of cake
    // 1", so Peach is lost — the documented fallback, not a graph novelty.
    assert_eq!(
        cursor().after(1).first(1).all(&db).await?,
        [(lonely(), None)]
    );

    // Paging on past the matched run reaches the unmatched roots, whose slot
    // decodes `None`.
    assert_eq!(
        cursor().after_with((1, 11)).first(2).all(&db).await?,
        [(lonely(), None), (mudcake(), None)]
    );

    // `before_with` mirrors it, and `last` takes the window from the far end.
    assert_eq!(
        cursor().before_with((2, 0)).last(2).all(&db).await?,
        [
            (cheesecake(), Some(cherry())),
            (cheesecake(), Some(peach()))
        ]
    );

    // Descending pages the same run from the other end.
    assert_eq!(
        cursor()
            .after_with((1, 11))
            .desc()
            .first(1)
            .all(&db)
            .await?,
        [(cheesecake(), Some(cherry()))]
    );

    // Neither arity: reported when the filters are composed, not panicked.
    assert_eq!(
        cursor()
            .after_with((1, 2, 3))
            .first(1)
            .all(&db)
            .await
            .unwrap_err()
            .to_string(),
        "Query Error: cursor boundary of arity 3 does not match 1 or 2 order column(s)"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:query.graph.cursor+4/test]    `cursor_by_on` orders on the
// slot its position names, typed against that slot's entity, completes that
// slot's own key, and then tiebreaks on the root's primary key
#[pgorm_macros::test]
async fn graph_cursor_on_a_slot() -> Result<(), Error> {
    let ctx = TestContext::new("graph_cursor_on_slot").await;
    let db = ctx.db.get().await?;
    schema(&db).await?;

    let cursor = || {
        cake::Entity::graph()
            .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
            .cursor_by_on::<1, _>(fruit::Column::Name)
    };

    // Ordered by the slot's name, the fruitless roots sort last: PostgreSQL
    // puts NULLs at the end of an ascending order.
    assert_eq!(
        cursor().first(3).all(&db).await?,
        [
            (cheesecake(), Some(cherry())),
            (cheesecake(), Some(peach())),
            (lonely(), None),
        ]
    );

    // The whole key is the slot's order column, its remaining primary key,
    // then the root's.
    assert_eq!(
        cursor()
            .after_with(("Cherry", 10, 1))
            .first(1)
            .all(&db)
            .await?,
        [(cheesecake(), Some(peach()))]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:query.graph.cursor+4/test]    an `_as` slot's tiebreak is
// qualified by its alias: the bare table is not in the query at all, so a
// tiebreak naming it would be SQL PostgreSQL refuses
#[pgorm_macros::test]
async fn graph_cursor_alias_qualifies_the_tiebreak() -> Result<(), Error> {
    let ctx = TestContext::new("graph_cursor_alias").await;
    let db = ctx.db.get().await?;
    schema(&db).await?;

    let cursor = || {
        cake::Entity::graph()
            .join_maybe_as::<fruit::Entity>(cake::Relation::Fruit.def(), alias("topping"))
            .cursor_by(cake::Column::Id)
    };

    assert_eq!(
        cursor().first(1).all(&db).await?,
        [(cheesecake(), Some(cherry()))]
    );
    assert_eq!(
        cursor().after_with((1, 10)).first(1).all(&db).await?,
        [(cheesecake(), Some(peach()))]
    );

    // The alias is the slot's one identifier for the order columns too.
    assert_eq!(
        cake::Entity::graph()
            .join_maybe_as::<fruit::Entity>(cake::Relation::Fruit.def(), alias("topping"))
            .cursor_by_on::<1, _>(fruit::Column::Id)
            .after_with((10, 1))
            .first(1)
            .all(&db)
            .await?,
        [(cheesecake(), Some(peach()))]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:query.graph.cursor+4/test]    the inherited NULL limitation is
// live on a graph: an unmatched `Opt` slot's primary key is null, so the
// extended boundary's tie disjunct is dead and such a row is reached through
// the order-column boundary instead — documented, not worked around
#[pgorm_macros::test]
async fn graph_cursor_null_tiebreaks() -> Result<(), Error> {
    let ctx = TestContext::new("graph_cursor_null_tiebreak").await;
    let db = ctx.db.get().await?;
    schema(&db).await?;

    let cursor = || {
        cake::Entity::graph()
            .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
            .cursor_by(cake::Column::Id)
    };

    // A null in the extended position makes its comparison null, so the whole
    // key degenerates to the order-column boundary: cake 1's remaining rows
    // are skipped exactly as `after` alone would skip them.
    let past_null = cursor()
        .after_with((1, Option::<i32>::None))
        .first(2)
        .all(&db)
        .await?;
    assert_eq!(past_null, [(lonely(), None), (mudcake(), None)]);
    assert_eq!(past_null, cursor().after(1).first(2).all(&db).await?);

    // Resuming *from* an unmatched root is the same story from the other side:
    // the order column carries the boundary, the null tiebreak contributes
    // nothing, and the next unmatched root is still reached.
    assert_eq!(
        cursor()
            .after_with((2, Option::<i32>::None))
            .first(1)
            .all(&db)
            .await?,
        [(mudcake(), None)]
    );

    // When the null is in the *order* column instead — ordering on an optional
    // slot's own column — no boundary reaches past it: nothing compares
    // against null, and the order-column boundary is that same comparison.
    let by_name = || {
        cake::Entity::graph()
            .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
            .cursor_by_on::<1, _>(fruit::Column::Name)
    };
    assert_eq!(by_name().first(4).all(&db).await?.len(), 4);
    assert_eq!(
        by_name()
            .after_with((Option::<String>::None, Option::<i32>::None, 2))
            .first(1)
            .all(&db)
            .await?,
        []
    );
    assert_eq!(
        by_name()
            .after(Option::<String>::None)
            .first(1)
            .all(&db)
            .await?,
        []
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A fruit whose `name` is NULL, which `fruit::Model` has no `Option` for.
const NULLABLE_SCHEMA: &str = r#"
    CREATE TABLE "cake" ("id" int PRIMARY KEY, "name" text NOT NULL);
    CREATE TABLE "fruit" ("id" int PRIMARY KEY, "name" text, "cake_id" int);
    INSERT INTO "cake" VALUES (1, 'Cheesecake');
    INSERT INTO "fruit" VALUES (10, NULL, 1);
"#;

// [spec:pgorm:sem:query.graph.decode+1/test]    a decode failure of a *present*
// row propagates rather than being read as absence — on a required slot,
// which never consults the witness, and on an optional one, whose witness
// columns are not all NULL
#[pgorm_macros::test]
async fn graph_decode_failure_is_not_absence() -> Result<(), Error> {
    let ctx = TestContext::new("graph_decode_failure").await;
    let db = ctx.db.get().await?;
    db.batch_execute(NULLABLE_SCHEMA).await?;

    let required = cake::Entity::graph()
        .join_one::<fruit::Entity>(cake::Relation::Fruit.def())
        .all(&db)
        .await;
    assert!(
        required.is_err(),
        "a NULL in a required slot's non-Option field is an error: {required:?}"
    );

    let optional = cake::Entity::graph()
        .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
        .all(&db)
        .await;
    assert!(
        optional.is_err(),
        "a matched row that fails to decode is not `None`: {optional:?}"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:query.graph.terminals+2/test]    `all`, `one_opt`, `stream`
// and `PaginatorTrait` all run through one `Selector<GraphRow<E, S>>`, and
// page boundaries fall between rows rather than between root models
#[pgorm_macros::test]
async fn graph_terminals() -> Result<(), Error> {
    let ctx = TestContext::new("graph_terminals").await;
    let db = ctx.db.get().await?;
    schema(&db).await?;

    let graph = || {
        cake::Entity::graph()
            .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
            .order_by_asc(cake::Column::Id)
            .order_by_asc(fruit::Column::Id)
    };

    let all = graph().all(&db).await?;
    assert_eq!(all.len(), 4);

    // `one_opt` injects LIMIT 1 and answers the first-row question.
    assert_eq!(
        graph().one_opt(&db).await?,
        Some((cheesecake(), Some(cherry())))
    );
    assert_eq!(
        graph().filter(cake::Column::Id.gt(99)).one_opt(&db).await?,
        None
    );

    // The stream decodes lazily per item and yields the same rows.
    let streamed: Vec<_> = graph().stream(&db).await?.try_collect().await?;
    assert_eq!(streamed, all);

    // Pagination cuts between rows: Cheesecake's two fruit rows fill page 0
    // whole, and the fruitless cakes make up page 1.
    let paginator = graph().paginate(&db, page_size(2));
    assert_eq!(paginator.fetch_page(0).await?, all[..2]);
    assert_eq!(paginator.fetch_page(1).await?, all[2..]);
    assert_eq!(paginator.num_items().await?, 4);

    assert_eq!(graph().count(&db).await?, 4);

    // A slotless graph decodes as a bare model, not a one-tuple.
    let bare: Vec<cake::Model> = cake::Entity::graph()
        .order_by_asc(cake::Column::Id)
        .all(&db)
        .await?;
    assert_eq!(bare, [cheesecake(), lonely(), mudcake()]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// The grouped fixture: a cake with two of everything, a cake with nothing at
/// all, and a cake whose one filling is shared with the first — so a junction
/// hop fans out, and an ordering on a child column tears the first cake's run
/// in two.
const GROUPED_SCHEMA: &str = r#"
    CREATE TABLE "cake" ("id" int PRIMARY KEY, "name" text NOT NULL);
    CREATE TABLE "vendor" ("id" int PRIMARY KEY, "name" text NOT NULL);
    CREATE TABLE "filling" ("id" int PRIMARY KEY, "name" text NOT NULL, "vendor_id" int);
    CREATE TABLE "cake_filling" (
        "cake_id" int, "filling_id" int, PRIMARY KEY ("cake_id", "filling_id")
    );
    CREATE TABLE "fruit" ("id" int PRIMARY KEY, "name" text NOT NULL, "cake_id" int);
    INSERT INTO "cake" VALUES (1, 'Cheesecake'), (2, 'Lonely'), (3, 'Mudcake');
    INSERT INTO "vendor" VALUES (7, 'Sweet Supplies');
    INSERT INTO "filling" VALUES (5, 'Cream', 7), (6, 'Orphanite', NULL);
    INSERT INTO "cake_filling" VALUES (1, 5), (1, 6), (3, 6);
    INSERT INTO "fruit" VALUES (10, 'Apricot', 1), (11, 'Blueberry', 3), (12, 'Cranberry', 1);
"#;

async fn grouped_schema(db: &DatabaseConnection) -> Result<(), Error> {
    db.batch_execute(GROUPED_SCHEMA).await
}

fn apricot() -> fruit::Model {
    fruit::Model {
        id: 10,
        name: "Apricot".to_owned(),
        cake_id: Some(1),
    }
}

fn blueberry() -> fruit::Model {
    fruit::Model {
        id: 11,
        name: "Blueberry".to_owned(),
        cake_id: Some(3),
    }
}

fn cranberry() -> fruit::Model {
    fruit::Model {
        id: 12,
        name: "Cranberry".to_owned(),
        cake_id: Some(1),
    }
}

// [spec:pgorm:sem:query.graph.grouped+2/test]    the fanout regroups: each root
// appears once with its matching models beneath it, and a root the slot did
// not match reads as an empty `Vec` rather than dropping out — with nothing
// ordered by the caller, the roots come back in pure primary-key order
#[pgorm_macros::test]
async fn graph_grouped_fanout() -> Result<(), Error> {
    let ctx = TestContext::new("graph_grouped_fanout").await;
    let db = ctx.db.get().await?;
    grouped_schema(&db).await?;

    let grouped: Vec<(cake::Model, Vec<fruit::Model>)> = cake::Entity::graph()
        .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
        .order_by_asc(fruit::Column::Id)
        .all_grouped(&db)
        .await?;

    assert_eq!(
        grouped,
        [
            (cheesecake(), vec![apricot(), cranberry()]),
            (mudcake(), vec![blueberry()]),
            // Matched nothing, so an empty `Vec` — the root is not dropped.
            (lonely(), vec![]),
        ]
    );

    // Ordering by nothing is pure primary-key order. Within a bucket the row
    // order is then the server's, so the children are compared as a set.
    let by_key: Vec<(cake::Model, Vec<fruit::Model>)> = cake::Entity::graph()
        .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
        .all_grouped(&db)
        .await?;

    let shape: Vec<(i32, Vec<i32>)> = by_key
        .into_iter()
        .map(|(cake, fruits)| {
            let mut ids: Vec<i32> = fruits.into_iter().map(|fruit| fruit.id).collect();
            ids.sort_unstable();
            (cake.id, ids)
        })
        .collect();

    assert_eq!(shape, [(1, vec![10, 12]), (2, vec![]), (3, vec![11])]);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:query.graph.grouped+2/test]    caller ordering dominates: the
// primary key is appended *behind* what the caller wrote, so a descending
// order on a root column reverses the entries — the constructor-injected
// leading ORDER BY of the pair surface would have silently overruled it
#[pgorm_macros::test]
async fn graph_grouped_caller_ordering_dominates() -> Result<(), Error> {
    let ctx = TestContext::new("graph_grouped_ordering").await;
    let db = ctx.db.get().await?;
    grouped_schema(&db).await?;

    // Descending on the very column the appended key orders ascending: only a
    // trailing key leaves this ordering standing.
    let by_id_desc: Vec<(cake::Model, Vec<fruit::Model>)> = cake::Entity::graph()
        .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
        .order_by_desc(cake::Column::Id)
        .order_by_asc(fruit::Column::Id)
        .all_grouped(&db)
        .await?;

    assert_eq!(
        by_id_desc,
        [
            (mudcake(), vec![blueberry()]),
            (lonely(), vec![]),
            (cheesecake(), vec![apricot(), cranberry()]),
        ]
    );

    // The same domination over a non-key column, where the appended key is a
    // tiebreak only.
    let by_name_desc: Vec<cake::Model> = cake::Entity::graph()
        .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
        .order_by_desc(cake::Column::Name)
        .all_grouped(&db)
        .await?
        .into_iter()
        .map(|(cake, _)| cake)
        .collect();

    assert_eq!(by_name_desc, [mudcake(), lonely(), cheesecake()]);

    // Children arrive in row order, so the caller's ordering orders the
    // buckets too.
    let children_desc: Vec<(cake::Model, Vec<fruit::Model>)> = cake::Entity::graph()
        .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
        .order_by_asc(cake::Column::Id)
        .order_by_desc(fruit::Column::Id)
        .all_grouped(&db)
        .await?;

    assert_eq!(
        children_desc.first().map(|(_, fruits)| fruits.clone()),
        Some(vec![cranberry(), apricot()])
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:query.graph.grouped+2/test]    a junction-mediated has-many is
// this shape: the `via` hop consumes no slot, so `(Opt<F>,)` still holds and
// the grouped read is available — through `related_maybe` and through the
// hand-written `via` + `join_maybe` alike
#[pgorm_macros::test]
async fn graph_grouped_through_a_junction() -> Result<(), Error> {
    let ctx = TestContext::new("graph_grouped_junction").await;
    let db = ctx.db.get().await?;
    grouped_schema(&db).await?;

    let folded: Vec<(cake::Model, Vec<filling::Model>)> = cake::Entity::graph()
        .related_maybe::<filling::Entity>()
        .order_by_asc(filling::Column::Id)
        .all_grouped(&db)
        .await?;

    assert_eq!(
        folded,
        [
            (cheesecake(), vec![cream(), orphanite()]),
            (mudcake(), vec![orphanite()]),
            (lonely(), vec![]),
        ]
    );

    let by_hand: Vec<(cake::Model, Vec<filling::Model>)> = cake::Entity::graph()
        .via(cake_filling::Relation::Cake.def().rev())
        .join_maybe::<filling::Entity>(cake_filling::Relation::Filling.def())
        .order_by_asc(filling::Column::Id)
        .all_grouped(&db)
        .await?;

    assert_eq!(by_hand, folded);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:query.graph.grouped+2/test]    grouping keys on the decoded
// root rather than on adjacency: an ordering that interleaves the roots merges
// the torn run into the entry at its first appearance instead of emitting the
// root twice
#[pgorm_macros::test]
async fn graph_grouped_merges_a_torn_run() -> Result<(), Error> {
    let ctx = TestContext::new("graph_grouped_torn_run").await;
    let db = ctx.db.get().await?;
    grouped_schema(&db).await?;

    // Ordering on the child's name interleaves the roots: Apricot (cake 1),
    // Blueberry (cake 3), Cranberry (cake 1), then the unmatched cake, whose
    // NULL name sorts last.
    let rows: Vec<(cake::Model, Option<fruit::Model>)> = cake::Entity::graph()
        .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
        .order_by_asc(fruit::Column::Name)
        .all(&db)
        .await?;

    assert_eq!(
        rows,
        [
            (cheesecake(), Some(apricot())),
            (mudcake(), Some(blueberry())),
            (cheesecake(), Some(cranberry())),
            (lonely(), None),
        ],
        "the fixture must actually tear cake 1's run apart"
    );

    let grouped: Vec<(cake::Model, Vec<fruit::Model>)> = cake::Entity::graph()
        .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
        .order_by_asc(fruit::Column::Name)
        .all_grouped(&db)
        .await?;

    assert_eq!(
        grouped,
        [
            // One entry, at the first occurrence, carrying both children.
            (cheesecake(), vec![apricot(), cranberry()]),
            (mudcake(), vec![blueberry()]),
            (lonely(), vec![]),
        ]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:query.graph.grouped+2/test]    the key is read at whatever
// arity it has: a composite-keyed root groups on every key column, not on the
// first one
#[pgorm_macros::test]
async fn graph_grouped_composite_key() -> Result<(), Error> {
    let ctx = TestContext::new("graph_grouped_composite").await;
    let db = ctx.db.get().await?;
    grouped_schema(&db).await?;

    let grouped: Vec<(cake_filling::Model, Vec<filling::Model>)> = cake_filling::Entity::graph()
        .join_maybe::<filling::Entity>(cake_filling::Relation::Filling.def())
        .all_grouped(&db)
        .await?;

    // Three junction rows, two of which share a `cake_id`: keying on the whole
    // key keeps them apart.
    assert_eq!(
        grouped,
        [
            (
                cake_filling::Model {
                    cake_id: 1,
                    filling_id: 5,
                },
                vec![cream()]
            ),
            (
                cake_filling::Model {
                    cake_id: 1,
                    filling_id: 6,
                },
                vec![orphanite()]
            ),
            (
                cake_filling::Model {
                    cake_id: 3,
                    filling_id: 6,
                },
                vec![orphanite()]
            ),
        ]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// The grouped fixture's cakes beside their fruit, one row per match:
/// Cheesecake twice, then the fruitless Lonely, then Mudcake.
fn fruit_rows() -> SelectGraph<cake::Entity, (Opt<fruit::Entity>,)> {
    cake::Entity::graph()
        .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
        .order_by_asc(cake::Column::Id)
        .order_by_asc(fruit::Column::Id)
}

// [spec:pgorm:sem:query.graph.slice/test]    a limit counts rows of the join
// product, not roots: two rows are both Cheesecake's, where the first two
// roots with their fruit would also bring Lonely; an offset starts part-way
// through Cheesecake's rows; and `None` takes a slice back
#[pgorm_macros::test]
async fn graph_slice_counts_rows_not_roots() -> Result<(), Error> {
    let ctx = TestContext::new("graph_slice_rows").await;
    let db = ctx.db.get().await?;
    grouped_schema(&db).await?;

    let first_two = fruit_rows().limit(2).all(&db).await?;
    assert_eq!(
        first_two,
        [
            (cheesecake(), Some(apricot())),
            (cheesecake(), Some(cranberry())),
        ]
    );

    // The root-level answer differs: the first two cakes, every fruit loaded.
    let roots = cake::Entity::find()
        .order_by_asc(cake::Column::Id)
        .limit(2)
        .all(&db)
        .await?;
    let fruit = roots.load_many(fruit::Entity, &db).await?;
    assert_eq!(roots, [cheesecake(), lonely()]);
    assert_eq!(fruit, [vec![apricot(), cranberry()], vec![]]);

    assert_eq!(
        fruit_rows().offset(1).limit(2).all(&db).await?,
        [(cheesecake(), Some(cranberry())), (lonely(), None)]
    );
    assert_eq!(fruit_rows().offset(3).all(&db).await?.len(), 1);
    assert_eq!(fruit_rows().limit(2).limit(None).all(&db).await?.len(), 4);

    // A slotless graph's rows are its roots.
    assert_eq!(
        cake::Entity::graph()
            .order_by_desc(cake::Column::Id)
            .limit(2)
            .all(&db)
            .await?,
        [mudcake(), lonely()]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:query.graph.slice/test]    `one_opt` narrows a slice to one
// row and never widens it — `limit(0)` reads none — and keeps the offset, and
// `stream` yields exactly the rows `all` returns for the same window
#[pgorm_macros::test]
async fn graph_slice_one_opt_and_stream() -> Result<(), Error> {
    let ctx = TestContext::new("graph_slice_terminals").await;
    let db = ctx.db.get().await?;
    grouped_schema(&db).await?;

    assert_eq!(
        fruit_rows().limit(3).one_opt(&db).await?,
        Some((cheesecake(), Some(apricot())))
    );
    assert_eq!(
        fruit_rows().offset(1).one_opt(&db).await?,
        Some((cheesecake(), Some(cranberry())))
    );
    assert_eq!(fruit_rows().limit(0).one_opt(&db).await?, None);
    assert_eq!(fruit_rows().offset(4).one_opt(&db).await?, None);

    let window = fruit_rows().offset(1).limit(2);
    let streamed: Vec<_> = window.clone().stream(&db).await?.try_collect().await?;
    assert_eq!(streamed, window.all(&db).await?);
    assert_eq!(streamed.len(), 2);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:query.graph.slice/test]    the reads a sliced graph lacks
// already own a window: an unsliced graph's page `k` is the slice at offset
// `k * n` limited to `n`, and its cursor's `first(n)` is the slice limited
// to `n` under the same order
#[pgorm_macros::test]
async fn graph_slice_matches_page_and_cursor_windows() -> Result<(), Error> {
    let ctx = TestContext::new("graph_slice_windows").await;
    let db = ctx.db.get().await?;
    grouped_schema(&db).await?;

    let pages = fruit_rows().paginate(&db, page_size(2));
    assert_eq!(
        pages.fetch_page(1).await?,
        fruit_rows().offset(2).limit(2).all(&db).await?
    );

    let first_three = cake::Entity::graph()
        .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
        .cursor_by(cake::Column::Id)
        .first(3)
        .all(&db)
        .await?;
    assert_eq!(first_three, fruit_rows().limit(3).all(&db).await?);

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// The cakes that share a filling with the root: the path runs through the
/// junction twice and reaches `cake` a second time, the far visit excluding
/// the root itself.
fn sibling_cakes() -> SelectGraph<cake::Entity, (Opt<cake::Entity>,)> {
    let shared = alias("shared");
    cake::Entity::graph()
        .via(cake_filling::Relation::Cake.def().rev())
        .via(cake_filling::Relation::Filling.def())
        .via_as(cake_filling::Relation::Filling.def().rev(), shared)
        .join_maybe_as::<cake::Entity>(
            cake_filling::Relation::Cake
                .def()
                .from_alias(shared)
                .on_condition(|_left, right| {
                    Expr::col((right, cake::Column::Id))
                        .ne(Expr::col((cake::Entity, cake::Column::Id)))
                        .into_condition()
                }),
            alias("sibling"),
        )
}

// [spec:pgorm:req:query.graph.aliases+1/test]    a self-referencing path
// through a junction: the second visit to `cake_filling` is a `via_as` hop,
// the edge after it starts from the alias, and the grouped read collects each
// cake's siblings — where the same path with the second visit unaliased is
// refused by the server
#[pgorm_macros::test]
async fn graph_via_as_reaches_cakes_sharing_a_filling() -> Result<(), Error> {
    let ctx = TestContext::new("graph_via_as_siblings").await;
    let db = ctx.db.get().await?;
    grouped_schema(&db).await?;

    // Cheesecake reaches Mudcake through Orphanite, and reaches itself
    // through both of its fillings, which the condition leaves unmatched.
    let rows = sibling_cakes()
        .order_by_asc(cake::Column::Id)
        .order_by_asc(Expr::col((alias("sibling"), cake::Column::Id)))
        .all(&db)
        .await?;
    assert_eq!(
        rows,
        [
            (cheesecake(), Some(mudcake())),
            (cheesecake(), None),
            (cheesecake(), None),
            (lonely(), None),
            (mudcake(), Some(cheesecake())),
            (mudcake(), None),
        ]
    );

    assert_eq!(
        sibling_cakes().all_grouped(&db).await?,
        [
            (cheesecake(), vec![mudcake()]),
            (lonely(), vec![]),
            (mudcake(), vec![cheesecake()]),
        ]
    );

    let unaliased = cake::Entity::graph()
        .via(cake_filling::Relation::Cake.def().rev())
        .via(cake_filling::Relation::Filling.def())
        .via(cake_filling::Relation::Filling.def().rev())
        .all(&db)
        .await
        .expect_err("a table the graph already names, joined unaliased");
    match &unaliased {
        Error::Postgres(e) => assert_eq!(e.code(), Some(&SqlState::DUPLICATE_ALIAS), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// A member table whose one text column's name, under any `s{i}_` prefix,
/// reaches PostgreSQL's 63-byte identifier bound and so takes the bounded
/// spelling, and the junction linking a member to its peers.
mod peers {
    pub mod member {
        use pgorm::entity::prelude::*;

        #[derive(Clone, Debug, PartialEq, Eq, Entity)]
        #[pgorm(table_name = "review_member")]
        pub struct Model {
            #[pgorm(primary_key, auto_increment = false)]
            pub id: i32,
            #[pgorm(column_name = "preferred_display_name_chosen_by_the_member_at_registrations")]
            pub nickname: String,
        }

        #[derive(Copy, Clone, Debug, Relation)]
        pub enum Relation {}
    }

    pub mod member_link {
        use pgorm::entity::prelude::*;

        #[derive(Clone, Debug, PartialEq, Eq, Entity)]
        #[pgorm(table_name = "review_member_link")]
        pub struct Model {
            #[pgorm(primary_key, auto_increment = false)]
            pub member_id: i32,
            #[pgorm(primary_key, auto_increment = false)]
            pub peer_id: i32,
        }

        #[derive(Copy, Clone, Debug, Relation)]
        pub enum Relation {
            #[pgorm(
                belongs_to = super::member::Entity,
                from = Column::MemberId,
                to = super::member::Column::Id
            )]
            Member,
            #[pgorm(
                belongs_to = super::member::Entity,
                from = Column::PeerId,
                to = super::member::Column::Id
            )]
            Peer,
        }
    }

    /// A member to the peers of its peers: the junction twice, and the member
    /// table three times.
    #[derive(Debug)]
    pub struct PeerOfPeer;

    impl pgorm::Linked for PeerOfPeer {
        type FromEntity = member::Entity;
        type ToEntity = member::Entity;

        fn link(&self) -> Vec<pgorm::RelationDef> {
            use pgorm::RelationTrait;
            vec![
                member_link::Relation::Member.def().rev(),
                member_link::Relation::Peer.def(),
                member_link::Relation::Member.def().rev(),
                member_link::Relation::Peer.def(),
            ]
        }
    }
}

/// Hop `i` of a chain starts where hop `i - 1` ended; the first starts at
/// the root.
fn from_previous(hop: pgorm::RelationDef, i: usize) -> pgorm::RelationDef {
    match i.checked_sub(1) {
        Some(previous) => hop.from_alias(LinkedAlias::hop(previous)),
        None => hop,
    }
}

/// The graph a `Linked` chain folds into: its source as the root, its target
/// as the one optional slot.
type LinkedGraph<L> = SelectGraph<<L as Linked>::FromEntity, (Opt<<L as Linked>::ToEntity>,)>;

/// A `Linked` chain as a graph, every hop under its own alias as SeaORM's
/// linked joins had them.
fn linked<L: Linked>(link: L) -> Option<LinkedGraph<L>> {
    let mut hops = link.link();
    let last = hops.pop()?;
    let end = hops.len();
    let mut graph = L::FromEntity::graph();
    for (i, hop) in hops.into_iter().enumerate() {
        graph = graph.via_as(from_previous(hop, i), LinkedAlias::hop(i));
    }
    Some(graph.join_maybe_as::<L::ToEntity>(from_previous(last, end), LinkedAlias::hop(end)))
}

fn member(id: i32, nickname: &str) -> peers::member::Model {
    peers::member::Model {
        id,
        nickname: nickname.to_owned(),
    }
}

// [spec:pgorm:req:query.graph.aliases+1/test]    a `Linked` chain through one
// junction twice folds into `via_as` hops, each starting from the alias
// before it; the member table read as root and as slot projects its long
// column under two bounded prefixes that decode their own values
#[pgorm_macros::test]
async fn graph_linked_chain_through_one_junction_twice() -> Result<(), Error> {
    let ctx = TestContext::new("graph_via_as_linked").await;
    let db = ctx.db.get().await?;
    db.batch_execute(&format!(
        r#"CREATE TABLE review_member (id integer PRIMARY KEY, {nick} text NOT NULL);
           CREATE TABLE review_member_link (
               member_id integer, peer_id integer, PRIMARY KEY (member_id, peer_id)
           );
           INSERT INTO review_member VALUES (1, 'ann'), (2, 'bob'), (3, 'cat'), (4, 'dan');
           INSERT INTO review_member_link VALUES (1, 2), (2, 3), (2, 4), (3, 1);"#,
        nick = "preferred_display_name_chosen_by_the_member_at_registrations",
    ))
    .await?;

    let graph = linked(peers::PeerOfPeer).expect("the chain has a last hop");
    let grouped = graph
        .order_by_asc(peers::member::Column::Id)
        .order_by_asc(Expr::col((LinkedAlias::hop(3), peers::member::Column::Id)))
        .all_grouped(&db)
        .await?;

    assert_eq!(
        grouped,
        [
            (member(1, "ann"), vec![member(3, "cat"), member(4, "dan")]),
            (member(2, "bob"), vec![member(1, "ann")]),
            (member(3, "cat"), vec![member(2, "bob")]),
            (member(4, "dan"), vec![]),
        ]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:query.graph.cursor+4/test]    equal order-column values
// under one root stay reachable: the ordered slot's own key completes the
// boundary, so resuming mid-run yields the sibling instead of skipping it
#[pgorm_macros::test]
async fn slot_cursor_resumes_through_equal_names() -> Result<(), Error> {
    let ctx = TestContext::new("graph_cursor_equal_names").await;
    let db = ctx.db.get().await?;
    schema(&db).await?;
    db.batch_execute(r#"INSERT INTO "fruit" VALUES (12, 'Same', 1), (13, 'Same', 1);"#)
        .await?;

    let cursor = || {
        cake::Entity::graph()
            .join_maybe::<fruit::Entity>(cake::Relation::Fruit.def())
            .cursor_by_on::<1, _>(fruit::Column::Name)
    };

    let resumed = cursor()
        .after_with(("Same", 12, 1))
        .first(1)
        .all(&db)
        .await?;
    assert_eq!(resumed.len(), 1, "the equal-named sibling is reachable");
    assert_eq!(resumed[0].1.as_ref().map(|f| f.id), Some(13));

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// The deterministic collision pair: `mimic`'s SQL name IS `victim`'s
/// bounded spelling minus the prefix, so under the old contract both aliases
/// were the identical 63 bytes.
mod alias_collision {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "review_alias_collision")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        #[pgorm(column_name = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")]
        pub victim: String,
        #[pgorm(column_name = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa08003a0bcb7eb8e2")]
        pub mimic: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

// [spec:pgorm:sem:query.graph.writer+4/test]    a column literally named like
// another column's bounded spelling keeps its own value through the graph and
// select_sources: the plain and bounded alias namespaces cannot meet
#[pgorm_macros::test]
async fn bounded_alias_never_shadows_a_short_column() -> Result<(), Error> {
    let ctx = TestContext::new("graph_alias_collision").await;
    let db = ctx.db.get().await?;
    db.batch_execute(&format!(
        r#"CREATE TABLE review_alias_collision (
               id integer PRIMARY KEY,
               {victim} text NOT NULL,
               {mimic} text NOT NULL
           );
           INSERT INTO review_alias_collision VALUES (1, 'left', 'right');"#,
        victim = "a".repeat(61),
        mimic = "a".repeat(44) + "08003a0bcb7eb8e2",
    ))
    .await?;

    let expected = alias_collision::Model {
        id: 1,
        victim: "left".to_owned(),
        mimic: "right".to_owned(),
    };
    assert_eq!(
        alias_collision::Entity::find().all(&db).await?,
        std::slice::from_ref(&expected)
    );
    assert_eq!(
        alias_collision::Entity::graph().all(&db).await?,
        std::slice::from_ref(&expected)
    );
    assert_eq!(
        pgorm::pipeline::Pipeline::from(alias_collision::Entity)
            .select_sources(alias_collision::Entity)
            .all(&db)
            .await?,
        [Some(expected)]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// The multibyte collision pair: `victim`'s bounded head retreats off a
/// UTF-8 boundary, and `mimic`'s plain composition spelled the retreated
/// alias exactly under the old contract.
mod unicode_collision {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "review_unicode_collision")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        #[pgorm(column_name = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaåbbbbbbbbbbbbbbbb")]
        pub victim: String,
        #[pgorm(column_name = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaad9004b29644599f8")]
        pub mimic: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

// [spec:pgorm:sem:query.graph.writer+4/test]    a multibyte name whose head
// retreats off a UTF-8 boundary still cannot be shadowed: the padded spelling
// stays out of the plain namespace through the graph and select_sources
#[pgorm_macros::test]
async fn unicode_bounded_alias_never_shadows_a_short_column() -> Result<(), Error> {
    let ctx = TestContext::new("graph_unicode_alias_collision").await;
    let db = ctx.db.get().await?;
    db.batch_execute(&format!(
        r#"CREATE TABLE review_unicode_collision (
               id integer PRIMARY KEY,
               "{victim}" text NOT NULL,
               {mimic} text NOT NULL
           );
           INSERT INTO review_unicode_collision VALUES (1, 'left', 'right');"#,
        victim = "a".repeat(43) + "å" + &"b".repeat(16),
        mimic = "a".repeat(43) + "d9004b29644599f8",
    ))
    .await?;

    let expected = unicode_collision::Model {
        id: 1,
        victim: "left".to_owned(),
        mimic: "right".to_owned(),
    };
    assert_eq!(
        unicode_collision::Entity::find().all(&db).await?,
        std::slice::from_ref(&expected)
    );
    assert_eq!(
        unicode_collision::Entity::graph().all(&db).await?,
        std::slice::from_ref(&expected)
    );
    assert_eq!(
        pgorm::pipeline::Pipeline::from(unicode_collision::Entity)
            .select_sources(unicode_collision::Entity)
            .all(&db)
            .await?,
        [Some(expected)]
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

mod long_name {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "review_long_name")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        #[pgorm(column_name = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")]
        pub value: String,
        #[pgorm(column_name = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaab")]
        pub sibling: String,
        #[pgorm(column_name = "åååååååååååååååååååååååååååååå")]
        pub wide: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

// [spec:pgorm:sem:query.graph.writer+4/test]    every valid PostgreSQL column
// identifier projects and decodes through the graph: a 61-byte name whose
// prefixed alias passes the 63-byte bound, a sibling sharing its whole head,
// and a multibyte name — through `find`, the graph, and `select_sources`
// [spec:pgorm:sem:query.graph.decode+1/test]    the decode witness reads the
// same bounded spelling the writer minted
#[pgorm_macros::test]
async fn long_column_names_survive_graph_reads() -> Result<(), Error> {
    let ctx = TestContext::new("graph_long_column_names").await;
    let db = ctx.db.get().await?;
    db.batch_execute(&format!(
        r#"CREATE TABLE review_long_name (
               id integer PRIMARY KEY,
               {a} text NOT NULL,
               {b} text NOT NULL,
               "{w}" text NOT NULL
           );
           INSERT INTO review_long_name VALUES (1, 'value', 'sibling', 'wide');"#,
        a = "a".repeat(61),
        b = "a".repeat(60) + "b",
        w = "å".repeat(30),
    ))
    .await?;

    let ordinary = long_name::Entity::find().all(&db).await?;
    assert_eq!(
        ordinary,
        [long_name::Model {
            id: 1,
            value: "value".to_owned(),
            sibling: "sibling".to_owned(),
            wide: "wide".to_owned(),
        }]
    );

    let graph = long_name::Entity::graph().all(&db).await?;
    assert_eq!(graph, ordinary);

    let sourced = pgorm::pipeline::Pipeline::from(long_name::Entity)
        .select_sources(long_name::Entity)
        .all(&db)
        .await?;
    assert_eq!(sourced, [Some(graph[0].clone())]);

    drop(db);
    ctx.delete().await;
    Ok(())
}
