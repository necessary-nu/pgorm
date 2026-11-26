//! `CREATE VIEW`, `CREATE RECURSIVE VIEW` and `DROP VIEW`; the materialized
//! view's statements are in [`materialized`].

use crate::{
    DropBehavior, IntoName, IntoTableName, Name, QueryBuilder, SelectStatement, TableName,
    error::Result,
};

mod materialized;

pub(crate) use materialized::RefreshMode;
pub use materialized::{
    MaterializedView, MaterializedViewCreateStatement, MaterializedViewDropStatement,
    MaterializedViewRefreshStatement,
};

/// Helper for constructing any view statement
///
/// A view is a relation, so it is named as a table is: a bare name or a
/// `(schema, name)` pair, each part a quoted identifier. Its body is a
/// [`SelectStatement`], taken by value as every embedder takes what it embeds,
/// refused by the constructor when PostgreSQL's grammar would refuse its tie
/// limit or WINDOW clause, and written with its values inlined: a view's
/// definition is stored as SQL and cannot take bind parameters.
// [spec:pgorm:req:sql.ddl.view+2]
#[derive(Debug, Clone, Copy)]
pub struct View;

/// Take the query a stored definition is written from — a view's, a
/// materialized view's, a `CREATE TABLE AS`'s — refusing a tie limit
/// ([`check_tie_limit`](SelectStatement::check_tie_limit)) and a WINDOW
/// clause, or a reference to one
/// ([`check_windows`](SelectStatement::check_windows)), that PostgreSQL's
/// grammar refuses.
///
/// The constructor is the one boundary every body passes: rendering is
/// infallible, and a DDL statement reaches the server as the text it renders,
/// through no executor that could check it first. The body has no setter, so
/// the query checked here is the one that renders. A `SELECT` nested in it is
/// the server's to refuse, as it is for the two checks on their own.
// [spec:pgorm:req:sql.ddl.view+2]
pub(crate) fn definition_body(body: SelectStatement) -> Result<SelectStatement> {
    body.check_clauses()?;
    Ok(body)
}

impl View {
    /// Construct a [`ViewCreateStatement`] over the view it creates and the
    /// query it stands for.
    ///
    /// # Errors
    ///
    /// The query's tie limit or WINDOW clause, when PostgreSQL's grammar
    /// refuses it: [`Error::TiesWithoutOrder`](crate::error::Error::TiesWithoutOrder),
    /// [`Error::TiesWithSkipLocked`](crate::error::Error::TiesWithSkipLocked)
    /// or one of the window refusals
    /// [`check_windows`](SelectStatement::check_windows) names.
    pub fn create<T>(name: T, body: SelectStatement) -> Result<ViewCreateStatement>
    where
        T: IntoTableName,
    {
        Ok(ViewCreateStatement {
            definition: ViewDefinition::new(
                name.into_table_name(),
                Vec::new(),
                definition_body(body)?,
            ),
            check_option: None,
        })
    }

    /// Construct a [`RecursiveViewCreateStatement`] over the view, its first
    /// column and the query that reads the view by that name.
    ///
    /// The column list is what the recursive query's rows are named by, and
    /// PostgreSQL refuses a recursive view without one (`42601`), so the first
    /// column is an argument and [`column`](RecursiveViewCreateStatement::column)
    /// appends the rest.
    ///
    /// # Errors
    ///
    /// The query's tie limit or WINDOW clause, when PostgreSQL's grammar
    /// refuses it, as for [`create`](Self::create).
    pub fn create_recursive<T, C>(
        name: T,
        column: C,
        body: SelectStatement,
    ) -> Result<RecursiveViewCreateStatement>
    where
        T: IntoTableName,
        C: IntoName,
    {
        Ok(RecursiveViewCreateStatement {
            definition: ViewDefinition::new(
                name.into_table_name(),
                vec![column.into_name()],
                definition_body(body)?,
            ),
        })
    }

    /// Construct a [`ViewDropStatement`] over the first view it drops
    pub fn drop<T>(name: T) -> ViewDropStatement
    where
        T: IntoTableName,
    {
        ViewDropStatement {
            relations: RelationDrop::new(name.into_table_name()),
        }
    }
}

/// What a plain view and a recursive one both carry, in the order they
/// render.
#[derive(Debug, Clone)]
pub(crate) struct ViewDefinition {
    pub(crate) name: TableName,
    pub(crate) columns: Vec<Name>,
    pub(crate) body: SelectStatement,
    pub(crate) or_replace: bool,
    pub(crate) security_barrier: bool,
    pub(crate) security_invoker: bool,
}

impl ViewDefinition {
    fn new(name: TableName, columns: Vec<Name>, body: SelectStatement) -> Self {
        Self {
            name,
            columns,
            body,
            or_replace: false,
            security_barrier: false,
            security_invoker: false,
        }
    }
}

/// Which views an insert or update through this one is checked against:
/// `WITH LOCAL CHECK OPTION` or `WITH CASCADED CHECK OPTION`.
///
/// A row written through a view with a check option must be one the view can
/// see, or the server refuses it (`44000`). `LOCAL` holds the row to this
/// view's own condition alone; `CASCADED` to the condition of every view it
/// reads from as well, whatever their own option says. The bare `WITH CHECK
/// OPTION` is `CASCADED`, so it has no spelling of its own here.
// [spec:pgorm:req:sql.ddl.view+2]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewCheckOption {
    /// `WITH LOCAL CHECK OPTION`: this view's condition alone.
    Local,
    /// `WITH CASCADED CHECK OPTION`: this view's and every underlying view's.
    Cascaded,
}

impl ViewCheckOption {
    /// The word between `WITH` and `CHECK OPTION`.
    pub(crate) fn keyword(self) -> &'static str {
        match self {
            Self::Local => "LOCAL",
            Self::Cascaded => "CASCADED",
        }
    }
}

/// Create a view
///
/// The name and the body are taken by [`View::create`]: `CREATE VIEW  AS` and
/// a view with no query are both syntax errors. The column names are
/// optional, and fewer than the query has rename only the leading columns.
///
/// `IF NOT EXISTS` is not offered, because PostgreSQL has no such clause for a
/// view (`42601`); [`or_replace`](Self::or_replace) is how a view that may
/// exist is defined.
///
/// ```compile_fail,E0599
/// use pgorm_query::*;
///
/// View::create(Name::runtime("v"), Query::select().expr(1).take())?.if_not_exists();
/// # Ok::<(), pgorm_query::error::Error>(())
/// ```
///
/// # Examples
///
/// ```
/// use pgorm_query::*;
///
/// let view = View::create(
///     (Name::runtime("app"), Name::runtime("active_account")),
///     Query::select()
///         .columns([Name::runtime("id"), Name::runtime("email")])
///         .from(Name::runtime("account"))
///         .and_where(Expr::col(Name::runtime("status")).eq("active"))
///         .take(),
/// )?
/// .or_replace()
/// .check_option(ViewCheckOption::Cascaded)
/// .to_owned();
///
/// assert_eq!(
///     view.to_string(),
///     [
///         r#"CREATE OR REPLACE VIEW "app"."active_account" AS"#,
///         r#"SELECT "id", "email" FROM "account" WHERE "status" = 'active'"#,
///         r#"WITH CASCADED CHECK OPTION"#,
///     ]
///     .join(" ")
/// );
/// # Ok::<(), pgorm_query::error::Error>(())
/// ```
// [spec:pgorm:req:sql.ddl.view+2]
#[derive(Debug, Clone)]
pub struct ViewCreateStatement {
    pub(crate) definition: ViewDefinition,
    pub(crate) check_option: Option<ViewCheckOption>,
}

/// Create a recursive view: `CREATE RECURSIVE VIEW "v" ("c", ...) AS ...`,
/// whose query reads the view itself by name.
///
/// PostgreSQL defines it as the view of a `WITH RECURSIVE` query, so the body
/// is the non-recursive term `UNION [ALL]` the recursive one. The column list
/// is never empty, being what the query's rows are called, and the statement
/// has no check option to set: the grammar refuses one on a recursive view
/// (`0A000`, *WITH CHECK OPTION not supported on recursive views*).
///
/// ```compile_fail,E0599
/// use pgorm_query::*;
///
/// View::create_recursive(Name::runtime("v"), Name::runtime("n"), Query::select().expr(1).take())?
///     .check_option(ViewCheckOption::Local);
/// # Ok::<(), pgorm_query::error::Error>(())
/// ```
///
/// # Examples
///
/// ```
/// use pgorm_query::*;
///
/// let n = || Name::runtime("n");
/// let view = View::create_recursive(
///     Name::runtime("counter"),
///     n(),
///     Query::select()
///         .expr(1)
///         .union(
///             UnionType::All,
///             Query::select()
///                 .expr(Expr::col(n()).add(1))
///                 .from(Name::runtime("counter"))
///                 .and_where(Expr::col(n()).lt(3))
///                 .take(),
///         )
///         .take(),
/// )?;
///
/// assert_eq!(
///     view.to_string(),
///     [
///         r#"CREATE RECURSIVE VIEW "counter" ("n") AS"#,
///         r#"SELECT 1 UNION ALL (SELECT "n" + 1 FROM "counter" WHERE "n" < 3)"#,
///     ]
///     .join(" ")
/// );
/// # Ok::<(), pgorm_query::error::Error>(())
/// ```
// [spec:pgorm:req:sql.ddl.view+2]
#[derive(Debug, Clone)]
pub struct RecursiveViewCreateStatement {
    pub(crate) definition: ViewDefinition,
}

/// The methods a plain view and a recursive one share, each on the
/// definition both carry.
macro_rules! view_definition_methods {
    ($statement:ident) => {
        impl $statement {
            /// `OR REPLACE`: define the view anew if it exists.
            ///
            /// The new query must keep the columns the view has, in their
            /// order, with their names and types, and may add columns after
            /// them; anything else the server refuses (`42P16`). The view's
            /// options and check option are replaced by this statement's, so
            /// one it leaves out is reset.
            pub fn or_replace(&mut self) -> &mut Self {
                self.definition.or_replace = true;
                self
            }

            /// Name the next column of the view.
            pub fn column<C>(&mut self, column: C) -> &mut Self
            where
                C: IntoName,
            {
                self.definition.columns.push(column.into_name());
                self
            }

            /// Name the next columns of the view, in order.
            pub fn columns<I, C>(&mut self, columns: I) -> &mut Self
            where
                I: IntoIterator<Item = C>,
                C: IntoName,
            {
                self.definition
                    .columns
                    .extend(columns.into_iter().map(IntoName::into_name));
                self
            }

            /// `security_barrier`: evaluate the view's own conditions before
            /// any condition a query over it supplies, so a function the
            /// caller passes never sees a row the view hides.
            pub fn security_barrier(&mut self) -> &mut Self {
                self.definition.security_barrier = true;
                self
            }

            /// `security_invoker`: check access to the relations the view
            /// reads as the user querying it, rather than as its owner.
            pub fn security_invoker(&mut self) -> &mut Self {
                self.definition.security_invoker = true;
                self
            }

            /// The column names given, in order.
            pub fn get_columns(&self) -> &[Name] {
                &self.definition.columns
            }
        }
    };
}

view_definition_methods!(ViewCreateStatement);
view_definition_methods!(RecursiveViewCreateStatement);

impl ViewCreateStatement {
    /// `WITH LOCAL CHECK OPTION` or `WITH CASCADED CHECK OPTION`: refuse a
    /// row written through the view that the view cannot see (`44000`).
    ///
    /// Only an automatically updatable view takes one — a query over one
    /// table or updatable view, with no aggregate, `DISTINCT`, `GROUP BY`,
    /// `LIMIT` or set operation — which is the server's to judge (`0A000`
    /// otherwise). A later call replaces an earlier one.
    pub fn check_option(&mut self, option: ViewCheckOption) -> &mut Self {
        self.check_option = Some(option);
        self
    }

    /// The check option set, if any
    pub fn get_check_option(&self) -> Option<ViewCheckOption> {
        self.check_option
    }
}

/// The relations a drop names, of which there is at least one, and how it
/// treats what depends on them.
#[derive(Debug, Clone)]
pub(crate) struct RelationDrop {
    pub(crate) first: TableName,
    pub(crate) rest: Vec<TableName>,
    pub(crate) if_exists: bool,
    pub(crate) behavior: Option<DropBehavior>,
}

impl RelationDrop {
    pub(crate) fn new(first: TableName) -> Self {
        Self {
            first,
            rest: Vec::new(),
            if_exists: false,
            behavior: None,
        }
    }
}

/// The methods a view drop and a materialized view drop share.
macro_rules! relation_drop_methods {
    ($statement:ident, $what:literal) => {
        impl $statement {
            #[doc = concat!("Drop a further ", $what)]
            pub fn name<T>(&mut self, name: T) -> &mut Self
            where
                T: crate::IntoTableName,
            {
                self.relations.rest.push(name.into_table_name());
                self
            }

            #[doc = concat!("`IF EXISTS`: a missing ", $what, " is a notice rather than an error.")]
            pub fn if_exists(&mut self) -> &mut Self {
                self.relations.if_exists = true;
                self
            }

            /// `CASCADE`: drop what depends on it too — a view reading it. The
            /// last of this and [`restrict`](Self::restrict) wins.
            pub fn cascade(&mut self) -> &mut Self {
                self.relations.behavior = Some(crate::DropBehavior::Cascade);
                self
            }

            /// `RESTRICT`: refuse the drop while anything depends on it
            /// (`2BP01`), which is also what an unsaid behavior does.
            pub fn restrict(&mut self) -> &mut Self {
                self.relations.behavior = Some(crate::DropBehavior::Restrict);
                self
            }

            #[doc = concat!("The ", $what, "s dropped, in declaration order, of which there is at least one")]
            pub fn names(&self) -> impl Iterator<Item = &crate::TableName> {
                std::iter::once(&self.relations.first).chain(&self.relations.rest)
            }
        }
    };
}

pub(crate) use relation_drop_methods;

/// Drop one or more views
///
/// The first name is taken by [`View::drop`] and every further one is
/// appended, so the list is non-empty: PostgreSQL rejects `DROP VIEW ` at end
/// of input. A materialized view is not a view to this statement, which the
/// server refuses on one (`42809`); [`MaterializedView::drop`] drops it.
///
/// ```
/// use pgorm_query::*;
///
/// assert_eq!(
///     View::drop(Name::runtime("a"))
///         .name((Name::runtime("app"), Name::runtime("b")))
///         .if_exists()
///         .cascade()
///         .to_string(),
///     r#"DROP VIEW IF EXISTS "a", "app"."b" CASCADE"#
/// );
/// ```
// [spec:pgorm:req:sql.ddl.view+2]
#[derive(Debug, Clone)]
pub struct ViewDropStatement {
    pub(crate) relations: RelationDrop,
}

relation_drop_methods!(ViewDropStatement, "view");

/// `Display` for each view statement, each through its own renderer.
macro_rules! impl_view_display {
    ( $( $struct_name:ident => $func_name:ident ),* $(,)? ) => {
        $(
            /// Renders the statement with every value inlined as an SQL
            /// literal. This is its only rendering: a definition is stored as
            /// SQL text and takes no bind parameters, so the values a body's
            /// query carries are written into it as escaped literals.
            // [spec:pgorm:req:sql.ddl+12]
            impl std::fmt::Display for $struct_name {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    let mut sql = String::with_capacity(256);
                    QueryBuilder.$func_name(self, &mut sql);
                    f.write_str(&sql)
                }
            }
        )*
    };
}

pub(crate) use impl_view_display;

impl_view_display!(
    ViewCreateStatement => prepare_view_create_statement,
    RecursiveViewCreateStatement => prepare_recursive_view_create_statement,
    ViewDropStatement => prepare_view_drop_statement,
);
