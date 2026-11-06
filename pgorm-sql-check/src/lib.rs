//! PostgreSQL's grammar held to SQL text, and PRQL compiled to it.
//!
//! [`check_sql`] parses text with libpg_query, the PostgreSQL server's own
//! parser, and [`compile_prql`] compiles PRQL to PostgreSQL SQL through prqlc,
//! holds the result to the same grammar, and counts its `$N` placeholders.
//! pgorm-sql-macro runs both on a literal while its caller compiles (`sql!`,
//! `prql!`), and pgorm runs them on text at run time for the language
//! bindings, which have no compile step; one implementation serves both, so a
//! refusal reads the same wherever it is made.
//!
//! The checks are of the grammar alone. libpg_query carries no catalog, so a
//! table or a column that does not exist passes.
#![warn(missing_docs)]
#![deny(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::todo,
    clippy::unreachable,
    clippy::unwrap_used
)]

use std::collections::BTreeSet;
use std::fmt;

/// What the grammar refused: the parser's own diagnostic, and where in the
/// text it stopped when the diagnostic names a token.
// [spec:pgorm:req:macros.sql.reject]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    message: String,
    offset: Option<usize>,
    caret: Option<String>,
}

impl Rejection {
    fn of(sql: &str, error: &pg_query::Error) -> Self {
        let message = match error {
            pg_query::Error::Parse(message) => message.clone(),
            other => other.to_string(),
        };
        let offset = token_offset(sql, &message);
        let caret = offset.and_then(|offset| caret_block(sql, offset));
        Self {
            message,
            offset,
            caret,
        }
    }

    /// The parser's diagnostic, such as `syntax error at or near "FORM"`.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The token the parser stopped at, when its diagnostic quotes one; a
    /// diagnostic such as `syntax error at end of input` quotes none.
    pub fn token(&self) -> Option<&str> {
        self.message
            .split("at or near \"")
            .nth(1)?
            .strip_suffix('"')
    }

    /// Where the parser stopped, as a byte offset into the text it read.
    pub fn offset(&self) -> Option<usize> {
        self.offset
    }

    /// The line of the text holding [`offset`](Rejection::offset), reprinted
    /// with a caret under the token, as a block that follows the message.
    pub fn caret(&self) -> Option<&str> {
        self.caret.as_deref()
    }
}

/// `PostgreSQL rejected this SQL: <message>`, the line the parser stopped on
/// reprinted beneath with a caret under the token.
impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PostgreSQL rejected this SQL: {}", self.message)?;
        if let Some(caret) = &self.caret {
            f.write_str(caret)?;
        }
        Ok(())
    }
}

impl std::error::Error for Rejection {}

/// Hold `sql` to PostgreSQL's grammar: one statement, or a `;`-separated
/// script, each statement of which is held to it.
///
/// `pg_query::parse` walks a whole script in one pass and stops at the first
/// statement the grammar refuses, so a script needs no splitting of its own.
// [spec:pgorm:sem:macros.sql.script]
// [spec:pgorm:req:macros.sql.ceiling]    syntax only: the parser has no catalog
pub fn check_sql(sql: &str) -> Result<(), Rejection> {
    pg_query::parse(sql, pg_query::ParserOptions::DEFAULT)
        .map(|_| ())
        .map_err(|error| Rejection::of(sql, &error))
}

/// The line of `sql` holding byte `offset`, with a caret beneath that column.
fn caret_block(sql: &str, offset: usize) -> Option<String> {
    let (before, after) = sql.split_at_checked(offset)?;
    let head = before.get(before.rfind('\n').map_or(0, |index| index + 1)..)?;
    let tail = after.get(..after.find('\n').unwrap_or(after.len()))?;
    let column = " ".repeat(head.chars().count());

    Some(format!("\n\n  {head}{tail}\n  {column}^"))
}

/// Where the parser stopped, as a byte offset into `sql`.
///
/// libpg_query surfaces only the message, not the cursor position, so the offset
/// is recovered from the token the message names — and messages that name no
/// token, such as `syntax error at end of input`, get no caret. Matching the
/// render oracle in `pgorm-query/tests/postgres/oracle.rs`, the last occurrence
/// is taken: a token the parser choked on rarely reappears past the point it
/// gave up.
fn token_offset(sql: &str, message: &str) -> Option<usize> {
    let token = message.split("at or near \"").nth(1)?.strip_suffix('"')?;

    sql.rfind(token)
}

/// PRQL compiled to PostgreSQL SQL, and the number of distinct `$N`
/// placeholders the SQL asks values for.
// [spec:pgorm:def:macros.prql+1]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledPrql {
    sql: String,
    placeholders: usize,
}

impl CompiledPrql {
    /// The SQL the PRQL compiled to, which the grammar accepted.
    pub fn sql(&self) -> &str {
        &self.sql
    }

    /// The SQL, handed over without a copy.
    pub fn into_sql(self) -> String {
        self.sql
    }

    /// How many values the SQL takes: its placeholders run `$1` to this.
    pub fn placeholders(&self) -> usize {
        self.placeholders
    }

    /// Hold `given` values to the placeholders: one value per distinct
    /// placeholder, in placeholder order. A placeholder reused is one value.
    // [spec:pgorm:req:macros.prql.reject]    arity errors name the placeholder
    pub fn check_arity(&self, given: usize) -> Result<(), ArityRejection> {
        let expected = self.placeholders;
        if given < expected {
            return Err(ArityRejection::Unbound { expected, given });
        }
        if given > expected {
            return Err(ArityRejection::Surplus {
                expected,
                number: expected + 1,
            });
        }
        Ok(())
    }
}

/// Why PRQL did not become SQL a statement can run.
// [spec:pgorm:req:macros.prql.reject]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PrqlRejection {
    /// prqlc refused the PRQL, with its own diagnostics. PRQL refuses a
    /// parameterized limit, `take $1`, here.
    Compile(String),
    /// The SQL prqlc emitted failed the grammar: an s-string's contents, or
    /// a compiler bug.
    Grammar {
        /// The SQL prqlc emitted.
        sql: String,
        /// What the grammar refused in it.
        rejection: Rejection,
    },
    /// The SQL uses `$0`, which the grammar accepts and no bind protocol can
    /// satisfy.
    ZeroPlaceholder,
    /// The SQL's placeholders skip a number, so the values would bind out of
    /// step with them.
    Gap {
        /// The highest placeholder used.
        max: i64,
        /// The first placeholder below it that is not.
        missing: i64,
    },
    /// The grammar accepted the SQL, and its parse tree did not serialize for
    /// the placeholder census: a pgorm bug.
    Census(String),
}

impl fmt::Display for PrqlRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compile(errors) => write!(f, "PRQL rejected this query:\n{errors}"),
            // The caller never wrote this SQL, so the message always shows
            // it: with a caret under the column the parser named when it
            // named one, bare otherwise.
            Self::Grammar { sql, rejection } => {
                write!(
                    f,
                    "PostgreSQL rejected the SQL compiled from this PRQL: {}",
                    rejection.message()
                )?;
                match rejection.caret() {
                    Some(caret) => f.write_str(caret),
                    None => write!(f, "\n\n  {sql}"),
                }
            }
            Self::ZeroPlaceholder => {
                f.write_str("PostgreSQL placeholders start at $1: $0 can never be bound")
            }
            Self::Gap { max, missing } => write!(
                f,
                "the query uses ${max} but never ${missing}: placeholders must be contiguous from $1"
            ),
            Self::Census(error) => write!(
                f,
                "pgorm bug: the accepted parse tree did not serialize: {error}"
            ),
        }
    }
}

impl std::error::Error for PrqlRejection {}

/// A number of values that does not match a query's placeholders.
// [spec:pgorm:req:macros.prql.reject]    arity errors name the placeholder
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ArityRejection {
    /// Fewer values than placeholders: nothing binds the rest.
    Unbound {
        /// The placeholders the query has.
        expected: usize,
        /// The values the caller gave, fewer than `expected`.
        given: usize,
    },
    /// More values than placeholders: the query has no `$number` for the
    /// first value past them.
    Surplus {
        /// The placeholders the query has.
        expected: usize,
        /// The placeholder the first surplus value would bind.
        number: usize,
    },
}

impl fmt::Display for ArityRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unbound { expected, given } => {
                let unbound = ((given + 1)..=*expected)
                    .map(|number| format!("${number}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(
                    f,
                    "the query has {expected} placeholder(s) but {given} argument(s) were given: nothing binds {unbound}"
                )
            }
            Self::Surplus { expected, number } => write!(
                f,
                "no ${number} in the query for this argument: it has {expected} placeholder(s)"
            ),
        }
    }
}

impl std::error::Error for ArityRejection {}

/// Compile PRQL text to PostgreSQL SQL, hold the SQL to the grammar, and
/// count its placeholders, refusing `$0` and a sequence with a gap.
///
/// The census reads the *emitted SQL's* parse tree rather than the PRQL AST:
/// an s-string can carry a raw `$N` straight into the SQL without any `Param`
/// node existing on the PRQL side, so the only complete account of what the
/// server will ask for is the parser's own view of the finished statement.
/// S-strings splice through unscreened; their boundary is the grammar, over
/// the finished statement.
// [spec:pgorm:def:macros.prql+1]
// [spec:pgorm:req:macros.prql.reject]    PRQL rejections carry prqlc's message
// [spec:pgorm:sem:macros.prql.sstring]    pass-through; the oracle is the gate
pub fn compile_prql(prql: &str) -> Result<CompiledPrql, PrqlRejection> {
    let options = prqlc::Options::default()
        .with_target(prqlc::Target::Sql(Some(prqlc::sql::Dialect::Postgres)))
        .with_display(prqlc::DisplayOptions::Plain)
        .no_format()
        .no_signature();

    let sql = prqlc::compile(prql, &options)
        .map_err(|errors| PrqlRejection::Compile(errors.to_string()))?;
    let placeholders = census(&sql)?;
    check_contiguity(&placeholders)?;

    Ok(CompiledPrql {
        sql,
        placeholders: placeholders.len(),
    })
}

/// Every `$N` the emitted SQL asks for, straight from the parser's tree.
///
/// `pg_query::parse` is both the grammar gate and the census: a statement it
/// refuses is a refusal, and a statement it accepts is walked for `ParamRef`
/// nodes, which is exactly the set the server will demand at prepare time —
/// `$5` inside a string literal does not count, `$N` smuggled in through an
/// s-string does.
// [spec:pgorm:req:macros.prql.reject]    the emitted SQL must pass the oracle
// [spec:pgorm:sem:macros.prql.census]
fn census(sql: &str) -> Result<BTreeSet<i64>, PrqlRejection> {
    let parsed = pg_query::parse(sql, pg_query::ParserOptions::DEFAULT).map_err(|error| {
        PrqlRejection::Grammar {
            sql: sql.to_owned(),
            rejection: Rejection::of(sql, &error),
        }
    })?;
    let tree = serde_json::to_value(&parsed.protobuf)
        .map_err(|error| PrqlRejection::Census(error.to_string()))?;

    let mut found = BTreeSet::new();
    collect_params(&tree, &mut found);

    Ok(found)
}

/// Walk the serialized parse tree for `ParamRef` nodes.
///
/// The tree is walked as JSON because pg_query's visitor covers only the node
/// types its table-extraction cares about; serde sees every field, so a
/// placeholder cannot hide in a clause the visitor skips.
// [spec:pgorm:sem:macros.prql.census]
fn collect_params(node: &serde_json::Value, found: &mut BTreeSet<i64>) {
    match node {
        serde_json::Value::Object(fields) => {
            if let Some(serde_json::Value::Object(param)) = fields.get("ParamRef")
                && let Some(number) = param.get("number").and_then(serde_json::Value::as_i64)
            {
                found.insert(number);
            }
            for value in fields.values() {
                collect_params(value, found);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_params(item, found);
            }
        }
        _ => {}
    }
}

/// The placeholders must run `$1..=$max` with no gaps.
///
/// prqlc passes any `$N` through untouched, so `$1, $3` compiles into SQL
/// happily — and would then bind `$3` to the second value. A gap is refused by
/// name before arity is even considered, as is `$0`, which PostgreSQL's
/// grammar accepts but no bind protocol can satisfy.
// [spec:pgorm:req:macros.prql.reject]    gaps and `$0` refused by name
fn check_contiguity(placeholders: &BTreeSet<i64>) -> Result<(), PrqlRejection> {
    if placeholders.contains(&0) {
        return Err(PrqlRejection::ZeroPlaceholder);
    }
    let Some(max) = placeholders.last().copied() else {
        return Ok(());
    };
    if let Some(missing) = (1..max).find(|number| !placeholders.contains(number)) {
        return Err(PrqlRejection::Gap { max, missing });
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests {
    use super::*;

    // [spec:pgorm:req:macros.sql.reject/test]    the token, its offset and the caret
    // [spec:pgorm:req:macros.runtime/test]    a run-time check refuses as the macro does
    #[test]
    fn a_rejection_points_at_its_token() {
        let sql = "SELECT id FROM cake\nORDER name";
        let Err(rejection) = check_sql(sql) else {
            panic!("ORDER takes BY");
        };
        assert_eq!(rejection.message(), r#"syntax error at or near "name""#);
        assert_eq!(rejection.token(), Some("name"));
        assert_eq!(rejection.offset(), Some(26));
        assert_eq!(
            rejection.to_string(),
            "PostgreSQL rejected this SQL: syntax error at or near \"name\"\n\n  ORDER name\n        ^"
        );
        let end = check_sql("SELECT").err();
        assert_eq!(end.as_ref().and_then(Rejection::token), None);
    }

    // [spec:pgorm:req:macros.sql.reject/test]    a script's offset counts from its start
    #[test]
    fn a_script_is_held_statement_by_statement() {
        assert!(check_sql("SELECT 1; SELECT 2").is_ok());
        let Err(rejection) = check_sql("SELECT 1;\nSELEC 2") else {
            panic!("the second statement is not SQL");
        };
        assert_eq!(rejection.token(), Some("SELEC"));
        assert_eq!(rejection.offset(), Some(10));
        assert_eq!(rejection.caret(), Some("\n\n  SELEC 2\n  ^"));
    }

    // [spec:pgorm:req:macros.prql.reject/test]    the census reads the parser's tree
    #[test]
    fn the_census_counts_only_parameters() {
        let quoted = compile_prql(r#"from cake | filter name == s"'$5'""#)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(quoted.placeholders(), 0);
        assert_eq!(
            compile_prql(r#"from cake | filter id > s"$0""#),
            Err(PrqlRejection::ZeroPlaceholder)
        );
        assert!(matches!(
            compile_prql(r#"from cake | filter s"id >""#),
            Err(PrqlRejection::Grammar { .. })
        ));
    }

    // [spec:pgorm:req:macros.prql.reject/test]    placeholders counted, gaps and arity refused
    // [spec:pgorm:req:macros.runtime/test]
    #[test]
    fn prql_counts_its_placeholders() {
        let compiled = compile_prql("from cake | filter price < $1 | filter id > $1")
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(compiled.placeholders(), 1);
        assert!(compiled.check_arity(1).is_ok());
        assert_eq!(
            compiled.check_arity(0).map_err(|error| error.to_string()),
            Err(
                "the query has 1 placeholder(s) but 0 argument(s) were given: nothing binds $1"
                    .to_owned()
            )
        );
        assert_eq!(
            compiled.check_arity(2).map_err(|error| error.to_string()),
            Err("no $2 in the query for this argument: it has 1 placeholder(s)".to_owned())
        );
        assert_eq!(
            compile_prql("from cake | filter a > $1 | filter b < $3"),
            Err(PrqlRejection::Gap { max: 3, missing: 2 })
        );
        assert!(matches!(
            compile_prql("from cake | take $1"),
            Err(PrqlRejection::Compile(_))
        ));
    }
}
