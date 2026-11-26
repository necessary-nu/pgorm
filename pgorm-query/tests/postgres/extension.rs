use super::*;
use crate::postgres::oracle::assert_eq;
use pgorm_query::extension::{Extension, PgLTree};

// [spec:pgorm:req:sql.ddl.extension+5/test]    every part of the CREATE EXTENSION grammar
#[test]
fn create_1() {
    assert_eq!(
        Extension::create(Name::runtime("ltree")).to_string(),
        r#"CREATE EXTENSION "ltree""#
    );
}

// [spec:pgorm:req:sql.ddl.extension+5/test]
#[test]
fn create_2() {
    assert_eq!(
        Extension::create(Name::runtime("ltree"))
            .schema(Name::runtime("public"))
            .version("v0.1.0")
            .cascade()
            .if_not_exists()
            .to_string(),
        r#"CREATE EXTENSION IF NOT EXISTS "ltree" WITH SCHEMA "public" VERSION 'v0.1.0' CASCADE"#
    );
}

// [spec:pgorm:req:sql.ddl.extension+5/test]    `PgLTree` is a ready-made `SqlName` rendering `ltree`,
// usable as an extension name through `From<PgLTree> for String`
#[test]
fn create_3() {
    assert_eq!(SqlName::to_string(&PgLTree), "ltree");
    assert_eq!(
        Extension::create(PgLTree).to_string(),
        r#"CREATE EXTENSION "ltree""#
    );

    // The matching column type is `ColumnType::LTree`.
    assert_eq!(
        Table::create(Glyph::Table)
            .col(ColumnDef::new(Glyph::Tokens).ltree())
            .to_string(),
        r#"CREATE TABLE "glyph" ( "tokens" ltree )"#
    );
}

// [spec:pgorm:req:sql.ddl.extension+5/test]    on drop, `cascade` and `restrict` share one slot,
// so the last call wins and the pair PostgreSQL rejects cannot be built
#[test]
fn drop_1() {
    assert_eq!(
        Extension::drop(Name::runtime("ltree")).to_string(),
        r#"DROP EXTENSION "ltree""#
    );
    assert_eq!(
        Extension::drop(Name::runtime("ltree"))
            .if_exists()
            .cascade()
            .to_string(),
        r#"DROP EXTENSION IF EXISTS "ltree" CASCADE"#
    );
    assert_eq!(
        Extension::drop(Name::runtime("ltree"))
            .restrict()
            .to_string(),
        r#"DROP EXTENSION "ltree" RESTRICT"#
    );
    assert_eq!(
        Extension::drop(Name::runtime("ltree"))
            .restrict()
            .cascade()
            .to_string(),
        r#"DROP EXTENSION "ltree" CASCADE"#
    );
}

// [spec:pgorm:sem:sql.render.ddl.extension+3/test]    name and schema are quoted identifiers and
// version a quoted literal, so an embedded quote escapes rather than ending the token
#[test]
fn extension_strings_are_quoted() {
    let mut sql = SqlWriterValues::new("$", true);
    let statement = Extension::create(Name::runtime(r#"pg"weird ext"#))
        .schema(Name::runtime("my schema"))
        .version("1.0; --")
        .to_owned();

    assert_eq!(
        statement.build_collect(&mut sql),
        r#"CREATE EXTENSION "pg""weird ext" WITH SCHEMA "my schema" VERSION '1.0; --'"#
    );
    // Nothing was parameterized on the way through a placeholder-emitting sink.
    let (_, values) = sql.into_parts();
    assert_eq!(values, Values(vec![]));

    assert_eq!(
        Extension::drop(Name::runtime(r#"pg"weird ext"#)).to_string(),
        r#"DROP EXTENSION "pg""weird ext""#
    );
}
