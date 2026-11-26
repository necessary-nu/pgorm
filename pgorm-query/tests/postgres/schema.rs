use super::*;
use crate::postgres::oracle::{assert_eq, assert_query_eq};
use pgorm_query::extension::{Extension, Type};

// [spec:pgorm:req:sql.ddl+12/test]    the whole DDL surface is reachable through the six
// entry-point helpers
#[test]
fn every_ddl_entry_point_is_reachable() {
    assert!(
        Table::create(Glyph::Table)
            .col(ColumnDef::new(Glyph::Id).integer())
            .to_string()
            .starts_with("CREATE TABLE")
    );
    assert!(
        Table::alter(Glyph::Table)
            .drop_column(Glyph::Id)
            .to_string()
            .starts_with("ALTER TABLE")
    );
    assert!(
        Table::drop(Glyph::Table)
            .to_string()
            .starts_with("DROP TABLE")
    );
    assert!(
        Table::rename(Glyph::Table, Name::runtime("g"))
            .to_string()
            .starts_with("ALTER TABLE")
    );
    assert!(
        Table::truncate(Glyph::Table)
            .to_string()
            .starts_with("TRUNCATE TABLE")
    );

    assert!(
        Index::create(Glyph::Table, Glyph::Id)
            .name(Name::runtime("idx"))
            .to_string()
            .starts_with("CREATE INDEX")
    );
    assert!(
        Index::drop(Name::runtime("idx"))
            .to_string()
            .starts_with("DROP INDEX")
    );

    assert!(
        ForeignKey::create(Char::Table, Char::FontId, Font::Table, Font::Id)
            .name(Name::runtime("fk"))
            .to_string()
            .starts_with("ALTER TABLE")
    );
    assert!(
        Table::rename_constraint(Char::Table, Name::runtime("fk"), Name::runtime("fk2"))
            .to_string()
            .starts_with("ALTER TABLE")
    );

    assert!(
        Type::create(Name::runtime("tea"))
            .values(["green"])
            .to_string()
            .starts_with("CREATE TYPE")
    );
    assert!(
        Type::alter(Name::runtime("tea"))
            .add_value("black")
            .to_string()
            .starts_with("ALTER TYPE")
    );
    assert!(
        Type::drop(Name::runtime("tea"))
            .to_string()
            .starts_with("DROP TYPE")
    );

    assert!(
        Domain::create(Name::runtime("quantity"), ColumnType::Integer)
            .to_string()
            .starts_with("CREATE DOMAIN")
    );
    assert!(
        Domain::alter(Name::runtime("quantity"))
            .drop_default()
            .to_string()
            .starts_with("ALTER DOMAIN")
    );
    assert!(
        Domain::drop(Name::runtime("quantity"))
            .to_string()
            .starts_with("DROP DOMAIN")
    );

    assert!(
        Schema::create(Name::runtime("app"))
            .to_string()
            .starts_with("CREATE SCHEMA")
    );
    assert!(
        Schema::drop(Name::runtime("app"))
            .to_string()
            .starts_with("DROP SCHEMA")
    );
    assert!(
        Schema::rename(Name::runtime("app"), Name::runtime("core"))
            .to_string()
            .starts_with("ALTER SCHEMA")
    );

    assert!(
        Extension::create(Name::runtime("ltree"))
            .to_string()
            .starts_with("CREATE EXTENSION")
    );
    assert!(
        Extension::drop(Name::runtime("ltree"))
            .to_string()
            .starts_with("DROP EXTENSION")
    );

    assert!(
        Comment::on_table(Glyph::Table, "glyphs")
            .to_string()
            .starts_with("COMMENT ON TABLE")
    );
    assert!(
        Comment::on_column(Glyph::Table, Glyph::Id, "the id")
            .to_string()
            .starts_with("COMMENT ON COLUMN")
    );
}

// [spec:pgorm:req:sql.ddl+12/test]    table, index, foreign-key and comment statements render
// through their `Display`, which delegates to the corresponding `prepare_*` method on the single
// Postgres `QueryBuilder`
#[test]
fn ddl_statements_render_through_display() {
    fn assert_renders<S: std::fmt::Display>(statement: &S, expected: &str) {
        assert_query_eq(&statement.to_string(), expected);
    }

    assert_renders(
        Table::create(Glyph::Table).col(ColumnDef::new(Glyph::Id).integer()),
        r#"CREATE TABLE "glyph" ( "id" integer )"#,
    );
    assert_renders(
        &Table::alter(Glyph::Table).drop_column(Glyph::Id),
        r#"ALTER TABLE "glyph" DROP COLUMN "id""#,
    );
    assert_renders(&Table::drop(Glyph::Table), r#"DROP TABLE "glyph""#);
    assert_renders(
        &Table::rename(Glyph::Table, Name::runtime("g")),
        r#"ALTER TABLE "glyph" RENAME TO "g""#,
    );
    assert_renders(&Table::truncate(Glyph::Table), r#"TRUNCATE TABLE "glyph""#);
    assert_renders(
        Index::create(Glyph::Table, Glyph::Id).name(Name::runtime("idx")),
        r#"CREATE INDEX "idx" ON "glyph" ("id")"#,
    );
    assert_renders(&Index::drop(Name::runtime("idx")), r#"DROP INDEX "idx""#);
    assert_renders(
        ForeignKey::create(Char::Table, Char::FontId, Font::Table, Font::Id)
            .name(Name::runtime("fk")),
        r#"ALTER TABLE "character" ADD CONSTRAINT "fk" FOREIGN KEY ("font_id") REFERENCES "font" ("id")"#,
    );
    assert_renders(
        &Table::rename_constraint(Char::Table, Name::runtime("fk"), Name::runtime("fk2")),
        r#"ALTER TABLE "character" RENAME CONSTRAINT "fk" TO "fk2""#,
    );
    assert_renders(
        &Comment::on_table(Glyph::Table, "glyphs"),
        r#"COMMENT ON TABLE "glyph" IS 'glyphs'"#,
    );
}

// [spec:pgorm:req:sql.ddl+12/test]    identifiers render double-quoted, with embedded quotes doubled
#[test]
fn ddl_identifiers_are_double_quoted() {
    assert_eq!(
        Table::create(Name::runtime(r#"he"llo"#))
            .col(ColumnDef::new(Name::runtime(r#"wor"ld"#)).integer())
            .to_string(),
        r#"CREATE TABLE "he""llo" ( "wor""ld" integer )"#
    );
    assert_eq!(
        Index::create((Name::runtime("schema"), Glyph::Table), Glyph::Id)
            .name(Name::runtime("idx"))
            .to_string(),
        r#"CREATE INDEX "idx" ON "schema"."glyph" ("id")"#
    );
    assert_eq!(
        ForeignKey::create(Char::Table, Char::FontId, Font::Table, Font::Id)
            .name(Name::runtime("fk"))
            .to_string(),
        r#"ALTER TABLE "character" ADD CONSTRAINT "fk" FOREIGN KEY ("font_id") REFERENCES "font" ("id")"#
    );
}

// [spec:pgorm:req:sql.ddl+12/test]    index and constraint names are idens, so an embedded
// quote is doubled at every site that writes one
#[test]
fn ddl_index_and_constraint_names_escape_quotes() {
    assert_eq!(
        Table::create(Glyph::Table)
            .col(ColumnDef::new(Glyph::Id).integer())
            .unique(TableKey::new(Glyph::Id).name(Name::runtime(r#"i"dx"#)))
            .to_string(),
        r#"CREATE TABLE "glyph" ( "id" integer, CONSTRAINT "i""dx" UNIQUE ("id") )"#
    );
    assert_eq!(
        Index::create(Glyph::Table, Glyph::Id)
            .name(Name::runtime(r#"i"dx"#))
            .to_string(),
        r#"CREATE INDEX "i""dx" ON "glyph" ("id")"#.to_owned()
    );
    assert_eq!(
        Index::drop(Name::runtime(r#"i"dx"#)).to_string(),
        r#"DROP INDEX "i""dx""#
    );
    assert_eq!(
        ForeignKey::create(Char::Table, Char::FontId, Font::Table, Font::Id)
            .name(Name::runtime(r#"f"k"#))
            .to_string(),
        r#"ALTER TABLE "character" ADD CONSTRAINT "f""k" FOREIGN KEY ("font_id") REFERENCES "font" ("id")"#
    );
    assert_eq!(
        Table::rename_constraint(
            Char::Table,
            Name::runtime(r#"f"k"#),
            Name::runtime(r#"k"f"#)
        )
        .to_string(),
        r#"ALTER TABLE "character" RENAME CONSTRAINT "f""k" TO "k""f""#
    );
    assert_eq!(
        Table::alter(Char::Table)
            .drop_constraint(Name::runtime(r#"f"k"#))
            .to_string(),
        r#"ALTER TABLE "character" DROP CONSTRAINT "f""k""#
    );
}
