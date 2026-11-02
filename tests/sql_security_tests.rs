#![allow(unused_imports, dead_code)]

//! Live regressions for SQL injection boundaries, identifier quoting, literal
//! round trips, and predicates that must not broaden a delete.
//!
//! Each test provisions its own database. Injection payloads only select
//! constants or modify the test's own schema.

pub mod common;
use common::TestContext;
use pgorm::{DecodeRaw, entity::prelude::*};
use pgorm_query::{Expr, Func, Query, inject_parameters};

#[tokio::test]
async fn inline_parameters_preserve_block_comments() {
    let ctx = TestContext::new("security_inline_comment").await;
    let db = ctx.db.get().await.unwrap();
    let source = "SELECT $1::text AS value /* $1 */";
    let payload = "*/ UNION ALL SELECT upper($$injected$$) --";
    let bound = db.query_all(source, &[&payload]).await.unwrap();
    let sql = inject_parameters(source, [payload.into()]).unwrap();
    let inline = db.query_all(&sql, &[]).await.unwrap();
    let expected = bound
        .iter()
        .map(|r| r.get::<_, String>(0))
        .collect::<Vec<_>>();
    let actual = inline
        .iter()
        .map(|r| r.get::<_, String>(0))
        .collect::<Vec<_>>();
    drop(db);
    ctx.delete().await;
    assert_eq!(actual, expected, "a value became executable SQL: {sql}");
}

#[tokio::test]
async fn inline_parameters_preserve_dollar_quoted_bodies() {
    let ctx = TestContext::new("security_inline_dollar").await;
    let db = ctx.db.get().await.unwrap();
    let source = "SELECT $$ $1 $$ AS body WHERE $1::text IS NOT NULL";
    let payload = "$$ || 12345::text || $$";
    let expected: String = db.query_one(source, &[&payload]).await.unwrap().get(0);
    let sql = inject_parameters(source, [payload.into()]).unwrap();
    let actual: String = db.query_one(&sql, &[]).await.unwrap().get(0);
    drop(db);
    ctx.delete().await;
    assert_eq!(actual, expected, "a value became executable SQL: {sql}");
}

/// Under `standard_conforming_strings`, the server default, a backslash means
/// nothing in a plain literal: `'C:\'` is complete, and the marker after it is
/// a marker. The inlined statement must read the way the server reads the
/// bound one.
#[tokio::test]
async fn inline_parameters_preserve_standard_backslash_literals() {
    let ctx = TestContext::new("security_inline_backslash").await;
    let db = ctx.db.get().await.unwrap();
    let source = r"SELECT 'C:\' AS path, $1::text AS value, E'C:\\' AS escaped";
    let payload = r"' || current_user || '\";
    let bound = db.query_one(source, &[&payload]).await.unwrap();
    let expected: (String, String, String) = (bound.get(0), bound.get(1), bound.get(2));
    let sql = inject_parameters(source, [payload.into()]).unwrap();
    let inline = db.query_one(&sql, &[]).await.unwrap();
    let actual: (String, String, String) = (inline.get(0), inline.get(1), inline.get(2));
    drop(db);
    ctx.delete().await;
    assert_eq!(
        expected,
        (r"C:\".to_owned(), payload.to_owned(), r"C:\".to_owned())
    );
    assert_eq!(actual, expected, "a value became executable SQL: {sql}");
}

/// A continuation of an `E'…'` string is scanned in the escape string's own
/// state, so a `\'` inside it does not end the literal and the `$1` after it
/// is text.
#[tokio::test]
async fn inline_parameters_preserve_escape_string_continuations() {
    let ctx = TestContext::new("security_inline_continuation").await;
    let db = ctx.db.get().await.unwrap();
    let source = "SELECT E'a'\n'\\' $1 ' AS body, $1::text AS value";
    let payload = "' || 12345::text || '";
    let bound = db.query_one(source, &[&payload]).await.unwrap();
    let expected: (String, String) = (bound.get(0), bound.get(1));
    let sql = inject_parameters(source, [payload.into()]).unwrap();
    let inline = db.query_one(&sql, &[]).await.unwrap();
    let actual: (String, String) = (inline.get(0), inline.get(1));
    drop(db);
    ctx.delete().await;
    assert_eq!(expected, ("a' $1 ".to_owned(), payload.to_owned()));
    assert_eq!(actual, expected, "a value became executable SQL: {sql}");
}

#[tokio::test]
async fn custom_function_identifier_is_not_sql() {
    let ctx = TestContext::new("security_function_identifier").await;
    let db = ctx.db.get().await.unwrap();
    db.batch_execute(
        r#"CREATE FUNCTION "COALESCE(7) + 100 --"() RETURNS integer
           LANGUAGE sql AS 'SELECT 7'"#,
    )
    .await
    .unwrap();
    let (sql, values) = Query::select()
        .expr(Func::named(Name::runtime("COALESCE(7) + 100 --")))
        .build();
    let result = (sql.as_str(), values).into_tuple::<i32>().one(&db).await;
    drop(db);
    ctx.delete().await;
    assert!(
        result.as_ref().is_ok_and(|v| *v == 7),
        "function name executed as SQL: {sql}; result={result:?}"
    );
}

#[tokio::test]
async fn cast_type_identifier_is_not_sql() {
    let ctx = TestContext::new("security_cast_identifier").await;
    let db = ctx.db.get().await.unwrap();
    db.batch_execute(r#"CREATE DOMAIN "int4) + 100 --" AS integer"#)
        .await
        .unwrap();
    let (sql, values) = Query::select()
        .expr(Expr::val(7).cast_as(Name::runtime("int4) + 100 --")))
        .build();
    let result = (sql.as_str(), values).into_tuple::<i32>().one(&db).await;
    drop(db);
    ctx.delete().await;
    assert!(
        result.as_ref().is_ok_and(|v| *v == 7),
        "cast type executed as SQL: {sql}; result={result:?}"
    );
}

#[tokio::test]
async fn enum_type_identifier_preserves_case() {
    let ctx = TestContext::new("security_enum_identifier").await;
    let db = ctx.db.get().await.unwrap();
    db.batch_execute("CREATE TYPE \"ReviewStatus\" AS ENUM ('ready')")
        .await
        .unwrap();
    let (sql, values) = Query::select()
        .expr(
            Expr::val("ready")
                .as_enum(Name::runtime("ReviewStatus"))
                .cast_as(Name::runtime("text")),
        )
        .build();
    let result = (sql.as_str(), values).into_tuple::<String>().one(&db).await;
    drop(db);
    ctx.delete().await;
    assert!(
        result.as_ref().is_ok_and(|v| v == "ready"),
        "valid enum name failed: {sql}; result={result:?}"
    );
}

mod item {
    use pgorm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "security_items")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        pub name: String,
    }
    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

#[tokio::test]
async fn substring_helpers_treat_search_text_literally() {
    let ctx = TestContext::new("security_like_wildcards").await;
    let db = ctx.db.get().await.unwrap();
    db.batch_execute("CREATE TABLE security_items (id integer PRIMARY KEY, name text NOT NULL); INSERT INTO security_items VALUES (1, 'alice'), (2, 'bob'), (3, '100%pure'), (4, 'under_score'), (5, E'folder\\\\name')").await.unwrap();
    let percent = item::Entity::find()
        .filter(item::Column::Name.contains("%"))
        .all(&db)
        .await
        .unwrap();
    let underscore = item::Entity::find()
        .filter(item::Column::Name.contains("_"))
        .all(&db)
        .await
        .unwrap();
    let backslash = item::Entity::find()
        .filter(item::Column::Name.starts_with("folder\\"))
        .all(&db)
        .await
        .unwrap();
    drop(db);
    ctx.delete().await;
    let ids = |rows: &[item::Model]| rows.iter().map(|r| r.id).collect::<Vec<_>>();
    assert_eq!(
        (ids(&percent), ids(&underscore), ids(&backslash)),
        (vec![3], vec![4], vec![5])
    );
}

#[tokio::test]
async fn bound_and_inline_values_keep_sql_payloads_as_data() {
    let ctx = TestContext::new("security_value_controls").await;
    let db = ctx.db.get().await.unwrap();
    let payloads = [
        "' OR TRUE --",
        "\\'; SELECT 1; --",
        "$$; SELECT 2; $$",
        "a\"b",
        "/* */",
        "\n\r\t",
    ];
    for payload in payloads {
        let query = Query::select().expr(Expr::val(payload)).to_owned();
        let bound = query.build().into_tuple::<String>().one(&db).await.unwrap();
        let inline = db
            .query_one(&query.to_string(), &[])
            .await
            .unwrap()
            .get::<_, String>(0);
        assert_eq!(bound, payload);
        assert_eq!(inline, payload);
    }
    drop(db);
    ctx.delete().await;
}

#[tokio::test]
async fn quoted_identifier_keeps_sql_payload_as_name() {
    let ctx = TestContext::new("security_identifier_controls").await;
    let db = ctx.db.get().await.unwrap();
    let name = "value\" FROM nowhere; SELECT 7 --";
    let query = Query::select()
        .expr_as(Expr::val(9), Name::runtime(name))
        .to_owned();
    let row = db.query_one(&query.to_string(), &[]).await.unwrap();
    assert_eq!(row.columns()[0].name(), name);
    assert_eq!(row.get::<_, i32>(0), 9);
    drop(db);
    ctx.delete().await;
}

#[tokio::test]
async fn empty_disjunction_does_not_delete_rows() {
    let ctx = TestContext::new("security_empty_predicate").await;
    let db = ctx.db.get().await.unwrap();
    db.batch_execute("CREATE TABLE security_items (id integer PRIMARY KEY, name text NOT NULL); INSERT INTO security_items VALUES (1, 'alice'), (2, 'bob')").await.unwrap();
    let deleted = pgorm::Delete::many(item::Entity)
        .filter(Condition::any())
        .exec(&db)
        .await
        .unwrap();
    let remaining = item::Entity::find().all(&db).await.unwrap();
    drop(db);
    ctx.delete().await;
    assert_eq!(deleted, 0);
    assert_eq!(remaining.len(), 2);
}

#[tokio::test]
async fn enum_ddl_name_cannot_add_a_column() {
    let ctx = TestContext::new("security_enum_ddl").await;
    let db = ctx.db.get().await.unwrap();
    db.batch_execute(r#"CREATE TYPE "text, injected integer" AS ENUM ('ready')"#)
        .await
        .unwrap();
    let sql = pgorm_query::Table::create(Name::runtime("security_enum_ddl"))
        .col(
            pgorm_query::ColumnDef::new(Name::runtime("value")).enumeration(
                Name::runtime("text, injected integer"),
                [Name::runtime("ready")],
            ),
        )
        .to_string();
    let result = db.execute(&sql, &[]).await;
    let columns = db.query_all("SELECT column_name, udt_name FROM information_schema.columns WHERE table_name = 'security_enum_ddl' ORDER BY ordinal_position", &[]).await.unwrap()
        .iter().map(|r| (r.get::<_, String>(0), r.get::<_, String>(1))).collect::<Vec<_>>();
    drop(db);
    ctx.delete().await;
    assert!(
        result.is_ok() && columns == [("value".to_owned(), "text, injected integer".to_owned())],
        "enum type name changed table structure: {sql}; columns={columns:?}"
    );
}

#[tokio::test]
async fn inline_control_characters_round_trip() {
    let ctx = TestContext::new("security_control_chars").await;
    let db = ctx.db.get().await.unwrap();
    let payload = "left\u{001a}right";
    let query = Query::select().expr(Expr::val(payload)).to_owned();
    let bound = query.build().into_tuple::<String>().one(&db).await.unwrap();
    let sql = query.to_string();
    let inline: String = db.query_one(&sql, &[]).await.unwrap().get(0);
    drop(db);
    ctx.delete().await;
    assert_eq!(inline, bound, "control character changed: {sql:?}");
}

/// A name carrying a double quote can render differently between quoting
/// implementations, so the pipeline refuses it outright rather than trusting
/// whichever prqlc is linked to escape it. The refusal is the security
/// property: a name that cannot render cannot become SQL.
#[tokio::test]
async fn quote_bearing_pipeline_names_are_refused_not_rendered() {
    use pgorm::pipeline::{ExprOps, Pipeline, PipelineError, col};
    let err = Pipeline::from(Name::runtime("sec\"table"))
        .filter(
            col(
                Name::runtime("sec\"table"),
                Name::runtime("note\" OR TRUE --"),
            )
            .eq("x"),
        )
        .into_sql()
        .expect_err("a quote-bearing identifier must refuse to render");
    assert!(
        matches!(err, PipelineError::UnquotableIdentifier(ref name) if name == "sec\"table"),
        "expected the first offending name back, got {err:?}"
    );
}

#[tokio::test]
async fn pipeline_keeps_hostile_names_and_values_as_data() {
    use pgorm::pipeline::{ExprOps, Pipeline, col};
    let ctx = TestContext::new("security_pipeline_control").await;
    let db = ctx.db.get().await.unwrap();
    db.batch_execute("CREATE TABLE \"sec'table\" (\"note' OR TRUE --\" text)")
        .await
        .unwrap();
    let payload = "\\'; SELECT 1; --";
    db.execute("INSERT INTO \"sec'table\" VALUES ($1)", &[&payload])
        .await
        .unwrap();
    let table = || Name::runtime("sec'table");
    let column = || col(table(), Name::runtime("note' OR TRUE --"));
    let inline_sql = Pipeline::from(table())
        .filter(column().eq(payload))
        .select(column())
        .into_sql()
        .unwrap();
    let bound_sql = Pipeline::from(table())
        .filter_with(|binder| {
            col(table(), Name::runtime("note' OR TRUE --")).eq(binder.bind(payload))
        })
        .select(column())
        .into_sql()
        .unwrap();
    eprintln!(
        "inline SQL = {:?}; bound SQL = {:?}",
        inline_sql.0, bound_sql.0
    );
    let inline = inline_sql.into_tuple::<String>().all(&db).await;
    let bound = bound_sql.into_tuple::<String>().all(&db).await;
    drop(db);
    ctx.delete().await;
    assert!(
        inline.as_ref().is_ok_and(|v| v == &[payload]),
        "inline={inline:?}; bound={bound:?}"
    );
    assert!(
        bound.as_ref().is_ok_and(|v| v == &[payload]),
        "bound={bound:?}"
    );
}

#[tokio::test]
async fn disjunction_stays_inside_tenant_filter_on_delete() {
    let ctx = TestContext::new("security_delete_grouping").await;
    let db = ctx.db.get().await.unwrap();
    db.batch_execute("CREATE TABLE security_items (id integer PRIMARY KEY, name text NOT NULL); INSERT INTO security_items VALUES (1, 'alice'), (2, 'bob'), (3, 'alice')").await.unwrap();
    let deleted = pgorm::Delete::many(item::Entity)
        .filter(
            Condition::any()
                .add(item::Column::Name.eq("alice"))
                .add(item::Column::Name.eq("bob")),
        )
        .filter(item::Column::Id.eq(1))
        .exec(&db)
        .await
        .unwrap();
    let remaining = item::Entity::find()
        .order_by_asc(item::Column::Id)
        .all(&db)
        .await
        .unwrap();
    drop(db);
    ctx.delete().await;
    assert_eq!(deleted, 1);
    assert_eq!(
        remaining.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![2, 3]
    );
}

#[tokio::test]
async fn pipeline_values_keep_sql_payload_as_data() {
    use pgorm::pipeline::{ExprOps, Pipeline, col};
    let ctx = TestContext::new("security_pipeline_values").await;
    let db = ctx.db.get().await.unwrap();
    db.batch_execute("CREATE TABLE security_items (id integer PRIMARY KEY, name text); INSERT INTO security_items VALUES (1, 'ordinary')").await.unwrap();
    let payload = "\\' OR TRUE --";
    db.execute("INSERT INTO security_items VALUES (2, $1)", &[&payload])
        .await
        .unwrap();
    let column = || col(Name::runtime("security_items"), Name::runtime("name"));
    let inline_sql = Pipeline::from(item::Entity)
        .filter(column().eq(payload))
        .select(column())
        .into_sql()
        .unwrap();
    let bound_sql = Pipeline::from(item::Entity)
        .filter_with(|binder| {
            col(Name::runtime("security_items"), Name::runtime("name")).eq(binder.bind(payload))
        })
        .select(column())
        .into_sql()
        .unwrap();
    eprintln!(
        "inline SQL = {:?}; bound SQL = {:?}",
        inline_sql.0, bound_sql.0
    );
    let inline = inline_sql.into_tuple::<String>().all(&db).await;
    let bound = bound_sql.into_tuple::<String>().all(&db).await;
    drop(db);
    ctx.delete().await;
    assert!(
        inline.as_ref().is_ok_and(|v| v == &[payload]),
        "inline={inline:?}; bound={bound:?}"
    );
    assert!(
        bound.as_ref().is_ok_and(|v| v == &[payload]),
        "bound={bound:?}"
    );
}
