//! The relation lists a write statement carries: UPDATE's `FROM` and
//! DELETE's `USING`.

use super::*;
use crate::postgres::oracle::assert_eq;

// [spec:pgorm:req:sql.ast.update+5/test]      the FROM relation list
// [spec:pgorm:req:sql.render.update-delete+3/test]
#[test]
fn update_from_joins_a_second_table() {
    assert_eq!(
        Query::update()
            .table(Char::Table)
            .value(Char::FontSize, Expr::col((Font::Table, Font::Id)))
            .from(Font::Table)
            .and_where(Expr::col((Char::Table, Char::FontId)).equals((Font::Table, Font::Id)))
            .to_string(),
        [
            r#"UPDATE "character" SET "font_size" = "font"."id" FROM "font""#,
            r#"WHERE "character"."font_id" = "font"."id""#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ast.update+5/test]      repeated calls accumulate
// [spec:pgorm:req:sql.render.update-delete+3/test]
#[test]
fn update_from_accumulates_a_relation_list() {
    assert_eq!(
        Query::update()
            .table(Char::Table)
            .value(Char::FontSize, 12)
            .from(Font::Table)
            .from(Glyph::Table)
            .and_where(Expr::col((Char::Table, Char::FontId)).equals((Font::Table, Font::Id)))
            .and_where(Expr::col((Glyph::Table, Glyph::Id)).eq(1))
            .to_string(),
        [
            r#"UPDATE "character" SET "font_size" = 12 FROM "font", "glyph""#,
            r#"WHERE "character"."font_id" = "font"."id" AND "glyph"."id" = 1"#,
        ]
        .join(" ")
    );
}

// The relation currency is SELECT's, so every `FromItem` variant renders in a
// write statement exactly as it does in a SELECT.
// [spec:pgorm:req:sql.ast.update+5/test]
// [spec:pgorm:req:sql.render.update-delete+3/test]
#[test]
fn update_from_takes_every_from_item_variant() {
    assert_eq!(
        Query::update()
            .table(Char::Table)
            .value(Char::FontSize, 12)
            .from(FromItem::SubQuery(
                Query::select().column(Font::Id).from(Font::Table).take(),
                Name::runtime("f"),
            ))
            .from(FromItem::ValuesList(
                vec![1i32.into_value_tuple(), 2i32.into_value_tuple()],
                Name::runtime("v"),
            ))
            .and_where(
                Expr::col((Char::Table, Char::FontId)).equals((Name::runtime("f"), Font::Id))
            )
            .to_string(),
        [
            r#"UPDATE "character" SET "font_size" = 12"#,
            r#"FROM (SELECT "id" FROM "font") AS "f", (VALUES (1), (2)) AS "v""#,
            r#"WHERE "character"."font_id" = "f"."id""#,
        ]
        .join(" ")
    );
}

// A `FromItem::Template` fragment carries its own `$N` markers, which must
// renumber into the update's parameter space — the update's own bound value
// takes `$1` because SET renders before FROM.
// [spec:pgorm:req:sql.ast.update+5/test]
// [spec:pgorm:req:sql.render.update-delete+3/test]
#[test]
fn update_from_renumbers_a_template_fragment() {
    let fragment = SqlTemplate::from_sql(
        r#"SELECT "id" FROM "font" WHERE "language" = $1"#,
        ["en".into()],
    )
    .expect("the fragment is valid SQL");

    let (sql, values) = Query::update()
        .table(Char::Table)
        .value(Char::FontSize, 12)
        .from(FromItem::Template(fragment, Name::runtime("f")))
        .and_where(Expr::col((Char::Table, Char::FontId)).equals((Name::runtime("f"), Font::Id)))
        .build();

    assert_eq!(
        sql,
        "UPDATE \"character\" SET \"font_size\" = $1 FROM \
         (SELECT \"id\" FROM \"font\" WHERE \"language\" = $2\n\
         ) AS \"f\" WHERE \"character\".\"font_id\" = \"f\".\"id\""
    );
    assert_eq!(values.0, vec![12i32.into(), "en".into()]);
}

// An empty relation list renders nothing — not a bare keyword — so a
// single-table write is byte-identical to what it rendered before the clause
// existed.
// [spec:pgorm:req:sql.render.update-delete+3/test]
#[test]
fn empty_from_and_using_render_no_keyword() {
    assert_eq!(
        Query::update()
            .table(Glyph::Table)
            .value(Glyph::Aspect, 1)
            .to_string(),
        r#"UPDATE "glyph" SET "aspect" = 1"#
    );
    assert_eq!(
        Query::delete().from_table(Glyph::Table).to_string(),
        r#"DELETE FROM "glyph""#
    );
}

// [spec:pgorm:def:sql.ast.delete+4/test]      the USING relation list
// [spec:pgorm:req:sql.render.update-delete+3/test]
#[test]
fn delete_using_joins_a_second_table() {
    assert_eq!(
        Query::delete()
            .from_table(Char::Table)
            .using(Font::Table)
            .and_where(Expr::col((Char::Table, Char::FontId)).equals((Font::Table, Font::Id)))
            .and_where(Expr::col((Font::Table, Font::Language)).eq("en"))
            .to_string(),
        [
            r#"DELETE FROM "character" USING "font""#,
            r#"WHERE "character"."font_id" = "font"."id" AND "font"."language" = 'en'"#,
        ]
        .join(" ")
    );
}

// USING accumulates and takes the same currency FROM does, RETURNING still
// last.
// [spec:pgorm:def:sql.ast.delete+4/test]
// [spec:pgorm:req:sql.render.update-delete+3/test]
#[test]
fn delete_using_accumulates_and_precedes_returning() {
    assert_eq!(
        Query::delete()
            .from_table(Char::Table)
            .using(Font::Table)
            .using(FromItem::SubQuery(
                Query::select().column(Glyph::Id).from(Glyph::Table).take(),
                Name::runtime("g"),
            ))
            .and_where(Expr::col((Char::Table, Char::FontId)).equals((Font::Table, Font::Id)))
            .returning_col((Char::Table, Char::Id))
            .to_string(),
        [
            r#"DELETE FROM "character" USING "font", (SELECT "id" FROM "glyph") AS "g""#,
            r#"WHERE "character"."font_id" = "font"."id" RETURNING "character"."id""#,
        ]
        .join(" ")
    );
}

// An aliased target and an aliased relation coexist, which is the only way to
// write a self-join on a write statement.
// [spec:pgorm:def:sql.ast.delete+4/test]
// [spec:pgorm:req:sql.ast.update+5/test]
#[test]
fn write_relations_alias_alongside_an_aliased_target() {
    assert_eq!(
        Query::delete()
            .from_table(Glyph::Table.into_named_table().alias(Name::runtime("a")))
            .using(Glyph::Table.into_named_table().alias(Name::runtime("b")))
            .and_where(
                Expr::col((Name::runtime("a"), Glyph::Image))
                    .equals((Name::runtime("b"), Glyph::Image))
            )
            .and_where(
                Expr::col((Name::runtime("a"), Glyph::Id))
                    .gt(Expr::col((Name::runtime("b"), Glyph::Id)))
            )
            .to_string(),
        [
            r#"DELETE FROM "glyph" AS "a" USING "glyph" AS "b""#,
            r#"WHERE "a"."image" = "b"."image" AND "a"."id" > "b"."id""#,
        ]
        .join(" ")
    );
}
