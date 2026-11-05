#![allow(unused_imports, dead_code)]

//! CTE, LATERAL, WINDOW and UNION composition against a live server.
//!
//! Each of these claims that a statement shaped by one of the composition
//! clauses still decodes through the ORM's own terminals — `one` with its
//! injected `LIMIT 1`, `all`, `stream`, the tuple decode. A renderer cannot
//! prove that; only rows coming back can.
//!
//! Run the test locally:
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:54329 cargo test --test query_composition_tests

pub mod common;

use futures::StreamExt;

use common::bakery_chain::{Bakery, Cake, bakery, cake, create_tables};
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    CommonTableExpression, Condition, Expr, ExtendedWindow, FrameType, Func, Order,
    OrderedStatement, Query, RecursiveWithClause, SelectStatement, SimpleExpr, UnionType,
    WindowStatement, WithClause, error::Error as QueryError,
};
use pgorm::{
    Error, SelectGetableTuple, SelectModel, Selector, SelectorRaw, alias, entity::prelude::*,
};
use pretty_assertions::assert_eq;
use tokio_postgres::error::SqlState;

async fn seed(db: &impl ConnectionTrait) -> Result<(i32, i32), Error> {
    let alpha = bakery::ActiveModel {
        name: set("Alpha"),
        profit_margin: set(10.0),
        ..Default::default()
    }
    .insert(db)
    .await?;
    let beta = bakery::ActiveModel {
        name: set("Beta"),
        profit_margin: set(20.0),
        ..Default::default()
    }
    .insert(db)
    .await?;

    let prices = [
        (alpha.id, "Alpha Bun", 5),
        (alpha.id, "Alpha Tart", 15),
        (alpha.id, "Alpha Torte", 25),
        (beta.id, "Beta Bun", 8),
        (beta.id, "Beta Slice", 18),
        (beta.id, "Beta Gateau", 35),
    ];
    for (bakery_id, name, price) in prices {
        cake::ActiveModel {
            name: set(name),
            price: set(rust_dec(price)),
            gluten_free: set(false),
            serial: set(Uuid::new_v4()),
            bakery_id: set(Some(bakery_id)),
            ..Default::default()
        }
        .insert(db)
        .await?;
    }

    Ok((alpha.id, beta.id))
}

/// `WITH RECURSIVE bracket (lo) AS (anchor UNION ALL step)`, counting the cakes
/// that fall in each ten-unit price band.
///
/// The anchor's literal carries an explicit cast: a bare `$n` there would settle
/// the CTE's column type as `text` and the step arm's `lo + 10` would have no
/// operator to resolve.
fn price_brackets() -> RecursiveWithClause {
    let bracket = alias("bracket");
    let lo_col = alias("lo");
    let lo = || Expr::col((bracket, lo_col));

    let mut anchor = Query::select()
        .expr_as(Expr::val(0i32).cast_as(alias("integer")), lo_col)
        .to_owned();

    let step = Query::select()
        .expr_as(lo().add(10i32), lo_col)
        .from(bracket)
        .and_where(lo().lt(30i32))
        .to_owned();

    RecursiveWithClause::new(
        CommonTableExpression::new(bracket, anchor.union(UnionType::All, step).to_owned())
            .columns([lo_col])
            .to_owned(),
    )
}

/// The band-count query whose driving table is the CTE, so no entity is behind
/// it and `Selector::from_select` is the only typed way in.
fn bracket_counts() -> SelectStatement {
    let bracket = alias("bracket");
    let lo_col = alias("lo");
    let lo = || Expr::col((bracket, lo_col));

    Query::select()
        .expr_as(lo(), lo_col)
        .expr_as(
            Expr::col((cake::Entity, cake::Column::Id)).count(),
            alias("cakes"),
        )
        .from(bracket)
        .join(
            JoinType::LeftJoin,
            cake::Entity,
            Condition::all()
                .add(Expr::col((cake::Entity, cake::Column::Price)).gte(lo()))
                .add(Expr::col((cake::Entity, cake::Column::Price)).lt(lo().add(10i32))),
        )
        .add_group_by([lo().into()])
        .to_owned()
}

#[derive(Debug, PartialEq, Eq, FromQueryResult)]
struct PriceBracket {
    lo: i32,
    cakes: i64,
}

// [spec:pgorm:def:query.build.with+2/test]    a recursive CTE drives the query and the rows land in a
// `FromQueryResult` struct through every terminal the ORM owns
// [spec:pgorm:sem:query.build.with.attach+1/test]
// [spec:pgorm:sem:exec.crud.selector-entry+2/test]
// [spec:pgorm:sem:sql.render.placeholder-typing+2/test]    the anchor's cast is what makes the
// recursion typecheck at all
#[pgorm_macros::test]
pub async fn recursive_cte_decodes_all_one_and_stream() -> Result<(), Error> {
    let ctx = TestContext::new("recursive_cte_decodes_all_one_and_stream").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;
    seed(&db).await?;

    let bracket = alias("bracket");
    let lo = alias("lo");
    let ascending = || {
        bracket_counts()
            .order_by((bracket, lo), Order::Asc)
            .to_owned()
            .with(price_brackets())
            .to_owned()
    };

    let all = Selector::<SelectModel<PriceBracket>>::from_select::<PriceBracket>(ascending())
        .all(&db)
        .await?;
    assert_eq!(
        all,
        [
            PriceBracket { lo: 0, cakes: 2 },
            PriceBracket { lo: 10, cakes: 2 },
            PriceBracket { lo: 20, cakes: 1 },
            PriceBracket { lo: 30, cakes: 1 },
        ],
        "four bands, each counting the cakes priced into it"
    );

    // `one` pushes its LIMIT onto the carrying select, not into the CTE, so a
    // multi-row query yields its first row instead of a protocol error.
    let fullest = Selector::<SelectModel<PriceBracket>>::from_select::<PriceBracket>(
        bracket_counts()
            .order_by_expr(
                Expr::col((cake::Entity, cake::Column::Id)).count(),
                Order::Desc,
            )
            .order_by((bracket, lo), Order::Asc)
            .to_owned()
            .with(price_brackets())
            .to_owned(),
    )
    .one(&db)
    .await?;
    assert_eq!(fullest, PriceBracket { lo: 0, cakes: 2 });

    let mut stream =
        Selector::<SelectModel<PriceBracket>>::from_select::<PriceBracket>(ascending())
            .stream(&db)
            .await?;
    let mut streamed = Vec::new();
    while let Some(row) = stream.next().await {
        streamed.push(row?);
    }
    assert_eq!(streamed, all, "the stream decodes what `all` collected");
    drop(stream);

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:sql.render.placeholder-typing+2/test]    the caller obligation has teeth: an
// unannotated anchor placeholder resolves to `text` and the server refuses the recursion
#[pgorm_macros::test]
pub async fn recursive_cte_anchor_needs_a_cast() -> Result<(), Error> {
    let ctx = TestContext::new("recursive_cte_anchor_needs_a_cast").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;

    let bracket = alias("bracket");
    let lo_col = alias("lo");
    let lo = || Expr::col((bracket, lo_col));

    let mut anchor = Query::select().expr_as(Expr::val(0i32), lo_col).to_owned();
    let step = Query::select()
        .expr_as(lo().add(10i32), lo_col)
        .from(bracket)
        .and_where(lo().lt(30i32))
        .to_owned();

    let uncast = Query::select()
        .expr_as(lo(), lo_col)
        .from(bracket)
        .to_owned()
        .with(RecursiveWithClause::new(
            CommonTableExpression::new(bracket, anchor.union(UnionType::All, step).to_owned())
                .columns([lo_col])
                .to_owned(),
        ))
        .to_owned();

    let refused = Selector::<SelectGetableTuple<(i32,)>>::into_tuple::<(i32,)>(uncast)
        .all(&db)
        .await
        .expect_err("an untyped anchor placeholder cannot carry an integer band");
    let message = refused.to_string();
    assert!(
        message.contains("operator does not exist: text +"),
        "the untyped anchor settles the CTE column as text: {message}"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:query.build.with.attach+1/test]    a CTE joined into an entity's own query leaves
// the builder a `Select<E>`, so filters, ordering and the `E::Model` terminals all still apply
// [spec:pgorm:sem:query.build.lateral/test]
#[pgorm_macros::test]
pub async fn cte_joins_into_entity_find() -> Result<(), Error> {
    let ctx = TestContext::new("cte_joins_into_entity_find").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;
    let (_alpha, beta) = seed(&db).await?;

    let best = alias("best");
    let bakery_id = alias("bakery_id");
    let top = alias("top");
    let cte = CommonTableExpression::new(
        best,
        Query::select()
            .column(cake::Column::BakeryId)
            .expr_as(Expr::col((cake::Entity, cake::Column::Price)).max(), top)
            .from(cake::Entity)
            .group_by_col(cake::Column::BakeryId)
            .to_owned(),
    )
    .columns([bakery_id, top])
    .to_owned();

    // A bare projected `$n` is untyped for the same reason a recursive anchor's
    // is, so the marker column is a constant rather than a bound value.
    let dearest = Query::select()
        .expr(SimpleExpr::Constant(1i32.into()))
        .from(best)
        .and_where(Expr::col((best, bakery_id)).equals((bakery::Entity, bakery::Column::Id)))
        .and_where(Expr::col((best, top)).gt(rust_dec(30)))
        .to_owned();

    let bakeries = Bakery::find()
        .with(WithClause::new(cte))
        .join_lateral_on_true(JoinType::InnerJoin, dearest, alias("m"))
        .filter(bakery::Column::Name.like("B%"))
        .order_by_asc(bakery::Column::Id)
        .all(&db)
        .await?;

    assert_eq!(
        bakeries.iter().map(|b| b.id).collect::<Vec<_>>(),
        [beta],
        "only the bakery whose dearest cake clears 30 survives both the CTE and the filter"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

/// `bakery CROSS JOIN LATERAL (SELECT … FROM cake WHERE cake.bakery_id =
/// bakery.id ORDER BY price DESC LIMIT 2) top ON TRUE`.
fn top_two_per_bakery() -> SelectStatement {
    let top = alias("top");
    let per_bakery = Query::select()
        .column(cake::Column::Name)
        .from(cake::Entity)
        .and_where(
            Expr::col((cake::Entity, cake::Column::BakeryId))
                .equals((bakery::Entity, bakery::Column::Id)),
        )
        .order_by(cake::Column::Price, Order::Desc)
        .limit(2)
        .to_owned();

    Bakery::find()
        .select_only()
        .column_as(bakery::Column::Name, Name::runtime("bakery"))
        .expr_as(Expr::col((top, cake::Column::Name)), Name::runtime("cake"))
        .join_lateral_on_true(JoinType::InnerJoin, per_bakery, top)
        .order_by_asc(bakery::Column::Id)
        .order_by(
            SimpleExpr::from(Expr::col((top, cake::Column::Name))),
            Order::Asc,
        )
        .into_query()
}

#[derive(Debug, PartialEq, Eq, FromQueryResult)]
struct TopCake {
    bakery: String,
    cake: String,
}

// [spec:pgorm:sem:query.build.lateral/test]    a LATERAL top-N-per-group is an in-place mutation of
// the statement, so a named decode target still works
#[pgorm_macros::test]
pub async fn lateral_top_n_decodes_into_struct() -> Result<(), Error> {
    let ctx = TestContext::new("lateral_top_n_decodes_into_struct").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;
    seed(&db).await?;

    let rows = Selector::<SelectModel<TopCake>>::from_select::<TopCake>(top_two_per_bakery())
        .all(&db)
        .await?;

    assert_eq!(
        rows,
        [
            TopCake {
                bakery: "Alpha".to_owned(),
                cake: "Alpha Tart".to_owned()
            },
            TopCake {
                bakery: "Alpha".to_owned(),
                cake: "Alpha Torte".to_owned()
            },
            TopCake {
                bakery: "Beta".to_owned(),
                cake: "Beta Gateau".to_owned()
            },
            TopCake {
                bakery: "Beta".to_owned(),
                cake: "Beta Slice".to_owned()
            },
        ],
        "the two dearest cakes of each bakery"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:exec.crud.selector-entry+2/test]    the very same statement decodes ordinally
// through `Selector::into_tuple`, and its raw counterpart through `SelectorRaw::into_tuple`
#[pgorm_macros::test]
pub async fn lateral_top_n_decodes_into_tuple() -> Result<(), Error> {
    let ctx = TestContext::new("lateral_top_n_decodes_into_tuple").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;
    seed(&db).await?;

    let tuples: Vec<(String, String)> =
        Selector::<SelectGetableTuple<(String, String)>>::into_tuple::<(String, String)>(
            top_two_per_bakery(),
        )
        .all(&db)
        .await?;
    assert_eq!(tuples.len(), 4);
    assert_eq!(
        tuples[0],
        ("Alpha".to_owned(), "Alpha Tart".to_owned()),
        "column order, not column name, is what an ordinal decode reads"
    );

    let (sql, values) = top_two_per_bakery().build();
    let raw: Vec<(String, String)> =
        SelectorRaw::<SelectGetableTuple<(String, String)>>::into_tuple::<(String, String)>(
            sql, values,
        )
        .all(&db)
        .await?;
    assert_eq!(raw, tuples, "the raw entry point decodes the same rows");

    drop(db);
    ctx.delete().await;
    Ok(())
}

#[derive(Debug, PartialEq, Eq, FromQueryResult)]
struct RankedCake {
    bakery_id: i32,
    name: String,
    rank: i64,
}

// [spec:pgorm:sem:query.build.window+2/test]    a named window declaration and the projection that
// refers to it, both reachable as ORM combinators
#[pgorm_macros::test]
pub async fn window_clause_ranks_rows_per_bakery() -> Result<(), Error> {
    let ctx = TestContext::new("window_clause_ranks_rows_per_bakery").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;
    seed(&db).await?;

    let w = alias("w");
    let ranked = Cake::find()
        .select([cake::Column::BakeryId, cake::Column::Name])
        .window_expr_as(Func::named(alias("row_number")), w, alias("rank"))
        .window(
            w,
            WindowStatement::partition_by(cake::Column::BakeryId)
                .order_by(cake::Column::Price, Order::Desc)
                .to_owned(),
        )
        .order_by_asc(cake::Column::BakeryId)
        .order_by_desc(cake::Column::Price)
        .into_model::<RankedCake>()
        .all(&db)
        .await?;

    assert_eq!(ranked.len(), 6);
    assert_eq!(
        ranked.iter().map(|c| c.rank).collect::<Vec<_>>(),
        [1, 2, 3, 1, 2, 3],
        "the rank restarts with each partition"
    );
    assert_eq!(ranked[0].name, "Alpha Torte");

    drop(db);
    ctx.delete().await;
    Ok(())
}

#[derive(Debug, PartialEq, Eq, FromQueryResult)]
struct PricedCake {
    name: String,
    rank: i64,
    cakes: i64,
}

// [spec:pgorm:sem:query.build.window+2/test]    `window` appends to the WINDOW clause, a window
// extending an earlier entry among them, and a clause PostgreSQL refuses is refused unsent
#[pgorm_macros::test]
pub async fn window_clause_holds_every_named_window() -> Result<(), Error> {
    let ctx = TestContext::new("window_clause_holds_every_named_window").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;
    seed(&db).await?;

    let (per_bakery, by_price) = (alias("per_bakery"), alias("by_price"));
    let priced = || {
        Cake::find()
            .select([cake::Column::Name])
            .window_expr_as(Func::named(alias("rank")), by_price, alias("rank"))
            .window_expr_as(
                Func::count(Expr::col(cake::Column::Id)),
                per_bakery,
                alias("cakes"),
            )
            .window(
                per_bakery,
                WindowStatement::partition_by(cake::Column::BakeryId),
            )
    };
    let ranked = priced()
        .window(
            by_price,
            ExtendedWindow::new(per_bakery)
                .order_by(cake::Column::Price, Order::Desc)
                .to_owned(),
        )
        .order_by_asc(cake::Column::BakeryId)
        .order_by_desc(cake::Column::Price)
        .into_model::<PricedCake>()
        .all(&db)
        .await?;
    assert_eq!(
        ranked
            .iter()
            .map(|cake| (cake.name.as_str(), cake.rank, cake.cakes))
            .collect::<Vec<_>>(),
        [
            ("Alpha Torte", 1, 3),
            ("Alpha Tart", 2, 3),
            ("Alpha Bun", 3, 3),
            ("Beta Gateau", 1, 3),
            ("Beta Slice", 2, 3),
            ("Beta Bun", 3, 3),
        ],
        "each window is read: the rank by the extended window, the count by its base"
    );

    let refused = priced()
        .window(per_bakery, WindowStatement::new())
        .into_model::<PricedCake>()
        .all(&db)
        .await
        .expect_err("two windows of one name");
    assert!(
        matches!(
            &refused,
            Error::QueryBuilder(QueryError::WindowRedefined { name }) if name == "per_bakery"
        ),
        "{refused:?}"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:def:sql.ast.window-statement+8/test]    against a live server: an inline window built
// on a named one copies its partition, and each inline refusal `check_windows` makes is one the
// server makes, with the code it names
#[pgorm_macros::test]
pub async fn inline_window_builds_on_a_named_one() -> Result<(), Error> {
    let ctx = TestContext::new("inline_window_builds_on_a_named_one").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;
    seed(&db).await?;

    let per_bakery = alias("per_bakery");
    let by_price = || {
        ExtendedWindow::new(per_bakery)
            .order_by(cake::Column::Price, Order::Desc)
            .to_owned()
    };
    let mut priced = Cake::find().select([cake::Column::Name]);
    QuerySelect::query(&mut priced)
        .expr_window_as(Func::named(alias("rank")), by_price(), alias("rank"))
        .expr_window_name_as(
            Func::count(Expr::col(cake::Column::Id)),
            per_bakery,
            alias("cakes"),
        );
    let ranked = priced
        .window(
            per_bakery,
            WindowStatement::partition_by(cake::Column::BakeryId),
        )
        .order_by_asc(cake::Column::BakeryId)
        .order_by_desc(cake::Column::Price)
        .into_model::<PricedCake>()
        .all(&db)
        .await?;
    assert_eq!(
        ranked
            .iter()
            .map(|cake| (cake.name.as_str(), cake.rank, cake.cakes))
            .collect::<Vec<_>>(),
        [
            ("Alpha Torte", 1, 3),
            ("Alpha Tart", 2, 3),
            ("Alpha Bun", 3, 3),
            ("Beta Gateau", 1, 3),
            ("Beta Slice", 2, 3),
            ("Beta Bun", 3, 3),
        ],
        "the inline window ranks within the partition it copies"
    );

    let counted = |inline: ExtendedWindow, base: WindowStatement| {
        Query::select()
            .expr_window(Func::count(Expr::col(cake::Column::Id)), inline)
            .from(cake::Entity)
            .window(per_bakery, base)
            .to_owned()
    };
    let ordered = || {
        WindowStatement::new()
            .order_by(cake::Column::Id, Order::Asc)
            .to_owned()
    };
    let refusals = [
        (
            counted(ExtendedWindow::new(alias("nowhere")), ordered()),
            QueryError::WindowBaseUndefined {
                window: None,
                base: "nowhere".to_owned(),
            },
            SqlState::UNDEFINED_OBJECT,
        ),
        (
            counted(
                ExtendedWindow::new(per_bakery),
                ordered().frame(FrameType::Rows.current_row()).to_owned(),
            ),
            QueryError::WindowFrameCopied {
                window: None,
                base: "per_bakery".to_owned(),
            },
            SqlState::WINDOWING_ERROR,
        ),
        (
            counted(by_price(), ordered()),
            QueryError::WindowOrderOverridden {
                window: None,
                base: "per_bakery".to_owned(),
            },
            SqlState::WINDOWING_ERROR,
        ),
    ];
    for (query, refusal, state) in refusals {
        assert_eq!(query.check_windows(), Err(refusal));
        let sent = db
            .query_all(query.to_string().as_str(), &[])
            .await
            .expect_err("the server refuses it too");
        match &sent {
            Error::Postgres(error) => assert_eq!(error.code(), Some(&state), "{error}"),
            other => panic!("expected the server's refusal, got {other:?}"),
        }
    }

    let mut unsent = Cake::find().select([cake::Column::Name]);
    QuerySelect::query(&mut unsent).expr_window(Func::named(alias("rank")), by_price());
    let refused = unsent
        .into_model::<RankedCake>()
        .all(&db)
        .await
        .expect_err("an inline window over an undeclared base");
    assert!(
        matches!(
            &refused,
            Error::QueryBuilder(QueryError::WindowBaseUndefined { window: None, base })
                if base == "per_bakery"
        ),
        "{refused:?}"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:query.build.union/test]    both arms are the same builder type, so the combined
// result still decodes as `E::Model`
#[pgorm_macros::test]
pub async fn union_and_intersect_of_typed_selects() -> Result<(), Error> {
    let ctx = TestContext::new("union_and_intersect_of_typed_selects").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;
    seed(&db).await?;

    let cheap = Cake::find().filter(cake::Column::Price.lt(rust_dec(10)));
    let dear = Cake::find().filter(cake::Column::Price.gt(rust_dec(30)));
    let extremes = cheap.union(UnionType::All, dear).all(&db).await?;
    assert_eq!(
        sorted_names(&extremes),
        ["Alpha Bun", "Beta Bun", "Beta Gateau"],
        "UNION ALL keeps every row of both arms"
    );

    let over_ten = Cake::find().filter(cake::Column::Price.gt(rust_dec(10)));
    let under_thirty = Cake::find().filter(cake::Column::Price.lt(rust_dec(30)));
    let middle = over_ten
        .union(UnionType::Intersect, under_thirty)
        .all(&db)
        .await?;
    assert_eq!(
        sorted_names(&middle),
        ["Alpha Tart", "Alpha Torte", "Beta Slice"],
        "INTERSECT rides the same combinator"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}

fn sorted_names(cakes: &[cake::Model]) -> Vec<String> {
    let mut names: Vec<String> = cakes.iter().map(|c| c.name.clone()).collect();
    names.sort();
    names
}

// [spec:pgorm:sem:exec.crud.selector-entry+2/test]    the new constructor is a `Selector`, so it
// inherits the empty-projection guard rather than sending `SELECT  FROM …`
// [spec:pgorm:sem:query.build.modifiers+11/test]
#[pgorm_macros::test]
pub async fn from_select_guards_an_empty_projection() -> Result<(), Error> {
    let ctx = TestContext::new("from_select_guards_an_empty_projection").await;
    create_tables(&ctx.db).await?;
    let db = ctx.db.get().await?;

    let empty = Query::select().from(cake::Entity).to_owned();
    let refused = Selector::<SelectModel<cake::Model>>::from_select::<cake::Model>(empty)
        .all(&db)
        .await
        .expect_err("a statement with no projection never reaches the server");
    assert_eq!(
        refused.to_string(),
        "Query Error: select list is empty; add at least one column or expression"
    );

    drop(db);
    ctx.delete().await;
    Ok(())
}
