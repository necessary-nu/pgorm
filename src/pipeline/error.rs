//! The pipeline's typed failure channel.

/// Why a finished [`Pipeline`](super::Pipeline) could not become SQL.
///
/// Construction itself is infallible; everything that can go wrong is
/// reported here, at the [`into_sql`](super::Pipeline::into_sql) boundary,
/// never as a panic.
// [spec:pgorm:req:pipeline.errors+5]
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PipelineError {
    /// An alias collides with a name the PRQL standard library binds.
    ///
    /// The colliding set is closed: the top-level names of prqlc's `std`
    /// module plus the language's keywords, listed in the
    /// `pipeline.errors` spec rule. An alias in that set would shadow the
    /// built-in for the rest of the pipeline, so it is refused up front
    /// with the name in hand rather than surfacing later as an opaque
    /// resolution failure.
    #[error("alias `{0}` collides with a PRQL built-in name; choose another alias")]
    ReservedAlias(String),
    /// An identifier the pipeline cannot guarantee to write as a name: it
    /// carries a double quote or a NUL byte, begins with `$`, or is `*`.
    ///
    /// Quoting is the compiler's rather than pgorm's — a table, schema
    /// segment, column or alias travels to prqlc as plain text and is
    /// rendered there — so what reaches the SQL is decided by whichever
    /// prqlc the build resolved. pgorm depends on its fork, but a consumer's
    /// own `patch` table can redirect that dependency and a crates.io release
    /// could not carry it at all, so pgorm cannot decide which compiler a
    /// consumer links, and a name whose meaning depends on that has no
    /// rendering worth choosing. Each refused shape is one where the answer
    /// does depend on it, or is wrong under both:
    ///
    /// - An embedded `"`. The fork doubles it; the registry crate's escaper
    ///   leaves a `"` that follows a backslash alone, which closes the quoted
    ///   identifier early and hands the rest of the name to the server as
    ///   SQL. Doubling it here would only move the ambiguity: the fork would
    ///   double it a second time.
    /// - NUL, which PostgreSQL carries in no identifier under any quoting.
    /// - A leading `$`. Both compilers write a name bare whenever it matches
    ///   `[a-z_$][a-z0-9_$]*` and is not a keyword, a pattern that predates
    ///   PRQL's own `$name` parameter token; PostgreSQL's lexer reads a bare
    ///   `$1` as a bound parameter and a bare `$$` or `$tag$` as a
    ///   dollar-quote delimiter, so two such names swallow the SQL between
    ///   them as a string constant.
    /// - A lone `*`, which the compiler resolves as its wildcard before any
    ///   quoting is decided, so no spelling of it reaches the SQL as a name.
    ///
    /// So the name is refused rather than escaped, at
    /// [`into_sql`](super::Pipeline::into_sql) and before prqlc is called, by
    /// pgorm's own code — which is what makes the outcome the same under
    /// either compiler.
    ///
    /// Length is deliberately not part of this: an identifier past
    /// PostgreSQL's 63-byte limit is truncated server-side and may then
    /// collide with another, which is a correctness question with a different
    /// answer.
    #[error(
        "identifier `{0}` cannot be written as a pipeline name: it contains a double quote or \
         NUL byte, begins with `$`, or is `*`; rename it"
    )]
    UnquotableIdentifier(String),
    /// prqlc rejected the pipeline during lowering.
    ///
    /// Name-resolution failures — a PRQL built-in used as a value, a column
    /// that is not in scope after `select` — surface here, carrying the
    /// compiler's own diagnostic text.
    ///
    /// A reference to a name no stage introduced is *not* among them: with
    /// no catalog it resolves as a column of the source relation, and the
    /// server answers for it at execution.
    #[error("PRQL compilation failed: {0}")]
    Compile(String),
    /// [`select_sources`](super::Pipeline::select_sources) was asked to
    /// project entity models out of a pipeline that no longer carries its
    /// sources' column namespaces.
    ///
    /// The named stage — `select`, `group().aggregate()`, a `distinct` that
    /// had to settle the pipeline into its own binding, or a set operation
    /// (`append`, `intersect`, `remove`) whose result more than one listed
    /// source would be read from — replaced, collapsed or combined the
    /// sources' own columns, so a per-source projection cannot resolve. Refused before prqlc
    /// compiles, so the answer names the stage rather than an opaque
    /// unresolved-name diagnostic; decode a reshaped pipeline with
    /// [`into_model`](super::Pipeline::into_model) or
    /// [`into_tuple`](super::Pipeline::into_tuple) instead, or move the
    /// reshaping after the terminal cannot-follow boundary by not doing it
    /// at all — `filter`, `derive`, `sort`, `take`, `join` and `window` all
    /// leave the sources addressable, and a set operation leaves one.
    // [spec:pgorm:sem:pipeline.select-sources+6]
    #[error(
        "select_sources after `{0}`: the stage replaced the sources' own column namespaces, \
         so entity models can no longer be projected; list sources before reshaping, or decode \
         with into_model / into_tuple"
    )]
    ReshapedSources(&'static str),
}

// [spec:pgorm:req:pipeline.errors+5]
impl From<PipelineError> for crate::Error {
    fn from(err: PipelineError) -> Self {
        crate::Error::Query(crate::error::RuntimeError::Internal(err.to_string()))
    }
}

/// Whether `name` is a name pgorm refuses to render rather than quote, as
/// [`PipelineError::UnquotableIdentifier`].
///
/// The set is closed and decided by pgorm alone, never by asking the
/// compiler: a name carrying `"` or NUL, a name beginning with `$`, and the
/// name `*` (the variant's documentation says why each). Every other name is
/// one both compilers either quote or write bare as an identifier the lexer
/// reads back as that name: a space, a capital, a keyword, or a `$` after the
/// first character (`a$b` is one identifier to PostgreSQL) is an ordinary
/// name and renders, and a name past the 63-byte identifier limit is a
/// different question, about what it may collide with once the server
/// truncates it.
// [spec:pgorm:req:pipeline.errors+5]
// [spec:pgorm:req:security.ident-oracle.nul+2] (the pipeline's refusal class)
pub(super) fn unquotable(name: &str) -> bool {
    name.contains(['"', '\0']) || name.starts_with('$') || name == "*"
}

/// The closed set of names an alias must not take: every top-level binding of
/// the fork's `std` module — prqlc 0.13's, and the two set operations it adds
/// — its submodule names, and the PRQL keywords.
// [spec:pgorm:req:pipeline.errors+5]
pub(super) const RESERVED: &[&str] = &[
    "_append_by_name",
    "_eq",
    "_except_all",
    "_intersect_all",
    "_is_null",
    "_param",
    "add",
    "aggregate",
    "all",
    "and",
    "any",
    "append",
    "as",
    "average",
    "case",
    "coalesce",
    "concat_array",
    "count",
    "count_distinct",
    "date",
    "default_db",
    "derive",
    "div_f",
    "div_i",
    "eq",
    "false",
    "filter",
    "first",
    "from",
    "from_text",
    "func",
    "group",
    "gt",
    "gte",
    "in",
    "internal",
    "intersect",
    "into",
    "join",
    "lag",
    "last",
    "lead",
    "let",
    "loop",
    "lt",
    "lte",
    "main",
    "math",
    "max",
    "min",
    "mod",
    "module",
    "mul",
    "ne",
    "neg",
    "not",
    "null",
    "or",
    "prql",
    "rank",
    "rank_dense",
    "read_csv",
    "read_json",
    "read_parquet",
    "regex_search",
    "remove",
    "row_number",
    "select",
    "sort",
    "std",
    "stddev",
    "sub",
    "sum",
    "take",
    "text",
    "that",
    "this",
    "true",
    "tuple_map",
    "tuple_reduce",
    "tuple_reverse",
    "tuple_uniq",
    "tuple_zip",
    "window",
];
