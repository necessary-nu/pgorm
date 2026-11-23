use crate::{Asterisk, ColumnRef, IntoColumnRef, IntoName, Name, SimpleExpr};

/// RETURNING clause: the rows an INSERT, UPDATE or DELETE yields back, and
/// the names its list reads a written row's two versions by.
///
/// The list is one of three forms: `*`, columns, or expressions, each built
/// by [`Returning`]. A column or expression that names a target column bare
/// reads the row as the statement left it. PostgreSQL 18 also lets the list
/// read the row as it was before the write and as it is after, through the
/// special relations `old` and `new`, which [`ReturningRow`] names:
///
/// ```
/// use pgorm_query::{tests_cfg::*, *};
///
/// let query = Query::update()
///     .table(Glyph::Table)
///     .value(Glyph::Aspect, Expr::col(Glyph::Aspect).add(1))
///     .and_where(Expr::col(Glyph::Id).eq(1))
///     .returning(Query::returning().columns([
///         (ReturningRow::Old, Glyph::Aspect),
///         (ReturningRow::New, Glyph::Aspect),
///     ]))
///     .to_owned();
///
/// assert_eq!(
///     query.to_string(),
///     r#"UPDATE "glyph" SET "aspect" = "aspect" + 1 WHERE "id" = 1 RETURNING old."aspect", new."aspect""#
/// );
/// ```
///
/// [`old_as`](Self::old_as) and [`new_as`](Self::new_as) rename them, with
/// `RETURNING WITH (OLD AS .., NEW AS ..)`. A renamed relation answers to its
/// new name only, so the list then reads it as an ordinary qualifier. An
/// [`alias`](crate::alias) token makes the declaration and every reference
/// one value:
///
/// ```
/// use pgorm_query::{tests_cfg::*, *};
///
/// let (before, after) = (alias("before"), alias("after"));
/// let query = Query::delete()
///     .from_table(Glyph::Table)
///     .and_where(Expr::col(Glyph::Id).eq(1))
///     .returning(
///         Query::returning()
///             .columns([(before, Glyph::Image), (after, Glyph::Image)])
///             .old_as(before)
///             .new_as(after),
///     )
///     .to_owned();
///
/// assert_eq!(
///     query.to_string(),
///     [
///         r#"DELETE FROM "glyph" WHERE "id" = 1"#,
///         r#"RETURNING WITH (OLD AS "before", NEW AS "after") "before"."image", "after"."image""#,
///     ]
///     .join(" ")
/// );
/// ```
// [spec:pgorm:def:sql.ast.returning+4]
#[derive(Clone, Debug, PartialEq)]
pub struct ReturningClause {
    pub(crate) old: Option<Name>,
    pub(crate) new: Option<Name>,
    pub(crate) items: ReturningItems,
}

/// What a RETURNING list returns.
// [spec:pgorm:def:sql.ast.returning+4]
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ReturningItems {
    All,
    Columns(Vec<ColumnRef>),
    Exprs(Vec<SimpleExpr>),
    /// Expressions each returned under a name: `<expr> AS "<name>"`.
    Named(Vec<(SimpleExpr, Name)>),
}

impl ReturningClause {
    fn of(items: ReturningItems) -> Self {
        Self {
            old: None,
            new: None,
            items,
        }
    }

    /// Rename the row as it was before the write: `RETURNING WITH (OLD AS
    /// "name") ..`. The list reads it as `name`, and `old` names nothing
    /// there (`42P01`). The last call wins, so the clause never names `OLD`
    /// twice.
    ///
    /// A rename is how the list reaches the old row when the statement
    /// already has a relation called `old`: the target table, its alias, or
    /// a `FROM` or `USING` item. PostgreSQL resolves `old` to that relation
    /// instead, without complaint. The new name must be unused in the
    /// statement too, but PostgreSQL refuses a clash with that one
    /// (`42712`).
    #[must_use]
    pub fn old_as<N>(mut self, name: N) -> Self
    where
        N: IntoName,
    {
        self.old = Some(name.into_name());
        self
    }

    /// Rename the row as the statement left it: `RETURNING WITH (NEW AS
    /// "name") ..`, on the terms of [`old_as`](Self::old_as).
    #[must_use]
    pub fn new_as<N>(mut self, name: N) -> Self
    where
        N: IntoName,
    {
        self.new = Some(name.into_name());
        self
    }
}

/// Which version of a written row a RETURNING reference reads: as it was
/// before the statement wrote it, or as the statement left it.
///
/// Paired with a column, or with [`Asterisk`], it converts into a
/// [`ColumnRef`] that renders the relation's keyword bare: `old."col"`,
/// `new.*`. PostgreSQL resolves these names in a RETURNING list only.
/// Elsewhere, such as a `SET` value or a `WHERE` condition, `old` names no
/// relation (`42P01`).
///
/// A version the statement did not produce reads as NULL in every column.
/// A plain `INSERT` has no old row, and neither does a row that `ON CONFLICT
/// DO UPDATE` inserted rather than updated. A `DELETE` has no new row.
// [spec:pgorm:def:sql.ast.returning+4]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReturningRow {
    /// `old`: the row before the write.
    Old,
    /// `new`: the row after the write.
    New,
}

impl ReturningRow {
    /// The keyword PostgreSQL names the version by. Quoting it would change
    /// nothing, since `"old"` folds to the same name, but bare it reads as
    /// the relation it is rather than a table of that name.
    pub(crate) fn keyword(self) -> &'static str {
        match self {
            Self::Old => "old",
            Self::New => "new",
        }
    }
}

// [spec:pgorm:def:sql.ast.returning+4]
impl<T: 'static> IntoColumnRef for (ReturningRow, T)
where
    T: IntoName,
{
    fn into_column_ref(self) -> ColumnRef {
        ColumnRef::RowColumn(self.0, self.1.into_name())
    }
}

// [spec:pgorm:def:sql.ast.returning+4]
impl IntoColumnRef for (ReturningRow, Asterisk) {
    fn into_column_ref(self) -> ColumnRef {
        ColumnRef::RowAsterisk(self.0)
    }
}

/// Shorthand for constructing [`ReturningClause`]
// [spec:pgorm:def:sql.ast.returning+4]
#[derive(Clone, Debug, Default)]
pub struct Returning;

impl Returning {
    /// Constructs a new [`Returning`].
    pub fn new() -> Self {
        Self
    }

    /// Return every column of the target: `RETURNING *`.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::delete()
    ///     .from_table(Character::Table)
    ///     .and_where(Expr::col(Character::Id).eq(1))
    ///     .returning(Query::returning().all())
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"DELETE FROM "character" WHERE "id" = 1 RETURNING *"#
    /// );
    /// ```
    pub fn all(&self) -> ReturningClause {
        ReturningClause::of(ReturningItems::All)
    }

    /// Return one column.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::delete()
    ///     .from_table(Character::Table)
    ///     .and_where(Expr::col(Character::Id).eq(1))
    ///     .returning(Query::returning().column(Character::Id))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"DELETE FROM "character" WHERE "id" = 1 RETURNING "id""#
    /// );
    /// ```
    pub fn column<C>(&self, col: C) -> ReturningClause
    where
        C: IntoColumnRef,
    {
        ReturningClause::of(ReturningItems::Columns(vec![col.into_column_ref()]))
    }

    /// Return these columns, in order.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::delete()
    ///     .from_table(Character::Table)
    ///     .and_where(Expr::col(Character::Id).eq(1))
    ///     .returning(Query::returning().columns([Character::Id, Character::Character]))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"DELETE FROM "character" WHERE "id" = 1 RETURNING "id", "character""#
    /// );
    /// ```
    pub fn columns<T, I>(self, cols: I) -> ReturningClause
    where
        T: IntoColumnRef,
        I: IntoIterator<Item = T>,
    {
        let cols: Vec<_> = cols.into_iter().map(|c| c.into_column_ref()).collect();
        ReturningClause::of(ReturningItems::Columns(cols))
    }

    /// Return one expression.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::delete()
    ///     .from_table(Character::Table)
    ///     .and_where(Expr::col(Character::Id).eq(1))
    ///     .returning(Query::returning().expr(Expr::col(Character::Id)))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"DELETE FROM "character" WHERE "id" = 1 RETURNING "id""#
    /// );
    /// ```
    pub fn expr<T>(&self, expr: T) -> ReturningClause
    where
        T: Into<SimpleExpr>,
    {
        ReturningClause::of(ReturningItems::Exprs(vec![expr.into()]))
    }

    /// Return these expressions, in order.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::delete()
    ///     .from_table(Character::Table)
    ///     .and_where(Expr::col(Character::Id).eq(1))
    ///     .returning(Query::returning().exprs([Expr::col(Character::Id), Expr::col(Character::Character)]))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"DELETE FROM "character" WHERE "id" = 1 RETURNING "id", "character""#
    /// );
    /// ```
    pub fn exprs<T, I>(self, exprs: I) -> ReturningClause
    where
        T: Into<SimpleExpr>,
        I: IntoIterator<Item = T>,
    {
        ReturningClause::of(ReturningItems::Exprs(
            exprs.into_iter().map(Into::into).collect(),
        ))
    }

    /// Return one expression under a name: `RETURNING <expr> AS "<name>"`.
    ///
    /// The name is the result column's. It is how a list reading both of a
    /// written row's versions tells them apart, `old."v"` and `new."v"` both
    /// being called `v` otherwise:
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::update()
    ///     .table(Glyph::Table)
    ///     .value(Glyph::Aspect, Expr::col(Glyph::Aspect).add(1))
    ///     .and_where(Expr::col(Glyph::Id).eq(1))
    ///     .returning(Query::returning().exprs_as([
    ///         (Expr::col((ReturningRow::Old, Glyph::Aspect)), alias("before")),
    ///         (Expr::col((ReturningRow::New, Glyph::Aspect)), alias("after")),
    ///     ]))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     [
    ///         r#"UPDATE "glyph" SET "aspect" = "aspect" + 1 WHERE "id" = 1"#,
    ///         r#"RETURNING old."aspect" AS "before", new."aspect" AS "after""#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    pub fn expr_as<T, N>(&self, expr: T, name: N) -> ReturningClause
    where
        T: Into<SimpleExpr>,
        N: IntoName,
    {
        ReturningClause::of(ReturningItems::Named(vec![(expr.into(), name.into_name())]))
    }

    /// Return these expressions, in order, each under the name paired with
    /// it, on the terms of [`expr_as`](Self::expr_as).
    pub fn exprs_as<T, N, I>(self, items: I) -> ReturningClause
    where
        T: Into<SimpleExpr>,
        N: IntoName,
        I: IntoIterator<Item = (T, N)>,
    {
        let items = items
            .into_iter()
            .map(|(expr, name)| (expr.into(), name.into_name()))
            .collect();
        ReturningClause::of(ReturningItems::Named(items))
    }
}
