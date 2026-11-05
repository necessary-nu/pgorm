//! The COPY family's cases, built with pgorm-query's COPY statements directly.
//! A COPY binds nothing: a value in its `WHERE` or its query is written as a
//! literal.

use std::num::NonZeroU32;

use pgorm::pgorm_query::{
    BinOper, CopyBinary, CopyCsv, CopyFromStatement, CopyIgnore, CopyLogVerbosity, CopySource,
    CopyText, CopyToStatement, Expr, ExprTrait, FromItem, NamedTable, Query, SimpleExpr, TableName,
    Values,
};

use super::n;

macro_rules! col {
    ($name:expr) => {
        SimpleExpr::from(Expr::col(n($name)))
    };
}

fn account() -> TableName {
    TableName::SchemaTable(n("app"), n("account"))
}

fn copy(statement: impl ToString) -> (String, Values) {
    (statement.to_string(), Values(Vec::new()))
}

fn from_text_options() -> (String, Values) {
    let ignore = CopyIgnore {
        reject_limit: NonZeroU32::new(10),
        log_verbosity: Some(CopyLogVerbosity::Verbose),
    };
    copy(
        CopyFromStatement::new(account(), CopyText)
            .columns([n("id"), n("name")])
            .columns([n("note")])
            .delimiter('|')
            .null_string("NULL")
            .default_string("@default")
            .header()
            .encoding("UTF8")
            .on_error_ignore(ignore)
            .cond_where(col!("id").binary(BinOper::GreaterThan, 3i64))
            .cond_where(col!("name").binary(BinOper::NotEqual, "O'Brien")),
    )
}

fn from_csv_forced() -> (String, Values) {
    copy(
        CopyFromStatement::new(TableName::Table(n("ticket")), CopyCsv)
            .columns([n("id"), n("note"), n("label")])
            .quote('\'')
            .escape('\\')
            .header_match()
            .force_not_null([n("note")])
            .force_null_all()
            .on_error_ignore(CopyIgnore {
                reject_limit: None,
                log_verbosity: Some(CopyLogVerbosity::Silent),
            }),
    )
}

fn to_select_csv() -> (String, Values) {
    let select = Query::select()
        .expr(col!("id"))
        .expr(col!("name"))
        .from(FromItem::Table(NamedTable::from(account())))
        .cond_where(col!("name").binary(BinOper::Equal, "O'Brien"))
        .to_owned();
    copy(
        CopyToStatement::new(select, CopyCsv)
            .header()
            .delimiter(';')
            .null_string("-")
            .quote('"')
            .escape('"')
            .force_quote([n("name")]),
    )
}

fn to_delete_returning() -> (String, Values) {
    let delete = Query::delete()
        .from_table(NamedTable::from(account()))
        .cond_where(col!("id").binary(BinOper::GreaterThan, 3i64))
        .returning(Query::returning().all())
        .to_owned();
    let source = CopySource::try_from(delete).expect("a DELETE with RETURNING is a source");
    copy(CopyToStatement::new(source, CopyCsv).force_quote_all())
}

fn to_insert_returning() -> (String, Values) {
    let insert = Query::insert()
        .into_table(NamedTable::from(TableName::Table(n("ticket"))))
        .columns([n("note")])
        .values_panic(["x".into()])
        .returning(Query::returning().exprs([col!("id")]))
        .to_owned();
    let source = CopySource::try_from(insert).expect("an INSERT with RETURNING is a source");
    copy(CopyToStatement::new(source, CopyBinary))
}

/// Every case, named as the golden file names it.
pub(super) fn cases() -> Vec<(&'static str, (String, Values))> {
    vec![
        ("from-text-options", from_text_options()),
        ("from-csv-forced", from_csv_forced()),
        (
            "from-binary-freeze",
            copy(
                CopyFromStatement::new(TableName::Table(n("ticket")), CopyBinary)
                    .columns([n("id")])
                    .freeze(),
            ),
        ),
        (
            "to-table-text",
            copy(
                CopyToStatement::new(account(), CopyText)
                    .null_string("")
                    .encoding("LATIN1"),
            ),
        ),
        ("to-select-csv", to_select_csv()),
        ("to-delete-returning", to_delete_returning()),
        ("to-insert-returning", to_insert_returning()),
    ]
}
