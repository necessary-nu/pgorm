//! A column's `NOT NULL` constraint: plain, named, or kept from child tables.
//! `ColumnDef`'s file is over its function cap, so the setters sit beside it
//! here.

use crate::types::{IntoName, Name};

use super::{ColumnDef, ColumnSpec};

impl ColumnDef {
    /// Refuse nulls in this column: `NOT NULL`.
    ///
    /// A column has one `NOT NULL` constraint, which PostgreSQL 18 keeps in its
    /// catalog under a name it derives (`<table>_<column>_not_null`) unless
    /// the column names it. So this and its two companions,
    /// [`not_null_named`](Self::not_null_named) and
    /// [`not_null_no_inherit`](Self::not_null_no_inherit), all set that one
    /// constraint: the first of them puts it among the column's clauses, in the
    /// order they were called, and the rest change it where it stands. Calling
    /// this on a column that already refuses nulls leaves the column as it
    /// was.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Table::create(Glyph::Table)
    ///         .col(ColumnDef::new(Glyph::Id).integer().not_null().default(1).not_null())
    ///         .to_string(),
    ///     r#"CREATE TABLE "glyph" ( "id" integer NOT NULL DEFAULT 1 )"#,
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.column-def+14]
    pub fn not_null(&mut self) -> &mut Self {
        self.update_not_null(|_, _| {});
        self
    }

    /// Name the column's `NOT NULL` constraint: `CONSTRAINT "name" NOT NULL`.
    ///
    /// The name is the one `pg_constraint` records, so the constraint can be
    /// renamed, dropped or altered by it later
    /// ([`TableAlterStatement::alter_constraint`](crate::TableAlterStatement::alter_constraint)).
    /// A column refuses nulls once however it says so, and PostgreSQL refuses
    /// a column that gives that constraint two names, so a later call renames
    /// it rather than adding a second.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Table::create(Glyph::Table)
    ///         .col(
    ///             ColumnDef::new(Glyph::Id)
    ///                 .integer()
    ///                 .not_null()
    ///                 .default(1)
    ///                 .not_null_named(Name::runtime("glyph_id_present"))
    ///         )
    ///         .to_string(),
    ///     r#"CREATE TABLE "glyph" ( "id" integer CONSTRAINT "glyph_id_present" NOT NULL DEFAULT 1 )"#,
    /// );
    /// ```
    ///
    /// Without this the server derives the name, so the plain
    /// [`not_null`](Self::not_null) is the spelling of an unnamed constraint
    /// and this one always carries a name.
    // [spec:pgorm:req:sql.ddl.column-def+14]
    pub fn not_null_named<N>(&mut self, name: N) -> &mut Self
    where
        N: IntoName,
    {
        let name = name.into_name();
        self.update_not_null(|named, _| *named = Some(name));
        self
    }

    /// Keep the column's `NOT NULL` constraint from the tables that inherit
    /// this one: `NOT NULL NO INHERIT`.
    ///
    /// A child table created `INHERITS` this one does not take the constraint,
    /// as it otherwise would. A partitioned table's constraint always reaches
    /// its partitions, so the server refuses this on one (`0A000`); on a
    /// partition itself, or on a table that is not partitioned, it is
    /// accepted.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Table::create(Glyph::Table)
    ///         .col(
    ///             ColumnDef::new(Glyph::Id)
    ///                 .integer()
    ///                 .not_null_no_inherit()
    ///                 .not_null_named(Name::runtime("own_id"))
    ///         )
    ///         .col(ColumnDef::new(Glyph::Aspect).integer().not_null_no_inherit())
    ///         .to_string(),
    ///     [
    ///         r#"CREATE TABLE "glyph" ( "id" integer CONSTRAINT "own_id" NOT NULL NO INHERIT,"#,
    ///         r#""aspect" integer NOT NULL NO INHERIT )"#,
    ///     ]
    ///     .join(" "),
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.column-def+14]
    pub fn not_null_no_inherit(&mut self) -> &mut Self {
        self.update_not_null(|_, no_inherit| *no_inherit = true);
        self
    }

    /// Apply `update` to the column's one `NOT NULL` spec — its name and its
    /// `NO INHERIT` — first putting a plain one at the end of the specs if the
    /// column has none.
    fn update_not_null<F>(&mut self, update: F)
    where
        F: FnOnce(&mut Option<Name>, &mut bool),
    {
        for spec in &mut self.spec {
            if let ColumnSpec::NotNull { name, no_inherit } = spec {
                update(name, no_inherit);
                return;
            }
        }
        let (mut name, mut no_inherit) = (None, false);
        update(&mut name, &mut no_inherit);
        self.spec.push(ColumnSpec::NotNull { name, no_inherit });
    }
}
