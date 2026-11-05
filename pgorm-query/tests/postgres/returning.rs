use super::*;
use crate::oracle::{assert_eq, parsed_nodes};
use ReturningRow::{New, Old};
use pg_query::protobuf::ReturningOptionKind;

fn update() -> UpdateStatement {
    Query::update()
        .table(Glyph::Table)
        .value(Glyph::Aspect, Expr::col(Glyph::Aspect).add(1))
        .and_where(Expr::col(Glyph::Id).eq(1))
        .to_owned()
}

fn update_head() -> &'static str {
    r#"UPDATE "glyph" SET "aspect" = "aspect" + 1 WHERE "id" = 1"#
}

/// The fields of each column reference the RETURNING list of `sql`'s one
/// `statement` (`UpdateStmt`, `DeleteStmt`) holds, in order, each field as
/// its string or `*`.
fn returned_refs(sql: &str, statement: &str) -> Vec<Vec<String>> {
    let found = parsed_nodes(sql, statement);
    assert_eq!(found.len(), 1, "exactly one {statement} in {sql}");
    found[0]["returning_clause"]["exprs"]
        .as_array()
        .expect("a RETURNING list")
        .iter()
        .map(|target| {
            target["ResTarget"]["val"]["ColumnRef"]["fields"]
                .as_array()
                .expect("a column reference")
                .iter()
                .map(|field| match field["String"]["sval"].as_str() {
                    Some(name) => name.to_owned(),
                    None => {
                        assert!(field.get("AStar").is_some(), "a name or a star: {field}");
                        "*".to_owned()
                    }
                })
                .collect()
        })
        .collect()
}

fn fields(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

/// Each `WITH (..)` option of `sql`'s RETURNING clause: which version it
/// renames, and the name.
fn renames(sql: &str) -> Vec<(ReturningOptionKind, String)> {
    parsed_nodes(sql, "ReturningOption")
        .iter()
        .map(|option| {
            let code = option["option"].as_i64().expect("an option kind");
            let kind = ReturningOptionKind::try_from(
                <i32 as TryFrom<i64>>::try_from(code).expect("a protobuf enum code fits i32"),
            )
            .expect("a returning option kind");
            let name = option["value"].as_str().expect("a name").to_owned();
            (kind, name)
        })
        .collect()
}

// [spec:pgorm:def:sql.ast.returning+3/test]    a `ReturningRow` paired with a column reads that
// version's column
// [spec:pgorm:req:sql.render.returning+4/test]    the version's keyword bare, then the quoted column
// [spec:pgorm:def:sql.types.column-ref+1/test]    `(ReturningRow, name)` converts into `RowColumn`
#[test]
fn old_and_new_columns_render_the_keyword_bare() {
    let sql = update()
        .returning(Query::returning().columns([(Old, Glyph::Aspect), (New, Glyph::Aspect)]))
        .to_string();
    assert_eq!(
        sql,
        format!(r#"{} RETURNING old."aspect", new."aspect""#, update_head())
    );
    assert_eq!(
        returned_refs(&sql, "UpdateStmt"),
        [fields(&["old", "aspect"]), fields(&["new", "aspect"])]
    );
}

// [spec:pgorm:def:sql.ast.returning+3/test]    the same parse as a quoted `"old"` qualifier: the
// typed form changes the AST, not what the server reads
#[test]
fn a_version_reads_as_the_quoted_name_would() {
    let typed = update()
        .returning(Query::returning().column((Old, Glyph::Aspect)))
        .to_string();
    let quoted = update()
        .returning(Query::returning().column((Name::runtime("old"), Glyph::Aspect)))
        .to_string();
    assert_eq!(
        quoted,
        format!(r#"{} RETURNING "old"."aspect""#, update_head())
    );
    assert_eq!(
        returned_refs(&typed, "UpdateStmt"),
        returned_refs(&quoted, "UpdateStmt")
    );
}

// [spec:pgorm:def:sql.types.column-ref+1/test]    `(ReturningRow, Asterisk)` converts into
// `RowAsterisk`
// [spec:pgorm:req:sql.render.returning+4/test]    `old.*`, `new.*`
#[test]
fn every_column_of_a_version() {
    let sql = Query::delete()
        .from_table(Glyph::Table)
        .and_where(Expr::col(Glyph::Id).eq(1))
        .returning(Query::returning().columns([(Old, Asterisk), (New, Asterisk)]))
        .to_string();
    assert_eq!(
        sql,
        r#"DELETE FROM "glyph" WHERE "id" = 1 RETURNING old.*, new.*"#
    );
    assert_eq!(
        returned_refs(&sql, "DeleteStmt"),
        [fields(&["old", "*"]), fields(&["new", "*"])]
    );
}

// [spec:pgorm:def:sql.ast.returning+3/test]    `old_as` and `new_as` rename the versions, OLD
// written first whichever was named first
// [spec:pgorm:req:sql.render.returning+4/test]    `WITH (OLD AS "o", NEW AS "n")` before the list
#[test]
fn renames_render_before_the_list() {
    let (before, after) = (alias("before"), alias("after"));
    let sql = update()
        .returning(
            Query::returning()
                .columns([(before, Glyph::Aspect), (after, Glyph::Aspect)])
                .new_as(after)
                .old_as(before),
        )
        .to_string();
    assert_eq!(
        sql,
        format!(
            r#"{} RETURNING WITH (OLD AS "before", NEW AS "after") "before"."aspect", "after"."aspect""#,
            update_head()
        )
    );
    assert_eq!(
        renames(&sql),
        [
            (ReturningOptionKind::ReturningOptionOld, "before".to_owned()),
            (ReturningOptionKind::ReturningOptionNew, "after".to_owned()),
        ]
    );
}

// [spec:pgorm:def:sql.ast.returning+3/test]    each version holds one name, the last call's, and
// a clause renaming nothing writes no `WITH`
#[test]
fn each_version_holds_one_name() {
    let sql = update()
        .returning(
            Query::returning()
                .column((alias("n"), Glyph::Aspect))
                .new_as(alias("first"))
                .new_as(alias("n")),
        )
        .to_string();
    assert_eq!(
        sql,
        format!(
            r#"{} RETURNING WITH (NEW AS "n") "n"."aspect""#,
            update_head()
        )
    );
    assert_eq!(
        renames(&sql),
        [(ReturningOptionKind::ReturningOptionNew, "n".to_owned())]
    );

    let plain = update().returning(Query::returning().all()).to_string();
    assert_eq!(plain, format!("{} RETURNING *", update_head()));
    assert!(renames(&plain).is_empty());
}

// [spec:pgorm:req:sql.render.returning+4/test]    the rename leads every list form: `*`, columns,
// expressions, on INSERT .. ON CONFLICT DO UPDATE and DELETE as on UPDATE
#[test]
fn every_statement_and_list_form_takes_a_rename() {
    let insert = Query::insert()
        .into_table(Glyph::Table)
        .columns([Glyph::Id, Glyph::Aspect])
        .values_panic([1.into(), 2.into()])
        .on_conflict(OnConflict::column(Glyph::Id).update_column(Glyph::Aspect))
        .returning(
            Query::returning()
                .exprs([
                    Expr::col((alias("o"), Glyph::Aspect)),
                    Expr::col((New, Glyph::Aspect)),
                ])
                .old_as(alias("o")),
        )
        .to_string();
    assert_eq!(
        insert,
        [
            r#"INSERT INTO "glyph" ("id", "aspect") VALUES (1, 2)"#,
            r#"ON CONFLICT ("id") DO UPDATE SET "aspect" = "excluded"."aspect""#,
            r#"RETURNING WITH (OLD AS "o") "o"."aspect", new."aspect""#,
        ]
        .join(" ")
    );

    let delete = Query::delete()
        .from_table(Glyph::Table)
        .returning(
            Query::returning()
                .all()
                .old_as(alias("o"))
                .new_as(alias("n")),
        )
        .to_string();
    assert_eq!(
        delete,
        r#"DELETE FROM "glyph" RETURNING WITH (OLD AS "o", NEW AS "n") *"#
    );
}

// [spec:pgorm:def:sql.ast.returning+3/test]    a value in a RETURNING expression over both
// versions is bound like any other
#[test]
fn a_value_beside_the_versions_is_bound() {
    let (sql, values) = update()
        .returning(
            Query::returning().expr(
                Expr::col((New, Glyph::Aspect))
                    .sub(Expr::col((Old, Glyph::Aspect)))
                    .eq(5),
            ),
        )
        .build();
    assert_eq!(
        sql,
        [
            r#"UPDATE "glyph" SET "aspect" = "aspect" + $1::int4 WHERE "id" = $2"#,
            r#"RETURNING new."aspect" - old."aspect" = $3"#,
        ]
        .join(" ")
    );
    assert_eq!(values, Values(vec![1.into(), 1.into(), 5.into()]));
}

// [spec:pgorm:def:sql.ast.returning+3/test]    `expr_as` / `exprs_as` return each expression under
// a name, the result column's
// [spec:pgorm:req:sql.render.returning+4/test]    `<expr> AS "<name>"`, after a rename where there is
// one, so a list can read both versions of a column apart
#[test]
fn named_items_name_their_result_columns() {
    let sql = update()
        .returning(
            Query::returning()
                .exprs_as([
                    (
                        Expr::col((alias("o"), Glyph::Aspect)).into(),
                        alias("before"),
                    ),
                    (Expr::col((New, Glyph::Aspect)).add(1), alias("after")),
                ])
                .old_as(alias("o")),
        )
        .to_string();
    assert_eq!(
        sql,
        [
            update_head(),
            r#"RETURNING WITH (OLD AS "o") "o"."aspect" AS "before","#,
            r#"new."aspect" + 1 AS "after""#,
        ]
        .join(" ")
    );
    let names: Vec<String> = parsed_nodes(&sql, "UpdateStmt")[0]["returning_clause"]["exprs"]
        .as_array()
        .expect("a RETURNING list")
        .iter()
        .map(|target| {
            target["ResTarget"]["name"]
                .as_str()
                .expect("a name")
                .to_owned()
        })
        .collect();
    assert_eq!(names, ["before", "after"]);

    assert_eq!(
        Query::delete()
            .from_table(Glyph::Table)
            .returning(Query::returning().expr_as(Expr::col(Glyph::Id), alias("gone")))
            .to_string(),
        r#"DELETE FROM "glyph" RETURNING "id" AS "gone""#
    );
}
