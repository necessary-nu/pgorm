use crate::{
    Condition, FromItem, IntoCondition, IntoName, IntoSubQueryStatement, Name, NamedTable,
    Overriding, QueryStatementBuilder, ReturningClause, SimpleExpr, SubQueryStatement, Values,
    WithClause, backend::QueryBuilder, prepare::*,
};
use inherent::inherent;

/// A `MERGE` before its first `WHEN` clause: the table it writes, the relation
/// it reads, and the condition that pairs their rows.
///
/// PostgreSQL refuses a `MERGE` with no `WHEN` clause (`42601`), so this is not
/// yet a statement and has nothing to render or build. Each of its six methods
/// adds the first arm and returns the [`MergeStatement`]:
///
/// ```compile_fail,E0599
/// use pgorm_query::{tests_cfg::*, *};
///
/// Query::merge(
///     Glyph::Table,
///     Font::Table,
///     Expr::col((Glyph::Table, Glyph::Id)).equals((Font::Table, Font::Id)),
/// )
/// .build();
/// ```
// [spec:pgorm:req:sql.ast.merge+2]
#[derive(Debug, Clone, PartialEq)]
pub struct PendingMerge {
    pub(crate) target: NamedTable,
    pub(crate) source: FromItem,
    pub(crate) on: Condition,
}

/// Insert, update or delete rows of a table by pairing them with the rows of
/// a source relation: `MERGE INTO <target> USING <source> ON <condition>`,
/// then its `WHEN` clauses.
///
/// [`Query::merge`](crate::Query::merge) names the three required parts, and
/// the statement exists from its first arm onward, so it always has one:
///
/// ```
/// use pgorm_query::{tests_cfg::*, *};
///
/// let query = Query::merge(
///     Glyph::Table,
///     Font::Table,
///     Expr::col((Glyph::Table, Glyph::Id)).equals((Font::Table, Font::Id)),
/// )
/// .when_matched(MergeUpdate::value(Glyph::Image, Expr::col((Font::Table, Font::Name))))
/// .when_not_matched(
///     MergeInsert::value(Glyph::Id, Expr::col((Font::Table, Font::Id)))
///         .and_value(Glyph::Image, Expr::col((Font::Table, Font::Name))),
/// )
/// .to_owned();
///
/// assert_eq!(
///     query.to_string(),
///     [
///         r#"MERGE INTO "glyph" USING "font" ON "glyph"."id" = "font"."id""#,
///         r#"WHEN MATCHED THEN UPDATE SET "image" = "font"."name""#,
///         r#"WHEN NOT MATCHED THEN INSERT ("id", "image") VALUES ("font"."id", "font"."name")"#,
///     ]
///     .join(" ")
/// );
/// ```
///
/// A target row the condition pairs with a source row is *matched*. A source
/// row it pairs with no target row is *not matched*, and a target row it
/// pairs with no source row is *not matched by source*. Each kind of row takes
/// its own arms, and an arm may carry an `AND` condition of its own. Within a
/// kind, a row takes the first conditional arm whose condition holds, in the
/// order the arms were added. If none holds, it takes that kind's
/// unconditional arm, and with no such arm the row is left alone. An
/// unconditional arm always renders after its kind's conditional arms. That is
/// the only place PostgreSQL accepts one: any arm after it is refused as
/// unreachable (`42601`), so that statement cannot be built. The three kinds
/// never compete for a row, so the order the kinds render in changes nothing.
///
/// The actions follow the grammar. A target row, matched or not matched by
/// source, can be updated, deleted or left alone ([`MatchedAction`]), and a
/// source row with no match can be inserted or skipped
/// ([`NotMatchedAction`]). An insert for a target row does not typecheck:
///
/// ```compile_fail,E0277
/// use pgorm_query::{tests_cfg::*, *};
///
/// Query::merge(
///     Glyph::Table,
///     Font::Table,
///     Expr::col((Glyph::Table, Glyph::Id)).equals((Font::Table, Font::Id)),
/// )
/// .when_matched(MergeInsert::value(Glyph::Id, 1));
/// ```
///
/// and neither does an update or a delete for a row that has no match:
///
/// ```compile_fail,E0277
/// use pgorm_query::{tests_cfg::*, *};
///
/// Query::merge(
///     Glyph::Table,
///     Font::Table,
///     Expr::col((Glyph::Table, Glyph::Id)).equals((Font::Table, Font::Id)),
/// )
/// .when_not_matched(MatchedAction::Delete);
/// ```
///
/// Values in the condition, the assignments and the inserted row are bound
/// like every other value: `build` returns them as parameters. PostgreSQL types
/// such a parameter by the column it is assigned to or compared with. A value
/// in the source relation, such as a `VALUES` row or a function argument, is
/// assigned to no column, so it needs a cast, as it would in any `FROM`
/// clause.
///
/// The statement renders and builds like the four DML statements, and it
/// nests where they do: as a common table expression's body, through
/// [`IntoSubQueryStatement`]. With [`returning`](Self::returning) the CTE
/// yields the rows the merge wrote.
// [spec:pgorm:req:sql.ast.merge+2]
#[derive(Debug, Clone, PartialEq)]
pub struct MergeStatement {
    pub(crate) with: Option<Box<WithClause>>,
    pub(crate) only: bool,
    pub(crate) target: NamedTable,
    pub(crate) source: FromItem,
    pub(crate) on: Condition,
    pub(crate) matched: MergeArms<MatchedAction>,
    pub(crate) not_matched: MergeArms<NotMatchedAction>,
    pub(crate) not_matched_by_source: MergeArms<MatchedAction>,
    pub(crate) returning: Option<ReturningClause>,
    pub(crate) returns_action: bool,
}

/// The arms of one kind: the conditional arms in the order they were added,
/// then at most one unconditional arm, which is where the grammar admits it.
// [spec:pgorm:req:sql.ast.merge+2]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MergeArms<A> {
    pub(crate) conditional: Vec<(Condition, A)>,
    pub(crate) otherwise: Option<A>,
}

impl<A> Default for MergeArms<A> {
    fn default() -> Self {
        Self {
            conditional: Vec::new(),
            otherwise: None,
        }
    }
}

/// What a `MERGE` does with a target row: one the join condition matched, or,
/// in a `NOT MATCHED BY SOURCE` arm, one it matched with no source row.
// [spec:pgorm:req:sql.ast.merge+2]
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum MatchedAction {
    /// `UPDATE SET ..`, built by [`MergeUpdate::value`].
    Update(MergeUpdate),
    /// `DELETE`: remove the target row.
    Delete,
    /// `DO NOTHING`: leave the row as it is. This stops the row from reaching
    /// a later arm.
    DoNothing,
}

/// What a `MERGE` does with a source row that matched no target row.
// [spec:pgorm:req:sql.ast.merge+2]
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum NotMatchedAction {
    /// `INSERT (..) VALUES (..)`, built by [`MergeInsert::value`].
    Insert(MergeInsert),
    /// `INSERT DEFAULT VALUES`: a row of every column's default.
    InsertDefaultValues,
    /// `DO NOTHING`: skip the source row.
    DoNothing,
}

/// The `(column, expression)` pairs of a merge's update or insert. The first
/// pair is given at construction, so the list is never empty, and a column
/// always arrives with its value.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MergeAssignments {
    pub(crate) first: (Name, SimpleExpr),
    pub(crate) rest: Vec<(Name, SimpleExpr)>,
}

impl MergeAssignments {
    fn new(column: Name, value: SimpleExpr) -> Self {
        Self {
            first: (column, value),
            rest: Vec::new(),
        }
    }
}

/// The `UPDATE SET` of a matched arm: one assignment or more.
///
/// The constructor takes the first assignment. The plural form extends an
/// update that already has one, so an empty `SET` cannot be built:
///
/// ```compile_fail,E0599
/// use pgorm_query::{tests_cfg::*, *};
///
/// MergeUpdate::values::<Glyph, _>([]);
/// ```
///
/// A column is a bare name: PostgreSQL resolves it against the target, and
/// refuses one qualified by the table's name (`42703`).
// [spec:pgorm:req:sql.ast.merge+2]
#[derive(Debug, Clone, PartialEq)]
pub struct MergeUpdate {
    pub(crate) sets: MergeAssignments,
}

impl MergeUpdate {
    /// Begin an update whose first assignment sets `column` to `value`.
    pub fn value<C, T>(column: C, value: T) -> Self
    where
        C: IntoName,
        T: Into<SimpleExpr>,
    {
        Self {
            sets: MergeAssignments::new(column.into_name(), value.into()),
        }
    }

    /// Also set `column` to `value`.
    #[must_use]
    pub fn and_value<C, T>(mut self, column: C, value: T) -> Self
    where
        C: IntoName,
        T: Into<SimpleExpr>,
    {
        self.sets.rest.push((column.into_name(), value.into()));
        self
    }

    /// Also apply these `(column, expression)` assignments, in order.
    #[must_use]
    pub fn and_values<C, I>(mut self, values: I) -> Self
    where
        C: IntoName,
        I: IntoIterator<Item = (C, SimpleExpr)>,
    {
        self.sets.rest.extend(
            values
                .into_iter()
                .map(|(column, value)| (column.into_name(), value)),
        );
        self
    }
}

impl From<MergeUpdate> for MatchedAction {
    fn from(update: MergeUpdate) -> Self {
        Self::Update(update)
    }
}

/// The `INSERT` of a not-matched arm: the columns it writes, each paired
/// with the expression it writes there.
///
/// Each column arrives with its value, so the column list and the `VALUES`
/// row always have the same length, and neither can be empty: the
/// constructor takes the first pair. A row of defaults is
/// [`NotMatchedAction::InsertDefaultValues`], which takes no `OVERRIDING`
/// clause. PostgreSQL accepts that clause only before `VALUES`.
// [spec:pgorm:req:sql.ast.merge+2]
#[derive(Debug, Clone, PartialEq)]
pub struct MergeInsert {
    pub(crate) values: MergeAssignments,
    pub(crate) overriding: Option<Overriding>,
}

impl MergeInsert {
    /// Begin an insert whose first column is `column`, written with `value`.
    pub fn value<C, T>(column: C, value: T) -> Self
    where
        C: IntoName,
        T: Into<SimpleExpr>,
    {
        Self {
            values: MergeAssignments::new(column.into_name(), value.into()),
            overriding: None,
        }
    }

    /// Also write `value` into `column`.
    #[must_use]
    pub fn and_value<C, T>(mut self, column: C, value: T) -> Self
    where
        C: IntoName,
        T: Into<SimpleExpr>,
    {
        self.values.rest.push((column.into_name(), value.into()));
        self
    }

    /// Also write these `(column, expression)` pairs, in order.
    #[must_use]
    pub fn and_values<C, I>(mut self, values: I) -> Self
    where
        C: IntoName,
        I: IntoIterator<Item = (C, SimpleExpr)>,
    {
        self.values.rest.extend(
            values
                .into_iter()
                .map(|(column, value)| (column.into_name(), value)),
        );
        self
    }

    /// Change what the inserted row's identity columns do with a supplied
    /// value, as [`InsertStatement::overriding`](crate::InsertStatement::overriding)
    /// does for a plain `INSERT`. The last call wins.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::merge(
    ///     Glyph::Table,
    ///     Font::Table,
    ///     Expr::col((Glyph::Table, Glyph::Id)).equals((Font::Table, Font::Id)),
    /// )
    /// .when_not_matched(
    ///     MergeInsert::value(Glyph::Id, Expr::col((Font::Table, Font::Id)))
    ///         .overriding(Overriding::SystemValue),
    /// )
    /// .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     [
    ///         r#"MERGE INTO "glyph" USING "font" ON "glyph"."id" = "font"."id""#,
    ///         r#"WHEN NOT MATCHED THEN INSERT ("id") OVERRIDING SYSTEM VALUE VALUES ("font"."id")"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    #[must_use]
    pub fn overriding(mut self, overriding: Overriding) -> Self {
        self.overriding = Some(overriding);
        self
    }
}

impl From<MergeInsert> for NotMatchedAction {
    fn from(insert: MergeInsert) -> Self {
        Self::Insert(insert)
    }
}

impl PendingMerge {
    fn into_statement(self) -> MergeStatement {
        MergeStatement {
            with: None,
            only: false,
            target: self.target,
            source: self.source,
            on: self.on,
            matched: MergeArms::default(),
            not_matched: MergeArms::default(),
            not_matched_by_source: MergeArms::default(),
            returning: None,
            returns_action: false,
        }
    }

    /// Begin the statement with the unconditional matched arm. See
    /// [`MergeStatement::when_matched`].
    pub fn when_matched<A>(self, action: A) -> MergeStatement
    where
        A: Into<MatchedAction>,
    {
        let mut statement = self.into_statement();
        statement.matched.otherwise = Some(action.into());
        statement
    }

    /// Begin the statement with a conditional matched arm. See
    /// [`MergeStatement::when_matched_and`].
    pub fn when_matched_and<C, A>(self, condition: C, action: A) -> MergeStatement
    where
        C: IntoCondition,
        A: Into<MatchedAction>,
    {
        let mut statement = self.into_statement();
        statement
            .matched
            .conditional
            .push((condition.into_condition(), action.into()));
        statement
    }

    /// Begin the statement with the unconditional not-matched arm. See
    /// [`MergeStatement::when_not_matched`].
    pub fn when_not_matched<A>(self, action: A) -> MergeStatement
    where
        A: Into<NotMatchedAction>,
    {
        let mut statement = self.into_statement();
        statement.not_matched.otherwise = Some(action.into());
        statement
    }

    /// Begin the statement with a conditional not-matched arm. See
    /// [`MergeStatement::when_not_matched_and`].
    pub fn when_not_matched_and<C, A>(self, condition: C, action: A) -> MergeStatement
    where
        C: IntoCondition,
        A: Into<NotMatchedAction>,
    {
        let mut statement = self.into_statement();
        statement
            .not_matched
            .conditional
            .push((condition.into_condition(), action.into()));
        statement
    }

    /// Begin the statement with the unconditional not-matched-by-source arm.
    /// See [`MergeStatement::when_not_matched_by_source`].
    pub fn when_not_matched_by_source<A>(self, action: A) -> MergeStatement
    where
        A: Into<MatchedAction>,
    {
        let mut statement = self.into_statement();
        statement.not_matched_by_source.otherwise = Some(action.into());
        statement
    }

    /// Begin the statement with a conditional not-matched-by-source arm. See
    /// [`MergeStatement::when_not_matched_by_source_and`].
    pub fn when_not_matched_by_source_and<C, A>(self, condition: C, action: A) -> MergeStatement
    where
        C: IntoCondition,
        A: Into<MatchedAction>,
    {
        let mut statement = self.into_statement();
        statement
            .not_matched_by_source
            .conditional
            .push((condition.into_condition(), action.into()));
        statement
    }
}

impl MergeStatement {
    /// Set the arm a matched row takes when no conditional matched arm took
    /// it: `WHEN MATCHED THEN <action>`.
    ///
    /// The arm renders after every conditional matched arm, whenever it was
    /// added, because PostgreSQL refuses any matched arm after an
    /// unconditional one. A kind holds one unconditional arm, so the last call
    /// wins.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::merge(
    ///     Glyph::Table,
    ///     Font::Table,
    ///     Expr::col((Glyph::Table, Glyph::Id)).equals((Font::Table, Font::Id)),
    /// )
    /// .when_matched(MatchedAction::Delete)
    /// .when_matched_and(Expr::col((Font::Table, Font::Name)).is_null(), MatchedAction::DoNothing)
    /// .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     [
    ///         r#"MERGE INTO "glyph" USING "font" ON "glyph"."id" = "font"."id""#,
    ///         r#"WHEN MATCHED AND "font"."name" IS NULL THEN DO NOTHING"#,
    ///         r#"WHEN MATCHED THEN DELETE"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    pub fn when_matched<A>(&mut self, action: A) -> &mut Self
    where
        A: Into<MatchedAction>,
    {
        self.matched.otherwise = Some(action.into());
        self
    }

    /// Add a matched arm that applies only where `condition` holds:
    /// `WHEN MATCHED AND <condition> THEN <action>`.
    ///
    /// Conditional arms are tried in the order they were added, and a row
    /// takes the first whose condition holds, so the order is part of the
    /// statement's meaning:
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::merge(
    ///     Glyph::Table,
    ///     Font::Table,
    ///     Expr::col((Glyph::Table, Glyph::Id)).equals((Font::Table, Font::Id)),
    /// )
    /// .when_matched_and(Expr::col((Font::Table, Font::Name)).is_null(), MatchedAction::Delete)
    /// .when_matched_and(
    ///     Expr::col((Glyph::Table, Glyph::Image)).is_null(),
    ///     MergeUpdate::value(Glyph::Image, Expr::col((Font::Table, Font::Name))),
    /// )
    /// .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     [
    ///         r#"MERGE INTO "glyph" USING "font" ON "glyph"."id" = "font"."id""#,
    ///         r#"WHEN MATCHED AND "font"."name" IS NULL THEN DELETE"#,
    ///         r#"WHEN MATCHED AND "glyph"."image" IS NULL THEN UPDATE SET "image" = "font"."name""#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    pub fn when_matched_and<C, A>(&mut self, condition: C, action: A) -> &mut Self
    where
        C: IntoCondition,
        A: Into<MatchedAction>,
    {
        self.matched
            .conditional
            .push((condition.into_condition(), action.into()));
        self
    }

    /// Set the arm a source row with no match takes when no conditional
    /// not-matched arm took it: `WHEN NOT MATCHED THEN <action>`. It renders
    /// after every conditional not-matched arm, and the last call wins, as
    /// with [`when_matched`](Self::when_matched).
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::merge(
    ///     Glyph::Table,
    ///     Font::Table,
    ///     Expr::col((Glyph::Table, Glyph::Id)).equals((Font::Table, Font::Id)),
    /// )
    /// .when_not_matched(NotMatchedAction::InsertDefaultValues)
    /// .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     [
    ///         r#"MERGE INTO "glyph" USING "font" ON "glyph"."id" = "font"."id""#,
    ///         r#"WHEN NOT MATCHED THEN INSERT DEFAULT VALUES"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    pub fn when_not_matched<A>(&mut self, action: A) -> &mut Self
    where
        A: Into<NotMatchedAction>,
    {
        self.not_matched.otherwise = Some(action.into());
        self
    }

    /// Add a not-matched arm that applies only where `condition` holds:
    /// `WHEN NOT MATCHED AND <condition> THEN <action>`. The condition can
    /// read only the source row, because there is no target row. PostgreSQL
    /// refuses a reference to the target here (`42P01`). Arms are tried in
    /// the order they were added, as with
    /// [`when_matched_and`](Self::when_matched_and).
    pub fn when_not_matched_and<C, A>(&mut self, condition: C, action: A) -> &mut Self
    where
        C: IntoCondition,
        A: Into<NotMatchedAction>,
    {
        self.not_matched
            .conditional
            .push((condition.into_condition(), action.into()));
        self
    }

    /// Set the arm a target row that no source row matched takes when no
    /// conditional arm of its kind took it: `WHEN NOT MATCHED BY SOURCE THEN
    /// <action>`. Such a row is a target row like a matched one, so it takes
    /// the same actions, a [`MatchedAction`]: update it, delete it, or leave
    /// it alone. An insert has nothing to insert from and does not typecheck:
    ///
    /// ```compile_fail,E0277
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// Query::merge(
    ///     Glyph::Table,
    ///     Font::Table,
    ///     Expr::col((Glyph::Table, Glyph::Id)).equals((Font::Table, Font::Id)),
    /// )
    /// .when_not_matched_by_source(NotMatchedAction::InsertDefaultValues);
    /// ```
    ///
    /// The arm renders after every conditional arm of its kind, and the last
    /// call wins, as with [`when_matched`](Self::when_matched). Its kind
    /// renders after the other two.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::merge(
    ///     Glyph::Table,
    ///     Font::Table,
    ///     Expr::col((Glyph::Table, Glyph::Id)).equals((Font::Table, Font::Id)),
    /// )
    /// .when_not_matched_by_source(MatchedAction::Delete)
    /// .when_matched(MergeUpdate::value(Glyph::Image, Expr::col((Font::Table, Font::Name))))
    /// .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     [
    ///         r#"MERGE INTO "glyph" USING "font" ON "glyph"."id" = "font"."id""#,
    ///         r#"WHEN MATCHED THEN UPDATE SET "image" = "font"."name""#,
    ///         r#"WHEN NOT MATCHED BY SOURCE THEN DELETE"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    ///
    /// The row has no source row, so neither the arm's condition nor its
    /// update can read the source: PostgreSQL refuses a reference to it
    /// (`42P01`).
    pub fn when_not_matched_by_source<A>(&mut self, action: A) -> &mut Self
    where
        A: Into<MatchedAction>,
    {
        self.not_matched_by_source.otherwise = Some(action.into());
        self
    }

    /// Add a not-matched-by-source arm that applies only where `condition`
    /// holds: `WHEN NOT MATCHED BY SOURCE AND <condition> THEN <action>`. The
    /// condition reads the target row alone, as with
    /// [`when_not_matched_by_source`](Self::when_not_matched_by_source), and
    /// arms are tried in the order they were added, as with
    /// [`when_matched_and`](Self::when_matched_and).
    pub fn when_not_matched_by_source_and<C, A>(&mut self, condition: C, action: A) -> &mut Self
    where
        C: IntoCondition,
        A: Into<MatchedAction>,
    {
        self.not_matched_by_source
            .conditional
            .push((condition.into_condition(), action.into()));
        self
    }

    /// Return a row for each row the merge wrote: `RETURNING ..`, after the
    /// arms. The last call wins.
    ///
    /// The list reads the source row and the target row, and the target row's
    /// two versions through [`ReturningRow`](crate::ReturningRow). A column
    /// named bare resolves against both relations, so one both have is
    /// ambiguous (`42702`) until it is qualified, and `*` is the source's
    /// columns followed by the target's. A target column reads the row as the
    /// merge left it, and a deleted row as it was. A version the action did
    /// not produce reads as NULL: an inserted row has no `old`, a deleted row
    /// no `new`. A row an arm left alone with `DO NOTHING` returns nothing.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::merge(
    ///     Glyph::Table,
    ///     Font::Table,
    ///     Expr::col((Glyph::Table, Glyph::Id)).equals((Font::Table, Font::Id)),
    /// )
    /// .when_matched(MergeUpdate::value(Glyph::Image, Expr::col((Font::Table, Font::Name))))
    /// .returning(Query::returning().columns([
    ///     (ReturningRow::Old, Glyph::Image),
    ///     (ReturningRow::New, Glyph::Image),
    /// ]))
    /// .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     [
    ///         r#"MERGE INTO "glyph" USING "font" ON "glyph"."id" = "font"."id""#,
    ///         r#"WHEN MATCHED THEN UPDATE SET "image" = "font"."name""#,
    ///         r#"RETURNING old."image", new."image""#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    pub fn returning(&mut self, returning: ReturningClause) -> &mut Self {
        self.returning = Some(returning);
        self
    }

    /// Also return the action each row took, as the RETURNING list's first
    /// column: `merge_action()`, the text `INSERT`, `UPDATE` or `DELETE`.
    /// With no [`returning`](Self::returning) list, it is the list.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::merge(
    ///     Glyph::Table,
    ///     Font::Table,
    ///     Expr::col((Glyph::Table, Glyph::Id)).equals((Font::Table, Font::Id)),
    /// )
    /// .when_matched(MatchedAction::Delete)
    /// .when_not_matched(MergeInsert::value(Glyph::Id, Expr::col((Font::Table, Font::Id))))
    /// .returning_action()
    /// .returning(Query::returning().column(Glyph::Id))
    /// .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     [
    ///         r#"MERGE INTO "glyph" USING "font" ON "glyph"."id" = "font"."id""#,
    ///         r#"WHEN MATCHED THEN DELETE"#,
    ///         r#"WHEN NOT MATCHED THEN INSERT ("id") VALUES ("font"."id")"#,
    ///         r#"RETURNING merge_action(), "id""#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    ///
    /// This is the only way to write `merge_action()`. PostgreSQL resolves it
    /// in a MERGE's RETURNING list and refuses it anywhere else (`42601`), so
    /// it is not an expression that could be placed elsewhere.
    pub fn returning_action(&mut self) -> &mut Self {
        self.returns_action = true;
        self
    }

    /// Attach a WITH clause, rendered as the statement's prefix. The last
    /// call wins.
    ///
    /// The clause is a plain [`WithClause`], not the recursive form that
    /// [`SelectStatement::with`](crate::SelectStatement::with) also takes,
    /// because PostgreSQL refuses `WITH RECURSIVE` before a `MERGE`
    /// (`42601`). A data-modifying common table expression is accepted.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let fonts = CommonTableExpression::new(
    ///     Name::runtime("f"),
    ///     Query::select().columns([Font::Id, Font::Name]).from(Font::Table).take(),
    /// );
    ///
    /// let query = Query::merge(
    ///     Glyph::Table,
    ///     Name::runtime("f"),
    ///     Expr::col((Glyph::Table, Glyph::Id)).equals((Name::runtime("f"), Font::Id)),
    /// )
    /// .when_matched(MatchedAction::Delete)
    /// .with(WithClause::new(fonts))
    /// .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     [
    ///         r#"WITH "f" AS (SELECT "id", "name" FROM "font")"#,
    ///         r#"MERGE INTO "glyph" USING "f" ON "glyph"."id" = "f"."id""#,
    ///         r#"WHEN MATCHED THEN DELETE"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    ///
    /// The recursive form does not typecheck:
    ///
    /// ```compile_fail,E0308
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let cte = CommonTableExpression::new(
    ///     Name::runtime("f"),
    ///     Query::select().column(Font::Id).from(Font::Table).take(),
    /// );
    ///
    /// Query::merge(
    ///     Glyph::Table,
    ///     Name::runtime("f"),
    ///     Expr::col((Glyph::Table, Glyph::Id)).equals((Name::runtime("f"), Font::Id)),
    /// )
    /// .when_matched(MatchedAction::Delete)
    /// .with(RecursiveWithClause::new(cte));
    /// ```
    // [spec:pgorm:def:query.build.with+2]
    pub fn with(&mut self, clause: WithClause) -> &mut Self {
        self.with = Some(Box::new(clause));
        self
    }

    /// Write the target alone, not the tables that inherit from it:
    /// `MERGE INTO ONLY <target>`. Without it, rows of a child table are
    /// matched, updated and deleted too.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::merge(
    ///     Glyph::Table,
    ///     Font::Table,
    ///     Expr::col((Glyph::Table, Glyph::Id)).equals((Font::Table, Font::Id)),
    /// )
    /// .when_matched(MatchedAction::Delete)
    /// .only()
    /// .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"MERGE INTO ONLY "glyph" USING "font" ON "glyph"."id" = "font"."id" WHEN MATCHED THEN DELETE"#
    /// );
    /// ```
    pub fn only(&mut self) -> &mut Self {
        self.only = true;
        self
    }
}

/// A MERGE nests as a common table expression's body, whose rows are those
/// its RETURNING list yields:
///
/// ```
/// use pgorm_query::{tests_cfg::*, *};
///
/// let merge = Query::merge(
///     Glyph::Table,
///     Font::Table,
///     Expr::col((Glyph::Table, Glyph::Id)).equals((Font::Table, Font::Id)),
/// )
/// .when_matched(MatchedAction::Delete)
/// .returning_action()
/// .returning(Query::returning().column(Glyph::Id))
/// .to_owned();
///
/// let query = Query::select()
///     .column(Asterisk)
///     .from(Name::runtime("m"))
///     .with(WithClause::new(CommonTableExpression::new(Name::runtime("m"), merge)))
///     .to_owned();
///
/// assert_eq!(
///     query.to_string(),
///     [
///         r#"WITH "m" AS (MERGE INTO "glyph" USING "font" ON "glyph"."id" = "font"."id""#,
///         r#"WHEN MATCHED THEN DELETE RETURNING merge_action(), "id")"#,
///         r#"SELECT * FROM "m""#,
///     ]
///     .join(" ")
/// );
/// ```
///
/// PostgreSQL accepts a MERGE without RETURNING there too, as it does an
/// UPDATE: the statement still runs, and only reading the CTE is refused
/// (`0A000`). It does not accept a MERGE as an expression subquery (`42601`),
/// which no typed constructor of one builds.
// [spec:pgorm:req:sql.ast+4]
impl IntoSubQueryStatement for MergeStatement {
    fn into_sub_query_statement(self) -> SubQueryStatement {
        SubQueryStatement::MergeStatement(Box::new(self))
    }
}

#[inherent]
impl QueryStatementBuilder for MergeStatement {
    pub fn build_collect_into(&self, sql: &mut dyn SqlWriter) {
        QueryBuilder.prepare_merge_statement(self, sql);
    }

    pub fn build(&self) -> (String, Values);
    pub fn build_collect(&self, sql: &mut dyn SqlWriter) -> String;
}

/// Renders every value inlined as an escaped SQL literal rather than bound —
/// good for logging and goldens. [`build`](Self::build) is the rendering to
/// execute: it emits `$N` placeholders and returns the values to bind.
// [spec:pgorm:req:sql.ast.build+3]
impl std::fmt::Display for MergeStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sql = String::with_capacity(256);
        QueryBuilder.prepare_merge_statement(self, &mut sql);
        f.write_str(&sql)
    }
}
