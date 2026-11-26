use super::*;
use crate::postgres::oracle::assert_eq;

// [spec:pgorm:sem:sql.render.locking+1/test]    the four lock types
#[test]
fn locking_1() {
    let locked = |lock: LockType| {
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .lock(lock)
            .to_string()
    };

    assert_eq!(
        locked(LockType::Update),
        r#"SELECT "id" FROM "glyph" FOR UPDATE"#
    );
    assert_eq!(
        locked(LockType::NoKeyUpdate),
        r#"SELECT "id" FROM "glyph" FOR NO KEY UPDATE"#
    );
    assert_eq!(
        locked(LockType::Share),
        r#"SELECT "id" FROM "glyph" FOR SHARE"#
    );
    assert_eq!(
        locked(LockType::KeyShare),
        r#"SELECT "id" FROM "glyph" FOR KEY SHARE"#
    );

    // The named shorthands.
    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .lock_exclusive()
            .to_string(),
        r#"SELECT "id" FROM "glyph" FOR UPDATE"#
    );
    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .lock_shared()
            .to_string(),
        r#"SELECT "id" FROM "glyph" FOR SHARE"#
    );
}

// [spec:pgorm:sem:sql.render.locking+1/test]    ` OF ` names the locked tables, comma-separated
// and quoted, and the behaviour follows
#[test]
fn locking_2() {
    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .from(Char::Table)
            .lock_with_tables(
                LockType::Update,
                [Glyph::Table.into_name(), Char::Table.into_name()]
            )
            .to_string(),
        r#"SELECT "id" FROM "glyph", "character" FOR UPDATE OF "glyph", "character""#
    );

    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .lock_with_behavior(LockType::Update, LockBehavior::Nowait)
            .to_string(),
        r#"SELECT "id" FROM "glyph" FOR UPDATE NOWAIT"#
    );

    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .lock_with_tables_behavior(LockType::Share, [Glyph::Table], LockBehavior::SkipLocked)
            .to_string(),
        r#"SELECT "id" FROM "glyph" FOR SHARE OF "glyph" SKIP LOCKED"#
    );

    // The clause is overwritten wholesale — the last call wins.
    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .lock_with_behavior(LockType::Update, LockBehavior::Nowait)
            .lock_shared()
            .to_string(),
        r#"SELECT "id" FROM "glyph" FOR SHARE"#
    );
}

// [spec:pgorm:def:sql.ast.select+6/test]    OF holds names, each the one a FROM item answers
// to, and the grammar reads each as an unqualified relation name
// [spec:pgorm:sem:sql.render.locking+1/test]
#[test]
fn lock_of_names_what_items_answer_to() {
    let sub = FromItem::SubQuery(
        Query::select().column(Font::Id).from(Font::Table).take(),
        Name::runtime("q"),
    );
    let sql = Query::select()
        .column(Asterisk)
        .from(Glyph::Table.into_named_table().alias(Name::runtime("g")))
        .from(sub)
        .from(Char::Table)
        .lock_with_tables(
            LockType::NoKeyUpdate,
            [
                Name::runtime("g"),
                Name::runtime("q"),
                Char::Table.into_name(),
            ],
        )
        .to_string();
    assert_eq!(
        sql,
        r#"SELECT * FROM "glyph" AS "g", (SELECT "id" FROM "font") AS "q", "character" FOR NO KEY UPDATE OF "g", "q", "character""#
    );

    let locked: Vec<serde_json::Value> =
        crate::postgres::oracle::parsed_nodes(&sql, "LockingClause")
            .into_iter()
            .flat_map(|clause| {
                clause["locked_rels"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
            .collect();
    let names: Vec<(&str, bool)> = locked
        .iter()
        .map(|rel| {
            let rel = &rel["RangeVar"];
            (
                rel["relname"].as_str().unwrap_or_default(),
                rel["schemaname"].as_str().unwrap_or_default().is_empty(),
            )
        })
        .collect();
    assert_eq!(names, [("g", true), ("q", true), ("character", true)]);
}
