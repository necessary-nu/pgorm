//! LIKE, ILIKE and SIMILAR TO patterns: a pattern written as it stands, or
//! one built from literal text whose metacharacters are escaped.

/// A `LIKE`, `ILIKE` or `SIMILAR TO` pattern, with the `ESCAPE` tail the
/// grammar admits only there.
///
/// [`new`](Self::new) takes a pattern as written, its `%` and `_` wildcards
/// and its escapes as they stand. [`literal`](Self::literal),
/// [`containing`](Self::containing), [`starting_with`](Self::starting_with)
/// and [`ending_with`](Self::ending_with) take literal text instead, a user's
/// search included: its `%`, its `_` and the escape character are escaped, so
/// the text matches itself and nothing else, with a `%` after it, before it
/// or on both sides. The entity columns' `contains`, `starts_with` and
/// `ends_with` are these.
///
/// Text is escaped for the pattern's escape character when the statement is
/// rendered, so [`escape`](Self::escape) chosen after the text still escapes
/// it; [`no_escape`](Self::no_escape) leaves nothing escaped, the text's `%`
/// and `_` then wildcards as the caller asked. The escaped set is LIKE's:
/// under `SIMILAR TO` the regular-expression characters (`|`, `*`, `+`, `?`,
/// brackets, braces and parentheses) keep their meaning.
///
/// ```
/// use pgorm_query::{tests_cfg::*, *};
///
/// let query = Query::select()
///     .column(Glyph::Image)
///     .from(Glyph::Table)
///     .and_where(Expr::col(Glyph::Image).like(LikeExpr::containing("50%_off")))
///     .and_where(Expr::col(Glyph::Image).ilike(LikeExpr::starting_with("a!b").escape('!')))
///     .to_owned();
///
/// assert_eq!(
///     query.to_string(),
///     [
///         r#"SELECT "image" FROM "glyph""#,
///         r#"WHERE "image" LIKE E'%50\\%\\_off%'"#,
///         r#"AND "image" ILIKE 'a!!b%' ESCAPE '!'"#,
///     ]
///     .join(" ")
/// );
/// ```
// [spec:pgorm:req:sql.ast.expr.similar-to+3]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LikeExpr {
    pattern: Pattern,
    pub(crate) escape: Option<LikeEscape>,
}

/// What a pattern's `ESCAPE` holds: one character, or nothing, which turns
/// escaping off. PostgreSQL refuses anything longer (`22025`), so nothing
/// longer has a spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LikeEscape {
    Char(char),
    Off,
}

/// A pattern's text: written by the caller, or literal text with a `%` on
/// either side, escaped when it renders.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pattern {
    Written(String),
    Text {
        text: String,
        any_before: bool,
        any_after: bool,
    },
}

pub trait IntoLikeExpr {
    fn into_like_expr(self) -> LikeExpr;
}

impl LikeExpr {
    pub fn new<T>(pattern: T) -> Self
    where
        T: Into<String>,
    {
        Self {
            pattern: Pattern::Written(pattern.into()),
            escape: None,
        }
    }

    /// A pattern matching exactly `text`.
    pub fn literal<T>(text: T) -> Self
    where
        T: Into<String>,
    {
        Self::text(text.into(), false, false)
    }

    /// A pattern matching any text holding `text`: `%text%`.
    pub fn containing<T>(text: T) -> Self
    where
        T: Into<String>,
    {
        Self::text(text.into(), true, true)
    }

    /// A pattern matching any text beginning with `text`: `text%`.
    pub fn starting_with<T>(text: T) -> Self
    where
        T: Into<String>,
    {
        Self::text(text.into(), false, true)
    }

    /// A pattern matching any text ending with `text`: `%text`.
    pub fn ending_with<T>(text: T) -> Self
    where
        T: Into<String>,
    {
        Self::text(text.into(), true, false)
    }

    fn text(text: String, any_before: bool, any_after: bool) -> Self {
        Self {
            pattern: Pattern::Text {
                text,
                any_before,
                any_after,
            },
            escape: None,
        }
    }

    /// Escape with `c`, `ESCAPE 'c'`, in place of the default backslash.
    pub fn escape(self, c: char) -> Self {
        Self {
            pattern: self.pattern,
            escape: Some(LikeEscape::Char(c)),
        }
    }

    /// Turn escaping off, `ESCAPE ''`: every character of the pattern, a
    /// backslash included, is then literal or a wildcard.
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .column(Glyph::Image)
    ///     .from(Glyph::Table)
    ///     .and_where(Expr::col(Glyph::Image).like(LikeExpr::new(r"a\b").no_escape()))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "image" FROM "glyph" WHERE "image" LIKE E'a\\b' ESCAPE ''"#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ast.expr.similar-to+3]
    pub fn no_escape(self) -> Self {
        Self {
            pattern: self.pattern,
            escape: Some(LikeEscape::Off),
        }
    }

    /// The pattern as it is bound: written text as it stands, literal text
    /// with each `%`, `_` and escape character preceded by the escape.
    // [spec:pgorm:req:sql.ast.expr.similar-to+3]
    pub(crate) fn pattern(&self) -> String {
        match &self.pattern {
            Pattern::Written(pattern) => pattern.clone(),
            Pattern::Text {
                text,
                any_before,
                any_after,
            } => {
                let mut pattern = String::with_capacity(text.len() + 2);
                if *any_before {
                    pattern.push('%');
                }
                let escape = match self.escape {
                    None => Some('\\'),
                    Some(LikeEscape::Char(c)) => Some(c),
                    Some(LikeEscape::Off) => None,
                };
                for character in text.chars() {
                    if escape
                        .is_some_and(|e| character == e || character == '%' || character == '_')
                    {
                        pattern.extend(escape);
                    }
                    pattern.push(character);
                }
                if *any_after {
                    pattern.push('%');
                }
                pattern
            }
        }
    }
}

impl IntoLikeExpr for LikeExpr {
    fn into_like_expr(self) -> LikeExpr {
        self
    }
}

impl<T> IntoLikeExpr for T
where
    T: Into<String>,
{
    fn into_like_expr(self) -> LikeExpr {
        LikeExpr::new(self)
    }
}
