//! The INSERT, UPDATE and DELETE family's cases, built with pgorm-query
//! directly.

use pgorm::pgorm_query::{
    Asterisk, BinOper, CommonTableExpression, Expr, ExprTrait, FromItem, Func, NamedTable,
    OnConflict, Overriding, Query, ReturningRow, SelectStatement, SimpleExpr, TableName, TypeName,
    Value, Values, WithClause,
};

use super::n;

macro_rules! account {
    () => {
        NamedTable::from(TableName::SchemaTable(n("app"), n("account")))
    };
}

macro_rules! col {
    ($name:expr) => {
        SimpleExpr::from(Expr::col(n($name)))
    };
}

macro_rules! ids {
    ($table:expr) => {
        Query::select()
            .expr(col!("id"))
            .from(FromItem::Table(NamedTable::from(TableName::Table(n(
                $table,
            )))))
            .to_owned()
    };
}

fn upsert() -> (String, Values) {
    Query::insert()
        .into_table(account!())
        .columns([n("id"), n("name")])
        .values_panic([1i64.into(), "Alice".into()])
        .values_panic([2i64.into(), "O'Brien".into()])
        .on_conflict(OnConflict::column(n("id")).update_column(n("name")))
        .returning(
            Query::returning().exprs([
                col!("id"),
                Expr::expr(SimpleExpr::from(Expr::col((ReturningRow::Old, n("id")))))
                    .is_null()
                    .binary(BinOper::As, Expr::col(n("inserted"))),
            ]),
        )
        .build()
}

fn constraint_set() -> (String, Values) {
    Query::insert()
        .into_table(account!())
        .columns([n("id"), n("name")])
        .values_panic([1i64.into(), "x".into()])
        .on_conflict(
            OnConflict::constraint(n("account_pkey"))
                .value(n("name"), "y")
                .update_column(n("note"))
                .cond_where(col!("name").binary(BinOper::NotEqual, "z")),
        )
        .build()
}

fn expression_target() -> (String, Values) {
    Query::insert()
        .into_table(account!())
        .columns([n("id")])
        .values_panic([1i64.into()])
        .overriding(Overriding::SystemValue)
        .on_conflict(
            OnConflict::column(n("id"))
                .and_expr(Func::lower(col!("name")))
                .cond_where(col!("active").eq(true))
                .do_nothing(),
        )
        .build()
}

fn typed_values() -> (String, Values) {
    let mood = TypeName::new(n("mood")).schema(n("app"));
    Query::insert()
        .into_table(account!())
        .columns([n("small"), n("missing"), n("mood")])
        .values_panic([
            5i16.into(),
            Value::String(None).into(),
            SimpleExpr::from("calm").cast_as_type(mood),
        ])
        .build()
}

fn with() -> (String, Values) {
    let source = SelectStatement::new()
        .expr(col!("id"))
        .from(FromItem::Table(NamedTable::from(TableName::Table(n(
            "src",
        )))))
        .to_owned();
    Query::insert()
        .into_table(account!())
        .with(WithClause::new(CommonTableExpression::new(
            n("src"),
            ids!("staged"),
        )))
        .columns([n("id")])
        .select_from(source)
        .expect("one column for one")
        .build()
}

fn versions_renamed() -> (String, Values) {
    Query::update()
        .table(account!())
        .value(n("name"), "Bob")
        .value(n("visits"), col!("visits").binary(BinOper::Add, 1i64))
        .cond_where(col!("id").eq(2i64))
        .returning(
            Query::returning()
                .exprs([
                    SimpleExpr::from(Expr::col((n("before"), n("name"))))
                        .binary(BinOper::As, Expr::col(n("was"))),
                    SimpleExpr::from(Expr::col((n("after"), n("name"))))
                        .binary(BinOper::As, Expr::col(n("now"))),
                ])
                .old_as(n("before"))
                .new_as(n("after")),
        )
        .build()
}

fn delete_using() -> (String, Values) {
    Query::delete()
        .from_table(account!())
        .using(FromItem::Table(NamedTable::from(TableName::Table(n(
            "gone",
        )))))
        .cond_where(
            SimpleExpr::from(Expr::col((n("gone"), n("id")))).eq(SimpleExpr::from(Expr::col((
                n("app"),
                n("account"),
                n("id"),
            )))),
        )
        .returning(
            Query::returning().exprs([SimpleExpr::from(Expr::col((ReturningRow::Old, Asterisk)))]),
        )
        .build()
}

fn delete_as_cte() -> (String, Values) {
    let moved = Query::delete()
        .from_table(account!())
        .cond_where(col!("id").binary(BinOper::GreaterThan, 3i64))
        .returning(Query::returning().exprs([col!("id")]))
        .to_owned();
    let kept = Query::update()
        .table(NamedTable::from(TableName::Table(n("log"))))
        .value(n("seen"), true)
        .returning(Query::returning().exprs([col!("id")]))
        .to_owned();
    let mut clause = WithClause::new(CommonTableExpression::new(n("moved"), moved));
    clause.cte(CommonTableExpression::new(n("kept"), kept));
    Query::select()
        .column(Asterisk)
        .from(FromItem::Table(NamedTable::from(TableName::Table(n(
            "moved",
        )))))
        .with(clause)
        .build()
}

pub(super) fn cases() -> Vec<(&'static str, (String, Values))> {
    vec![
        ("insert-values-upsert", upsert()),
        (
            "insert-select-do-nothing",
            Query::insert()
                .into_table(account!())
                .columns([n("id")])
                .select_from(ids!("staged"))
                .expect("one column for one")
                .on_conflict(OnConflict::do_nothing())
                .build(),
        ),
        (
            "insert-defaults",
            Query::insert()
                .into_table(account!())
                .or_default_values()
                .returning(Query::returning().all())
                .build(),
        ),
        ("insert-constraint-set", constraint_set()),
        ("insert-expression-target", expression_target()),
        ("insert-typed-values", typed_values()),
        ("insert-with", with()),
        ("update-versions-renamed", versions_renamed()),
        (
            "update-from-all-rows",
            Query::update()
                .table(account!())
                .value(n("name"), SimpleExpr::from(Expr::col((n("s"), n("other")))))
                .from(FromItem::Table(
                    NamedTable::from(TableName::Table(n("staged"))).alias(n("s")),
                ))
                .build(),
        ),
        (
            "update-where-twice",
            Query::update()
                .table(account!())
                .value(n("a"), 1i64)
                .cond_where(col!("b").eq(2i64))
                .cond_where(col!("c").eq(3i64))
                .returning(
                    Query::returning()
                        .exprs([SimpleExpr::from(Expr::col((ReturningRow::New, Asterisk)))]),
                )
                .build(),
        ),
        ("delete-using", delete_using()),
        (
            "delete-all-rows",
            Query::delete()
                .from_table(NamedTable::from(TableName::Table(n("t"))))
                .build(),
        ),
        ("delete-as-cte", delete_as_cte()),
    ]
}
