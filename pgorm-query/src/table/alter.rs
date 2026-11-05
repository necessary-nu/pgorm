use crate::{
    Check, Collation, ColumnDef, ColumnType, ConstraintChange, ConstraintDrop, IntoAddedConstraint,
    IntoCollation, IntoColumnDef, IntoConstraintDrop, IntoTableKey, NotNullConstraint, Primary,
    SimpleExpr, TableForeignKey, TableKey, Unique, backend::QueryBuilder, types::*,
};

/// A table awaiting its first alter action.
///
/// PostgreSQL has no spelling for an `ALTER TABLE` that does nothing, so this is
/// what [`Table::alter`] returns: naming the table is not yet a statement. Each
/// action method consumes it and yields a [`TableAlterStatement`], which carries
/// the table and at least one action for the rest of its life.
///
/// ```compile_fail,E0599
/// use pgorm_query::{tests_cfg::*, *};
///
/// Table::alter(Font::Table).to_string();
/// ```
///
/// [`Table::alter`]: crate::Table::alter
// [spec:pgorm:req:sql.ddl.alter-table+15]
#[derive(Debug, Clone)]
pub struct PendingTableAlter {
    table: TableName,
}

impl PendingTableAlter {
    pub(crate) fn new(table: TableName) -> Self {
        Self { table }
    }

    fn with(self, option: TableAlterOption) -> TableAlterStatement {
        TableAlterStatement {
            table: self.table,
            options: vec![option],
        }
    }

    /// Add a column to an existing table
    pub fn add_column<C: IntoColumnDef>(self, column_def: C) -> TableAlterStatement {
        self.with(TableAlterOption::add_column(column_def, false))
    }

    /// Try add a column to an existing table if it does not exists
    pub fn add_column_if_not_exists<C: IntoColumnDef>(self, column_def: C) -> TableAlterStatement {
        self.with(TableAlterOption::add_column(column_def, true))
    }

    /// Modify a column in an existing table
    pub fn modify_column<C: IntoColumnDef>(self, column_def: C) -> TableAlterStatement {
        self.with(TableAlterOption::ModifyColumn(column_def.into_column_def()))
    }

    /// See [`TableAlterStatement::alter_column_type_using`].
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn alter_column_type_using<C, E>(
        self,
        column: C,
        column_type: ColumnType,
        using: E,
    ) -> TableAlterStatement
    where
        C: IntoName,
        E: Into<SimpleExpr>,
    {
        self.with(TableAlterOption::alter_column_type_using(
            column,
            column_type,
            None,
            using,
        ))
    }

    /// See [`TableAlterStatement::alter_column_type_collated_using`].
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn alter_column_type_collated_using<C, L, E>(
        self,
        column: C,
        column_type: ColumnType,
        collation: L,
        using: E,
    ) -> TableAlterStatement
    where
        C: IntoName,
        L: IntoCollation,
        E: Into<SimpleExpr>,
    {
        self.with(TableAlterOption::alter_column_type_using(
            column,
            column_type,
            Some(collation.into_collation()),
            using,
        ))
    }

    /// See [`TableAlterStatement::drop_default`].
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn drop_default<C>(self, column: C) -> TableAlterStatement
    where
        C: IntoName,
    {
        self.with(TableAlterOption::DropDefault(column.into_name()))
    }

    /// Drop a column from an existing table
    pub fn drop_column<T>(self, col_name: T) -> TableAlterStatement
    where
        T: IntoName,
    {
        self.with(TableAlterOption::DropColumn(col_name.into_name()))
    }

    /// Add a foreign key to existing table, or with
    /// [`TableForeignKey::not_valid`] add it `NOT VALID`.
    ///
    /// The key is consumed, as [`add_column`](Self::add_column) consumes its
    /// column: pass an owned value, and `.to_owned()` a binding you mean to
    /// reuse.
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn add_foreign_key<F>(self, foreign_key: F) -> TableAlterStatement
    where
        F: IntoAddedConstraint<TableForeignKey>,
    {
        self.with(TableAlterOption::add_foreign_key(foreign_key))
    }

    /// Drop a constraint of any kind by name: `DROP CONSTRAINT "name"`, or
    /// with the `IF EXISTS` and behavior a [`ConstraintDrop`] carries.
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn drop_constraint<D>(self, drop: D) -> TableAlterStatement
    where
        D: IntoConstraintDrop,
    {
        self.with(TableAlterOption::DropConstraint(
            drop.into_constraint_drop(),
        ))
    }

    /// Give the table its primary key: `ADD PRIMARY KEY (…)`.
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn add_primary_key<K>(self, key: K) -> TableAlterStatement
    where
        K: IntoTableKey<Primary>,
    {
        self.with(TableAlterOption::AddPrimaryKey(key.into_table_key()))
    }

    /// Add a unique key to the table: `ADD UNIQUE (…)`.
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn add_unique<K>(self, key: K) -> TableAlterStatement
    where
        K: IntoTableKey<Unique>,
    {
        self.with(TableAlterOption::AddUnique(key.into_table_key()))
    }

    /// Recompute a generated column from a new expression:
    /// `ALTER COLUMN "c" SET EXPRESSION AS (<expr>)`. See
    /// [`TableAlterStatement::set_expression`].
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn set_expression<C, E>(self, column: C, expr: E) -> TableAlterStatement
    where
        C: IntoName,
        E: Into<SimpleExpr>,
    {
        self.with(TableAlterOption::SetExpression {
            column: column.into_name(),
            expr: expr.into(),
        })
    }

    /// Make a stored generated column a plain one that keeps its values:
    /// `ALTER COLUMN "c" DROP EXPRESSION`. See
    /// [`TableAlterStatement::drop_expression`].
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn drop_expression<C>(self, column: C) -> TableAlterStatement
    where
        C: IntoName,
    {
        self.with(TableAlterOption::drop_expression(column, false))
    }

    /// `ALTER COLUMN "c" DROP EXPRESSION IF EXISTS`, which leaves a column
    /// that is not generated as it is. See
    /// [`TableAlterStatement::drop_expression_if_exists`].
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn drop_expression_if_exists<C>(self, column: C) -> TableAlterStatement
    where
        C: IntoName,
    {
        self.with(TableAlterOption::drop_expression(column, true))
    }

    /// Add a `NOT NULL` constraint over a column:
    /// `ADD [CONSTRAINT "name" ]NOT NULL "c"…`. See
    /// [`TableAlterStatement::add_not_null`].
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn add_not_null(self, constraint: NotNullConstraint) -> TableAlterStatement {
        self.with(TableAlterOption::AddNotNull(constraint))
    }

    /// Add a `CHECK` constraint: `ADD [CONSTRAINT "name" ]CHECK (<expr>)…`.
    /// See [`TableAlterStatement::add_check`].
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn add_check<C>(self, check: C) -> TableAlterStatement
    where
        C: IntoAddedConstraint<Check>,
    {
        self.with(TableAlterOption::add_check(check))
    }

    /// Check the rows a `NOT VALID` constraint skipped:
    /// `VALIDATE CONSTRAINT "name"`. See
    /// [`TableAlterStatement::validate_constraint`].
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn validate_constraint<N>(self, name: N) -> TableAlterStatement
    where
        N: IntoName,
    {
        self.with(TableAlterOption::ValidateConstraint(name.into_name()))
    }

    /// Change a constraint that exists: `ALTER CONSTRAINT "name" <change>`.
    /// See [`TableAlterStatement::alter_constraint`].
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn alter_constraint<N>(self, name: N, change: ConstraintChange) -> TableAlterStatement
    where
        N: IntoName,
    {
        self.with(TableAlterOption::AlterConstraint {
            name: name.into_name(),
            change,
        })
    }
}

/// Alter a table
///
/// A statement of this type always names a table and always carries at least one
/// action: it is reachable only by choosing an action on a
/// [`PendingTableAlter`], so the `ALTER TABLE "font"` PostgreSQL rejects has no
/// constructor.
///
/// # Examples
///
/// ```
/// use pgorm_query::{tests_cfg::*, *};
///
/// let table = Table::alter(Font::Table).add_column(
///     ColumnDef::new(Name::runtime("new_col"))
///         .integer()
///         .not_null()
///         .default(100),
/// );
///
/// assert_eq!(
///     table.to_string(),
///     r#"ALTER TABLE "font" ADD COLUMN "new_col" integer NOT NULL DEFAULT 100"#
/// );
/// ```
///
/// There is no `take()`: draining the options would leave the action-less
/// statement this type exists to rule out, so the method a reader would expect
/// to move is absent rather than quietly copying. A second copy is
/// `.to_owned()`.
///
/// ```compile_fail,E0599
/// use pgorm_query::{tests_cfg::*, *};
///
/// let mut alter = Table::alter(Font::Table).drop_column(Font::Name).to_owned();
/// let moved: TableAlterStatement = alter.take();
/// ```
// [spec:pgorm:req:sql.ddl.alter-table+15]
// [spec:pgorm:req:sql.ast+4]
#[derive(Debug, Clone)]
pub struct TableAlterStatement {
    pub(crate) table: TableName,
    pub(crate) options: Vec<TableAlterOption>,
}

/// table alter add column options
#[derive(Debug, Clone)]
pub struct AddColumnOption {
    pub(crate) column: ColumnDef,
    pub(crate) if_not_exists: bool,
}

/// All available table alter options
///
/// `RENAME` is absent: PostgreSQL takes it only as the sole action of a
/// statement, so it lives in [`ColumnRenameStatement`](crate::ColumnRenameStatement)
/// where it cannot be
/// listed beside anything else.
// Boxing a variant would change the public shape of a DDL statement enum callers match on.
#[allow(clippy::large_enum_variant)]
// [spec:pgorm:req:sql.ddl.alter-table+15]
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum TableAlterOption {
    AddColumn(AddColumnOption),
    ModifyColumn(ColumnDef),
    /// `ALTER COLUMN "c" TYPE <type>[ COLLATE <collation>] USING <expr>`: the
    /// column retyped, under the collation when there is one, each row's
    /// value computed by the expression, which a retype no assignment cast
    /// covers needs.
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    AlterColumnTypeUsing {
        column: Name,
        column_type: ColumnType,
        collation: Option<Collation>,
        using: SimpleExpr,
    },
    /// `ALTER COLUMN "c" DROP DEFAULT`.
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    DropDefault(Name),
    DropColumn(Name),
    /// `ADD [CONSTRAINT "name" ]FOREIGN KEY …[ NOT VALID]`.
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    AddForeignKey {
        foreign_key: TableForeignKey,
        not_valid: bool,
    },
    /// `DROP CONSTRAINT [IF EXISTS ]"name"[ RESTRICT | CASCADE]`: a constraint
    /// of any kind, by name.
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    DropConstraint(ConstraintDrop),
    /// `ADD [CONSTRAINT "name"] PRIMARY KEY (…)`. The table may already have
    /// one, which only the server knows: it refuses a second (`42P16`).
    AddPrimaryKey(TableKey<Primary>),
    /// `ADD [CONSTRAINT "name"] UNIQUE [NULLS NOT DISTINCT] (…)`.
    AddUnique(TableKey<Unique>),
    /// `ALTER COLUMN "c" SET EXPRESSION AS (<expr>)`: a generated column's new
    /// expression. Whether the column is generated is the server's knowledge;
    /// it refuses one that is not (`55000`).
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    SetExpression {
        column: Name,
        expr: SimpleExpr,
    },
    /// `ALTER COLUMN "c" DROP EXPRESSION[ IF EXISTS]`: a stored generated
    /// column made plain, keeping its values.
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    DropExpression {
        column: Name,
        if_exists: bool,
    },
    /// `ADD [CONSTRAINT "name" ]NOT NULL "c"[ NO INHERIT][ NOT VALID]`: a
    /// not-null constraint added at table level, the one spelling that can be
    /// `NOT VALID`.
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    AddNotNull(NotNullConstraint),
    /// `ADD [CONSTRAINT "name" ]CHECK (<expr>)[ NO INHERIT][ ENFORCED | NOT
    /// ENFORCED][ NOT VALID]`.
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    AddCheck {
        check: Check,
        not_valid: bool,
    },
    /// `VALIDATE CONSTRAINT "name"`: the rows a `NOT VALID` foreign key,
    /// `CHECK` or `NOT NULL` skipped, checked now. Any other kind is refused
    /// (`42809`).
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    ValidateConstraint(Name),
    /// `ALTER CONSTRAINT "name" <change>`.
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    AlterConstraint {
        name: Name,
        change: ConstraintChange,
    },
}

impl TableAlterOption {
    fn add_column<C: IntoColumnDef>(column_def: C, if_not_exists: bool) -> Self {
        Self::AddColumn(AddColumnOption {
            column: column_def.into_column_def(),
            if_not_exists,
        })
    }

    fn add_foreign_key<F: IntoAddedConstraint<TableForeignKey>>(foreign_key: F) -> Self {
        let (foreign_key, not_valid) = foreign_key.into_added_constraint();
        Self::AddForeignKey {
            foreign_key,
            not_valid,
        }
    }

    fn alter_column_type_using<C: IntoName, E: Into<SimpleExpr>>(
        column: C,
        column_type: ColumnType,
        collation: Option<Collation>,
        using: E,
    ) -> Self {
        Self::AlterColumnTypeUsing {
            column: column.into_name(),
            column_type,
            collation,
            using: using.into(),
        }
    }

    fn add_check<C: IntoAddedConstraint<Check>>(check: C) -> Self {
        let (check, not_valid) = check.into_added_constraint();
        Self::AddCheck { check, not_valid }
    }

    fn drop_expression<C: IntoName>(column: C, if_exists: bool) -> Self {
        Self::DropExpression {
            column: column.into_name(),
            if_exists,
        }
    }
}

impl TableAlterStatement {
    /// Add a column to an existing table
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let table = Table::alter(Font::Table)
    ///     .drop_column(Name::runtime("old_col"))
    ///     .add_column(
    ///         ColumnDef::new(Name::runtime("new_col"))
    ///             .integer()
    ///             .not_null()
    ///             .default(100),
    ///     )
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     table.to_string(),
    ///     [
    ///         r#"ALTER TABLE "font" DROP COLUMN "old_col","#,
    ///         r#"ADD COLUMN "new_col" integer NOT NULL DEFAULT 100"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    pub fn add_column<C: IntoColumnDef>(&mut self, column_def: C) -> &mut Self {
        self.add_alter_option(TableAlterOption::add_column(column_def, false))
    }

    /// Try add a column to an existing table if it does not exists
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let table = Table::alter(Font::Table).add_column_if_not_exists(
    ///     ColumnDef::new(Name::runtime("new_col"))
    ///         .integer()
    ///         .not_null()
    ///         .default(100),
    /// );
    ///
    /// assert_eq!(
    ///     table.to_string(),
    ///     r#"ALTER TABLE "font" ADD COLUMN IF NOT EXISTS "new_col" integer NOT NULL DEFAULT 100"#
    /// );
    /// ```
    pub fn add_column_if_not_exists<C: IntoColumnDef>(&mut self, column_def: C) -> &mut Self {
        self.add_alter_option(TableAlterOption::add_column(column_def, true))
    }

    /// Modify a column in an existing table
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let table = Table::alter(Font::Table).modify_column(
    ///     ColumnDef::new(Name::runtime("new_col"))
    ///         .big_integer()
    ///         .default(999),
    /// );
    ///
    /// assert_eq!(
    ///     table.to_string(),
    ///     [
    ///         r#"ALTER TABLE "font""#,
    ///         r#"ALTER COLUMN "new_col" TYPE bigint,"#,
    ///         r#"ALTER COLUMN "new_col" SET DEFAULT 999"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    pub fn modify_column<C: IntoColumnDef>(&mut self, column_def: C) -> &mut Self {
        self.add_alter_option(TableAlterOption::ModifyColumn(column_def.into_column_def()))
    }

    /// Retype a column, computing each row's new value with `using`: `ALTER
    /// COLUMN "c" TYPE <type> USING <expr>`.
    ///
    /// [`modify_column`](Self::modify_column)'s retype converts each value
    /// by the assignment cast from the old type to the new one, and a type no
    /// such cast reaches, as one enum type from another, needs the expression
    /// instead. The server refuses `USING` on a generated column (`42611`),
    /// and a default it cannot cast to the new type (`42804`), which
    /// [`drop_default`](Self::drop_default) drops first.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let kind = TypeName::new(Name::runtime("font_kind_next"));
    /// let table = Table::alter(Font::Table)
    ///     .drop_default(Font::Variant)
    ///     .alter_column_type_using(
    ///         Font::Variant,
    ///         ColumnType::named("font_kind_next"),
    ///         Expr::col(Font::Variant)
    ///             .cast_as(Name::runtime("text"))
    ///             .cast_as_type(kind),
    ///     )
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     table.to_string(),
    ///     [
    ///         r#"ALTER TABLE "font" ALTER COLUMN "variant" DROP DEFAULT,"#,
    ///         r#"ALTER COLUMN "variant" TYPE font_kind_next"#,
    ///         r#"USING CAST(CAST("variant" AS text) AS font_kind_next)"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn alter_column_type_using<C, E>(
        &mut self,
        column: C,
        column_type: ColumnType,
        using: E,
    ) -> &mut Self
    where
        C: IntoName,
        E: Into<SimpleExpr>,
    {
        self.add_alter_option(TableAlterOption::alter_column_type_using(
            column,
            column_type,
            None,
            using,
        ))
    }

    /// Retype a column under a collation, computing each row's new value by
    /// an expression: `ALTER COLUMN "c" TYPE <type> COLLATE <collation> USING
    /// <expr>`, [`alter_column_type_using`](Self::alter_column_type_using)
    /// with the collation the retype gives the column. The server refuses a
    /// collation on a type that takes none (`42804`) and one it does not have
    /// (`42704`).
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let table = Table::alter(Font::Table)
    ///     .alter_column_type_collated_using(
    ///         Font::Name,
    ///         ColumnType::Text,
    ///         Name::runtime("C"),
    ///         Expr::col(Font::Name).cast_as(Name::runtime("text")),
    ///     )
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     table.to_string(),
    ///     r#"ALTER TABLE "font" ALTER COLUMN "name" TYPE text COLLATE "C" USING CAST("name" AS text)"#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn alter_column_type_collated_using<C, L, E>(
        &mut self,
        column: C,
        column_type: ColumnType,
        collation: L,
        using: E,
    ) -> &mut Self
    where
        C: IntoName,
        L: IntoCollation,
        E: Into<SimpleExpr>,
    {
        self.add_alter_option(TableAlterOption::alter_column_type_using(
            column,
            column_type,
            Some(collation.into_collation()),
            using,
        ))
    }

    /// Drop a column's default: `ALTER COLUMN "c" DROP DEFAULT`. A column
    /// with none is left as it is.
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn drop_default<C>(&mut self, column: C) -> &mut Self
    where
        C: IntoName,
    {
        self.add_alter_option(TableAlterOption::DropDefault(column.into_name()))
    }

    /// Drop a column from an existing table
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let table = Table::alter(Font::Table).drop_column(Name::runtime("new_column"));
    ///
    /// assert_eq!(
    ///     table.to_string(),
    ///     r#"ALTER TABLE "font" DROP COLUMN "new_column""#
    /// );
    /// ```
    pub fn drop_column<T>(&mut self, col_name: T) -> &mut Self
    where
        T: IntoName,
    {
        self.add_alter_option(TableAlterOption::DropColumn(col_name.into_name()))
    }

    /// Add a foreign key to existing table
    ///
    /// The key is consumed, as [`add_column`](Self::add_column) consumes its
    /// column: pass an owned value, and `.to_owned()` a binding you mean to
    /// reuse.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let foreign_key_char =
    ///     TableForeignKey::new(Char::Table, Char::FontId, Glyph::Table, Char::FontId)
    ///         .name(Name::runtime("FK_character_glyph"))
    ///         .col(Char::Id, Char::Id)
    ///         .on_delete(ForeignKeyAction::Cascade)
    ///         .on_update(ForeignKeyAction::Cascade)
    ///         .to_owned();
    ///
    /// let foreign_key_font = TableForeignKey::new(Char::Table, Char::FontId, Font::Table, Font::Id)
    ///     .name(Name::runtime("FK_character_font"))
    ///     .on_delete(ForeignKeyAction::Cascade)
    ///     .on_update(ForeignKeyAction::Cascade)
    ///     .to_owned();
    ///
    /// let table = Table::alter(Character::Table)
    ///     .add_foreign_key(foreign_key_char)
    ///     .add_foreign_key(foreign_key_font)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     table.to_string(),
    ///     [
    ///         r#"ALTER TABLE "character""#,
    ///         r#"ADD CONSTRAINT "FK_character_glyph""#,
    ///         r#"FOREIGN KEY ("font_id", "id") REFERENCES "glyph" ("font_id", "id")"#,
    ///         r#"ON DELETE CASCADE ON UPDATE CASCADE,"#,
    ///         r#"ADD CONSTRAINT "FK_character_font""#,
    ///         r#"FOREIGN KEY ("font_id") REFERENCES "font" ("id")"#,
    ///         r#"ON DELETE CASCADE ON UPDATE CASCADE"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn add_foreign_key<F>(&mut self, foreign_key: F) -> &mut Self
    where
        F: IntoAddedConstraint<TableForeignKey>,
    {
        self.add_alter_option(TableAlterOption::add_foreign_key(foreign_key))
    }

    /// Drop a constraint of any kind by name: `DROP CONSTRAINT [IF EXISTS
    /// ]"name"[ RESTRICT | CASCADE]`. A name is the plain drop, and a
    /// [`ConstraintDrop`] carries `IF EXISTS` and the behavior.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let table = Table::alter(Character::Table)
    ///     .drop_constraint(Name::runtime("FK_character_glyph"))
    ///     .drop_constraint(Name::runtime("character_font_id_not_null"))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     table.to_string(),
    ///     [
    ///         r#"ALTER TABLE "character""#,
    ///         r#"DROP CONSTRAINT "FK_character_glyph","#,
    ///         r#"DROP CONSTRAINT "character_font_id_not_null""#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn drop_constraint<D>(&mut self, drop: D) -> &mut Self
    where
        D: IntoConstraintDrop,
    {
        self.add_alter_option(TableAlterOption::DropConstraint(
            drop.into_constraint_drop(),
        ))
    }

    /// Give the table its primary key: `ADD PRIMARY KEY (…)`, the key the
    /// table declares when it is created
    /// ([`TableCreateStatement::primary_key`](crate::TableCreateStatement::primary_key)).
    ///
    /// Whether the table has a key already is the server's knowledge, not the
    /// builder's, so a second is refused there (`42P16`).
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let table = Table::alter(Glyph::Table)
    ///     .add_column(ColumnDef::new(Glyph::Aspect).integer().not_null())
    ///     .add_primary_key(TableKey::new(Glyph::Id).col(Glyph::Aspect).name(Name::runtime("glyph_pk")))
    ///     .add_unique(TableKey::new(Glyph::Image).nulls_not_distinct())
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     table.to_string(),
    ///     [
    ///         r#"ALTER TABLE "glyph" ADD COLUMN "aspect" integer NOT NULL,"#,
    ///         r#"ADD CONSTRAINT "glyph_pk" PRIMARY KEY ("id", "aspect"),"#,
    ///         r#"ADD UNIQUE NULLS NOT DISTINCT ("image")"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn add_primary_key<K>(&mut self, key: K) -> &mut Self
    where
        K: IntoTableKey<Primary>,
    {
        self.add_alter_option(TableAlterOption::AddPrimaryKey(key.into_table_key()))
    }

    /// Add a unique key to the table: `ADD UNIQUE (…)`, the key a table
    /// declares with [`TableCreateStatement::unique`](crate::TableCreateStatement::unique).
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn add_unique<K>(&mut self, key: K) -> &mut Self
    where
        K: IntoTableKey<Unique>,
    {
        self.add_alter_option(TableAlterOption::AddUnique(key.into_table_key()))
    }

    /// Recompute a generated column from a new expression:
    /// `ALTER COLUMN "c" SET EXPRESSION AS (<expr>)`.
    ///
    /// The column keeps its kind and every existing row takes the new value.
    /// A [stored](crate::GeneratedKind::Stored) column is rewritten to hold
    /// it, a [virtual](crate::GeneratedKind::Virtual) one is not, since it
    /// computes on read. The expression is held to what a generated column's
    /// is (immutable, reading no other generated column: `42P17`), and a
    /// column that is not generated, an identity included, is refused
    /// (`55000`). So is a virtual column whose table belongs to a publication
    /// (`0A000`), or on PostgreSQL 18 has a `CHECK` constraint; from 19 the
    /// rows are checked against the table's `CHECK` constraints instead
    /// (`23514`).
    ///
    /// This is the one way to change a generated column's expression:
    /// [`modify_column`](Self::modify_column) writes no
    /// [`generated`](crate::ColumnDef::generated) spec, since a kind it
    /// carried could not be honoured.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let table = Table::alter(Glyph::Table)
    ///     .set_expression(Glyph::Aspect, Expr::col(Glyph::Id).mul(3))
    ///     .drop_expression(Glyph::Tokens)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     table.to_string(),
    ///     [
    ///         r#"ALTER TABLE "glyph""#,
    ///         r#"ALTER COLUMN "aspect" SET EXPRESSION AS ("id" * 3),"#,
    ///         r#"ALTER COLUMN "tokens" DROP EXPRESSION"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn set_expression<C, E>(&mut self, column: C, expr: E) -> &mut Self
    where
        C: IntoName,
        E: Into<SimpleExpr>,
    {
        self.add_alter_option(TableAlterOption::SetExpression {
            column: column.into_name(),
            expr: expr.into(),
        })
    }

    /// Make a stored generated column a plain one:
    /// `ALTER COLUMN "c" DROP EXPRESSION`.
    ///
    /// Every row keeps the value it was last computed to, and from then on
    /// the column is written like any other. PostgreSQL refuses it for a
    /// [virtual](crate::GeneratedKind::Virtual) column, which has no stored
    /// values to keep (`0A000`), and for a column that is not generated
    /// (`55000`) — unless the action says
    /// [`IF EXISTS`](Self::drop_expression_if_exists).
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn drop_expression<C>(&mut self, column: C) -> &mut Self
    where
        C: IntoName,
    {
        self.add_alter_option(TableAlterOption::drop_expression(column, false))
    }

    /// `ALTER COLUMN "c" DROP EXPRESSION IF EXISTS`: as
    /// [`drop_expression`](Self::drop_expression), except that a column that
    /// is not generated is left as it is, with a notice, rather than refused.
    /// A virtual column is refused either way (`0A000`).
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Table::alter(Glyph::Table)
    ///         .drop_expression_if_exists(Glyph::Aspect)
    ///         .to_string(),
    ///     r#"ALTER TABLE "glyph" ALTER COLUMN "aspect" DROP EXPRESSION IF EXISTS"#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn drop_expression_if_exists<C>(&mut self, column: C) -> &mut Self
    where
        C: IntoName,
    {
        self.add_alter_option(TableAlterOption::drop_expression(column, true))
    }

    /// Add a `NOT NULL` constraint over a column:
    /// `ADD [CONSTRAINT "name" ]NOT NULL "c"[ NO INHERIT][ NOT VALID]`.
    ///
    /// This is PostgreSQL 18's table-level spelling, and the one that can
    /// carry a name, `NO INHERIT` and `NOT VALID` together; a column added in
    /// the same statement carries its own with
    /// [`ColumnDef::not_null`](crate::ColumnDef::not_null) and its companions.
    /// Without `NOT VALID` the rows already there are checked, and a null
    /// among them is refused (`23502`). With it they are left for
    /// [`validate_constraint`](Self::validate_constraint) to check later,
    /// while new rows are held to the constraint at once.
    ///
    /// A column has one not-null constraint, so adding one where it exists
    /// does nothing when the one there already says as much — the same name
    /// or none, the same `NO INHERIT`, and valid where this one is — and is
    /// refused when it does not (`55000`): under another name, with another
    /// `NO INHERIT`, or as a valid constraint over one still `NOT VALID`. The
    /// last is where [`modify_column`](Self::modify_column)'s `SET NOT NULL`
    /// differs: it validates the constraint it finds.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let table = Table::alter(Glyph::Table)
    ///     .add_not_null(NotNullConstraint::new(Glyph::Aspect).not_valid())
    ///     .add_not_null(
    ///         NotNullConstraint::new(Glyph::Image)
    ///             .name(Name::runtime("glyph_image_present"))
    ///             .no_inherit(),
    ///     )
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     table.to_string(),
    ///     [
    ///         r#"ALTER TABLE "glyph" ADD NOT NULL "aspect" NOT VALID,"#,
    ///         r#"ADD CONSTRAINT "glyph_image_present" NOT NULL "image" NO INHERIT"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn add_not_null(&mut self, constraint: NotNullConstraint) -> &mut Self {
        self.add_alter_option(TableAlterOption::AddNotNull(constraint))
    }

    /// Add a `CHECK` constraint to the table:
    /// `ADD [CONSTRAINT "name" ]CHECK (<expr>)[ ENFORCED | NOT ENFORCED]`, the
    /// constraint a table declares with
    /// [`TableCreateStatement::check`](crate::TableCreateStatement::check).
    ///
    /// The rows already there are checked, and one that breaks the condition
    /// refuses the action (`23514`) — unless the [`Check`] is
    /// [`NotEnforced`](crate::Enforcement::NotEnforced), which no row is
    /// held to (`[spec:pgorm:req:sql.ddl.enforcement]`), or is added
    /// [`not_valid`](Check::not_valid), which leaves them for
    /// [`validate_constraint`](Self::validate_constraint) and holds every row
    /// written from then on.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let table = Table::alter(Glyph::Table)
    ///     .add_check(Expr::col(Glyph::Aspect).gt(0))
    ///     .add_check(
    ///         Check::new(Expr::col(Glyph::Aspect).lt(100))
    ///             .name(Name::runtime("aspect_small"))
    ///             .enforcement(Enforcement::NotEnforced),
    ///     )
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     table.to_string(),
    ///     [
    ///         r#"ALTER TABLE "glyph" ADD CHECK ("aspect" > 0),"#,
    ///         r#"ADD CONSTRAINT "aspect_small" CHECK ("aspect" < 100) NOT ENFORCED"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn add_check<C>(&mut self, check: C) -> &mut Self
    where
        C: IntoAddedConstraint<Check>,
    {
        self.add_alter_option(TableAlterOption::add_check(check))
    }

    /// Check the rows a `NOT VALID` constraint skipped:
    /// `VALIDATE CONSTRAINT "name"`.
    ///
    /// A foreign key, `CHECK` or `NOT NULL` constraint takes it; the server
    /// refuses any other kind (`42809`) and a name the table has no
    /// constraint under (`42704`), and a row the constraint does not hold for
    /// fails the statement as it would have failed the insert. A constraint
    /// that is already valid is left as it is.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Table::alter(Glyph::Table)
    ///         .validate_constraint(Name::runtime("glyph_aspect_present"))
    ///         .to_string(),
    ///     r#"ALTER TABLE "glyph" VALIDATE CONSTRAINT "glyph_aspect_present""#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn validate_constraint<N>(&mut self, name: N) -> &mut Self
    where
        N: IntoName,
    {
        self.add_alter_option(TableAlterOption::ValidateConstraint(name.into_name()))
    }

    /// Change a constraint that exists: `ALTER CONSTRAINT "name" <change>`.
    ///
    /// Which kind of constraint the name holds is the server's knowledge, and
    /// each [`ConstraintChange`] applies to the kinds it names: asked of
    /// another, it is refused (`42809`). The builder cannot see the kind, so
    /// the one change whose kinds differ by release —
    /// [`Enforced`](ConstraintChange::Enforced) and
    /// [`NotEnforced`](ConstraintChange::NotEnforced) on a `CHECK`, refused by
    /// PostgreSQL 18 and taken by 19 — builds under either target.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Table::alter(Glyph::Table)
    ///         .alter_constraint(Name::runtime("glyph_aspect_present"), ConstraintChange::NoInherit)
    ///         .to_string(),
    ///     r#"ALTER TABLE "glyph" ALTER CONSTRAINT "glyph_aspect_present" NO INHERIT"#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.alter-table+15]
    pub fn alter_constraint<N>(&mut self, name: N, change: ConstraintChange) -> &mut Self
    where
        N: IntoName,
    {
        self.add_alter_option(TableAlterOption::AlterConstraint {
            name: name.into_name(),
            change,
        })
    }

    /// The table the statement alters, which a caller adding a foreign key
    /// to a statement it did not build names as the key's table.
    pub fn get_table_name(&self) -> &TableName {
        &self.table
    }

    fn add_alter_option(&mut self, alter_option: TableAlterOption) -> &mut Self {
        self.options.push(alter_option);
        self
    }
}

/// Renders the statement with every value inlined as an escaped SQL literal.
/// This is its only rendering: it exposes no placeholder-emitting build, so
/// nothing here is left to bind.
// [spec:pgorm:req:sql.ddl+12]
impl std::fmt::Display for TableAlterStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sql = String::with_capacity(256);
        QueryBuilder.prepare_table_alter_statement(self, &mut sql);
        f.write_str(&sql)
    }
}
