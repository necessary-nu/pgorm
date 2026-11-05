//! Independent Rust builders verify the installed Python suite's exact SQL/Values.

use std::collections::BTreeMap;

use pgorm::pgorm_query::{
    BinOper, Condition, CopyBinary, CopyCsv, CopyFromStatement, CopySource, CopyText,
    CopyToStatement, Expr, ExprTrait, IntoNamedTable, JoinType, MatchedAction, MergeInsert,
    MergeUpdate, Name, NullOrdering, Order, Query, ReturningRow, SimpleExpr, Value, Values,
};
use pgorm_python::values::PyValue;
use pyo3::prelude::*;
use serde_json::{Value as Json, json};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Programs = BTreeMap<&'static str, (String, Values)>;

fn text<'a>(input: &'a Json, name: &str) -> TestResult<&'a str> {
    input[name]
        .as_str()
        .ok_or_else(|| format!("missing string {name}").into())
}

fn operand(value: impl Into<Value>, literal: bool) -> SimpleExpr {
    if literal {
        SimpleExpr::Constant(value.into())
    } else {
        Expr::value(value)
    }
}

fn programs(report: &Json, run: &Json) -> TestResult<Programs> {
    let schema = text(report, "schema")?;
    let accounts = (
        Name::runtime(schema),
        Name::runtime(text(report, "accounts")?),
    )
        .into_named_table();
    let events = (
        Name::runtime(schema),
        Name::runtime(text(report, "events")?),
    )
        .into_named_table();
    let literal = text(run, "mode")? == "literal";
    let variant = run["variant"].as_i64().ok_or("missing variant")?;
    let term = text(run, "term")?;
    let threshold = run["threshold"].as_i64().ok_or("missing threshold")?;
    let mut programs = Programs::new();

    let mut insert = Query::insert();
    insert.into_table(accounts.clone()).columns([
        Name::runtime("id"),
        Name::runtime("name"),
        Name::runtime("active"),
        Name::runtime("visits"),
    ]);
    for (id, name, visits) in [
        (1i64, "Nora O'Brien", 10i64),
        (2, "Beta%_\\雪", 20),
        (3, "No event", 30),
    ] {
        insert.values([
            operand(id, literal),
            operand(name, literal),
            operand(true, literal),
            operand(visits, literal),
        ])?;
    }
    programs.insert("insert_accounts", insert.build());
    let mut insert = Query::insert();
    insert.into_table(events.clone()).columns([
        Name::runtime("id"),
        Name::runtime("account_id"),
        Name::runtime("kind"),
        Name::runtime("points"),
    ]);
    for (id, account, kind, points) in [
        (10i64, 1i64, term, 2i64),
        (11, 1, "other", 3),
        (12, 2, term, 4),
    ] {
        insert.values([
            operand(id, literal),
            operand(account, literal),
            operand(kind, literal),
            operand(points, literal),
        ])?;
    }
    programs.insert("insert_events", insert.build());

    let mut select = Query::select();
    select
        .expr_as(
            Expr::col((Name::runtime("a"), Name::runtime("id"))),
            Name::runtime("account_id"),
        )
        .column((Name::runtime("a"), Name::runtime("name")))
        .expr_as(
            Expr::col((Name::runtime("e"), Name::runtime("id"))),
            Name::runtime("event_id"),
        )
        .column((Name::runtime("e"), Name::runtime("kind")))
        .from(accounts.clone().alias(Name::runtime("a")))
        .join(
            JoinType::LeftJoin,
            events.clone().alias(Name::runtime("e")),
            Expr::col((Name::runtime("a"), Name::runtime("id")))
                .equals((Name::runtime("e"), Name::runtime("account_id"))),
        );
    let mut predicate = Condition::all()
        .add(Expr::col((Name::runtime("a"), Name::runtime("active"))).eq(operand(true, literal)))
        .add(
            Condition::any()
                .add(
                    Expr::col((Name::runtime("e"), Name::runtime("kind")))
                        .eq(operand(term, literal)),
                )
                .add(Expr::col((Name::runtime("e"), Name::runtime("id"))).is_null()),
        )
        .add(
            Expr::col((Name::runtime("a"), Name::runtime("visits")))
                .gte(operand(threshold, literal)),
        );
    if variant != 0 {
        predicate = predicate
            .add(Expr::col((Name::runtime("a"), Name::runtime("id"))).gte(operand(2i64, literal)));
    }
    select
        .cond_where(predicate)
        .order_by((Name::runtime("a"), Name::runtime("id")), Order::Asc)
        .order_by_with_nulls(
            (Name::runtime("e"), Name::runtime("id")),
            Order::Asc,
            NullOrdering::Last,
        );
    if variant != 0 {
        select.limit(2);
    }
    programs.insert("select_join", select.build());

    programs.insert(
        "update_returning",
        Query::update()
            .table(accounts.clone())
            .value(
                Name::runtime("visits"),
                Expr::col(Name::runtime("visits")).add(operand(5 + variant, literal)),
            )
            .and_where(Expr::col(Name::runtime("id")).eq(operand(1i64, literal)))
            .returning(Query::returning().columns([
                Name::runtime("id"),
                Name::runtime("name"),
                Name::runtime("visits"),
            ]))
            .build(),
    );
    programs.insert(
        "delete_event",
        Query::delete()
            .from_table(events.clone())
            .and_where(Expr::col(Name::runtime("id")).eq(operand(11i64, literal)))
            .build(),
    );
    programs.insert(
        "select_missing",
        Query::select()
            .column(Name::runtime("id"))
            .from(accounts.clone())
            .and_where(Expr::col(Name::runtime("id")).eq(operand(999i64, literal)))
            .build(),
    );
    let target = |column: &str| Expr::col((Name::runtime("e"), Name::runtime(column)));
    let source = |column: &str| Expr::col((Name::runtime("a"), Name::runtime(column)));
    let version = |row: ReturningRow, alias: &str| {
        Expr::col((row, Name::runtime("points")))
            .binary(BinOper::As, Expr::col(Name::runtime(alias)))
    };
    programs.insert(
        "merge_events",
        Query::merge(
            events.clone().alias(Name::runtime("e")),
            accounts.clone().alias(Name::runtime("a")),
            target("id").eq(source("id").add(operand(10i64, literal))),
        )
        .when_matched_and(
            target("kind").eq(operand(term, literal)),
            MergeUpdate::value(
                Name::runtime("points"),
                target("points").add(operand(1i64, literal)),
            ),
        )
        .when_not_matched(
            MergeInsert::value(
                Name::runtime("id"),
                source("id").add(operand(10i64, literal)),
            )
            .and_value(Name::runtime("account_id"), source("id"))
            .and_value(Name::runtime("kind"), operand("merged", literal))
            .and_value(Name::runtime("points"), operand(0i64, literal)),
        )
        .when_not_matched_by_source(MatchedAction::Delete)
        .returning_action()
        .returning(Query::returning().exprs([
            target("id").into(),
            version(ReturningRow::Old, "before"),
            version(ReturningRow::New, "after"),
        ]))
        .build(),
    );
    programs.insert(
        "select_final",
        Query::select()
            .columns([Name::runtime("id"), Name::runtime("visits")])
            .from(accounts)
            .order_by(Name::runtime("id"), Order::Asc)
            .build(),
    );
    copy_programs(&mut programs, schema, text(report, "events")?, literal)?;
    Ok(programs)
}

/// The COPY cases, each way in each format: over the events table, a SELECT
/// of it and a DELETE's RETURNING list. COPY binds nothing, so each renders
/// through `Display` with no parameters, a SELECT's or a DELETE's bound
/// values written inline.
fn copy_programs(programs: &mut Programs, schema: &str, events: &str, literal: bool) -> TestResult {
    let table = || (Name::runtime(schema), Name::runtime(events));
    let columns = || ["id", "account_id", "kind", "points"].map(Name::runtime);
    let mut copy = |name, statement: String| {
        programs.insert(name, (statement, Values(Vec::new())));
    };
    copy(
        "copy_from_csv",
        CopyFromStatement::new(table(), CopyCsv)
            .columns(columns())
            .header()
            .delimiter(';')
            .to_string(),
    );
    copy(
        "copy_from_text",
        CopyFromStatement::new(table(), CopyText)
            .columns(columns())
            .null_string("NULL")
            .to_string(),
    );
    let points = Query::select()
        .columns([Name::runtime("id"), Name::runtime("kind")])
        .from(table().into_named_table())
        .and_where(Expr::col(Name::runtime("points")).gte(operand(7i64, literal)))
        .order_by(Name::runtime("id"), Order::Asc)
        .to_owned();
    copy(
        "copy_to_csv",
        CopyToStatement::new(points, CopyCsv)
            .header()
            .force_quote([Name::runtime("kind")])
            .to_string(),
    );
    let moved = Query::delete()
        .from_table(table().into_named_table())
        .and_where(Expr::col(Name::runtime("id")).gte(operand(20i64, literal)))
        .returning(Query::returning().columns(columns()))
        .to_owned();
    copy(
        "copy_to_binary",
        CopyToStatement::new(CopySource::try_from(moved)?, CopyBinary).to_string(),
    );
    copy(
        "copy_from_binary",
        CopyFromStatement::new(table(), CopyBinary)
            .columns(columns())
            .to_string(),
    );
    copy(
        "copy_to_text",
        CopyToStatement::new(table(), CopyText)
            .null_string("-")
            .to_string(),
    );
    Ok(())
}

fn snapshots(values: Values) -> TestResult<Json> {
    Python::attach(|py| {
        let json = py.import("json")?;
        let mut result = Vec::new();
        for value in values.0 {
            // Only the stable snapshot encoder is shared. Expected query ASTs
            // and their parameter values above are built independently in Rust.
            let value = Py::new(py, PyValue::from_rust(value))?;
            let snapshot = value.bind(py).call_method0("snapshot")?;
            let encoded: String = json.call_method1("dumps", (snapshot,))?.extract()?;
            result.push(serde_json::from_str::<Json>(&encoded)?);
        }
        Ok(Json::Array(result))
    })
}

// [spec:pgorm:req:python.direct-builders/test]
#[test]
fn installed_query_evidence_matches_rust_builders() -> TestResult {
    Python::initialize();
    let report = std::env::var("PGORM_DIRECT_REPORT")?;
    let report: Json = serde_json::from_str(&std::fs::read_to_string(report)?)?;
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["transport"], "in-process");
    let runs = report["runs"].as_array().ok_or("missing runs")?;
    assert_eq!(runs.len(), 4);
    for (index, run) in runs.iter().enumerate() {
        assert_eq!(run["mode"], if index < 2 { "bound" } else { "literal" });
        let variant = index % 2;
        assert_eq!(run["variant"], variant);
        assert_eq!(run["threshold"], if variant == 0 { 0 } else { 15 });
        assert_eq!(
            run["verified"],
            json!({"inserted_accounts":3,"inserted_events":3,"selected":3-variant,"updated_visits":15+variant,"deleted":[1,0],"missing":true,"merged":4,"copied":{"csv":2,"text":1,"binary":3}})
        );
        let expected = programs(&report, run)?;
        assert_eq!(
            run["cases"].as_object().ok_or("missing cases")?.len(),
            expected.len()
        );
        for (name, (sql, values)) in expected {
            assert_eq!(
                run["cases"][name]["sql"], sql,
                "SQL for {} {variant} {name}",
                run["mode"]
            );
            assert_eq!(
                run["cases"][name]["params"],
                snapshots(values)?,
                "parameters for {} {variant} {name}",
                run["mode"]
            );
        }
    }
    Ok(())
}
