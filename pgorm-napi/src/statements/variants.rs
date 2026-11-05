//! Every variant of the open enums JavaScript spells into pgorm-query's
//! builders is one the binding spells or one named here as left out, so a
//! variant pgorm-query gains fails these tests until it is either.

use pgorm::pgorm_query::{
    BinOper, JsonExistsBehavior, JsonQueryBehavior, JsonTableBehavior, JsonValueBehavior, Value,
    tests_cfg::Variants,
};

use super::{expr, json, json_table};

/// What a table of spellings spells, and `rest` beside it.
fn spelled<T: Clone>(table: &[(&str, T)], rest: impl IntoIterator<Item = T>) -> Vec<T> {
    table
        .iter()
        .map(|(_, meaning)| meaning.clone())
        .chain(rest)
        .collect()
}

// [spec:pgorm:req:bindings.expressions+4/test]
#[test]
fn every_binary_operator_is_spelled_or_left_out() {
    // What another method writes: exprLogic, exprUnary, exprIn, exprBetween,
    // the pattern methods, exprBinary's isNotDistinctFrom and exprPathExists.
    let elsewhere = [
        BinOper::And,
        BinOper::Or,
        BinOper::Is,
        BinOper::IsNot,
        BinOper::In,
        BinOper::NotIn,
        BinOper::Between,
        BinOper::NotBetween,
        BinOper::BetweenSymmetric,
        BinOper::NotBetweenSymmetric,
        BinOper::Like,
        BinOper::NotLike,
        BinOper::ILike,
        BinOper::NotILike,
        BinOper::SimilarTo,
        BinOper::NotSimilarTo,
        BinOper::IsNotDistinctFrom,
        BinOper::JsonPathExists,
    ];
    // `As` is a projection's alias, no operator a program applies; the rest
    // are pgorm-query's pg_trgm, JSON, time zone and pgvector operators and
    // its raw one, which the module does not spell.
    assert_eq!(
        BinOper::missing_from(&spelled(&expr::BINARY, elsewhere)),
        [
            "As",
            "Similarity",
            "WordSimilarity",
            "StrictWordSimilarity",
            "SimilarityDistance",
            "WordSimilarityDistance",
            "StrictWordSimilarityDistance",
            "GetJsonField",
            "CastJsonField",
            "GetJsonPath",
            "CastJsonPath",
            "HasJsonKey",
            "HasAnyJsonKeys",
            "HasAllJsonKeys",
            "AtTimeZone",
            "EuclideanDistance",
            "NegativeInnerProduct",
            "CosineDistance",
            "Raw",
        ],
    );
}

// [spec:pgorm:req:bindings.sql-json/test]
#[test]
fn every_sql_json_behaviour_is_spelled() {
    // DEFAULT is spelled by a `jsonDefault` node, which no table names.
    let default = Value::Int(Some(0));
    assert_eq!(
        JsonExistsBehavior::missing_from(&spelled(json::EXISTS_BEHAVIORS, [])),
        [""; 0]
    );
    assert_eq!(
        JsonValueBehavior::missing_from(&spelled(
            json::VALUE_BEHAVIORS,
            [JsonValueBehavior::Default(default.clone())]
        )),
        [""; 0]
    );
    assert_eq!(
        JsonQueryBehavior::missing_from(&spelled(
            json::QUERY_BEHAVIORS,
            [JsonQueryBehavior::Default(default)]
        )),
        [""; 0]
    );
    assert_eq!(
        JsonTableBehavior::missing_from(&spelled(json_table::TABLE_BEHAVIORS, [])),
        [""; 0]
    );
}
