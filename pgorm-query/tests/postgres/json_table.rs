//! `JSON_TABLE` as a FROM item: its columns, paths and clauses, rendered and
//! held to PostgreSQL 19's grammar.

use super::*;
use crate::oracle::{assert_eq, parsed_nodes};

fn data() -> Expr {
    Expr::col((Name::runtime("d"), Name::runtime("doc")))
}

// [spec:pgorm:def:sql.ast.json-table+2/test]    every column kind, a nested path, PASSING and
// ON ERROR
// [spec:pgorm:req:sql.render.json-table/test]    the columns in order, the paths as literals
// under build(), the PASSING value bound
#[test]
fn a_json_table_renders_every_column_kind() {
    let table = Func::json_table(
        data(),
        "$.items[*] ? (@.n > $min)",
        JsonTableColumn::ordinality(Name::runtime("i")),
    )
    .path_name(Name::runtime("items"))
    .passing(1, Name::runtime("min"))
    .column(JsonTableColumn::value(Name::runtime("n"), ColumnType::Integer).path("$.n"))
    .column(
        JsonTableColumn::value(Name::runtime("label"), ColumnType::Text)
            .on_empty(JsonValueBehavior::Default("none".into()))
            .on_error(JsonValueBehavior::Error),
    )
    .column(
        JsonTableColumn::query(Name::runtime("tags"), ColumnType::JsonBinary)
            .path("$.tags[*]")
            .with_wrapper()
            .on_empty(JsonQueryBehavior::EmptyArray),
    )
    .column(
        JsonTableColumn::exists(Name::runtime("flagged"), ColumnType::Boolean)
            .path("strict $.flag")
            .on_error(JsonExistsBehavior::False),
    )
    .column(
        JsonTableColumn::nested(
            "$.parts[*]",
            JsonTableColumn::value(Name::runtime("part"), ColumnType::Text).path("$"),
        )
        .path_name(Name::runtime("parts"))
        .column(JsonTableColumn::ordinality(Name::runtime("p"))),
    )
    .on_error(JsonTableBehavior::Error);

    let query = Query::select()
        .column(Asterisk)
        .from_as(Name::runtime("docs"), Name::runtime("d"))
        .from(table.alias(Name::runtime("jt")))
        .to_owned();
    let expected = |path: &str, min: &str| {
        [
            r#"SELECT * FROM "docs" AS "d", JSON_TABLE("d"."doc", "#,
            path,
            r#" AS "items" PASSING "#,
            min,
            r#" AS "min" COLUMNS ("i" FOR ORDINALITY, "n" integer PATH '$.n',"#,
            r#" "label" text DEFAULT 'none' ON EMPTY ERROR ON ERROR,"#,
            r#" "tags" jsonb FORMAT JSON PATH '$.tags[*]' WITH UNCONDITIONAL WRAPPER EMPTY ARRAY ON EMPTY,"#,
            r#" "flagged" bool EXISTS PATH 'strict $.flag' FALSE ON ERROR,"#,
            r#" NESTED PATH '$.parts[*]' AS "parts" COLUMNS ("part" text PATH '$', "p" FOR ORDINALITY))"#,
            r#" ERROR ON ERROR) AS "jt""#,
        ]
        .concat()
    };
    assert_eq!(
        query.to_string(),
        expected("'$.items[*] ? (@.n > $min)'", "1::int4")
    );
    let (sql, values) = query.build();
    assert_eq!(sql, expected("'$.items[*] ? (@.n > $min)'", "$1::int4"));
    assert_eq!(
        values,
        Values(vec![1i32.into()]),
        "only the PASSING value is bound"
    );

    let parsed = parsed_nodes(&sql, "JsonTable");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0]["alias"]["aliasname"], "jt");
}

// A path holding a quote or a backslash is one literal, escaped the way the
// value pipeline writes an inlined string.
// [spec:pgorm:req:sql.render.json-table/test]    paths escaped, never interpolated
#[test]
fn a_hostile_path_stays_one_literal() {
    let hostile = r#"$."it's" ? (@ like_regex "\\d')) --")"#;
    let sql = Query::select()
        .column(Asterisk)
        .from(
            Func::json_table(
                data(),
                hostile,
                JsonTableColumn::value(Name::runtime("v"), ColumnType::Text).path(hostile),
            )
            .alias(Name::runtime("jt")),
        )
        .build()
        .0;
    let literal = r#"E'$."it\'s" ? (@ like_regex "\\\\d\')) --")'"#;
    assert_eq!(
        sql,
        format!(
            r#"SELECT * FROM JSON_TABLE("d"."doc", {literal} COLUMNS ("v" text PATH {literal})) AS "jt""#
        )
    );
    let table = &parsed_nodes(&sql, "JsonTable")[0];
    assert_eq!(
        table["pathspec"]["string"]["AConst"]["val"]["Sval"]["sval"], hostile,
        "{table}"
    );
}
