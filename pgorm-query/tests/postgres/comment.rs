use super::*;
use crate::postgres::oracle::assert_eq;

// [spec:pgorm:req:sql.ddl.comment+5/test]    both targets render, at every level of qualification
#[test]
fn comment_statements_render_their_targets() {
    assert_eq!(
        Comment::on_table(Glyph::Table, "one row per glyph").to_string(),
        r#"COMMENT ON TABLE "glyph" IS 'one row per glyph'"#
    );
    assert_eq!(
        Comment::on_table((Name::runtime("public"), Glyph::Table), "qualified").to_string(),
        r#"COMMENT ON TABLE "public"."glyph" IS 'qualified'"#
    );
    assert_eq!(
        Comment::on_column(Glyph::Table, Glyph::Aspect, "the ratio").to_string(),
        r#"COMMENT ON COLUMN "glyph"."aspect" IS 'the ratio'"#
    );
    assert_eq!(
        Comment::on_column(
            (Name::runtime("public"), Glyph::Table),
            Glyph::Aspect,
            "the ratio"
        )
        .to_string(),
        r#"COMMENT ON COLUMN "public"."glyph"."aspect" IS 'the ratio'"#
    );

    // The target and the unescaped text are readable back off the statement.
    let statement = Comment::on_column(Glyph::Table, Glyph::Aspect, "it's fine");
    assert_eq!(statement.get_comment(), "it's fine");
    match statement.get_target() {
        CommentTarget::Column(TableName::Table(table), column) => {
            assert_eq!(table.to_string(), "glyph");
            assert_eq!(column.to_string(), "aspect");
        }
        other => panic!("expected a bare-table column target, got {other:?}"),
    }
}

// [spec:pgorm:req:sql.ddl.comment+5/test]    comment text is a string literal whose reading does
// not depend on a server setting: quotes doubled, and an E-string with doubled
// backslashes as soon as there is a backslash to read
#[test]
fn comment_text_is_a_quoted_literal() {
    assert_eq!(
        Comment::on_table(Glyph::Table, "it's a 'quoted' word").to_string(),
        r#"COMMENT ON TABLE "glyph" IS 'it''s a ''quoted'' word'"#
    );

    // A backslash is what the two readings of a literal disagree about, so text
    // holding one is written as an E-string saying what it is.
    assert_eq!(
        Comment::on_table(Glyph::Table, r"C:\glyphs\ or \n").to_string(),
        r#"COMMENT ON TABLE "glyph" IS E'C:\\glyphs\\ or \\n'"#
    );

    // A statement-terminating attempt stays inside the literal.
    assert_eq!(
        Comment::on_table(Glyph::Table, "'; DROP TABLE glyph; --").to_string(),
        r#"COMMENT ON TABLE "glyph" IS '''; DROP TABLE glyph; --'"#
    );

    // The same attempt led by a backslash is what doubling alone cannot hold:
    // under `standard_conforming_strings = off` the `\` would escape the first
    // of the doubled quotes, leaving the second to close the literal and the
    // rest to be read as SQL. The E-string spells the backslash out, so both
    // readings agree the quote is content.
    let rendered = Comment::on_table(Glyph::Table, r"\'; DROP TABLE glyph; --").to_string();
    assert_eq!(
        rendered,
        r#"COMMENT ON TABLE "glyph" IS E'\\''; DROP TABLE glyph; --'"#
    );
    let body = rendered
        .split_once("IS E'")
        .and_then(|(_, rest)| rest.strip_suffix('\''))
        .expect("the literal should be an E-string");
    assert!(
        !body.replace(r"\\", "").contains('\\'),
        "every backslash must itself be escaped, or one could escape a quote: {rendered}"
    );

    assert_eq!(
        Comment::on_table(Glyph::Table, "").to_string(),
        r#"COMMENT ON TABLE "glyph" IS ''"#
    );

    // Identifiers keep their own quoting rule: embedded double quotes are doubled.
    assert_eq!(
        Comment::on_column(
            Name::runtime(r#"gl"yph"#),
            Name::runtime(r#"as"pect"#),
            "odd"
        )
        .to_string(),
        r#"COMMENT ON COLUMN "gl""yph"."as""pect" IS 'odd'"#
    );
}

// [spec:pgorm:req:sql.ddl.comment+5/test]    one `TableName` value serves a comment target and a
// DDL target, so a comment cannot name a table the DDL beside it could not
#[test]
fn comment_and_ddl_share_one_table_name() {
    let name = (Name::runtime("public"), Glyph::Table).into_table_name();

    assert_eq!(
        Comment::on_table(name.clone(), "shared").to_string(),
        r#"COMMENT ON TABLE "public"."glyph" IS 'shared'"#
    );
    assert_eq!(
        Table::truncate(name.clone()).to_string(),
        r#"TRUNCATE TABLE "public"."glyph""#
    );
    assert_eq!(
        name.schema().map(|s| s.to_string()).as_deref(),
        Some("public")
    );
    assert_eq!(name.table().to_string(), "glyph");
}

// [spec:pgorm:req:sql.ddl.comment+5/test]    a create statement renders the comments it carries,
// table first then columns in order, each on the statement's own table
#[test]
fn create_statement_renders_the_comments_it_carries() {
    let table = Table::create((Name::runtime("public"), Glyph::Table))
        .comment("one row per glyph")
        .col(ColumnDef::new(Glyph::Id).integer().not_null())
        .col(ColumnDef::new(Glyph::Aspect).integer().comment("the ratio"))
        .col(ColumnDef::new(Glyph::Image).string().comment("it's a path"))
        .to_owned();

    assert_eq!(
        table
            .comments()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec![
            r#"COMMENT ON TABLE "public"."glyph" IS 'one row per glyph'"#.to_owned(),
            r#"COMMENT ON COLUMN "public"."glyph"."aspect" IS 'the ratio'"#.to_owned(),
            r#"COMMENT ON COLUMN "public"."glyph"."image" IS 'it''s a path'"#.to_owned(),
        ]
    );

    // The create statement's own SQL holds one command, so it can be prepared:
    // the comments ride beside it rather than inside it.
    let sql = table.to_string();
    assert!(
        !sql.contains("COMMENT"),
        "create SQL carries no comment: {sql}"
    );
    assert!(!sql.contains(';'), "create SQL is a single command: {sql}");

    // A statement with nothing commented yields nothing.
    assert!(
        Table::create(Glyph::Table)
            .col(ColumnDef::new(Glyph::Id).integer())
            .comments()
            .is_empty()
    );
}
