use std::num::NonZeroU32;

use super::*;
use crate::oracle::{assert_eq, parsed_nodes};
use pgorm_query::error::Error;
use serde_json::{Value as Json, json};

fn n(name: &str) -> Name {
    Name::runtime(name)
}

/// The one `CopyStmt` in `sql`, as PostgreSQL's parser read it.
fn statement(sql: &str) -> Json {
    let mut found = parsed_nodes(sql, "CopyStmt");
    assert_eq!(found.len(), 1, "exactly one CopyStmt in {sql}");
    found.remove(0)
}

/// The statement's options as the parser read them: each `DefElem`'s name and
/// argument, in order.
fn options(copy: &Json) -> Vec<(String, Json)> {
    copy["options"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|option| {
            let def = &option["DefElem"];
            (
                def["defname"].as_str().expect("an option name").to_owned(),
                def.get("arg").cloned().unwrap_or(Json::Null),
            )
        })
        .collect()
}

fn string(value: &str) -> Json {
    json!({ "String": { "sval": value } })
}

fn names(values: &[&str]) -> Json {
    json!({ "List": { "items": values.iter().map(|value| string(value)).collect::<Vec<_>>() } })
}

fn option(name: &str, arg: Json) -> (String, Json) {
    (name.to_owned(), arg)
}

/// The column names a parsed list holds.
fn attributes(copy: &Json) -> Vec<&str> {
    copy["attlist"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|item| item["String"]["sval"].as_str().expect("a column name"))
        .collect()
}

// [spec:pgorm:def:sql.ast.copy+3/test]    a bare COPY FROM names its table, its direction and its format
// [spec:pgorm:req:sql.render.copy/test]
#[test]
fn copy_from_names_table_and_format() {
    let sql = CopyFromStatement::new(Glyph::Table, CopyText).to_string();
    assert_eq!(sql, r#"COPY "glyph" FROM STDIN WITH (FORMAT text)"#);
    let copy = statement(&sql);
    assert_eq!(copy["relation"]["relname"], "glyph");
    assert_eq!(copy["is_from"], true);
    assert_eq!(copy["filename"], "", "{sql}");
    assert_eq!(copy["is_program"], false, "{sql}");
    assert_eq!(options(&copy), [option("format", string("text"))]);
}

// [spec:pgorm:def:sql.ast.copy+3/test]    the table is schema-qualifiable and every name is quoted, a hostile one included
// [spec:pgorm:req:sql.render.copy/test]
#[test]
fn copy_names_are_quoted_identifiers() {
    let sql = CopyFromStatement::new((n("bulk"), n("in\"box")), CopyCsv)
        .columns([n("id"), n("weird, \"name\"")])
        .columns([n("Mixed")])
        .force_not_null([n("weird, \"name\"")])
        .to_string();
    assert_eq!(
        sql,
        [
            r#"COPY "bulk"."in""box" ("id", "weird, ""name""", "Mixed") FROM STDIN"#,
            r#"WITH (FORMAT csv, FORCE_NOT_NULL ("weird, ""name"""))"#,
        ]
        .join(" ")
    );
    let copy = statement(&sql);
    assert_eq!(copy["relation"]["schemaname"], "bulk");
    assert_eq!(copy["relation"]["relname"], "in\"box");
    assert_eq!(attributes(&copy), ["id", "weird, \"name\"", "Mixed"]);
    assert_eq!(
        options(&copy)[1],
        option("force_not_null", names(&["weird, \"name\""]))
    );
}

// [spec:pgorm:def:sql.ast.copy+3/test]    every option a text-format COPY FROM takes, read back as the option it was written as
// [spec:pgorm:req:sql.render.copy/test]
#[test]
fn copy_from_text_takes_its_options() {
    let sql = CopyFromStatement::new(Glyph::Table, CopyText)
        .freeze()
        .delimiter('|')
        .null_string("\\N")
        .default_string("\\D")
        .header_match()
        .encoding("UTF8")
        .on_error_ignore(CopyIgnore {
            reject_limit: NonZeroU32::new(10),
            log_verbosity: Some(CopyLogVerbosity::Verbose),
        })
        .to_string();
    assert_eq!(
        sql,
        [
            r#"COPY "glyph" FROM STDIN WITH (FORMAT text, FREEZE true, DELIMITER '|',"#,
            r#"NULL E'\\N', DEFAULT E'\\D', HEADER match, ON_ERROR ignore,"#,
            r#"REJECT_LIMIT 10, ENCODING 'UTF8', LOG_VERBOSITY verbose)"#,
        ]
        .join(" ")
    );
    assert_eq!(
        options(&statement(&sql)),
        [
            option("format", string("text")),
            option("freeze", string("true")),
            option("delimiter", string("|")),
            option("null", string("\\N")),
            option("default", string("\\D")),
            option("header", string("match")),
            option("on_error", string("ignore")),
            option("reject_limit", json!({ "Integer": { "ival": 10 } })),
            option("encoding", string("UTF8")),
            option("log_verbosity", string("verbose")),
        ]
    );
}

// [spec:pgorm:def:sql.ast.copy+3/test]    ignoring errors without a limit or a verbosity writes ON_ERROR alone
#[test]
fn copy_from_ignore_alone() {
    let sql = CopyFromStatement::new(Glyph::Table, CopyCsv)
        .on_error_ignore(CopyIgnore::default())
        .header()
        .to_string();
    assert_eq!(
        sql,
        r#"COPY "glyph" FROM STDIN WITH (FORMAT csv, HEADER true, ON_ERROR ignore)"#
    );
}

// [spec:pgorm:def:sql.ast.copy+3/test]    the CSV-only options, and FORCE_NULL over every column
// [spec:pgorm:req:sql.render.copy/test]
#[test]
fn copy_from_csv_takes_quote_and_forces() {
    let sql = CopyFromStatement::new(Glyph::Table, CopyCsv)
        .columns([Glyph::Id, Glyph::Image])
        .delimiter(';')
        .quote('\'')
        .escape('\\')
        .force_not_null([Glyph::Id])
        .force_null_all()
        .to_string();
    assert_eq!(
        sql,
        [
            r#"COPY "glyph" ("id", "image") FROM STDIN WITH (FORMAT csv, DELIMITER ';',"#,
            r#"QUOTE E'\'', ESCAPE E'\\', FORCE_NOT_NULL ("id"), FORCE_NULL *)"#,
        ]
        .join(" ")
    );
    assert_eq!(
        options(&statement(&sql)),
        [
            option("format", string("csv")),
            option("delimiter", string(";")),
            option("quote", string("'")),
            option("escape", string("\\")),
            option("force_not_null", names(&["id"])),
            option("force_null", json!({ "AStar": {} })),
        ]
    );
}

// [spec:pgorm:def:sql.ast.copy+3/test]    an empty forced list forces nothing, since `FORCE_QUOTE ()` is a syntax error
#[test]
fn copy_empty_force_list_is_left_out() {
    let none: [Name; 0] = [];
    let from = CopyFromStatement::new(Glyph::Table, CopyCsv)
        .force_null(none.clone())
        .force_not_null(none.clone())
        .to_string();
    assert_eq!(from, r#"COPY "glyph" FROM STDIN WITH (FORMAT csv)"#);
    let to = CopyToStatement::new(Glyph::Table, CopyCsv)
        .force_quote(none)
        .to_string();
    assert_eq!(to, r#"COPY "glyph" TO STDOUT WITH (FORMAT csv)"#);
}

// [spec:pgorm:def:sql.ast.copy+3/test]    a binary COPY FROM takes a column list, FREEZE and a WHERE, and nothing textual
// [spec:pgorm:req:sql.render.copy/test]
#[test]
fn copy_from_binary_with_where() {
    let sql = CopyFromStatement::new(Glyph::Table, CopyBinary)
        .columns([Glyph::Id, Glyph::Aspect])
        .freeze()
        .and_where(Expr::col(Glyph::Aspect).gt(2))
        .and_where(Expr::col(Glyph::Id).ne(7))
        .to_string();
    assert_eq!(
        sql,
        [
            r#"COPY "glyph" ("id", "aspect") FROM STDIN WITH (FORMAT binary, FREEZE true)"#,
            r#"WHERE "aspect" > 2 AND "id" <> 7"#,
        ]
        .join(" ")
    );
    let copy = statement(&sql);
    assert!(copy.get("where_clause").is_some(), "{copy}");
}

// [spec:pgorm:def:sql.ast.copy+3/test]    a COPY TO reads a whole table
// [spec:pgorm:req:sql.render.copy/test]
#[test]
fn copy_to_reads_a_table() {
    let sql = CopyToStatement::new((n("public"), n("glyph")), CopyText)
        .delimiter('\t')
        .null_string("")
        .header()
        .encoding("LATIN1")
        .to_string();
    assert_eq!(
        sql,
        [
            r#"COPY "public"."glyph" TO STDOUT WITH (FORMAT text, DELIMITER E'\t',"#,
            r#"NULL '', HEADER true, ENCODING 'LATIN1')"#,
        ]
        .join(" ")
    );
    let copy = statement(&sql);
    assert_eq!(copy["is_from"], false, "{copy}");
    assert_eq!(copy["relation"]["relname"], "glyph");
    assert_eq!(
        options(&copy),
        [
            option("format", string("text")),
            option("delimiter", string("\t")),
            option("null", string("")),
            option("header", string("true")),
            option("encoding", string("LATIN1")),
        ]
    );
}

// [spec:pgorm:def:sql.ast.copy+3/test]    a COPY TO over a query inlines its values, since COPY binds none
// [spec:pgorm:req:sql.render.copy/test]
#[test]
fn copy_to_query_inlines_values() {
    let select = Query::select()
        .columns([Glyph::Id, Glyph::Image])
        .from(Glyph::Table)
        .and_where(Expr::col(Glyph::Image).eq("it's"))
        .and_where(Expr::col(Glyph::Aspect).between(1.5, 2.5))
        .order_by(Glyph::Id, Order::Asc)
        .take();
    let sql = CopyToStatement::new(select, CopyCsv)
        .header()
        .quote('"')
        .escape('"')
        .force_quote([Glyph::Image])
        .to_string();
    assert_eq!(
        sql,
        [
            r#"COPY (SELECT "id", "image" FROM "glyph" WHERE "image" = E'it\'s'"#,
            r#"AND "aspect" BETWEEN 1.5 AND 2.5 ORDER BY "id" ASC) TO STDOUT"#,
            r#"WITH (FORMAT csv, HEADER true, QUOTE '"', ESCAPE '"', FORCE_QUOTE ("image"))"#,
        ]
        .join(" ")
    );
    let copy = statement(&sql);
    assert!(copy.get("relation").is_none(), "{copy}");
    assert!(copy["query"].get("SelectStmt").is_some(), "{copy}");
}

// [spec:pgorm:def:sql.ast.copy+3/test]    a write with RETURNING, and a WITH-prefixed SELECT, are sources too
// [spec:pgorm:req:sql.render.copy/test]
#[test]
fn copy_to_write_and_with_query() {
    let insert = Query::insert()
        .into_table(Glyph::Table)
        .columns([Glyph::Image])
        .values_panic(["new".into()])
        .returning_col(Glyph::Id)
        .to_owned();
    let source = CopySource::try_from(insert).expect("an INSERT with RETURNING is a source");
    let sql = CopyToStatement::new(source, CopyBinary).to_string();
    assert_eq!(
        sql,
        r#"COPY (INSERT INTO "glyph" ("image") VALUES ('new') RETURNING "id") TO STDOUT WITH (FORMAT binary)"#
    );
    assert!(statement(&sql)["query"].get("InsertStmt").is_some());

    let cte = CommonTableExpression::new(
        n("ids"),
        Query::select().column(Glyph::Id).from(Glyph::Table).take(),
    );
    let select = Query::select()
        .column(Asterisk)
        .from(n("ids"))
        .with(WithClause::new(cte))
        .to_owned();
    let sql = CopyToStatement::new(select, CopyText).to_string();
    assert_eq!(
        sql,
        [
            r#"COPY (WITH "ids" AS (SELECT "id" FROM "glyph") SELECT * FROM "ids")"#,
            r#"TO STDOUT WITH (FORMAT text)"#,
        ]
        .join(" ")
    );
}

/// A MERGE that writes `glyph` from `font`, with no RETURNING list yet.
fn merge() -> MergeStatement {
    Query::merge(
        Glyph::Table,
        Font::Table,
        Expr::col((Glyph::Table, Glyph::Id)).equals((Font::Table, Font::Id)),
    )
    .when_matched(MergeUpdate::value(
        Glyph::Image,
        Expr::col((Font::Table, Font::Name)),
    ))
}

// [spec:pgorm:def:sql.ast.copy+3/test]    each write without RETURNING is refused as it converts, naming the write
#[test]
fn copy_to_refuses_writes_without_returning() {
    let insert = Query::insert()
        .into_table(Glyph::Table)
        .columns([Glyph::Image])
        .values_panic(["new".into()])
        .to_owned();
    let update = Query::update()
        .table(Glyph::Table)
        .value(Glyph::Aspect, 2)
        .to_owned();
    let delete = Query::delete().from_table(Glyph::Table).to_owned();
    let refused = |write: &'static str| Some(Error::CopyWithoutReturning { write });

    assert_eq!(
        CopySource::try_from(insert.clone()).err(),
        refused("INSERT")
    );
    assert_eq!(
        CopySource::try_from(update.clone()).err(),
        refused("UPDATE")
    );
    assert_eq!(
        CopySource::try_from(delete.clone()).err(),
        refused("DELETE")
    );
    assert_eq!(CopySource::try_from(merge()).err(), refused("MERGE"));

    // The same check holds a write reached as a `SubQueryStatement`.
    let nested = [
        (insert.into_sub_query_statement(), "INSERT"),
        (update.into_sub_query_statement(), "UPDATE"),
        (delete.into_sub_query_statement(), "DELETE"),
        (merge().into_sub_query_statement(), "MERGE"),
    ];
    for (statement, write) in nested {
        assert_eq!(CopySource::try_from(statement).err(), refused(write));
    }

    // The message is no SQL, so it bypasses the oracle's `assert_eq`.
    std::assert_eq!(
        Error::CopyWithoutReturning { write: "MERGE" }.to_string(),
        "COPY TO copies the rows a write returns, and this MERGE has no RETURNING list"
    );
}

// [spec:pgorm:def:sql.ast.copy+3/test]    UPDATE, DELETE and MERGE with RETURNING are sources, merge_action() alone a list
// [spec:pgorm:req:sql.render.copy/test]
#[test]
fn copy_to_writes_with_returning() {
    let update = Query::update()
        .table(Glyph::Table)
        .value(Glyph::Aspect, 2)
        .and_where(Expr::col(Glyph::Id).eq(1))
        .returning(Query::returning().columns([
            (ReturningRow::Old, Glyph::Aspect),
            (ReturningRow::New, Glyph::Aspect),
        ]))
        .to_owned();
    let delete = Query::delete()
        .from_table(Glyph::Table)
        .and_where(Expr::col(Glyph::Id).eq(1))
        .returning_all()
        .to_owned();
    let listed = merge()
        .returning(Query::returning().column(Glyph::Id))
        .to_owned();
    let action = merge().returning_action().to_owned();

    let cases = [
        (
            CopySource::try_from(update),
            "UpdateStmt",
            r#"COPY (UPDATE "glyph" SET "aspect" = 2 WHERE "id" = 1 RETURNING old."aspect", new."aspect") TO STDOUT WITH (FORMAT text)"#,
        ),
        (
            CopySource::try_from(delete),
            "DeleteStmt",
            r#"COPY (DELETE FROM "glyph" WHERE "id" = 1 RETURNING *) TO STDOUT WITH (FORMAT text)"#,
        ),
        (
            CopySource::try_from(listed),
            "MergeStmt",
            r#"COPY (MERGE INTO "glyph" USING "font" ON "glyph"."id" = "font"."id" WHEN MATCHED THEN UPDATE SET "image" = "font"."name" RETURNING "id") TO STDOUT WITH (FORMAT text)"#,
        ),
        (
            CopySource::try_from(action),
            "MergeStmt",
            r#"COPY (MERGE INTO "glyph" USING "font" ON "glyph"."id" = "font"."id" WHEN MATCHED THEN UPDATE SET "image" = "font"."name" RETURNING merge_action()) TO STDOUT WITH (FORMAT text)"#,
        ),
    ];
    for (source, kind, expected) in cases {
        let source = source.expect("a write with RETURNING is a source");
        let sql = CopyToStatement::new(source, CopyText).to_string();
        assert_eq!(sql, expected);
        let returned = &statement(&sql)["query"][kind]["returning_clause"]["exprs"];
        assert!(
            returned.as_array().is_some_and(|exprs| !exprs.is_empty()),
            "{sql}"
        );
    }
}

// [spec:pgorm:def:sql.ast.copy+3/test]    a SELECT whose write CTE returns nothing is a source, as the server holds
#[test]
fn copy_to_select_over_silent_write_cte() {
    let write = Query::delete()
        .from_table(Glyph::Table)
        .and_where(Expr::col(Glyph::Id).eq(1))
        .to_owned();
    let select = Query::select()
        .expr(Expr::val(1))
        .with(WithClause::new(CommonTableExpression::new(
            n("gone"),
            write,
        )))
        .to_owned();
    let source = CopySource::try_from(select.into_sub_query_statement())
        .expect("a SELECT is a source whatever its CTEs return");
    let sql = CopyToStatement::new(source, CopyCsv).to_string();
    assert_eq!(
        sql,
        [
            r#"COPY (WITH "gone" AS (DELETE FROM "glyph" WHERE "id" = 1) SELECT 1)"#,
            r#"TO STDOUT WITH (FORMAT csv)"#,
        ]
        .join(" ")
    );
    assert!(statement(&sql)["query"].get("SelectStmt").is_some());
}

// [spec:pgorm:def:sql.ast.copy+3/test]    check_source refuses a SELECT source's tie limit and windows, never a table or a write
#[test]
fn copy_check_source_reads_select_clauses() {
    let tied = Query::select()
        .column(Glyph::Image)
        .from(Glyph::Table)
        .limit_with_ties(2)
        .to_owned();
    std::assert_eq!(
        CopyToStatement::new(tied.clone(), CopyCsv).check_source(),
        Err(Error::TiesWithoutOrder)
    );
    let skipping = tied
        .clone()
        .order_by(Glyph::Aspect, Order::Asc)
        .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
        .to_owned();
    std::assert_eq!(
        CopyToStatement::new(skipping, CopyText).check_source(),
        Err(Error::TiesWithSkipLocked)
    );
    let ordered = tied.clone().order_by(Glyph::Aspect, Order::Asc).to_owned();
    std::assert_eq!(
        CopyToStatement::new(ordered, CopyBinary).check_source(),
        Ok(())
    );

    let windowed = Query::select()
        .expr_window_name(Func::count(Expr::col(Glyph::Id)), n("w"))
        .from(Glyph::Table)
        .to_owned();
    std::assert_eq!(
        CopyToStatement::new(windowed.clone(), CopyText).check_source(),
        Err(Error::WindowUndefined {
            name: "w".to_owned()
        })
    );
    // The same refusal through the conversion a `SubQueryStatement` takes.
    let source = CopySource::try_from(windowed.clone().into_sub_query_statement())
        .expect("a SELECT is a source");
    std::assert_eq!(
        CopyToStatement::new(source, CopyText).check_source(),
        Err(Error::WindowUndefined {
            name: "w".to_owned()
        })
    );
    let declared = windowed
        .clone()
        .window(n("w"), WindowStatement::partition_by(Glyph::Aspect))
        .to_owned();
    std::assert_eq!(
        CopyToStatement::new(declared, CopyText).check_source(),
        Ok(())
    );

    // A refused SELECT nested in the source is the server's to refuse.
    let nested = Query::select()
        .expr(Expr::val(1))
        .from_subquery(tied, n("t"))
        .to_owned();
    std::assert_eq!(
        CopyToStatement::new(nested, CopyText).check_source(),
        Ok(())
    );
    std::assert_eq!(
        CopyToStatement::new(Glyph::Table, CopyText).check_source(),
        Ok(())
    );
    let delete = Query::delete()
        .from_table(Glyph::Table)
        .returning_all()
        .to_owned();
    let source = CopySource::try_from(delete).expect("a write with RETURNING is a source");
    std::assert_eq!(
        CopyToStatement::new(source, CopyText).check_source(),
        Ok(())
    );
}

// [spec:pgorm:def:sql.ast.copy.parts/test]    the getters give back a COPY's table, column list and source
#[test]
fn copy_parts_read_back() {
    let bare = CopyFromStatement::new((n("art"), n("glyph")), CopyBinary);
    std::assert_eq!(bare.get_table(), &(n("art"), n("glyph")).into_table_name());
    std::assert_eq!(bare.get_columns(), &[] as &[Name]);
    let listed = bare.clone().columns([n("id"), n("image")]).to_owned();
    std::assert_eq!(listed.get_columns(), &[n("id"), n("image")]);

    let table = CopyToStatement::new(Glyph::Table, CopyText);
    std::assert_eq!(
        table.get_source().get_table(),
        Some(&Glyph::Table.into_table_name())
    );
    std::assert!(table.get_source().get_query().is_none());

    let select = Query::select()
        .column(Glyph::Id)
        .from(Glyph::Table)
        .to_owned();
    let query = CopyToStatement::new(select.clone(), CopyCsv);
    std::assert!(query.get_source().get_table().is_none());
    std::assert_eq!(
        query.get_source().get_query(),
        Some(&select.into_sub_query_statement())
    );
}
