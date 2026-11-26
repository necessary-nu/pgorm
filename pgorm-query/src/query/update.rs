use crate::{
    AnyWithClause, IntoSubQueryStatement, QueryStatementBuilder, Returning, ReturningClause,
    SubQueryStatement, backend::QueryBuilder, expr::*, prepare::*, query::condition::*, types::*,
    value::*,
};
use inherent::inherent;

/// Update existing rows in the table
///
/// # Examples
///
/// ```
/// use pgorm_query::{tests_cfg::*, *};
///
/// let query = Query::update()
///     .table(Glyph::Table)
///     .values([(Glyph::Aspect, 1.23.into()), (Glyph::Image, "123".into())])
///     .and_where(Expr::col(Glyph::Id).eq(1))
///     .to_owned();
///
/// assert_eq!(
///     query.to_string(),
///     r#"UPDATE "glyph" SET "aspect" = 1.23, "image" = '123' WHERE "id" = 1"#
/// );
/// ```
///
/// PostgreSQL admits neither `ORDER BY` nor `LIMIT` on an UPDATE, so the
/// statement carries neither and an ordered update does not typecheck:
///
/// ```compile_fail,E0599
/// use pgorm_query::{tests_cfg::*, *};
///
/// Query::update()
///     .table(Glyph::Table)
///     .value(Glyph::Aspect, 1)
///     .order_by(Glyph::Id, Order::Asc);
/// ```
///
/// nor a limited one:
///
/// ```compile_fail,E0599
/// use pgorm_query::{tests_cfg::*, *};
///
/// Query::update()
///     .table(Glyph::Table)
///     .value(Glyph::Aspect, 1)
///     .limit(1);
/// ```
///
/// Both belong to the SELECT that chooses the rows, so an update over an
/// ordered, limited set is spelled as a subquery filter:
///
/// ```
/// use pgorm_query::{tests_cfg::*, *};
///
/// let query = Query::update()
///     .table(Glyph::Table)
///     .value(Glyph::Aspect, 1)
///     .and_where(Expr::col(Glyph::Id).in_subquery(
///         Query::select()
///             .column(Glyph::Id)
///             .from(Glyph::Table)
///             .order_by(Glyph::Id, Order::Asc)
///             .limit(1)
///             .take(),
///     ))
///     .to_owned();
///
/// assert_eq!(
///     query.to_string(),
///     r#"UPDATE "glyph" SET "aspect" = 1 WHERE "id" IN (SELECT "id" FROM "glyph" ORDER BY "id" ASC LIMIT 1)"#
/// );
/// ```
// [spec:pgorm:req:sql.ast.update+5]
// [spec:pgorm:def:query.build.with+2]
#[derive(Default, Debug, Clone, PartialEq)]
pub struct UpdateStatement {
    pub(crate) with: Option<Box<AnyWithClause>>,
    pub(crate) table: Option<NamedTable>,
    pub(crate) values: Vec<(Name, Box<SimpleExpr>)>,
    pub(crate) from: Vec<FromItem>,
    pub(crate) r#where: ConditionHolder,
    pub(crate) returning: Option<ReturningClause>,
}

impl UpdateStatement {
    /// Construct a new [`UpdateStatement`]
    pub fn new() -> Self {
        Self::default()
    }

    /// Specify which table to update.
    ///
    /// The target is a name, optionally aliased:
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::update()
    ///     .table(Glyph::Table.into_named_table().alias(Name::runtime("g")))
    ///     .value(Glyph::Aspect, 1.23)
    ///     .and_where(Expr::col((Name::runtime("g"), Glyph::Id)).eq(1))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"UPDATE "glyph" AS "g" SET "aspect" = 1.23 WHERE "g"."id" = 1"#
    /// );
    /// ```
    ///
    /// # Examples
    ///
    /// See [`UpdateStatement::values`]
    // [spec:pgorm:req:sql.ast.update+5]
    #[allow(clippy::wrong_self_convention)]
    pub fn table<T>(&mut self, tbl_ref: T) -> &mut Self
    where
        T: IntoNamedTable,
    {
        self.table = Some(tbl_ref.into_named_table());
        self
    }

    /// Update column values. To set multiple column-value pairs at once.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::update()
    ///     .table(Glyph::Table)
    ///     .values([
    ///         (Glyph::Aspect, 2.1345.into()),
    ///         (Glyph::Image, "235m".into()),
    ///     ])
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"UPDATE "glyph" SET "aspect" = 2.1345, "image" = '235m'"#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ast.update+5]
    pub fn values<T, I>(&mut self, values: I) -> &mut Self
    where
        T: IntoName,
        I: IntoIterator<Item = (T, SimpleExpr)>,
    {
        for (k, v) in values.into_iter() {
            self.values.push((k.into_name(), Box::new(v)));
        }
        self
    }

    /// Update column value by [`SimpleExpr`].
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::update()
    ///     .table(Glyph::Table)
    ///     .value(Glyph::Aspect, Expr::raw("60 * 24 * 24"))
    ///     .values([
    ///         (Glyph::Image, "24B0E11951B03B07F8300FD003983F03F0780060".into()),
    ///     ])
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"UPDATE "glyph" SET "aspect" = 60 * 24 * 24, "image" = '24B0E11951B03B07F8300FD003983F03F0780060'"#
    /// );
    /// ```
    pub fn value<C, T>(&mut self, col: C, value: T) -> &mut Self
    where
        C: IntoName,
        T: Into<SimpleExpr>,
    {
        self.values.push((col.into_name(), Box::new(value.into())));
        self
    }

    /// Add a relation to the `FROM` clause, so the assignments and the `WHERE`
    /// can read columns of a table other than the one being updated.
    ///
    /// The join condition belongs in `WHERE`: PostgreSQL's `UPDATE .. FROM`
    /// takes a plain relation list, and a row of the target table is updated
    /// once for every row the `FROM` relations match it with.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::update()
    ///     .table(Char::Table)
    ///     .value(Char::FontSize, Expr::col((Font::Table, Font::Id)))
    ///     .from(Font::Table)
    ///     .and_where(Expr::col((Char::Table, Char::FontId)).equals((Font::Table, Font::Id)))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"UPDATE "character" SET "font_size" = "font"."id" FROM "font" WHERE "character"."font_id" = "font"."id""#
    /// );
    /// ```
    ///
    /// Calling it repeatedly accumulates a comma-separated relation list, as
    /// on a `SELECT`:
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::update()
    ///     .table(Char::Table)
    ///     .value(Char::FontSize, 12)
    ///     .from(Font::Table)
    ///     .from(Glyph::Table)
    ///     .and_where(Expr::col((Char::Table, Char::FontId)).equals((Font::Table, Font::Id)))
    ///     .and_where(Expr::col((Glyph::Table, Glyph::Id)).eq(1))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     [
    ///         r#"UPDATE "character" SET "font_size" = 12 FROM "font", "glyph""#,
    ///         r#"WHERE "character"."font_id" = "font"."id" AND "glyph"."id" = 1"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    ///
    /// It takes the same relation currency a `SELECT`'s `from` takes, so a
    /// subquery, a values list or a validated fragment stands where a table
    /// stands:
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let fragment = SqlTemplate::from_sql(
    ///     r#"SELECT "id", "name" FROM "font" WHERE "language" = $1"#,
    ///     ["en".into()],
    /// )?;
    ///
    /// let (sql, values) = Query::update()
    ///     .table(Char::Table)
    ///     .value(Char::FontSize, 12)
    ///     .from(FromItem::Template(fragment, Name::runtime("f")))
    ///     .and_where(Expr::col((Char::Table, Char::FontId)).equals((Name::runtime("f"), Font::Id)))
    ///     .build();
    ///
    /// assert_eq!(
    ///     sql,
    ///     "UPDATE \"character\" SET \"font_size\" = $1 FROM \
    ///      (SELECT \"id\", \"name\" FROM \"font\" WHERE \"language\" = $2\n\
    ///      ) AS \"f\" WHERE \"character\".\"font_id\" = \"f\".\"id\""
    /// );
    /// assert_eq!(values.0, vec![12i32.into(), "en".into()]);
    /// # Ok::<(), pgorm_query::error::Error>(())
    /// ```
    // [spec:pgorm:req:sql.ast.update+5]
    pub fn from<R>(&mut self, tbl_ref: R) -> &mut Self
    where
        R: IntoFromItem,
    {
        self.from.push(tbl_ref.into_from_item());
        self
    }

    /// RETURNING expressions.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::update()
    ///     .table(Glyph::Table)
    ///     .value(Glyph::Aspect, 2.1345)
    ///     .value(Glyph::Image, "235m")
    ///     .returning(Query::returning().columns([Glyph::Id]))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"UPDATE "glyph" SET "aspect" = 2.1345, "image" = '235m' RETURNING "id""#
    /// );
    /// ```
    pub fn returning(&mut self, returning: ReturningClause) -> &mut Self {
        self.returning = Some(returning);
        self
    }

    /// RETURNING expressions for a column.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::update()
    ///     .table(Glyph::Table)
    ///     .table(Glyph::Table)
    ///     .value(Glyph::Aspect, 2.1345)
    ///     .value(Glyph::Image, "235m")
    ///     .returning_col(Glyph::Id)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"UPDATE "glyph" SET "aspect" = 2.1345, "image" = '235m' RETURNING "id""#
    /// );
    /// ```
    pub fn returning_col<C>(&mut self, col: C) -> &mut Self
    where
        C: IntoColumnRef,
    {
        self.returning(Returning.column(col))
    }

    /// RETURNING expressions all columns.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::update()
    ///     .table(Glyph::Table)
    ///     .table(Glyph::Table)
    ///     .value(Glyph::Aspect, 2.1345)
    ///     .value(Glyph::Image, "235m")
    ///     .returning_all()
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"UPDATE "glyph" SET "aspect" = 2.1345, "image" = '235m' RETURNING *"#
    /// );
    /// ```
    pub fn returning_all(&mut self) -> &mut Self {
        self.returning(Returning.all())
    }

    /// Get column values
    pub fn get_values(&self) -> &[(Name, Box<SimpleExpr>)] {
        &self.values
    }

    /// The accumulated `WHERE` condition, when at least one predicate was
    /// added.
    // [spec:pgorm:sem:exec.crud.update+8]
    pub fn where_condition(&self) -> Option<&Condition> {
        self.r#where
            .contents
            .as_ref()
            .filter(|cond| !cond.conditions.is_empty())
    }
}

#[inherent]
impl QueryStatementBuilder for UpdateStatement {
    pub fn build_collect_into(&self, sql: &mut dyn SqlWriter) {
        QueryBuilder.prepare_update_statement(self, sql);
    }

    pub fn build(&self) -> (String, Values);
    pub fn build_collect(&self, sql: &mut dyn SqlWriter) -> String;
}

#[inherent]
impl IntoSubQueryStatement for UpdateStatement {
    pub fn into_sub_query_statement(self) -> SubQueryStatement {
        SubQueryStatement::UpdateStatement(self)
    }
}

/// Renders every value inlined as an escaped SQL literal rather than bound —
/// good for logging and goldens. [`build`](Self::build) is the rendering to
/// execute: it emits `$N` placeholders and returns the values to bind.
// [spec:pgorm:req:sql.ast.build+3]
impl std::fmt::Display for UpdateStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sql = String::with_capacity(256);
        QueryBuilder.prepare_update_statement(self, &mut sql);
        f.write_str(&sql)
    }
}

#[inherent]
impl ConditionalStatement for UpdateStatement {
    pub fn cond_where<C>(&mut self, condition: C) -> &mut Self
    where
        C: IntoCondition,
    {
        self.r#where.add_condition(condition.into_condition());
        self
    }

    pub fn and_where_option(&mut self, other: Option<SimpleExpr>) -> &mut Self;
    pub fn and_where(&mut self, other: SimpleExpr) -> &mut Self;
}
