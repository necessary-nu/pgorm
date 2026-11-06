//! The MERGE family's cases, built with pgorm-query directly.

use pgorm::pgorm_query::{
    BinOper, CommonTableExpression, Expr, ExprTrait, FromItem, MatchedAction, MergeInsert,
    MergeUpdate, NamedTable, NotMatchedAction, Overriding, Query, ReturningRow, SimpleExpr,
    TableName, Values, WithClause,
};

use super::n;

macro_rules! target {
    () => {
        NamedTable::from(TableName::SchemaTable(n("app"), n("account"))).alias(n("t"))
    };
}

macro_rules! at {
    ($table:expr, $name:expr) => {
        SimpleExpr::from(Expr::col((n($table), n($name))))
    };
}

macro_rules! pending {
    () => {
        Query::merge(
            target!(),
            FromItem::Table(NamedTable::from(TableName::Table(n("staged"))).alias(n("s"))),
            at!("t", "id").eq(at!("s", "id")),
        )
    };
}

fn every_arm() -> (String, Values) {
    pending!()
        .when_matched(
            MergeUpdate::value(n("name"), at!("s", "name"))
                .and_value(n("visits"), at!("t", "visits").binary(BinOper::Add, 1i64)),
        )
        .when_matched_and(
            Expr::expr(at!("s", "name")).is_null(),
            MatchedAction::Delete,
        )
        .when_not_matched(
            MergeInsert::value(n("id"), at!("s", "id")).and_value(n("name"), at!("s", "name")),
        )
        .when_not_matched_by_source(MatchedAction::Delete)
        .returning_action()
        .returning(
            Query::returning().exprs([
                at!("t", "id"),
                SimpleExpr::from(Expr::col((ReturningRow::Old, n("name"))))
                    .binary(BinOper::As, Expr::col(n("was"))),
            ]),
        )
        .build()
}

fn unconditional_last() -> (String, Values) {
    pending!()
        .when_matched(MatchedAction::DoNothing)
        .when_matched_and(
            at!("s", "kind").eq("rename"),
            MergeUpdate::value(n("name"), "x"),
        )
        .when_matched(MatchedAction::Delete)
        .when_not_matched_and(at!("s", "kind").eq("skip"), NotMatchedAction::DoNothing)
        .when_not_matched(NotMatchedAction::InsertDefaultValues)
        .build()
}

fn overriding_only() -> (String, Values) {
    pending!()
        .when_not_matched(MergeInsert::value(n("id"), 7i32).overriding(Overriding::SystemValue))
        .when_not_matched_by_source_and(
            at!("t", "active").eq(true),
            MergeUpdate::value(n("active"), false),
        )
        .only()
        .build()
}

fn with_source() -> (String, Values) {
    let fresh = Query::select()
        .expr(Expr::col(n("id")))
        .from(FromItem::Table(NamedTable::from(TableName::Table(n(
            "staged",
        )))))
        .cond_where(SimpleExpr::from(Expr::col(n("id"))).binary(BinOper::GreaterThan, 10i64))
        .to_owned();
    let source = FromItem::Table(NamedTable::from(TableName::Table(n("fresh"))));
    Query::merge(target!(), source, at!("t", "id").eq(at!("fresh", "id")))
        .when_not_matched(MergeInsert::value(n("id"), at!("fresh", "id")))
        .with(WithClause::new(CommonTableExpression::new(
            n("fresh"),
            fresh,
        )))
        .returning(Query::returning().all())
        .build()
}

fn subquery_source() -> (String, Values) {
    let staged = Query::select()
        .expr(Expr::col(n("id")))
        .expr(Expr::col(n("name")))
        .from(FromItem::Table(NamedTable::from(TableName::Table(n(
            "staged",
        )))))
        .to_owned();
    Query::merge(
        target!(),
        FromItem::SubQuery(staged, n("s")),
        at!("t", "id").eq(at!("s", "id")),
    )
    .when_matched(MergeUpdate::value(n("name"), at!("s", "name")))
    .build()
}

fn as_cte() -> (String, Values) {
    let changed = pending!()
        .when_matched(MergeUpdate::value(n("name"), at!("s", "name")))
        .when_not_matched(
            MergeInsert::value(n("id"), at!("s", "id")).and_value(n("name"), at!("s", "name")),
        )
        .returning_action()
        .returning(Query::returning().exprs([at!("t", "id")]))
        .to_owned();
    Query::select()
        .expr(Expr::col(n("merge_action")))
        .expr(Expr::col(n("id")))
        .from(FromItem::Table(NamedTable::from(TableName::Table(n(
            "changed",
        )))))
        .with(WithClause::new(CommonTableExpression::new(
            n("changed"),
            changed,
        )))
        .build()
}

pub(super) fn cases() -> Vec<(&'static str, (String, Values))> {
    vec![
        ("merge-every-arm", every_arm()),
        ("merge-unconditional-last", unconditional_last()),
        ("merge-overriding-only", overriding_only()),
        ("merge-with-source", with_source()),
        ("merge-subquery-source", subquery_source()),
        ("merge-as-cte", as_cte()),
    ]
}
