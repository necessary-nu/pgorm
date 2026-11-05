#![allow(unused_imports, dead_code)]

//! Window frames with expression offsets and an `EXCLUDE` clause, against a
//! live PostgreSQL server.
//!
//! The render tests hold every frame to libpg_query's `WindowDef`, which
//! settles that it parses as the frame it was written as. What they cannot
//! settle is what the frame then *means*, and both halves of this work are
//! ones whose meaning only the server decides. A `RANGE` offset is a distance
//! in the ordering column's own values, so its type is the one PostgreSQL
//! pairs with that column's — an `interval` over a timestamp, a `numeric` over
//! a numeric — and the same offset under `ROWS` counts rows instead. An
//! `EXCLUDE` clause removes rows relative to the current row's *peers*, so it
//! only changes an answer where the ordering ties. Each case below is
//! therefore read over rows chosen so that the construct changes the answer,
//! beside a control that would give the other one.
//!
//! The weights are powers of two, so a `SUM(weight)` names exactly the rows
//! its frame held.

pub mod common;
pub use common::{TestContext, setup::*};
use pgorm::pgorm_query::{
    Expr, FrameClause, FrameExclusion, FrameType, Func, Name, Order, OrderedStatement, Query,
    SelectStatement, SimpleExpr, WindowStatement,
};
use pgorm::{ConnectionTrait, SelectGetableTuple, SelectorRaw, entity::prelude::*};
use rust_decimal::Decimal;
use tokio_postgres::error::SqlState;

/// Four readings: timestamps 12h, 18h and 42h apart, levels 0.4, 0.6 and 1.5
/// apart, and a grade on which the middle two tie.
const SEED: &str = "
    CREATE TABLE reading (
        id int primary key,
        at timestamp not null,
        level numeric not null,
        grade int not null,
        weight int not null
    );
    INSERT INTO reading (id, at, level, grade, weight) VALUES
        (1, TIMESTAMP '2026-01-01 00:00', 1.0, 1, 1),
        (2, TIMESTAMP '2026-01-01 12:00', 1.4, 2, 2),
        (3, TIMESTAMP '2026-01-02 06:00', 2.0, 2, 4),
        (4, TIMESTAMP '2026-01-04 00:00', 3.5, 3, 8);
";

#[pgorm_macros::test]
async fn main() -> Result<(), Error> {
    let ctx = TestContext::new("window_frame_tests").await;
    let db = ctx.db.get().await?;
    db.batch_execute(SEED).await?;

    an_interval_offset_measures_time(&db).await?;
    a_numeric_offset_measures_value_distance(&db).await?;
    each_exclusion_removes_its_own_rows(&db).await?;
    a_groups_frame_takes_a_bound_offset(&db).await?;
    an_offset_of_the_wrong_type_is_refused(&db).await?;
    a_range_offset_needs_one_ordering_column(&db).await?;

    drop(db);
    ctx.delete().await;

    Ok(())
}

fn col(name: &'static str) -> Expr {
    Expr::col(Name::runtime(name))
}

/// `SELECT SUM(weight) OVER (ORDER BY <by> <frame>) FROM reading ORDER BY id`.
fn framed(by: &'static str, frame: impl Into<FrameClause>) -> SelectStatement {
    Query::select()
        .expr_window(
            Func::sum(col("weight")),
            WindowStatement::new()
                .order_by(Name::runtime(by), Order::Asc)
                .frame(frame)
                .take(),
        )
        .from(Name::runtime("reading"))
        .order_by(Name::runtime("id"), Order::Asc)
        .take()
}

/// The window sums of [`framed`], one per reading in id order, inlined.
async fn sums(db: &DatabaseConnection, query: &SelectStatement) -> Result<Vec<Option<i64>>, Error> {
    let rows = db.query_all(query.to_string().as_str(), &[]).await?;
    Ok(rows.iter().map(|row| row.get(0)).collect())
}

/// The same sums through `build()`, every offset a bound `$N`.
async fn bound_sums(
    db: &DatabaseConnection,
    query: &SelectStatement,
) -> Result<Vec<Option<i64>>, Error> {
    let (sql, values) = query.build();
    let rows: Vec<(Option<i64>,)> =
        SelectorRaw::<SelectGetableTuple<(Option<i64>,)>>::into_tuple(&sql, values)
            .all(db)
            .await?;
    Ok(rows.into_iter().map(|(sum,)| sum).collect())
}

fn one_day() -> SimpleExpr {
    Expr::val("1 day").cast_as(Name::runtime("interval"))
}

/// `RANGE BETWEEN '1 day' PRECEDING AND CURRENT ROW` over timestamps reaches
/// back one day of *time*: the fourth reading, 42 hours after the third, sees
/// only itself, where `ROWS 1 PRECEDING` would reach the third regardless.
/// The offset is an expression — a cast to `interval` — which the `u32`
/// offsets could not spell, and it answers the same bound as inlined.
// [spec:pgorm:def:sql.ast.window-statement+8/test]    against a live server: a RANGE offset is
// a distance in the ordering column's values, an interval over a timestamp
// [spec:pgorm:req:sql.render.window+7/test]    the offset renders through the expression path,
// bound and inlined alike
// [spec:pgorm:req:sql.scope+23/test]
async fn an_interval_offset_measures_time(db: &DatabaseConnection) -> Result<(), Error> {
    let by_time = framed(
        "at",
        FrameType::Range.preceding(one_day()).and_current_row(),
    );
    let by_rows = framed("at", FrameType::Rows.preceding(1).and_current_row());

    assert_eq!(
        sums(db, &by_time).await?,
        [Some(1), Some(1 + 2), Some(2 + 4), Some(8)],
        "each reading plus those within the day before it"
    );
    assert_eq!(bound_sums(db, &by_time).await?, sums(db, &by_time).await?);
    assert_eq!(
        sums(db, &by_rows).await?,
        [Some(1), Some(1 + 2), Some(2 + 4), Some(4 + 8)],
        "the control: one row back, however long ago"
    );

    // An uncast string is the caller's obligation of
    // `sql.render.placeholder-typing`: inlined, `'1 day'` is an untyped
    // literal the server reads as the interval the column pairs with; bound,
    // the server asks for an interval and a text value cannot be written as one.
    let untyped = framed("at", FrameType::Range.preceding("1 day").and_current_row());
    assert_eq!(sums(db, &untyped).await?, sums(db, &by_time).await?);
    let refused = bound_sums(db, &untyped)
        .await
        .expect_err("a text value bound where the server typed an interval");
    assert!(refused.to_string().contains("interval"), "{refused}");

    Ok(())
}

/// A numeric offset over a numeric column: `RANGE BETWEEN 0.5 PRECEDING AND
/// 0.5 FOLLOWING` holds each level and those within half a unit of it, so the
/// two close readings see each other and the two far ones see only themselves.
// [spec:pgorm:def:sql.ast.window-statement+8/test]    against a live server: a numeric offset
// over a numeric ordering column
async fn a_numeric_offset_measures_value_distance(db: &DatabaseConnection) -> Result<(), Error> {
    let half = Decimal::new(5, 1);
    let near = framed(
        "level",
        FrameType::Range.preceding(half).and_following(half),
    );

    assert!(
        near.to_string()
            .contains("RANGE BETWEEN 0.5 PRECEDING AND 0.5 FOLLOWING"),
        "{near}"
    );
    assert_eq!(
        sums(db, &near).await?,
        [Some(1 + 2), Some(1 + 2), Some(4), Some(8)]
    );
    assert_eq!(bound_sums(db, &near).await?, sums(db, &near).await?);

    Ok(())
}

/// Each `EXCLUDE` option over the whole partition, ordered by a grade on which
/// readings 2 and 3 tie. `CURRENT ROW` drops the row, `GROUP` the row and its
/// peer, `TIES` the peer but not the row, and `NO OTHERS` nothing — the same
/// answer as no clause at all. Four options, four different columns of sums.
// [spec:pgorm:def:sql.ast.window-statement+8/test]    against a live server: each exclusion
// removes the rows it names relative to the current row's peers
// [spec:pgorm:req:sql.render.window+7/test]    ` EXCLUDE …` renders after the frame's bounds
// [spec:pgorm:req:sql.scope+23/test]
async fn each_exclusion_removes_its_own_rows(db: &DatabaseConnection) -> Result<(), Error> {
    let whole = || {
        FrameType::Rows
            .unbounded_preceding()
            .and_unbounded_following()
    };
    let excluding = |exclusion| framed("grade", whole().exclude(exclusion));

    let everything = sums(db, &framed("grade", whole())).await?;
    assert_eq!(everything, [Some(15); 4]);
    assert_eq!(
        sums(db, &excluding(FrameExclusion::NoOthers)).await?,
        everything,
        "NO OTHERS is the default spelled out"
    );
    assert_eq!(
        sums(db, &excluding(FrameExclusion::CurrentRow)).await?,
        [Some(15 - 1), Some(15 - 2), Some(15 - 4), Some(15 - 8)],
        "each row without itself"
    );
    assert_eq!(
        sums(db, &excluding(FrameExclusion::Group)).await?,
        [
            Some(15 - 1),
            Some(15 - 2 - 4),
            Some(15 - 2 - 4),
            Some(15 - 8)
        ],
        "the tied pair loses both of its rows"
    );
    assert_eq!(
        sums(db, &excluding(FrameExclusion::Ties)).await?,
        [Some(15), Some(15 - 4), Some(15 - 2), Some(15)],
        "each of the tied pair loses the other and keeps itself"
    );

    Ok(())
}

/// A `GROUPS` offset counts peer groups, bound as a `$N` the server types
/// `bigint`. One group back from either tied reading reaches the grade-1 row
/// and both of the pair; with `EXCLUDE CURRENT ROW` the pair each lose only
/// themselves, and the first reading's frame is empty, so its sum is NULL.
// [spec:pgorm:def:sql.ast.window-statement+8/test]    against a live server: GROUPS takes the
// widened offsets, bound or inlined, and an exclusion
async fn a_groups_frame_takes_a_bound_offset(db: &DatabaseConnection) -> Result<(), Error> {
    let back_one = framed("grade", FrameType::Groups.preceding(1).and_current_row());
    let (sql, _) = back_one.build();
    assert!(
        sql.contains("GROUPS BETWEEN $1 PRECEDING AND CURRENT ROW"),
        "{sql}"
    );

    assert_eq!(
        bound_sums(db, &back_one).await?,
        [Some(1), Some(1 + 2 + 4), Some(1 + 2 + 4), Some(2 + 4 + 8)]
    );
    assert_eq!(bound_sums(db, &back_one).await?, sums(db, &back_one).await?);

    let without_self = framed(
        "grade",
        FrameType::Groups
            .preceding(1)
            .and_current_row()
            .exclude(FrameExclusion::CurrentRow),
    );
    assert_eq!(
        bound_sums(db, &without_self).await?,
        [None, Some(1 + 4), Some(1 + 2), Some(2 + 4)]
    );

    Ok(())
}

fn refused_with(error: &Error, state: &SqlState) {
    match error {
        Error::Postgres(e) => assert_eq!(e.code(), Some(state), "{e}"),
        other => panic!("expected Error::Postgres, got {other:?}"),
    }
}

/// The offset's type is checked against the ordering column's, and the
/// builder cannot know that column's type, so these are the server's
/// refusals: an integer offset under `RANGE` over a timestamp has no distance
/// to mean (`0A000`), and an interval under `ROWS` is no count (`42804`).
// [spec:pgorm:def:sql.ast.window-statement+8/test]    against a live server: an offset type the
// ordering column's type does not pair with is refused
async fn an_offset_of_the_wrong_type_is_refused(db: &DatabaseConnection) -> Result<(), Error> {
    let integer_over_time = framed("at", FrameType::Range.preceding(1).and_current_row());
    let error = sums(db, &integer_over_time)
        .await
        .expect_err("an integer is no distance between timestamps");
    refused_with(&error, &SqlState::FEATURE_NOT_SUPPORTED);

    let interval_as_count = framed("at", FrameType::Rows.preceding(one_day()).and_current_row());
    let error = sums(db, &interval_as_count)
        .await
        .expect_err("an interval is no row count");
    refused_with(&error, &SqlState::DATATYPE_MISMATCH);

    Ok(())
}

/// A `RANGE` offset is a distance along one ordering column, so a window that
/// orders by two columns, or by none, gives it nothing to measure (`42P20`).
/// The unbounded `RANGE` frame measures nothing and is the control: it is
/// admitted over the same two-column ordering.
// [spec:pgorm:def:sql.ast.window-statement+8/test]    against a live server: a RANGE offset
// needs exactly one ORDER BY column
async fn a_range_offset_needs_one_ordering_column(db: &DatabaseConnection) -> Result<(), Error> {
    let over = |window: &mut WindowStatement| {
        Query::select()
            .expr_window(Func::sum(col("weight")), window.take())
            .from(Name::runtime("reading"))
            .to_string()
    };

    let two_columns = over(
        WindowStatement::new()
            .order_by(Name::runtime("level"), Order::Asc)
            .order_by(Name::runtime("id"), Order::Asc)
            .frame(FrameType::Range.preceding(1).and_current_row()),
    );
    let error = db
        .query_all(two_columns.as_str(), &[])
        .await
        .expect_err("two ordering columns");
    refused_with(&error, &SqlState::WINDOWING_ERROR);

    let unordered =
        over(WindowStatement::new().frame(FrameType::Range.preceding(1).and_current_row()));
    let error = db
        .query_all(unordered.as_str(), &[])
        .await
        .expect_err("no ordering column");
    refused_with(&error, &SqlState::WINDOWING_ERROR);

    let unbounded = over(
        WindowStatement::new()
            .order_by(Name::runtime("level"), Order::Asc)
            .order_by(Name::runtime("id"), Order::Asc)
            .frame(FrameType::Range.unbounded_preceding().and_current_row()),
    );
    assert_eq!(db.query_all(unbounded.as_str(), &[]).await?.len(), 4);

    Ok(())
}
