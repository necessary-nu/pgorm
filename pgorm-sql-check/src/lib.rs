//! PostgreSQL's grammar held to SQL text, and PRQL compiled to it.
//!
//! [`check_sql`] parses text with PostgreSQL 19's grammar (pgsql-syntax, a
//! Rust port of the server's own scanner and parser), and [`compile_prql`]
//! compiles PRQL to PostgreSQL SQL through prqlc, holds the result to the same
//! grammar, and counts its `$N` placeholders. pgorm-sql-macro runs both on a
//! literal while its caller compiles (`sql!`, `prql!`), and pgorm runs them on
//! text at run time for the language bindings, which have no compile step; one
//! implementation serves both, so a refusal reads the same wherever it is
//! made.
//!
//! The checks are of the grammar alone. The parser carries no catalog, so a
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

use pgsql_syntax::detailed;

/// What the grammar refused: the parser's own diagnostic, and where in the
/// text it stopped when the parser gives a cursor.
// [spec:pgorm:req:macros.sql.reject+2]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    message: String,
    offset: Option<usize>,
    caret: Option<String>,
}

impl Rejection {
    fn of(sql: &str, error: &detailed::Error) -> Self {
        match error {
            detailed::Error::Postgres { diagnostic, .. } => Self::at(
                sql,
                diagnostic.message.clone(),
                diagnostic
                    .cursor_position()
                    .and_then(|position| byte_offset(sql, position.get())),
            ),
            detailed::Error::Api(pgsql_syntax::Error::Parse(message)) => {
                Self::at(sql, message.clone(), None)
            }
            detailed::Error::Api(other) => Self::at(sql, other.to_string(), None),
        }
    }

    fn at(sql: &str, message: String, offset: Option<usize>) -> Self {
        let offset = offset.map(|offset| offset.min(sql.trim_end().len()));
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

    /// Where the parser stopped, as a byte offset into the text it read: the
    /// token it refused, or the end of the text when the text ended too soon.
    pub fn offset(&self) -> Option<usize> {
        self.offset
    }

    /// The line of the text holding [`offset`](Rejection::offset), reprinted
    /// with a caret under that column, as a block that follows the message.
    pub fn caret(&self) -> Option<&str> {
        self.caret.as_deref()
    }
}

/// `PostgreSQL rejected this SQL: <message>`, the line the parser stopped on
/// reprinted beneath with a caret where it stopped.
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

/// Hold `sql` to PostgreSQL 19's grammar: one statement, or a `;`-separated
/// script, each statement of which is held to it.
///
/// One parse walks a whole script and stops at the first statement the
/// grammar refuses, so a script needs no splitting of its own.
// [spec:pgorm:sem:macros.sql.script+2]
// [spec:pgorm:req:macros.sql.ceiling+2]    syntax only: the parser has no catalog
// [spec:pgorm:req:sql.target+4]    PostgreSQL 19's grammar, the one release targeted
pub fn check_sql(sql: &str) -> Result<(), Rejection> {
    parse(sql).map(|_| ())
}

/// What a statement does to the database, as far as its grammar shows.
// [spec:pgorm:def:read-only.classify]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Every statement only reads: a `SELECT`, `VALUES` or `TABLE` that
    /// holds no data-modifying `WITH`, no row-locking clause and no `INTO`,
    /// a `SHOW`, or an `EXPLAIN` that does not `ANALYZE`, whatever it
    /// explains.
    Reads,
    /// At least one statement may write: anything else.
    Writes,
}

/// Hold `sql` to the grammar, as [`check_sql`] does, and say whether every
/// statement in it only reads.
///
/// The answer is the grammar's, so it is what the text shows, not what it
/// runs: a `SELECT` calling a function that writes is a read here, and only
/// a server running the statement read-only refuses it.
// [spec:pgorm:def:read-only.classify]
pub fn classify_sql(sql: &str) -> Result<Effect, Rejection> {
    use pgsql_syntax::{NodeEnum, NodeRef};

    let parsed = parse(sql)?;
    let mut effect = Effect::Reads;
    let mut explained = false;
    pgsql_syntax::visit(&parsed.protobuf, |node| match node {
        NodeRef::RawStmt(raw) => {
            explained = false;
            match raw.stmt.as_ref().and_then(|stmt| stmt.node.as_ref()) {
                Some(NodeEnum::SelectStmt(_) | NodeEnum::VariableShowStmt(_)) => {}
                Some(NodeEnum::ExplainStmt(explain)) if !analyzes(explain) => explained = true,
                _ => effect = Effect::Writes,
            }
        }
        NodeRef::InsertStmt(_)
        | NodeRef::UpdateStmt(_)
        | NodeRef::DeleteStmt(_)
        | NodeRef::MergeStmt(_)
        | NodeRef::LockingClause(_)
        | NodeRef::IntoClause(_)
            if !explained =>
        {
            effect = Effect::Writes;
        }
        _ => {}
    });
    Ok(effect)
}

/// Whether an `EXPLAIN` runs what it explains: an `ANALYZE` option that is
/// not turned off by `false`, `off` or `0`. An argument the server would
/// refuse counts as running, so nothing unclear passes as a read.
fn analyzes(explain: &pgsql_syntax::protobuf::ExplainStmt) -> bool {
    use pgsql_syntax::NodeEnum;

    explain.options.iter().any(|option| {
        let Some(NodeEnum::DefElem(element)) = option.node.as_ref() else {
            return false;
        };
        if !element.defname.eq_ignore_ascii_case("analyze") {
            return false;
        }
        match element.arg.as_ref().and_then(|arg| arg.node.as_ref()) {
            Some(NodeEnum::String(text)) => {
                !(text.sval.eq_ignore_ascii_case("false") || text.sval.eq_ignore_ascii_case("off"))
            }
            Some(NodeEnum::Integer(number)) => number.ival != 0,
            Some(NodeEnum::Boolean(flag)) => flag.boolval,
            _ => true,
        }
    })
}

/// The tree of `sql`, or the grammar's refusal of it.
fn parse(sql: &str) -> Result<pgsql_syntax::ParseResult, Rejection> {
    detailed::parse(sql, pgsql_syntax::ParserOptions::DEFAULT)
        .map_err(|error| Rejection::of(sql, &error))
}

/// The byte offset of the one-based character `position` the parser reports,
/// which is one past the last character when the text ended too soon.
fn byte_offset(sql: &str, position: usize) -> Option<usize> {
    let index = position.checked_sub(1)?;
    sql.char_indices()
        .map(|(offset, _)| offset)
        .chain(std::iter::once(sql.len()))
        .nth(index)
}

/// The line of `sql` holding byte `offset`, with a caret beneath that column.
fn caret_block(sql: &str, offset: usize) -> Option<String> {
    let (before, after) = sql.split_at_checked(offset)?;
    let head = before.get(before.rfind('\n').map_or(0, |index| index + 1)..)?;
    let tail = after.get(..after.find('\n').unwrap_or(after.len()))?;
    let column = " ".repeat(head.chars().count());

    Some(format!("\n\n  {head}{tail}\n  {column}^"))
}

/// PRQL compiled to PostgreSQL SQL, and the number of distinct `$N`
/// placeholders the SQL asks values for.
// [spec:pgorm:def:macros.prql+5]
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
// [spec:pgorm:def:macros.prql+5]
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
/// The parse is both the grammar gate and the census: a statement it refuses
/// is a refusal, and a statement it accepts is walked for `ParamRef` nodes,
/// which is exactly the set the server will demand at prepare time — `$5`
/// inside a string literal does not count, `$N` smuggled in through an
/// s-string does. pgsql-syntax's `visit` reaches every record of the tree, so
/// a placeholder cannot hide in a clause a narrower walk would skip.
// [spec:pgorm:req:macros.prql.reject]    the emitted SQL must pass the oracle
// [spec:pgorm:sem:macros.prql.census+2]
fn census(sql: &str) -> Result<BTreeSet<i64>, PrqlRejection> {
    let parsed = parse(sql).map_err(|rejection| PrqlRejection::Grammar {
        sql: sql.to_owned(),
        rejection,
    })?;

    let mut found = BTreeSet::new();
    pgsql_syntax::visit(&parsed.protobuf, |node| {
        if let pgsql_syntax::NodeRef::ParamRef(param) = node {
            found.insert(i64::from(param.number));
        }
    });

    Ok(found)
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

    // [spec:pgorm:req:macros.sql.reject+2/test]    the token, its offset and the caret
    // [spec:pgorm:req:macros.runtime+2/test]    a run-time check refuses as the macro does
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
        let Err(end) = check_sql("SELECT id FROM\n") else {
            panic!("FROM takes a table");
        };
        assert_eq!(end.message(), "syntax error at end of input");
        assert_eq!(end.token(), None);
        assert_eq!(end.offset(), Some(14));
        assert_eq!(end.caret(), Some("\n\n  SELECT id FROM\n                ^"));
    }

    // [spec:pgorm:req:macros.sql.ceiling+2/test]    the parser's own cursor, not the
    // token's last occurrence in the text
    #[test]
    fn the_caret_is_the_parsers_cursor() {
        let Err(rejection) = check_sql("SELECT name FROM cake ORDER name") else {
            panic!("ORDER takes BY");
        };
        assert_eq!(rejection.token(), Some("name"));
        assert_eq!(rejection.offset(), Some(28));
        let Err(wide) = check_sql("SELECT 'é' FROM cake ORDER name") else {
            panic!("ORDER takes BY");
        };
        assert_eq!(wide.offset(), Some(28));
    }

    // [spec:pgorm:req:macros.sql.reject+2/test]    a script's offset counts from its start
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
    // [spec:pgorm:req:macros.runtime+2/test]
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

    // [spec:pgorm:def:read-only.classify/test]    reads are SELECT, VALUES, TABLE, SHOW and an EXPLAIN that does not ANALYZE; a data-modifying WITH, a row lock or an INTO anywhere makes a write
    #[test]
    fn statements_are_classified_by_what_they_write() {
        let effect = |sql: &str| classify_sql(sql).unwrap_or_else(|error| panic!("{error}"));
        for read in [
            "SELECT id, name FROM cake WHERE id > $1",
            "VALUES (1, 'a'), (2, 'b')",
            "TABLE cake",
            "SHOW search_path",
            "SELECT 1; SELECT 2",
            "WITH recent AS (SELECT * FROM cake ORDER BY id DESC LIMIT 5) SELECT * FROM recent",
            "SELECT * FROM cake UNION ALL SELECT * FROM pie",
            "SELECT (SELECT count(*) FROM pie WHERE pie.cake_id = cake.id) FROM cake",
            "EXPLAIN SELECT * FROM cake",
            "EXPLAIN INSERT INTO cake (id) VALUES (1)",
            "EXPLAIN (ANALYZE false) DELETE FROM cake",
            "EXPLAIN (ANALYZE off, VERBOSE) UPDATE cake SET name = 'x'",
            "EXPLAIN (ANALYZE 0) SELECT * FROM cake FOR UPDATE",
            "SELECT nextval('cake_id_seq')",
            "SELECT 1; EXPLAIN DELETE FROM cake; SELECT 2",
        ] {
            assert_eq!(effect(read), Effect::Reads, "{read}");
        }
        for write in [
            "INSERT INTO cake (id) VALUES (1)",
            "UPDATE cake SET name = 'x'",
            "DELETE FROM cake",
            "MERGE INTO cake USING pie ON cake.id = pie.id WHEN MATCHED THEN DELETE",
            "CREATE TABLE cake (id int)",
            "SET search_path TO public",
            "BEGIN",
            "CALL refresh()",
            "COPY cake TO STDOUT",
            "WITH gone AS (DELETE FROM cake RETURNING *) SELECT * FROM gone",
            "WITH added AS (INSERT INTO cake (id) VALUES (1) RETURNING id) SELECT * FROM added",
            "SELECT * FROM cake FOR UPDATE",
            "SELECT * FROM cake FOR NO KEY UPDATE",
            "SELECT * FROM cake FOR SHARE",
            "SELECT * FROM cake FOR KEY SHARE SKIP LOCKED",
            "SELECT * FROM (SELECT * FROM cake FOR UPDATE) AS locked",
            "SELECT * INTO cake_copy FROM cake",
            "EXPLAIN ANALYZE SELECT * FROM cake",
            "EXPLAIN (ANALYZE) SELECT * FROM cake",
            "EXPLAIN (ANALYZE true) DELETE FROM cake",
            "EXPLAIN (ANALYZE on) SELECT * FROM cake",
            "SELECT 1; DELETE FROM cake",
            "DELETE FROM cake; EXPLAIN SELECT 1",
        ] {
            assert_eq!(effect(write), Effect::Writes, "{write}");
        }
        assert_eq!(
            classify_sql("SELEC 1").map_err(|refusal| refusal.token().map(str::to_owned)),
            Err(Some("SELEC".to_owned()))
        );
    }

    // [spec:pgorm:req:sql.target+4/test]    the grammar is PostgreSQL 19's:
    // what 19 added passes, in `sql!`'s check and in PRQL's emitted SQL alike
    #[test]
    fn nineteens_grammar_is_the_one_linked() {
        for sql in [
            "INSERT INTO cake (id) VALUES (1) ON CONFLICT (id) DO SELECT RETURNING *",
            "SELECT lag(price) IGNORE NULLS OVER (ORDER BY id) FROM cake",
            "REPACK cake USING INDEX cake_pkey",
            "WAIT FOR LSN '0/1'",
        ] {
            assert!(check_sql(sql).is_ok(), "{sql}");
        }
        let compiled = compile_prql(r#"from cake | select {s"lag(price) RESPECT NULLS OVER ()"}"#)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(
            compiled.sql().contains("RESPECT NULLS"),
            "{}",
            compiled.sql()
        );
        // 19 made `ignore` and `respect` keywords that are not bare column
        // labels, which 18 read as identifiers.
        for word in ["ignore", "respect"] {
            let bare = check_sql(&format!("SELECT 1 {word}"));
            assert_eq!(bare.as_ref().err().and_then(Rejection::token), Some(word));
            assert!(check_sql(&format!("SELECT 1 AS {word}")).is_ok());
        }
    }
}
