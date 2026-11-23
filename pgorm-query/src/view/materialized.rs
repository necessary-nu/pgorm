//! `CREATE MATERIALIZED VIEW`, `REFRESH MATERIALIZED VIEW` and `DROP
//! MATERIALIZED VIEW`.

use crate::{
    IntoName, IntoTableName, Name, QueryBuilder, SelectStatement, TableName, error::Result,
};

use super::{RelationDrop, definition_body, impl_view_display, relation_drop_methods};

/// Helper for constructing any materialized view statement
///
/// A materialized view is a relation holding the rows its query returned when
/// it was last populated, named as a table is. Its body is a
/// [`SelectStatement`] checked as it is taken and written with its values
/// inlined, as a view's is.
// [spec:pgorm:req:sql.ddl.materialized-view+2]
#[derive(Debug, Clone, Copy)]
pub struct MaterializedView;

impl MaterializedView {
    /// Construct a [`MaterializedViewCreateStatement`] over the materialized
    /// view it creates and the query it is populated from.
    ///
    /// # Errors
    ///
    /// The query's tie limit or WINDOW clause, when PostgreSQL's grammar
    /// refuses it, as for [`View::create`](crate::View::create).
    pub fn create<T>(name: T, body: SelectStatement) -> Result<MaterializedViewCreateStatement>
    where
        T: IntoTableName,
    {
        Ok(MaterializedViewCreateStatement {
            name: name.into_table_name(),
            columns: Vec::new(),
            body: definition_body(body)?,
            if_not_exists: false,
            no_data: false,
        })
    }

    /// Construct a [`MaterializedViewRefreshStatement`] over the materialized
    /// view it repopulates
    pub fn refresh<T>(name: T) -> MaterializedViewRefreshStatement
    where
        T: IntoTableName,
    {
        MaterializedViewRefreshStatement {
            name: name.into_table_name(),
            mode: None,
        }
    }

    /// Construct a [`MaterializedViewDropStatement`] over the first
    /// materialized view it drops
    pub fn drop<T>(name: T) -> MaterializedViewDropStatement
    where
        T: IntoTableName,
    {
        MaterializedViewDropStatement {
            relations: RelationDrop::new(name.into_table_name()),
        }
    }
}

/// Create a materialized view
///
/// The name and the query are taken by [`MaterializedView::create`]. The
/// query runs when the statement does and its rows are stored, unless
/// [`with_no_data`](Self::with_no_data) leaves the view unpopulated, which the
/// server refuses to read (`55000`) until it is refreshed.
///
/// PostgreSQL 19 has no `OR REPLACE` for a materialized view (`42601`), so
/// none is offered: one that changes is dropped and created again.
///
/// ```compile_fail,E0599
/// use pgorm_query::*;
///
/// MaterializedView::create(Name::runtime("m"), Query::select().expr(1).take())?.or_replace();
/// # Ok::<(), pgorm_query::error::Error>(())
/// ```
///
/// # Examples
///
/// ```
/// use pgorm_query::*;
///
/// let create = MaterializedView::create(
///     Name::runtime("daily_total"),
///     Query::select()
///         .column(Name::runtime("day"))
///         .expr(Func::sum(Expr::col(Name::runtime("amount"))))
///         .from(Name::runtime("payment"))
///         .group_by_col(Name::runtime("day"))
///         .take(),
/// )?
/// .if_not_exists()
/// .columns([Name::runtime("day"), Name::runtime("total")])
/// .with_no_data()
/// .to_owned();
///
/// assert_eq!(
///     create.to_string(),
///     [
///         r#"CREATE MATERIALIZED VIEW IF NOT EXISTS "daily_total" ("day", "total") AS"#,
///         r#"SELECT "day", SUM("amount") FROM "payment" GROUP BY "day""#,
///         r#"WITH NO DATA"#,
///     ]
///     .join(" ")
/// );
/// # Ok::<(), pgorm_query::error::Error>(())
/// ```
// [spec:pgorm:req:sql.ddl.materialized-view+2]
#[derive(Debug, Clone)]
pub struct MaterializedViewCreateStatement {
    pub(crate) name: TableName,
    pub(crate) columns: Vec<Name>,
    pub(crate) body: SelectStatement,
    pub(crate) if_not_exists: bool,
    pub(crate) no_data: bool,
}

impl MaterializedViewCreateStatement {
    /// `IF NOT EXISTS`: leave an existing relation of this name alone, and
    /// its definition and rows with it.
    pub fn if_not_exists(&mut self) -> &mut Self {
        self.if_not_exists = true;
        self
    }

    /// Name the next column of the view.
    pub fn column<C>(&mut self, column: C) -> &mut Self
    where
        C: IntoName,
    {
        self.columns.push(column.into_name());
        self
    }

    /// Name the next columns of the view, in order.
    pub fn columns<I, C>(&mut self, columns: I) -> &mut Self
    where
        I: IntoIterator<Item = C>,
        C: IntoName,
    {
        self.columns
            .extend(columns.into_iter().map(IntoName::into_name));
        self
    }

    /// `WITH NO DATA`: create the view without running its query, leaving it
    /// unpopulated until it is refreshed.
    pub fn with_no_data(&mut self) -> &mut Self {
        self.no_data = true;
        self
    }

    /// The column names given, in order.
    pub fn get_columns(&self) -> &[Name] {
        &self.columns
    }
}

/// How a refresh repopulates its view. One slot holds it, because PostgreSQL
/// refuses `CONCURRENTLY` beside `WITH NO DATA` (`42601`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefreshMode {
    Concurrently,
    NoData,
}

/// Repopulate a materialized view: `REFRESH MATERIALIZED VIEW
/// [CONCURRENTLY ]"m"[ WITH NO DATA]`.
///
/// Unqualified, the refresh runs the view's query and replaces its rows under
/// a lock that blocks reads until it commits. [`concurrently`](Self::concurrently)
/// replaces them without blocking reads, and
/// [`with_no_data`](Self::with_no_data) empties the view and leaves it
/// unpopulated. The two are one choice — the last call wins — because
/// PostgreSQL refuses them together (`42601`).
///
/// A concurrent refresh needs what the statement cannot see, and the server
/// refuses it without: a unique index on the view over plain columns with no
/// `WHERE` (`55000`), and a view already populated (`0A000`). It runs inside a
/// transaction block, unlike `CREATE INDEX CONCURRENTLY`.
///
/// ```
/// use pgorm_query::*;
///
/// assert_eq!(
///     MaterializedView::refresh((Name::runtime("report"), Name::runtime("daily_total")))
///         .with_no_data()
///         .concurrently()
///         .to_string(),
///     r#"REFRESH MATERIALIZED VIEW CONCURRENTLY "report"."daily_total""#
/// );
/// ```
// [spec:pgorm:req:sql.ddl.materialized-view+2]
#[derive(Debug, Clone)]
pub struct MaterializedViewRefreshStatement {
    pub(crate) name: TableName,
    pub(crate) mode: Option<RefreshMode>,
}

impl MaterializedViewRefreshStatement {
    /// `CONCURRENTLY`: replace the rows without blocking reads, replacing a
    /// [`with_no_data`](Self::with_no_data) already given.
    pub fn concurrently(&mut self) -> &mut Self {
        self.mode = Some(RefreshMode::Concurrently);
        self
    }

    /// `WITH NO DATA`: empty the view and leave it unpopulated, replacing a
    /// [`concurrently`](Self::concurrently) already given.
    pub fn with_no_data(&mut self) -> &mut Self {
        self.mode = Some(RefreshMode::NoData);
        self
    }

    /// Whether the refresh is `CONCURRENTLY`
    pub fn is_concurrently(&self) -> bool {
        self.mode == Some(RefreshMode::Concurrently)
    }

    /// Whether the refresh is `WITH NO DATA`
    pub fn is_with_no_data(&self) -> bool {
        self.mode == Some(RefreshMode::NoData)
    }
}

/// Drop one or more materialized views
///
/// The first name is taken by [`MaterializedView::drop`] and every further
/// one is appended, so the list is non-empty. A view is not a materialized
/// view to this statement, which the server refuses on one (`42809`).
///
/// ```
/// use pgorm_query::*;
///
/// assert_eq!(
///     MaterializedView::drop(Name::runtime("daily_total"))
///         .if_exists()
///         .restrict()
///         .to_string(),
///     r#"DROP MATERIALIZED VIEW IF EXISTS "daily_total" RESTRICT"#
/// );
/// ```
// [spec:pgorm:req:sql.ddl.materialized-view+2]
#[derive(Debug, Clone)]
pub struct MaterializedViewDropStatement {
    pub(crate) relations: RelationDrop,
}

relation_drop_methods!(MaterializedViewDropStatement, "materialized view");

impl_view_display!(
    MaterializedViewCreateStatement => prepare_materialized_view_create_statement,
    MaterializedViewRefreshStatement => prepare_materialized_view_refresh_statement,
    MaterializedViewDropStatement => prepare_materialized_view_drop_statement,
);
