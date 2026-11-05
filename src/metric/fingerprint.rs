//! Query identity: what a metrics hook is told about the statement it reports
//! on, and the libpg_query fingerprint that names its shape.

use std::fmt;

/// A statement's identity with its constants normalized away, as computed by
/// libpg_query — the same notion of "same query" that `pg_stat_statements`
/// aggregates by.
///
/// Two statements that differ only in their literal values share a
/// fingerprint; two that differ in shape do not. It is a parse-tree hash, not
/// a hash of the text: whitespace and literals do not move it.
///
/// [`Display`](fmt::Display) renders libpg_query's canonical 16-character
/// zero-padded hex form; [`value`](Self::value) is the same number as an
/// integer, which is the cheaper key for aggregation.
// [spec:pgorm:req:metric.fingerprint+3]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QueryFingerprint(u64);

impl QueryFingerprint {
    /// The fingerprint as libpg_query's 64-bit integer.
    pub fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for QueryFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// Which statement a query hook is reporting on.
///
/// The wrappers hand this to
/// [`MetricsCollector::record_query_success`](super::MetricsCollector::record_query_success)
/// and
/// [`MetricsCollector::record_query_error`](super::MetricsCollector::record_query_error)
/// in place of a bare operation name, so a collector can key its aggregates on
/// the query itself rather than on the seven method names. It is a borrowed
/// view, valid only for the duration of the hook call; a collector that keeps
/// anything must copy it out.
///
/// [`sql`](Self::sql) is `Some` for every statement operation — a statement is
/// its text and nothing else (`conn.sql-text`) — and `None` only for the
/// failed transaction verbs `record_query_error` reports (`"begin"`,
/// `"commit"`, `"rollback"`), where there is no statement to name.
// [spec:pgorm:req:metric.fingerprint+3]
#[derive(Clone, Copy, Debug)]
pub struct QueryContext<'a> {
    operation: &'a str,
    sql: Option<&'a str>,
}

impl<'a> QueryContext<'a> {
    /// A context for `operation` over `sql`.
    ///
    /// The fingerprint is not taken here: it is derived from `sql` on demand,
    /// so a collector that ignores it costs nothing beyond this struct.
    pub fn new(operation: &'a str, sql: Option<&'a str>) -> Self {
        Self { operation, sql }
    }

    /// The name of the operation being reported: one of the seven
    /// `ConnectionTrait` methods, `"copy_in"` / `"copy_out"` for the start of
    /// a COPY, or `"begin"` / `"commit"` / `"rollback"` for a failed
    /// transaction round trip.
    pub fn operation(&self) -> &'a str {
        self.operation
    }

    /// The statement's SQL text, where it is still available.
    pub fn sql(&self) -> Option<&'a str> {
        self.sql
    }

    /// The statement's fingerprint, or `None` when it cannot be had.
    ///
    /// It cannot be had for three distinct reasons, and they are deliberately
    /// not distinguished: the `metrics-fingerprint` feature is off, and no
    /// parser is linked in; there is no statement to parse, as for a
    /// transaction verb; or libpg_query rejected the text. That last case is
    /// not an error — raw SQL
    /// PostgreSQL's own grammar accepts may still be text this parser will not
    /// reduce to a tree, and such a statement executes normally. Metrics simply
    /// cannot name it.
    ///
    /// Fingerprinting parses, so the answer is memoized process-wide by
    /// statement text — including the `None` of a rejected one — and repeated
    /// shapes pay a hash lookup rather than a parse.
    pub fn fingerprint(&self) -> Option<QueryFingerprint> {
        self.sql.and_then(memo::of)
    }
}

/// ` [<fingerprint>]` for a statement that has one, and nothing at all for a
/// statement that does not — an unidentified query leaves no empty brackets
/// behind in the message.
#[derive(Debug)]
pub(super) struct FingerprintSuffix(pub(super) Option<QueryFingerprint>);

impl fmt::Display for FingerprintSuffix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(fingerprint) => write!(f, " [{fingerprint}]"),
            None => Ok(()),
        }
    }
}

/// The memo behind [`QueryContext::fingerprint`], parsing with libpg_query.
// [spec:pgorm:req:metric.fingerprint+3]    computation site and memoization
#[cfg(feature = "metrics-fingerprint")]
mod memo {
    use super::QueryFingerprint;
    use std::{
        collections::HashMap,
        sync::{OnceLock, PoisonError, RwLock},
    };

    /// Distinct statement texts held before the memo stops admitting new ones.
    ///
    /// An application's statement shapes are a fixed set an order of magnitude
    /// below this; what is unbounded is text built per call — an `IN` list
    /// whose arity follows the input, a script assembled by a migration. The
    /// cap keeps those from retaining memory forever, at the cost of
    /// re-parsing them once the memo is full. There is no eviction: an LRU
    /// would trade a bounded, predictable cost for a moving one, and the
    /// entries worth keeping are the ones seen first and often.
    const CAPACITY: usize = 1024;

    type Memo = RwLock<HashMap<Box<str>, Option<QueryFingerprint>>>;

    fn memo() -> &'static Memo {
        static MEMO: OnceLock<Memo> = OnceLock::new();
        MEMO.get_or_init(Memo::default)
    }

    /// The fingerprint of `sql`, parsing it only the first time it is seen.
    pub(super) fn of(sql: &str) -> Option<QueryFingerprint> {
        let memo = memo();

        if let Some(known) = memo.read().unwrap_or_else(PoisonError::into_inner).get(sql) {
            return *known;
        }

        // The default options are PostgreSQL 18's query-ID rules: an alias
        // stands for the relation it names and a schema qualifier is not
        // part of the identity, so a fingerprint groups statements the way
        // the server's own `pg_stat_statements` does.
        let computed = pg_query::fingerprint(
            sql,
            pg_query::ParserOptions::DEFAULT,
            pg_query::FingerprintOptions::DEFAULT,
        )
        .ok()
        .map(|fingerprint| QueryFingerprint(fingerprint.value));

        let mut memo = memo.write().unwrap_or_else(PoisonError::into_inner);
        if memo.len() < CAPACITY {
            memo.insert(Box::from(sql), computed);
        }

        computed
    }
}

/// The stand-in for the memo when fingerprints are not asked for.
// [spec:pgorm:req:metric.fingerprint+3]    the feature-off answer
#[cfg(not(feature = "metrics-fingerprint"))]
mod memo {
    use super::QueryFingerprint;

    /// Always `None`: without `metrics-fingerprint` no statement is parsed for
    /// its identity, so none costs a parse.
    pub(super) fn of(_sql: &str) -> Option<QueryFingerprint> {
        None
    }
}
