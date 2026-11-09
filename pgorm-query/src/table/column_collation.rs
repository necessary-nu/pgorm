//! A column's collation. `ColumnDef`'s file is over its function cap, so the
//! setter and its reader sit beside it here.

use crate::types::{Collation, IntoCollation};

use super::ColumnDef;

impl ColumnDef {
    /// Store and compare this column's text under the named collation:
    /// `"title" text COLLATE "C"`.
    ///
    /// The clause is written directly after the type, and a second call
    /// replaces the first rather than adding one, because PostgreSQL refuses a
    /// column with two `COLLATE` clauses. Every comparison and sort the column
    /// takes part in then uses it unless an expression names another — see
    /// [`ExprTrait::collate`](crate::ExprTrait::collate).
    ///
    /// In [`modify_column`](crate::TableAlterStatement::modify_column) the
    /// collation rides on the retype, `ALTER COLUMN "c" TYPE <type> COLLATE
    /// "name"`, which is the only way PostgreSQL changes a column's collation;
    /// a modified column with no type has nowhere to put one.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let table = Table::create(Glyph::Table)
    ///     .col(
    ///         ColumnDef::new(Glyph::Image)
    ///             .text()
    ///             .collate(Name::runtime("C"))
    ///             .not_null(),
    ///     )
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     table.to_string(),
    ///     r#"CREATE TABLE "glyph" ( "image" text COLLATE "C" NOT NULL )"#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.column-def+13]
    pub fn collate<C>(&mut self, collation: C) -> &mut Self
    where
        C: IntoCollation,
    {
        self.collation = Some(collation.into_collation());
        self
    }

    /// The collation the column was declared with, if any.
    pub fn get_collation(&self) -> Option<&Collation> {
        self.collation.as_ref()
    }
}
