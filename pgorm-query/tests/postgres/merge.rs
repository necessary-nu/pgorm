use super::*;
use crate::oracle::{assert_eq, parsed_nodes};
use CmdType::{CmdDelete, CmdInsert, CmdNothing, CmdUpdate};
use MergeMatchKind::{
    MergeWhenMatched as Matched, MergeWhenNotMatchedBySource as BySource,
    MergeWhenNotMatchedByTarget as NotMatched,
};
use pg_query::protobuf::{BoolExprType, CmdType, MergeMatchKind, OverridingKind};

fn on() -> SimpleExpr {
    Expr::col((Glyph::Table, Glyph::Id)).equals((Font::Table, Font::Id))
}

fn merge() -> PendingMerge {
    Query::merge(Glyph::Table, Font::Table, on())
}

fn head() -> &'static str {
    r#"MERGE INTO "glyph" USING "font" ON "glyph"."id" = "font"."id""#
}

fn font(column: Font) -> Expr {
    Expr::col((Font::Table, column))
}

/// The one `MergeStmt` in `sql`, as PostgreSQL's parser read it.
fn statement(sql: &str) -> serde_json::Value {
    let mut found = parsed_nodes(sql, "MergeStmt");
    assert_eq!(found.len(), 1, "exactly one MERGE in {sql}");
    found.remove(0)
}

fn code(clause: &serde_json::Value, field: &str) -> i32 {
    let code = clause[field]
        .as_i64()
        .unwrap_or_else(|| panic!("a {field} code in {clause}"));
    <i32 as TryFrom<i64>>::try_from(code).expect("a protobuf enum code fits i32")
}

/// Each `WHEN` clause of `sql` in the order the parser read them: which rows
/// it takes, what it does to them, and whether it carries an `AND` condition.
fn arms(sql: &str) -> Vec<(MergeMatchKind, CmdType, bool)> {
    parsed_nodes(sql, "MergeWhenClause")
        .iter()
        .map(|clause| {
            let kind = MergeMatchKind::try_from(code(clause, "match_kind")).expect("a match kind");
            let command = CmdType::try_from(code(clause, "command_type")).expect("a command");
            (kind, command, clause.get("condition").is_some())
        })
        .collect()
}

// [spec:pgorm:req:sql.ast.merge+2/test]    the target, the source and the condition, then arms
// [spec:pgorm:req:sql.render.merge+1/test]    `MERGE INTO <target> USING <source> ON <condition>`
#[test]
fn merge_renders_target_source_condition_and_arms() {
    let sql = merge()
        .when_matched(MergeUpdate::value(Glyph::Image, font(Font::Name)))
        .when_not_matched(
            MergeInsert::value(Glyph::Id, font(Font::Id)).and_value(Glyph::Image, font(Font::Name)),
        )
        .to_string();
    assert_eq!(
        sql,
        [
            head(),
            r#"WHEN MATCHED THEN UPDATE SET "image" = "font"."name""#,
            r#"WHEN NOT MATCHED THEN INSERT ("id", "image") VALUES ("font"."id", "font"."name")"#,
        ]
        .join(" ")
    );
    let merge = statement(&sql);
    assert_eq!(merge["relation"]["relname"], "glyph");
    assert_eq!(merge["source_relation"]["RangeVar"]["relname"], "font");
    assert!(merge.get("join_condition").is_some(), "{sql}");
    assert_eq!(
        arms(&sql),
        [(Matched, CmdUpdate, false), (NotMatched, CmdInsert, false)]
    );
}

// [spec:pgorm:req:sql.ast.merge+2/test]    conditional arms are tried in the order they were added
// [spec:pgorm:req:sql.render.merge+1/test]
#[test]
fn conditional_arms_keep_the_order_they_were_added() {
    let gone = || Expr::col((Font::Table, Font::Name)).is_null();
    let blank = || Expr::col((Glyph::Table, Glyph::Image)).is_null();
    let rename = || MergeUpdate::value(Glyph::Image, font(Font::Name));

    let delete_first = merge()
        .when_matched_and(gone(), MatchedAction::Delete)
        .when_matched_and(blank(), rename())
        .to_string();
    assert_eq!(
        delete_first,
        [
            head(),
            r#"WHEN MATCHED AND "font"."name" IS NULL THEN DELETE"#,
            r#"WHEN MATCHED AND "glyph"."image" IS NULL THEN UPDATE SET "image" = "font"."name""#,
        ]
        .join(" ")
    );
    assert_eq!(
        arms(&delete_first),
        [(Matched, CmdDelete, true), (Matched, CmdUpdate, true)]
    );

    let update_first = merge()
        .when_matched_and(blank(), rename())
        .when_matched_and(gone(), MatchedAction::Delete)
        .to_string();
    assert_eq!(
        arms(&update_first),
        [(Matched, CmdUpdate, true), (Matched, CmdDelete, true)]
    );
}

// [spec:pgorm:req:sql.ast.merge+2/test]    the unreachable-arm statement has no representation: an
// unconditional arm renders after its kind's conditional arms whenever it was added
// [spec:pgorm:req:sql.render.merge+1/test]
#[test]
fn unconditional_arm_renders_after_conditional_arms() {
    let sql = merge()
        .when_matched(MatchedAction::Delete)
        .when_matched_and(font(Font::Name).is_null(), MatchedAction::DoNothing)
        .when_not_matched(NotMatchedAction::DoNothing)
        .when_not_matched_and(
            font(Font::Language).eq("en"),
            MergeInsert::value(Glyph::Id, font(Font::Id)),
        )
        .to_string();
    assert_eq!(
        sql,
        [
            head(),
            r#"WHEN MATCHED AND "font"."name" IS NULL THEN DO NOTHING"#,
            r#"WHEN MATCHED THEN DELETE"#,
            r#"WHEN NOT MATCHED AND "font"."language" = 'en' THEN INSERT ("id") VALUES ("font"."id")"#,
            r#"WHEN NOT MATCHED THEN DO NOTHING"#,
        ]
        .join(" ")
    );
    assert_eq!(
        arms(&sql),
        [
            (Matched, CmdNothing, true),
            (Matched, CmdDelete, false),
            (NotMatched, CmdInsert, true),
            (NotMatched, CmdNothing, false),
        ]
    );
}

// [spec:pgorm:req:sql.render.merge+1/test]    an arm's condition takes no parentheses: `AND` is
// the clause's keyword, and a top-level OR after it is read as the whole condition
#[test]
fn an_arms_condition_is_read_whole() {
    let sql = merge()
        .when_matched_and(
            Condition::any()
                .add(font(Font::Name).is_null())
                .add(font(Font::Variant).eq("gone")),
            MatchedAction::Delete,
        )
        .to_string();
    assert_eq!(
        sql,
        format!(
            "{} {}",
            head(),
            r#"WHEN MATCHED AND "font"."name" IS NULL OR "font"."variant" = 'gone' THEN DELETE"#
        )
    );
    let clauses = parsed_nodes(&sql, "MergeWhenClause");
    let condition = &clauses[0]["condition"]["BoolExpr"];
    assert_eq!(
        BoolExprType::try_from(code(condition, "boolop")).expect("a boolean operator"),
        BoolExprType::OrExpr,
        "{sql}"
    );
    assert_eq!(condition["args"].as_array().map(Vec::len), Some(2), "{sql}");
}

// [spec:pgorm:req:sql.ast.merge+2/test]    a kind holds one unconditional arm: the last call wins
#[test]
fn the_last_unconditional_arm_of_a_kind_wins() {
    let sql = merge()
        .when_matched(MatchedAction::Delete)
        .when_matched(MatchedAction::DoNothing)
        .to_string();
    assert_eq!(sql, format!("{} WHEN MATCHED THEN DO NOTHING", head()));
    assert_eq!(arms(&sql), [(Matched, CmdNothing, false)]);
}

// [spec:pgorm:req:sql.render.merge+1/test]    the matched arms render before the not-matched arms
#[test]
fn matched_arms_render_before_not_matched_arms() {
    let sql = Query::merge(Glyph::Table, Font::Table, on())
        .when_not_matched(NotMatchedAction::InsertDefaultValues)
        .when_matched(MatchedAction::Delete)
        .to_string();
    assert_eq!(
        sql,
        [
            head(),
            r#"WHEN MATCHED THEN DELETE"#,
            r#"WHEN NOT MATCHED THEN INSERT DEFAULT VALUES"#,
        ]
        .join(" ")
    );
    assert_eq!(
        arms(&sql),
        [(Matched, CmdDelete, false), (NotMatched, CmdInsert, false)]
    );
}

// [spec:pgorm:req:sql.render.merge+1/test]    every action in its grammar position, the
// assignments in the order added
#[test]
fn every_action_renders_where_the_grammar_puts_it() {
    let sql = merge()
        .when_matched_and(
            font(Font::Variant).eq("bold"),
            MergeUpdate::value(Glyph::Image, font(Font::Name))
                .and_value(Glyph::Aspect, Expr::col(Glyph::Aspect).add(1))
                .and_values([(Glyph::Tokens, Expr::val(2).into())]),
        )
        .when_matched_and(font(Font::Variant).eq("gone"), MatchedAction::Delete)
        .when_matched(MatchedAction::DoNothing)
        .when_not_matched_and(
            font(Font::Variant).eq("system"),
            MergeInsert::value(Glyph::Id, font(Font::Id)).overriding(Overriding::SystemValue),
        )
        .when_not_matched_and(
            font(Font::Variant).eq("user"),
            MergeInsert::value(Glyph::Id, font(Font::Id))
                .and_values([(Glyph::Image, font(Font::Name).into())])
                .overriding(Overriding::UserValue),
        )
        .when_not_matched(NotMatchedAction::InsertDefaultValues)
        .to_string();
    assert_eq!(
        sql,
        [
            head(),
            r#"WHEN MATCHED AND "font"."variant" = 'bold' THEN UPDATE SET"#,
            r#""image" = "font"."name", "aspect" = "aspect" + 1, "tokens" = 2"#,
            r#"WHEN MATCHED AND "font"."variant" = 'gone' THEN DELETE"#,
            r#"WHEN MATCHED THEN DO NOTHING"#,
            r#"WHEN NOT MATCHED AND "font"."variant" = 'system' THEN"#,
            r#"INSERT ("id") OVERRIDING SYSTEM VALUE VALUES ("font"."id")"#,
            r#"WHEN NOT MATCHED AND "font"."variant" = 'user' THEN"#,
            r#"INSERT ("id", "image") OVERRIDING USER VALUE VALUES ("font"."id", "font"."name")"#,
            r#"WHEN NOT MATCHED THEN INSERT DEFAULT VALUES"#,
        ]
        .join(" ")
    );
    let clauses = parsed_nodes(&sql, "MergeWhenClause");
    let overriding: Vec<_> = clauses
        .iter()
        .map(|clause| OverridingKind::try_from(code(clause, "override")).expect("a kind"))
        .collect();
    assert_eq!(
        overriding[3..],
        [
            OverridingKind::OverridingSystemValue,
            OverridingKind::OverridingUserValue,
            OverridingKind::OverridingNotSet,
        ]
    );
    let set: Vec<_> = clauses[0]["target_list"]
        .as_array()
        .expect("the update's assignments")
        .iter()
        .map(|target| target["ResTarget"]["name"].clone())
        .collect();
    assert_eq!(set, ["image", "aspect", "tokens"]);
}

// [spec:pgorm:req:sql.ast.merge+2/test]    the insert's columns and its row come from the same
// pairs, so they always have the same length
// [spec:pgorm:req:sql.render.merge+1/test]
#[test]
fn an_inserts_columns_and_row_line_up() {
    let sql = merge()
        .when_not_matched(
            MergeInsert::value(Glyph::Id, font(Font::Id))
                .and_value(Glyph::Image, font(Font::Name))
                .and_values([
                    (Glyph::Aspect, Expr::val(1.5).into()),
                    (Glyph::Tokens, Expr::val(3).into()),
                ]),
        )
        .to_string();
    let clauses = parsed_nodes(&sql, "MergeWhenClause");
    let columns: Vec<_> = clauses[0]["target_list"]
        .as_array()
        .expect("the insert's columns")
        .iter()
        .map(|target| target["ResTarget"]["name"].clone())
        .collect();
    assert_eq!(columns, ["id", "image", "aspect", "tokens"]);
    assert_eq!(
        clauses[0]["values"].as_array().map(Vec::len),
        Some(columns.len()),
        "{sql}"
    );
}

// [spec:pgorm:req:sql.ast.merge+2/test]    the target is a named table: schema, alias and ONLY
// [spec:pgorm:req:sql.render.merge+1/test]
#[test]
fn target_takes_schema_alias_and_only() {
    let target = (Name::runtime("app"), Glyph::Table)
        .into_named_table()
        .alias(Name::runtime("g"));
    let on = Expr::col((Name::runtime("g"), Glyph::Id)).equals((Font::Table, Font::Id));

    let inherited = Query::merge(target.clone(), Font::Table, on.clone())
        .when_matched(MatchedAction::Delete)
        .to_string();
    assert_eq!(
        inherited,
        r#"MERGE INTO "app"."glyph" AS "g" USING "font" ON "g"."id" = "font"."id" WHEN MATCHED THEN DELETE"#
    );
    let relation = &statement(&inherited)["relation"];
    assert_eq!(relation["schemaname"], "app");
    assert_eq!(relation["relname"], "glyph");
    assert_eq!(relation["alias"]["aliasname"], "g");
    assert_eq!(relation["inh"], true);

    let only = Query::merge(target, Font::Table, on)
        .when_matched(MatchedAction::Delete)
        .only()
        .to_string();
    assert_eq!(
        only,
        r#"MERGE INTO ONLY "app"."glyph" AS "g" USING "font" ON "g"."id" = "font"."id" WHEN MATCHED THEN DELETE"#
    );
    assert_eq!(statement(&only)["relation"]["inh"], false);
}

// [spec:pgorm:req:sql.ast.merge+2/test]    the source is any relation a FROM clause takes
// [spec:pgorm:req:sql.render.merge+1/test]
#[test]
fn the_source_is_any_from_item() {
    let s = || Name::runtime("s");
    let on = || Expr::col((Glyph::Table, Glyph::Id)).equals((s(), Font::Id));
    let delete = |source: FromItem| {
        Query::merge(Glyph::Table, source, on())
            .when_matched(MatchedAction::Delete)
            .to_string()
    };

    let subquery = delete(FromItem::SubQuery(
        Query::select().column(Font::Id).from(Font::Table).take(),
        s(),
    ));
    assert_eq!(
        subquery,
        r#"MERGE INTO "glyph" USING (SELECT "id" FROM "font") AS "s" ON "glyph"."id" = "s"."id" WHEN MATCHED THEN DELETE"#
    );
    assert!(
        statement(&subquery)["source_relation"]
            .get("RangeSubselect")
            .is_some()
    );

    let values = delete(FromItem::ValuesList(vec![1i32.into_value_tuple()], s()));
    assert_eq!(
        values,
        r#"MERGE INTO "glyph" USING (VALUES (1)) AS "s" ON "glyph"."id" = "s"."id" WHEN MATCHED THEN DELETE"#
    );
    assert!(
        statement(&values)["source_relation"]
            .get("RangeSubselect")
            .is_some()
    );

    let function = delete(FromItem::FunctionCall(
        Func::named(Name::runtime("generate_series"))
            .args([Expr::val(1).into(), Expr::val(3).into()]),
        s(),
    ));
    assert!(
        statement(&function)["source_relation"]
            .get("RangeFunction")
            .is_some()
    );
}

// [spec:pgorm:def:query.build.with+2/test]    a plain WITH clause prefixes the MERGE
// [spec:pgorm:req:sql.render.merge+1/test]
#[test]
fn a_with_clause_prefixes_the_merge() {
    let f = || Name::runtime("f");
    let sql = Query::merge(
        Glyph::Table,
        f(),
        Expr::col((Glyph::Table, Glyph::Id)).equals((f(), Font::Id)),
    )
    .when_matched(MatchedAction::Delete)
    .with(
        WithClause::new(CommonTableExpression::new(
            f(),
            Query::select().column(Font::Id).from(Font::Table).take(),
        ))
        .cte(CommonTableExpression::new(
            Name::runtime("g"),
            Query::delete()
                .from_table(Font::Table)
                .returning_all()
                .to_owned(),
        ))
        .to_owned(),
    )
    .to_string();
    assert_eq!(
        sql,
        [
            r#"WITH "f" AS (SELECT "id" FROM "font") , "g" AS (DELETE FROM "font" RETURNING *)"#,
            r#"MERGE INTO "glyph" USING "f" ON "glyph"."id" = "f"."id" WHEN MATCHED THEN DELETE"#,
        ]
        .join(" ")
    );
    let ctes = statement(&sql)["with_clause"]["ctes"]
        .as_array()
        .map(Vec::len);
    assert_eq!(ctes, Some(2));
}

// [spec:pgorm:req:sql.render.merge+1/test]    every value is bound, numbered in text order
// [spec:pgorm:req:sql.ast.build+3/test]
#[test]
fn build_binds_every_value_in_text_order() {
    let (sql, values) = Query::merge(
        Glyph::Table,
        Font::Table,
        Condition::all()
            .add(on())
            .add(font(Font::Language).eq("en")),
    )
    .when_matched_and(
        Expr::col((Glyph::Table, Glyph::Aspect)).gt(2),
        MergeUpdate::value(Glyph::Image, "rename").and_value(Glyph::Aspect, 3),
    )
    .when_not_matched_and(
        font(Font::Variant).eq("bold"),
        MergeInsert::value(Glyph::Id, font(Font::Id)).and_value(Glyph::Image, "fresh"),
    )
    .build();
    assert_eq!(
        sql,
        [
            r#"MERGE INTO "glyph" USING "font" ON "glyph"."id" = "font"."id" AND "font"."language" = $1"#,
            r#"WHEN MATCHED AND "glyph"."aspect" > $2 THEN UPDATE SET "image" = $3, "aspect" = $4"#,
            r#"WHEN NOT MATCHED AND "font"."variant" = $5 THEN INSERT ("id", "image") VALUES ("font"."id", $6)"#,
        ]
        .join(" ")
    );
    assert_eq!(
        values.0,
        vec![
            "en".into(),
            2i32.into(),
            "rename".into(),
            3i32.into(),
            "bold".into(),
            "fresh".into(),
        ]
    );
}

// [spec:pgorm:req:sql.ast.merge+2/test]    a not-matched-by-source arm takes a target row's
// actions, its conditional arms in call order and its unconditional arm after them
// [spec:pgorm:req:sql.render.merge+1/test]    ` WHEN NOT MATCHED BY SOURCE`, after the other kinds
#[test]
fn by_source_arms_render_last_in_call_order() {
    let sql = merge()
        .when_not_matched_by_source(MatchedAction::Delete)
        .when_not_matched_by_source_and(
            Expr::col((Glyph::Table, Glyph::Aspect)).gt(2),
            MergeUpdate::value(Glyph::Aspect, 0),
        )
        .when_not_matched_by_source_and(
            Expr::col((Glyph::Table, Glyph::Image)).is_null(),
            MatchedAction::DoNothing,
        )
        .when_matched(MatchedAction::Delete)
        .to_string();
    assert_eq!(
        sql,
        [
            head(),
            r#"WHEN MATCHED THEN DELETE"#,
            r#"WHEN NOT MATCHED BY SOURCE AND "glyph"."aspect" > 2 THEN UPDATE SET "aspect" = 0"#,
            r#"WHEN NOT MATCHED BY SOURCE AND "glyph"."image" IS NULL THEN DO NOTHING"#,
            r#"WHEN NOT MATCHED BY SOURCE THEN DELETE"#,
        ]
        .join(" ")
    );
    assert_eq!(
        arms(&sql),
        [
            (Matched, CmdDelete, false),
            (BySource, CmdUpdate, true),
            (BySource, CmdNothing, true),
            (BySource, CmdDelete, false),
        ]
    );
}

// [spec:pgorm:req:sql.ast.merge+2/test]    `PendingMerge` begins a statement with either
// not-matched-by-source arm
#[test]
fn a_by_source_arm_can_begin_the_statement() {
    let unconditional = merge()
        .when_not_matched_by_source(MergeUpdate::value(Glyph::Aspect, 0))
        .to_string();
    assert_eq!(
        unconditional,
        format!(
            r#"{} WHEN NOT MATCHED BY SOURCE THEN UPDATE SET "aspect" = 0"#,
            head()
        )
    );
    let conditional = merge()
        .when_not_matched_by_source_and(
            Expr::col((Glyph::Table, Glyph::Aspect)).gt(2),
            MatchedAction::Delete,
        )
        .to_string();
    assert_eq!(
        conditional,
        format!(
            r#"{} WHEN NOT MATCHED BY SOURCE AND "glyph"."aspect" > 2 THEN DELETE"#,
            head()
        )
    );
    assert_eq!(arms(&conditional), [(BySource, CmdDelete, true)]);
}

/// The column references of `sql`'s MERGE RETURNING list, each a list of
/// its fields, and `"merge_action()"` for the action.
fn returned(sql: &str) -> Vec<Vec<String>> {
    statement(sql)["returning_clause"]["exprs"]
        .as_array()
        .expect("a RETURNING list")
        .iter()
        .map(|target| {
            let value = &target["ResTarget"]["val"];
            if value.get("MergeSupportFunc").is_some() {
                return vec!["merge_action()".to_owned()];
            }
            value["ColumnRef"]["fields"]
                .as_array()
                .expect("a column reference")
                .iter()
                .map(|field| {
                    field["String"]["sval"]
                        .as_str()
                        .map_or_else(|| "*".to_owned(), str::to_owned)
                })
                .collect()
        })
        .collect()
}

fn fields(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

// [spec:pgorm:req:sql.ast.merge+2/test]    `returning` sets the list, read after the arms
// [spec:pgorm:req:sql.render.merge+1/test]    RETURNING last
// [spec:pgorm:req:sql.render.returning+4/test]    on a MERGE as on the other writes
#[test]
fn returning_follows_the_arms() {
    let sql = merge()
        .when_matched(MergeUpdate::value(Glyph::Image, font(Font::Name)))
        .when_not_matched_by_source(MatchedAction::Delete)
        .returning(Query::returning().columns([
            (ReturningRow::Old, Glyph::Image),
            (ReturningRow::New, Glyph::Image),
        ]))
        .to_string();
    assert_eq!(
        sql,
        [
            head(),
            r#"WHEN MATCHED THEN UPDATE SET "image" = "font"."name""#,
            r#"WHEN NOT MATCHED BY SOURCE THEN DELETE"#,
            r#"RETURNING old."image", new."image""#,
        ]
        .join(" ")
    );
    assert_eq!(
        returned(&sql),
        [fields(&["old", "image"]), fields(&["new", "image"])]
    );
}

// [spec:pgorm:req:sql.ast.merge+2/test]    `returning_action` leads the list with
// `merge_action()`, and alone is the list
// [spec:pgorm:req:sql.render.returning+4/test]    after the renames, before the caller's list
#[test]
fn the_action_leads_the_list() {
    let o = alias("o");
    let sql = merge()
        .when_matched(MatchedAction::Delete)
        .returning(Query::returning().column((o, Glyph::Id)).old_as(o))
        .returning_action()
        .to_string();
    assert_eq!(
        sql,
        format!(
            r#"{} WHEN MATCHED THEN DELETE RETURNING WITH (OLD AS "o") merge_action(), "o"."id""#,
            head()
        )
    );
    assert_eq!(
        returned(&sql),
        [fields(&["merge_action()"]), fields(&["o", "id"])]
    );

    let alone = merge()
        .when_matched(MatchedAction::Delete)
        .returning_action()
        .to_string();
    assert_eq!(
        alone,
        format!(
            "{} WHEN MATCHED THEN DELETE RETURNING merge_action()",
            head()
        )
    );
    assert_eq!(returned(&alone), [fields(&["merge_action()"])]);
}

// [spec:pgorm:req:sql.ast.merge+2/test]    the name a function call would take is quoted, so
// `Func::named` cannot reach `merge_action()`
#[test]
fn a_named_function_is_not_the_action() {
    let sql = Query::select()
        .expr(Func::named(Name::runtime("merge_action")))
        .to_string();
    assert_eq!(sql, r#"SELECT "merge_action"()"#);
    assert!(parsed_nodes(&sql, "MergeSupportFunc").is_empty());
    assert_eq!(parsed_nodes(&sql, "FuncCall").len(), 1);
}

// [spec:pgorm:req:sql.ast+4/test]    a MERGE nests as a CTE body
// [spec:pgorm:def:sql.ast.with+5/test]
#[test]
fn a_merge_is_a_cte_body() {
    let body = merge()
        .when_matched(MatchedAction::Delete)
        .returning_action()
        .returning(Query::returning().column(Glyph::Id))
        .to_owned();
    let m = || Name::runtime("m");
    let sql = Query::select()
        .column(Asterisk)
        .from(m())
        .with(WithClause::new(CommonTableExpression::new(m(), body)))
        .to_string();
    assert_eq!(
        sql,
        [
            r#"WITH "m" AS (MERGE INTO "glyph" USING "font" ON "glyph"."id" = "font"."id""#,
            r#"WHEN MATCHED THEN DELETE RETURNING merge_action(), "id")"#,
            r#"SELECT * FROM "m""#,
        ]
        .join(" ")
    );
    let ctes = parsed_nodes(&sql, "CommonTableExpr");
    assert_eq!(ctes.len(), 1);
    assert!(ctes[0]["ctequery"].get("MergeStmt").is_some());
}

// [spec:pgorm:req:sql.render.merge+1/test]    values in a by-source arm and in RETURNING are
// bound, numbered in text order
#[test]
fn by_source_and_returning_values_are_bound() {
    let (sql, values) = merge()
        .when_matched_and(font(Font::Language).eq("en"), MatchedAction::Delete)
        .when_not_matched_by_source_and(
            Expr::col((Glyph::Table, Glyph::Aspect)).gt(2),
            MergeUpdate::value(Glyph::Image, "orphan"),
        )
        .returning(Query::returning().expr(Expr::col((ReturningRow::New, Glyph::Aspect)).add(1)))
        .returning_action()
        .build();
    assert_eq!(
        sql,
        [
            head(),
            r#"WHEN MATCHED AND "font"."language" = $1 THEN DELETE"#,
            r#"WHEN NOT MATCHED BY SOURCE AND "glyph"."aspect" > $2 THEN UPDATE SET "image" = $3"#,
            r#"RETURNING merge_action(), new."aspect" + $4::int4"#,
        ]
        .join(" ")
    );
    assert_eq!(
        values.0,
        vec!["en".into(), 2i32.into(), "orphan".into(), 1i32.into()]
    );
}
