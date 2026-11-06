#![allow(unused_imports, dead_code)]

//! A bound value where an overloaded operator or a polymorphic function reads
//! its type, against a live server.
//!
//! Each case is one PostgreSQL refuses with a bare placeholder, and
//! each guard is one the column's type must keep deciding.

pub mod common;
pub use common::{TestContext, setup::*};
use jiff::{Timestamp, civil::Date};
use pgorm::pgorm_query::{
    Expr, ExprTrait, Func, Interval, Name, Query, Range, SelectStatement, SimpleExpr,
};
use pgorm::{ConnectionTrait, Error, SelectGetableTuple, Selector, TryGetableMany};
use rust_decimal::Decimal;

async fn read<T, C>(db: &C, query: SelectStatement) -> Result<Vec<T>, Error>
where
    T: TryGetableMany,
    C: ConnectionTrait,
{
    Selector::<SelectGetableTuple<T>>::into_tuple(query)
        .all(db)
        .await
}

fn cast(value: SimpleExpr, type_name: &'static str) -> SimpleExpr {
    value.cast_as(Name::runtime(type_name))
}

// [spec:pgorm:sem:sql.render.placeholder-typing+2/test]    against a live server:
// `unnest` and `generate_series` read a bound argument's own type, a range
// containing a timestamp reads the range's, and a date plus an integer, a
// timestamp minus an interval and `->` an integer read the operand's
#[pgorm_macros::test]
async fn bound_operands_carry_the_type_their_operator_reads() -> Result<(), Error> {
    let ctx = TestContext::new("bound_operands_carry_the_type_their_operator_reads").await;
    let db = ctx.db.get().await?;

    let unnest = Query::select()
        .expr(Func::unnest(Expr::val(vec![4, 1, 3])))
        .take();
    assert_eq!(unnest.build().0, "SELECT UNNEST($1::int4[])");
    assert_eq!(read::<(i32,), _>(&db, unnest).await?, [(4,), (1,), (3,)]);

    let series = Query::select()
        .expr(Func::generate_series(Expr::val(1), Expr::val(3)))
        .take();
    assert_eq!(read::<(i32,), _>(&db, series).await?, [(1,), (2,), (3,)]);

    let start: Timestamp = "2026-11-02T00:00:00Z".parse().expect("an instant");
    let end: Timestamp = "2026-11-03T00:00:00Z".parse().expect("an instant");
    let noon = cast(Expr::val("2026-11-02T12:00:00Z").into(), "timestamptz");
    let within = Query::select()
        .expr(Expr::val(Range::from(start..end)).contains(noon.clone()))
        .expr(noon.contained(Expr::val(Range::from(start..end))))
        .take();
    assert_eq!(
        within.build().0,
        r#"SELECT $1::tstzrange @> CAST($2::text AS timestamptz), CAST($3::text AS timestamptz) <@ $4::tstzrange"#
    );
    assert_eq!(read::<(bool, bool), _>(&db, within).await?, [(true, true)]);

    let day = cast(Expr::val(Date::constant(2026, 11, 1)).into(), "date");
    let later = Query::select()
        .expr(day.clone().add(1))
        .expr(day.sub(Interval::new(0, 1, 0)))
        .expr(cast(Expr::val(serde_json::json!([10, 20])).into(), "jsonb").get_json_field(1))
        .take();
    assert_eq!(
        later.build().0,
        r#"SELECT CAST($1::date AS date) + $2::int4, CAST($3::date AS date) - $4::interval, CAST($5 AS jsonb) -> $6::int4"#
    );
    let rows: Vec<(Date, jiff::civil::DateTime, serde_json::Value)> = read(&db, later).await?;
    assert_eq!(rows[0].0, Date::constant(2026, 11, 2));
    assert_eq!(rows[0].1, Date::constant(2026, 10, 31).at(0, 0, 0, 0));
    assert_eq!(rows[0].2, serde_json::json!(20));

    drop(db);
    ctx.delete().await;
    Ok(())
}

// [spec:pgorm:sem:sql.render.placeholder-typing+2/test]    against a live server:
// a float beside a numeric stays the numeric's, text beside a `varchar` is read
// as one, and a comparison's placeholder stays the column's
#[pgorm_macros::test]
async fn a_column_still_types_what_it_should() -> Result<(), Error> {
    let ctx = TestContext::new("a_column_still_types_what_it_should").await;
    let db = ctx.db.get().await?;

    let price = cast(Expr::val(Decimal::new(250, 2)).into(), "numeric");
    let name = cast(Expr::val("Lemon").into(), "varchar");
    let kept = Query::select()
        .expr(price.clone().mul(1.5f64))
        .expr(name.concat(" Tart"))
        .expr(price.gt(2))
        .take();
    assert_eq!(
        kept.build().0,
        r#"SELECT CAST($1::numeric AS numeric) * $2, CAST($3::text AS varchar) || $4, CAST($5::numeric AS numeric) > $6"#
    );
    let rows: Vec<(Decimal, String, bool)> = read(&db, kept).await?;
    assert_eq!(rows[0].0, Decimal::new(3750, 3));
    assert_eq!(rows[0].1, "Lemon Tart");
    assert!(rows[0].2);

    drop(db);
    ctx.delete().await;
    Ok(())
}
