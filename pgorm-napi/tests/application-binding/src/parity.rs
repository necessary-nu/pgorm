//! Parity: each case of `tests/parity.json`, built here with the
//! application's entities through pgorm's own API, is the SQL and values the
//! JavaScript suite's `inspect()` gives for the same case through the
//! binding, which holds itself to the same file in both runtimes.

use std::collections::BTreeMap;

use pgorm::pgorm_query::{Expr, ExprTrait, Name, Query, Values};
use pgorm::{
    ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait, pgorm_query::Value,
};
use serde_json::Value as Json;

use pgorm::pipeline as pl;

use crate::{account, graphs, membership, note, parcel};

fn built<Q: QueryTrait>(query: &Q) -> (String, Values)
where
    Q::QueryStatement: pgorm::pgorm_query::QueryStatementBuilder,
{
    use pgorm::pgorm_query::QueryStatementBuilder;
    query.as_query().build()
}

/// A value as the golden file writes it: its kind and its text.
fn canonical(value: &Value) -> [String; 2] {
    let (kind, text) = match value {
        Value::Int(Some(v)) => ("i32", v.to_string()),
        Value::BigUnsigned(Some(v)) => ("u64", v.to_string()),
        Value::String(Some(v)) => ("text", v.to_string()),
        other => panic!("no canonical text for {other:?}"),
    };
    [kind.to_owned(), text]
}

fn aliases(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn cases() -> Vec<(&'static str, (String, Values))> {
    vec![
        ("find", built(&account::Entity::find())),
        (
            "find-filtered",
            built(
                &account::Entity::find()
                    .filter(account::Column::Id.gte(10))
                    .order_by_desc(account::Column::Name)
                    .limit(20),
            ),
        ),
        (
            "find-enum",
            built(&account::Entity::find().filter(account::Column::Mood.eq(account::Mood::Busy))),
        ),
        ("find-one", built(&account::Entity::find().limit(1))),
        ("find-one-limit-zero", {
            let mut query = account::Entity::find().limit(0);
            QueryTrait::query(&mut query).narrow_limit_to_one();
            built(&query)
        }),
        (
            "find-composite",
            built(
                &membership::Entity::find()
                    .filter(membership::Column::AccountId.eq(1))
                    .filter(membership::Column::Team.eq("red")),
            ),
        ),
        ("graph-notes", built(&graphs::optional(&aliases(&["n"])))),
        (
            "graph-mixed-default-aliases",
            built(
                &graphs::required(&aliases(&["g1"])).join_maybe_as::<crate::note::Entity>(
                    pgorm::RelationTrait::def(&account::Relation::Note),
                    Name::runtime("g2"),
                ),
            ),
        ),
        (
            "graph-filtered",
            built(
                &graphs::optional(&aliases(&["n"]))
                    .filter(Expr::col((Name::runtime("n"), Name::runtime("body"))).eq("x"))
                    .order_by_asc(account::Column::Id),
            ),
        ),
        (
            "find-member-subquery",
            built(
                &parcel::Entity::find().filter(
                    parcel::Column::Mood.in_subquery(
                        Query::select()
                            .expr(Expr::col(parcel::Column::Mood.as_column_ref()))
                            .from((Name::runtime("napi_entities"), Name::runtime("parcels")))
                            .and_where(parcel::Column::Id.eq(2))
                            .to_owned(),
                    ),
                ),
            ),
        ),
        (
            "find-member-values",
            built(&parcel::Entity::find().filter(parcel::Column::Mood.is_not_in(["busy"]))),
        ),
    ]
}

/// A pipeline's SQL and values, as pgorm compiles them.
fn compiled(built: Result<(String, Values), pl::PipelineError>) -> (String, Values) {
    match built {
        Ok(built) => built,
        Err(error) => panic!("the pipeline compiles: {error}"),
    }
}

fn pipelines() -> Vec<(&'static str, (String, Values))> {
    use pl::ExprOps;
    let joined = || {
        pl::Pipeline::from(account::Entity).join(
            pl::JoinSide::Left,
            pl::named_runtime(note::Entity, Name::runtime("n")),
            pl::col(Name::runtime("accounts"), Name::runtime("id"))
                .eq(pl::col(Name::runtime("n"), Name::runtime("account_id"))),
        )
    };
    vec![
        (
            "pipeline-registered-column",
            compiled(
                pl::Pipeline::from(parcel::Entity)
                    .filter_with(|binder| {
                        pl::named_runtime(parcel::Entity, Name::runtime("parcels"))
                            .col(parcel::Column::Mood)
                            .eq(binder.bind("busy"))
                    })
                    .into_sql(),
            ),
        ),
        (
            "pipeline-entity",
            compiled(pl::Pipeline::from(account::Entity).into_sql()),
        ),
        (
            "sources-single",
            compiled(
                pl::Pipeline::from(account::Entity)
                    .select_sources(pl::named_runtime(
                        account::Entity,
                        Name::runtime("accounts"),
                    ))
                    .into_sql(),
            ),
        ),
        (
            "sources-joined",
            compiled(
                joined()
                    .select_sources((
                        pl::named_runtime(account::Entity, Name::runtime("accounts")),
                        pl::named_runtime(note::Entity, Name::runtime("n")),
                    ))
                    .into_sql(),
            ),
        ),
        (
            "sources-joined-one",
            compiled(
                joined()
                    .take(1)
                    .select_sources((
                        pl::named_runtime(account::Entity, Name::runtime("accounts")),
                        pl::named_runtime(note::Entity, Name::runtime("n")),
                    ))
                    .into_sql(),
            ),
        ),
    ]
}

// [spec:pgorm:req:bindings.entity-reads+4/test]
// [spec:pgorm:req:bindings.entity-graphs+4/test]
// [spec:pgorm:req:bindings.pipeline-sources+3/test]
#[test]
fn entity_statements_match_the_golden_file() {
    let golden: BTreeMap<String, Json> = serde_json::from_str(include_str!("../tests/parity.json"))
        .expect("the golden file is JSON");
    let cases = [cases(), pipelines()].concat();
    let mut names: Vec<&str> = cases.iter().map(|(name, _)| *name).collect();
    names.sort_unstable();
    assert_eq!(names, golden.keys().map(String::as_str).collect::<Vec<_>>());
    for (name, (sql, values)) in cases {
        assert_eq!(
            golden[name]["sql"].as_str(),
            Some(sql.as_str()),
            "{name}: SQL"
        );
        let values: Vec<[String; 2]> = values.0.iter().map(canonical).collect();
        let expected: Vec<[String; 2]> =
            serde_json::from_value(golden[name]["values"].clone()).expect("values are pairs");
        assert_eq!(values, expected, "{name}: values");
    }
}
