use serde_json::{Map, Value, json};

pub(crate) fn operations() -> Map<String, Value> {
    [
        ("identifier", "pgorm_query::Name"),
        ("col", "pgorm_query::Expr::col"),
        ("bind", "pgorm_query::Expr::value"),
        ("literal", "pgorm_query::SimpleExpr::Constant"),
        ("tuple_expr", "pgorm_query::Expr::tuple"),
        ("call", "pgorm_query::Func"),
        ("func", "pgorm_query::Func::named"),
        ("template", "pgorm_query::SqlTemplate::from_pieces"),
        ("expr.get_json_field", "pgorm_query::Expr::get_json_field"),
        ("expr.cast_json_field", "pgorm_query::Expr::cast_json_field"),
        ("expr.get_json_path", "pgorm_query::Expr::get_json_path"),
        ("expr.cast_json_path", "pgorm_query::Expr::cast_json_path"),
        ("expr.has_json_key", "pgorm_query::Expr::has_json_key"),
        ("expr.has_any_json_keys", "pgorm_query::Expr::has_any_json_keys"),
        ("expr.has_all_json_keys", "pgorm_query::Expr::has_all_json_keys"),
        ("domain_value", "pgorm_query::Expr::domain_value"),
        ("current_date", "pgorm_query::Expr::current_date"),
        ("current_time", "pgorm_query::Expr::current_time"),
        ("current_timestamp", "pgorm_query::Expr::current_timestamp"),
        ("expr.eq", "pgorm_query::SimpleExpr::binary(BinOper::Equal)"),
        (
            "expr.ne",
            "pgorm_query::SimpleExpr::binary(BinOper::NotEqual)",
        ),
        (
            "expr.gt",
            "pgorm_query::SimpleExpr::binary(BinOper::GreaterThan)",
        ),
        (
            "expr.gte",
            "pgorm_query::SimpleExpr::binary(BinOper::GreaterThanOrEqual)",
        ),
        (
            "expr.lt",
            "pgorm_query::SimpleExpr::binary(BinOper::SmallerThan)",
        ),
        (
            "expr.lte",
            "pgorm_query::SimpleExpr::binary(BinOper::SmallerThanOrEqual)",
        ),
        ("expr.add", "pgorm_query::SimpleExpr::binary(BinOper::Add)"),
        ("expr.sub", "pgorm_query::SimpleExpr::binary(BinOper::Sub)"),
        ("expr.mul", "pgorm_query::SimpleExpr::binary(BinOper::Mul)"),
        ("expr.div", "pgorm_query::SimpleExpr::binary(BinOper::Div)"),
        ("expr.mod", "pgorm_query::SimpleExpr::binary(BinOper::Mod)"),
        ("expr.and", "pgorm_query::SimpleExpr::and"),
        ("expr.or", "pgorm_query::SimpleExpr::or"),
        ("expr.not", "pgorm_query::SimpleExpr::not"),
        ("expr.is_null", "pgorm_query::Expr::is_null"),
        ("expr.is_not_null", "pgorm_query::Expr::is_not_null"),
        ("expr.is_in", "pgorm_query::Expr::is_in, Expr::in_subquery"),
        (
            "expr.is_not_in",
            "pgorm_query::Expr::is_not_in, Expr::not_in_subquery",
        ),
        (
            "expr.between",
            "pgorm_query::Expr::between, Expr::between_symmetric",
        ),
        (
            "expr.not_between",
            "pgorm_query::Expr::not_between, Expr::not_between_symmetric",
        ),
        ("expr.at", "pgorm_query::Expr::index"),
        (
            "expr.slice",
            "pgorm_query::Expr::subscript(Subscript::Slice)",
        ),
        ("expr.cast", "pgorm_query::SimpleExpr::cast_as_type"),
        ("expr.collate", "pgorm_query::Expr::collate"),
        ("expr.starts_with", "pgorm_query::Func::starts_with"),
        (
            "expr.contains_text",
            "pgorm_query::Func::named(strpos), Expr::gt",
        ),
        (
            "expr.ends_with",
            "pgorm_query::Func::named(right), Func::char_length, Expr::eq",
        ),
        ("like_expr", "pgorm_query::LikeExpr::{new, escape, no_escape}"),
        ("expr.like", "pgorm_query::SimpleExpr::like"),
        ("expr.not_like", "pgorm_query::SimpleExpr::not_like"),
        ("expr.ilike", "pgorm_query::Expr::ilike"),
        ("expr.not_ilike", "pgorm_query::Expr::not_ilike"),
        ("expr.similar_to", "pgorm_query::SimpleExpr::similar_to"),
        (
            "expr.not_similar_to",
            "pgorm_query::SimpleExpr::not_similar_to",
        ),
        (
            "expr.regex",
            "pgorm_query::SimpleExpr::binary(BinOper::Regex)",
        ),
        (
            "expr.not_regex",
            "pgorm_query::SimpleExpr::binary(BinOper::NotRegex)",
        ),
        (
            "expr.iregex",
            "pgorm_query::SimpleExpr::binary(BinOper::RegexCaseInsensitive)",
        ),
        (
            "expr.not_iregex",
            "pgorm_query::SimpleExpr::binary(BinOper::NotRegexCaseInsensitive)",
        ),
        (
            "expr.contains",
            "pgorm_query::SimpleExpr::binary(BinOper::Contains)",
        ),
        (
            "expr.contained_by",
            "pgorm_query::SimpleExpr::binary(BinOper::Contained)",
        ),
        (
            "expr.overlaps",
            "pgorm_query::SimpleExpr::binary(BinOper::Overlap)",
        ),
        (
            "expr.strictly_left_of",
            "pgorm_query::SimpleExpr::binary(BinOper::LShift)",
        ),
        (
            "expr.strictly_right_of",
            "pgorm_query::SimpleExpr::binary(BinOper::RShift)",
        ),
        (
            "expr.not_extend_right_of",
            "pgorm_query::SimpleExpr::binary(BinOper::NotExtendRightOf)",
        ),
        (
            "expr.not_extend_left_of",
            "pgorm_query::SimpleExpr::binary(BinOper::NotExtendLeftOf)",
        ),
        (
            "expr.adjacent_to",
            "pgorm_query::SimpleExpr::binary(BinOper::AdjacentTo)",
        ),
        (
            "expr.matches",
            "pgorm_query::SimpleExpr::binary(BinOper::Matches)",
        ),
        (
            "expr.concat",
            "pgorm_query::SimpleExpr::binary(BinOper::Concatenate)",
        ),
        (
            "expr.is_distinct_from",
            "pgorm_query::Expr::is_distinct_from",
        ),
        (
            "expr.is_not_distinct_from",
            "pgorm_query::Expr::is_not_distinct_from",
        ),
        ("expr.path_exists", "pgorm_query::Expr::json_path_exists"),
        (
            "expr.inspect",
            "pgorm_query::SelectStatement::expr, SelectStatement::build",
        ),
        (
            "expr.over",
            "pgorm_query::WindowFunction (FunctionCall, JsonArrayAgg, JsonObjectAgg), SelectStatement::{expr_window, expr_window_as, expr_window_name, expr_window_name_as}",
        ),
        ("condition.all", "pgorm_query::Condition::all"),
        ("condition.any", "pgorm_query::Condition::any"),
        ("condition.add", "pgorm_query::Condition::add"),
        ("condition.add_option", "pgorm_query::Condition::add_option"),
        ("condition.not", "pgorm_query::Condition::not"),
        (
            "condition.inspect",
            "pgorm_query::SelectStatement::cond_where, SelectStatement::build",
        ),
        ("case_when", "pgorm_query::Expr::case"),
        ("searched_case.when", "pgorm_query::CaseStatement::case"),
        ("searched_case.else_", "pgorm_query::CaseStatement::finally"),
        ("case_of", "pgorm_query::Expr::case_of"),
        ("case_operand.when", "pgorm_query::CaseOperand::when"),
        ("simple_case.when", "pgorm_query::SimpleCaseStatement::when"),
        (
            "simple_case.else_",
            "pgorm_query::SimpleCaseStatement::finally",
        ),
        ("order_by", "pgorm_query::Order, pgorm_query::NullOrdering"),
        ("expr.as_", "pgorm_query::SimpleExpr, pgorm_query::Name"),
    ]
    .into_iter()
    .map(|(name, rust_api)| (name.into(), json!({"rust_api": rust_api, "features": []})))
    .collect()
}
