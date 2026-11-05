use pgorm_query::{AliasName, Name, Value, Values, alias};

use crate::tests_cfg::{cake, cake_filling_price, fruit, lunch_set};

use super::adapter::compile_text;
use super::*;

const INVOICE: AliasName = alias("invoice");
const CUSTOMER: AliasName = alias("customer");
const TOTAL: AliasName = alias("total");
const CUSTOMER_ID: AliasName = alias("customer_id");
const ID: AliasName = alias("id");

fn total<'brand>() -> Expr<'brand> {
    col(INVOICE, TOTAL)
}

/// Golden output plus the pg_query oracle: the emitted SQL must be a string
/// the real PostgreSQL grammar accepts.
// [spec:pgorm:req:pipeline.errors+4/test]
fn sql_of(pipeline: Pipeline) -> String {
    let (sql, _) = pipeline.into_sql().expect("pipeline compiles");
    if let Err(err) = pg_query::parse(&sql, pg_query::ParserOptions::DEFAULT) {
        panic!("PostgreSQL grammar rejected the emitted SQL: {err}\n  {sql}");
    }
    sql
}

// [spec:pgorm:def:pipeline.adapter+2/test]    direct PL construction is
// interchangeable with compiling the equivalent PRQL text
#[test]
fn built_filter_matches_text_compilation() {
    let built =
        sql_of(Pipeline::from(INVOICE).filter_with(|binder| total().gt(binder.bind(5_i64))));
    let text = compile_text("from invoice | filter invoice.total > $1").expect("compiles");
    assert_eq!(built, text);
}

// [spec:pgorm:sem:pipeline.qualify+3/test]
#[test]
fn schema_qualified_from_renders_both_parts() {
    let built = sql_of(Pipeline::from_schema(alias("archive"), INVOICE));
    assert_eq!(built, "SELECT * FROM archive.invoice");
}

// [spec:pgorm:sem:pipeline.qualify+3/test]
#[test]
fn quoted_identifiers_survive_rendering() {
    let table = alias("User Order");
    let built = sql_of(Pipeline::from(table).select(col(table, alias("Total Price"))));
    assert_eq!(built, r#"SELECT "Total Price" FROM "User Order""#);
}

// [spec:pgorm:sem:pipeline.qualify+3/test]
#[test]
fn entity_source_uses_table_metadata() {
    let built = sql_of(Pipeline::from(cake::Entity));
    assert_eq!(built, "SELECT * FROM cake");
}

// [spec:pgorm:sem:pipeline.qualify+3/test]
#[test]
fn entity_source_honours_schema_name() {
    let built = sql_of(Pipeline::from(cake_filling_price::Entity));
    assert_eq!(built, "SELECT * FROM public.cake_filling_price");
}

// [spec:pgorm:sem:pipeline.qualify+3/test]    a column carries its own table
#[test]
fn entity_columns_are_qualified_by_construction() {
    let built = sql_of(
        Pipeline::from(cake::Entity)
            .join(
                JoinSide::Inner,
                fruit::Entity,
                cake::Column::Id.eq(fruit::Column::CakeId),
            )
            .select((cake::Column::Name, fruit::Column::Name)),
    );
    assert_eq!(
        built,
        "SELECT cake.name AS _expr_0, fruit.name FROM cake \
         INNER JOIN fruit ON cake.id = fruit.cake_id"
    );
}

// [spec:pgorm:req:pipeline.surface+4/test]
#[test]
fn cast_renders_as_postgres_cast() {
    let built = sql_of(Pipeline::from(INVOICE).derive(total().cast(CastType::Integer).as_("t")));
    assert_eq!(built, "SELECT *, CAST(total AS integer) AS t FROM invoice");
}

// [spec:pgorm:req:pipeline.surface+4/test]
#[test]
fn in_array_of_bound_params_renders_in_list() {
    let built = sql_of(
        Pipeline::from(INVOICE)
            .filter_with(|binder| total().in_array([binder.bind(1_i64), binder.bind(2_i64)])),
    );
    assert_eq!(built, "SELECT * FROM invoice WHERE total IN ($1, $2)");
}

// [spec:pgorm:req:pipeline.surface+4/test]
#[test]
fn in_array_of_literals_inlines_them() {
    let built = sql_of(Pipeline::from(INVOICE).filter(total().in_array([1, 2, 3])));
    assert_eq!(built, "SELECT * FROM invoice WHERE total IN (1, 2, 3)");
}

// [spec:pgorm:req:pipeline.surface+4/test]
#[test]
fn take_range_renders_limit_offset() {
    let built = sql_of(Pipeline::from(INVOICE).take_range(21..=30));
    assert_eq!(built, "SELECT * FROM invoice LIMIT 10 OFFSET 20");
}

// [spec:pgorm:req:pipeline.surface+4/test]
#[test]
fn case_renders_case_when() {
    let built = sql_of(
        Pipeline::from(INVOICE)
            .derive(case([(total().gt(100), "big")], "small").as_(alias("bucket"))),
    );
    assert_eq!(
        built,
        "SELECT *, CASE WHEN total > 100 THEN 'big' ELSE 'small' END AS bucket FROM invoice"
    );
}

// [spec:pgorm:req:pipeline.params+4/test]    a string literal is escaped, not
// interpolated: it cannot close the quote it is written into
#[test]
fn string_literals_are_escaped() {
    let built = sql_of(Pipeline::from(INVOICE).filter(col(INVOICE, alias("note")).eq("o'clock")));
    assert_eq!(built, "SELECT * FROM invoice WHERE note = 'o''clock'");
}

/// The string constant the PostgreSQL parser recovered from `sql`, decoded
/// by the server's own scanner — so an ordinary literal and an `E''` one are
/// compared by what they denote rather than by how they were written.
fn parsed_string(sql: &str) -> Option<String> {
    let parsed = pg_query::parse(sql, pg_query::ParserOptions::DEFAULT).ok()?;
    parsed.protobuf.nodes().into_iter().find_map(|node| {
        let pg_query::NodeRef::AConst(constant) = node.0 else {
            return None;
        };
        match constant.val.as_ref()? {
            pg_query::protobuf::a_const::Val::Sval(text) => Some(text.sval.clone()),
            _ => None,
        }
    })
}

// [spec:pgorm:req:pipeline.params+4/test]    an inlined literal is data: it
// parses as one statement and denotes exactly the value it was given
#[test]
fn hostile_literals_stay_one_statement() {
    let payloads = [
        "o'clock",
        "\\'; SELECT 1; --",
        "\\' OR TRUE --",
        "'; DROP TABLE invoice; --",
        "a''b",
        "'",
        "\\",
        "a\\\\'b",
        "back\\slash",
        "costs $1",
        "E'x'",
        "a\nb\tc",
        "nul\u{1a}end",
    ];
    for payload in payloads {
        let sql = sql_of(Pipeline::from(INVOICE).filter(col(INVOICE, alias("note")).eq(payload)));
        let parsed =
            pg_query::parse(&sql, pg_query::ParserOptions::DEFAULT).expect("grammar accepts");
        assert_eq!(
            parsed.protobuf.stmts.len(),
            1,
            "a value became a second statement: {sql}"
        );
        assert_eq!(
            parsed_string(&sql).as_deref(),
            Some(payload),
            "the literal does not denote its value: {sql}"
        );
    }
}

// [spec:pgorm:req:pipeline.surface+4/test]
#[test]
fn null_handling_renders_is_null_forms() {
    let built = sql_of(Pipeline::from(INVOICE).filter(total().is_null().or(total().is_not_null())));
    assert_eq!(
        built,
        "SELECT * FROM invoice WHERE total IS NULL OR total IS NOT NULL"
    );
}

// [spec:pgorm:req:pipeline.surface+4/test]
#[test]
fn coalesce_and_arithmetic_render_inline() {
    let built = sql_of(Pipeline::from(INVOICE).derive([
        (total().coalesce(0.0) * 1.1).as_(alias("gross")),
        (total() + 1 - 2).as_(alias("adjusted")),
        total().div(2.0).as_(alias("half")),
        total().rem(10).as_(alias("cents")),
    ]));
    assert_eq!(
        built,
        "SELECT *, COALESCE(total, 0.0) * 1.1 AS gross, total + 1 - 2 AS adjusted, \
         (total * 1.0 / 2.0) AS half, total % 10 AS cents FROM invoice"
    );
}

// [spec:pgorm:req:pipeline.surface+4/test]
#[test]
fn aggregate_functions_render_expected_sql() {
    let built = sql_of(
        Pipeline::from(INVOICE)
            .group(col(INVOICE, CUSTOMER_ID))
            .aggregate((
                sum(total()).as_("s"),
                min(total()).as_("lo"),
                max(total()).as_("hi"),
                average(total()).as_("mean"),
                stddev(total()).as_("sd"),
                count_rows().as_("n"),
                count_distinct(total()).as_("distinct_totals"),
            )),
    );
    assert_eq!(
        built,
        "SELECT customer_id, COALESCE(SUM(total), 0) AS s, MIN(total) AS lo, MAX(total) AS hi, \
         AVG(total) AS mean, STDDEV(total) AS sd, COUNT(*) AS n, \
         COUNT(DISTINCT total) AS distinct_totals FROM invoice GROUP BY customer_id"
    );
}

/// `count(expr)` counts the expression and `count_rows()` counts rows; the
/// two are different answers wherever the expression is nullable, so the
/// counted expression has to reach the SQL.
// [spec:pgorm:sem:pipeline.count-argument/test]
#[test]
fn a_counted_expression_reaches_the_aggregate() {
    let built = sql_of(
        Pipeline::from(INVOICE)
            .group(col(INVOICE, CUSTOMER_ID))
            .aggregate((count(total()).as_("scored"), count_rows().as_("n"))),
    );
    assert_eq!(
        built,
        "SELECT customer_id, COUNT(total) AS scored, COUNT(*) AS n \
         FROM invoice GROUP BY customer_id"
    );
}

/// The aggregate a `HAVING` inlines is the written one, not prqlc's.
// [spec:pgorm:sem:pipeline.count-argument/test]
#[test]
fn a_counted_expression_inlines_into_having() {
    let scored = alias("scored");
    let built = sql_of(
        Pipeline::from(INVOICE)
            .group(col(INVOICE, CUSTOMER_ID))
            .aggregate(count(total()).as_(scored))
            .filter_with(|binder| scored.gt(binder.bind(1_i64))),
    );
    assert_eq!(
        built,
        "SELECT customer_id, COUNT(total) AS scored FROM invoice \
         GROUP BY customer_id HAVING COUNT(total) > $1"
    );
}

/// Every position a counted expression can be written in, and the clause it
/// carries there: none inside an `aggregate`, the window's own inside a
/// [`window`](Pipeline::window), and the empty one everywhere else — which
/// is the implicit window prqlc gives an aggregate used outside a grouping.
// [spec:pgorm:sem:pipeline.count-argument/test]
#[test]
fn a_counted_expression_carries_its_own_window() {
    let scored = alias("scored");
    let cases = [
        (
            Pipeline::from(INVOICE).derive(count(total()).as_(scored)),
            "SELECT *, COUNT(total) OVER () AS scored FROM invoice",
        ),
        (
            Pipeline::from(INVOICE).select(count(total()).as_(scored)),
            "SELECT COUNT(total) OVER () AS scored FROM invoice",
        ),
        (
            Pipeline::from(INVOICE).window(count(total()).as_(scored), over()),
            "SELECT *, COUNT(total) OVER () AS scored FROM invoice",
        ),
        (
            Pipeline::from(INVOICE)
                .window(count(total()).as_(scored), by(col(INVOICE, CUSTOMER_ID))),
            "SELECT *, COUNT(total) OVER (PARTITION BY customer_id) AS scored FROM invoice",
        ),
        (
            Pipeline::from(INVOICE).window(
                count(total()).as_(scored),
                sort_by(col(INVOICE, ID)).rows(Some(-1), Some(0)),
            ),
            "SELECT *, COUNT(total) OVER (ORDER BY id ROWS BETWEEN 1 PRECEDING \
             AND CURRENT ROW) AS scored FROM invoice ORDER BY id",
        ),
    ];
    for (pipeline, expected) in cases {
        assert_eq!(sql_of(pipeline), expected);
    }
}

/// `count_rows()` is PRQL's `count this`, which prqlc renders as `COUNT(*)`
/// already — including the window it is given.
// [spec:pgorm:sem:pipeline.count-argument/test]
#[test]
fn counting_rows_is_still_left_to_prqlc() {
    let built = sql_of(Pipeline::from(INVOICE).window(
        count_rows().as_(alias("n")),
        sort_by(col(INVOICE, ID)).rows(Some(-1), Some(0)),
    ));
    assert_eq!(
        built,
        "SELECT *, COUNT(*) OVER (ORDER BY id ROWS BETWEEN 1 PRECEDING \
         AND CURRENT ROW) AS n FROM invoice ORDER BY id"
    );
}

/// A written call holds its argument inside an expression pgorm spelled, so
/// every rewrite that walks the tree has to walk into one: embedding a
/// pipeline shifts the placeholders of the counted expression like any other.
// [spec:pgorm:sem:pipeline.count-argument/test]
#[test]
fn an_embedded_count_renumbers_its_placeholder() {
    let held = alias("held");
    let counted = Pipeline::from(INVOICE).derive_with(|binder| {
        let floor = binder.bind(5_i64);
        [count(case([(total().gt(floor), total())], null())).as_(alias("scored"))]
    });
    let (sql, values) = Pipeline::from(INVOICE)
        .filter_with(|binder| total().gt(binder.bind(1_i64)))
        .join(
            JoinSide::Inner,
            counted.named(held),
            col(INVOICE, ID).eq(col(held, ID)),
        )
        .select(col(held, alias("scored")))
        .into_sql()
        .expect("pipeline compiles");
    assert_eq!(values.0, [Value::BigInt(Some(1)), Value::BigInt(Some(5))]);
    assert!(
        sql.contains("COUNT(CASE WHEN total > $2 THEN total ELSE NULL END) OVER ()"),
        "{sql}"
    );
    assert!(sql.contains("total > $1"), "{sql}");
}

/// `count_distinct` already keeps its column — prqlc's standard library
/// renders it with one — so it is left alone in every position.
// [spec:pgorm:sem:pipeline.count-argument/test]
#[test]
fn counting_distinct_values_needs_no_rewrite() {
    let built = sql_of(Pipeline::from(INVOICE).window(
        count_distinct(total()).as_(alias("d")),
        sort_by(col(INVOICE, ID)).rows(Some(-1), Some(0)),
    ));
    assert_eq!(
        built,
        "SELECT *, COUNT(DISTINCT total) OVER (ORDER BY id ROWS BETWEEN 1 PRECEDING \
         AND CURRENT ROW) AS d FROM invoice ORDER BY id"
    );
}

// [spec:pgorm:req:pipeline.surface+4/test]
#[test]
fn filter_after_aggregate_lands_in_having() {
    let spent = alias("total_spent");
    let built = sql_of(
        Pipeline::from(INVOICE)
            .group(col(INVOICE, CUSTOMER_ID))
            .aggregate(sum(total()).as_(spent))
            .filter_with(|binder| spent.gt(binder.bind(40.0_f64)))
            .sort(spent.desc())
            .take(5),
    );
    assert_eq!(
        built,
        "SELECT customer_id, COALESCE(SUM(total), 0) AS total_spent FROM invoice \
         GROUP BY customer_id HAVING COALESCE(SUM(total), 0) > $1 \
         ORDER BY total_spent DESC LIMIT 5"
    );
    let parsed = parsed_select(&built);
    assert!(parsed.having_clause.is_some());
}

// [spec:pgorm:req:pipeline.surface+4/test]
#[test]
fn filter_after_window_nests_through_cte() {
    let rn = alias("rn");
    let built = sql_of(
        Pipeline::from(INVOICE)
            .window(
                row_number().as_(rn),
                by(col(INVOICE, CUSTOMER_ID)).sort_by(total().desc()),
            )
            .filter_with(|binder| rn.lte(binder.bind(2_i64))),
    );
    assert_eq!(
        built,
        "WITH table_0 AS (SELECT *, ROW_NUMBER() OVER (PARTITION BY customer_id \
         ORDER BY total DESC) AS rn FROM invoice) SELECT * FROM table_0 WHERE rn <= $1"
    );
    let parsed = parsed_select(&built);
    assert!(parsed.with_clause.is_some());
    assert!(parsed.where_clause.is_some());
}

// [spec:pgorm:req:pipeline.surface+4/test]
#[test]
fn window_frame_renders_rows_between() {
    let built = sql_of(Pipeline::from(INVOICE).window(
        sum(total()).as_(alias("running")),
        sort_by(col(INVOICE, ID)).rows(Some(-2), Some(0)),
    ));
    assert_eq!(
        built,
        "SELECT *, SUM(total) OVER (ORDER BY id ROWS BETWEEN 2 PRECEDING AND CURRENT ROW) AS running FROM invoice ORDER BY id"
    );
}

// [spec:pgorm:req:pipeline.surface+4/test]
#[test]
fn window_over_the_whole_relation_needs_no_keys() {
    let built = sql_of(Pipeline::from(INVOICE).window(sum(total()).as_(alias("grand")), over()));
    assert_eq!(built, "SELECT *, SUM(total) OVER () AS grand FROM invoice");
}

/// Both bounds run over their whole range independently: two preceding, one
/// each side, two following, and unbounded on either side. `LAST_VALUE` is
/// the reading, not the writing, of a frame — the function whose answer is
/// the frame — and it is the one prqlc renders without one.
// [spec:pgorm:sem:pipeline.window-frame/test]
#[test]
fn every_frame_direction_reaches_a_frame_blind_function() {
    for (start, end, frame) in [
        (
            Some(-2),
            Some(-1),
            "ROWS BETWEEN 2 PRECEDING AND 1 PRECEDING",
        ),
        (
            Some(-1),
            Some(0),
            "ROWS BETWEEN 1 PRECEDING AND CURRENT ROW",
        ),
        (
            Some(-1),
            Some(1),
            "ROWS BETWEEN 1 PRECEDING AND 1 FOLLOWING",
        ),
        (Some(1), Some(1), "ROWS BETWEEN 1 FOLLOWING AND 1 FOLLOWING"),
        (Some(1), Some(2), "ROWS BETWEEN 1 FOLLOWING AND 2 FOLLOWING"),
        (
            None,
            Some(0),
            "ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW",
        ),
        (
            Some(0),
            None,
            "ROWS BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING",
        ),
        (
            None,
            None,
            "ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING",
        ),
    ] {
        let built = sql_of(Pipeline::from(INVOICE).window(
            last(total()).as_(alias("tail")),
            sort_by(col(INVOICE, ID)).rows(start, end),
        ));
        assert_eq!(
            built,
            format!(
                "SELECT *, LAST_VALUE(total) OVER (ORDER BY id {frame}) \
                 AS tail FROM invoice ORDER BY id"
            )
        );
    }
}

/// The filed shape: a partitioned window ordered descending, reading the one
/// row that follows. The frame is the whole of what it computes.
// [spec:pgorm:sem:pipeline.window-frame/test]
#[test]
fn the_filed_following_frame_partitions_and_orders_descending() {
    let built = sql_of(
        Pipeline::from(INVOICE).window(
            last(total()).as_(alias("next")),
            by(col(INVOICE, CUSTOMER_ID))
                .sort_by(col(INVOICE, ID).desc())
                .rows(Some(1), Some(1)),
        ),
    );
    assert_eq!(
        built,
        "SELECT *, LAST_VALUE(total) OVER (PARTITION BY customer_id ORDER BY id DESC \
         ROWS BETWEEN 1 FOLLOWING AND 1 FOLLOWING) AS next FROM invoice"
    );
}

/// One window, two rendering paths — prqlc's for the aggregate, pgorm's for
/// the frame-blind call — spelling one clause.
// [spec:pgorm:sem:pipeline.window-frame/test]
#[test]
fn both_render_paths_spell_one_frame() {
    let built = sql_of(
        Pipeline::from(INVOICE).window(
            (
                sum(total()).as_(alias("run")),
                first(total()).as_(alias("head")),
            ),
            by(col(INVOICE, CUSTOMER_ID))
                .sort_by(col(INVOICE, ID))
                .rows(Some(-1), Some(1)),
        ),
    );
    let clause =
        "OVER (PARTITION BY customer_id ORDER BY id ROWS BETWEEN 1 PRECEDING AND 1 FOLLOWING)";
    assert_eq!(
        built,
        format!(
            "SELECT *, SUM(total) {clause} AS run, FIRST_VALUE(total) {clause} \
             AS head FROM invoice"
        )
    );
}

/// `RANGE` is the other unit, and reaches a frame-blind call the same way.
// [spec:pgorm:sem:pipeline.window-frame/test]
#[test]
fn a_range_frame_reaches_a_frame_blind_function() {
    let built = sql_of(Pipeline::from(INVOICE).window(
        first(total()).as_(alias("head")),
        sort_by(col(INVOICE, ID)).range(Some(-1), Some(1)),
    ));
    assert_eq!(
        built,
        "SELECT *, FIRST_VALUE(total) OVER (ORDER BY id RANGE BETWEEN 1 PRECEDING AND 1 FOLLOWING) \
         AS head FROM invoice ORDER BY id"
    );
}

/// The offset functions take their offset first and render it last, and the
/// ranking ones drop the column PRQL gives them.
// [spec:pgorm:sem:pipeline.window-frame/test]
#[test]
fn a_written_call_keeps_its_argument_order() {
    let built = sql_of(Pipeline::from(INVOICE).window(
        (
            lag(2, total()).as_(alias("prev")),
            lead(3, total()).as_(alias("next")),
            rank(total()).as_(alias("r")),
            rank_dense(total()).as_(alias("rd")),
            row_number().as_(alias("n")),
        ),
        sort_by(col(INVOICE, ID)).rows(Some(0), None),
    ));
    let clause = "OVER (ORDER BY id ROWS BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING)";
    assert_eq!(
        built,
        format!(
            "SELECT *, LAG(total, 2) {clause} AS prev, LEAD(total, 3) {clause} AS next, \
             RANK() {clause} AS r, DENSE_RANK() {clause} AS rd, ROW_NUMBER() {clause} AS n \
             FROM invoice ORDER BY id"
        )
    );
}

/// An unpartitioned window that states no ordering reads the relation's, so
/// the written clause has to as well — otherwise the frame would run over a
/// different order than the one prqlc gives the aggregates beside it.
// [spec:pgorm:sem:pipeline.window-frame/test]
#[test]
fn a_framed_window_reads_the_carried_ordering() {
    let built = sql_of(
        Pipeline::from(INVOICE)
            .sort(col(INVOICE, ID).desc())
            .window(
                first(total()).as_(alias("head")),
                over().rows(Some(-1), Some(0)),
            ),
    );
    assert_eq!(
        built,
        "SELECT *, FIRST_VALUE(total) OVER (ORDER BY id DESC ROWS BETWEEN 1 PRECEDING \
         AND CURRENT ROW) AS head FROM invoice ORDER BY id DESC"
    );
}

/// Nothing is rewritten without an authored frame: the stage stays prqlc's,
/// and a partitioned window still nests its ordering inside the `group`.
// [spec:pgorm:sem:pipeline.window-frame/test]
#[test]
fn an_unframed_window_is_left_to_prqlc() {
    let built = sql_of(Pipeline::from(INVOICE).window(
        first(total()).as_(alias("head")),
        by(col(INVOICE, CUSTOMER_ID)).sort_by(col(INVOICE, ID)),
    ));
    assert_eq!(
        built,
        "SELECT *, FIRST_VALUE(total) OVER (PARTITION BY customer_id ORDER BY id) \
         AS head FROM invoice"
    );
}

/// A filter after a written window still nests it in a CTE: the rewritten
/// column is a window function wherever prqlc puts it.
// [spec:pgorm:sem:pipeline.window-frame/test]
#[test]
fn a_filter_after_a_written_window_nests_it() {
    let built = sql_of(
        Pipeline::from(INVOICE)
            .window(
                first(total()).as_(alias("head")),
                sort_by(col(INVOICE, ID)).rows(Some(1), Some(1)),
            )
            .filter(alias("head").gt(5)),
    );
    assert_eq!(
        built,
        "WITH table_0 AS (SELECT *, FIRST_VALUE(total) OVER (ORDER BY id \
         ROWS BETWEEN 1 FOLLOWING AND 1 FOLLOWING) AS head FROM invoice) \
         SELECT * FROM table_0 WHERE head > 5 ORDER BY id"
    );
}

// [spec:pgorm:req:pipeline.surface+4/test]
#[test]
fn lag_lead_first_last_render_window_calls() {
    let built = sql_of(Pipeline::from(INVOICE).window(
        (
            lag(1, total()).as_("prev"),
            lead(1, total()).as_("next"),
            first(total()).as_("head"),
            last(total()).as_("tail"),
            rank(total()).as_("r"),
            rank_dense(total()).as_("rd"),
        ),
        by(col(INVOICE, CUSTOMER_ID)).sort_by(total()),
    ));
    assert_eq!(
        built,
        "SELECT *, LAG(total, 1) OVER (PARTITION BY customer_id ORDER BY total) AS prev, \
         LEAD(total, 1) OVER (PARTITION BY customer_id ORDER BY total) AS next, \
         FIRST_VALUE(total) OVER (PARTITION BY customer_id ORDER BY total) AS head, \
         LAST_VALUE(total) OVER (PARTITION BY customer_id ORDER BY total) AS tail, \
         RANK() OVER (PARTITION BY customer_id ORDER BY total) AS r, \
         DENSE_RANK() OVER (PARTITION BY customer_id ORDER BY total) AS rd FROM invoice"
    );
}

// [spec:pgorm:req:pipeline.surface+4/test]
#[test]
fn join_takes_an_explicit_condition() {
    let built = sql_of(Pipeline::from(INVOICE).join(
        JoinSide::Left,
        CUSTOMER,
        col(INVOICE, CUSTOMER_ID).eq(col(CUSTOMER, ID)),
    ));
    assert_eq!(
        built,
        "SELECT invoice.*, customer.* FROM invoice \
         LEFT OUTER JOIN customer ON invoice.customer_id = customer.id"
    );
}

// [spec:pgorm:req:pipeline.params+4/test]
#[test]
fn placeholders_number_in_bind_order_across_stages() {
    let gross = alias("gross");
    let (sql, values) = Pipeline::from(INVOICE)
        .filter_with(|binder| total().gt(binder.bind(1_i64)))
        .derive_with(|binder| [(total() * binder.bind(2.0_f64)).as_(gross)])
        .filter_with(|binder| gross.lt(binder.bind(3_i64)))
        .into_sql()
        .expect("pipeline compiles");
    assert_eq!(values.0.len(), 3);
    for placeholder in ["$1", "$2", "$3"] {
        assert!(sql.contains(placeholder), "{placeholder} missing: {sql}");
    }
}

// [spec:pgorm:req:pipeline.params+4/test]    a literal is inlined, a bound
// value is not
#[test]
fn literals_inline_and_bound_values_do_not() {
    let (sql, values) = Pipeline::from(INVOICE)
        .filter(total().gt(10))
        .filter_with(|binder| col(INVOICE, CUSTOMER_ID).eq(binder.bind(7_i32)))
        .into_sql()
        .expect("pipeline compiles");
    assert_eq!(
        sql,
        "SELECT * FROM invoice WHERE total > 10 AND customer_id = $1"
    );
    assert_eq!(values.0.len(), 1);
}

// [spec:pgorm:req:pipeline.surface+4/test]    one, a list, or a mix
#[test]
fn expression_lists_take_every_shape() {
    let one = sql_of(Pipeline::from(cake::Entity).select(cake::Column::Id));
    assert_eq!(one, "SELECT id FROM cake");

    let array = sql_of(Pipeline::from(cake::Entity).select([cake::Column::Id, cake::Column::Name]));
    assert_eq!(array, "SELECT id, name FROM cake");

    let owned = sql_of(Pipeline::from(cake::Entity).select(vec![cake::Column::Name]));
    assert_eq!(owned, "SELECT name FROM cake");

    let n = alias("n");
    let mixed = sql_of(
        Pipeline::from(cake::Entity)
            .derive(cake::Column::Id.add(1).as_(n))
            .select((cake::Column::Name, n, cake::Column::Id.mul(2))),
    );
    assert_eq!(mixed, "SELECT name, id + 1 AS n, id * 2 FROM cake");
}

// [spec:pgorm:req:pipeline.surface+4/test]
#[test]
fn scopes_compose_as_pipeline_functions() {
    fn expensive(pipeline: Pipeline) -> Pipeline {
        pipeline.filter_with(|binder| total().gt(binder.bind(100_i64)))
    }
    fn newest_first(pipeline: Pipeline) -> Pipeline {
        pipeline.sort(col(INVOICE, alias("invoice_date")).desc())
    }
    let built = sql_of(newest_first(expensive(Pipeline::from(INVOICE))).take(3));
    assert_eq!(
        built,
        "SELECT * FROM invoice WHERE total > $1 ORDER BY invoice_date DESC LIMIT 3"
    );
}

// [spec:pgorm:req:pipeline.errors+4/test]
#[test]
fn reserved_alias_is_a_typed_error() {
    let err = Pipeline::from(INVOICE)
        .derive(total().as_("sum"))
        .into_sql()
        .expect_err("reserved alias must be refused");
    assert_eq!(err, PipelineError::ReservedAlias("sum".to_owned()));
}

// [spec:pgorm:req:pipeline.errors+4/test]
#[test]
fn stdlib_name_reference_is_a_compile_error() {
    let err = Pipeline::from(INVOICE)
        .filter(alias("count").gt(1))
        .into_sql()
        .expect_err("std name as a value must be refused");
    assert!(matches!(err, PipelineError::Compile(_)));
}

// [spec:pgorm:req:pipeline.errors+4/test]    an unattached token is not a
// compile-time error; the server answers for it
#[test]
fn unattached_alias_token_compiles_to_a_column_reference() {
    let built = sql_of(Pipeline::from(INVOICE).filter(alias("never_declared").gt(1)));
    assert_eq!(built, "SELECT * FROM invoice WHERE never_declared > 1");
}

/// The name that exfiltrates when the build resolved registry prqlc
/// 0.13.14: sqlparser's escaper leaves a `"` preceded by a backslash alone,
/// so the quoted identifier ends at the backslash-quote and the rest is read
/// as SQL. Rendered into an alias position — where the leading name need not
/// resolve — `SELECT 'alice' AS "x\" , (SELECT password FROM secret) AS
/// "leak"` returns the secret beside the row against a live server. The
/// fork pgorm depends on doubles the quote instead, which is the whole point:
/// a consumer can patch that dependency away, so the outcome would otherwise
/// be a property of the consumer's dependency graph.
const EXFILTRATING: &str = r#"x\" , (SELECT password FROM secret) AS "leak"#;

/// Every identifier the pipeline can be given at runtime reaches
/// `collect_identifiers`, whichever constructor minted it.
// [spec:pgorm:req:pipeline.errors+4/test]
fn refuses(pipeline: Pipeline) {
    let err = pipeline
        .into_sql()
        .expect_err("an unquotable identifier must be refused");
    assert_eq!(
        err,
        PipelineError::UnquotableIdentifier(EXFILTRATING.to_owned())
    );
}

// [spec:pgorm:req:pipeline.errors+4/test]
#[test]
fn exfiltrating_column_name_is_refused() {
    refuses(Pipeline::from(INVOICE).select(col(INVOICE, Name::runtime(EXFILTRATING))));
}

// [spec:pgorm:req:pipeline.errors+4/test]
#[test]
fn exfiltrating_runtime_source_name_is_refused() {
    refuses(Pipeline::from(named_runtime(
        INVOICE,
        Name::runtime(EXFILTRATING),
    )));
}

// [spec:pgorm:req:pipeline.errors+4/test]
#[test]
fn exfiltrating_schema_name_is_refused() {
    refuses(Pipeline::from_schema(Name::runtime(EXFILTRATING), INVOICE));
}

// [spec:pgorm:req:pipeline.errors+4/test]
#[test]
fn exfiltrating_runtime_alias_is_refused() {
    refuses(Pipeline::from(INVOICE).derive(total().as_runtime(Name::runtime(EXFILTRATING))));
}

// [spec:pgorm:req:pipeline.errors+4/test]    the backslash is what defeats
// one escaper; a bare quote is the same representability problem and is
// refused on its own
#[test]
fn a_bare_quote_in_an_identifier_is_refused() {
    let err = Pipeline::from(INVOICE)
        .select(col(INVOICE, Name::runtime("dis\"count")))
        .into_sql()
        .expect_err("a quote in an identifier must be refused");
    assert_eq!(
        err,
        PipelineError::UnquotableIdentifier("dis\"count".to_owned())
    );
}

// [spec:pgorm:req:pipeline.errors+4/test]
#[test]
fn a_nul_byte_in_an_identifier_is_refused() {
    let err = Pipeline::from(INVOICE)
        .select(col(INVOICE, Name::runtime("tot\0al")))
        .into_sql()
        .expect_err("a NUL in an identifier must be refused");
    assert_eq!(
        err,
        PipelineError::UnquotableIdentifier("tot\0al".to_owned())
    );
}

// [spec:pgorm:req:pipeline.errors+4/test]    only the quote and the NUL are
// refused: a backslash alone means nothing inside a quoted identifier, and
// both compilers render it the same way
#[test]
fn a_backslash_without_a_quote_still_renders() {
    let table = alias("share");
    let built = sql_of(Pipeline::from(table).select(col(table, Name::runtime("a\\b"))));
    assert_eq!(built, r#"SELECT "a\b" FROM share"#);
}

/// Names prqlc writes bare that PostgreSQL's lexer does not read back as a
/// name: a parameter, two dollar-quote delimiters, and the wildcard.
const LEXED_AS_SOMETHING_ELSE: [&str; 4] = ["$1", "$$", "$tag$", "*"];

// [spec:pgorm:req:pipeline.errors+4/test]    a leading `$` would be written
// bare and lexed as a parameter or a dollar quote, and `*` is the compiler's
// wildcard, so each is refused at every kind of position: a column, a
// relation, a schema, a join relation and an alias
#[test]
fn a_leading_dollar_or_star_is_refused() {
    for name in LEXED_AS_SOMETHING_ELSE {
        let runtime = || Name::runtime(name);
        let pipelines = [
            Pipeline::from(INVOICE).select(col(INVOICE, runtime())),
            Pipeline::from(runtime()),
            Pipeline::from_schema(runtime(), INVOICE),
            Pipeline::from(INVOICE).join(JoinSide::Inner, runtime(), this(ID).eq(that(ID))),
            Pipeline::from(INVOICE)
                .group(runtime())
                .aggregate(count_rows()),
            Pipeline::from(INVOICE).derive(total().as_runtime(runtime())),
        ];
        for pipeline in pipelines {
            assert_eq!(
                pipeline.into_sql(),
                Err(PipelineError::UnquotableIdentifier(name.to_owned())),
                "`{name}` must be refused"
            );
        }
    }
}

// [spec:pgorm:req:pipeline.errors+4/test]    `$` is refused only where it
// leads: after the first character it is an identifier character to
// PostgreSQL, so `a$1` reads back as one column and the census still counts
// only the binder's placeholder
#[test]
fn an_inner_dollar_still_renders_as_a_name() {
    let (sql, values) = Pipeline::from(INVOICE)
        .filter_with(|binder| col(INVOICE, Name::runtime("a$1")).eq(binder.bind(7_i32)))
        .select(col(INVOICE, Name::runtime("b$$c")))
        .into_sql()
        .expect("an inner `$` is an ordinary name");
    assert_eq!(sql, "SELECT b$$c FROM invoice WHERE a$1 = $1");
    let select = parsed_select(&sql);
    let column = |node: &pg_query::protobuf::Node| match node.node.as_ref() {
        Some(pg_query::NodeEnum::ColumnRef(column)) => match column.fields[0].node.as_ref() {
            Some(pg_query::NodeEnum::String(name)) => name.sval.clone(),
            other => panic!("expected a column name, got {other:?}"),
        },
        other => panic!("expected a column reference, got {other:?}"),
    };
    let target = match select.target_list[0].node.as_ref() {
        Some(pg_query::NodeEnum::ResTarget(target)) => target.val.as_deref().map(column),
        other => panic!("expected a projection, got {other:?}"),
    };
    assert_eq!(target.as_deref(), Some("b$$c"));
    let condition = match select.where_clause.as_deref().and_then(|w| w.node.as_ref()) {
        Some(pg_query::NodeEnum::AExpr(expr)) => expr.lexpr.as_deref().map(column),
        other => panic!("expected a comparison, got {other:?}"),
    };
    assert_eq!(condition.as_deref(), Some("a$1"));
    assert_eq!(values.0.len(), 1);
}

// [spec:pgorm:req:pipeline.errors+4/test]    the refusal is pgorm's own and
// happens before `adapter::compile`, so it cannot depend on which prqlc the
// build resolved — and only one can be linked, so the property is asserted
// structurally rather than by compiling twice. This pipeline also mismatches
// its append's column counts, which *every* prqlc rejects as `Compile`;
// getting the identifier error instead is only possible if the screen
// returned before the compiler was ever called.
#[test]
fn identifier_refusal_precedes_the_prqlc_call() {
    refuses(
        Pipeline::from(alias("a"))
            .select(col(alias("a"), Name::runtime(EXFILTRATING)))
            .append(
                Pipeline::from(alias("b"))
                    .select((col(alias("b"), alias("y")), col(alias("b"), ID))),
            ),
    );
}

fn parsed_select(sql: &str) -> pg_query::protobuf::SelectStmt {
    let parsed = pg_query::parse(sql, pg_query::ParserOptions::DEFAULT).expect("grammar accepts");
    let node = parsed.protobuf.stmts[0]
        .stmt
        .as_ref()
        .and_then(|stmt| stmt.node.as_ref())
        .expect("statement present");
    match node {
        pg_query::NodeEnum::SelectStmt(select) => (**select).clone(),
        other => panic!("expected SelectStmt, got {other:?}"),
    }
}

// [spec:pgorm:sem:pipeline.qualify+3/test]
#[test]
fn reserved_word_table_is_quoted() {
    let order = alias("order");
    let built = sql_of(Pipeline::from(order).select((col(order, ID), col(order, TOTAL))));
    assert_eq!(built, r#"SELECT id, total FROM "order""#);
}

// [spec:pgorm:req:pipeline.compose/test]    a pipeline is a from-source; its
// params keep their positions and the consumer's continue after them
#[test]
fn from_pipeline_binds_as_cte() {
    let spent = alias("spent");
    let (sql, values) = Pipeline::from(
        Pipeline::from(INVOICE)
            .group(col(INVOICE, CUSTOMER_ID))
            .aggregate(sum(total()).as_(spent))
            .filter_with(|binder| spent.gt(binder.bind(10_i64))),
    )
    .filter_with(|binder| CUSTOMER_ID.ne(binder.bind(7_i32)))
    .sort(spent.desc())
    .into_sql()
    .expect("pipeline compiles");
    pg_query::parse(&sql, pg_query::ParserOptions::DEFAULT).expect("grammar accepts");
    assert_eq!(
        sql,
        "WITH table_0 AS (SELECT customer_id, COALESCE(SUM(total), 0) AS spent FROM invoice \
         GROUP BY customer_id HAVING COALESCE(SUM(total), 0) > $1) \
         SELECT customer_id, spent FROM table_0 WHERE customer_id <> $2 ORDER BY spent DESC"
    );
    assert_eq!(values.0.len(), 2);
}

// [spec:pgorm:req:pipeline.compose/test]    joining an aggregated pipeline:
// the consumer binds first, the embedded params renumber after it
#[test]
fn join_pipeline_renumbers_embedded_params() {
    let spent = alias("spent");
    let spenders = Pipeline::from(INVOICE)
        .group(col(INVOICE, CUSTOMER_ID))
        .aggregate(sum(total()).as_(spent))
        .filter_with(|binder| spent.gt(binder.bind(100_i64)));
    let (sql, values) = Pipeline::from(CUSTOMER)
        .filter_with(|binder| col(CUSTOMER, alias("active")).eq(binder.bind(true)))
        .join(JoinSide::Inner, spenders, col(CUSTOMER, ID).eq(CUSTOMER_ID))
        .select((col(CUSTOMER, alias("name")), spent))
        .into_sql()
        .expect("pipeline compiles");
    pg_query::parse(&sql, pg_query::ParserOptions::DEFAULT).expect("grammar accepts");
    assert_eq!(
        sql,
        "WITH table_1 AS (SELECT name, id FROM customer WHERE active = $1), \
         table_0 AS (SELECT customer_id, COALESCE(SUM(total), 0) AS spent FROM invoice \
         GROUP BY customer_id HAVING COALESCE(SUM(total), 0) > $2) \
         SELECT table_1.name, table_0.spent FROM table_1 \
         INNER JOIN table_0 ON table_1.id = table_0.customer_id"
    );
    assert_eq!(values.0.len(), 2);
}

// [spec:pgorm:req:pipeline.compose/test]
#[test]
fn append_renders_union_all() {
    let (sql, values) = Pipeline::from(INVOICE)
        .filter_with(|binder| total().gt(binder.bind(1_i64)))
        .append(
            Pipeline::from(alias("archived_invoice"))
                .filter_with(|binder| col(alias("archived_invoice"), TOTAL).gt(binder.bind(2_i64))),
        )
        .into_sql()
        .expect("pipeline compiles");
    pg_query::parse(&sql, pg_query::ParserOptions::DEFAULT).expect("grammar accepts");
    assert_eq!(
        sql,
        "SELECT * FROM invoice WHERE total > $1 \
         UNION ALL SELECT * FROM archived_invoice WHERE total > $2"
    );
    assert_eq!(values.0.len(), 2);
}

// [spec:pgorm:req:pipeline.compose/test]    distinct directly after append
// folds to UNION DISTINCT
#[test]
fn append_then_distinct_renders_union_distinct() {
    let built = sql_of(
        Pipeline::from(INVOICE)
            .select(col(INVOICE, CUSTOMER_ID))
            .append(Pipeline::from(alias("archive")).select(col(alias("archive"), CUSTOMER_ID)))
            .distinct(),
    );
    assert_eq!(
        built,
        "SELECT customer_id FROM invoice UNION DISTINCT SELECT customer_id FROM archive"
    );
}

// [spec:pgorm:req:pipeline.compose/test]
#[test]
fn intersect_renders_intersect_all() {
    let built = sql_of(
        Pipeline::from(INVOICE)
            .select(col(INVOICE, CUSTOMER_ID))
            .intersect(Pipeline::from(alias("refund")).select(col(alias("refund"), CUSTOMER_ID))),
    );
    assert_eq!(
        built,
        "WITH table_0 AS (SELECT customer_id FROM refund) \
         SELECT customer_id FROM invoice INTERSECT ALL SELECT * FROM table_0 AS b"
    );
}

// [spec:pgorm:req:pipeline.compose/test]
#[test]
fn remove_renders_except_all() {
    let built = sql_of(
        Pipeline::from(INVOICE)
            .select(col(INVOICE, CUSTOMER_ID))
            .remove(Pipeline::from(alias("refund")).select(col(alias("refund"), CUSTOMER_ID))),
    );
    assert_eq!(
        built,
        "WITH table_0 AS (SELECT customer_id FROM refund) \
         SELECT customer_id FROM invoice EXCEPT ALL SELECT * FROM table_0 AS b"
    );
}

// [spec:pgorm:req:pipeline.compose/test]
#[test]
fn distinct_alone_renders_select_distinct() {
    let built = sql_of(Pipeline::from(INVOICE).distinct());
    assert_eq!(built, "SELECT DISTINCT * FROM invoice");
}

// [spec:pgorm:req:pipeline.compose/test]    a set operation takes a plain
// table too
#[test]
fn append_accepts_a_table_source() {
    let built = sql_of(Pipeline::from(INVOICE).append(alias("archived_invoice")));
    assert_eq!(
        built,
        "SELECT * FROM invoice UNION ALL SELECT * FROM archived_invoice"
    );
}

// [spec:pgorm:req:pipeline.compose/test]    sort and take inside an embedded
// pipeline stay inside its CTE; PRQL's sticky sort carries outward
#[test]
fn take_and_sort_survive_embedding() {
    let (sql, values) = Pipeline::from(
        Pipeline::from(INVOICE)
            .filter_with(|binder| total().gt(binder.bind(5_i64)))
            .sort(total().desc())
            .take(3),
    )
    .filter(CUSTOMER_ID.gt(1))
    .into_sql()
    .expect("pipeline compiles");
    pg_query::parse(&sql, pg_query::ParserOptions::DEFAULT).expect("grammar accepts");
    assert_eq!(
        sql,
        "WITH table_0 AS (SELECT * FROM invoice WHERE total > $1 ORDER BY total DESC LIMIT 3) \
         SELECT * FROM table_0 WHERE customer_id > 1 ORDER BY total DESC"
    );
    assert_eq!(values.0.len(), 1);
}

// [spec:pgorm:req:pipeline.compose/test]    embedding an embedder: bindings
// renumber past the consumer's and three params stay aligned
#[test]
fn nested_embedding_renumbers_bindings_and_params() {
    let inner = Pipeline::from(INVOICE).filter_with(|binder| total().gt(binder.bind(1_i64)));
    let middle = Pipeline::from(inner).filter_with(|binder| CUSTOMER_ID.gt(binder.bind(2_i64)));
    let (sql, values) = Pipeline::from(CUSTOMER)
        .filter_with(|binder| col(CUSTOMER, ID).gt(binder.bind(3_i64)))
        .join(
            JoinSide::Inner,
            middle,
            col(CUSTOMER, ID).eq(that(CUSTOMER_ID)),
        )
        .into_sql()
        .expect("pipeline compiles");
    pg_query::parse(&sql, pg_query::ParserOptions::DEFAULT).expect("grammar accepts");
    assert_eq!(
        sql,
        "WITH table_2 AS (SELECT * FROM customer WHERE id > $1), \
         table_0 AS (SELECT * FROM invoice WHERE total > $2), \
         table_1 AS (SELECT * FROM table_0 WHERE customer_id > $3) \
         SELECT table_2.*, table_1.* FROM table_2 \
         INNER JOIN table_1 ON table_2.id = table_1.customer_id"
    );
    assert_eq!(values.0.len(), 3);
}

// [spec:pgorm:req:pipeline.compose/test]    the same alias declared in two
// composed pipelines lives in two scopes; neither collides
#[test]
fn duplicate_aliases_across_pipelines_coexist() {
    let n = alias("n");
    let other = Pipeline::from(alias("b")).derive((col(alias("b"), alias("x")) + 1).as_(n));
    let built = sql_of(
        Pipeline::from(alias("a"))
            .derive((col(alias("a"), alias("y")) + 2).as_(n))
            .join(JoinSide::Left, other, col(alias("a"), ID).eq(that(ID))),
    );
    assert_eq!(
        built,
        "WITH table_1 AS (SELECT *, y + 2 AS n FROM a), \
         table_0 AS (SELECT *, x + 1 AS n FROM b) \
         SELECT table_1.*, table_0.* FROM table_1 \
         LEFT OUTER JOIN table_0 ON table_1.id = table_0.id"
    );
}

// [spec:pgorm:req:pipeline.compose/test]    an unqualified name both sides
// export is refused by prqlc, by name
#[test]
fn ambiguous_embedded_column_is_a_compile_error() {
    let x = alias("x");
    let left = Pipeline::from(alias("a")).select(col(alias("a"), x));
    let right = Pipeline::from(alias("b")).select(col(alias("b"), x));
    let err = Pipeline::from(left)
        .join(JoinSide::Inner, right, x.eq(x))
        .into_sql()
        .expect_err("ambiguity must be refused");
    assert!(matches!(err, PipelineError::Compile(ref text) if text.contains("Ambiguous")));
}

// [spec:pgorm:req:pipeline.compose/test]    this() and that() name the two
// sides of the join when neither relation has a writable name
#[test]
fn that_qualifies_the_joined_relation() {
    let x = alias("x");
    let left = Pipeline::from(alias("a")).select(col(alias("a"), x));
    let right = Pipeline::from(alias("b")).select(col(alias("b"), x));
    let built = sql_of(Pipeline::from(left).join(JoinSide::Inner, right, this(x).eq(that(x))));
    assert_eq!(
        built,
        "WITH table_0 AS (SELECT x FROM a), table_1 AS (SELECT x FROM b) \
         SELECT table_0.x, table_1.x FROM table_0 \
         INNER JOIN table_1 ON table_0.x = table_1.x"
    );
}

// [spec:pgorm:req:pipeline.compose/test]    mismatched column counts are
// refused at compile time when prqlc can see both projections
#[test]
fn append_column_count_mismatch_is_refused() {
    let err = Pipeline::from(alias("a"))
        .select(col(alias("a"), alias("x")))
        .append(
            Pipeline::from(alias("b")).select((col(alias("b"), alias("y")), col(alias("b"), ID))),
        )
        .into_sql()
        .expect_err("column count mismatch must be refused");
    assert!(matches!(err, PipelineError::Compile(_)));
}

// [spec:pgorm:req:pipeline.compose/test]    a reserved alias inside an
// embedded pipeline is still screened
#[test]
fn reserved_alias_inside_embedded_pipeline_is_refused() {
    let err = Pipeline::from(Pipeline::from(INVOICE).derive(total().as_("sum")))
        .into_sql()
        .expect_err("reserved alias must be refused");
    assert_eq!(err, PipelineError::ReservedAlias("sum".to_owned()));
}

// [spec:pgorm:def:pipeline.adapter+2/test]    a let-bound composition built
// directly matches the same query compiled from PRQL text
#[test]
fn built_composition_matches_text_compilation() {
    let built = sql_of(
        Pipeline::from(
            Pipeline::from(INVOICE).filter_with(|binder| total().gt(binder.bind(5_i64))),
        )
        .filter(CUSTOMER_ID.gt(1)),
    );
    let text = compile_text(
        "let table_0 = (from invoice | filter invoice.total > $1)\n\
         from table_0 | filter customer_id > 1",
    )
    .expect("compiles");
    assert_eq!(built, text);
}

// [spec:pgorm:req:pipeline.compose/test]    after remove or intersect the
// relation is renamed, so later stages refer to columns by bare name
#[test]
fn stages_after_remove_use_bare_names() {
    let refund = Pipeline::from(alias("refund")).select(col(alias("refund"), CUSTOMER_ID));
    let built = sql_of(
        Pipeline::from(INVOICE)
            .select(col(INVOICE, CUSTOMER_ID))
            .remove(refund.clone())
            .sort(CUSTOMER_ID),
    );
    assert_eq!(
        built,
        "WITH table_0 AS (SELECT customer_id FROM refund), \
         table_1 AS (SELECT customer_id FROM invoice \
         EXCEPT ALL SELECT * FROM table_0 AS b) \
         SELECT customer_id FROM table_1 ORDER BY customer_id"
    );

    let err = Pipeline::from(INVOICE)
        .select(col(INVOICE, CUSTOMER_ID))
        .remove(refund)
        .sort(col(INVOICE, CUSTOMER_ID))
        .into_sql()
        .expect_err("the source qualification is gone after a set op");
    assert!(matches!(err, PipelineError::Compile(_)));
}

const EMPLOYEE: AliasName = alias("employee");
const MANAGER: AliasName = alias("manager");
const NAME: AliasName = alias("name");
const MANAGER_ID: AliasName = alias("manager_id");

// [spec:pgorm:sem:pipeline.self-join+1/test]    the classic employee-manager
// query: one table, two names, both sides selected
#[test]
fn a_named_operand_joins_a_table_to_itself() {
    let built = sql_of(
        Pipeline::from(EMPLOYEE)
            .join(
                JoinSide::Inner,
                EMPLOYEE.named(MANAGER),
                col(EMPLOYEE, MANAGER_ID).eq(col(MANAGER, ID)),
            )
            .select((col(EMPLOYEE, NAME), col(MANAGER, NAME).as_(alias("boss"))))
            .sort(col(EMPLOYEE, NAME)),
    );
    assert_eq!(
        built,
        "SELECT employee.name, manager.name AS boss FROM employee \
         INNER JOIN employee AS manager ON employee.manager_id = manager.id \
         ORDER BY employee.name"
    );
}

// [spec:pgorm:sem:pipeline.self-join+1/test]    the name reaches every stage
// after the join, not only the condition
#[test]
fn a_source_name_qualifies_later_stages() {
    let built = sql_of(
        Pipeline::from(EMPLOYEE)
            .join(
                JoinSide::Left,
                EMPLOYEE.named(MANAGER),
                col(EMPLOYEE, MANAGER_ID).eq(col(MANAGER, ID)),
            )
            .filter(col(MANAGER, NAME).is_not_null())
            .sort(col(MANAGER, NAME).desc()),
    );
    assert_eq!(
        built,
        "SELECT employee.*, manager.* FROM employee \
         LEFT OUTER JOIN employee AS manager ON employee.manager_id = manager.id \
         WHERE manager.name IS NOT NULL ORDER BY manager.name DESC"
    );
}

// [spec:pgorm:sem:pipeline.self-join+1/test]    naming a source replaces the
// name it had, as SQL's AS does
#[test]
fn a_named_source_drops_its_own_name() {
    let err = Pipeline::from(EMPLOYEE.named(alias("e")))
        .select(col(EMPLOYEE, NAME))
        .into_sql()
        .expect_err("the table name is gone once the source is named");
    assert!(matches!(err, PipelineError::Compile(ref text) if text.contains("Unknown name")));
}

// [spec:pgorm:sem:pipeline.self-join+1/test]    an entity is named the same
// way, and its columns then travel through col
#[test]
fn an_entity_operand_takes_a_name() {
    let peer = alias("peer");
    let built = sql_of(
        Pipeline::from(fruit::Entity)
            .join(
                JoinSide::Inner,
                fruit::Entity.named(peer),
                fruit::Column::CakeId.eq(col(peer, alias("cake_id"))),
            )
            .filter(fruit::Column::Id.lt(col(peer, ID)))
            .select((fruit::Column::Name, col(peer, NAME))),
    );
    assert_eq!(
        built,
        "SELECT fruit.name AS _expr_0, peer.name FROM fruit \
         INNER JOIN fruit AS peer ON fruit.cake_id = peer.cake_id \
         WHERE fruit.id < peer.id"
    );
}

// [spec:pgorm:sem:pipeline.self-join+1/test]    a named pipeline is a self-join
// over a derived relation, params and all
#[test]
fn a_named_pipeline_joins_as_an_aliased_cte() {
    let seniors = Pipeline::from(EMPLOYEE)
        .filter_with(|binder| col(EMPLOYEE, alias("level")).gt(binder.bind(3_i32)));
    let (sql, values) = Pipeline::from(EMPLOYEE)
        .join(
            JoinSide::Inner,
            seniors.named(MANAGER),
            col(EMPLOYEE, MANAGER_ID).eq(col(MANAGER, ID)),
        )
        .select((col(EMPLOYEE, NAME), col(MANAGER, NAME).as_(alias("boss"))))
        .into_sql()
        .expect("pipeline compiles");
    pg_query::parse(&sql, pg_query::ParserOptions::DEFAULT).expect("grammar accepts");
    assert_eq!(
        sql,
        "WITH table_0 AS (SELECT * FROM employee WHERE level > $1) \
         SELECT employee.name, manager.name AS boss FROM employee \
         INNER JOIN table_0 AS manager ON employee.manager_id = manager.id"
    );
    assert_eq!(values.0.len(), 1);
}

// [spec:pgorm:sem:pipeline.self-join+1/test]    renaming inside an embedded
// pipeline reaches both sides without naming the operand
#[test]
fn renaming_before_embedding_reaches_both_sides() {
    let manager_pk = alias("manager_pk");
    let boss = alias("boss");
    let managers = Pipeline::from(EMPLOYEE).select((
        col(EMPLOYEE, ID).as_(manager_pk),
        col(EMPLOYEE, NAME).as_(boss),
    ));
    let built = sql_of(
        Pipeline::from(EMPLOYEE)
            .join(
                JoinSide::Inner,
                managers,
                col(EMPLOYEE, MANAGER_ID).eq(manager_pk),
            )
            .select((col(EMPLOYEE, NAME), boss)),
    );
    assert_eq!(
        built,
        "WITH table_0 AS (SELECT id AS manager_pk, name AS boss FROM employee) \
         SELECT employee.name, table_0.boss FROM employee \
         INNER JOIN table_0 ON employee.manager_id = table_0.manager_pk"
    );
}

// [spec:pgorm:sem:pipeline.self-join+1/test]    a chain of names: three
// generations of the same table in one query
#[test]
fn named_operands_chain_across_generations() {
    let grandparent = alias("grandparent");
    let built = sql_of(
        Pipeline::from(EMPLOYEE)
            .join(
                JoinSide::Inner,
                EMPLOYEE.named(MANAGER),
                col(EMPLOYEE, MANAGER_ID).eq(col(MANAGER, ID)),
            )
            .join(
                JoinSide::Inner,
                EMPLOYEE.named(grandparent),
                col(MANAGER, MANAGER_ID).eq(col(grandparent, ID)),
            )
            .select((
                col(EMPLOYEE, NAME),
                col(MANAGER, NAME).as_(alias("boss")),
                col(grandparent, NAME).as_(alias("skip")),
            )),
    );
    assert_eq!(
        built,
        "SELECT employee.name, manager.name AS boss, grandparent.name AS skip \
         FROM employee INNER JOIN employee AS manager ON employee.manager_id = manager.id \
         INNER JOIN employee AS grandparent ON manager.manager_id = grandparent.id"
    );
}

// [spec:pgorm:sem:pipeline.self-join+1/test]    a name is screened like any
// other introduced name
#[test]
fn a_reserved_source_name_is_refused() {
    let err = Pipeline::from(EMPLOYEE)
        .join(
            JoinSide::Inner,
            EMPLOYEE.named(alias("sum")),
            col(EMPLOYEE, MANAGER_ID).eq(col(alias("sum"), ID)),
        )
        .into_sql()
        .expect_err("a reserved name must be refused");
    assert_eq!(err, PipelineError::ReservedAlias("sum".to_owned()));
}

/// Like [`sql_of`], keeping the values: the emitted SQL must pass the
/// grammar oracle after any census rewrite too.
// [spec:pgorm:req:pipeline.params+4/test]
fn sql_and_values_of(pipeline: Pipeline) -> (String, Values) {
    let (sql, values) = pipeline.into_sql().expect("pipeline compiles");
    if let Err(err) = pg_query::parse(&sql, pg_query::ParserOptions::DEFAULT) {
        panic!("PostgreSQL grammar rejected the emitted SQL: {err}\n  {sql}");
    }
    (sql, values)
}

fn ints(values: &Values) -> Vec<Value> {
    values.0.clone()
}

/// Three bound derivations to prune from: `a = $1`, `b = $2`, `c = $3`.
fn three_bound() -> Pipeline {
    Pipeline::from(cake::Entity).derive_with(|binder| {
        [
            binder.bind(1_i32).as_(alias("a")),
            binder.bind(2_i32).as_(alias("b")),
            binder.bind(3_i32).as_(alias("c")),
        ]
    })
}

// [spec:pgorm:req:pipeline.params+4/test]    a bound derivation nothing
// reads is pruned by the optimizer; its value must not survive it
#[test]
fn pruned_binding_drops_its_value() {
    let (sql, values) = sql_and_values_of(
        Pipeline::from(cake::Entity)
            .derive_with(|binder| [binder.bind(42_i32).as_(alias("unused"))])
            .select(cake::Column::Id),
    );
    assert_eq!(sql, "SELECT id FROM cake");
    assert!(values.0.is_empty(), "{values:?}");
}

// [spec:pgorm:req:pipeline.params+4/test]    pruning the first placeholder
// renumbers the survivors down
#[test]
fn pruning_the_first_placeholder_renumbers_survivors() {
    let (sql, values) = sql_and_values_of(three_bound().select((alias("b"), alias("c"))));
    assert_eq!(sql, "SELECT $1 AS b, $2 AS c FROM cake");
    assert_eq!(ints(&values), vec![2_i32.into(), 3_i32.into()]);
}

// [spec:pgorm:req:pipeline.params+4/test]    pruning a middle placeholder
// leaves a gap the census closes
#[test]
fn pruning_a_middle_placeholder_renumbers_survivors() {
    let (sql, values) = sql_and_values_of(three_bound().select((alias("a"), alias("c"))));
    assert_eq!(sql, "SELECT $1 AS a, $2 AS c FROM cake");
    assert_eq!(ints(&values), vec![1_i32.into(), 3_i32.into()]);
}

// [spec:pgorm:req:pipeline.params+4/test]    pruning the last placeholder
// changes no numbering but still drops the value
#[test]
fn pruning_the_last_placeholder_compacts_the_values() {
    let (sql, values) = sql_and_values_of(three_bound().select((alias("a"), alias("b"))));
    assert_eq!(sql, "SELECT $1 AS a, $2 AS b FROM cake");
    assert_eq!(ints(&values), vec![1_i32.into(), 2_i32.into()]);
}

// [spec:pgorm:req:pipeline.params+4/test]    pruning every placeholder
// leaves an unparameterized statement and no values at all
#[test]
fn pruning_every_placeholder_empties_the_values() {
    let (sql, values) = sql_and_values_of(three_bound().select(cake::Column::Id));
    assert_eq!(sql, "SELECT id FROM cake");
    assert!(values.0.is_empty(), "{values:?}");
}

// [spec:pgorm:req:pipeline.params+4/test]    a placeholder written twice
// keeps its value once, both occurrences renumbered alike
#[test]
fn repeated_placeholder_keeps_one_value() {
    let (sql, values) = sql_and_values_of(
        Pipeline::from(cake::Entity)
            .derive_with(|binder| [binder.bind(9_i32).as_(alias("unused"))])
            .filter_with(|binder| {
                let bound = binder.bind(7_i32);
                bound.clone().gt(0).and(bound.lt(100))
            })
            .select(cake::Column::Id),
    );
    assert_eq!(
        sql,
        "WITH table_0 AS (SELECT id FROM cake) \
         SELECT id FROM table_0 WHERE $1 > 0 AND $1 < 100"
    );
    assert_eq!(ints(&values), vec![7_i32.into()]);
}

// [spec:pgorm:req:pipeline.params+4/test]    an embedded pipeline's pruned
// binding sits below the consumer's surviving one, which renumbers down
// past it — the rebase offset and the census compose
#[test]
fn pruned_embedded_binding_renumbers_the_consumer() {
    let inner = Pipeline::from(fruit::Entity)
        .derive_with(|binder| [binder.bind(5_i32).as_(alias("unused"))])
        .select(fruit::Column::CakeId);
    let (sql, values) = sql_and_values_of(
        Pipeline::from(cake::Entity)
            .join(
                JoinSide::Inner,
                inner,
                cake::Column::Id.eq(alias("cake_id")),
            )
            .filter_with(|binder| cake::Column::Id.gt(binder.bind(3_i32)))
            .select(cake::Column::Name),
    );
    assert_eq!(
        sql,
        "WITH table_0 AS (SELECT cake_id FROM fruit) \
         SELECT cake.name FROM cake \
         INNER JOIN table_0 ON cake.id = table_0.cake_id \
         WHERE cake.id > $1"
    );
    assert_eq!(ints(&values), vec![3_i32.into()]);
}

// [spec:pgorm:req:pipeline.params+4/test]    the mirror ordering: the
// consumer's binding survives at $1 and the embedded pipeline's, rebased
// past it, is the one pruned
#[test]
fn pruned_embedded_binding_after_a_surviving_one() {
    let inner = Pipeline::from(fruit::Entity)
        .derive_with(|binder| [binder.bind(5_i32).as_(alias("unused"))])
        .select(fruit::Column::CakeId);
    let (sql, values) = sql_and_values_of(
        Pipeline::from(cake::Entity)
            .filter_with(|binder| cake::Column::Id.gt(binder.bind(3_i32)))
            .join(
                JoinSide::Inner,
                inner,
                cake::Column::Id.eq(alias("cake_id")),
            )
            .select(cake::Column::Name),
    );
    assert_eq!(
        sql,
        "WITH table_1 AS (SELECT name, id FROM cake WHERE id > $1), \
         table_0 AS (SELECT cake_id FROM fruit) \
         SELECT table_1.name FROM table_1 \
         INNER JOIN table_0 ON table_1.id = table_0.cake_id"
    );
    assert_eq!(ints(&values), vec![3_i32.into()]);
}

// [spec:pgorm:req:pipeline.params+4/test]    a pruned placeholder between
// two survivors, one on each side of an embedding
#[test]
fn prune_between_survivors_across_pipelines() {
    let inner = Pipeline::from(fruit::Entity)
        .derive_with(|binder| [binder.bind(5_i32).as_(alias("unused"))])
        .select(fruit::Column::CakeId);
    let (sql, values) = sql_and_values_of(
        Pipeline::from(cake::Entity)
            .filter_with(|binder| cake::Column::Id.gt(binder.bind(3_i32)))
            .join(
                JoinSide::Inner,
                inner,
                cake::Column::Id.eq(alias("cake_id")),
            )
            .filter_with(|binder| cake::Column::Id.lt(binder.bind(100_i32)))
            .select(cake::Column::Name),
    );
    assert_eq!(
        sql,
        "WITH table_1 AS (SELECT name, id FROM cake WHERE id > $1), \
         table_0 AS (SELECT cake_id FROM fruit) \
         SELECT table_1.name FROM table_1 \
         INNER JOIN table_0 ON table_1.id = table_0.cake_id \
         WHERE table_1.id < $2"
    );
    assert_eq!(ints(&values), vec![3_i32.into(), 100_i32.into()]);
}

// [spec:pgorm:req:pipeline.params+4/test]    nesting rides along: the
// innermost pipeline's pruned binding crosses two embeddings before the
// census discards it
#[test]
fn nested_embedding_prunes_through_two_levels() {
    let innermost = Pipeline::from(fruit::Entity)
        .derive_with(|binder| [binder.bind(1_i32).as_(alias("unused"))])
        .select(fruit::Column::CakeId);
    let middle = Pipeline::from(cake::Entity)
        .join(
            JoinSide::Inner,
            innermost,
            cake::Column::Id.eq(alias("cake_id")),
        )
        .filter_with(|binder| cake::Column::Id.gt(binder.bind(2_i32)))
        .select(cake::Column::Id);
    let (sql, values) = sql_and_values_of(
        Pipeline::from(cake::Entity)
            .filter_with(|binder| cake::Column::Id.lt(binder.bind(50_i32)))
            .select(cake::Column::Id)
            .append(middle),
    );
    assert!(sql.contains("UNION ALL"), "{sql}");
    assert!(sql.contains("$1") && sql.contains("$2"), "{sql}");
    assert!(!sql.contains("$3"), "{sql}");
    assert_eq!(ints(&values), vec![50_i32.into(), 2_i32.into()]);
}

/// Like [`sql_of`], for the source-select terminal: golden output plus the
/// grammar oracle.
// [spec:pgorm:sem:pipeline.select-sources+4/test]
fn sources_sql_of<T: SourceList>(selected: SelectedSources<T>) -> String {
    let (sql, _) = selected.into_sql().expect("select_sources compiles");
    if let Err(err) = pg_query::parse(&sql, pg_query::ParserOptions::DEFAULT) {
        panic!("PostgreSQL grammar rejected the emitted SQL: {err}\n  {sql}");
    }
    sql
}

// [spec:pgorm:sem:pipeline.select-sources+4/test]    two sources with a
// colliding column name land under different prefixes by construction, so
// prqlc never mints an _expr_N the decode could not predict
#[test]
fn select_sources_prefixes_dissolve_expr_n() {
    let built = sources_sql_of(
        Pipeline::from(cake::Entity)
            .join(
                JoinSide::Left,
                fruit::Entity,
                cake::Column::Id.eq(fruit::Column::CakeId),
            )
            .select_sources((cake::Entity, fruit::Entity)),
    );
    assert_eq!(
        built,
        "SELECT cake.id AS s0_id, cake.name AS s0_name, fruit.id AS s1_id, \
         fruit.name AS s1_name, fruit.cake_id AS s1_cake_id FROM cake \
         LEFT OUTER JOIN fruit ON cake.id = fruit.cake_id"
    );
    assert!(!built.contains("_expr_"), "{built}");
}

// [spec:pgorm:sem:pipeline.select-sources+4/test]    a single source needs no
// tuple and projects one block under s0_
#[test]
fn select_sources_takes_a_single_source() {
    let built = sources_sql_of(Pipeline::from(cake::Entity).select_sources(cake::Entity));
    assert_eq!(built, "SELECT id AS s0_id, name AS s0_name FROM cake");
}

// [spec:pgorm:sem:pipeline.select-sources+4/test]    a named restatement
// qualifies its block by the name, exactly as the join told the two
// occurrences apart
#[test]
fn select_sources_named_self_join_qualifies_by_name() {
    let peer = alias("peer");
    let built = sources_sql_of(
        Pipeline::from(fruit::Entity)
            .join(
                JoinSide::Inner,
                fruit::Entity.named(peer),
                fruit::Column::CakeId.eq(col(peer, alias("cake_id"))),
            )
            .select_sources((fruit::Entity, fruit::Entity.named(peer))),
    );
    assert_eq!(
        built,
        "SELECT fruit.id AS s0_id, fruit.name AS s0_name, fruit.cake_id AS s0_cake_id, \
         peer.id AS s1_id, peer.name AS s1_name, peer.cake_id AS s1_cake_id FROM fruit \
         INNER JOIN fruit AS peer ON fruit.cake_id = peer.cake_id"
    );
}

// [spec:pgorm:sem:pipeline.select-sources+4/test]    the widest list the
// terminal takes — six sources — projects the i-th listed source's columns
// under s{i}_, qualified by that source's own name, and types one
// Option<Model> per position in listing order
#[test]
fn select_sources_projects_six_sources_in_listing_order() {
    use crate::tests_cfg::{cake_filling, filling, vendor};
    use core::marker::PhantomData;

    /// One `Option<Model>` per listed source, in listing order.
    type SixRow = (
        Option<cake::Model>,
        Option<cake_filling::Model>,
        Option<filling::Model>,
        Option<vendor::Model>,
        Option<fruit::Model>,
        Option<fruit::Model>,
    );

    /// The row type a selection decodes into, read off at compile time.
    fn row_of<T: SourceList>(_: &SelectedSources<T>) -> PhantomData<T::Row> {
        PhantomData
    }

    let rival = alias("rival");
    let selected = Pipeline::from(cake::Entity)
        .join(
            JoinSide::Inner,
            cake_filling::Entity,
            cake::Column::Id.eq(cake_filling::Column::CakeId),
        )
        .join(
            JoinSide::Inner,
            filling::Entity,
            cake_filling::Column::FillingId.eq(filling::Column::Id),
        )
        .join(
            JoinSide::Left,
            vendor::Entity,
            filling::Column::VendorId.eq(vendor::Column::Id),
        )
        .join(
            JoinSide::Left,
            fruit::Entity,
            cake::Column::Id.eq(fruit::Column::CakeId),
        )
        .join(
            JoinSide::Left,
            fruit::Entity.named(rival),
            fruit::Column::Name.eq(col(rival, alias("name"))),
        )
        .select_sources((
            cake::Entity,
            cake_filling::Entity,
            filling::Entity,
            vendor::Entity,
            fruit::Entity,
            fruit::Entity.named(rival),
        ));

    let _: PhantomData<SixRow> = row_of(&selected);

    let built = sources_sql_of(selected);
    let (projection, _) = built.split_once(" FROM ").expect("a FROM clause");
    assert_eq!(
        projection,
        [
            "SELECT cake.id AS s0_id, cake.name AS s0_name",
            "cake_filling.cake_id AS s1_cake_id, cake_filling.filling_id AS s1_filling_id",
            "filling.id AS s2_id, filling.name AS s2_name, filling.vendor_id AS s2_vendor_id",
            "vendor.id AS s3_id, vendor.name AS s3_name",
            "fruit.id AS s4_id, fruit.name AS s4_name, fruit.cake_id AS s4_cake_id",
            "rival.id AS s5_id, rival.name AS s5_name, rival.cake_id AS s5_cake_id",
        ]
        .join(", ")
    );
}

// [spec:pgorm:sem:pipeline.select-sources+4/test]    the writer's cast
// discipline reaches the PRQL side: an enum column reads back as text
// [spec:pgorm:sem:query.graph.writer+4/test]
#[test]
fn select_sources_casts_enum_columns_to_text() {
    let built = sources_sql_of(Pipeline::from(lunch_set::Entity).select_sources(lunch_set::Entity));
    assert_eq!(
        built,
        "SELECT id AS s0_id, name AS s0_name, CAST(tea AS text) AS s0_tea FROM lunch_set"
    );
}

mod read_casts {
    use crate as pgorm;
    use crate::entity::prelude::*;
    use pgorm_query::SimpleExpr;

    /// A type name that needs quoting, through the ordinary cast.
    fn label_type(expr: Expr) -> SimpleExpr {
        expr.cast_as(Name::runtime("Label Type"))
    }

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "ledger")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        #[pgorm(column_type = ColumnType::Text, select_as = "numeric(12, 2)")]
        pub amount: String,
        #[pgorm(column_type = ColumnType::Text, select_with = label_type)]
        pub label: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

// [spec:pgorm:sem:pipeline.select-sources+4/test]    the read cast's type
// is written by pgorm-query's TypeName policy, never handed to prqlc as text:
// a name that needs quoting is quoted, a raw type expression stays verbatim,
// and both spell the cast exactly as the SelectStatement writer does
#[test]
fn select_sources_writes_read_casts_by_type_policy() {
    use crate::{EntityTrait, QueryTrait};

    let built =
        sources_sql_of(Pipeline::from(read_casts::Entity).select_sources(read_casts::Entity));
    assert_eq!(
        built,
        "SELECT id AS s0_id, CAST(amount AS numeric(12, 2)) AS s0_amount, \
         CAST(label AS \"Label Type\") AS s0_label FROM ledger"
    );
    let graph = read_casts::Entity::graph().build().0;
    for cast in [" AS numeric(12, 2))", " AS \"Label Type\")"] {
        assert!(
            built.contains(cast) && graph.contains(cast),
            "{cast}: {graph}"
        );
    }
}

// [spec:pgorm:sem:pipeline.select-sources+4/test]    reshaping before the
// terminal is refused by the stage's own name, before prqlc compiles
#[test]
fn select_sources_refuses_a_reshaped_pipeline() {
    let err = Pipeline::from(cake::Entity)
        .select(cake::Column::Id)
        .select_sources(cake::Entity)
        .into_sql()
        .expect_err("a projection replaced the source namespace");
    assert_eq!(err, PipelineError::ReshapedSources("select"));

    let err = Pipeline::from(cake::Entity)
        .group(cake::Column::Name)
        .aggregate(count_rows().as_(alias("n")))
        .select_sources(cake::Entity)
        .into_sql()
        .expect_err("an aggregation collapsed the source namespace");
    assert_eq!(err, PipelineError::ReshapedSources("group().aggregate()"));

    let err = Pipeline::from(cake::Entity)
        .intersect(fruit::Entity)
        .select_sources(cake::Entity)
        .into_sql()
        .expect_err("a set-op rename dissolved the source namespace");
    assert_eq!(err, PipelineError::ReshapedSources("intersect"));

    let err = Pipeline::from(cake::Entity)
        .remove(fruit::Entity)
        .select_sources(cake::Entity)
        .into_sql()
        .expect_err("a set-op rename dissolved the source namespace");
    assert_eq!(err, PipelineError::ReshapedSources("remove"));
}

// [spec:pgorm:sem:pipeline.select-sources+4/test]    the refusal names the
// stage that did the replacing: the first offender, not the last
#[test]
fn select_sources_refusal_names_the_first_offender() {
    let err = Pipeline::from(cake::Entity)
        .select(cake::Column::Id)
        .intersect(fruit::Entity)
        .select_sources(cake::Entity)
        .into_sql()
        .expect_err("reshaped twice over");
    assert_eq!(err, PipelineError::ReshapedSources("select"));
}

// [spec:pgorm:sem:pipeline.select-sources+4/test]    the whole allowed set
// ahead of the terminal: filter, derive, sort, take, join, window, distinct
// and append leave every source addressable
#[test]
fn select_sources_composes_after_the_allowed_stages() {
    let built = sources_sql_of(
        Pipeline::from(cake::Entity)
            .join(
                JoinSide::Left,
                fruit::Entity,
                cake::Column::Id.eq(fruit::Column::CakeId),
            )
            .filter(cake::Column::Id.gt(0))
            .derive(cake::Column::Id.add(1).as_(alias("next_id")))
            .window(row_number().as_(alias("rn")), by(cake::Column::Id))
            .distinct()
            .sort(cake::Column::Id)
            .take(10)
            .select_sources((cake::Entity, fruit::Entity)),
    );
    assert!(
        built.contains("s0_id") && built.contains("s1_cake_id"),
        "{built}"
    );
}

// [spec:pgorm:sem:pipeline.select-sources+4/test]    append is in the allowed
// set because the left side's naming survives it
#[test]
fn select_sources_composes_after_append() {
    let built = sources_sql_of(
        Pipeline::from(cake::Entity)
            .append(cake::Entity)
            .select_sources(cake::Entity),
    );
    assert!(
        built.contains("UNION ALL") && built.contains("s0_id"),
        "{built}"
    );
}

// [spec:pgorm:sem:pipeline.select-sources+4/test]    an embedded pipeline's
// reshaping stays its own: the CTE boundary re-exposes its projection as a
// table-like namespace, and the consumer's sources are untouched
#[test]
fn select_sources_ignores_an_embedded_reshape() {
    let cake_ids = Pipeline::from(fruit::Entity).select(fruit::Column::CakeId);
    let built = sources_sql_of(
        Pipeline::from(cake::Entity)
            .join(
                JoinSide::Inner,
                cake_ids,
                cake::Column::Id.eq(alias("cake_id")),
            )
            .select_sources(cake::Entity),
    );
    assert_eq!(
        built,
        "WITH table_0 AS (SELECT cake_id FROM fruit) \
         SELECT cake.id AS s0_id, cake.name AS s0_name FROM cake \
         INNER JOIN table_0 ON cake.id = table_0.cake_id"
    );
}

// [spec:pgorm:sem:pipeline.select-sources+4/test]    the catalog-less ceiling:
// a listed source the pipeline never read reaches prqlc, which refuses the
// unresolvable columns as Compile diagnostics
#[test]
fn select_sources_unread_source_fails_in_prqlc() {
    let err = Pipeline::from(cake::Entity)
        .select_sources((cake::Entity, fruit::Entity))
        .into_sql()
        .expect_err("fruit was never read");
    assert!(matches!(err, PipelineError::Compile(_)), "{err:?}");
}

// [spec:pgorm:req:pipeline.compose/test]    a deduplicated relation stays
// combinable: prqlc cannot take a set operation off a grouped relation, so the
// grouped stages are hoisted into their own binding first
#[test]
fn deduplicated_relations_still_combine() {
    let projected = || {
        Pipeline::from_schema(alias("fixture"), alias("accounts"))
            .select(col(alias("accounts"), alias("id")))
    };

    let (sql, _) = projected()
        .distinct()
        .append(projected().distinct())
        .into_sql()
        .expect("matching projections stay composable");
    assert_eq!(
        sql,
        "WITH table_0 AS (SELECT DISTINCT id FROM fixture.accounts) \
SELECT id FROM table_0 UNION ALL SELECT DISTINCT id FROM fixture.accounts"
    );

    // The right side alone needs no hoist, and the left one still gets it.
    let (sql, _) = projected()
        .distinct()
        .append(projected())
        .into_sql()
        .expect("a plain right side combines too");
    assert_eq!(
        sql,
        "WITH table_0 AS (SELECT DISTINCT id FROM fixture.accounts) \
SELECT id FROM table_0 UNION ALL SELECT id FROM fixture.accounts"
    );

    // Undeduplicated set operations are untouched: no binding is minted.
    let (sql, _) = projected()
        .append(projected())
        .into_sql()
        .expect("the baseline append is unchanged");
    assert_eq!(
        sql,
        "SELECT id FROM fixture.accounts UNION ALL SELECT id FROM fixture.accounts"
    );

    // Deduplicating *after* the append is the UNION DISTINCT fold, and keeps it.
    let (sql, _) = projected()
        .append(projected())
        .distinct()
        .into_sql()
        .expect("append then distinct still folds");
    assert_eq!(
        sql,
        "SELECT id FROM fixture.accounts UNION DISTINCT SELECT id FROM fixture.accounts"
    );
}

// [spec:pgorm:req:pipeline.compose/test]    compilation is deterministic: a
// star expanded over a join and then deduplicated used to come out in hash
// order, so the same pipeline emitted two different projections
#[test]
fn a_joined_deduplicated_relation_compiles_once() {
    let compile = || {
        let inner = Pipeline::from_schema(alias("fixture"), alias("notes")).select((
            col(alias("notes"), alias("id")).as_(alias("j_id")),
            col(alias("notes"), alias("account_id")).as_(alias("j_account_id")),
        ));
        Pipeline::from_schema(alias("fixture"), alias("accounts"))
            .select((
                col(alias("accounts"), alias("id")).as_(alias("p_id")),
                col(alias("accounts"), alias("rank")).as_(alias("p_rank")),
            ))
            .join(
                JoinSide::Left,
                named_runtime(inner, pgorm_query::Name::runtime("n")),
                that(alias("j_account_id")).eq(this(alias("p_rank"))),
            )
            .distinct()
            .into_sql()
            .expect("the joined, deduplicated pipeline compiles")
            .0
    };
    let first = compile();
    for _ in 0..64 {
        assert_eq!(compile(), first, "one pipeline, one projection order");
    }
}

const ACCOUNTS: AliasName = alias("accounts");
const FIXTURE: AliasName = alias("fixture");

fn accounts() -> Pipeline {
    Pipeline::from_schema(FIXTURE, ACCOUNTS)
}

// [spec:pgorm:req:pipeline.compose/test]    a renamed column keeps the position
// it was declared in through a deduplication. prqlc files a column still
// qualified by its source under that source's submodule and a renamed one at
// the top level, then orders the two by different keys — the source's position
// against the column's index — so `group this` used to emit every unrenamed
// column first and the renamed one last.
#[test]
fn a_renamed_column_keeps_its_declared_position() {
    let projection = || {
        accounts().select((
            col(ACCOUNTS, ID),
            col(ACCOUNTS, alias("tenant")).as_(alias("p_tenant")),
            col(ACCOUNTS, alias("name")),
        ))
    };
    // Undeduplicated, the declared order was never in question.
    assert_eq!(
        sql_of(projection()),
        "SELECT id, tenant AS p_tenant, name FROM fixture.accounts"
    );
    assert_eq!(
        sql_of(projection().distinct()),
        "WITH table_0 AS (SELECT id, tenant AS p_tenant, name FROM fixture.accounts) \
         SELECT DISTINCT id, p_tenant, name FROM table_0"
    );

    // The shape the campaign reduced: one renamed column among four plain ones.
    assert_eq!(
        sql_of(
            accounts()
                .select((
                    col(ACCOUNTS, ID),
                    col(ACCOUNTS, alias("tenant")).as_(alias("p_tenant")),
                    col(ACCOUNTS, alias("name")),
                    col(ACCOUNTS, alias("score")),
                    col(ACCOUNTS, alias("rank")),
                ))
                .distinct()
        ),
        "WITH table_0 AS (SELECT id, tenant AS p_tenant, name, score, rank FROM fixture.accounts) \
         SELECT DISTINCT id, p_tenant, name, score, rank FROM table_0"
    );
}

// [spec:pgorm:req:pipeline.compose/test]    the same misordering, reached the
// other way: two sources interleaved in one projection, which `this` regroups
// by source however the projection ordered them.
#[test]
fn a_two_source_projection_keeps_its_declared_order() {
    let built = sql_of(
        Pipeline::from(cake::Entity)
            .join(
                JoinSide::Inner,
                fruit::Entity,
                cake::Column::Id.eq(fruit::Column::CakeId),
            )
            .select((cake::Column::Id, fruit::Column::CakeId, cake::Column::Name))
            .distinct(),
    );
    assert_eq!(
        built,
        "WITH table_0 AS (SELECT cake.id, fruit.cake_id, cake.name FROM cake \
         INNER JOIN fruit ON cake.id = fruit.cake_id) \
         SELECT DISTINCT id, cake_id, name FROM table_0"
    );
}

// [spec:pgorm:req:pipeline.compose/test]    a row range in front of a
// deduplication is taken first. prqlc renders the group's own `take 1` and the
// range take as one query, which applied the LIMIT/OFFSET after the
// deduplication and so returned a different set of rows.
#[test]
fn a_row_range_is_taken_before_deduplication() {
    let ranged = || {
        accounts()
            .select((
                col(ACCOUNTS, ID).as_(alias("p_id")),
                col(ACCOUNTS, alias("score")).as_(alias("p_score")),
            ))
            .take_range(3..=5)
    };
    assert_eq!(
        sql_of(ranged()),
        "SELECT id AS p_id, score AS p_score FROM fixture.accounts LIMIT 3 OFFSET 2"
    );
    assert_eq!(
        sql_of(ranged().distinct()),
        "WITH table_0 AS (SELECT id AS p_id, score AS p_score FROM fixture.accounts \
         LIMIT 3 OFFSET 2) SELECT DISTINCT p_id, p_score FROM table_0"
    );
}

// [spec:pgorm:req:pipeline.compose/test]    the whole three-stage shape the
// campaign filed (item runtime-947): deduplicate, sort, take a range, project,
// deduplicate again. The range must be taken from the *sorted* relation, and
// before the second deduplication — the emitted SQL used to carry no ORDER BY
// at all and hung its LIMIT/OFFSET off the outermost SELECT DISTINCT, so it
// returned one arbitrary row where two were due.
#[test]
fn a_deduplicated_sorted_range_keeps_its_rows() {
    let (p_id, p_tenant, p_name, p_score, p_rank, nonce) = (
        alias("p_id"),
        alias("p_tenant"),
        alias("p_name"),
        alias("p_score"),
        alias("p_rank"),
        alias("nonce"),
    );
    let built = sql_of(
        accounts()
            .select((
                col(ACCOUNTS, ID).as_(p_id),
                col(ACCOUNTS, alias("tenant")).as_(p_tenant),
                col(ACCOUNTS, alias("name")).as_(p_name),
                col(ACCOUNTS, alias("score")).as_(p_score),
                col(ACCOUNTS, alias("rank")).as_(p_rank),
            ))
            .derive(Expr::from(947_i64).cast(CastType::BigInt).as_(nonce))
            .distinct()
            .sort((
                p_id.asc(),
                p_tenant.desc(),
                p_name.asc(),
                p_score.asc(),
                p_rank.asc(),
                nonce.asc(),
            ))
            .take_range(3..=5)
            .select((p_score, p_rank, nonce))
            .distinct()
            .select((p_score, p_rank, nonce)),
    );
    assert_eq!(
        built,
        "WITH table_1 AS (SELECT DISTINCT ON (id, tenant, name, score, rank, \
         CAST(947 AS bigint)) score AS p_score, rank AS p_rank, \
         CAST(947 AS bigint) AS nonce, id AS _expr_0, tenant AS _expr_1, name AS _expr_2 \
         FROM fixture.accounts), \
         table_2 AS (SELECT p_score, p_rank, nonce, _expr_0, _expr_1, _expr_2 FROM table_1 \
         ORDER BY _expr_0, _expr_1 DESC, _expr_2, p_score, p_rank, nonce LIMIT 3 OFFSET 2), \
         table_0 AS (SELECT p_score, p_rank, nonce, _expr_0, _expr_1, _expr_2 FROM table_2) \
         SELECT DISTINCT p_score, p_rank, nonce FROM table_0"
    );

    // The two claims the golden is there to hold: the sort reaches the query the
    // range is taken in, and the range is taken before the final deduplication.
    let (ordered, deduplicated) = built
        .split_once("SELECT DISTINCT p_score, p_rank, nonce")
        .expect("the final deduplication is the outermost select");
    assert!(ordered.contains("ORDER BY"), "{built}");
    assert!(ordered.contains("LIMIT 3 OFFSET 2"), "{built}");
    assert!(!deduplicated.contains("LIMIT"), "{built}");
}

// [spec:pgorm:req:pipeline.compose/test]    the shapes a deduplication composes
// with untouched keep their rendering: nothing is settled that did not need it.
#[test]
fn a_composable_deduplication_mints_no_binding() {
    // A bare source: `this` is the source's own wildcard.
    assert_eq!(
        sql_of(Pipeline::from(INVOICE).distinct()),
        "SELECT DISTINCT * FROM invoice"
    );
    // One source, projected: every column answers to the same namespace.
    assert_eq!(
        sql_of(accounts().select(col(ACCOUNTS, ID)).distinct()),
        "SELECT DISTINCT id FROM fixture.accounts"
    );
    // Only introduced names: they are ordered by their column index alone.
    assert_eq!(
        sql_of(
            accounts()
                .select((
                    col(ACCOUNTS, ID).as_(alias("p_id")),
                    col(ACCOUNTS, alias("score")).as_(alias("p_score")),
                ))
                .distinct()
        ),
        "SELECT DISTINCT id AS p_id, score AS p_score FROM fixture.accounts"
    );
    // Derived names trail the source's columns either way.
    assert_eq!(
        sql_of(
            accounts()
                .derive(Expr::from(1_i32).as_(alias("z")))
                .distinct()
        ),
        "SELECT DISTINCT *, 1 AS z FROM fixture.accounts"
    );
    // And the fold that makes a set operation's deduplication a UNION DISTINCT.
    assert_eq!(
        sql_of(
            Pipeline::from(INVOICE)
                .select(col(INVOICE, CUSTOMER_ID))
                .append(Pipeline::from(alias("archive")).select(col(alias("archive"), CUSTOMER_ID)))
                .distinct()
        ),
        "SELECT customer_id FROM invoice UNION DISTINCT SELECT customer_id FROM archive"
    );
}

/// A sort whose keys a later projection drops still has to reach ORDER BY.
///
/// prqlc folds `sort | take` into one `Take` carrying the sort, and `Flattener`
/// then deletes the standalone `Sort` because a later `group` resets the order
/// — leaving `Take.sort` the only carrier of an ordering that still decides
/// which rows the take keeps. The SQL backend's anchoring never looked inside
/// `Take`, so those columns were neither selected nor named, and generating
/// ORDER BY aborted the process instead of erroring. Reduced from campaign item
/// runtime-3287, whose own shape reaches this through `distinct`; an aggregate
/// is used here because `distinct` settles into its own binding first
/// (`a_deduplicated_sorted_range_keeps_its_rows`) and would mask it.
// [spec:pgorm:req:pipeline.compose/test]
#[test]
fn a_dropped_sort_key_still_reaches_order_by() {
    let sql = Pipeline::from_schema(alias("fixture"), alias("accounts"))
        .sort([
            Expr::from(alias("p_id")),
            Expr::from(alias("p_tenant")),
            Expr::from(alias("p_score")),
        ])
        .take_range(2i64..=4i64)
        // p_id and p_score are ordered by, then projected away.
        .select([Expr::from(alias("p_tenant")), Expr::from(alias("nonce"))])
        .group([Expr::from(alias("p_tenant"))])
        .aggregate([count_rows().as_(alias("n"))])
        .into_sql()
        .expect("a sort whose keys are later dropped still compiles")
        .0;

    // Each ordered column is carried by the relation its ORDER BY reads from.
    // That is the whole of the fix: the keys survive the projection that
    // dropped them rather than becoming unnameable at generation time.
    let (carrier, ordering) = sql
        .split_once(" ORDER BY ")
        .expect("the range is selected by an ordering");
    for key in ["p_id", "p_tenant", "p_score"] {
        assert!(ordering.contains(key), "{key} is not ordered by:\n{sql}");
        assert!(carrier.contains(key), "{key} is not projected:\n{sql}");
    }
}

// [spec:pgorm:req:pipeline.compose/test]    a projected expression with no name
// of its own survives the deduplication. prqlc resolves the group's `this` over
// a namespace keyed by name, and an unnamed expression is filed in it nowhere,
// so the column used to be absent from the key — and so from the result, which
// came back two columns wide where three were asked for, with no error.
#[test]
fn an_unnamed_computed_column_survives_deduplication() {
    let projection = || {
        accounts().select((
            col(ACCOUNTS, ID),
            col(ACCOUNTS, alias("score")).mul(2),
            col(ACCOUNTS, alias("name")),
        ))
    };
    // Undeduplicated, the expression was never in question.
    assert_eq!(
        sql_of(projection()),
        "SELECT id, score * 2, name FROM fixture.accounts"
    );
    assert_eq!(
        sql_of(projection().distinct()),
        "WITH table_0 AS (SELECT id, score * 2 AS _col_1, name FROM fixture.accounts) \
         SELECT DISTINCT id, _col_1, name FROM table_0"
    );

    // The fault is naming, not computation: a name of the caller's own already
    // reached the key, and still does, with nothing minted over it.
    assert_eq!(
        sql_of(
            accounts()
                .select((
                    col(ACCOUNTS, ID),
                    col(ACCOUNTS, alias("score")).mul(2).as_(alias("doubled")),
                    col(ACCOUNTS, alias("name")),
                ))
                .distinct()
        ),
        "WITH table_0 AS (SELECT id, score * 2 AS doubled, name FROM fixture.accounts) \
         SELECT DISTINCT id, doubled, name FROM table_0"
    );

    // A minted name steps around a column already spelled that way rather
    // than colliding with it.
    assert_eq!(
        sql_of(
            accounts()
                .select((
                    col(ACCOUNTS, ID),
                    col(ACCOUNTS, alias("score")).mul(2),
                    col(ACCOUNTS, alias("name")).as_(alias("_col_1")),
                ))
                .distinct()
        ),
        "WITH table_0 AS (SELECT id, score * 2 AS _col_2, name AS _col_1 FROM fixture.accounts) \
         SELECT DISTINCT id, _col_2, _col_1 FROM table_0"
    );
}

// [spec:pgorm:req:pipeline.compose/test]    two columns drawn from different
// sources under one bare name both survive. Only one of them can be filed under
// that name in the namespace `this` expands, so the loser used to be dropped
// from the key and from the result — and `id` and `name` collide between almost
// any pair of joined tables.
#[test]
fn a_collided_column_name_survives_deduplication() {
    let joined = || {
        Pipeline::from(cake::Entity).join(
            JoinSide::Inner,
            fruit::Entity,
            cake::Column::Id.eq(fruit::Column::CakeId),
        )
    };
    assert_eq!(
        sql_of(
            joined()
                .select((cake::Column::Id, fruit::Column::Name, cake::Column::Name))
                .distinct()
        ),
        "WITH table_0 AS (SELECT cake.id, fruit.name AS _col_1, cake.name FROM cake \
         INNER JOIN fruit ON cake.id = fruit.cake_id) \
         SELECT DISTINCT id, _col_1, name FROM table_0"
    );

    // The later of the two keeps the bare name, matching both prqlc's own
    // namespace and its rendering, so a stage written against the surviving
    // column still means what it meant. Naming it explicitly is the caller's
    // way of choosing which one that is.
    assert_eq!(
        sql_of(
            joined()
                .select((
                    cake::Column::Id,
                    fruit::Column::Name,
                    Expr::from(cake::Column::Name).as_(alias("cake_name")),
                ))
                .distinct()
        ),
        "WITH table_0 AS (SELECT cake.id, fruit.name, cake.name AS cake_name FROM cake \
         INNER JOIN fruit ON cake.id = fruit.cake_id) \
         SELECT DISTINCT id, name, cake_name FROM table_0"
    );
}

// [spec:pgorm:req:pipeline.compose/test]    a source name written after a
// deduplication still resolves. Settling moves the stages behind a binding,
// which exposes their columns under the binding's name alone, so every
// reference a later stage writes against the source it read — `col(source,
// column)`, an entity column — stopped resolving and prqlc refused the
// pipeline before any SQL was emitted. Reduced from campaign item
// runtime-2387.
#[test]
fn a_settled_source_name_still_resolves() {
    // A table source, settled by the row range standing in front of the
    // deduplication, then filtered by the name it was read under.
    assert_eq!(
        sql_of(
            accounts()
                .select((col(ACCOUNTS, ID), col(ACCOUNTS, alias("name"))))
                .take_range(2i64..=7i64)
                .distinct()
                .filter(col(ACCOUNTS, ID).gt(1))
        ),
        "WITH table_0 AS (SELECT id, name FROM fixture.accounts LIMIT 6 OFFSET 1), \
         table_1 AS (SELECT DISTINCT id, name FROM table_0) \
         SELECT id, name FROM table_1 WHERE id > 1"
    );

    // The campaign's own shape: a pipeline read under a name of its own, and
    // a projection written against that name after the deduplication.
    let nested = alias("nested");
    let renamed = accounts()
        .select((
            col(ACCOUNTS, ID).as_(alias("p_id")),
            col(ACCOUNTS, alias("name")).as_(alias("p_name")),
        ))
        .derive(Expr::from(0_i64).as_(alias("nonce")));
    assert_eq!(
        sql_of(
            Pipeline::from(renamed.named(nested))
                .take_range(2i64..=7i64)
                .distinct()
                .select((col(nested, alias("p_id")), col(nested, alias("nonce"))))
        ),
        "WITH table_0 AS (SELECT id AS p_id, name AS p_name, 0 AS nonce FROM fixture.accounts), \
         table_1 AS (SELECT p_id, p_name, nonce FROM table_0 AS nested LIMIT 6 OFFSET 1) \
         SELECT DISTINCT ON (p_id, p_name, nonce) p_id, nonce FROM table_1"
    );
}

/// A settled pipeline's own binding references follow it into a consumer.
///
/// A repointed reference names the binding in the *qualifying* position of an
/// identifier rather than the bare one a `from` stage uses, and embedding
/// renumbers an embedded pipeline's bindings past the consumer's. Both
/// spellings have to move together, or the reference lands on whichever
/// relation the consumer happened to bind at that index.
// [spec:pgorm:req:pipeline.compose/test]
#[test]
fn a_settled_reference_survives_embedding() {
    let settled = || {
        accounts()
            .select((col(ACCOUNTS, ID), col(ACCOUNTS, alias("name"))))
            .take_range(2i64..=7i64)
            .distinct()
            .select((col(ACCOUNTS, ID), col(ACCOUNTS, alias("name"))))
    };
    // Alone, the reference is the pipeline's own first binding.
    assert_eq!(
        sql_of(settled()),
        "WITH table_0 AS (SELECT id, name FROM fixture.accounts LIMIT 6 OFFSET 1) \
         SELECT DISTINCT id, name FROM table_0"
    );
    // Embedded behind a binding the consumer already had, it is the second —
    // in the `from` stage and in the projection alike.
    let plain = accounts().select((col(ACCOUNTS, ID), col(ACCOUNTS, alias("name"))));
    assert_eq!(
        sql_of(
            Pipeline::from(plain.named(alias("plain")))
                .join(
                    JoinSide::Inner,
                    settled().named(alias("dedup")),
                    col(alias("plain"), ID).eq(col(alias("dedup"), ID)),
                )
                .select((
                    col(alias("plain"), alias("name")),
                    col(alias("dedup"), alias("name")),
                ))
        ),
        "WITH table_0 AS (SELECT id, name FROM fixture.accounts), \
         table_1 AS (SELECT id, name FROM fixture.accounts LIMIT 6 OFFSET 1), \
         table_2 AS (SELECT DISTINCT id, name FROM table_1) \
         SELECT plain.name AS _expr_0, dedup.name FROM table_0 AS plain \
         INNER JOIN table_2 AS dedup ON plain.id = dedup.id"
    );
}

// [spec:pgorm:req:pipeline.compose/test]    a relation joined after a settle
// answers for itself: its own name is in scope again, and the settled side
// stays qualified by its binding, so the two never compete for a bare name.
#[test]
fn a_join_after_a_settle_keeps_both_sides() {
    let built = sql_of(
        Pipeline::from(cake::Entity)
            .select((cake::Column::Id, cake::Column::Name))
            .take_range(2i64..=7i64)
            .distinct()
            .join(
                JoinSide::Inner,
                fruit::Entity,
                cake::Column::Id.eq(fruit::Column::CakeId),
            )
            .select((cake::Column::Name, fruit::Column::Name)),
    );
    assert_eq!(
        built,
        "WITH table_0 AS (SELECT id, name FROM cake LIMIT 6 OFFSET 1), \
         table_1 AS (SELECT DISTINCT ON (id, name) name, id FROM table_0) \
         SELECT table_1.name, fruit.name FROM table_1 \
         INNER JOIN fruit ON table_1.id = fruit.cake_id"
    );

    // The same relation read again after the settle shadowed it: the second
    // `fruit` is the caller's, not the binding's, so the condition's two sides
    // land on different relations.
    let reread = sql_of(
        Pipeline::from(cake::Entity)
            .join(
                JoinSide::Inner,
                fruit::Entity,
                cake::Column::Id.eq(fruit::Column::CakeId),
            )
            .select((cake::Column::Id, cake::Column::Name))
            .take_range(2i64..=7i64)
            .distinct()
            .join(
                JoinSide::Inner,
                fruit::Entity,
                cake::Column::Id.eq(fruit::Column::CakeId),
            )
            .select((cake::Column::Name, fruit::Column::Name)),
    );
    assert!(
        reread.contains("INNER JOIN fruit ON table_1.id = fruit.cake_id"),
        "{reread}"
    );
}

// [spec:pgorm:sem:pipeline.select-sources+4/test]    a settle replaces the
// sources' namespaces exactly as `select` does, so the terminal that projects
// *by* those names is refused by the same gate rather than reaching prqlc with
// qualifiers nothing can resolve. The refusal names the deduplication that
// owed the hoist. A deduplication that needs no hoist leaves every source
// addressable, which `select_sources_composes_after_the_allowed_stages` holds.
#[test]
fn select_sources_refuses_a_settled_pipeline() {
    let err = Pipeline::from(cake::Entity)
        .take(10)
        .distinct()
        .select_sources(cake::Entity)
        .into_sql()
        .expect_err("the row range settled the pipeline behind a binding");
    assert_eq!(err, PipelineError::ReshapedSources("distinct"));

    // A set operation that inherits the hoist settles for the same reason.
    let err = Pipeline::from(cake::Entity)
        .distinct()
        .append(cake::Entity)
        .select_sources(cake::Entity)
        .into_sql()
        .expect_err("the append hoisted the deduplicated stages");
    assert_eq!(err, PipelineError::ReshapedSources("distinct"));
}

// [spec:pgorm:req:pipeline.compose/test]    a sort standing in front of a
// deduplication still orders the result. PRQL's `group` resets the order and
// prqlc's flattener deletes the standalone `sort` a group follows, so the
// ordering was undone rather than applied and the rows came back deduplicated
// but unordered. Restating it behind the group is the `distinct` then `sort`
// shape, which already worked.
#[test]
fn a_sort_before_a_deduplication_still_orders() {
    let projection = || accounts().select((col(ACCOUNTS, ID), col(ACCOUNTS, alias("name"))));
    let ordered = "WITH table_0 AS (SELECT DISTINCT id, name FROM fixture.accounts) \
                   SELECT id, name FROM table_0 ORDER BY id";
    assert_eq!(
        sql_of(projection().sort(col(ACCOUNTS, ID)).distinct()),
        ordered
    );
    // Written the other way round it already worked, and renders identically.
    assert_eq!(
        sql_of(projection().distinct().sort(col(ACCOUNTS, ID))),
        ordered
    );

    // The ordering belongs to a query of its own, never to the deduplicating
    // one — PostgreSQL requires an ORDER BY under DISTINCT to be projected
    // (42P10), and an ordering restated behind a binding never is.
    let (query, _) = ordered
        .rsplit_once(" ORDER BY ")
        .expect("the restated ordering is the outermost clause");
    let last = query
        .rfind("SELECT")
        .expect("the ordering has a query to belong to");
    assert!(
        !query[last..].starts_with("SELECT DISTINCT"),
        "the ordering sits on the deduplicating select: {ordered}"
    );

    // A star relation orders by a column the projection never listed, which a
    // binding still exposes.
    assert_eq!(
        sql_of(Pipeline::from(INVOICE).sort(total()).distinct()),
        "WITH table_0 AS (SELECT DISTINCT * FROM invoice) SELECT * FROM table_0 ORDER BY total"
    );

    // Settled, the key follows its column: past the binding boundary the
    // relation answers to bare names alone, and to the minted one where the
    // projection had to rename.
    assert_eq!(
        sql_of(
            accounts()
                .select((
                    col(ACCOUNTS, ID),
                    col(ACCOUNTS, alias("score")).mul(2),
                    col(ACCOUNTS, alias("name")),
                ))
                .sort(col(ACCOUNTS, ID).desc())
                .distinct()
        ),
        "WITH table_0 AS (SELECT id, score * 2 AS _col_1, name FROM fixture.accounts), \
         table_1 AS (SELECT DISTINCT id, _col_1, name FROM table_0) \
         SELECT id, _col_1, name FROM table_1 ORDER BY id DESC"
    );
}

/// A deduplication whose key is a star hoists as soon as a stage follows it.
///
/// `distinct` is PRQL's `group this (take 1)`, and over a relation whose
/// columns are still a wildcard prqlc expands `this` to a column set that
/// holds the wildcard itself. While nothing is fused into the deduplicating
/// query that set *is* the frame, so prqlc renders plain `SELECT DISTINCT *`;
/// the moment a later stage lands in the same query the two differ and it
/// falls through to `DISTINCT ON (<key>)` — with the wildcard still in the
/// key, spelled `*`, which is not an expression PostgreSQL's grammar has.
/// Reduced from campaign item `distinct-star-projection`.
// [spec:pgorm:req:pipeline.compose/test]
#[test]
fn a_star_deduplication_hoists_before_the_next_stage() {
    let one = alias("one");
    // The filed shape: project after deduplicating a star relation.
    assert_eq!(
        sql_of(Pipeline::from(cake::Entity).distinct().select(NAME)),
        "WITH table_0 AS (SELECT DISTINCT * FROM cake) SELECT name FROM table_0"
    );
    // A derived column is added to the deduplicating query just the same.
    assert_eq!(
        sql_of(
            Pipeline::from(cake::Entity)
                .distinct()
                .derive(Expr::from(1_i32).as_(one))
        ),
        "WITH table_0 AS (SELECT DISTINCT * FROM cake) SELECT *, 1 AS one FROM table_0"
    );
    // A join brings a second relation's columns in, and its condition names a
    // column of this one, which is enough on its own.
    assert_eq!(
        sql_of(
            Pipeline::from(cake::Entity)
                .distinct()
                .join(
                    JoinSide::Inner,
                    fruit::Entity,
                    cake::Column::Id.eq(fruit::Column::CakeId),
                )
                .select((cake::Column::Name, fruit::Column::Name))
        ),
        "WITH table_0 AS (SELECT DISTINCT * FROM cake) \
         SELECT table_0.name AS _expr_0, fruit.name FROM table_0 \
         INNER JOIN fruit ON table_0.id = fruit.cake_id"
    );
    // An aggregate and a window both replace or extend the projection.
    assert_eq!(
        sql_of(
            Pipeline::from(cake::Entity)
                .distinct()
                .group(NAME)
                .aggregate(count_rows().as_(alias("n")))
        ),
        "WITH table_0 AS (SELECT DISTINCT * FROM cake) \
         SELECT name, COUNT(*) AS n FROM table_0 GROUP BY name"
    );
    assert_eq!(
        sql_of(
            Pipeline::from(cake::Entity)
                .distinct()
                .window(row_number().as_(alias("rn")), by(cake::Column::Id))
        ),
        "WITH table_0 AS (SELECT DISTINCT * FROM cake) \
         SELECT *, ROW_NUMBER() OVER (PARTITION BY id) AS rn FROM table_0"
    );
    // A stage that keeps the projection intact still hoists, and renders as
    // it always did — prqlc had to split the query there anyway.
    assert_eq!(
        sql_of(
            Pipeline::from(cake::Entity)
                .distinct()
                .filter(cake::Column::Id.gt(1))
                .select(NAME)
        ),
        "WITH table_0 AS (SELECT DISTINCT * FROM cake) \
         SELECT name FROM table_0 WHERE id > 1"
    );
    // And a star relation reached through a settle of its own is no different.
    assert_eq!(
        sql_of(Pipeline::from(cake::Entity).take(3).distinct().select(NAME)),
        "WITH table_0 AS (SELECT * FROM cake LIMIT 3), \
         table_1 AS (SELECT DISTINCT * FROM table_0) \
         SELECT name FROM table_1"
    );
    // The ordering restated behind the deduplication discharges the hoist
    // itself, so a sort written either side of it reaches the same query —
    // the symmetry `a_sort_before_a_deduplication_still_orders` holds for a
    // projected relation, now over a star one too.
    let ordered = "WITH table_0 AS (SELECT DISTINCT * FROM cake) \
                   SELECT name FROM table_0 ORDER BY name";
    assert_eq!(
        sql_of(
            Pipeline::from(cake::Entity)
                .sort(cake::Column::Name)
                .distinct()
                .select(NAME)
        ),
        ordered
    );
    assert_eq!(
        sql_of(
            Pipeline::from(cake::Entity)
                .distinct()
                .sort(cake::Column::Name)
                .select(NAME)
        ),
        ordered
    );
}

/// The renderings a star deduplication already had, which the hoist must not
/// disturb: prqlc can spell the key in both, so nothing is owed.
// [spec:pgorm:req:pipeline.compose/test]
#[test]
fn a_spellable_star_key_hoists_nothing() {
    // Nothing follows, so the key is the frame and `SELECT DISTINCT *` stands.
    assert_eq!(
        sql_of(Pipeline::from(cake::Entity).distinct()),
        "SELECT DISTINCT * FROM cake"
    );
    assert_eq!(
        sql_of(
            Pipeline::from(cake::Entity)
                .derive(Expr::from(1_i32).as_(alias("one")))
                .distinct()
        ),
        "SELECT DISTINCT *, 1 AS one FROM cake"
    );
    // Deduplicating twice adds nothing to the frame, so prqlc folds the two.
    assert_eq!(
        sql_of(Pipeline::from(cake::Entity).distinct().distinct()),
        "SELECT DISTINCT * FROM cake"
    );
    // Two relations in the deduplicating query: prqlc qualifies every star it
    // writes there (`omit_ident_prefix` is set only for a single table), and a
    // qualified star is a whole-row reference PostgreSQL accepts as a key.
    let joined = sql_of(
        Pipeline::from(cake::Entity)
            .join(
                JoinSide::Inner,
                fruit::Entity,
                cake::Column::Id.eq(fruit::Column::CakeId),
            )
            .distinct()
            .derive(Expr::from(1_i32).as_(alias("one"))),
    );
    assert!(
        joined.contains("cake.*") && joined.contains("fruit.*"),
        "{joined}"
    );
    assert!(!joined.contains("with_clause_placeholder"), "{joined}");
}

/// A star deduplication that owes a hoist refuses the per-source projection,
/// exactly as one that already performed it does
/// (`select_sources_refuses_a_settled_pipeline`): behind the binding the
/// relation answers under that binding alone, which is the one thing a
/// per-source projection cannot do without.
// [spec:pgorm:sem:pipeline.select-sources+4/test]
#[test]
fn select_sources_refuses_a_star_deduplication() {
    let err = Pipeline::from(cake::Entity)
        .distinct()
        .select_sources(cake::Entity)
        .into_sql()
        .expect_err("the deduplication keys on a star it cannot project through");
    assert_eq!(err, PipelineError::ReshapedSources("distinct"));

    // A stage between the two does not make it addressable again.
    let err = Pipeline::from(cake::Entity)
        .distinct()
        .filter(cake::Column::Id.gt(1))
        .select_sources(cake::Entity)
        .into_sql()
        .expect_err("the filter discharged the hoist the deduplication owed");
    assert_eq!(err, PipelineError::ReshapedSources("distinct"));
}

/// One column of the campaign's fixture table, the operand every set-operation
/// test below combines with itself.
fn projected_id() -> Pipeline {
    accounts().select(col(ACCOUNTS, ID))
}

/// How PostgreSQL will associate the emitted query's set operations, written
/// as nested calls over their arms.
///
/// This reads the *parse*, not the text, so precedence has already been
/// applied: a chain the pipeline wrote left to right shows up here as the tree
/// the server evaluates, whatever the rendering made of it. A relation reached
/// through a binding is a leaf, because a CTE is exactly the bracket that ends
/// the chain.
// [spec:pgorm:req:pipeline.compose/test]
fn set_association(sql: &str) -> String {
    fn walk(select: Option<&pg_query::protobuf::SelectStmt>) -> String {
        let Some(select) = select else {
            return "()".to_owned();
        };
        let verb = match select.op() {
            pg_query::protobuf::SetOperation::SetopNone => return "select".to_owned(),
            pg_query::protobuf::SetOperation::SetopUnion => "union",
            pg_query::protobuf::SetOperation::SetopIntersect => "intersect",
            pg_query::protobuf::SetOperation::SetopExcept => "except",
            other => panic!("unexpected set operation {other:?}"),
        };
        let arm = |side: &Option<Box<pg_query::protobuf::SelectStmt>>| {
            walk(side.as_ref().map(std::convert::AsRef::as_ref))
        };
        format!("{verb}({}, {})", arm(&select.larg), arm(&select.rarg))
    }
    walk(Some(&parsed_select(sql)))
}

/// A chain of set operations means what it was written as, left to right.
///
/// SQL binds `INTERSECT` tighter than `UNION` and `EXCEPT`, which share one
/// precedence level and associate left. A flat rendering of
/// `A UNION ALL B INTERSECT ALL C` is therefore the server's
/// `A UNION ALL (B INTERSECT ALL C)` — a different relation, and for a
/// self-combining source every row twice. Hoisting the pending stages into
/// their own binding is the bracket: the looser operation becomes a CTE the
/// tighter one reads as one whole relation.
///
/// Reduced from campaign items runtime-252, runtime-1857, runtime-2187,
/// runtime-3267 and runtime-4317, which all shrank to the same
/// `from t | append t | intersect t` and all returned exactly twice the
/// reference's rows.
// [spec:pgorm:req:pipeline.compose/test]
#[test]
fn a_tighter_set_operation_brackets_the_looser_chain() {
    let t = projected_id;

    // The reduced campaign shape, and its `remove` twin: the tighter operation
    // applies to the whole of the looser one, which a binding holds.
    let appended = sql_of(t().append(t()).intersect(t()));
    assert_eq!(set_association(&appended), "intersect(select, select)");
    assert!(appended.contains("UNION ALL"), "{appended}");

    let removed = sql_of(t().remove(t()).intersect(t()));
    assert_eq!(set_association(&removed), "intersect(select, select)");
    assert!(removed.contains("EXCEPT ALL"), "{removed}");

    // And the same shape as the campaign wrote it, reading the source whole
    // with no projection in front of the operations.
    let starred = sql_of(accounts().append(accounts()).intersect(accounts()));
    assert_eq!(set_association(&starred), "intersect(select, select)");
    assert!(starred.contains("UNION ALL"), "{starred}");

    // A looser operation after a tighter one needs no bracket: SQL's own
    // left-association already reads it the way the pipeline wrote it.
    for (association, built) in [
        ("union(union(select, select), select)", t().append(t())),
        (
            "union(intersect(select, select), select)",
            t().intersect(t()),
        ),
        ("union(except(select, select), select)", t().remove(t())),
    ] {
        assert_eq!(set_association(&sql_of(built.append(t()))), association);
    }
    for (association, built) in [
        ("except(union(select, select), select)", t().append(t())),
        (
            "except(intersect(select, select), select)",
            t().intersect(t()),
        ),
        ("except(except(select, select), select)", t().remove(t())),
    ] {
        assert_eq!(set_association(&sql_of(built.remove(t()))), association);
    }
    assert_eq!(
        set_association(&sql_of(t().intersect(t()).intersect(t()))),
        "intersect(intersect(select, select), select)"
    );

    // Three deep, where the bracket has to survive a further operation: the
    // append on the outside reads the bracketed intersect, not a reassociated
    // chain.
    assert_eq!(
        set_association(&sql_of(t().append(t()).intersect(t()).append(t()))),
        "union(intersect(select, select), select)"
    );
    assert_eq!(
        set_association(&sql_of(t().append(t()).append(t()).intersect(t()))),
        "intersect(select, select)"
    );
    assert_eq!(
        set_association(&sql_of(t().intersect(t()).append(t()).intersect(t()))),
        "intersect(select, select)"
    );

    // A stage between the two operations does not make the bracket optional:
    // prqlc folds a projection the relation already carries straight back into
    // the set operation's own arms, leaving the chain as flat as before.
    assert_eq!(
        set_association(&sql_of(
            t().append(t()).select(col(ACCOUNTS, ID)).intersect(t())
        )),
        "intersect(select, select)"
    );
}

/// Every set operation composes with a deduplicated branch — on the left, on
/// the right, on both, and on neither.
///
/// `distinct` before a set operation and `distinct` after it are different
/// relations and render differently: before it the deduplicated branch is
/// hoisted into its own binding (prqlc cannot take a set operation off a
/// grouped relation), while directly after an `append` it is the
/// `UNION DISTINCT` fold. Both are held here against every verb.
// [spec:pgorm:req:pipeline.compose/test]
#[test]
fn deduplicated_branches_compose_with_every_set_operation() {
    let branch = |deduplicated: bool| match deduplicated {
        true => projected_id().distinct(),
        false => projected_id(),
    };
    let combined = |left: bool, verb: &str, right: bool| {
        let (left, right) = (branch(left), branch(right));
        sql_of(match verb {
            "append" => left.append(right),
            "intersect" => left.intersect(right),
            "remove" => left.remove(right),
            other => panic!("not a set operation: {other}"),
        })
    };

    // Neither branch deduplicated: the plain `ALL` forms.
    assert_eq!(
        combined(false, "append", false),
        "SELECT id FROM fixture.accounts UNION ALL SELECT id FROM fixture.accounts"
    );
    assert_eq!(
        combined(false, "intersect", false),
        "WITH table_0 AS (SELECT id FROM fixture.accounts) \
         SELECT id FROM fixture.accounts INTERSECT ALL SELECT * FROM table_0 AS b"
    );
    assert_eq!(
        combined(false, "remove", false),
        "WITH table_0 AS (SELECT id FROM fixture.accounts) \
         SELECT id FROM fixture.accounts EXCEPT ALL SELECT * FROM table_0 AS b"
    );

    // The left branch alone: its deduplicating group is hoisted into a binding
    // so the set operation reads a settled arity.
    assert_eq!(
        combined(true, "append", false),
        "WITH table_0 AS (SELECT DISTINCT id FROM fixture.accounts) \
         SELECT id FROM table_0 UNION ALL SELECT id FROM fixture.accounts"
    );
    assert_eq!(
        combined(true, "intersect", false),
        "WITH table_0 AS (SELECT DISTINCT id FROM fixture.accounts), \
         table_1 AS (SELECT id FROM fixture.accounts) \
         SELECT id FROM table_0 AS t INTERSECT ALL SELECT * FROM table_1 AS b"
    );
    assert_eq!(
        combined(true, "remove", false),
        "WITH table_0 AS (SELECT DISTINCT id FROM fixture.accounts), \
         table_1 AS (SELECT id FROM fixture.accounts) \
         SELECT id FROM table_0 AS t EXCEPT ALL SELECT * FROM table_1 AS b"
    );

    // The right branch alone: an embedded pipeline is a binding already, and
    // deduplicating inside it needs no hoist of its own.
    assert_eq!(
        combined(false, "append", true),
        "SELECT id FROM fixture.accounts UNION ALL SELECT DISTINCT id FROM fixture.accounts"
    );
    assert_eq!(
        combined(false, "intersect", true),
        "WITH table_0 AS (SELECT DISTINCT id FROM fixture.accounts) \
         SELECT id FROM fixture.accounts INTERSECT ALL SELECT * FROM table_0 AS b"
    );
    assert_eq!(
        combined(false, "remove", true),
        "WITH table_0 AS (SELECT DISTINCT id FROM fixture.accounts) \
         SELECT id FROM fixture.accounts EXCEPT ALL SELECT * FROM table_0 AS b"
    );

    // Both branches: the two deduplications are independent, and neither is
    // the deduplication of the combined relation.
    assert_eq!(
        combined(true, "append", true),
        "WITH table_0 AS (SELECT DISTINCT id FROM fixture.accounts) \
         SELECT id FROM table_0 UNION ALL SELECT DISTINCT id FROM fixture.accounts"
    );
    assert_eq!(
        combined(true, "intersect", true),
        "WITH table_0 AS (SELECT DISTINCT id FROM fixture.accounts), \
         table_1 AS (SELECT DISTINCT id FROM fixture.accounts) \
         SELECT id FROM table_0 AS t INTERSECT ALL SELECT * FROM table_1 AS b"
    );
    assert_eq!(
        combined(true, "remove", true),
        "WITH table_0 AS (SELECT DISTINCT id FROM fixture.accounts), \
         table_1 AS (SELECT DISTINCT id FROM fixture.accounts) \
         SELECT id FROM table_0 AS t EXCEPT ALL SELECT * FROM table_1 AS b"
    );

    // And deduplicating the *combined* relation, which is the fold after
    // `append` and a plain `SELECT DISTINCT` over a binding after the others.
    assert_eq!(
        sql_of(projected_id().append(projected_id()).distinct()),
        "SELECT id FROM fixture.accounts UNION DISTINCT SELECT id FROM fixture.accounts"
    );
    assert_eq!(
        sql_of(projected_id().intersect(projected_id()).distinct()),
        "WITH table_0 AS (SELECT id FROM fixture.accounts), \
         table_1 AS (SELECT id FROM fixture.accounts INTERSECT ALL SELECT * FROM table_0 AS b) \
         SELECT DISTINCT id FROM table_1"
    );
    assert_eq!(
        sql_of(projected_id().remove(projected_id()).distinct()),
        "WITH table_0 AS (SELECT id FROM fixture.accounts), \
         table_1 AS (SELECT id FROM fixture.accounts EXCEPT ALL SELECT * FROM table_0 AS b) \
         SELECT DISTINCT id FROM table_1"
    );
}

/// Two tables whose `mood` columns are typed with `cheerful`, a domain over
/// the enum `mood`.
mod cheerful {
    macro_rules! entity {
        ($module:ident, $table:literal, $key:ident, $other:ident) => {
            pub mod $module {
                use crate as pgorm;
                use crate::entity::prelude::*;

                fn cheerful() -> ColumnType {
                    ColumnType::Domain {
                        name: Name::runtime("cheerful"),
                        schema: None,
                        base: Arc::new(ColumnType::Enum {
                            name: Name::runtime("mood"),
                            schema: None,
                            variants: vec![Name::runtime("calm"), Name::runtime("glad")],
                        }),
                    }
                }

                #[derive(Clone, Debug, PartialEq, Eq, Entity)]
                #[pgorm(table_name = $table)]
                pub struct Model {
                    #[pgorm(primary_key, auto_increment = false)]
                    pub $key: i32,
                    #[pgorm(column_type = cheerful())]
                    pub mood: String,
                    #[pgorm(column_type = ColumnType::Text)]
                    pub $other: String,
                }

                #[derive(Copy, Clone, Debug, Relation)]
                pub enum Relation {}
            }
        };
    }

    entity!(line, "line", id, note);
    entity!(temper, "temper", rank, label);
}

// [spec:pgorm:sem:entity.traits.column.enum-cast+16/test]    a pipeline
// comparison casts each side that is an entity column whose domain is over an
// enum to the enum — against a literal, a bound value, a list or another such
// column — and so do coalesce, min and max, while sorting, grouping, count and
// a plain column keep the bare column
#[test]
fn domain_enum_columns_compare_as_the_enum() {
    use cheerful::{line, temper};

    assert_eq!(
        sql_of(
            Pipeline::from(line::Entity)
                .filter(line::Column::Mood.eq("glad").and(line::Column::Id.gt(1)))
                .sort(line::Column::Mood)
                .select((line::Column::Id, line::Column::Mood))
        ),
        "SELECT id, mood FROM line WHERE CAST(mood AS mood) = CAST('glad' AS mood) AND id > 1 \
         ORDER BY mood"
    );
    let (bound, values) = Pipeline::from(line::Entity)
        .filter_with(|binder| line::Column::Mood.ne(binder.bind("calm")))
        .select(line::Column::Id)
        .into_sql()
        .expect("pipeline compiles");
    assert_eq!(
        bound,
        "SELECT id FROM line WHERE CAST(mood AS mood) <> CAST($1::text AS mood)"
    );
    assert_eq!(values.0, [Value::from("calm")]);
    assert_eq!(
        sql_of(
            Pipeline::from(line::Entity)
                .filter(line::Column::Mood.in_array(["calm", "glad"]))
                .filter(line::Column::Mood.coalesce("calm").lte("glad"))
                .select(line::Column::Id)
        ),
        "SELECT id FROM line WHERE CAST(mood AS mood) IN (CAST('calm' AS mood), \
         CAST('glad' AS mood)) AND COALESCE(CAST(mood AS mood), CAST('calm' AS mood)) <= 'glad'"
    );
    assert_eq!(
        sql_of(
            Pipeline::from(line::Entity)
                .join(
                    JoinSide::Inner,
                    temper::Entity,
                    line::Column::Mood.eq(temper::Column::Mood),
                )
                .select((line::Column::Id, temper::Column::Label))
        ),
        "SELECT line.id, temper.label FROM line \
         INNER JOIN temper ON CAST(line.mood AS mood) = CAST(temper.mood AS mood)"
    );
    assert_eq!(
        sql_of(
            Pipeline::from(line::Entity)
                .group(line::Column::Mood)
                .aggregate((
                    max(line::Column::Mood).as_("highest"),
                    min(line::Column::Mood).as_("lowest"),
                    count(line::Column::Mood).as_("n"),
                ))
        ),
        "SELECT mood, MAX(CAST(mood AS mood)) AS highest, MIN(CAST(mood AS mood)) AS lowest, \
         COUNT(mood) AS n FROM line GROUP BY mood"
    );
}

// [spec:pgorm:sem:entity.traits.column.enum-cast+16/test]    a named relation's
// column compares as its entity's column through Named::col, and a col
// reference or an alias token as the enum compares_as names, schema-qualified
// and quoted by the type-name policy, while projecting and grouping keep each
// bare and a col reference left unnamed compares as it is
// [spec:pgorm:req:pipeline.surface+4/test]    compares_as casts the column and
// each literal or bound value compared with it to the type it names, `money`
// included, a bound value's placeholder pinned to its own type
#[test]
fn untyped_columns_compare_as_the_named_type() {
    use cheerful::line;
    use pgorm_query::TypeName;

    let peer = line::Entity.named(alias("peer"));
    assert_eq!(
        sql_of(
            Pipeline::from(line::Entity)
                .join(
                    JoinSide::Inner,
                    peer.clone(),
                    line::Column::Mood.eq(peer.col(line::Column::Mood)),
                )
                .select((line::Column::Id, peer.col(line::Column::Mood)))
        ),
        "SELECT line.id, peer.mood FROM line \
         INNER JOIN line AS peer ON CAST(line.mood AS mood) = CAST(peer.mood AS mood)"
    );

    let qualified = || TypeName::new(alias("Mood")).schema(alias("app"));
    let (bound, values) = Pipeline::from(alias("line"))
        .filter_with(|binder| {
            col(alias("line"), alias("mood"))
                .compares_as(qualified())
                .in_array([binder.bind("glad"), "calm".into()])
        })
        .into_sql()
        .expect("pipeline compiles");
    assert_eq!(
        bound,
        r#"SELECT * FROM line WHERE CAST(mood AS app."Mood") IN (CAST($1::text AS app."Mood"), CAST('calm' AS app."Mood"))"#
    );
    assert_eq!(values.0, [Value::from("glad")]);

    let (bound, values) = Pipeline::from(alias("till"))
        .filter_with(|binder| {
            let takings = col(alias("till"), alias("takings")).compares_as(alias("money"));
            takings
                .clone()
                .gt(binder.bind(rust_decimal::Decimal::new(5, 0)))
                .and(takings.lte(12.5))
        })
        .into_sql()
        .expect("pipeline compiles");
    assert_eq!(
        bound,
        "SELECT * FROM till WHERE CAST(takings AS money) > CAST($1::numeric AS money) \
         AND CAST(takings AS money) <= CAST(12.5 AS money)"
    );
    assert_eq!(values.0.len(), 1);

    let m = alias("m");
    assert_eq!(
        sql_of(
            Pipeline::from(alias("line"))
                .derive(col(alias("line"), alias("mood")).as_(m))
                .filter(m.compares_as(alias("mood")).gte("calm"))
                .select(m)
        ),
        "WITH table_0 AS (SELECT mood AS m FROM line) \
         SELECT m FROM table_0 WHERE CAST(m AS mood) >= CAST('calm' AS mood)"
    );
    assert_eq!(
        sql_of(
            Pipeline::from(alias("line"))
                .group(col(alias("line"), alias("note")))
                .aggregate(
                    max(col(alias("line"), alias("mood")).compares_as(alias("mood")))
                        .as_("highest")
                )
        ),
        "SELECT note, MAX(CAST(mood AS mood)) AS highest FROM line GROUP BY note"
    );
    assert_eq!(
        sql_of(Pipeline::from(alias("line")).filter(col(alias("line"), alias("mood")).eq("glad"))),
        "SELECT * FROM line WHERE mood = 'glad'"
    );
}

// [spec:pgorm:def:sql.value.created-range+7/test]    a bound built-in range
// or multirange compared under a column's cast goes through its text: pinned
// to its own type, then cast to text and to the column's, by a comparison on
// either side and by in_array, while one compared bare stays a placeholder
#[test]
fn bound_ranges_compare_through_their_text() {
    use pgorm_query::{Multirange, Range};

    let (sql, values) = Pipeline::from(alias("booking"))
        .filter_with(|binder| {
            let slot = || col(alias("booking"), alias("slot")).compares_as(alias("slot"));
            let range = binder.bind(Range::from(1..5));
            slot()
                .eq(range.clone())
                .or(binder.bind(Range::from(7..9)).lt(slot()))
                .or(slot().in_array([binder.bind(Range::from(2..3))]))
                .or(col(alias("booking"), alias("seats")).eq(range))
                .or(col(alias("booking"), alias("slots"))
                    .compares_as(alias("slots"))
                    .eq(binder.bind(Multirange::from_iter([Range::from(1..2)]))))
        })
        .into_sql()
        .expect("pipeline compiles");
    assert_eq!(
        sql,
        "SELECT * FROM booking WHERE CAST(slot AS slot) = CAST($1::int4range::text AS slot) \
         OR CAST($2::int4range::text AS slot) < CAST(slot AS slot) \
         OR CAST(slot AS slot) IN (CAST($3::int4range::text AS slot)) OR seats = $1 \
         OR CAST(slots AS slots) = CAST($4::int4multirange::text AS slots)"
    );
    assert_eq!(values.0.len(), 4);
}

/// A till whose `takings` are `money`, with an array of them, a `state` of
/// the enum `till_state`, and a `code` written through a `save_as` cast.
mod till {
    use crate as pgorm;
    use crate::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(table_name = "till")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub id: i32,
        #[pgorm(column_type = ColumnType::Money)]
        pub takings: Decimal,
        #[pgorm(column_type = ColumnType::Array(Arc::new(ColumnType::Money)))]
        pub floats: Vec<Decimal>,
        #[pgorm(column_type = ColumnType::Enum { name: Name::runtime("till_state"), schema: None, variants: vec![Name::runtime("open"), Name::runtime("shut")] })]
        pub state: String,
        #[pgorm(save_as = "citext")]
        pub code: String,
        pub note: String,
    }

    #[derive(Copy, Clone, Debug, Relation)]
    pub enum Relation {}
}

// [spec:pgorm:sem:entity.traits.column.enum-cast+16/test]    a literal or a
// bound value compared with an entity column, by a comparison, coalesce or
// in_array, either side, is cast as the column's ColumnTrait casts a compared
// value — money, an array of money, an enum, a save_as type — its placeholder
// pinned to its own type, while arithmetic, a column on the other side, null
// and a column whose values travel bare are left as written
#[test]
fn compared_values_cast_as_their_column_writes_them() {
    use rust_decimal::Decimal;
    use till::Column as T;

    let (sql, values) = Pipeline::from(till::Entity)
        .filter_with(|binder| {
            T::Takings
                .gte(binder.bind(Decimal::new(150, 2)))
                .and(binder.bind(Decimal::ONE_HUNDRED).gt(T::Takings))
                .and(T::Floats.eq(binder.bind(vec![Decimal::ONE])))
                .and(T::State.ne(binder.bind("shut")))
                .and(T::Code.eq(binder.bind("ab")))
                .and(T::Note.eq(binder.bind("plain")))
        })
        .into_sql()
        .expect("pipeline compiles");
    assert_eq!(
        sql,
        "SELECT * FROM till WHERE takings >= CAST($1::numeric AS money) \
         AND CAST($2::numeric AS money) > takings \
         AND floats = CAST($3::numeric[] AS money[]) \
         AND state <> CAST($4::text AS till_state) \
         AND code = CAST($5::text AS citext) AND note = $6"
    );
    assert_eq!(values.0.len(), 6);

    assert_eq!(
        sql_of(
            Pipeline::from(till::Entity)
                .filter(T::Takings.in_array([5, 10]))
                .filter(T::Takings.coalesce(0).lt(2.5))
                .filter(T::State.eq("open").and(T::Takings.is_not_null()))
                .derive(ExprOps::mul(T::Takings, 2).as_("doubled"))
                .select((T::Id, alias("doubled")))
        ),
        "SELECT id, takings * 2 AS doubled FROM till WHERE takings IN (CAST(5 AS money), \
         CAST(10 AS money)) AND COALESCE(takings, CAST(0 AS money)) < 2.5 \
         AND state = CAST('open' AS till_state) AND takings IS NOT NULL"
    );
}
