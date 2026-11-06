//! What `prql!` leaves behind: the compiled SQL and the arguments as Values.

use pgorm::{Value, Values};
use pgorm_sql_macro::prql;

// [spec:pgorm:def:macros.prql+2/test]    the expansion is the emitted SQL plus Values
#[test]
fn expansion_is_sql_and_values() {
    let (sql, values) = prql!("from invoice | filter total > $1 | take 5", 100_i64);

    assert_eq!(sql, "SELECT * FROM invoice WHERE total > $1 LIMIT 5");
    assert_eq!(values, Values(vec![Value::from(100_i64)]));
}

// [spec:pgorm:def:macros.prql+2/test]    a `&'static str`, so it outlives any frame
#[test]
fn emitted_sql_is_a_static_str() {
    fn keep(query: &'static str) -> &'static str {
        query
    }

    let (sql, _) = prql!("from invoice | take 3");
    assert_eq!(keep(sql), "SELECT * FROM invoice LIMIT 3");
}

// [spec:pgorm:sem:macros.prql.census/test]    `$1` twice is one argument, bound once
#[test]
fn reused_placeholder_binds_one_argument() {
    let (sql, values) = prql!(
        "from invoice | filter total > $1 | filter subtotal < $1",
        7_i64,
    );

    assert_eq!(
        sql,
        "SELECT * FROM invoice WHERE total > $1 AND subtotal < $1"
    );
    assert_eq!(sql.matches("$1").count(), 2);
    assert_eq!(values, Values(vec![Value::from(7_i64)]));
}

// [spec:pgorm:sem:macros.prql.census/test]    no placeholders, no arguments, empty Values
#[test]
fn placeholder_free_query_takes_no_arguments() {
    let (sql, values) = prql!("from invoice | sort {-total} | take 1");

    assert_eq!(sql, "SELECT * FROM invoice ORDER BY total DESC LIMIT 1");
    assert_eq!(values, Values(Vec::new()));
}

// [spec:pgorm:def:macros.prql+2/test]    arguments convert in placeholder order
#[test]
fn arguments_convert_in_placeholder_order() {
    let city = "Berlin".to_owned();
    let (sql, values) = prql!(
        "from invoice | filter total > $1 | filter billing_city == $2",
        250_i64,
        city,
    );

    assert_eq!(
        sql,
        "SELECT * FROM invoice WHERE total > $1 AND billing_city = $2"
    );
    assert_eq!(
        values,
        Values(vec![Value::from(250_i64), Value::from("Berlin")])
    );
}

// [spec:pgorm:def:macros.prql+2/test]    prqlc places the clause: a filter after
// an aggregation compiles to HAVING
#[test]
fn filter_after_aggregate_becomes_having() {
    let (sql, values) = prql!(
        "from invoice
         group {customer_id} (aggregate {spent = sum total})
         filter spent > $1",
        500_i64,
    );

    assert_eq!(
        sql,
        "SELECT customer_id, COALESCE(SUM(total), 0) AS spent FROM invoice \
         GROUP BY customer_id HAVING COALESCE(SUM(total), 0) > $1"
    );
    assert_eq!(values.0.len(), 1);
}

// [spec:pgorm:def:macros.prql+2/test]    a join with a bound filter survives whole
#[test]
fn join_compiles_with_bound_filter() {
    let (sql, values) = prql!(
        "from invoice
         join customer (invoice.customer_id == customer.customer_id)
         filter customer.country == $1
         select {invoice.total, customer.last_name}",
        "Germany",
    );

    assert_eq!(
        sql,
        "SELECT invoice.total, customer.last_name FROM invoice INNER JOIN customer \
         ON invoice.customer_id = customer.customer_id WHERE customer.country = $1"
    );
    assert_eq!(values, Values(vec![Value::from("Germany")]));
}

// [spec:pgorm:sem:macros.prql.sstring/test]    an s-string passes through to the
// SQL as written, and the finished statement still cleared the oracle
#[test]
fn sstring_passes_through_to_emitted_sql() {
    let (sql, values) = prql!(
        r#"from invoice | derive city = s"lower(billing_city)" | filter total > $1"#,
        10_i64,
    );

    assert_eq!(
        sql,
        "WITH table_0 AS (SELECT *, lower(billing_city) AS city FROM invoice) \
         SELECT * FROM table_0 WHERE total > $1"
    );
    assert_eq!(values.0.len(), 1);
}

// [spec:pgorm:sem:macros.prql.census/test]    a raw `$N` inside an s-string is
// invisible to PRQL but not to the census: it still demands an argument
#[test]
fn sstring_placeholder_counts_toward_arity() {
    let (sql, values) = prql!(r#"from invoice | filter total > s"$1 + 0""#, 5_i64);

    assert_eq!(sql, "SELECT * FROM invoice WHERE total > $1 + 0");
    assert_eq!(values, Values(vec![Value::from(5_i64)]));
}

// [spec:pgorm:req:macros.prql.ceiling/test]    no catalog: unknown names compile
#[test]
fn unknown_tables_and_columns_pass() {
    let (sql, values) = prql!("from no_such_table | filter nowhere > $1", 1_i64);

    assert_eq!(sql, "SELECT * FROM no_such_table WHERE nowhere > $1");
    assert_eq!(values.0.len(), 1);
}
