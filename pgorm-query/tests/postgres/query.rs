use super::*;
use crate::oracle::{assert_eq, assert_query_eq, parsed_nodes};

// [spec:pgorm:req:sql.ast+4/test]
// [spec:pgorm:def:sql.ast.select+6/test]
// [spec:pgorm:req:sql.render.ident-quoting+7/test]
#[test]
fn select_1() {
    assert_eq!(
        Query::select()
            .columns([Char::Character, Char::SizeW, Char::SizeH])
            .from(Char::Table)
            .limit(10)
            .offset(100)
            .to_string(),
        r#"SELECT "character", "size_w", "size_h" FROM "character" LIMIT 10 OFFSET 100"#
    );
}

// [spec:pgorm:def:sql.ast.expr+8/test]
#[test]
fn select_2() {
    assert_eq!(
        Query::select()
            .columns([Char::Character, Char::SizeW, Char::SizeH])
            .from(Char::Table)
            .and_where(Expr::col(Char::SizeW).eq(3))
            .to_string(),
        r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "size_w" = 3"#
    );
}

#[test]
fn select_3() {
    assert_eq!(
        Query::select()
            .columns([Char::Character, Char::SizeW, Char::SizeH])
            .from(Char::Table)
            .and_where(Expr::col(Char::SizeW).eq(3))
            .and_where(Expr::col(Char::SizeH).eq(4))
            .to_string(),
        r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "size_w" = 3 AND "size_h" = 4"#
    );
}

// [spec:pgorm:req:sql.ast.select.from+4/test]
#[test]
fn select_4() {
    assert_eq!(
        Query::select()
            .columns([Glyph::Aspect])
            .from_subquery(
                Query::select()
                    .columns([Glyph::Image, Glyph::Aspect])
                    .from(Glyph::Table)
                    .take(),
                Name::runtime("subglyph")
            )
            .to_string(),
        r#"SELECT "aspect" FROM (SELECT "image", "aspect" FROM "glyph") AS "subglyph""#
    );
}

// [spec:pgorm:req:sql.ast.expr.in+1/test]
#[test]
fn select_5() {
    assert_eq!(
        Query::select()
            .column((Glyph::Table, Glyph::Image))
            .from(Glyph::Table)
            .and_where(Expr::col((Glyph::Table, Glyph::Aspect)).is_in([3, 4]))
            .to_string(),
        r#"SELECT "glyph"."image" FROM "glyph" WHERE "glyph"."aspect" IN (3, 4)"#
    );
}

// [spec:pgorm:req:sql.render.condition-chain+1/test]
#[test]
fn select_6() {
    assert_eq!(
        Query::select()
            .columns([Glyph::Aspect])
            .exprs([Expr::col(Glyph::Image).max()])
            .from(Glyph::Table)
            .group_by_columns([Glyph::Aspect])
            .and_having(Expr::col(Glyph::Aspect).gt(2))
            .to_string(),
        r#"SELECT "aspect", MAX("image") FROM "glyph" GROUP BY "aspect" HAVING "aspect" > 2"#
    );
}

#[test]
fn select_7() {
    assert_eq!(
        Query::select()
            .columns([Glyph::Aspect])
            .from(Glyph::Table)
            .and_where(Expr::expr(Expr::col(Glyph::Aspect).if_null(0)).gt(2))
            .to_string(),
        r#"SELECT "aspect" FROM "glyph" WHERE COALESCE("aspect", 0) > 2"#
    );
}

// [spec:pgorm:req:sql.render.joins+2/test]
#[test]
fn select_8() {
    assert_eq!(
        Query::select()
            .columns([Char::Character])
            .from(Char::Table)
            .left_join(
                Font::Table,
                Expr::col((Char::Table, Char::FontId)).equals((Font::Table, Font::Id))
            )
            .to_string(),
        r#"SELECT "character" FROM "character" LEFT JOIN "font" ON "character"."font_id" = "font"."id""#
    );
}

// [spec:pgorm:req:sql.ast.select.join+1/test]
#[test]
fn select_9() {
    assert_eq!(
        Query::select()
            .columns([Char::Character])
            .from(Char::Table)
            .left_join(
                Font::Table,
                Expr::col((Char::Table, Char::FontId)).equals((Font::Table, Font::Id))
            )
            .inner_join(
                Glyph::Table,
                Expr::col((Char::Table, Char::Character)).equals((Glyph::Table, Glyph::Image))
            )
            .to_string(),
        r#"SELECT "character" FROM "character" LEFT JOIN "font" ON "character"."font_id" = "font"."id" INNER JOIN "glyph" ON "character"."character" = "glyph"."image""#
    );
}

#[test]
fn select_10() {
    assert_eq!(
        Query::select()
            .columns([Char::Character])
            .from(Char::Table)
            .left_join(
                Font::Table,
                Expr::col((Char::Table, Char::FontId))
                    .equals((Font::Table, Font::Id))
                    .and(Expr::col((Char::Table, Char::FontId)).equals((Font::Table, Font::Id)))
            )
            .to_string(),
        r#"SELECT "character" FROM "character" LEFT JOIN "font" ON "character"."font_id" = "font"."id" AND "character"."font_id" = "font"."id""#
    );
}

#[test]
fn select_11() {
    assert_eq!(
        Query::select()
            .columns([Glyph::Aspect])
            .from(Glyph::Table)
            .and_where(Expr::expr(Expr::col(Glyph::Aspect).if_null(0)).gt(2))
            .order_by(Glyph::Image, Order::Desc)
            .order_by((Glyph::Table, Glyph::Aspect), Order::Asc)
            .to_string(),
        r#"SELECT "aspect" FROM "glyph" WHERE COALESCE("aspect", 0) > 2 ORDER BY "image" DESC, "glyph"."aspect" ASC"#
    );
}

#[test]
fn select_12() {
    assert_eq!(
        Query::select()
            .columns([Glyph::Aspect])
            .from(Glyph::Table)
            .and_where(Expr::expr(Expr::col(Glyph::Aspect).if_null(0)).gt(2))
            .order_by_columns([(Glyph::Id, Order::Asc), (Glyph::Aspect, Order::Desc)])
            .to_string(),
        r#"SELECT "aspect" FROM "glyph" WHERE COALESCE("aspect", 0) > 2 ORDER BY "id" ASC, "aspect" DESC"#
    );
}

#[test]
fn select_13() {
    assert_eq!(
        Query::select()
            .columns([Glyph::Aspect])
            .from(Glyph::Table)
            .and_where(Expr::expr(Expr::col(Glyph::Aspect).if_null(0)).gt(2))
            .order_by_columns([
                ((Glyph::Table, Glyph::Id), Order::Asc),
                ((Glyph::Table, Glyph::Aspect), Order::Desc),
            ])
            .to_string(),
        r#"SELECT "aspect" FROM "glyph" WHERE COALESCE("aspect", 0) > 2 ORDER BY "glyph"."id" ASC, "glyph"."aspect" DESC"#
    );
}

#[test]
fn select_14() {
    assert_eq!(
        Query::select()
            .columns([Glyph::Id, Glyph::Aspect])
            .expr(Expr::col(Glyph::Image).max())
            .from(Glyph::Table)
            .group_by_columns([(Glyph::Table, Glyph::Id), (Glyph::Table, Glyph::Aspect)])
            .and_having(Expr::col(Glyph::Aspect).gt(2))
            .to_string(),
        r#"SELECT "id", "aspect", MAX("image") FROM "glyph" GROUP BY "glyph"."id", "glyph"."aspect" HAVING "aspect" > 2"#
    );
}

#[test]
fn select_15() {
    assert_eq!(
        Query::select()
            .columns([Char::Character])
            .from(Char::Table)
            .and_where(Expr::col(Char::FontId).is_null())
            .to_string(),
        r#"SELECT "character" FROM "character" WHERE "font_id" IS NULL"#
    );
}

#[test]
fn select_16() {
    assert_eq!(
        Query::select()
            .columns([Char::Character])
            .from(Char::Table)
            .and_where(Expr::col(Char::FontId).is_null())
            .and_where(Expr::col(Char::Character).is_not_null())
            .to_string(),
        r#"SELECT "character" FROM "character" WHERE "font_id" IS NULL AND "character" IS NOT NULL"#
    );
}

#[test]
fn select_17() {
    assert_eq!(
        Query::select()
            .columns([(Glyph::Table, Glyph::Image)])
            .from(Glyph::Table)
            .and_where(Expr::col((Glyph::Table, Glyph::Aspect)).between(3, 5))
            .to_string(),
        r#"SELECT "glyph"."image" FROM "glyph" WHERE "glyph"."aspect" BETWEEN 3 AND 5"#
    );
}

#[test]
fn select_18() {
    assert_eq!(
        Query::select()
            .columns([Glyph::Aspect])
            .from(Glyph::Table)
            .and_where(Expr::col(Glyph::Aspect).between(3, 5))
            .and_where(Expr::col(Glyph::Aspect).not_between(8, 10))
            .to_string(),
        r#"SELECT "aspect" FROM "glyph" WHERE "aspect" BETWEEN 3 AND 5 AND "aspect" NOT BETWEEN 8 AND 10"#
    );
}

#[test]
fn select_19() {
    assert_eq!(
        Query::select()
            .columns([Char::Character])
            .from(Char::Table)
            .and_where(Expr::col(Char::Character).eq("A"))
            .to_string(),
        r#"SELECT "character" FROM "character" WHERE "character" = 'A'"#
    );
}

#[test]
fn select_20() {
    assert_eq!(
        Query::select()
            .column(Char::Character)
            .from(Char::Table)
            .and_where(Expr::col(Char::Character).like("A"))
            .to_string(),
        r#"SELECT "character" FROM "character" WHERE "character" LIKE 'A'"#
    );
}

#[test]
fn select_21() {
    assert_eq!(
        Query::select()
            .columns([Char::Character])
            .from(Char::Table)
            .cond_where(
                Condition::any()
                    .add(Expr::col(Char::Character).like("A%"))
                    .add(Expr::col(Char::Character).like("%B"))
                    .add(Expr::col(Char::Character).like("%C%"))
            )
            .to_string(),
        r#"SELECT "character" FROM "character" WHERE "character" LIKE 'A%' OR "character" LIKE '%B' OR "character" LIKE '%C%'"#
    );
}

// [spec:pgorm:def:sql.ast.condition+2/test]
#[test]
fn select_22() {
    assert_eq!(
        Query::select()
            .column(Char::Character)
            .from(Char::Table)
            .cond_where(
                Condition::all()
                    .add(
                        Condition::any()
                            .add(Expr::col(Char::Character).like("C"))
                            .add(
                                Expr::col(Char::Character)
                                    .like("D")
                                    .and(Expr::col(Char::Character).like("E"))
                            )
                    )
                    .add(
                        Expr::col(Char::Character)
                            .like("F")
                            .or(Expr::col(Char::Character).like("G"))
                    )
            )
            .to_string(),
        r#"SELECT "character" FROM "character" WHERE ("character" LIKE 'C' OR ("character" LIKE 'D' AND "character" LIKE 'E')) AND ("character" LIKE 'F' OR "character" LIKE 'G')"#
    );
}

#[test]
fn select_23() {
    assert_eq!(
        Query::select()
            .column(Char::Character)
            .from(Char::Table)
            .and_where_option(None)
            .to_string(),
        r#"SELECT "character" FROM "character""#
    );
}

#[test]
fn select_24() {
    assert_eq!(
        Query::select()
            .column(Char::Character)
            .from(Char::Table)
            .conditions(
                true,
                |x| {
                    x.and_where(Expr::col(Char::FontId).eq(5));
                },
                |_| ()
            )
            .to_string(),
        r#"SELECT "character" FROM "character" WHERE "font_id" = 5"#
    );
}

#[test]
fn select_25() {
    assert_eq!(
        Query::select()
            .column(Char::Character)
            .from(Char::Table)
            .and_where(
                Expr::col(Char::SizeW)
                    .mul(2)
                    .eq(Expr::col(Char::SizeH).div(2))
            )
            .to_string(),
        r#"SELECT "character" FROM "character" WHERE "size_w" * 2 = "size_h" / 2"#
    );
}

#[test]
fn select_26() {
    assert_eq!(
        Query::select()
            .column(Char::Character)
            .from(Char::Table)
            .and_where(
                Expr::expr(Expr::col(Char::SizeW).add(1))
                    .mul(2)
                    .eq(Expr::expr(Expr::col(Char::SizeH).div(2)).sub(1))
            )
            .to_string(),
        r#"SELECT "character" FROM "character" WHERE ("size_w" + 1) * 2 = ("size_h" / 2) - 1"#
    );
}

#[test]
fn select_27() {
    assert_eq!(
        Query::select()
            .columns([Char::Character, Char::SizeW, Char::SizeH])
            .from(Char::Table)
            .and_where(Expr::col(Char::SizeW).eq(3))
            .and_where(Expr::col(Char::SizeH).eq(4))
            .and_where(Expr::col(Char::SizeH).eq(5))
            .to_string(),
        r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "size_w" = 3 AND "size_h" = 4 AND "size_h" = 5"#
    );
}

#[test]
fn select_28() {
    assert_eq!(
        Query::select()
            .columns([Char::Character, Char::SizeW, Char::SizeH])
            .from(Char::Table)
            .cond_where(
                Condition::any()
                    .add(Expr::col(Char::SizeW).eq(3))
                    .add(Expr::col(Char::SizeH).eq(4))
                    .add(Expr::col(Char::SizeH).eq(5))
            )
            .to_string(),
        r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE "size_w" = 3 OR "size_h" = 4 OR "size_h" = 5"#
    );
}

#[test]
fn select_30() {
    assert_eq!(
        Query::select()
            .columns([Char::Character, Char::SizeW, Char::SizeH])
            .from(Char::Table)
            .and_where(
                Expr::col(Char::SizeW)
                    .mul(2)
                    .add(Expr::col(Char::SizeH).div(3))
                    .eq(4)
            )
            .to_string(),
        r#"SELECT "character", "size_w", "size_h" FROM "character" WHERE ("size_w" * 2) + ("size_h" / 3) = 4"#
    );
}

#[test]
fn select_31() {
    assert_eq!(
        Query::select()
            .expr((1..10_i32).fold(Expr::value(0), |expr, i| { expr.add(i) }))
            .to_string(),
        r#"SELECT 0 + 1 + 2 + 3 + 4 + 5 + 6 + 7 + 8 + 9"#
    );
}

#[test]
fn select_32() {
    assert_eq!(
        Query::select()
            .expr_as(Expr::col(Char::Character), Name::runtime("C"))
            .from(Char::Table)
            .to_string(),
        r#"SELECT "character" AS "C" FROM "character""#
    );
}

#[test]
fn select_33() {
    assert_eq!(
        Query::select()
            .column(Glyph::Image)
            .from(Glyph::Table)
            .and_where(
                Expr::col(Glyph::Aspect)
                    .in_subquery(Query::select().expr(Expr::raw("3 + 2 * 2")).take())
            )
            .to_string(),
        r#"SELECT "image" FROM "glyph" WHERE "aspect" IN (SELECT 3 + 2 * 2)"#
    );
}

#[test]
fn select_34a() {
    assert_eq!(
        Query::select()
            .column(Glyph::Aspect)
            .expr(Expr::col(Glyph::Image).max())
            .from(Glyph::Table)
            .group_by_columns([Glyph::Aspect])
            .cond_having(
                Condition::any()
                    .add(
                        Expr::col(Glyph::Aspect)
                            .gt(2)
                            .or(Expr::col(Glyph::Aspect).lt(8))
                    )
                    .add(
                        Expr::col(Glyph::Aspect)
                            .gt(12)
                            .and(Expr::col(Glyph::Aspect).lt(18))
                    )
                    .add(Expr::col(Glyph::Aspect).gt(32))
            )
            .to_string(),
        [
            r#"SELECT "aspect", MAX("image") FROM "glyph" GROUP BY "aspect""#,
            r#"HAVING "aspect" > 2 OR "aspect" < 8"#,
            r#"OR ("aspect" > 12 AND "aspect" < 18)"#,
            r#"OR "aspect" > 32"#,
        ]
        .join(" ")
    );
}

#[test]
fn select_35() {
    let (statement, values) = Query::select()
        .column(Glyph::Id)
        .from(Glyph::Table)
        .and_where(Expr::col(Glyph::Aspect).is_null())
        .build();

    assert_eq!(
        statement,
        r#"SELECT "id" FROM "glyph" WHERE "aspect" IS NULL"#
    );
    assert_eq!(values.0, vec![]);
}

#[test]
fn select_36() {
    let (statement, values) = Query::select()
        .column(Glyph::Id)
        .from(Glyph::Table)
        .cond_where(Condition::any().add(Expr::col(Glyph::Aspect).is_null()))
        .build();

    assert_eq!(
        statement,
        r#"SELECT "id" FROM "glyph" WHERE "aspect" IS NULL"#
    );
    assert_eq!(values.0, vec![]);
}

// [spec:pgorm:sem:sql.ast.condition.flattening/test]
#[test]
fn select_37() {
    let (statement, values) = Query::select()
        .column(Glyph::Id)
        .from(Glyph::Table)
        .cond_where(Condition::any().add(Condition::all()).add(Condition::any()))
        .build();

    assert_eq!(statement, r#"SELECT "id" FROM "glyph" WHERE TRUE OR FALSE"#);
    assert_eq!(values.0, vec![]);
}

#[test]
fn select_37a() {
    let (statement, values) = Query::select()
        .column(Glyph::Id)
        .from(Glyph::Table)
        .cond_where(
            Condition::all()
                .add(Condition::all().not())
                .add(Condition::any().not())
                .not(),
        )
        .build();

    assert_eq!(
        statement,
        r#"SELECT "id" FROM "glyph" WHERE NOT ((NOT TRUE) AND (NOT FALSE))"#
    );
    assert_eq!(values.0, vec![]);
}

#[test]
fn select_38() {
    let (statement, values) = Query::select()
        .column(Glyph::Id)
        .from(Glyph::Table)
        .cond_where(
            Condition::any()
                .add(Expr::col(Glyph::Aspect).is_null())
                .add(Expr::col(Glyph::Aspect).is_not_null()),
        )
        .build();

    assert_eq!(
        statement,
        r#"SELECT "id" FROM "glyph" WHERE "aspect" IS NULL OR "aspect" IS NOT NULL"#
    );
    assert_eq!(values.0, vec![]);
}

#[test]
fn select_39() {
    let (statement, values) = Query::select()
        .column(Glyph::Id)
        .from(Glyph::Table)
        .cond_where(
            Condition::all()
                .add(Expr::col(Glyph::Aspect).is_null())
                .add(Expr::col(Glyph::Aspect).is_not_null()),
        )
        .build();

    assert_eq!(
        statement,
        r#"SELECT "id" FROM "glyph" WHERE "aspect" IS NULL AND "aspect" IS NOT NULL"#
    );
    assert_eq!(values.0, vec![]);
}

#[test]
fn select_40() {
    let statement = Query::select()
        .column(Glyph::Id)
        .from(Glyph::Table)
        .cond_where(
            Condition::any()
                .add(Expr::col(Glyph::Aspect).is_null())
                .add(
                    Condition::all()
                        .add(Expr::col(Glyph::Aspect).is_not_null())
                        .add(Expr::col(Glyph::Aspect).lt(8)),
                ),
        )
        .to_string();

    assert_eq!(
        statement,
        r#"SELECT "id" FROM "glyph" WHERE "aspect" IS NULL OR ("aspect" IS NOT NULL AND "aspect" < 8)"#
    );
}

#[test]
fn select_41() {
    assert_eq!(
        Query::select()
            .columns([Glyph::Aspect])
            .exprs([Expr::col(Glyph::Image).max()])
            .from(Glyph::Table)
            .group_by_columns([Glyph::Aspect])
            .cond_having(Condition::any().add(Expr::col(Glyph::Aspect).gt(2)))
            .to_string(),
        r#"SELECT "aspect", MAX("image") FROM "glyph" GROUP BY "aspect" HAVING "aspect" > 2"#
    );
}

#[test]
fn select_42() {
    let statement = Query::select()
        .column(Glyph::Id)
        .from(Glyph::Table)
        .cond_where(
            Condition::all()
                .add_option(Some(Expr::col(Glyph::Aspect).lt(8)))
                .add(Expr::col(Glyph::Aspect).is_not_null()),
        )
        .to_string();

    assert_eq!(
        statement,
        r#"SELECT "id" FROM "glyph" WHERE "aspect" < 8 AND "aspect" IS NOT NULL"#
    );
}

// [spec:pgorm:req:sql.render.condition-chain+1/test]
#[test]
fn select_43() {
    let statement = Query::select()
        .column(Glyph::Id)
        .from(Glyph::Table)
        .cond_where(Condition::all().add_option::<SimpleExpr>(None))
        .to_string();

    assert_eq!(statement, r#"SELECT "id" FROM "glyph" WHERE TRUE"#);
}

#[test]
fn select_44() {
    let statement = Query::select()
        .column(Glyph::Id)
        .from(Glyph::Table)
        .cond_where(
            Condition::any()
                .not()
                .add_option(Some(Expr::col(Glyph::Aspect).lt(8))),
        )
        .to_string();

    assert_eq!(
        statement,
        r#"SELECT "id" FROM "glyph" WHERE NOT "aspect" < 8"#
    );
}

#[test]
fn select_45() {
    let statement = Query::select()
        .column(Glyph::Id)
        .from(Glyph::Table)
        .cond_where(
            Condition::any()
                .not()
                .add_option(Some(Expr::col(Glyph::Aspect).lt(8)))
                .add(Expr::col(Glyph::Aspect).is_not_null()),
        )
        .to_string();

    assert_eq!(
        statement,
        r#"SELECT "id" FROM "glyph" WHERE NOT ("aspect" < 8 OR "aspect" IS NOT NULL)"#
    );
}

#[test]
fn select_46() {
    let statement = Query::select()
        .column(Glyph::Id)
        .from(Glyph::Table)
        .cond_where(
            Condition::all()
                .not()
                .add_option(Some(Expr::col(Glyph::Aspect).lt(8))),
        )
        .to_string();

    assert_eq!(
        statement,
        r#"SELECT "id" FROM "glyph" WHERE NOT "aspect" < 8"#
    );
}

#[test]
fn select_47() {
    let statement = Query::select()
        .column(Glyph::Id)
        .from(Glyph::Table)
        .cond_where(
            Condition::all()
                .not()
                .add_option(Some(Expr::col(Glyph::Aspect).lt(8)))
                .add(Expr::col(Glyph::Aspect).is_not_null()),
        )
        .to_string();

    assert_eq!(
        statement,
        r#"SELECT "id" FROM "glyph" WHERE NOT ("aspect" < 8 AND "aspect" IS NOT NULL)"#
    );
}

#[test]
fn select_48() {
    let statement = Query::select()
        .column(Glyph::Id)
        .from(Glyph::Table)
        .cond_where(
            Condition::all().add_option(Some(ConditionExpression::SimpleExpr(
                Expr::tuple([Expr::col(Glyph::Aspect).into(), Expr::value(100)])
                    .lt(Expr::tuple([Expr::value(8), Expr::value(100)])),
            ))),
        )
        .to_string();

    assert_eq!(
        statement,
        r#"SELECT "id" FROM "glyph" WHERE ("aspect", 100) < (8, 100)"#
    );
}

#[test]
fn select_48a() {
    let statement = Query::select()
        .column(Glyph::Id)
        .from(Glyph::Table)
        .cond_where(
            Condition::all().add_option(Some(ConditionExpression::SimpleExpr(
                Expr::tuple([
                    Expr::col(Glyph::Aspect).into(),
                    Expr::value(String::from("100")),
                ])
                .in_tuples([(8, String::from("100"))]),
            ))),
        )
        .to_string();

    assert_eq!(
        statement,
        r#"SELECT "id" FROM "glyph" WHERE ("aspect", '100') IN ((8, '100'))"#
    );
}

// [spec:pgorm:def:sql.ast.keywords+6/test]    `Asterisk` as a bare projection
#[test]
fn select_49() {
    let statement = Query::select()
        .column(Asterisk)
        .from(Char::Table)
        .to_string();

    assert_eq!(statement, r#"SELECT * FROM "character""#);
}

// [spec:pgorm:def:sql.ast.keywords+6/test]    `(Table, Asterisk)` renders `"table".*`
#[test]
fn select_50() {
    let statement = Query::select()
        .column((Char::Table, Asterisk))
        .column((Font::Table, Font::Name))
        .from(Char::Table)
        .inner_join(
            Font::Table,
            Expr::col((Char::Table, Char::FontId)).equals((Font::Table, Font::Id)),
        )
        .to_string();

    assert_eq!(
        statement,
        r#"SELECT "character".*, "font"."name" FROM "character" INNER JOIN "font" ON "character"."font_id" = "font"."id""#
    )
}

// [spec:pgorm:req:sql.ast.order+3/test]
#[test]
fn select_51() {
    assert_eq!(
        Query::select()
            .columns([Glyph::Aspect])
            .from(Glyph::Table)
            .and_where(Expr::expr(Expr::col(Glyph::Aspect).if_null(0)).gt(2))
            .order_by_with_nulls(Glyph::Image, Order::Desc, NullOrdering::First)
            .order_by_with_nulls(
                (Glyph::Table, Glyph::Aspect),
                Order::Asc,
                NullOrdering::Last
            )
            .to_string(),
        [
            r#"SELECT "aspect""#,
            r#"FROM "glyph""#,
            r#"WHERE COALESCE("aspect", 0) > 2"#,
            r#"ORDER BY "image" DESC NULLS FIRST,"#,
            r#""glyph"."aspect" ASC NULLS LAST"#,
        ]
        .join(" ")
    );
}

#[test]
fn select_52() {
    assert_eq!(
        Query::select()
            .columns([Glyph::Aspect])
            .from(Glyph::Table)
            .and_where(Expr::expr(Expr::col(Glyph::Aspect).if_null(0)).gt(2))
            .order_by_columns_with_nulls([
                (Glyph::Id, Order::Asc, NullOrdering::First),
                (Glyph::Aspect, Order::Desc, NullOrdering::Last),
            ])
            .to_string(),
        [
            r#"SELECT "aspect""#,
            r#"FROM "glyph""#,
            r#"WHERE COALESCE("aspect", 0) > 2"#,
            r#"ORDER BY "id" ASC NULLS FIRST,"#,
            r#""aspect" DESC NULLS LAST"#,
        ]
        .join(" ")
    );
}

#[test]
fn select_53() {
    assert_eq!(
        Query::select()
            .columns([Glyph::Aspect])
            .from(Glyph::Table)
            .and_where(Expr::expr(Expr::col(Glyph::Aspect).if_null(0)).gt(2))
            .order_by_columns_with_nulls([
                ((Glyph::Table, Glyph::Id), Order::Asc, NullOrdering::First),
                (
                    (Glyph::Table, Glyph::Aspect),
                    Order::Desc,
                    NullOrdering::Last
                ),
            ])
            .to_string(),
        [
            r#"SELECT "aspect""#,
            r#"FROM "glyph""#,
            r#"WHERE COALESCE("aspect", 0) > 2"#,
            r#"ORDER BY "glyph"."id" ASC NULLS FIRST,"#,
            r#""glyph"."aspect" DESC NULLS LAST"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ast.select.projection+2/test]
#[test]
fn select_54() {
    assert_eq!(
        Query::select()
            .distinct_on([Glyph::Aspect])
            .columns([Glyph::Aspect])
            .from(Glyph::Table)
            .and_where(Expr::expr(Expr::col(Glyph::Aspect).if_null(0)).gt(2))
            .order_by_columns_with_nulls([
                ((Glyph::Table, Glyph::Id), Order::Asc, NullOrdering::First),
                (
                    (Glyph::Table, Glyph::Aspect),
                    Order::Desc,
                    NullOrdering::Last
                ),
            ])
            .to_string(),
        [
            r#"SELECT DISTINCT ON ("aspect") "aspect""#,
            r#"FROM "glyph""#,
            r#"WHERE COALESCE("aspect", 0) > 2"#,
            r#"ORDER BY "glyph"."id" ASC NULLS FIRST,"#,
            r#""glyph"."aspect" DESC NULLS LAST"#,
        ]
        .join(" ")
    );
}

#[test]
fn select_55() {
    let statement = Query::select()
        .column(Asterisk)
        .from(Char::Table)
        .from(Font::Table)
        .and_where(Expr::col((Font::Table, Font::Id)).equals((Char::Table, Char::FontId)))
        .to_string();

    assert_eq!(
        statement,
        r#"SELECT * FROM "character", "font" WHERE "font"."id" = "character"."font_id""#
    );
}

// [spec:pgorm:req:sql.render.select-order+7/test]
#[test]
fn select_56() {
    assert_eq!(
        Query::select()
            .columns([Glyph::Aspect])
            .from(Glyph::Table)
            .and_where(Expr::expr(Expr::col(Glyph::Aspect).if_null(0)).gt(2))
            .order_by(
                Glyph::Id,
                Order::Field(Values(vec![
                    Value::Int(Some(4)),
                    Value::Int(Some(5)),
                    Value::Int(Some(1)),
                    Value::Int(Some(3))
                ]))
            )
            .order_by((Glyph::Table, Glyph::Aspect), Order::Asc)
            .to_string(),
        [
            r#"SELECT "aspect""#,
            r#"FROM "glyph""#,
            r#"WHERE COALESCE("aspect", 0) > 2"#,
            r#"ORDER BY CASE"#,
            r#"WHEN "id"=4 THEN 0"#,
            r#"WHEN "id"=5 THEN 1"#,
            r#"WHEN "id"=1 THEN 2"#,
            r#"WHEN "id"=3 THEN 3"#,
            r#"ELSE 4 END,"#,
            r#""glyph"."aspect" ASC"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ast.order+3/test]
#[test]
fn select_57() {
    assert_eq!(
        Query::select()
            .columns([Glyph::Aspect])
            .from(Glyph::Table)
            .and_where(Expr::expr(Expr::col(Glyph::Aspect).if_null(0)).gt(2))
            .order_by((Glyph::Table, Glyph::Aspect), Order::Asc)
            .order_by(
                Glyph::Id,
                Order::Field(Values(vec![
                    Value::Int(Some(4)),
                    Value::Int(Some(5)),
                    Value::Int(Some(1)),
                    Value::Int(Some(3))
                ]))
            )
            .to_string(),
        [
            r#"SELECT "aspect""#,
            r#"FROM "glyph""#,
            r#"WHERE COALESCE("aspect", 0) > 2"#,
            r#"ORDER BY "glyph"."aspect" ASC,"#,
            r#"CASE WHEN "id"=4 THEN 0"#,
            r#"WHEN "id"=5 THEN 1"#,
            r#"WHEN "id"=1 THEN 2"#,
            r#"WHEN "id"=3 THEN 3"#,
            r#"ELSE 4 END"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:def:sql.ast.with+5/test]
// [spec:pgorm:req:sql.render.cte+5/test]
#[test]
fn select_58() {
    let select = SelectStatement::new()
        .columns([Glyph::Id, Glyph::Image, Glyph::Aspect])
        .from(Glyph::Table)
        .to_owned();
    let cte = CommonTableExpression::new(Name::runtime("cte"), select);
    let with_clause = WithClause::new(cte);
    let mut select = SelectStatement::new()
        .columns([Glyph::Id, Glyph::Image, Glyph::Aspect])
        .from(Name::runtime("cte"))
        .to_owned();
    assert_eq!(
        select.with(with_clause).to_string(),
        [
            r#"WITH "cte" AS"#,
            r#"(SELECT "id", "image", "aspect""#,
            r#"FROM "glyph")"#,
            r#"SELECT "id", "image", "aspect" FROM "cte""#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:def:sql.ast.case+3/test]
#[test]
fn select_59() {
    let query = Query::select()
        .expr_as(
            Expr::case(Expr::col((Glyph::Table, Glyph::Aspect)).gt(0), "positive")
                .case(Expr::col((Glyph::Table, Glyph::Aspect)).lt(0), "negative")
                .finally("zero"),
            Name::runtime("polarity"),
        )
        .from(Glyph::Table)
        .to_owned();

    assert_eq!(
        query.to_string(),
        r#"SELECT (CASE WHEN ("glyph"."aspect" > 0) THEN 'positive' WHEN ("glyph"."aspect" < 0) THEN 'negative' ELSE 'zero' END) AS "polarity" FROM "glyph""#
    );
}

// [spec:pgorm:req:sql.ast.build+3/test]
// [spec:pgorm:req:sql.render.placeholders+1/test]
// [spec:pgorm:req:sql.render.param-vs-inline+1/test]
// [spec:pgorm:req:sql.render.custom-expr+3/test]
#[test]
fn select_60() {
    let (cust_query, cust_values) = Query::select()
        .column(Character::Id)
        .from(Character::Table)
        .and_where(Expr::col(Character::FontSize).eq(3))
        .build();

    let (statement, values) = Query::select()
        .expr(Expr::template(&cust_query[7..], cust_values.0).expect("template arity"))
        .limit(5)
        .build();

    assert_eq!(
        statement,
        r#"SELECT "id" FROM "character" WHERE "font_size" = $1 LIMIT $2"#
    );
    assert_eq!(values, Values(vec![3i32.into(), 5u64.into()]));
}

// [spec:pgorm:req:sql.ast.expr.operators+6/test]
#[test]
fn select_61() {
    assert_eq!(
        Query::select()
            .column(Char::Character)
            .from(Char::Table)
            .and_where(Expr::col(Char::Character).like(LikeExpr::new("A").escape('\\')))
            .build(),
        (
            r#"SELECT "character" FROM "character" WHERE "character" LIKE $1 ESCAPE E'\\'"#
                .to_owned(),
            Values(vec!["A".into()])
        )
    );
}

// [spec:pgorm:req:sql.ast.select.from+4/test]
// [spec:pgorm:req:sql.render.subquery+5/test]
#[test]
fn select_62() {
    let select = SelectStatement::new()
        .column(Asterisk)
        .from_values([(1i32, "hello"), (2, "world")], Name::runtime("x"))
        .to_owned();
    let cte = CommonTableExpression::new(Name::runtime("cte"), select);
    let with_clause = WithClause::new(cte);
    let mut select = SelectStatement::new()
        .columns([Name::runtime("column1"), Name::runtime("column2")])
        .from(Name::runtime("cte"))
        .to_owned();
    assert_eq!(
        select.with(with_clause).to_string(),
        [
            r#"WITH "cte" AS"#,
            r#"(SELECT * FROM (VALUES (1, 'hello'), (2, 'world')) AS "x")"#,
            r#"SELECT "column1", "column2""#,
            r#"FROM "cte""#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:def:sql.ast.insert+3/test]
// [spec:pgorm:req:sql.render.insert+2/test]
// [spec:pgorm:def:sql.render.value-literals+5/test]
#[test]
#[allow(clippy::approx_constant)]
fn insert_2() {
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Image, Glyph::Aspect])
            .values_panic([
                "04108048005887010020060000204E0180400400".into(),
                3.1415.into(),
            ])
            .to_string(),
        r#"INSERT INTO "glyph" ("image", "aspect") VALUES ('04108048005887010020060000204E0180400400', 3.1415)"#
    );
}

#[test]
#[allow(clippy::approx_constant)]
fn insert_3() {
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Image, Glyph::Aspect])
            .values_panic([
                "04108048005887010020060000204E0180400400".into(),
                3.1415.into(),
            ])
            .values_panic([Value::String(None).into(), 2.1345.into()])
            .to_string(),
        r#"INSERT INTO "glyph" ("image", "aspect") VALUES ('04108048005887010020060000204E0180400400', 3.1415), (NULL, 2.1345)"#
    );
}

#[test]

fn insert_4() {
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Image])
            .values_panic([jiff::civil::datetime(1970, 1, 1, 0, 0, 0, 0).into()])
            .to_string(),
        "INSERT INTO \"glyph\" (\"image\") VALUES ('1970-01-01 00:00:00')"
    );
}

#[test]

fn insert_5() {
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Image])
            .values_panic([uuid::Uuid::nil().into()])
            .to_string(),
        "INSERT INTO \"glyph\" (\"image\") VALUES ('00000000-0000-0000-0000-000000000000')"
    );
}

#[test]
fn insert_from_select() {
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .or_default_values()
            .columns([Glyph::Aspect, Glyph::Image])
            .select_from(
                Query::select()
                    .column(Glyph::Aspect)
                    .column(Glyph::Image)
                    .from(Glyph::Table)
                    .conditions(
                        true,
                        |x| {
                            x.and_where(Expr::col(Glyph::Image).like("%"));
                        },
                        |x| {
                            x.and_where(Expr::col(Glyph::Id).eq(6));
                        },
                    )
                    .to_owned()
            )
            .unwrap()
            .to_owned()
            .to_string(),
        r#"INSERT INTO "glyph" ("aspect", "image") SELECT "aspect", "image" FROM "glyph" WHERE "image" LIKE '%'"#
    );
}

// [spec:pgorm:def:sql.ast.with+5/test]
#[test]
fn insert_6() -> error::Result<()> {
    let select = SelectStatement::new()
        .columns([Glyph::Id, Glyph::Image, Glyph::Aspect])
        .from(Glyph::Table)
        .to_owned();
    let cte = CommonTableExpression::new(Name::runtime("cte"), select)
        .column(Glyph::Id)
        .column(Glyph::Image)
        .column(Glyph::Aspect)
        .to_owned();
    let with_clause = WithClause::new(cte);
    let select = SelectStatement::new()
        .columns([Glyph::Id, Glyph::Image, Glyph::Aspect])
        .from(Name::runtime("cte"))
        .to_owned();
    let mut insert = Query::insert();
    insert
        .into_table(Glyph::Table)
        .columns([Glyph::Id, Glyph::Image, Glyph::Aspect])
        .select_from(select)?;
    let sql = insert.with(with_clause).to_string();
    assert_eq!(
        sql.as_str(),
        [
            r#"WITH "cte" ("id", "image", "aspect") AS (SELECT "id", "image", "aspect" FROM "glyph")"#,
            r#"INSERT INTO "glyph" ("id", "image", "aspect") SELECT "id", "image", "aspect" FROM "cte""#,
        ].join(" ")
    );
    Ok(())
}

#[test]
fn insert_7() {
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .or_default_values()
            .to_string(),
        r#"INSERT INTO "glyph" VALUES (DEFAULT)"#
    );
}

#[test]
fn insert_8() {
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .or_default_values()
            .returning_col(Glyph::Id)
            .to_string(),
        r#"INSERT INTO "glyph" VALUES (DEFAULT) RETURNING "id""#
    );
}

#[test]
#[allow(clippy::approx_constant)]
fn insert_10() {
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Aspect, Glyph::Tokens])
            .values_panic([
                3.1415.into(),
                vec![
                    "Token1".to_string(),
                    "Token2".to_string(),
                    "Token3".to_string()
                ]
                .into()
            ])
            .to_string(),
        r#"INSERT INTO "glyph" ("aspect", "tokens") VALUES (3.1415, ARRAY ['Token1','Token2','Token3'])"#
    );
}

// [spec:pgorm:req:sql.ast.on-conflict+4/test]
#[test]
#[allow(clippy::approx_constant)]
// [spec:pgorm:req:sql.render.on-conflict+2/test]
fn insert_on_conflict_1() {
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Aspect, Glyph::Image])
            .values_panic([
                "04108048005887010020060000204E0180400400".into(),
                3.1415.into(),
            ])
            .on_conflict(OnConflict::column(Glyph::Id).update_column(Glyph::Aspect))
            .to_string(),
        [
            r#"INSERT INTO "glyph" ("aspect", "image")"#,
            r#"VALUES ('04108048005887010020060000204E0180400400', 3.1415)"#,
            r#"ON CONFLICT ("id") DO UPDATE SET "aspect" = "excluded"."aspect""#,
        ]
        .join(" ")
    );
}

#[test]
#[allow(clippy::approx_constant)]
fn insert_on_conflict_2() {
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Aspect, Glyph::Image])
            .values_panic([
                "04108048005887010020060000204E0180400400".into(),
                3.1415.into(),
            ])
            .on_conflict(
                OnConflict::column(Glyph::Id)
                    .and_column(Glyph::Aspect)
                    .update_column(Glyph::Aspect)
                    .update_columns([Glyph::Image])
            )
            .to_string(),
        [
            r#"INSERT INTO "glyph" ("aspect", "image")"#,
            r#"VALUES ('04108048005887010020060000204E0180400400', 3.1415)"#,
            r#"ON CONFLICT ("id", "aspect") DO UPDATE SET "aspect" = "excluded"."aspect", "image" = "excluded"."image""#,
        ]
        .join(" ")
    );
}

#[test]
#[allow(clippy::approx_constant)]
fn insert_on_conflict_3() {
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Aspect, Glyph::Image])
            .values_panic([
                "04108048005887010020060000204E0180400400".into(),
                3.1415.into(),
            ])
            .on_conflict(
                OnConflict::column(Glyph::Id)
                    .and_column(Glyph::Aspect)
                    .value(
                        Glyph::Aspect,
                        Expr::val("04108048005887010020060000204E0180400400")
                    )
                    .values([(Glyph::Image, 3.1415.into())])
            )
            .to_string(),
        [
            r#"INSERT INTO "glyph" ("aspect", "image")"#,
            r#"VALUES ('04108048005887010020060000204E0180400400', 3.1415)"#,
            r#"ON CONFLICT ("id", "aspect") DO UPDATE SET "aspect" = '04108048005887010020060000204E0180400400', "image" = 3.1415"#,
        ]
        .join(" ")
    );
}

#[test]
#[allow(clippy::approx_constant)]
fn insert_on_conflict_4() {
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Aspect, Glyph::Image])
            .values_panic([
                "04108048005887010020060000204E0180400400".into(),
                3.1415.into(),
            ])
            .on_conflict(
                OnConflict::column(Glyph::Id)
                    .and_column(Glyph::Aspect)
                    .value(Glyph::Image, Expr::val(1).add(2))
            )
            .to_string(),
        [
            r#"INSERT INTO "glyph" ("aspect", "image")"#,
            r#"VALUES ('04108048005887010020060000204E0180400400', 3.1415)"#,
            r#"ON CONFLICT ("id", "aspect") DO UPDATE SET "image" = 1 + 2"#,
        ]
        .join(" ")
    );
}

#[test]
#[allow(clippy::approx_constant)]
fn insert_on_conflict_5() {
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Aspect, Glyph::Image])
            .values_panic([
                "04108048005887010020060000204E0180400400".into(),
                3.1415.into(),
            ])
            .on_conflict(
                OnConflict::column(Glyph::Id)
                    .and_column(Glyph::Aspect)
                    .value(
                        Glyph::Aspect,
                        Expr::val("04108048005887010020060000204E0180400400")
                    )
                    .update_column(Glyph::Image)
            )
            .to_string(),
        [
            r#"INSERT INTO "glyph" ("aspect", "image")"#,
            r#"VALUES ('04108048005887010020060000204E0180400400', 3.1415)"#,
            r#"ON CONFLICT ("id", "aspect") DO UPDATE SET "aspect" = '04108048005887010020060000204E0180400400', "image" = "excluded"."image""#,
        ]
        .join(" ")
    );
}

#[test]
#[allow(clippy::approx_constant)]
fn insert_on_conflict_6() {
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Aspect, Glyph::Image])
            .values_panic([
                "04108048005887010020060000204E0180400400".into(),
                3.1415.into(),
            ])
            .on_conflict(
                OnConflict::column(Glyph::Id)
                    .and_column(Glyph::Aspect)
                    .update_column(Glyph::Aspect)
                    .value(Glyph::Image, Expr::val(1).add(2))
            )
            .to_string(),
        [
            r#"INSERT INTO "glyph" ("aspect", "image")"#,
            r#"VALUES ('04108048005887010020060000204E0180400400', 3.1415)"#,
            r#"ON CONFLICT ("id", "aspect") DO UPDATE SET "aspect" = "excluded"."aspect", "image" = 1 + 2"#,
        ]
        .join(" ")
    );
}

#[test]
#[allow(clippy::approx_constant)]
fn insert_on_conflict_7() {
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Aspect, Glyph::Image])
            .values_panic([
                "04108048005887010020060000204E0180400400".into(),
                3.1415.into(),
            ])
            .on_conflict(OnConflict::expr(Expr::col(Glyph::Id)).update_column(Glyph::Aspect))
            .to_string(),
        [
            r#"INSERT INTO "glyph" ("aspect", "image")"#,
            r#"VALUES ('04108048005887010020060000204E0180400400', 3.1415)"#,
            r#"ON CONFLICT ("id") DO UPDATE SET "aspect" = "excluded"."aspect""#,
        ]
        .join(" ")
    );
}

#[test]
#[allow(clippy::approx_constant)]
fn insert_on_conflict_8() {
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Aspect, Glyph::Image])
            .values_panic([
                "04108048005887010020060000204E0180400400".into(),
                3.1415.into(),
            ])
            .on_conflict(
                OnConflict::expr(Expr::col(Glyph::Id))
                    .and_exprs([Expr::col(Glyph::Aspect)])
                    .update_column(Glyph::Aspect)
            )
            .to_string(),
        [
            r#"INSERT INTO "glyph" ("aspect", "image")"#,
            r#"VALUES ('04108048005887010020060000204E0180400400', 3.1415)"#,
            r#"ON CONFLICT ("id", "aspect") DO UPDATE SET "aspect" = "excluded"."aspect""#,
        ]
        .join(" ")
    );
}

#[test]
#[allow(clippy::approx_constant)]
fn insert_on_conflict_9() {
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Aspect, Glyph::Image])
            .values_panic([
                "04108048005887010020060000204E0180400400".into(),
                3.1415.into(),
            ])
            .on_conflict(
                OnConflict::column(Glyph::Id)
                    .and_expr(Func::lower(Expr::col(Glyph::Tokens)))
                    .update_column(Glyph::Aspect)
            )
            .to_string(),
        [
            r#"INSERT INTO "glyph" ("aspect", "image")"#,
            r#"VALUES ('04108048005887010020060000204E0180400400', 3.1415)"#,
            r#"ON CONFLICT ("id", LOWER("tokens")) DO UPDATE SET "aspect" = "excluded"."aspect""#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ast.on-conflict+4/test]
#[test]
#[allow(clippy::approx_constant)]
fn insert_on_conflict_do_nothing() {
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Aspect, Glyph::Image])
            .values_panic(["abcd".into(), 3.1415.into()])
            .on_conflict(
                OnConflict::column(Glyph::Id)
                    .and_column(Glyph::Aspect)
                    .do_nothing()
            )
            .to_string(),
        [
            r#"INSERT INTO "glyph" ("aspect", "image")"#,
            r#"VALUES ('abcd', 3.1415)"#,
            r#"ON CONFLICT ("id", "aspect") DO NOTHING"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ast.on-conflict+4/test]
// [spec:pgorm:req:sql.render.on-conflict+2/test]
#[test]
#[allow(clippy::approx_constant)]
fn insert_on_conflict_bare_do_nothing() {
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Aspect, Glyph::Image])
            .values_panic(["abcd".into(), 3.1415.into()])
            .on_conflict(OnConflict::do_nothing())
            .to_string(),
        [
            r#"INSERT INTO "glyph" ("aspect", "image")"#,
            r#"VALUES ('abcd', 3.1415)"#,
            r#"ON CONFLICT DO NOTHING"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ast.on-conflict+4/test]
// [spec:pgorm:req:sql.render.on-conflict+2/test]
#[test]
fn insert_on_conflict_both_filters() {
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Aspect])
            .values_panic([1.into()])
            .on_conflict(
                OnConflict::column(Glyph::Id)
                    .and_where(Expr::col(Glyph::Aspect).is_null())
                    .update_column(Glyph::Aspect)
                    .and_where(Expr::col(Glyph::Image).gt(0))
            )
            .to_string(),
        [
            r#"INSERT INTO "glyph" ("aspect") VALUES (1)"#,
            r#"ON CONFLICT ("id") WHERE "aspect" IS NULL"#,
            r#"DO UPDATE SET "aspect" = "excluded"."aspect" WHERE "image" > 0"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:def:sql.ast.returning+3/test]
#[test]
#[allow(clippy::approx_constant)]
// [spec:pgorm:req:sql.render.returning+4/test]
fn insert_returning_all_columns() {
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Image, Glyph::Aspect])
            .values_panic([
                "04108048005887010020060000204E0180400400".into(),
                3.1415.into(),
            ])
            .returning(Query::returning().all())
            .to_string(),
        r#"INSERT INTO "glyph" ("image", "aspect") VALUES ('04108048005887010020060000204E0180400400', 3.1415) RETURNING *"#
    );
}

#[test]
#[allow(clippy::approx_constant)]
fn insert_returning_specific_columns() {
    assert_eq!(
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Image, Glyph::Aspect])
            .values_panic([
                "04108048005887010020060000204E0180400400".into(),
                3.1415.into(),
            ])
            .returning(Query::returning().columns([Glyph::Id, Glyph::Image]))
            .to_string(),
        r#"INSERT INTO "glyph" ("image", "aspect") VALUES ('04108048005887010020060000204E0180400400', 3.1415) RETURNING "id", "image""#
    );
}

// [spec:pgorm:req:sql.ast.update+5/test]
#[test]
// [spec:pgorm:req:sql.render.update-delete+3/test]
fn update_1() {
    assert_eq!(
        Query::update()
            .table(Glyph::Table)
            .values([
                (Glyph::Aspect, 2.1345.into()),
                (
                    Glyph::Image,
                    "24B0E11951B03B07F8300FD003983F03F0780060".into()
                ),
            ])
            .and_where(Expr::col(Glyph::Id).eq(1))
            .to_string(),
        r#"UPDATE "glyph" SET "aspect" = 2.1345, "image" = '24B0E11951B03B07F8300FD003983F03F0780060' WHERE "id" = 1"#
    );
}

#[test]
fn update_3() {
    assert_eq!(
        Query::update()
            .table(Glyph::Table)
            .value(Glyph::Aspect, Expr::raw("60 * 24 * 24"))
            .values([(
                Glyph::Image,
                "24B0E11951B03B07F8300FD003983F03F0780060".into()
            )])
            .and_where(Expr::col(Glyph::Id).eq(1))
            .to_string(),
        r#"UPDATE "glyph" SET "aspect" = 60 * 24 * 24, "image" = '24B0E11951B03B07F8300FD003983F03F0780060' WHERE "id" = 1"#
    );
}

#[test]
fn update_4() {
    assert_eq!(
        Query::update()
            .table(Glyph::Table)
            .value(Glyph::Aspect, Expr::col(Glyph::Aspect).add(1))
            .values([(
                Glyph::Image,
                "24B0E11951B03B07F8300FD003983F03F0780060".into()
            )])
            .and_where(Expr::col(Glyph::Id).eq(1))
            .to_string(),
        r#"UPDATE "glyph" SET "aspect" = "aspect" + 1, "image" = '24B0E11951B03B07F8300FD003983F03F0780060' WHERE "id" = 1"#
    );
}

#[test]
fn update_returning_all_columns() {
    assert_eq!(
        Query::update()
            .table(Glyph::Table)
            .value(Glyph::Aspect, Expr::raw("60 * 24 * 24"))
            .values([(
                Glyph::Image,
                "24B0E11951B03B07F8300FD003983F03F0780060".into()
            )])
            .and_where(Expr::col(Glyph::Id).eq(1))
            .returning(Query::returning().all())
            .to_string(),
        r#"UPDATE "glyph" SET "aspect" = 60 * 24 * 24, "image" = '24B0E11951B03B07F8300FD003983F03F0780060' WHERE "id" = 1 RETURNING *"#
    );
}

#[test]
fn update_returning_specified_columns() {
    assert_eq!(
        Query::update()
            .table(Glyph::Table)
            .value(Glyph::Aspect, Expr::raw("60 * 24 * 24"))
            .values([(
                Glyph::Image,
                "24B0E11951B03B07F8300FD003983F03F0780060".into()
            )])
            .and_where(Expr::col(Glyph::Id).eq(1))
            .returning(Query::returning().columns([Glyph::Id, Glyph::Image]))
            .to_string(),
        r#"UPDATE "glyph" SET "aspect" = 60 * 24 * 24, "image" = '24B0E11951B03B07F8300FD003983F03F0780060' WHERE "id" = 1 RETURNING "id", "image""#
    );
}

// [spec:pgorm:def:sql.ast.delete+4/test]
#[test]
// [spec:pgorm:req:sql.render.update-delete+3/test]
fn delete_1() {
    assert_eq!(
        Query::delete()
            .from_table(Glyph::Table)
            .and_where(Expr::col(Glyph::Id).eq(1))
            .to_string(),
        r#"DELETE FROM "glyph" WHERE "id" = 1"#
    );
}

#[test]
// [spec:pgorm:req:sql.render.string-escape+1/test]    a double quote is a
// legal string character, not an escape: it passes through raw
fn escape_1() {
    let test = r#" "abc" "#;
    assert_eq!(QueryBuilder.escape_string(test), test.to_owned());
    assert_eq!(
        QueryBuilder.unescape_string(QueryBuilder.escape_string(test).as_str()),
        test
    )
}

#[test]
fn escape_2() {
    let test = "a\nb\tc";
    assert_eq!(QueryBuilder.escape_string(test), "a\\nb\\tc".to_owned());
    assert_eq!(
        QueryBuilder.unescape_string(QueryBuilder.escape_string(test).as_str()),
        test
    );
}

#[test]
fn escape_3() {
    let test = "a\\b";
    assert_eq!(QueryBuilder.escape_string(test), "a\\\\b".to_owned());
    assert_eq!(
        QueryBuilder.unescape_string(QueryBuilder.escape_string(test).as_str()),
        test
    );
}

#[test]
fn escape_4() {
    let test = "a\"b";
    assert_eq!(QueryBuilder.escape_string(test), test.to_owned());
    assert_eq!(
        QueryBuilder.unescape_string(QueryBuilder.escape_string(test).as_str()),
        test
    )
}

#[test]
// [spec:pgorm:req:sql.render.string-escape+1/test]    0x1A is a legal string
// character; the retired MySQL \z mapping turned it into the letter z
fn escape_5() {
    let test = "left\x1aright";
    assert_eq!(QueryBuilder.escape_string(test), test.to_owned());
    assert_eq!(
        QueryBuilder.unescape_string(QueryBuilder.escape_string(test).as_str()),
        test
    )
}

#[test]
fn delete_returning_all_columns() {
    assert_eq!(
        Query::delete()
            .from_table(Glyph::Table)
            .and_where(Expr::col(Glyph::Id).eq(1))
            .returning(Query::returning().all())
            .to_string(),
        r#"DELETE FROM "glyph" WHERE "id" = 1 RETURNING *"#
    );
}

#[test]
fn delete_returning_specific_columns() {
    assert_eq!(
        Query::delete()
            .from_table(Glyph::Table)
            .and_where(Expr::col(Glyph::Id).eq(1))
            .returning(Query::returning().columns([Glyph::Id, Glyph::Image]))
            .to_string(),
        r#"DELETE FROM "glyph" WHERE "id" = 1 RETURNING "id", "image""#
    );
}

#[test]
fn delete_returning_specific_exprs() {
    assert_eq!(
        Query::delete()
            .from_table(Glyph::Table)
            .and_where(Expr::col(Glyph::Id).eq(1))
            .returning(Query::returning().exprs([Expr::col(Glyph::Id), Expr::col(Glyph::Image)]))
            .to_string(),
        r#"DELETE FROM "glyph" WHERE "id" = 1 RETURNING "id", "image""#
    );
}

#[test]
// [spec:pgorm:def:sql.render.operators+8/test]
fn select_pgtrgm_similarity() {
    assert_eq!(
        Query::select()
            .expr(Expr::col(Font::Name).binary(BinOper::Similarity, Expr::value("serif")))
            .from(Font::Table)
            .to_string(),
        r#"SELECT "name" % 'serif' FROM "font""#
    );
}

#[test]
fn select_pgtrgm_word_similarity() {
    assert_eq!(
        Query::select()
            .expr(Expr::col(Font::Name).binary(BinOper::WordSimilarity, Expr::value("serif")))
            .from(Font::Table)
            .to_string(),
        r#"SELECT "name" <% 'serif' FROM "font""#
    );
}

#[test]
fn select_pgtrgm_strict_word_similarity() {
    assert_eq!(
        Query::select()
            .expr(Expr::col(Font::Name).binary(BinOper::StrictWordSimilarity, Expr::value("serif")))
            .from(Font::Table)
            .to_string(),
        r#"SELECT "name" <<% 'serif' FROM "font""#
    );
}

#[test]
fn select_pgtrgm_similarity_distance() {
    assert_eq!(
        Query::select()
            .expr(Expr::col(Font::Name).binary(BinOper::SimilarityDistance, Expr::value("serif")))
            .from(Font::Table)
            .to_string(),
        r#"SELECT "name" <-> 'serif' FROM "font""#
    );
}

#[test]
fn select_pgtrgm_word_similarity_distance() {
    assert_eq!(
        Query::select()
            .expr(
                Expr::col(Font::Name).binary(BinOper::WordSimilarityDistance, Expr::value("serif"))
            )
            .from(Font::Table)
            .to_string(),
        r#"SELECT "name" <<-> 'serif' FROM "font""#
    );
}

#[test]
fn select_pgtrgm_strict_word_similarity_distance() {
    assert_eq!(
        Query::select()
            .expr(
                Expr::col(Font::Name)
                    .binary(BinOper::StrictWordSimilarityDistance, Expr::value("serif"))
            )
            .from(Font::Table)
            .to_string(),
        r#"SELECT "name" <<<-> 'serif' FROM "font""#
    );
}

#[test]
fn select_custom_operator() {
    assert_eq!(
        Query::select()
            .expr(Expr::col(Font::Name).binary(BinOper::Raw("~*"), Expr::value("serif")))
            .from(Font::Table)
            .to_string(),
        r#"SELECT "name" ~* 'serif' FROM "font""#
    );
    assert_eq!(
        Query::select()
            .expr(Expr::col(Font::Name).binary(BinOper::Raw("~"), Expr::value("serif")))
            .from(Font::Table)
            .to_string(),
        r#"SELECT "name" ~ 'serif' FROM "font""#
    );
}

// [spec:pgorm:sem:sql.ast.select.union+2/test]
#[test]
fn union_1() {
    assert_eq!(
        Query::select()
            .column(Char::Character)
            .from(Char::Table)
            .union(
                UnionType::Distinct,
                Query::select()
                    .column(Char::Character)
                    .from(Char::Table)
                    .left_join(
                        Font::Table,
                        Expr::col((Char::Table, Char::FontId)).equals((Font::Table, Font::Id))
                    )
                    .order_by((Font::Table, Font::Id), Order::Asc)
                    .take()
            )
            .to_string(),
        [
            r#"SELECT "character" FROM "character" UNION (SELECT "character" FROM "character""#,
            r#"LEFT JOIN "font" ON "character"."font_id" = "font"."id" ORDER BY "font"."id" ASC)"#
        ]
        .join(" ")
    );
}

const SET_OPERATIONS: [UnionType; 6] = [
    UnionType::Distinct,
    UnionType::All,
    UnionType::Intersect,
    UnionType::IntersectAll,
    UnionType::Except,
    UnionType::ExceptAll,
];

fn arm(table: &str) -> SelectStatement {
    Query::select()
        .column(Name::runtime("v"))
        .from(Name::runtime(table))
        .take()
}

/// A set operation as the parse tree records it on a `SelectStmt`: its `op`
/// and whether it keeps duplicates.
fn parsed_operation(union: UnionType) -> (i64, bool) {
    use pg_query::protobuf::SetOperation;
    let (op, all) = match union {
        UnionType::Distinct => (SetOperation::SetopUnion, false),
        UnionType::All => (SetOperation::SetopUnion, true),
        UnionType::Intersect => (SetOperation::SetopIntersect, false),
        UnionType::IntersectAll => (SetOperation::SetopIntersect, true),
        UnionType::Except => (SetOperation::SetopExcept, false),
        UnionType::ExceptAll => (SetOperation::SetopExcept, true),
        other => panic!("no parse-tree set operation is known for {other:?}"),
    };
    (op as i64, all)
}

/// The set operation a parsed `SelectStmt` performs, and its left and right
/// operands.
fn operation_of(node: &serde_json::Value) -> ((i64, bool), &serde_json::Value, &serde_json::Value) {
    let op = node["op"]
        .as_i64()
        .unwrap_or_else(|| panic!("no op in {node:#}"));
    let all = node["all"].as_bool().unwrap_or(false);
    ((op, all), &node["larg"], &node["rarg"])
}

/// The tables a set operation's leaves read, left to right.
fn tables(node: &serde_json::Value) -> Vec<String> {
    match node["from_clause"][0]["RangeVar"]["relname"].as_str() {
        Some(name) if node.get("larg").is_none() => vec![name.to_owned()],
        _ => {
            let mut found = tables(&node["larg"]);
            found.extend(tables(&node["rarg"]));
            found
        }
    }
}

// [spec:pgorm:sem:sql.ast.select.union+2/test]    for every pair of operators, the server's parser
// reads the chain a op1 b op2 c as (a op1 b) op2 c, and only an INTERSECT after a UNION or EXCEPT
// adds a parenthesis
// [spec:pgorm:req:sql.render.select-order+7/test]
#[test]
fn chained_set_operations_parse_in_call_order() {
    for first in SET_OPERATIONS {
        for second in SET_OPERATIONS {
            let sql = arm("a")
                .union(first, arm("b"))
                .union(second, arm("c"))
                .to_string();
            let regrouped = matches!(second, UnionType::Intersect | UnionType::IntersectAll)
                && !matches!(first, UnionType::Intersect | UnionType::IntersectAll);
            assert_eq!(
                sql.starts_with('('),
                regrouped,
                "{first:?} then {second:?}: {sql}"
            );

            let statements = parsed_nodes(&sql, "SelectStmt");
            let (outer, left, right) = operation_of(&statements[0]);
            assert_eq!(outer, parsed_operation(second), "{sql}");
            assert_eq!(tables(right), ["c"], "{sql}");
            let (inner, a, b) = operation_of(left);
            assert_eq!(inner, parsed_operation(first), "{sql}");
            assert_eq!(
                (tables(a), tables(b)),
                (vec!["a".to_owned()], vec!["b".to_owned()])
            );
        }
    }
}

// [spec:pgorm:sem:sql.ast.select.union+2/test]    a grouped prefix is one operand, so a longer
// chain closes one parenthesis per INTERSECT that follows a UNION or EXCEPT, and none elsewhere
// [spec:pgorm:req:sql.render.select-order+7/test]
#[test]
fn a_long_chain_groups_each_regrouping_arm() {
    let chain = arm("a")
        .union(UnionType::Distinct, arm("b"))
        .union(UnionType::Intersect, arm("c"))
        .union(UnionType::IntersectAll, arm("d"))
        .union(UnionType::ExceptAll, arm("e"))
        .union(UnionType::Intersect, arm("f"))
        .to_string();
    assert_query_eq(
        &chain,
        &[
            r#"((SELECT "v" FROM "a" UNION (SELECT "v" FROM "b"))"#,
            r#"INTERSECT (SELECT "v" FROM "c") INTERSECT ALL (SELECT "v" FROM "d")"#,
            r#"EXCEPT ALL (SELECT "v" FROM "e"))"#,
            r#"INTERSECT (SELECT "v" FROM "f")"#,
        ]
        .join(" "),
    );

    let statements = parsed_nodes(&chain, "SelectStmt");
    let (last, rest, f) = operation_of(&statements[0]);
    assert_eq!(last, parsed_operation(UnionType::Intersect));
    assert_eq!(tables(f), ["f"]);
    let (except, rest, e) = operation_of(rest);
    assert_eq!(except, parsed_operation(UnionType::ExceptAll));
    assert_eq!(tables(e), ["e"]);
    let (intersect_all, rest, d) = operation_of(rest);
    assert_eq!(intersect_all, parsed_operation(UnionType::IntersectAll));
    assert_eq!(tables(d), ["d"]);
    let (intersect, rest, c) = operation_of(rest);
    assert_eq!(intersect, parsed_operation(UnionType::Intersect));
    assert_eq!(tables(c), ["c"]);
    assert_eq!(tables(rest), ["a", "b"]);

    // UNION and EXCEPT after anything, and INTERSECT after INTERSECT, already
    // read left to right, so such a chain renders flat.
    assert_eq!(
        arm("a")
            .union(UnionType::Intersect, arm("b"))
            .union(UnionType::IntersectAll, arm("c"))
            .union(UnionType::All, arm("d"))
            .union(UnionType::Except, arm("e"))
            .to_string(),
        [
            r#"SELECT "v" FROM "a" INTERSECT (SELECT "v" FROM "b")"#,
            r#"INTERSECT ALL (SELECT "v" FROM "c") UNION ALL (SELECT "v" FROM "d")"#,
            r#"EXCEPT (SELECT "v" FROM "e")"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.render.select-order+7/test]    the WITH clause stays ahead of the opened
// parentheses, the receiver's ORDER BY and limit follow the last arm, and an arm keeps its own
// chain, ORDER BY and LIMIT inside its parentheses
#[test]
fn grouping_leaves_with_and_the_tail_outside() {
    let inner = arm("c")
        .union(UnionType::Except, arm("d"))
        .union(UnionType::Intersect, arm("e"))
        .order_by(Name::runtime("v"), Order::Desc)
        .limit(2)
        .take();
    let sql = arm("a")
        .union(UnionType::All, arm("b"))
        .union(UnionType::IntersectAll, inner)
        .with(WithClause::new(CommonTableExpression::new(
            Name::runtime("a"),
            arm("source"),
        )))
        .order_by(Name::runtime("v"), Order::Asc)
        .limit(5)
        .offset(1)
        .take()
        .to_string();
    assert_query_eq(
        &sql,
        &[
            r#"WITH "a" AS (SELECT "v" FROM "source")"#,
            r#"(SELECT "v" FROM "a" UNION ALL (SELECT "v" FROM "b"))"#,
            r#"INTERSECT ALL ((SELECT "v" FROM "c" EXCEPT (SELECT "v" FROM "d"))"#,
            r#"INTERSECT (SELECT "v" FROM "e") ORDER BY "v" DESC LIMIT 2)"#,
            r#"ORDER BY "v" ASC LIMIT 5 OFFSET 1"#,
        ]
        .join(" "),
    );
}

#[test]
fn select_array_contains_bin_oper() {
    assert_eq!(
        Query::select()
            .column(Char::Character)
            .from(Char::Table)
            .and_where(Expr::col(Char::Character).binary(BinOper::Contains, Expr::val("test")))
            .build(),
        (
            r#"SELECT "character" FROM "character" WHERE "character" @> $1"#.to_owned(),
            Values(vec!["test".into()])
        )
    );
}

#[test]
fn select_array_contained_bin_oper() {
    assert_eq!(
        Query::select()
            .column(Char::Character)
            .from(Char::Table)
            .and_where(Expr::col(Char::Character).binary(BinOper::Contained, Expr::val("test")))
            .build(),
        (
            r#"SELECT "character" FROM "character" WHERE "character" <@ $1"#.to_owned(),
            Values(vec!["test".into()])
        )
    );
}

#[test]
fn select_array_overlap_bin_oper() {
    assert_eq!(
        Query::select()
            .column(Char::Character)
            .from(Char::Table)
            .and_where(Expr::col(Char::Character).binary(BinOper::Overlap, Expr::val("test")))
            .build(),
        (
            r#"SELECT "character" FROM "character" WHERE "character" && $1"#.to_owned(),
            Values(vec!["test".into()])
        )
    );
}

// [spec:pgorm:req:sql.ast.expr.operators+6/test]
#[test]
fn get_json_field_bin_oper() {
    assert_eq!(
        Query::select()
            .column(Char::Character)
            .from(Char::Table)
            .and_where(Expr::col(Char::Character).binary(BinOper::GetJsonField, Expr::val("test")))
            .build(),
        (
            r#"SELECT "character" FROM "character" WHERE "character" -> $1"#.to_owned(),
            Values(vec!["test".into()])
        )
    );
}

#[test]
fn cast_json_field_bin_oper() {
    assert_eq!(
        Query::select()
            .column(Char::Character)
            .from(Char::Table)
            .and_where(Expr::col(Char::Character).binary(BinOper::CastJsonField, Expr::val("test")))
            .build(),
        (
            r#"SELECT "character" FROM "character" WHERE "character" ->> $1"#.to_owned(),
            Values(vec!["test".into()])
        )
    );
}

// [spec:pgorm:req:sql.ast.expr.json+3/test]    a path of any depth is one `text[]`
// parameter, so the statement text does not vary with it
#[test]
fn json_path_operators_take_one_text_array() {
    assert_eq!(
        Query::select()
            .column(Char::Id)
            .from(Char::Table)
            .and_where(
                Expr::expr(Expr::col(Char::UserData).get_json_path(["a", "b"])).is_not_null()
            )
            .to_string(),
        r#"SELECT "id" FROM "character" WHERE ("user_data" #> ARRAY ['a','b']) IS NOT NULL"#
    );

    assert_eq!(
        Query::select()
            .column(Char::Id)
            .from(Char::Table)
            .and_where(Expr::expr(Expr::col(Char::UserData).cast_json_path(["a"])).eq("x"))
            .build(),
        (
            r#"SELECT "id" FROM "character" WHERE ("user_data" #>> $1) = $2"#.to_owned(),
            Values(vec![Value::array(["a".to_owned()]), "x".into()])
        )
    );
}

// [spec:pgorm:req:sql.ast.expr.json+3/test]    the existence family's three lexemes,
// and the single-key form's text operand
#[test]
fn json_key_existence_operators_render() {
    assert_eq!(
        Query::select()
            .column(Char::Id)
            .from(Char::Table)
            .and_where(Expr::col(Char::UserData).has_json_key("a"))
            .build(),
        (
            r#"SELECT "id" FROM "character" WHERE "user_data" ? $1"#.to_owned(),
            Values(vec!["a".into()])
        )
    );

    assert_eq!(
        Query::select()
            .column(Char::Id)
            .from(Char::Table)
            .and_where(Expr::col(Char::UserData).has_any_json_keys(["a", "b"]))
            .to_string(),
        r#"SELECT "id" FROM "character" WHERE "user_data" ?| ARRAY ['a','b']"#
    );

    assert_eq!(
        Query::select()
            .column(Char::Id)
            .from(Char::Table)
            .and_where(Expr::col(Char::UserData).has_all_json_keys(["a", "b"]))
            .to_string(),
        r#"SELECT "id" FROM "character" WHERE "user_data" ?& ARRAY ['a','b']"#
    );
}

// [spec:pgorm:def:sql.render.precedence+14/test]    the existence operators return
// boolean, so a logical outer operator drops their parentheses the way it does
// for `@>`; the accessors return JSON or text and keep theirs
#[test]
fn json_existence_drops_parens_under_and() {
    assert_eq!(
        Query::select()
            .column(Char::Id)
            .from(Char::Table)
            .and_where(
                Expr::col(Char::UserData)
                    .has_json_key("a")
                    .and(Expr::col(Char::UserData).has_all_json_keys(["b", "c"]))
            )
            .to_string(),
        r#"SELECT "id" FROM "character" WHERE "user_data" ? 'a' AND "user_data" ?& ARRAY ['b','c']"#
    );

    assert_eq!(
        Query::select()
            .column(Char::Id)
            .from(Char::Table)
            .and_where(Expr::expr(Expr::col(Char::UserData).get_json_path(["a"])).is_null())
            .to_string(),
        r#"SELECT "id" FROM "character" WHERE ("user_data" #> ARRAY ['a']) IS NULL"#
    );
}

// [spec:pgorm:req:sql.ast.expr.json+3/test]    an empty key list is not special-cased:
// it is a typed empty array, and the statement text is the same at every cardinality
#[test]
fn empty_json_key_list_is_typed_array() {
    assert_eq!(
        Query::select()
            .column(Char::Id)
            .from(Char::Table)
            .and_where(Expr::col(Char::UserData).has_any_json_keys(Vec::<String>::new()))
            .to_string(),
        r#"SELECT "id" FROM "character" WHERE "user_data" ?| ARRAY []::text[]"#
    );

    assert_eq!(
        Query::select()
            .column(Char::Id)
            .from(Char::Table)
            .and_where(Expr::col(Char::UserData).has_all_json_keys(Vec::<String>::new()))
            .build()
            .0,
        Query::select()
            .column(Char::Id)
            .from(Char::Table)
            .and_where(Expr::col(Char::UserData).has_all_json_keys(["a", "b"]))
            .build()
            .0
    );
}

// [spec:pgorm:req:sql.ast.expr.operators+6/test]    `@>`, `<@` and `||` are one
// operator each across every type that has them, so the JSON family names no
// duplicate: these are already the JSON containment and merge tests
#[test]
fn containment_and_concat_serve_json_operands() {
    assert_eq!(
        Query::select()
            .column(Char::Id)
            .from(Char::Table)
            .and_where(Expr::col(Char::UserData).contains(json!({ "a": 1 })))
            .build(),
        (
            r#"SELECT "id" FROM "character" WHERE "user_data" @> $1"#.to_owned(),
            Values(vec![json!({ "a": 1 }).into()])
        )
    );

    assert_eq!(
        Query::select()
            .column(Char::Id)
            .from(Char::Table)
            .and_where(
                Expr::expr(Expr::col(Char::UserData).concat(json!({ "b": 2 }))).is_not_null()
            )
            .to_string(),
        r#"SELECT "id" FROM "character" WHERE ("user_data" || '{"b":2}') IS NOT NULL"#
    );
}

#[test]
fn regex_bin_oper() {
    assert_eq!(
        Query::select()
            .column(Char::Character)
            .from(Char::Table)
            .and_where(Expr::col(Char::Character).binary(BinOper::Regex, Expr::val("test")))
            .build(),
        (
            r#"SELECT "character" FROM "character" WHERE "character" ~ $1"#.to_owned(),
            Values(vec!["test".into()])
        )
    );
}

#[test]
fn regex_case_insensitive_bin_oper() {
    assert_eq!(
        Query::select()
            .column(Char::Character)
            .from(Char::Table)
            .and_where(
                Expr::col(Char::Character).binary(BinOper::RegexCaseInsensitive, Expr::val("test"))
            )
            .build(),
        (
            r#"SELECT "character" FROM "character" WHERE "character" ~* $1"#.to_owned(),
            Values(vec!["test".into()])
        )
    );
}

#[test]
// [spec:pgorm:req:sql.render.parens+5/test]
fn test_issue_674_nested_logical() {
    let t = SimpleExpr::Value(true.into());
    let f = SimpleExpr::Value(false.into());

    let x_op_y = |x, op, y| SimpleExpr::Binary(Box::new(x), op, Box::new(y));
    let t_or_t = x_op_y(t.clone(), BinOper::Or, t.clone());
    let t_or_t_or_f = x_op_y(t_or_t, BinOper::Or, f);
    let t_or_t_or_f_and_t = x_op_y(t_or_t_or_f.clone(), BinOper::And, t);

    assert_eq!(
        Query::select()
            .columns([Char::Character])
            .from(Char::Table)
            .and_where(t_or_t_or_f_and_t)
            .to_string(),
        r#"SELECT "character" FROM "character" WHERE (TRUE OR TRUE OR FALSE) AND TRUE"#
    );
}

#[test]
// [spec:pgorm:def:sql.render.precedence+14/test]
fn test_issue_674_nested_comparison() {
    let int100 = SimpleExpr::Value(100i32.into());
    let int0 = SimpleExpr::Value(0i32.into());
    let int1 = SimpleExpr::Value(1i32.into());

    let x_op_y = |x, op, y| SimpleExpr::Binary(Box::new(x), op, Box::new(y));
    let t_smaller_than_t = x_op_y(int100, BinOper::SmallerThan, int0);
    let t_smaller_than_t_smaller_than_f = x_op_y(t_smaller_than_t, BinOper::SmallerThan, int1);

    assert_eq!(
        Query::select()
            .columns([Char::Character])
            .from(Char::Table)
            .and_where(t_smaller_than_t_smaller_than_f)
            .to_string(),
        r#"SELECT "character" FROM "character" WHERE (100 < 0) < 1"#
    );
}

#[test]
fn test_issue_674_and_inside_not() {
    let t = SimpleExpr::Value(true.into());
    let f = SimpleExpr::Value(false.into());

    let op_x = |op, x| SimpleExpr::Unary(op, Box::new(x));
    let x_op_y = |x, op, y| SimpleExpr::Binary(Box::new(x), op, Box::new(y));
    let f_and_t = x_op_y(f, BinOper::And, t);
    let not_f_and_t = op_x(UnOper::Not, f_and_t);

    assert_eq!(
        Query::select()
            .columns([Char::Character])
            .from(Char::Table)
            .and_where(not_f_and_t)
            .to_string(),
        r#"SELECT "character" FROM "character" WHERE NOT (FALSE AND TRUE)"#
    );
}

// [spec:pgorm:sem:sql.ast.condition.flattening/test]
#[test]
fn test_issue_674_nested_logical_panic() {
    let e = SimpleExpr::from(true).and(SimpleExpr::from(true).and(true.into()).and(true.into()));

    assert_eq!(
        Query::select()
            .columns([Char::Character])
            .from(Char::Table)
            .and_where(e)
            .to_string(),
        r#"SELECT "character" FROM "character" WHERE TRUE AND (TRUE AND TRUE AND TRUE)"#
    );
}

#[test]
fn test_pgvector_select() {
    assert_eq!(
        Query::select()
            .columns([Char::Character])
            .from(Char::Table)
            .and_where(
                Expr::col(Char::Character).eq(Expr::val(pgvector::Vector::from(vec![1.0, 2.0])))
            )
            .to_string(),
        r#"SELECT "character" FROM "character" WHERE "character" = '[1,2]'"#
    );
}

// [spec:pgorm:req:sql.ast.cast-shape+2/test]    every cast spelling — `cast_as`, `cast_as_raw`,
// `as_enum`, `cast_as_type` and `as_range` on `Expr`, `cast_as` and `cast_as_type` on
// `SimpleExpr`, the quoted name and the verbatim type expression alike — is one `AsEnum`
// node differing only in its `TypeName`, and `as_range` writing a range value as its
// text first still builds that node
#[test]
fn every_cast_spelling_builds_one_node_shape() {
    let operand = Expr::col(Char::SizeW);
    let slot = || TypeName::new(Name::runtime("slot")).schema(Name::runtime("booking"));
    let casts = [
        operand.clone().cast_as(Name::runtime("citext")),
        operand.clone().cast_as_raw("numeric(10, 2)"),
        operand.clone().as_enum(Name::runtime("citext")),
        operand
            .clone()
            .cast_as_type(TypeName::new(Name::runtime("status")).schema(Name::runtime("tenant_a"))),
        operand.clone().as_range(slot()),
        SimpleExpr::from(operand.clone()).cast_as(Name::runtime("citext")),
        SimpleExpr::from(operand.clone()).cast_as_type(TypeName::new(Name::runtime("tea")).array()),
    ];

    let type_names: Vec<TypeName> = casts
        .iter()
        .map(|cast| match cast {
            SimpleExpr::AsEnum(type_name, cast_operand) => {
                assert_eq!(
                    **cast_operand,
                    SimpleExpr::from(operand.clone()),
                    "the operand rides untouched under every spelling"
                );
                TypeName::clone(type_name)
            }
            other => panic!("a cast that is not an `AsEnum`: {other:?}"),
        })
        .collect();

    assert_eq!(
        type_names,
        [
            TypeName::new(Name::runtime("citext")),
            TypeName::raw("numeric(10, 2)"),
            TypeName::new(Name::runtime("citext")),
            TypeName::new(Name::runtime("status")).schema(Name::runtime("tenant_a")),
            slot(),
            TypeName::new(Name::runtime("citext")),
            TypeName::new(Name::runtime("tea")).array(),
        ]
    );

    // A range or multirange value reaches a created range type only as its
    // text, so `as_range` rewrites that operand; the node stays the one cast.
    let text = |text: &str| SimpleExpr::Value(Value::String(Some(Box::new(text.to_owned()))));
    assert_eq!(
        Expr::val(Range::from(1..5)).as_range(slot()),
        SimpleExpr::AsEnum(Box::new(slot()), Box::new(text("[1,5)")))
    );
    assert_eq!(
        Expr::val(Multirange::from(vec![Range::from(1..5), Range::from(7..9)])).as_range(slot()),
        SimpleExpr::AsEnum(Box::new(slot()), Box::new(text("{[1,5),[7,9)}")))
    );
    assert_eq!(
        Expr::val(Value::Range(RangeType::Int4, None)).as_range(slot()),
        SimpleExpr::AsEnum(
            Box::new(slot()),
            Box::new(SimpleExpr::Value(Value::String(None)))
        )
    );

    // Quoted or verbatim is the `TypeName`'s answer, never the node's.
    assert_eq!(
        Query::select().expr(casts[0].clone()).to_string(),
        r#"SELECT CAST("size_w" AS citext)"#
    );
    assert_eq!(
        Query::select().expr(casts[1].clone()).to_string(),
        r#"SELECT CAST("size_w" AS numeric(10, 2))"#
    );
    assert_eq!(
        Query::select().expr(casts[3].clone()).to_string(),
        r#"SELECT CAST("size_w" AS tenant_a.status)"#
    );
}

// [spec:pgorm:req:sql.render.cast-param-type+4/test]
#[test]
fn cast_param_is_pinned_to_the_source_type() {
    assert_eq!(
        Query::select()
            .expr(Expr::val(8i64).cast_as_raw("BIT(8)"))
            .build(),
        (
            r#"SELECT CAST($1::int8 AS BIT(8))"#.to_owned(),
            Values(vec![8i64.into()])
        )
    );

    // Array-ness is structural, so the element name stays a bare identifier.
    assert_eq!(
        Query::select()
            .expr(
                Expr::val(vec!["a".to_owned()])
                    .cast_as_type(TypeName::new(Name::runtime("tea")).array())
            )
            .build(),
        (
            r#"SELECT CAST($1::text[] AS tea[])"#.to_owned(),
            Values(vec![vec!["a".to_owned()].into()])
        )
    );

    // Brackets written *into* the name are not an array spelling — they make it
    // an identifier no bare rendering can carry, so it is quoted.
    assert_eq!(
        Query::select()
            .expr(Expr::val(vec!["a".to_owned()]).cast_as(Name::runtime("tea[]")))
            .build()
            .0,
        r#"SELECT CAST($1::text[] AS "tea[]")"#.to_owned(),
    );

    assert_eq!(
        Query::select()
            .expr(Expr::val(json!({ "a": 1 })).cast_as(Name::runtime("jsonb")))
            .build(),
        (
            r#"SELECT CAST($1 AS jsonb)"#.to_owned(),
            Values(vec![json!({ "a": 1 }).into()])
        )
    );
}

// [spec:pgorm:req:sql.render.cast-param-type+4/test]
#[test]
fn cast_param_is_not_pinned_when_rendered_inline() {
    assert_eq!(
        Query::select()
            .expr(Expr::val(8i64).cast_as_raw("BIT(8)"))
            .to_string(),
        r#"SELECT CAST(8 AS BIT(8))"#
    );

    assert_eq!(
        Query::select()
            .expr(Expr::col(Char::SizeW).cast_as(Name::runtime("text")))
            .build()
            .0,
        r#"SELECT CAST("size_w" AS text)"#
    );
}

// [spec:pgorm:def:sql.ast.keywords+6/test]    the bare-keyword expressions and their constructors
#[test]
fn keywords_1() {
    assert_eq!(
        Query::select()
            .expr(Expr::current_date())
            .expr(Expr::current_time())
            .expr(Expr::current_timestamp())
            .expr(Keyword::Null)
            .expr(Expr::domain_value())
            .to_string(),
        "SELECT CURRENT_DATE, CURRENT_TIME, CURRENT_TIMESTAMP, NULL, VALUE"
    );
}

// [spec:pgorm:def:sql.ast.keywords+6/test]    `Name::runtime` wraps an arbitrary string as an
// identifier, and it is the only runtime-name helper — there is no empty-name alias
#[test]
fn keywords_2() {
    assert_eq!(
        Query::select()
            .expr_as(Expr::col(Glyph::Id), Name::runtime("an alias"))
            .expr_as(Expr::col(Glyph::Aspect), Name::runtime("ratio"))
            .from(Glyph::Table)
            .to_string(),
        r#"SELECT "id" AS "an alias", "aspect" AS "ratio" FROM "glyph""#
    );
}

// [spec:pgorm:req:sql.ast.condition.holder+2/test]    the holder's two states: absent emits no
// clause, present renders the condition
#[test]
fn condition_holder_1() {
    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .to_string(),
        r#"SELECT "id" FROM "glyph""#
    );

    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .cond_where(Condition::all().add(Expr::col(Glyph::Aspect).eq(1)))
            .to_string(),
        r#"SELECT "id" FROM "glyph" WHERE "aspect" = 1"#
    );
}

// [spec:pgorm:req:sql.ast.condition.holder+2/test]    `and_where` is a `cond_where` shorthand, so
// the two styles are interchangeable and freely interleaved
#[test]
fn condition_holder_2() {
    let expected = r#"SELECT "id" FROM "glyph" WHERE "aspect" = 1 AND "aspect" = 2"#;

    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .and_where(Expr::col(Glyph::Aspect).eq(1))
            .and_where(Expr::col(Glyph::Aspect).eq(2))
            .to_string(),
        expected
    );

    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .cond_where(Condition::all().add(Expr::col(Glyph::Aspect).eq(1)))
            .and_where(Expr::col(Glyph::Aspect).eq(2))
            .to_string(),
        expected
    );

    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .and_where(Expr::col(Glyph::Aspect).eq(1))
            .cond_where(Condition::all().add(Expr::col(Glyph::Aspect).eq(2)))
            .to_string(),
        expected
    );
}

// [spec:pgorm:req:sql.ast.condition.holder+2/test]    HAVING is backed by the same holder, with the
// same conjoining semantics
#[test]
fn condition_holder_3a() {
    assert_eq!(
        Query::select()
            .column(Glyph::Aspect)
            .from(Glyph::Table)
            .group_by_col(Glyph::Aspect)
            .cond_having(Condition::all().add(Expr::col(Glyph::Aspect).gt(1)))
            .and_having(Expr::col(Glyph::Aspect).lt(9))
            .cond_having(
                Condition::any()
                    .add(Expr::col(Glyph::Aspect).eq(3))
                    .add(Expr::col(Glyph::Aspect).eq(5))
            )
            .to_string(),
        [
            r#"SELECT "aspect" FROM "glyph" GROUP BY "aspect""#,
            r#"HAVING "aspect" > 1 AND "aspect" < 9"#,
            r#"AND ("aspect" = 3 OR "aspect" = 5)"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ast.condition.holder+2/test]    repeated `cond_where` conjoins: two
// non-negated `All` sets are appended flat, in call order
#[test]
fn condition_holder_4() {
    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .cond_where(
                Condition::all()
                    .add(Expr::col(Glyph::Aspect).eq(1))
                    .add(Expr::col(Glyph::Aspect).eq(2))
            )
            .cond_where(Condition::all().add(Expr::col(Glyph::Aspect).eq(3)))
            .to_string(),
        r#"SELECT "id" FROM "glyph" WHERE "aspect" = 1 AND "aspect" = 2 AND "aspect" = 3"#
    );
}

// [spec:pgorm:req:sql.ast.condition.holder+2/test]    anything else is combined under a fresh
// `Condition::all()`
#[test]
fn condition_holder_5() {
    // Current is a non-negated `All`, the addition is an `Any`: the addition is
    // nested inside the existing set.
    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .cond_where(Condition::all().add(Expr::col(Glyph::Aspect).eq(1)))
            .cond_where(
                Condition::any()
                    .add(Expr::col(Glyph::Aspect).eq(2))
                    .add(Expr::col(Glyph::Aspect).eq(3))
            )
            .to_string(),
        r#"SELECT "id" FROM "glyph" WHERE "aspect" = 1 AND ("aspect" = 2 OR "aspect" = 3)"#
    );

    // Current is an `Any`: both sides go under a fresh conjunction, call order kept.
    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .cond_where(
                Condition::any()
                    .add(Expr::col(Glyph::Aspect).eq(1))
                    .add(Expr::col(Glyph::Aspect).eq(2))
            )
            .cond_where(Condition::all().add(Expr::col(Glyph::Aspect).eq(3)))
            .to_string(),
        r#"SELECT "id" FROM "glyph" WHERE ("aspect" = 1 OR "aspect" = 2) AND "aspect" = 3"#
    );
}

// [spec:pgorm:def:sql.ast.with+5/test]    a non-recursive clause takes its first CTE at
// construction and renders every one it was given
// [spec:pgorm:req:sql.render.cte+5/test]
#[test]
fn with_clause_renders_each_of_its_ctes() {
    let cte = |name: &str| {
        CommonTableExpression::new(
            Name::runtime(name),
            Query::select().column(Glyph::Id).from(Glyph::Table).take(),
        )
    };

    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Name::runtime("one"))
            .take()
            .with(WithClause::new(cte("one")).cte(cte("two")).to_owned())
            .to_string(),
        [
            r#"WITH "one" AS (SELECT "id" FROM "glyph") ,"#,
            r#""two" AS (SELECT "id" FROM "glyph")"#,
            r#"SELECT "id" FROM "one""#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.render.cte+5/test]    a materialization hint follows `AS ` with one
// space on each side, before the parenthesised body
#[test]
fn with_clause_renders_each_materialization() {
    let cte = |name: &str, materialized: Option<bool>| {
        let mut cte = CommonTableExpression::new(
            Name::runtime(name),
            Query::select().column(Glyph::Id).from(Glyph::Table).take(),
        );
        if let Some(materialized) = materialized {
            cte.materialized(materialized);
        }
        cte
    };

    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Name::runtime("one"))
            .take()
            .with(
                WithClause::new(cte("one", Some(true)))
                    .cte(cte("two", Some(false)))
                    .cte(cte("three", None))
                    .to_owned()
            )
            .to_string(),
        [
            r#"WITH "one" AS MATERIALIZED (SELECT "id" FROM "glyph") ,"#,
            r#""two" AS NOT MATERIALIZED (SELECT "id" FROM "glyph") ,"#,
            r#""three" AS (SELECT "id" FROM "glyph")"#,
            r#"SELECT "id" FROM "one""#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:def:sql.ast.with+5/test]    `from_select` names the CTE after the select's first
// FROM table and takes its columns from the projection
#[test]
fn from_select_names_the_cte_after_its_table() {
    let cte = CommonTableExpression::from_select(
        Query::select()
            .columns([Glyph::Id, Glyph::Aspect])
            .from(Glyph::Table)
            .take(),
    )
    .expect("a select with a FROM table names its CTE");

    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Name::runtime("cte_glyph"))
            .take()
            .with(WithClause::new(cte))
            .to_string(),
        [
            r#"WITH "cte_glyph" ("id", "aspect") AS (SELECT "id", "aspect" FROM "glyph")"#,
            r#"SELECT "id" FROM "cte_glyph""#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:def:sql.ast.with+5/test]    a select with no FROM table has no name to derive, so
// `from_select` declines rather than yielding a nameless CTE
#[test]
fn from_select_declines_a_select_without_a_table() {
    assert!(CommonTableExpression::from_select(Query::select().expr(1i32).take()).is_none());
}

// [spec:pgorm:req:sql.ast.with.recursive+1/test]    the recursive form renders `WITH RECURSIVE`
// around the single CTE it holds
#[test]
fn recursive_with_clause_renders_its_single_cte() {
    assert_eq!(
        Query::select()
            .column(Asterisk)
            .from(Name::runtime("cte"))
            .take()
            .with(RecursiveWithClause::new(recursive_cte()))
            .to_string(),
        [
            r#"WITH RECURSIVE "cte" ("id", "depth") AS"#,
            r#"(SELECT "id", 1 FROM "glyph""#,
            r#"UNION ALL (SELECT "id", "depth" + 1 FROM "glyph""#,
            r#"INNER JOIN "cte" ON "cte"."id" = "glyph"."id"))"#,
            r#"SELECT * FROM "cte""#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ast.with.recursive+1/test]    the optional SEARCH and CYCLE clauses attach
// to the recursive form only, and carry the column names given to their constructors
#[test]
fn recursive_with_clause_renders_search_and_cycle() {
    let with_clause = RecursiveWithClause::new(recursive_cte())
        .search(Search::new(
            SearchOrder::BREADTH,
            Expr::col(Name::runtime("depth")),
            Name::runtime("ordercol"),
        ))
        .cycle(Cycle::new(
            Expr::col(Glyph::Id),
            Name::runtime("looped"),
            Name::runtime("path"),
        ))
        .to_owned();

    assert_eq!(
        Query::select()
            .column(Asterisk)
            .from(Name::runtime("cte"))
            .take()
            .with(with_clause)
            .to_string(),
        [
            r#"WITH RECURSIVE "cte" ("id", "depth") AS"#,
            r#"(SELECT "id", 1 FROM "glyph""#,
            r#"UNION ALL (SELECT "id", "depth" + 1 FROM "glyph""#,
            r#"INNER JOIN "cte" ON "cte"."id" = "glyph"."id"))"#,
            r#"SEARCH BREADTH FIRST BY "depth" SET "ordercol""#,
            r#"CYCLE "id" SET "looped" USING "path""#,
            r#"SELECT * FROM "cte""#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:def:query.build.with+2/test]    the clause is carried on the select, so the value is
// still a `SelectStatement` and every builder method still applies afterwards
// [spec:pgorm:sem:query.build.with.attach+1/test]    and the last clause set is the one that renders
#[test]
fn carried_with_clause_leaves_the_select_shapeable() {
    let cte = |name: &str| {
        CommonTableExpression::new(
            Name::runtime(name),
            Query::select().column(Glyph::Id).from(Glyph::Table).take(),
        )
    };

    let mut select = Query::select()
        .column(Glyph::Id)
        .from(Name::runtime("second"))
        .take();

    select
        .with(WithClause::new(cte("first")))
        .with(WithClause::new(cte("second")))
        .and_where(Expr::col(Glyph::Id).gt(1))
        .order_by(Glyph::Id, Order::Asc)
        .limit(2);

    assert_eq!(
        select.to_string(),
        [
            r#"WITH "second" AS (SELECT "id" FROM "glyph")"#,
            r#"SELECT "id" FROM "second" WHERE "id" > 1 ORDER BY "id" ASC LIMIT 2"#,
        ]
        .join(" ")
    );

    assert_eq!(
        Query::select()
            .column(Asterisk)
            .from(Name::runtime("recursive"))
            .take()
            .with(RecursiveWithClause::new(recursive_cte()))
            .with(WithClause::new(cte("recursive")))
            .to_string(),
        [
            r#"WITH "recursive" AS (SELECT "id" FROM "glyph")"#,
            r#"SELECT * FROM "recursive""#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:query.build.with.single+1/test]    one clause, one place: every statement carries
// its own and renders exactly one WITH, with no wrapper type to give a second one a home
// [spec:pgorm:def:query.build.with+2/test]    and `with` is the same verb on all four statements —
// same receiver, same return, same last-call-wins slot
// [spec:pgorm:req:sql.render.cte+5/test]
#[test]
fn a_with_clause_has_one_place_to_live() {
    let cte = || {
        CommonTableExpression::new(
            Name::runtime("cte"),
            Query::select().column(Glyph::Id).from(Glyph::Table).take(),
        )
    };
    let picked = || {
        Expr::col(Glyph::Id).in_subquery(
            Query::select()
                .column(Glyph::Id)
                .from(Name::runtime("cte"))
                .take(),
        )
    };
    let prefix = r#"WITH "cte" AS (SELECT "id" FROM "glyph")"#;

    let rendered = [
        Query::select()
            .column(Glyph::Id)
            .from(Name::runtime("cte"))
            .with(WithClause::new(cte()))
            .to_string(),
        Query::insert()
            .into_table(Glyph::Table)
            .columns([Glyph::Id])
            .select_from(
                Query::select()
                    .column(Glyph::Id)
                    .from(Name::runtime("cte"))
                    .take(),
            )
            .expect("a select source is accepted")
            .with(WithClause::new(cte()))
            .to_string(),
        Query::update()
            .table(Glyph::Table)
            .value(Glyph::Aspect, 1)
            .and_where(picked())
            .with(WithClause::new(cte()))
            .to_string(),
        Query::delete()
            .from_table(Glyph::Table)
            .and_where(picked())
            .with(WithClause::new(cte()))
            .to_string(),
    ];

    for sql in &rendered {
        assert_eq!(sql.matches("WITH").count(), 1);
        assert!(sql.starts_with(prefix), "{sql}");
    }

    assert_eq!(
        rendered,
        [
            format!(r#"{prefix} SELECT "id" FROM "cte""#),
            format!(r#"{prefix} INSERT INTO "glyph" ("id") SELECT "id" FROM "cte""#),
            format!(
                r#"{prefix} UPDATE "glyph" SET "aspect" = 1 WHERE "id" IN (SELECT "id" FROM "cte")"#
            ),
            format!(r#"{prefix} DELETE FROM "glyph" WHERE "id" IN (SELECT "id" FROM "cte")"#),
        ]
    );
}

// [spec:pgorm:sem:query.build.with.attach+1/test]    the prefix is legal PostgreSQL at every level a
// select can occupy — standalone, FROM subquery, union arm, CTE body, LATERAL body — which is what
// lets the clause ride along instead of collapsing the statement into a wrapper
// [spec:pgorm:req:sql.render.oracle+4/test]
// [spec:pgorm:req:sql.render.select-order+7/test]
#[test]
fn carried_with_clause_renders_at_every_nesting_level() {
    let inner = || {
        Query::select()
            .column(Glyph::Id)
            .from(Name::runtime("cte"))
            .take()
            .with(WithClause::new(CommonTableExpression::new(
                Name::runtime("cte"),
                Query::select().column(Glyph::Id).from(Glyph::Table).take(),
            )))
            .take()
    };
    let prefix = r#"WITH "cte" AS (SELECT "id" FROM "glyph") SELECT "id" FROM "cte""#;

    assert_eq!(inner().to_string(), prefix);

    assert_eq!(
        Query::select()
            .column(Asterisk)
            .from_subquery(inner(), Name::runtime("sub"))
            .to_string(),
        format!(r#"SELECT * FROM ({prefix}) AS "sub""#)
    );

    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .union(UnionType::All, inner())
            .to_string(),
        format!(r#"SELECT "id" FROM "glyph" UNION ALL ({prefix})"#)
    );

    assert_eq!(
        Query::select()
            .column(Asterisk)
            .from(Name::runtime("outer"))
            .take()
            .with(WithClause::new(CommonTableExpression::new(
                Name::runtime("outer"),
                inner(),
            )))
            .to_string(),
        format!(r#"WITH "outer" AS ({prefix}) SELECT * FROM "outer""#)
    );

    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .join_lateral(
                JoinType::InnerJoin,
                inner(),
                Name::runtime("lat"),
                Expr::raw("TRUE"),
            )
            .to_string(),
        format!(r#"SELECT "id" FROM "glyph" INNER JOIN LATERAL ({prefix}) AS "lat" ON TRUE"#)
    );
}

fn recursive_cte() -> CommonTableExpression {
    let mut base = Query::select()
        .column(Glyph::Id)
        .expr(1i32)
        .from(Glyph::Table)
        .take();
    let step = Query::select()
        .column(Glyph::Id)
        .expr(Expr::col(Name::runtime("depth")).add(1i32))
        .from(Glyph::Table)
        .inner_join(
            Name::runtime("cte"),
            Expr::col((Name::runtime("cte"), Glyph::Id)).equals((Glyph::Table, Glyph::Id)),
        )
        .take();

    CommonTableExpression::new(
        Name::runtime("cte"),
        base.union(UnionType::All, step).take(),
    )
    .column(Glyph::Id)
    .column(Name::runtime("depth"))
    .to_owned()
}

// [spec:pgorm:sem:sql.render.empty-in+1/test]    an empty `IN` is rewritten to the always-false
// comparison of two distinct string values
#[test]
fn empty_in_1() {
    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .and_where(Expr::col(Glyph::Aspect).is_in(Vec::<i32>::new()))
            .build(),
        (
            r#"SELECT "id" FROM "glyph" WHERE $1 = $2"#.to_owned(),
            Values(vec!["a".into(), "b".into()])
        )
    );

    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .and_where(Expr::col(Glyph::Aspect).is_in(Vec::<i32>::new()))
            .to_string(),
        r#"SELECT "id" FROM "glyph" WHERE 'a' = 'b'"#
    );
}

// [spec:pgorm:sem:sql.render.empty-in+1/test]    an empty `NOT IN` renders the always-true
// comparison instead — vacuous non-membership matches every row
#[test]
fn empty_in_2() {
    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .and_where(Expr::col(Glyph::Aspect).is_not_in(Vec::<i32>::new()))
            .build(),
        (
            r#"SELECT "id" FROM "glyph" WHERE $1 = $2"#.to_owned(),
            Values(vec!["a".into(), "a".into()])
        )
    );

    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .and_where(Expr::col(Glyph::Aspect).is_not_in(Vec::<i32>::new()))
            .to_string(),
        r#"SELECT "id" FROM "glyph" WHERE 'a' = 'a'"#
    );

    // A non-empty list is unaffected.
    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .and_where(Expr::col(Glyph::Aspect).is_not_in([1, 2]))
            .to_string(),
        r#"SELECT "id" FROM "glyph" WHERE "aspect" NOT IN (1, 2)"#
    );
}

// [spec:pgorm:req:sql.ast.expr.eq-any+2/test]    `eq_any` spends one
// placeholder whatever the list length, so the SQL text a statement cache keys
// on no longer varies with cardinality the way `IN` makes it
#[test]
fn eq_any_binds_one_array_parameter() {
    let query = |aspects: Vec<i32>| {
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .and_where(Expr::col(Glyph::Aspect).eq_any(aspects))
            .to_owned()
    };

    assert_eq!(
        query(vec![1, 2, 3]).build(),
        (
            r#"SELECT "id" FROM "glyph" WHERE "aspect" = ANY($1)"#.to_owned(),
            Values(vec![Value::array([1, 2, 3])])
        )
    );

    // Two elements and twenty render the same statement; `IN` renders neither
    // the same as the other.
    assert_eq!(
        query(vec![1, 2]).build().0,
        query((1..=20).collect()).build().0
    );
    assert_ne!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .and_where(Expr::col(Glyph::Aspect).is_in([1, 2]))
            .build()
            .0,
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .and_where(Expr::col(Glyph::Aspect).is_in([1, 2, 3]))
            .build()
            .0
    );

    assert_eq!(
        query(vec![1, 2, 3]).to_string(),
        r#"SELECT "id" FROM "glyph" WHERE "aspect" = ANY(ARRAY [1,2,3])"#
    );
}

// [spec:pgorm:req:sql.ast.expr.eq-any+2/test]    the negation is spelled
// `<> ALL`, the shape PostgreSQL's own parser gives `NOT IN`, rather than a
// `NOT` wrapped around `= ANY`
#[test]
fn ne_all_negates_with_one_parameter() {
    let query = Query::select()
        .column(Glyph::Id)
        .from(Glyph::Table)
        .and_where(Expr::col(Glyph::Aspect).ne_all([1, 2, 3]))
        .to_owned();

    assert_eq!(
        query.build(),
        (
            r#"SELECT "id" FROM "glyph" WHERE "aspect" <> ALL($1)"#.to_owned(),
            Values(vec![Value::array([1, 2, 3])])
        )
    );
    assert_eq!(
        query.to_string(),
        r#"SELECT "id" FROM "glyph" WHERE "aspect" <> ALL(ARRAY [1,2,3])"#
    );
}

// [spec:pgorm:req:sql.ast.expr.eq-any+2/test]    an empty list needs no
// constant fall-back: an empty array carries the vacuous truth of the predicate
// natively, where an empty `IN` has to be rewritten
#[test]
fn empty_list_stays_an_array_parameter() {
    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .and_where(Expr::col(Glyph::Aspect).eq_any(Vec::<i32>::new()))
            .build(),
        (
            r#"SELECT "id" FROM "glyph" WHERE "aspect" = ANY($1)"#.to_owned(),
            Values(vec![Value::array(Vec::<i32>::new())])
        )
    );

    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .and_where(Expr::col(Glyph::Aspect).eq_any(Vec::<i32>::new()))
            .to_string(),
        r#"SELECT "id" FROM "glyph" WHERE "aspect" = ANY(ARRAY []::int4[])"#
    );
    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .and_where(Expr::col(Glyph::Aspect).ne_all(Vec::<i32>::new()))
            .to_string(),
        r#"SELECT "id" FROM "glyph" WHERE "aspect" <> ALL(ARRAY []::int4[])"#
    );
}
