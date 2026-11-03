use super::*;
use crate::oracle::{assert_eq, parsed_nodes};

const STATES: [(Deferrability, &str); 3] = [
    (Deferrability::NotDeferrable, "NOT DEFERRABLE"),
    (
        Deferrability::DeferrableInitiallyImmediate,
        "DEFERRABLE INITIALLY IMMEDIATE",
    ),
    (
        Deferrability::DeferrableInitiallyDeferred,
        "DEFERRABLE INITIALLY DEFERRED",
    ),
];

// [spec:pgorm:req:sql.ddl.deferrability+4/test]    a primary or unique key carries the clause
// after its column list and INCLUDE
// [spec:pgorm:req:sql.ddl.create-table+16/test]
#[test]
fn a_table_key_carries_its_deferrability() {
    for (deferrability, text) in STATES {
        let sql = Table::create(Glyph::Table)
            .col(ColumnDef::new(Glyph::Id).integer())
            .col(ColumnDef::new(Glyph::Aspect).integer())
            .col(ColumnDef::new(Glyph::Image).text())
            .primary_key(TableKey::new(Glyph::Id).deferrability(deferrability))
            .unique(
                TableKey::new(Glyph::Aspect)
                    .name(Name::runtime("glyph_aspect"))
                    .include([Glyph::Image])
                    .nulls_not_distinct()
                    .deferrability(deferrability),
            )
            .to_string();
        assert_eq!(
            sql,
            [
                r#"CREATE TABLE "glyph" ( "id" integer, "aspect" integer, "image" text,"#,
                &format!(r#"PRIMARY KEY ("id") {text},"#),
                &format!(
                    r#"CONSTRAINT "glyph_aspect" UNIQUE NULLS NOT DISTINCT ("aspect") INCLUDE ("image") {text} )"#
                ),
            ]
            .join(" ")
        );

        // A key folds the clause into its own constraint node.
        let constraints = parsed_nodes(&sql, "Constraint");
        assert_eq!(constraints.len(), 2, "{sql}");
        let deferrable = deferrability != Deferrability::NotDeferrable;
        let deferred = deferrability == Deferrability::DeferrableInitiallyDeferred;
        for constraint in &constraints {
            assert_eq!(constraint["deferrable"], deferrable, "{sql}");
            assert_eq!(constraint["initdeferred"], deferred, "{sql}");
        }
    }

    // A key that is not told otherwise says nothing about deferral.
    assert_eq!(
        Table::create(Glyph::Table)
            .col(ColumnDef::new(Glyph::Id).integer())
            .col(ColumnDef::new(Glyph::Aspect).integer())
            .primary_key(Glyph::Id)
            .unique(Glyph::Aspect)
            .to_string(),
        r#"CREATE TABLE "glyph" ( "id" integer, "aspect" integer, PRIMARY KEY ("id"), UNIQUE ("aspect") )"#
    );
}

// [spec:pgorm:req:sql.ddl.deferrability+4/test]    `ALTER TABLE` adds a key as
// `ADD UNIQUE (…)` / `ADD PRIMARY KEY (…)`, the clause after it
// [spec:pgorm:req:sql.ddl.alter-table+14/test]
#[test]
fn an_added_key_carries_its_deferrability() {
    let sql = Table::alter(Glyph::Table)
        .add_unique(
            TableKey::new(Glyph::Aspect).deferrability(Deferrability::DeferrableInitiallyDeferred),
        )
        .add_primary_key(
            TableKey::new(Glyph::Id).deferrability(Deferrability::DeferrableInitiallyImmediate),
        )
        .to_string();
    assert_eq!(
        sql,
        [
            r#"ALTER TABLE "glyph""#,
            r#"ADD UNIQUE ("aspect") DEFERRABLE INITIALLY DEFERRED,"#,
            r#"ADD PRIMARY KEY ("id") DEFERRABLE INITIALLY IMMEDIATE"#,
        ]
        .join(" ")
    );
    let constraints = parsed_nodes(&sql, "Constraint");
    assert_eq!(constraints.len(), 2, "{sql}");
    assert_eq!(constraints[0]["initdeferred"], true);
    assert_eq!(constraints[1]["deferrable"], true);
    assert_eq!(constraints[1]["initdeferred"], false);
}
