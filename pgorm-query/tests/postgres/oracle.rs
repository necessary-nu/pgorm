//! Render-conformance oracle: rendered SQL is fed to the real PostgreSQL grammar.
//!
//! pgsql-syntax is a Rust port of PostgreSQL 19.0's own scanner and parser,
//! built from the release's Bison tables, the grammar every pgorm build parses
//! with, so a string it accepts is a string the server's parser accepts.
//!
//! The oracle is syntax-only: it has no catalog, so unknown tables, unknown
//! columns and type-modifier misuse pass. The live-Postgres integration suite in
//! the root crate remains the semantic oracle.

/// Statement keywords the [`assert_eq`] shim uses to tell a whole rendered
/// statement from a rendered fragment (a bare column reference, a value literal,
/// an expression) that no grammar could parse on its own. Every
/// [`QUERY_KEYWORDS`] entry is here too: a query that opens a render bare is as
/// much a statement as one wrapped in parentheses, and a keyword missing here
/// lets such a render pass the shim unparsed.
const STATEMENT_KEYWORDS: [&str; 16] = [
    "SELECT ",
    "VALUES ",
    "TABLE ",
    "INSERT ",
    "UPDATE ",
    "DELETE ",
    "MERGE ",
    "WITH ",
    "CREATE ",
    "ALTER ",
    "DROP ",
    "TRUNCATE ",
    "COMMENT ",
    "REPLACE ",
    "REFRESH ",
    "COPY ",
];

/// The keywords a parenthesised query opens with once its parentheses are
/// stripped. The grammar lets a statement begin with `(` only as
/// `select_with_parens`, whose body is a `SELECT`, `VALUES`, `TABLE` or
/// `WITH` query; no other statement can be wrapped.
const QUERY_KEYWORDS: [&str; 4] = ["SELECT ", "VALUES ", "TABLE ", "WITH "];

/// Whether `sql` opens with a statement keyword, or with parentheses around
/// a query (a grouped set operation's leading arm), and so is a candidate for
/// the oracle rather than a rendered fragment. A parenthesised fragment that
/// is not a query — a collation, a tuple, a parenthesised operand — has no
/// query keyword inside its opening parentheses and stays skipped.
pub fn looks_like_statement(sql: &str) -> bool {
    let sql = sql.trim_start();
    let query = sql.trim_start_matches(|c: char| c == '(' || c.is_whitespace());
    if query.len() < sql.len() {
        opens_with_any(query, &QUERY_KEYWORDS)
    } else {
        opens_with_any(sql, &STATEMENT_KEYWORDS)
    }
}

fn opens_with_any(sql: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|kw| {
        sql.get(..kw.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(kw))
    })
}

/// Run `sql` through the PostgreSQL grammar, returning the parser's diagnostic on
/// rejection.
pub fn parses(sql: &str) -> Result<(), String> {
    pgsql_syntax::detailed::parse(sql, pgsql_syntax::ParserOptions::DEFAULT)
        .map(|_| ())
        .map_err(|error| diagnostic(sql, &error))
}

/// [`parses`], as an assertion.
///
/// # Panics
/// Panics when the PostgreSQL grammar rejects `sql`.
// [spec:pgorm:req:sql.render.oracle+6]
pub fn assert_parses(sql: &str) {
    if let Err(report) = parses(sql) {
        panic!("{report}");
    }
}

/// Assert that the PostgreSQL grammar *rejects* `sql`, returning the parser's
/// message. Used by the pins in `oracle_pins.rs` to hold a known-invalid render
/// in place until the plan node that fixes it lands.
///
/// # Panics
/// Panics when the grammar accepts `sql`.
pub fn assert_rejected(sql: &str) -> String {
    match pgsql_syntax::parse(sql, pgsql_syntax::ParserOptions::DEFAULT) {
        Ok(_) => panic!(
            "oracle pin is stale: PostgreSQL now accepts this render, so the pin \
             should become an ordinary conformance assertion\n  {sql}"
        ),
        Err(err) => err.to_string(),
    }
}

/// Compare a rendered statement against its expected text *and* hold it to the
/// PostgreSQL grammar.
///
/// # Panics
/// Panics when the strings differ or the grammar rejects `built`.
pub fn assert_query_eq(built: &str, expected: &str) {
    pretty_assertions::assert_eq!(built, expected);
    assert_parses(built);
}

/// Every node of kind `kind` (`"CaseExpr"`, `"AIndirection"`, `"GroupingSet"`)
/// in `sql`'s parse tree, as JSON, each before the nodes nested inside it.
///
/// Parsing settles that a render is grammatical; this settles that it is the
/// construct it was written as. A simple `CASE` whose operand was dropped, a
/// subscript that became a function call, or a grouping set that flattened
/// into a plain `GROUP BY` list all still parse.
///
/// The tree is read with the protobuf's `{"node": …}` wrappers and its unset
/// (`null`) fields removed, so a path reads as the parser's own node names —
/// `case["arg"]["ColumnRef"]` — and an absent field is `None` from `get`.
///
/// # Panics
/// Panics when the PostgreSQL grammar rejects `sql`.
pub fn parsed_nodes(sql: &str, kind: &str) -> Vec<serde_json::Value> {
    let parsed = pgsql_syntax::detailed::parse(sql, pgsql_syntax::ParserOptions::DEFAULT)
        .unwrap_or_else(|err| panic!("{}", diagnostic(sql, &err)));
    let tree = serde_json::to_value(&parsed.protobuf).expect("a parse tree serialises");
    let mut found = Vec::new();
    collect_nodes(&unwrapped(tree), kind, &mut found);
    found
}

fn unwrapped(node: serde_json::Value) -> serde_json::Value {
    match node {
        serde_json::Value::Object(mut fields) => {
            if fields.len() == 1
                && let Some(inner) = fields.remove("node")
            {
                return unwrapped(inner);
            }
            fields
                .into_iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(key, value)| (key, unwrapped(value)))
                .collect()
        }
        serde_json::Value::Array(items) => items.into_iter().map(unwrapped).collect(),
        other => other,
    }
}

fn collect_nodes(node: &serde_json::Value, kind: &str, found: &mut Vec<serde_json::Value>) {
    match node {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                if key == kind {
                    found.push(value.clone());
                }
                collect_nodes(value, kind, found);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_nodes(item, kind, found);
            }
        }
        _ => {}
    }
}

fn diagnostic(sql: &str, error: &pgsql_syntax::detailed::Error) -> String {
    let mut report = format!(
        "render-conformance oracle: PostgreSQL rejected the rendered SQL\n  {error}\n\n  {sql}\n"
    );
    // The parser's cursor is a one-based character position.
    if let pgsql_syntax::detailed::Error::Postgres { diagnostic, .. } = error
        && let Some(position) = diagnostic.cursor_position()
    {
        report.push_str("  ");
        report.push_str(&"-".repeat(position.get() - 1));
        report.push_str("^\n");
    }
    report
}

/// Carrier for the [`assert_eq`] shim's type dispatch: the inherent impls below
/// claim the string types, and every other type falls through to the no-op
/// [`NotSql`] blanket impl. Inherent methods outrank trait methods in resolution,
/// so the string types get the oracle and nothing else does.
#[derive(Debug)]
pub struct Probe<T>(pub T);

impl Probe<&String> {
    pub fn oracle_check(&self) {
        if looks_like_statement(self.0) {
            assert_parses(self.0);
        }
    }
}

impl Probe<&&str> {
    pub fn oracle_check(&self) {
        if looks_like_statement(self.0) {
            assert_parses(self.0);
        }
    }
}

/// Fallback arm of the [`Probe`] dispatch: anything that is not a rendered string
/// is not the oracle's business.
pub trait NotSql {
    fn oracle_check(&self);
}

impl<T> NotSql for Probe<T> {
    fn oracle_check(&self) {}
}

/// Drop-in replacement for `pretty_assertions::assert_eq!` that additionally
/// holds any whole-statement string on the left to the PostgreSQL grammar.
/// A test module opts in by importing it in place of `pretty_assertions::assert_eq`.
macro_rules! assert_eq {
    ($left:expr, $right:expr $(,)?) => {
        match (&$left, &$right) {
            (left, right) => {
                {
                    #[allow(unused_imports)]
                    use $crate::postgres::oracle::NotSql as _;
                    $crate::postgres::oracle::Probe(left).oracle_check();
                }
                ::pretty_assertions::assert_eq!(*left, *right);
            }
        }
    };
    ($left:expr, $right:expr, $($arg:tt)+) => {
        match (&$left, &$right) {
            (left, right) => {
                {
                    #[allow(unused_imports)]
                    use $crate::postgres::oracle::NotSql as _;
                    $crate::postgres::oracle::Probe(left).oracle_check();
                }
                ::pretty_assertions::assert_eq!(*left, *right, $($arg)+);
            }
        }
    };
}

/// `pretty_assertions::assert_eq!` under a name that says why the oracle is not
/// watching this site: the render is known-invalid and pinned in `oracle_pins.rs`.
macro_rules! assert_eq_unparsed {
    ($left:expr, $right:expr $(,)?) => {
        ::pretty_assertions::assert_eq!($left, $right)
    };
    ($left:expr, $right:expr, $($arg:tt)+) => {
        ::pretty_assertions::assert_eq!($left, $right, $($arg)+)
    };
}

pub(crate) use assert_eq;
pub(crate) use assert_eq_unparsed;
