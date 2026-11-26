use super::*;
use crate::postgres::oracle::{assert_eq, parsed_nodes};
use pgsql_syntax::protobuf::OnConflictAction;

fn insert(conflict: impl Into<OnConflict>) -> String {
    Query::insert()
        .into_table(Glyph::Table)
        .columns([Glyph::Aspect, Glyph::Image])
        .values_panic([1.into(), "a".into()])
        .on_conflict(conflict)
        .to_string()
}

fn key() -> Name {
    Name::runtime("GlyphAspectKey")
}

/// The `ON CONFLICT` clause of `sql`, as PostgreSQL's parser read it. Its
/// `infer` holds the arbiter, inferred or named.
fn conflict_clause(sql: &str) -> serde_json::Value {
    let mut found = parsed_nodes(sql, "on_conflict_clause");
    assert_eq!(found.len(), 1, "exactly one ON CONFLICT in {sql}");
    found.remove(0)
}

fn action(clause: &serde_json::Value) -> OnConflictAction {
    let code = clause["action"].as_i64().expect("an action code");
    OnConflictAction::try_from(code as i32).expect("a known action")
}

// [spec:pgorm:req:sql.ast.on-conflict+4/test]    a named constraint is an arbiter of its own,
// with no column list and no predicate
// [spec:pgorm:req:sql.render.on-conflict+2/test]    ` ON CONSTRAINT ` and the quoted name
#[test]
fn a_named_constraint_arbitrates_do_nothing() {
    let sql = insert(OnConflict::constraint(key()).do_nothing());
    assert_eq!(
        sql,
        [
            r#"INSERT INTO "glyph" ("aspect", "image") VALUES (1, 'a')"#,
            r#"ON CONFLICT ON CONSTRAINT "GlyphAspectKey" DO NOTHING"#,
        ]
        .join(" ")
    );
    let clause = conflict_clause(&sql);
    assert_eq!(action(&clause), OnConflictAction::OnconflictNothing);
    let infer = &clause["infer"];
    assert_eq!(infer["conname"], "GlyphAspectKey");
    assert!(
        infer["index_elems"].as_array().is_none_or(Vec::is_empty),
        "{sql}"
    );
    assert!(infer.get("where_clause").is_none(), "{sql}");
}

// [spec:pgorm:req:sql.ast.on-conflict+4/test]    both update transitions start from the named
// arbiter, and the update keeps its own filter
// [spec:pgorm:req:sql.render.on-conflict+2/test]
#[test]
fn a_named_constraint_arbitrates_do_update() {
    let sql = insert(
        OnConflict::constraint(key())
            .update_column(Glyph::Image)
            .value(
                Glyph::Aspect,
                Expr::col((Glyph::Table, Glyph::Aspect)).add(1),
            )
            .and_where(Expr::col((Glyph::Table, Glyph::Image)).ne("b")),
    );
    assert_eq!(
        sql,
        [
            r#"INSERT INTO "glyph" ("aspect", "image") VALUES (1, 'a')"#,
            r#"ON CONFLICT ON CONSTRAINT "GlyphAspectKey""#,
            r#"DO UPDATE SET "image" = "excluded"."image", "aspect" = "glyph"."aspect" + 1"#,
            r#"WHERE "glyph"."image" <> 'b'"#,
        ]
        .join(" ")
    );
    let clause = conflict_clause(&sql);
    assert_eq!(action(&clause), OnConflictAction::OnconflictUpdate);
    assert_eq!(clause["infer"]["conname"], "GlyphAspectKey");
    assert_eq!(clause["target_list"].as_array().map(Vec::len), Some(2));
    assert!(clause.get("where_clause").is_some(), "{sql}");

    let sql = insert(OnConflict::constraint(key()).value(Glyph::Image, "c"));
    assert!(
        sql.ends_with(r#"ON CONFLICT ON CONSTRAINT "GlyphAspectKey" DO UPDATE SET "image" = 'c'"#),
        "{sql}"
    );
}

// [spec:pgorm:req:sql.ast.on-conflict+4/test]    the arbiter a clause holds is the one it was
// built from, inferred or named
#[test]
fn the_clause_holds_its_arbiter() {
    let named: OnConflict = OnConflict::constraint(key())
        .update_column(Glyph::Image)
        .into();
    let OnConflict::Targeted { arbiter, .. } = named else {
        panic!("a named arbiter is a targeted clause");
    };
    assert_eq!(arbiter, ConflictArbiter::Constraint(key()));

    let inferred = OnConflict::column(Glyph::Id).do_nothing();
    let OnConflict::Targeted { arbiter, .. } = inferred else {
        panic!("an inferred arbiter is a targeted clause");
    };
    assert!(matches!(arbiter, ConflictArbiter::Inference(_)));

    assert_eq!(OnConflict::constraint(key()).name(), &key());
}

/// The column names of the inference target in `sql`, in order.
fn inferred_columns(sql: &str) -> Vec<String> {
    let clause = conflict_clause(sql);
    clause["infer"]["index_elems"]
        .as_array()
        .unwrap_or_else(|| panic!("an inference target in {sql}"))
        .iter()
        .map(|elem| {
            elem["IndexElem"]["name"]
                .as_str()
                .unwrap_or_else(|| panic!("a column entry in {sql}"))
                .to_owned()
        })
        .collect()
}

// [spec:pgorm:req:sql.ast.on-conflict+4/test]    one call names a composite key, every column in
// order, as the target the step-by-step chain builds
// [spec:pgorm:req:sql.render.on-conflict+2/test]
#[test]
fn a_tuple_names_every_key_column() {
    let sql = insert(OnConflict::columns((Glyph::Id, Glyph::Aspect, Glyph::Image)).do_nothing());
    assert_eq!(
        sql,
        [
            r#"INSERT INTO "glyph" ("aspect", "image") VALUES (1, 'a')"#,
            r#"ON CONFLICT ("id", "aspect", "image") DO NOTHING"#,
        ]
        .join(" ")
    );
    assert_eq!(inferred_columns(&sql), ["id", "aspect", "image"]);

    assert_eq!(
        OnConflict::columns((Glyph::Image, Glyph::Id)),
        OnConflict::column(Glyph::Image).and_column(Glyph::Id)
    );
    assert_eq!(
        OnConflict::columns(Glyph::Id),
        OnConflict::column(Glyph::Id)
    );

    let sql = insert(
        OnConflict::columns((Glyph::Id, Glyph::Aspect))
            .and_where(Expr::col(Glyph::Image).is_not_null())
            .update_column(Glyph::Image),
    );
    assert_eq!(inferred_columns(&sql), ["id", "aspect"]);
    assert!(
        sql.ends_with(
            r#"ON CONFLICT ("id", "aspect") WHERE "image" IS NOT NULL DO UPDATE SET "image" = "excluded"."image""#
        ),
        "{sql}"
    );
}
