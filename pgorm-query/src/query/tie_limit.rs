//! `FETCH FIRST n ROWS WITH TIES`: a row limit that keeps every row tied with
//! the last one it counts, and the check that refuses the two places
//! PostgreSQL's grammar will not take one.

use crate::{
    LockBehavior, SelectStatement,
    error::{Error, Result},
};

/// A SELECT's row limit. The two forms share one slot because PostgreSQL's
/// grammar takes a `LIMIT` or a `FETCH FIRST`, never both, so setting either
/// replaces the other.
// [spec:pgorm:req:sql.ast.select.tie-limit+2]
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RowLimit {
    /// `LIMIT n`: at most `n` rows.
    Count(u64),
    /// `FETCH FIRST n ROWS WITH TIES`: the first `n` rows and every further
    /// row equal to the `n`-th on the ORDER BY keys.
    WithTies(u64),
}

impl SelectStatement {
    /// Limit the result to its first `count` rows *and every row tied with the
    /// last of them* on the ORDER BY keys: `FETCH FIRST $n ROWS WITH TIES`.
    /// It replaces any row limit already set, a plain [`limit`](Self::limit)
    /// included, and [`reset_limit`](Self::reset_limit) removes it.
    ///
    /// The ties are measured on the statement's ORDER BY, so a tie limit
    /// needs one, and PostgreSQL's grammar refuses `WITH TIES` beside
    /// `SKIP LOCKED`. Both are checked by
    /// [`check_tie_limit`](Self::check_tie_limit) rather than at this call,
    /// because a statement is built in any order and its ORDER BY can be
    /// added after the limit (or cleared before it renders).
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .column(Glyph::Image)
    ///     .from(Glyph::Table)
    ///     .order_by(Glyph::Aspect, Order::Desc)
    ///     .limit_with_ties(3)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "image" FROM "glyph" ORDER BY "aspect" DESC FETCH FIRST 3 ROWS WITH TIES"#
    /// );
    /// assert_eq!(query.check_tie_limit(), Ok(()));
    /// ```
    // [spec:pgorm:req:sql.ast.select.tie-limit+2]
    pub fn limit_with_ties(&mut self, count: u64) -> &mut Self {
        self.limit = Some(RowLimit::WithTies(count));
        self
    }

    /// Whether the row limit is a tie limit, which counts rows only up to
    /// the ties of the last: a caller that pages or narrows the result by
    /// row count reads this before replacing the limit.
    // [spec:pgorm:req:sql.ast.select.tie-limit+2]
    pub fn has_tie_limit(&self) -> bool {
        matches!(self.limit, Some(RowLimit::WithTies(_)))
    }

    /// Narrow the row limit to at most one row, for a reader that asks for
    /// the first: `LIMIT 1`, or `LIMIT 0` where the limit already counts no
    /// row. A tie limit narrows the same way, to a plain `LIMIT`: the first
    /// row it returns is the first row of the order, and one of zero returns
    /// none. OFFSET is kept.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let mut query = Query::select();
    /// query.column(Glyph::Image).from(Glyph::Table);
    /// assert_eq!(
    ///     query.clone().narrow_limit_to_one().to_string(),
    ///     r#"SELECT "image" FROM "glyph" LIMIT 1"#
    /// );
    ///
    /// query.order_by(Glyph::Aspect, Order::Desc).limit_with_ties(3);
    /// assert_eq!(
    ///     query.clone().narrow_limit_to_one().to_string(),
    ///     r#"SELECT "image" FROM "glyph" ORDER BY "aspect" DESC LIMIT 1"#
    /// );
    ///
    /// query.limit(0);
    /// assert_eq!(
    ///     query.narrow_limit_to_one().to_string(),
    ///     r#"SELECT "image" FROM "glyph" ORDER BY "aspect" DESC LIMIT 0"#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ast.select.tie-limit+2]
    pub fn narrow_limit_to_one(&mut self) -> &mut Self {
        let none = matches!(self.limit, Some(RowLimit::Count(0) | RowLimit::WithTies(0)));
        self.limit = Some(RowLimit::Count(if none { 0 } else { 1 }));
        self
    }

    /// Refuse a tie limit PostgreSQL's grammar would refuse (`42601`): one on
    /// a statement with no ORDER BY to measure its ties on
    /// ([`Error::TiesWithoutOrder`]), and one beside `SKIP LOCKED`
    /// ([`Error::TiesWithSkipLocked`]). A statement without a tie limit
    /// always passes.
    ///
    /// Rendering is infallible, so it writes what the statement holds and
    /// leaves the refusal to this check; pgorm's executors and the Python
    /// and JavaScript bindings call it before they render. It reads this
    /// statement's own clauses only: a SELECT nested in it is checked by the
    /// server.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, error::Error, tests_cfg::*};
    ///
    /// let mut query = Query::select();
    /// query.column(Glyph::Image).from(Glyph::Table).limit_with_ties(3);
    /// assert_eq!(query.check_tie_limit(), Err(Error::TiesWithoutOrder));
    ///
    /// query.order_by(Glyph::Aspect, Order::Asc);
    /// assert_eq!(query.check_tie_limit(), Ok(()));
    ///
    /// query.lock_with_behavior(LockType::Update, LockBehavior::SkipLocked);
    /// assert_eq!(query.check_tie_limit(), Err(Error::TiesWithSkipLocked));
    /// ```
    // [spec:pgorm:req:sql.ast.select.tie-limit+2]
    pub fn check_tie_limit(&self) -> Result<()> {
        if !self.has_tie_limit() {
            Ok(())
        } else if self.orders.is_empty() {
            Err(Error::TiesWithoutOrder)
        } else if self
            .lock
            .as_ref()
            .is_some_and(|lock| lock.behavior == Some(LockBehavior::SkipLocked))
        {
            Err(Error::TiesWithSkipLocked)
        } else {
            Ok(())
        }
    }

    /// Both checks of this statement's own clauses that rendering leaves to
    /// its caller: [`check_tie_limit`](Self::check_tie_limit) first and
    /// [`check_windows`](Self::check_windows) second, the first refusal found
    /// the error. It is the check for whoever is about to render a `SELECT`
    /// as it stands — pgorm's executors, the Python and JavaScript bindings,
    /// a stored definition's body, a COPY TO's source — so every one reports
    /// the same refusal for the same statement.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, error::Error, tests_cfg::*};
    ///
    /// let mut query = Query::select();
    /// query
    ///     .expr_window_name(Func::count(Expr::col(Char::Id)), Name::runtime("w"))
    ///     .from(Char::Table)
    ///     .limit_with_ties(3);
    /// assert_eq!(query.check_clauses(), Err(Error::TiesWithoutOrder));
    ///
    /// query.order_by(Char::Id, Order::Asc);
    /// assert_eq!(
    ///     query.check_clauses(),
    ///     Err(Error::WindowUndefined { name: "w".to_owned() })
    /// );
    ///
    /// query.window(Name::runtime("w"), WindowStatement::partition_by(Char::FontSize));
    /// assert_eq!(query.check_clauses(), Ok(()));
    /// ```
    // [spec:pgorm:req:sql.ast.select.tie-limit+2]
    // [spec:pgorm:req:sql.ddl.view+1]
    // [spec:pgorm:def:sql.ast.copy+3]
    pub fn check_clauses(&self) -> Result<()> {
        self.check_tie_limit()?;
        self.check_windows()
    }
}
