//! The SELECT family's cases, built with pgorm-query directly.

use std::{str::FromStr, sync::Arc};

use pgorm::pgorm_query::{
    ArrayType, Asterisk, BinOper, ColumnRef, ColumnType, CommonTableExpression, Condition, Cycle,
    Expr, ExprTrait, FromItem, Func, GroupingElement, JoinType, LikeExpr, LockBehavior, LockType,
    NamedTable, NullOrdering, Order, Query, RecursiveWithClause, SampleMethod, Search, SearchOrder,
    SelectStatement, SimpleExpr, SubQueryStatement, Subscript, TableFunction, TableName,
    TableSample, TypeName, UnionType, Value, Values, WithClause,
};

use super::n;

macro_rules! table {
    ($name:expr) => {
        FromItem::Table(NamedTable::from(TableName::Table(n($name))))
    };
}

fn account() -> NamedTable {
    NamedTable::from(TableName::SchemaTable(n("app"), n("account"))).alias(n("a"))
}

fn event() -> NamedTable {
    NamedTable::from(TableName::Table(n("event"))).alias(n("e"))
}

macro_rules! col {
    ($name:expr) => {
        SimpleExpr::from(Expr::col(n($name)))
    };
}

macro_rules! qualified {
    ($table:expr, $name:expr) => {
        SimpleExpr::from(Expr::col((n($table), n($name))))
    };
}

macro_rules! star {
    () => {
        Query::select().column(Asterisk).to_owned()
    };
}

fn join_group() -> SelectStatement {
    let count: SimpleExpr = Func::count(qualified!("e", "id")).into();
    Query::select()
        .expr(qualified!("a", "name"))
        .expr_as(count.clone(), n("events"))
        .from(FromItem::Table(account()))
        .join(
            JoinType::LeftJoin,
            FromItem::Table(event()),
            qualified!("a", "id").eq(qualified!("e", "account_id")),
        )
        .cond_where(
            Condition::all()
                .add(qualified!("a", "active").eq(true))
                .add(
                    Condition::any()
                        .add(qualified!("e", "kind").eq("click"))
                        .add(Expr::expr(qualified!("e", "id")).is_null()),
                ),
        )
        .add_group_by([qualified!("a", "name")])
        .cond_having(count.clone().binary(BinOper::GreaterThan, 2i64))
        .order_by_expr_with_nulls(count, Order::Desc, NullOrdering::Last)
        .order_by_expr_with_nulls(qualified!("a", "name"), Order::Asc, NullOrdering::First)
        .limit(5)
        .offset(10)
        .to_owned()
}

fn star_distinct() -> SelectStatement {
    let mut select = star!();
    select.from(table!("t")).distinct();
    select.clear_selects();
    select
        .expr(Expr::col(ColumnRef::TableAsterisk(n("t"))))
        .expr_as(col!("id"), n("key"))
        .to_owned()
}

fn joins() -> SelectStatement {
    star!()
        .from(table!("t"))
        .join(
            JoinType::InnerJoin,
            table!("u"),
            qualified!("t", "a").eq(qualified!("u", "a")),
        )
        .join(
            JoinType::RightJoin,
            FromItem::Table(NamedTable::from(TableName::SchemaTable(n("s"), n("v")))),
            SimpleExpr::from(Expr::col(ColumnRef::SchemaTableColumn(
                n("s"),
                n("v"),
                n("b"),
            )))
            .binary(BinOper::GreaterThan, 1i64),
        )
        .join(JoinType::FullOuterJoin, table!("w"), Condition::any())
        .cross_join(table!("x"))
        .to_owned()
}

fn operators() -> SelectStatement {
    Query::select()
        .expr(
            col!("a")
                .binary(BinOper::Add, 1i64)
                .binary(BinOper::Mul, 2i64),
        )
        .expr(col!("b").binary(BinOper::Concatenate, "x"))
        .expr(col!("c").binary(BinOper::Mod, 3i64))
        .expr(
            col!("d")
                .binary(BinOper::Div, 2i32)
                .binary(BinOper::Sub, 1.5f64),
        )
        .expr(
            col!("e")
                .binary(BinOper::NotEqual, col!("f"))
                .and(col!("g").binary(BinOper::SmallerThanOrEqual, 4i64))
                .or(col!("h").binary(BinOper::GreaterThanOrEqual, 5i64).not()),
        )
        .expr(col!("i").binary(BinOper::IsDistinctFrom, 1i64))
        .expr(Expr::expr(col!("j")).is_not_distinct_from("k"))
        .to_owned()
}

fn membership() -> SelectStatement {
    let ids = |name: &str| {
        Query::select()
            .expr(col!("id"))
            .from(table!(name))
            .to_owned()
    };
    star!()
        .from(table!("t"))
        .cond_where(
            Condition::all()
                .add(Expr::expr(col!("id")).is_in(Vec::<SimpleExpr>::new()))
                .add(Expr::expr(col!("id")).is_not_in(Vec::<SimpleExpr>::new()))
                .add(Expr::expr(col!("id")).is_in([1i64, 2i64]))
                .add(Expr::expr(col!("kind")).is_not_in(["a"]))
                .add(
                    Expr::expr(SimpleExpr::from(Expr::tuple([col!("a"), col!("b")])))
                        .is_in([SimpleExpr::from(Expr::tuple([1i64.into(), "x".into()]))]),
                )
                .add(Expr::expr(col!("id")).in_subquery(ids("u")))
                .add(Expr::expr(col!("id")).not_in_subquery(ids("v"))),
        )
        .to_owned()
}

fn between() -> SelectStatement {
    star!()
        .from(table!("t"))
        .cond_where(
            Condition::all()
                .add(Expr::expr(col!("a")).between(1i64, 10i64))
                .add(Expr::expr(col!("b")).not_between(1i64, 10i64))
                .add(Expr::expr(col!("c")).between_symmetric(10i64, 1i64))
                .add(Expr::expr(col!("d")).not_between_symmetric(10i64, 1i64)),
        )
        .to_owned()
}

fn patterns() -> SelectStatement {
    let suffix: SimpleExpr = "O'Brien".into();
    let right = pgorm::pgorm_query::Func::named(n("right"))
        .args([col!("f"), Func::char_length(suffix.clone()).into()]);
    let position = Func::named(n("strpos")).args([col!("g"), "\\".into()]);
    star!()
        .from(table!("t"))
        .cond_where(
            Condition::any()
                .add(Expr::expr(col!("a")).like(LikeExpr::new("A%")))
                .add(Expr::expr(col!("b")).not_like(LikeExpr::new("50\\%%").escape('\\')))
                .add(Expr::expr(col!("c")).ilike(LikeExpr::new("x_y")))
                .add(Expr::expr(col!("d")).not_ilike(LikeExpr::new("%z").escape('!')))
                .add(SimpleExpr::from(Func::starts_with(col!("e"), "50%_")))
                .add(Expr::expr(right).eq(suffix))
                .add(Expr::expr(position).gt(SimpleExpr::Constant(0i32.into()))),
        )
        .to_owned()
}

fn regex_and_json_path() -> SelectStatement {
    star!()
        .from(table!("t"))
        .cond_where(
            Condition::any()
                .add(col!("a").binary(BinOper::Regex, "^x"))
                .add(col!("b").binary(BinOper::NotRegex, col!("c")))
                .add(col!("d").binary(BinOper::RegexCaseInsensitive, "y$"))
                .add(col!("e").binary(BinOper::NotRegexCaseInsensitive, "z"))
                .add(Expr::expr(col!("doc")).json_path_exists(r#"$.tags[*] ? (@ == "blue")"#)),
        )
        .to_owned()
}

fn matches_and_quantifiers() -> SelectStatement {
    let ids = |name: &str| {
        Query::select()
            .expr(col!("id"))
            .from(table!(name))
            .to_owned()
    };
    let jsonpath = Expr::value("$.a == 1").cast_as_type(TypeName::new(n("jsonpath")));
    star!()
        .from(table!("t"))
        .cond_where(
            Condition::all()
                .add(col!("body").binary(BinOper::Matches, "fat & rat"))
                .add(col!("doc").binary(BinOper::Matches, jsonpath))
                .add(col!("id").binary(BinOper::Equal, Func::any(Expr::val(vec![1i64, 2i64]))))
                .add(col!("id").binary(BinOper::NotEqual, Func::all(Expr::array(ids("u")))))
                .add(col!("score").binary(BinOper::GreaterThan, Expr::all(ids("v"))))
                .add(col!("id").binary(BinOper::SmallerThanOrEqual, Expr::any(ids("w"))))
                .add(col!("a").binary(BinOper::GreaterThanOrEqual, Func::any(col!("b")))),
        )
        .to_owned()
}

fn quantifiers_beyond_comparisons() -> SelectStatement {
    let patterns = || Query::select().expr(col!("p")).from(table!("u")).to_owned();
    let bound = Func::any(Expr::val(vec!["x%".to_owned(), "y%".to_owned()]));
    star!()
        .from(table!("t"))
        .cond_where(
            Condition::all()
                .add(col!("a").binary(BinOper::Like, bound))
                .add(col!("a").binary(BinOper::NotILike, Expr::all(patterns())))
                .add(col!("a").binary(BinOper::RegexCaseInsensitive, Func::any(col!("b"))))
                .add(col!("span").binary(BinOper::Contained, Func::any(col!("spans"))))
                .add(col!("span").binary(BinOper::AdjacentTo, Expr::all(patterns())))
                .add(col!("doc").binary(BinOper::JsonPathExists, Func::any(col!("paths"))))
                .add(col!("body").binary(BinOper::Matches, Func::all(col!("queries")))),
        )
        .to_owned()
}

fn casts() -> SelectStatement {
    let mood = TypeName::new(n("Mood")).schema(n("app")).array();
    let twice: SimpleExpr = Expr::expr(col!("f")).index(1i64);
    Query::select()
        .expr(col!("a").cast_as_type(TypeName::new(n("integer"))))
        .expr(col!("b").cast_as_type(mood))
        .expr(SimpleExpr::from("1 day").cast_as_type(TypeName::new(n("interval"))))
        .expr(Expr::expr(col!("c")).collate(n("C")))
        .expr(Expr::expr(col!("d")).collate((n("pg_catalog"), n("de-x-icu"))))
        .expr(Expr::expr(col!("e")).index(1i64))
        .expr(Expr::expr(twice).index(2i32))
        .expr(Expr::expr(col!("g")).slice(2i64, 3i64))
        .expr(Expr::expr(col!("h")).slice_to(3i64))
        .expr(Expr::expr(col!("i")).slice_from(2i64))
        .expr(Expr::expr(col!("j")).subscript(Subscript::Slice(None, None)))
        .to_owned()
}

fn case() -> SelectStatement {
    Query::select()
        .expr(
            Expr::case(col!("x").binary(BinOper::GreaterThan, 0i64), "positive").case(
                Condition::all()
                    .add(col!("x").binary(BinOper::SmallerThan, 0i64))
                    .add(Expr::expr(col!("y")).is_not_null()),
                "negative",
            ),
        )
        .expr_as(
            Expr::case(Expr::expr(col!("x")).is_null(), 0i64).finally(col!("x")),
            n("filled"),
        )
        .expr(Expr::case_of(col!("kind")).when("a", 1i64).when("b", 2i64))
        .expr(Expr::case_of(col!("kind")).when("a", 1i64).finally(-1i64))
        .to_owned()
}

fn values() -> SelectStatement {
    let moods = Value::Array(
        ArrayType::String,
        Some(Box::new(vec!["calm".into(), "glad".into()])),
    );
    let decimal = rust_decimal::Decimal::from_str("19.9900").expect("a decimal");
    let uuid = uuid::Uuid::parse_str("0190a7a6-8c8e-7000-8000-000000000001").expect("a uuid");
    let instant: jiff::Timestamp = "2026-10-09T12:00:00Z".parse().expect("an instant");
    Query::select()
        .expr(SimpleExpr::from(7i16).binary(BinOper::Add, 3i64))
        .expr(SimpleExpr::from("calm").cast_as_type(TypeName::new(n("mood")).schema(n("app"))))
        .expr(SimpleExpr::Value(moods).cast_as_type(TypeName::new(n("mood")).array()))
        .expr(decimal)
        .expr(uuid)
        .expr(jiff::civil::date(2026, 10, 9))
        .expr(instant)
        .expr(true)
        .expr(Value::Array(
            ArrayType::BigInt,
            Some(Box::new(vec![1i64.into(), 2i64.into(), 3i64.into()])),
        ))
        .to_owned()
}

fn literals() -> SelectStatement {
    let constant = |value: Value| SimpleExpr::Constant(value);
    let moods = Value::Array(
        ArrayType::String,
        Some(Box::new(vec!["calm".into(), "glad".into()])),
    );
    let decimal = rust_decimal::Decimal::from_str("19.9900").expect("a decimal");
    let uuid = uuid::Uuid::parse_str("0190a7a6-8c8e-7000-8000-000000000001").expect("a uuid");
    Query::select()
        .expr(constant(7i64.into()).binary(BinOper::Add, 3i64))
        .expr(constant("O'Brien".into()))
        .expr(constant("calm".into()).cast_as_type(TypeName::new(n("mood")).schema(n("app"))))
        .expr(constant(moods).cast_as_type(TypeName::new(n("mood")).array()))
        .expr(constant(decimal.into()))
        .expr(constant(uuid.into()))
        .expr(constant(jiff::civil::date(2026, 10, 9).into()))
        .expr(constant(true.into()))
        .expr(constant(Value::Array(
            ArrayType::BigInt,
            Some(Box::new(vec![1i64.into(), 2i64.into(), 3i64.into()])),
        )))
        .expr(constant(Value::Int(None)))
        .from(table!("t"))
        .cond_where(
            col!("name")
                .binary(BinOper::Equal, constant("x".into()))
                .and(col!("id").binary(BinOper::GreaterThan, 2i64)),
        )
        .to_owned()
}

fn functions() -> SelectStatement {
    let shift = SimpleExpr::from("-1 hour").cast_as_type(TypeName::new(n("interval")));
    Query::select()
        .expr(Func::lower(col!("a")))
        .expr(Func::upper(col!("a")))
        .expr(Func::abs(col!("b")))
        .expr(Func::char_length(col!("a")))
        .expr(Func::count_distinct(col!("a")))
        .expr(Func::sum(col!("b")))
        .expr(Func::avg(col!("b")))
        .expr(Func::min(col!("b")))
        .expr(Func::max(col!("b")))
        .expr(Func::round(col!("b")))
        .expr(Func::round_with_precision(col!("b"), 2i32))
        .expr(Func::coalesce([col!("a"), "none".into()]))
        .expr(Func::random())
        .expr(Func::gen_random_uuid())
        .expr(Func::uuidv4())
        .expr(Func::uuidv7())
        .expr(Func::uuidv7_shifted(shift))
        .expr(Func::uuid_extract_timestamp(col!("u")))
        .expr(Func::uuid_extract_version(col!("u")))
        .to_owned()
}

fn subqueries() -> SelectStatement {
    let latest = Query::select()
        .expr(col!("id"))
        .expr(col!("at"))
        .from(table!("event"))
        .cond_where(qualified!("event", "account").eq(qualified!("a", "id")))
        .to_owned();
    let last = Query::select()
        .expr(Func::max(col!("at")))
        .from(table!("event"))
        .to_owned();
    let flagged = Query::select()
        .expr(1i64)
        .from(table!("flag"))
        .cond_where(col!("account").eq(qualified!("a", "id")))
        .to_owned();
    Query::select()
        .expr(qualified!("a", "id"))
        .expr_as(
            SimpleExpr::SubQuery(None, Box::new(SubQueryStatement::SelectStatement(last))),
            n("last"),
        )
        .from(FromItem::Table(account()))
        .join_lateral(JoinType::LeftJoin, latest, n("l"), Condition::all())
        .cond_where(Expr::exists(flagged))
        .to_owned()
}

fn array_subquery() -> SelectStatement {
    let events = Query::select()
        .expr(col!("id"))
        .from(table!("event"))
        .cond_where(col!("account").eq(qualified!("a", "id")))
        .to_owned();
    Query::select()
        .expr(qualified!("a", "id"))
        .expr_as(Expr::array(events.clone()), n("events"))
        .from(FromItem::Table(account()))
        .cond_where(
            Expr::expr(Expr::array(events)).binary(BinOper::Contains, Expr::val(vec![4i64])),
        )
        .to_owned()
}

fn from_subquery() -> SelectStatement {
    let inner = Query::select()
        .expr(col!("id"))
        .expr(col!("kind"))
        .from(table!("t"))
        .cond_where(col!("kind").binary(BinOper::NotEqual, "x"))
        .to_owned();
    Query::select()
        .expr(qualified!("sub", "id"))
        .expr(Expr::col(ColumnRef::TableAsterisk(n("sub"))))
        .from(FromItem::SubQuery(inner, n("sub")))
        .cross_join(table!("u"))
        .to_owned()
}

fn set_operations() -> SelectStatement {
    let ids = |name: &str| {
        Query::select()
            .expr(col!("id"))
            .from(table!(name))
            .to_owned()
    };
    ids("t")
        .union(UnionType::Distinct, ids("u"))
        .union(UnionType::All, ids("v"))
        .union(UnionType::Intersect, ids("w"))
        .union(UnionType::IntersectAll, ids("x"))
        .union(UnionType::Except, ids("y"))
        .union(UnionType::ExceptAll, ids("z"))
        .to_owned()
}

fn locks() -> SelectStatement {
    star!()
        .from(FromItem::Table(account()))
        .join(
            JoinType::InnerJoin,
            FromItem::Table(event()),
            qualified!("a", "id").eq(qualified!("e", "account_id")),
        )
        .lock_with_tables_behavior(LockType::Update, [n("a")], LockBehavior::SkipLocked)
        .to_owned()
}

fn with() -> SelectStatement {
    let recent = CommonTableExpression::new(
        n("recent"),
        Query::select()
            .expr(col!("id"))
            .from(table!("t"))
            .cond_where(col!("at").binary(BinOper::GreaterThan, 1i64))
            .to_owned(),
    )
    .column(n("id"))
    .materialized(true)
    .to_owned();
    let older = CommonTableExpression::new(
        n("older"),
        Query::select()
            .expr(col!("id"))
            .from(table!("recent"))
            .to_owned(),
    )
    .materialized(false)
    .to_owned();
    let mut clause = WithClause::new(recent);
    clause.cte(older);
    star!().from(table!("older")).with(clause).to_owned()
}

fn with_recursive() -> SelectStatement {
    let body = Query::select()
        .expr_as(1i32, n("n"))
        .union(
            UnionType::All,
            Query::select()
                .expr(col!("n").binary(BinOper::Add, 1i64))
                .from(table!("r"))
                .cond_where(col!("n").binary(BinOper::SmallerThan, 5i64))
                .to_owned(),
        )
        .to_owned();
    let clause = RecursiveWithClause::new(
        CommonTableExpression::new(n("r"), body)
            .column(n("n"))
            .to_owned(),
    )
    .search(Search::new(SearchOrder::DEPTH, col!("n"), n("ord")))
    .cycle(Cycle::new(col!("n"), n("looped"), n("path")))
    .to_owned();
    Query::select()
        .expr(col!("n"))
        .from(table!("r"))
        .with(clause)
        .to_owned()
}

fn conditions() -> SelectStatement {
    star!()
        .from(table!("t"))
        .cond_where(Condition::all())
        .cond_where(Condition::any())
        .cond_having(Condition::all().add(col!("a").eq(1i64)).not())
        .cond_where(
            Condition::any()
                .add(col!("b").eq(2i64))
                .add(col!("c").eq(3i64))
                .add(
                    Condition::all()
                        .add(col!("d").eq(4i64))
                        .add(col!("e").eq(5i64)),
                ),
        )
        .to_owned()
}

fn ordered_ids() -> SelectStatement {
    Query::select()
        .expr(col!("id"))
        .from(table!("t"))
        .order_by_expr(col!("score"), Order::Desc)
        .to_owned()
}

fn grouping_elements() -> SelectStatement {
    let sets = GroupingElement::sets(GroupingElement::set([col!("a"), col!("b")]))
        .add(GroupingElement::set([col!("b")]))
        .add(GroupingElement::empty())
        .add(GroupingElement::rollup([col!("a")]));
    Query::select()
        .expr(col!("a"))
        .expr(col!("b"))
        .expr(Func::count(col!("id")))
        .from(table!("t"))
        .add_group_by([col!("a")])
        .group_by_element(GroupingElement::rollup([col!("a"), col!("b")]))
        .group_by_element(GroupingElement::cube([
            col!("b"),
            SimpleExpr::from(Expr::tuple([col!("a"), col!("b")])),
        ]))
        .group_by_element(sets)
        .group_by_element(GroupingElement::empty())
        .group_by_element(GroupingElement::rollup(Vec::<SimpleExpr>::new()))
        .group_by_distinct()
        .to_owned()
}

fn similar_to() -> SelectStatement {
    star!()
        .from(table!("t"))
        .cond_where(
            Condition::all()
                .add(Expr::expr(col!("a")).similar_to("(x|y)%"))
                .add(Expr::expr(col!("b")).not_similar_to(LikeExpr::new("!%%").escape('!')))
                .add(Expr::expr(col!("c")).similar_to(LikeExpr::new("a\\_b").escape('\\')))
                .add(Expr::expr(col!("d")).similar_to("%z").not()),
        )
        .to_owned()
}

fn table_function_ordinality() -> SelectStatement {
    let tags = TableFunction::new(Func::unnest(qualified!("a", "tags")))
        .with_ordinality()
        .columns([n("tag"), n("position")])
        .alias(n("t"));
    Query::select()
        .expr(qualified!("a", "id"))
        .expr(qualified!("t", "tag"))
        .expr(qualified!("t", "position"))
        .from(FromItem::Table(account()))
        .cross_join(tags)
        .to_owned()
}

fn rows_from() -> SelectStatement {
    let letters = Value::Array(
        ArrayType::String,
        Some(Box::new(vec!["a".into(), "b".into(), "c".into()])),
    );
    let zipped = TableFunction::new(Func::generate_series_step(10i64, 1i64, -3i64))
        .and(Func::unnest(letters))
        .with_ordinality()
        .columns([n("x"), n("y"), n("n")])
        .alias(n("r"));
    let single = TableFunction::rows_from(Func::generate_series(1i64, 2i64)).alias(n("s"));
    Query::select()
        .expr(Expr::col(ColumnRef::TableAsterisk(n("r"))))
        .expr(qualified!("s", "s"))
        .from(zipped)
        .cross_join(single)
        .to_owned()
}

/// Calls returning `record`, a lone one with its list in the alias and one
/// carrying its own inside a `ROWS FROM` numbered `WITH ORDINALITY`.
fn record_functions() -> SelectStatement {
    let one = TableFunction::new(Func::json_to_record(col!("doc")))
        .column_definition(n("a"), ColumnType::Integer)
        .column_definition(n("tags"), ColumnType::Array(Arc::new(ColumnType::Text)))
        .alias(n("o"));
    let zipped = TableFunction::new(Func::jsonb_to_recordset(col!("docs")))
        .column_definition(n("b"), ColumnType::Text)
        .and(Func::generate_series(1i64, 2i64))
        .with_ordinality()
        .alias(n("z"));
    Query::select()
        .expr(Expr::col(ColumnRef::TableAsterisk(n("o"))))
        .expr(Expr::col(ColumnRef::TableAsterisk(n("z"))))
        .from(table!("t"))
        .cross_join(one)
        .cross_join(zipped)
        .to_owned()
}

fn tablesample() -> SelectStatement {
    let events = TableSample::new(
        NamedTable::from(TableName::Table(n("event"))),
        SampleMethod::Bernoulli,
        50,
    );
    Query::select()
        .expr(qualified!("a", "id"))
        .expr(qualified!("event", "kind"))
        .from(TableSample::new(account(), SampleMethod::System, 12.5).repeatable(42))
        .join(
            JoinType::LeftJoin,
            FromItem::from(events),
            qualified!("a", "id").eq(qualified!("event", "account_id")),
        )
        .lock_with_tables(LockType::Share, [n("a")])
        .to_owned()
}

pub(super) fn cases() -> Vec<(&'static str, (String, Values))> {
    vec![
        ("select-join-group", join_group().build()),
        ("select-star-distinct", star_distinct().build()),
        ("joins-every-kind", joins().build()),
        ("operators", operators().build()),
        ("membership", membership().build()),
        ("between", between().build()),
        ("patterns", patterns().build()),
        ("regex-and-json-path", regex_and_json_path().build()),
        ("matches-and-quantifiers", matches_and_quantifiers().build()),
        (
            "quantifiers-beyond-comparisons",
            quantifiers_beyond_comparisons().build(),
        ),
        ("casts-collations-subscripts", casts().build()),
        ("case", case().build()),
        ("values", values().build()),
        ("literals", literals().build()),
        ("functions", functions().build()),
        ("subqueries", subqueries().build()),
        ("from-subquery", from_subquery().build()),
        ("array-subquery", array_subquery().build()),
        ("set-operations", set_operations().build()),
        ("locks", locks().build()),
        (
            "lock-share",
            star!()
                .from(table!("t"))
                .lock_with_behavior(LockType::KeyShare, LockBehavior::Nowait)
                .build(),
        ),
        (
            "lock-no-key",
            star!()
                .from(table!("t"))
                .lock(LockType::NoKeyUpdate)
                .lock(LockType::Share)
                .build(),
        ),
        ("with", with().build()),
        ("with-recursive", with_recursive().build()),
        (
            "limits-reset",
            star!()
                .from(table!("t"))
                .limit(3)
                .offset(4)
                .reset_limit()
                .reset_offset()
                .limit(9_007_199_254_740_991)
                .build(),
        ),
        ("conditions", conditions().build()),
        (
            "tie-limit",
            ordered_ids()
                .limit_with_ties(5)
                .reset_limit()
                .limit(3)
                .limit_with_ties(2)
                .offset(1)
                .build(),
        ),
        (
            "tie-limit-replaced",
            ordered_ids().limit_with_ties(2).limit(4).build(),
        ),
        ("grouping-elements", grouping_elements().build()),
        (
            "group-by-distinct-ungrouped",
            Query::select()
                .expr(col!("a"))
                .from(table!("t"))
                .group_by_distinct()
                .build(),
        ),
        ("similar-to", similar_to().build()),
        (
            "table-function",
            star!()
                .from(TableFunction::new(Func::generate_series(1i64, 3i64)).alias(n("n")))
                .build(),
        ),
        (
            "table-function-ordinality",
            table_function_ordinality().build(),
        ),
        ("rows-from", rows_from().build()),
        ("record-functions", record_functions().build()),
        ("tablesample", tablesample().build()),
        (
            "expression-inspect",
            Query::select()
                .expr(col!("x").binary(BinOper::Add, 7i16))
                .build(),
        ),
        (
            "condition-inspect",
            Query::select()
                .expr(SimpleExpr::Constant(true.into()))
                .cond_where(
                    Condition::any()
                        .add(col!("a").eq(1i64))
                        .add(Expr::expr(col!("b")).is_null()),
                )
                .build(),
        ),
    ]
}
