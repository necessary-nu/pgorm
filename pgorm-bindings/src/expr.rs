//! The functions a binding's `call(name, *args)` reaches, each through its
//! pgorm-query constructor and taking the argument counts the constructor
//! does: the name selects a constructor and never reaches SQL.

use pgorm::pgorm_query::{Func, FunctionCall, Name, SimpleExpr};

use crate::{Dialect, Refusal};

/// The functions `call` reaches, in the order its refusal lists them.
const FUNCTIONS: &[&str] = &[
    "lower",
    "upper",
    "abs",
    "char_length",
    "count",
    "count_distinct",
    "sum",
    "avg",
    "min",
    "max",
    "round",
    "coalesce",
    "random",
    "gen_random_uuid",
    "uuidv4",
    "uuidv7",
    "uuid_extract_timestamp",
    "uuid_extract_version",
    "generate_series",
    "unnest",
    "json_to_record",
    "jsonb_to_record",
    "json_to_recordset",
    "jsonb_to_recordset",
    "grouping",
    "to_tsvector",
    "to_tsquery",
    "plainto_tsquery",
    "phraseto_tsquery",
    "websearch_to_tsquery",
    "ts_rank",
    "ts_rank_cd",
];

/// PostgreSQL's general-purpose window function `name` over `args`, at the
/// argument counts it takes, usable only through `over` since the server
/// refuses one without a window (`42809`).
// [spec:pgorm:req:bindings.windows]
pub fn window_function(
    name: &str,
    args: Vec<SimpleExpr>,
    dialect: Dialect,
) -> Result<FunctionCall, Refusal> {
    let accepted = match name {
        "row_number" | "rank" | "dense_rank" | "percent_rank" | "cume_dist" => Some(0..=0),
        "ntile" | "first_value" | "last_value" => Some(1..=1),
        "nth_value" => Some(2..=2),
        "lag" | "lead" => Some(1..=3),
        _ => None,
    };
    if !accepted.is_some_and(|counts| counts.contains(&args.len())) {
        return Err(Refusal(format!(
            "{}() has no {name:?} taking {} argument(s)",
            dialect.option("window_function"),
            args.len()
        )));
    }
    Ok(Func::named(Name::runtime(name)).args(args))
}

/// `name` called with `args`, through pgorm-query's constructor of that
/// function, or the refusal naming the functions `call` reaches; `func`
/// calls any other function by name.
// [spec:pgorm:req:bindings.expressions+4]
pub fn call(name: &str, args: &[SimpleExpr]) -> Result<SimpleExpr, Refusal> {
    if let ("grouping", [first, rest @ ..]) = (name, args) {
        let grouping = rest
            .iter()
            .fold(Func::grouping(first.clone()), |grouping, arg| {
                grouping.arg(arg.clone())
            });
        return Ok(grouping.into());
    }
    function(name, args).map(Into::into).ok_or_else(|| {
        let (last, rest) = FUNCTIONS.split_last().unwrap_or((&"", &[]));
        Refusal(format!(
            "call() has no {name:?} taking {} argument(s); it reaches {} and {last}; func() calls \
             any other by name",
            args.len(),
            rest.join(", ")
        ))
    })
}

fn function(name: &str, args: &[SimpleExpr]) -> Option<FunctionCall> {
    Some(match (name, args) {
        ("lower", [value]) => Func::lower(value.clone()),
        ("upper", [value]) => Func::upper(value.clone()),
        ("abs", [value]) => Func::abs(value.clone()),
        ("char_length", [value]) => Func::char_length(value.clone()),
        ("count", [value]) => Func::count(value.clone()),
        ("count_distinct", [value]) => Func::count_distinct(value.clone()),
        ("sum", [value]) => Func::sum(value.clone()),
        ("avg", [value]) => Func::avg(value.clone()),
        ("min", [value]) => Func::min(value.clone()),
        ("max", [value]) => Func::max(value.clone()),
        ("round", [value]) => Func::round(value.clone()),
        ("round", [value, precision]) => {
            Func::round_with_precision(value.clone(), precision.clone())
        }
        ("coalesce", [_, ..]) => Func::coalesce(args.to_vec()),
        ("random", []) => Func::random(),
        ("gen_random_uuid", []) => Func::gen_random_uuid(),
        ("uuidv4", []) => Func::uuidv4(),
        ("uuidv7", []) => Func::uuidv7(),
        ("uuidv7", [shift]) => Func::uuidv7_shifted(shift.clone()),
        ("uuid_extract_timestamp", [value]) => Func::uuid_extract_timestamp(value.clone()),
        ("uuid_extract_version", [value]) => Func::uuid_extract_version(value.clone()),
        ("generate_series", [start, stop]) => Func::generate_series(start.clone(), stop.clone()),
        ("generate_series", [start, stop, step]) => {
            Func::generate_series_step(start.clone(), stop.clone(), step.clone())
        }
        ("unnest", [array]) => Func::unnest(array.clone()),
        ("json_to_record", [json]) => Func::json_to_record(json.clone()),
        ("jsonb_to_record", [jsonb]) => Func::jsonb_to_record(jsonb.clone()),
        ("json_to_recordset", [json]) => Func::json_to_recordset(json.clone()),
        ("jsonb_to_recordset", [jsonb]) => Func::jsonb_to_recordset(jsonb.clone()),
        ("to_tsvector", [text]) => Func::to_tsvector(text.clone(), None),
        ("to_tsquery", [text]) => Func::to_tsquery(text.clone(), None),
        ("plainto_tsquery", [text]) => Func::plainto_tsquery(text.clone(), None),
        ("phraseto_tsquery", [text]) => Func::phraseto_tsquery(text.clone(), None),
        ("websearch_to_tsquery", [text]) => Func::websearch_to_tsquery(text.clone(), None),
        ("ts_rank", [vector, query]) => Func::ts_rank(vector.clone(), query.clone()),
        ("ts_rank_cd", [vector, query]) => Func::ts_rank_cd(vector.clone(), query.clone()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use pgorm::pgorm_query::Expr;

    use super::*;

    // [spec:pgorm:req:bindings.windows/test]    a window function at a count it does not take
    // is refused, named as the binding spells window_function
    #[test]
    fn a_window_function_takes_its_own_argument_counts() {
        let one = vec![Expr::val(1).into()];
        assert!(window_function("lag", one.clone(), Dialect::Python).is_ok());
        assert!(window_function("row_number", Vec::new(), Dialect::Python).is_ok());
        let refused = |name, args, dialect| {
            window_function(name, args, dialect)
                .map_or_else(|refusal| refusal.into_message(), |_| String::new())
        };
        assert_eq!(
            refused("rank", one.clone(), Dialect::JavaScript),
            "windowFunction() has no \"rank\" taking 1 argument(s)"
        );
        assert_eq!(
            refused("median", one, Dialect::Python),
            "window_function() has no \"median\" taking 1 argument(s)"
        );
    }

    // [spec:pgorm:req:bindings.expressions+4/test]    a name selects its constructor at the
    // argument counts the constructor takes, and any other is refused naming those call reaches
    #[test]
    fn a_name_selects_its_constructor() {
        let value: SimpleExpr = Expr::val(1).into();
        assert!(call("lower", std::slice::from_ref(&value)).is_ok());
        assert!(call("round", &[value.clone(), value.clone()]).is_ok());
        assert!(call("grouping", &[value.clone(), value.clone()]).is_ok());
        let refused = call("lower", &[]).map(|_| ());
        assert!(refused.as_ref().is_err_and(|refusal| {
            refusal
                .message()
                .starts_with("call() has no \"lower\" taking 0 argument(s)")
                && refusal.message().contains("ts_rank and ts_rank_cd")
        }));
        assert!(call("drop_table", &[value]).is_err());
    }
}
