use pgorm_sql_macro::sql;

// The parser names no token when it runs out of input, so the caret sits
// where the text ended.
const QUERY: &str = sql!("SELECT id FROM");

fn main() {
    let _ = QUERY;
}
