use super::*;
use crate::postgres::oracle::assert_eq;

// [spec:pgorm:req:sql.ddl.foreign-key+11/test]
#[test]
fn create_1() {
    assert_eq!(
        ForeignKey::create(Char::Table, Char::FontId, Font::Table, Font::Id)
            .name(Name::runtime("FK_2e303c3a712662f1fc2a4d0aad6"))
            .on_delete(ForeignKeyAction::Cascade)
            .on_update(ForeignKeyAction::Cascade)
            .to_string(),
        [
            r#"ALTER TABLE "character" ADD CONSTRAINT "FK_2e303c3a712662f1fc2a4d0aad6""#,
            r#"FOREIGN KEY ("font_id") REFERENCES "font" ("id")"#,
            r#"ON DELETE CASCADE ON UPDATE CASCADE"#,
        ]
        .join(" ")
    );
}

#[test]
fn create_2() {
    assert_eq!(
        ForeignKey::create(
            (Name::runtime("schema"), Char::Table),
            Char::FontId,
            Font::Table,
            Font::Id
        )
        .name(Name::runtime("FK_2e303c3a712662f1fc2a4d0aad6"))
        .on_delete(ForeignKeyAction::Cascade)
        .on_update(ForeignKeyAction::Cascade)
        .to_string(),
        [
            r#"ALTER TABLE "schema"."character" ADD CONSTRAINT "FK_2e303c3a712662f1fc2a4d0aad6""#,
            r#"FOREIGN KEY ("font_id") REFERENCES "font" ("id")"#,
            r#"ON DELETE CASCADE ON UPDATE CASCADE"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ddl.foreign-key+11/test]    all three check-timing states render, after the
// referential actions, and the default renders only when it is asked for
#[test]
fn deferrability_renders_after_the_referential_actions() {
    let created = |deferrability| {
        ForeignKey::create(Char::Table, Char::FontId, Font::Table, Font::Id)
            .name(Name::runtime("fk_font"))
            .on_delete(ForeignKeyAction::Cascade)
            .deferrability(deferrability)
            .to_string()
    };

    for (deferrability, tail) in [
        (Deferrability::NotDeferrable, "NOT DEFERRABLE"),
        (
            Deferrability::DeferrableInitiallyImmediate,
            "DEFERRABLE INITIALLY IMMEDIATE",
        ),
        (
            Deferrability::DeferrableInitiallyDeferred,
            "DEFERRABLE INITIALLY DEFERRED",
        ),
    ] {
        assert_eq!(
            created(deferrability),
            [
                r#"ALTER TABLE "character" ADD CONSTRAINT "fk_font""#,
                r#"FOREIGN KEY ("font_id") REFERENCES "font" ("id") ON DELETE CASCADE"#,
                tail,
            ]
            .join(" ")
        );
    }

    // Unset, the clause is absent rather than rendered as the server default.
    assert_eq!(
        ForeignKey::create(Char::Table, Char::FontId, Font::Table, Font::Id)
            .name(Name::runtime("fk_font"))
            .to_string(),
        [
            r#"ALTER TABLE "character" ADD CONSTRAINT "fk_font""#,
            r#"FOREIGN KEY ("font_id") REFERENCES "font" ("id")"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ddl.foreign-key+11/test]    a PERIOD pair closes both column lists, after
// every other pair whatever order the calls come in, in the standalone statement, inside CREATE
// TABLE and after ALTER TABLE's ADD alike
#[test]
fn a_period_pair_closes_both_column_lists() {
    let period = || {
        ForeignKey::create(Char::Table, Char::FontId, Font::Table, Font::Id)
            .name(Name::runtime("fk_font"))
            .period(Char::SizeW, Font::Name)
            .col(Char::Id, Font::Variant)
            .period(Char::SizeH, Font::Language)
            .deferrability(Deferrability::DeferrableInitiallyDeferred)
            .enforcement(Enforcement::NotEnforced)
            .to_owned()
    };
    let sql = period().to_string();
    assert_eq!(
        sql,
        [
            r#"ALTER TABLE "character" ADD CONSTRAINT "fk_font""#,
            r#"FOREIGN KEY ("font_id", "id", PERIOD "size_h")"#,
            r#"REFERENCES "font" ("id", "variant", PERIOD "language")"#,
            r#"DEFERRABLE INITIALLY DEFERRED NOT ENFORCED"#,
        ]
        .join(" "),
        "a later pair replaces the earlier"
    );
    let node = crate::postgres::oracle::parsed_nodes(&sql, "Constraint").remove(0);
    assert_eq!(node["fk_with_period"], true, "{sql}");
    assert_eq!(node["pk_with_period"], true, "{sql}");

    let key = period().get_foreign_key().to_owned();
    assert_eq!(key.get_columns(), ["font_id", "id"]);
    assert_eq!(key.get_ref_columns(), ["id", "variant"]);
    assert_eq!(
        key.get_period()
            .map(|(column, ref_column)| (column.to_string(), ref_column.to_string())),
        Some(("size_h".to_owned(), "language".to_owned()))
    );
    assert!(
        ForeignKey::create(Char::Table, Char::FontId, Font::Table, Font::Id)
            .get_foreign_key()
            .get_period()
            .is_none()
    );

    assert_eq!(
        Table::create(Char::Table)
            .col(ColumnDef::new(Char::FontId).integer())
            .foreign_key(period())
            .to_string(),
        [
            r#"CREATE TABLE "character" ( "font_id" integer, CONSTRAINT "fk_font""#,
            r#"FOREIGN KEY ("font_id", "id", PERIOD "size_h")"#,
            r#"REFERENCES "font" ("id", "variant", PERIOD "language")"#,
            r#"DEFERRABLE INITIALLY DEFERRED NOT ENFORCED )"#,
        ]
        .join(" ")
    );
    assert_eq!(
        Table::alter(Char::Table)
            .add_foreign_key(
                TableForeignKey::new(Char::Table, Char::FontId, Font::Table, Font::Id)
                    .period(Char::SizeW, Font::Name)
                    .to_owned()
            )
            .to_string(),
        [
            r#"ALTER TABLE "character" ADD"#,
            r#"FOREIGN KEY ("font_id", PERIOD "size_w") REFERENCES "font" ("id", PERIOD "name")"#,
        ]
        .join(" ")
    );
}
