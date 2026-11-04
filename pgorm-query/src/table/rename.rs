use crate::{QueryBuilder, types::*};

/// Rename a table
///
/// Both names are taken by the constructor: a rename that names neither end has
/// no PostgreSQL spelling, so it does not construct.
///
/// ```compile_fail,E0061
/// use pgorm_query::{tests_cfg::*, *};
///
/// Table::rename().to_string();
/// ```
///
/// # Examples
///
/// ```
/// use pgorm_query::{tests_cfg::*, *};
///
/// let table = Table::rename(Font::Table, Name::runtime("font_new"));
///
/// assert_eq!(
///     table.to_string(),
///     r#"ALTER TABLE "font" RENAME TO "font_new""#
/// );
/// ```
// [spec:pgorm:req:sql.ddl.drop-rename-truncate+4]
#[derive(Debug, Clone)]
pub struct TableRenameStatement {
    pub(crate) from_name: TableName,
    pub(crate) to_name: Name,
}

impl TableRenameStatement {
    /// Construct rename table statement from the old and new table name.
    ///
    /// The new name is a bare identifier: `RENAME TO` leaves the table in the
    /// schema it is already in, so a qualified target does not construct.
    pub fn new<T, R>(from_name: T, to_name: R) -> Self
    where
        T: IntoTableName,
        R: IntoName,
    {
        Self {
            from_name: from_name.into_table_name(),
            to_name: to_name.into_name(),
        }
    }
}

/// Renders the statement with every value inlined as an escaped SQL literal.
/// This is its only rendering: it exposes no placeholder-emitting build, so
/// nothing here is left to bind.
// [spec:pgorm:req:sql.ddl+12]
impl std::fmt::Display for TableRenameStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sql = String::with_capacity(256);
        QueryBuilder.prepare_table_rename_statement(self, &mut sql);
        f.write_str(&sql)
    }
}

/// Rename a column of an existing table
///
/// PostgreSQL admits `RENAME` only as the sole action of an `ALTER TABLE`, so a
/// column rename is a statement of its own rather than an option that could be
/// listed beside `ADD COLUMN` or `DROP COLUMN`.
///
/// All three names are taken by the constructor: none of them has a spelling the
/// grammar can do without, so a partly-named rename does not construct.
///
/// ```compile_fail,E0061
/// use pgorm_query::{tests_cfg::*, *};
///
/// Table::rename_column().to_string();
/// ```
///
/// # Examples
///
/// ```
/// use pgorm_query::{tests_cfg::*, *};
///
/// let table = Table::rename_column(
///     Font::Table,
///     Name::runtime("new_col"),
///     Name::runtime("new_column"),
/// );
///
/// assert_eq!(
///     table.to_string(),
///     r#"ALTER TABLE "font" RENAME COLUMN "new_col" TO "new_column""#
/// );
/// ```
// [spec:pgorm:req:sql.ddl.alter-table+15]
#[derive(Debug, Clone)]
pub struct ColumnRenameStatement {
    pub(crate) table: TableName,
    pub(crate) from_name: Name,
    pub(crate) to_name: Name,
}

impl ColumnRenameStatement {
    /// Construct rename column statement from the table and the two column names
    pub fn new<T, F, R>(table: T, from_name: F, to_name: R) -> Self
    where
        T: IntoTableName,
        F: IntoName,
        R: IntoName,
    {
        Self {
            table: table.into_table_name(),
            from_name: from_name.into_name(),
            to_name: to_name.into_name(),
        }
    }
}

/// Renders the statement with every value inlined as an escaped SQL literal.
/// This is its only rendering: it exposes no placeholder-emitting build, so
/// nothing here is left to bind.
// [spec:pgorm:req:sql.ddl+12]
impl std::fmt::Display for ColumnRenameStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sql = String::with_capacity(256);
        QueryBuilder.prepare_column_rename_statement(self, &mut sql);
        f.write_str(&sql)
    }
}

/// Rename a constraint of an existing table
///
/// PostgreSQL admits `RENAME` only as the sole action of an `ALTER TABLE`, as
/// it does a column's rename, so a constraint rename is a statement of its own:
/// `ALTER TABLE <table> RENAME CONSTRAINT "from" TO "to"`. A constraint of any
/// kind is renamed this way, PostgreSQL 18's `NOT NULL` among them, and a
/// primary or unique key's index takes the new name with it.
///
/// All three names are taken by the constructor, so a partly-named rename does
/// not construct.
///
/// ```compile_fail,E0061
/// use pgorm_query::{tests_cfg::*, *};
///
/// Table::rename_constraint(Font::Table, Name::runtime("font_pkey")).to_string();
/// ```
///
/// # Examples
///
/// ```
/// use pgorm_query::{tests_cfg::*, *};
///
/// let rename = Table::rename_constraint(
///     Font::Table,
///     Name::runtime("font_pkey"),
///     Name::runtime("font_key"),
/// );
///
/// assert_eq!(
///     rename.to_string(),
///     r#"ALTER TABLE "font" RENAME CONSTRAINT "font_pkey" TO "font_key""#
/// );
/// ```
///
/// What the rename can do is the server's knowledge: a name the table has no
/// constraint under is refused (`42704`), as is a new name another of its
/// constraints holds (`42710`); a constraint the table inherited is renamed
/// only through the parent, whose rename reaches every child's copy (`42P16`
/// on the child).
// [spec:pgorm:req:sql.ddl.alter-table+15]
#[derive(Debug, Clone)]
pub struct ConstraintRenameStatement {
    pub(crate) table: TableName,
    pub(crate) from_name: Name,
    pub(crate) to_name: Name,
}

impl ConstraintRenameStatement {
    /// Construct rename constraint statement from the table and the two
    /// constraint names
    pub fn new<T, F, R>(table: T, from_name: F, to_name: R) -> Self
    where
        T: IntoTableName,
        F: IntoName,
        R: IntoName,
    {
        Self {
            table: table.into_table_name(),
            from_name: from_name.into_name(),
            to_name: to_name.into_name(),
        }
    }
}

/// Renders the statement with every value inlined as an escaped SQL literal.
/// This is its only rendering: it exposes no placeholder-emitting build, so
/// nothing here is left to bind.
// [spec:pgorm:req:sql.ddl+12]
impl std::fmt::Display for ConstraintRenameStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sql = String::with_capacity(256);
        QueryBuilder.prepare_constraint_rename_statement(self, &mut sql);
        f.write_str(&sql)
    }
}
