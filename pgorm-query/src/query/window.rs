use crate::{expr::*, query::*, types::*};
use inherent::inherent;

pub trait OverStatement {
    #[doc(hidden)]
    // Implementation for the trait.
    fn add_partition_by(&mut self, partition: SimpleExpr) -> &mut Self;

    /// Partition by column.
    fn partition_by<T>(&mut self, col: T) -> &mut Self
    where
        T: IntoColumnRef,
    {
        self.add_partition_by(SimpleExpr::Column(col.into_column_ref()))
    }

    /// Partition by vector of columns.
    fn partition_by_columns<I, T>(&mut self, cols: I) -> &mut Self
    where
        T: IntoColumnRef,
        I: IntoIterator<Item = T>,
    {
        cols.into_iter().for_each(|c| {
            self.add_partition_by(SimpleExpr::Column(c.into_column_ref()));
        });
        self
    }
}

/// Window expression
///
/// # Reference
///
/// <https://www.postgresql.org/docs/current/tutorial-window.html>
// [spec:pgorm:def:sql.ast.window-statement+8]
#[derive(Default, Debug, Clone, PartialEq)]
pub struct WindowStatement {
    pub(crate) partition_by: Vec<SimpleExpr>,
    pub(crate) order_by: Vec<OrderExpr>,
    pub(crate) frame: Option<FrameClause>,
}

/// What `OVER` may follow: a function call, or one of SQL/JSON's two
/// aggregates, which are calls in the grammar but carry clauses a
/// [`FunctionCall`](crate::FunctionCall) cannot. Sealed, because the grammar fixes the set.
// [spec:pgorm:def:sql.ast.window-statement+8]
pub trait WindowFunction: Into<SimpleExpr> + sealed::Sealed {}

mod sealed {
    pub trait Sealed {}
    impl Sealed for crate::FunctionCall {}
    impl Sealed for crate::JsonArrayAgg {}
    impl Sealed for crate::JsonObjectAgg {}
}

impl WindowFunction for crate::FunctionCall {}
impl WindowFunction for crate::JsonArrayAgg {}
impl WindowFunction for crate::JsonObjectAgg {}

impl WindowStatement {
    /// Construct a new [`WindowStatement`]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn take(&mut self) -> Self {
        Self {
            partition_by: std::mem::take(&mut self.partition_by),
            order_by: std::mem::take(&mut self.order_by),
            frame: self.frame.take(),
        }
    }

    /// Construct a new [`WindowStatement`] with PARTITION BY column
    pub fn partition_by<T>(col: T) -> Self
    where
        T: IntoColumnRef,
    {
        let mut window = Self::new();
        window.add_partition_by(SimpleExpr::Column(col.into_column_ref()));
        window
    }

    /// Sets the window's frame, replacing any frame already set.
    ///
    /// A frame is begun from its [`FrameType`] and takes its end from the
    /// [`FrameStart`] that returns, so only the frames PostgreSQL's grammar
    /// admits are built; a preceding or current-row start stands alone. See
    /// [`FrameClause`] for the `EXCLUDE` clause.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let query = Query::select()
    ///     .from(Char::Table)
    ///     .expr_window_as(
    ///         Func::count(Expr::col(Char::Id)),
    ///         WindowStatement::partition_by(Char::FontSize)
    ///             .frame(FrameType::Rows.unbounded_preceding())
    ///             .take(),
    ///         Name::runtime("C"))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT COUNT("id") OVER ( PARTITION BY "font_size" ROWS UNBOUNDED PRECEDING ) AS "C" FROM "character""#
    /// );
    ///
    /// let query = Query::select()
    ///     .from(Char::Table)
    ///     .expr_window_as(
    ///         Func::count(Expr::col(Char::Id)),
    ///         WindowStatement::partition_by(Char::FontSize)
    ///             .frame(FrameType::Rows.unbounded_preceding().and_unbounded_following())
    ///             .take(),
    ///         Name::runtime("C"))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT COUNT("id") OVER ( PARTITION BY "font_size" ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING ) AS "C" FROM "character""#
    /// );
    /// ```
    // [spec:pgorm:def:sql.ast.window-statement+8]
    pub fn frame<F>(&mut self, frame: F) -> &mut Self
    where
        F: Into<FrameClause>,
    {
        self.frame = Some(frame.into());
        self
    }
}

impl OverStatement for WindowStatement {
    fn add_partition_by(&mut self, partition: SimpleExpr) -> &mut Self {
        self.partition_by.push(partition);
        self
    }
}

#[inherent]
impl OrderedStatement for WindowStatement {
    pub fn add_order_by(&mut self, order: OrderExpr) -> &mut Self {
        self.order_by.push(order);
        self
    }

    pub fn clear_order_by(&mut self) -> &mut Self {
        self.order_by = Vec::new();
        self
    }

    pub fn order_by<T>(&mut self, col: T, order: Order) -> &mut Self
    where
        T: IntoColumnRef;

    pub fn order_by_expr(&mut self, expr: SimpleExpr, order: Order) -> &mut Self;
    pub fn order_by_columns<I, T>(&mut self, cols: I) -> &mut Self
    where
        T: IntoColumnRef,
        I: IntoIterator<Item = (T, Order)>;
    pub fn order_by_with_nulls<T>(
        &mut self,
        col: T,
        order: Order,
        nulls: NullOrdering,
    ) -> &mut Self
    where
        T: IntoColumnRef;
    pub fn order_by_expr_with_nulls(
        &mut self,
        expr: SimpleExpr,
        order: Order,
        nulls: NullOrdering,
    ) -> &mut Self;
    pub fn order_by_columns_with_nulls<I, T>(&mut self, cols: I) -> &mut Self
    where
        T: IntoColumnRef,
        I: IntoIterator<Item = (T, Order, NullOrdering)>;
}
