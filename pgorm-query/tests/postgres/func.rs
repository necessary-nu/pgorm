//! Function calls: the aggregate modifier clauses, and a call in
//! subquery position.

use super::*;
use crate::oracle::assert_eq;

// [spec:pgorm:def:sql.ast.func+12/test]      FILTER takes any IntoCondition
// [spec:pgorm:req:sql.render.func-mods/test]
#[test]
fn aggregate_filter_renders_after_the_arguments() {
    assert_eq!(
        Query::select()
            .expr(Func::count(Expr::col(Char::Id)).filter(Expr::col(Char::FontSize).gt(12)))
            .from(Char::Table)
            .to_string(),
        r#"SELECT COUNT("id") FILTER (WHERE "font_size" > 12) FROM "character""#
    );

    assert_eq!(
        Query::select()
            .expr(
                Func::sum(Expr::col(Char::SizeW)).filter(
                    Condition::all()
                        .add(Expr::col(Char::FontSize).gt(12))
                        .add(Expr::col(Char::Ascii).eq(true)),
                )
            )
            .from(Char::Table)
            .to_string(),
        r#"SELECT SUM("size_w") FILTER (WHERE "font_size" > 12 AND "ascii" = TRUE) FROM "character""#
    );
}

// A filter binds its values in the enclosing statement's parameter sequence,
// after the ones the projection already bound.
// [spec:pgorm:req:sql.render.func-mods/test]
#[test]
fn aggregate_filter_numbers_its_parameters_in_sequence() {
    assert_eq!(
        Query::select()
            .expr(Func::count(Expr::col(Char::Id)).filter(Expr::col(Char::FontSize).gt(12)))
            .from(Char::Table)
            .and_where(Expr::col(Char::SizeW).lt(100))
            .build(),
        (
            [
                r#"SELECT COUNT("id") FILTER (WHERE "font_size" > $1)"#,
                r#"FROM "character" WHERE "size_w" < $2"#,
            ]
            .join(" "),
            Values(vec![12i32.into(), 100i32.into()])
        )
    );
}

// Each `filter` call replaces the last rather than accumulating.
// [spec:pgorm:def:sql.ast.func+12/test]
#[test]
fn a_second_filter_replaces_the_first() {
    assert_eq!(
        Query::select()
            .expr(
                Func::count(Expr::col(Char::Id))
                    .filter(Expr::col(Char::FontSize).gt(12))
                    .filter(Expr::col(Char::SizeW).gt(3))
            )
            .from(Char::Table)
            .to_string(),
        r#"SELECT COUNT("id") FILTER (WHERE "size_w" > 3) FROM "character""#
    );
}

// [spec:pgorm:def:sql.ast.func+12/test]      WITHIN GROUP, and the two
// ordered-set constructors
// [spec:pgorm:req:sql.render.func-mods/test]
#[test]
fn within_group_renders_the_ordered_set_ordering() {
    assert_eq!(
        Query::select()
            .expr(Func::percentile_cont(0.5).within_group(Char::SizeW, Order::Asc))
            .from(Char::Table)
            .to_string(),
        r#"SELECT PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY "size_w" ASC) FROM "character""#
    );

    assert_eq!(
        Query::select()
            .expr(
                Func::percentile_disc(0.9)
                    .within_group_expr(Expr::col(Char::SizeW).mul(2), Order::Desc)
            )
            .from(Char::Table)
            .to_string(),
        r#"SELECT PERCENTILE_DISC(0.9) WITHIN GROUP (ORDER BY "size_w" * 2 DESC) FROM "character""#
    );
}

// The ordering accumulates, for the hypothetical-set aggregates that rank
// against several columns at once.
// [spec:pgorm:def:sql.ast.func+12/test]
#[test]
fn within_group_accumulates_its_ordering() {
    assert_eq!(
        Query::select()
            .expr(
                Func::named(Name::runtime("rank"))
                    .arg(1)
                    .arg("a")
                    .within_group(Char::SizeW, Order::Asc)
                    .within_group(Char::Character, Order::Desc)
            )
            .from(Char::Table)
            .to_string(),
        [
            r#"SELECT rank(1, 'a') WITHIN GROUP (ORDER BY "size_w" ASC, "character" DESC)"#,
            r#"FROM "character""#,
        ]
        .join(" ")
    );
}

// The grammar's order is WITHIN GROUP, then FILTER, then OVER — and the last
// of those is written by a different rule, so the three composing is the whole
// claim.
// [spec:pgorm:req:sql.render.func-mods/test]
// [spec:pgorm:req:sql.render.window+7/test]
#[test]
fn modifiers_compose_with_each_other_and_with_over() {
    assert_eq!(
        Query::select()
            .expr(
                Func::percentile_cont(0.5)
                    .within_group(Char::SizeW, Order::Asc)
                    .filter(Expr::col(Char::Ascii).eq(true))
            )
            .from(Char::Table)
            .to_string(),
        [
            r#"SELECT PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY "size_w" ASC)"#,
            r#"FILTER (WHERE "ascii" = TRUE) FROM "character""#,
        ]
        .join(" ")
    );

    assert_eq!(
        Query::select()
            .from(Char::Table)
            .expr_window(
                Func::count(Expr::col(Char::Id)).filter(Expr::col(Char::FontSize).gt(12)),
                WindowStatement::partition_by(Char::FontSize),
            )
            .to_string(),
        [
            r#"SELECT COUNT("id") FILTER (WHERE "font_size" > 12)"#,
            r#"OVER ( PARTITION BY "font_size" ) FROM "character""#,
        ]
        .join(" ")
    );
}

// DISTINCT is an argument modifier and the two clauses are call modifiers, so
// they stack without interfering.
// [spec:pgorm:req:sql.render.func-mods/test]
#[test]
fn filter_stacks_with_the_distinct_argument_modifier() {
    assert_eq!(
        Query::select()
            .expr(
                Func::count_distinct(Expr::col(Char::FontId))
                    .filter(Expr::col(Char::Ascii).eq(true))
            )
            .from(Char::Table)
            .to_string(),
        r#"SELECT COUNT(DISTINCT "font_id") FILTER (WHERE "ascii" = TRUE) FROM "character""#
    );
}

// A call carrying neither clause renders exactly what it always did.
// [spec:pgorm:req:sql.render.func-mods/test]
#[test]
fn an_unmodified_call_renders_unchanged() {
    assert_eq!(
        Query::select()
            .expr(Func::count(Expr::col(Char::Id)))
            .from(Char::Table)
            .to_string(),
        r#"SELECT COUNT("id") FROM "character""#
    );
}

// [spec:pgorm:def:sql.ast.func+12/test]
#[test]
fn sub_query_with_fn() {
    #[derive(SqlName)]
    #[iden = "jsonb_agg"]
    pub struct ArrayFunc;

    let sub_select = Query::select()
        .column(Asterisk)
        .from(Char::Table)
        .to_owned();

    let select = Query::select()
        .expr(Func::named(ArrayFunc).arg(SimpleExpr::SubQuery(
            None,
            Box::new(sub_select.into_sub_query_statement()),
        )))
        .to_owned();

    assert_eq!(
        select.to_string(),
        r#"SELECT jsonb_agg((SELECT * FROM "character"))"#
    );
}

// PostgreSQL 18's UUID family: the two generators, v7's shifted form with its
// interval bound like any value, and the two readers over a column.
// [spec:pgorm:def:sql.ast.func+12/test]
#[test]
fn the_uuid_family_renders_its_calls() {
    assert_eq!(
        Query::select()
            .expr(Func::uuidv4())
            .expr(Func::uuidv7())
            .to_string(),
        r#"SELECT UUIDV4(), UUIDV7()"#
    );
    assert_eq!(
        Query::select()
            .expr(Func::uuidv7_shifted(
                Expr::val("-1 day").cast_as(Name::runtime("interval"))
            ))
            .build(),
        (
            r#"SELECT UUIDV7(CAST($1::text AS interval))"#.to_owned(),
            Values(vec!["-1 day".into()])
        )
    );
    assert_eq!(
        Query::select()
            .expr(Func::uuid_extract_timestamp(Expr::col(Glyph::Id)))
            .expr(Func::uuid_extract_version(Expr::col(Glyph::Id)))
            .from(Glyph::Table)
            .to_string(),
        r#"SELECT UUID_EXTRACT_TIMESTAMP("id"), UUID_EXTRACT_VERSION("id") FROM "glyph""#
    );
}

// A v7 call is a column default like any expression.
// [spec:pgorm:def:sql.ast.func+12/test]
#[test]
fn a_uuidv7_call_is_a_column_default() {
    assert_eq!(
        Table::create(Glyph::Table)
            .col(ColumnDef::new(Glyph::Id).uuid().default(Func::uuidv7()))
            .primary_key(Glyph::Id)
            .to_string(),
        r#"CREATE TABLE "glyph" ( "id" uuid DEFAULT UUIDV7(), PRIMARY KEY ("id") )"#
    );
}
