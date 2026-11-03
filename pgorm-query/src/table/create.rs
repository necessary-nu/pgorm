use crate::{
    Check, ColumnDef, ColumnSpec, Comment, CommentStatement, IntoCheck, IntoColumnDef,
    IntoTableKey, Primary, QueryBuilder, TableKey, Unique, foreign_key::*, types::*,
};

/// Create a table
///
/// The table is taken by the constructor: `CREATE TABLE` has no spelling without
/// one, so a statement that never names its target does not construct.
///
/// ```compile_fail,E0061
/// use pgorm_query::{tests_cfg::*, *};
///
/// Table::create().col(ColumnDef::new(Glyph::Id).integer());
/// ```
///
/// # Examples
///
/// ```
/// use pgorm_query::{*, tests_cfg::*};
///
/// let table = Table::create(Char::Table)
///     .if_not_exists()
///     .comment("table's comment")
///     .col(ColumnDef::new(Char::Id).integer().not_null().auto_increment())
///     .col(ColumnDef::new(Char::FontSize).integer().not_null().comment("font's size"))
///     .col(ColumnDef::new(Char::Character).string().not_null())
///     .col(ColumnDef::new(Char::SizeW).integer().not_null())
///     .col(ColumnDef::new(Char::SizeH).integer().not_null())
///     .col(ColumnDef::new(Char::FontId).integer().default(Value::Int(None)))
///     .primary_key(Char::Id)
///     .foreign_key(
///         ForeignKey::create(Char::Table, Char::FontId, Font::Table, Font::Id)
///             .name(Name::runtime("FK_2e303c3a712662f1fc2a4d0aad6"))
///             .on_delete(ForeignKeyAction::Cascade)
///             .on_update(ForeignKeyAction::Cascade)
///             .to_owned()
///     )
///     .to_owned();
///
/// assert_eq!(
///     table.to_string(),
///     [
///         r#"CREATE TABLE IF NOT EXISTS "character" ("#,
///             r#""id" serial NOT NULL,"#,
///             r#""font_size" integer NOT NULL,"#,
///             r#""character" varchar NOT NULL,"#,
///             r#""size_w" integer NOT NULL,"#,
///             r#""size_h" integer NOT NULL,"#,
///             r#""font_id" integer DEFAULT NULL,"#,
///             r#"PRIMARY KEY ("id"),"#,
///             r#"CONSTRAINT "FK_2e303c3a712662f1fc2a4d0aad6""#,
///                 r#"FOREIGN KEY ("font_id") REFERENCES "font" ("id")"#,
///                 r#"ON DELETE CASCADE ON UPDATE CASCADE"#,
///         r#")"#,
///     ].join(" ")
/// );
/// ```
///
/// The comments are carried, not written into that SQL — PostgreSQL spells a
/// comment as a statement of its own, which [`comments()`] renders:
///
/// ```
/// # use pgorm_query::{*, tests_cfg::*};
/// # let table = Table::create(Char::Table)
/// #     .comment("table's comment")
/// #     .col(ColumnDef::new(Char::FontSize).integer().not_null().comment("font's size"))
/// #     .to_owned();
/// assert_eq!(
///     table.comments().iter().map(ToString::to_string).collect::<Vec<_>>(),
///     [
///         r#"COMMENT ON TABLE "character" IS 'table''s comment'"#,
///         r#"COMMENT ON COLUMN "character"."font_size" IS 'font''s size'"#,
///     ]
/// );
/// ```
///
/// [`comments()`]: TableCreateStatement::comments
// [spec:pgorm:req:sql.ddl.create-table+16]
#[derive(Debug, Clone)]
pub struct TableCreateStatement {
    pub(crate) table: TableName,
    pub(crate) columns: Vec<ColumnDef>,
    /// One slot, because a table has one primary key.
    pub(crate) primary_key: Option<TableKey<Primary>>,
    pub(crate) unique_keys: Vec<TableKey<Unique>>,
    pub(crate) foreign_keys: Vec<ForeignKeyCreateStatement>,
    pub(crate) if_not_exists: bool,
    pub(crate) check: Vec<Check>,
    pub(crate) comment: Option<String>,
    pub(crate) raw_suffix: Option<&'static str>,
}

impl TableCreateStatement {
    /// Construct create table statement over the table it creates
    pub fn new<T>(table: T) -> Self
    where
        T: IntoTableName,
    {
        Self {
            table: table.into_table_name(),
            columns: Vec::new(),
            primary_key: None,
            unique_keys: Vec::new(),
            foreign_keys: Vec::new(),
            if_not_exists: false,
            check: Vec::new(),
            comment: None,
            raw_suffix: None,
        }
    }

    /// Create table if table not exists
    pub fn if_not_exists(&mut self) -> &mut Self {
        self.if_not_exists = true;
        self
    }

    /// Set table comment
    ///
    /// PostgreSQL has no comment clause of `CREATE TABLE`, so the text is
    /// carried rather than written into this statement's SQL: render it with
    /// [`comments()`](Self::comments), which yields the `COMMENT ON`
    /// statements to execute after the create.
    // [spec:pgorm:req:sql.ddl.comment+5]
    pub fn comment<T>(&mut self, comment: T) -> &mut Self
    where
        T: Into<String>,
    {
        self.comment = Some(comment.into());
        self
    }

    /// Add a new table column
    pub fn col<C: IntoColumnDef>(&mut self, column: C) -> &mut Self {
        let mut column = column.into_column_def();
        column.table = Some(self.table.clone());
        self.columns.push(column);
        self
    }

    /// Add a table-level `CHECK (<expr>)`, after the ones already added.
    ///
    /// An expression is the unnamed, enforced constraint; a [`Check`] names it
    /// or says it is `NOT ENFORCED` (`[spec:pgorm:req:sql.ddl.enforcement]`).
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Table::create(Glyph::Table)
    ///         .col(ColumnDef::new(Glyph::Aspect).integer())
    ///         .check(Expr::col(Glyph::Aspect).gt(0))
    ///         .check(
    ///             Check::new(Expr::col(Glyph::Aspect).lt(100))
    ///                 .name(Name::runtime("aspect_small"))
    ///                 .enforcement(Enforcement::NotEnforced),
    ///         )
    ///         .to_string(),
    ///     [
    ///         r#"CREATE TABLE "glyph" ( "aspect" integer, CHECK ("aspect" > 0),"#,
    ///         r#"CONSTRAINT "aspect_small" CHECK ("aspect" < 100) NOT ENFORCED )"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.create-table+16]
    pub fn check<C>(&mut self, check: C) -> &mut Self
    where
        C: IntoCheck,
    {
        self.check.push(check.into_check());
        self
    }

    /// Declare the table's primary key: `PRIMARY KEY (…)`.
    ///
    /// A table has one primary key, and PostgreSQL refuses a second
    /// (`42P16`), so this is one slot rather than a list: a later call
    /// replaces the key an earlier one declared, as a second
    /// [`raw_suffix`](Self::raw_suffix) replaces the first. A column has no key
    /// clause of its own, so this is the only place a key is declared, and the
    /// table cannot be written with two. One column or a tuple of them
    /// converts into the key; [`TableKey::new`] builds one with a name,
    /// `INCLUDE` or deferrability.
    ///
    /// # Examples
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let mut statement = Table::create(Glyph::Table);
    /// statement
    ///     .col(ColumnDef::new(Glyph::Id).integer().not_null())
    ///     .col(ColumnDef::new(Glyph::Image).string().not_null())
    ///     .primary_key(Glyph::Id)
    ///     .primary_key((Glyph::Id, Glyph::Image));
    ///
    /// assert_eq!(
    ///     statement.to_string(),
    ///     [
    ///         r#"CREATE TABLE "glyph" ("#,
    ///         r#""id" integer NOT NULL,"#,
    ///         r#""image" varchar NOT NULL,"#,
    ///         r#"PRIMARY KEY ("id", "image")"#,
    ///         r#")"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.create-table+16]
    pub fn primary_key<K>(&mut self, key: K) -> &mut Self
    where
        K: IntoTableKey<Primary>,
    {
        self.primary_key = Some(key.into_table_key());
        self
    }

    /// Add a unique key: `UNIQUE (…)`, after the ones already added.
    ///
    /// A table has any number of unique keys, so each call appends one. One
    /// column or a tuple of them converts into the key; [`TableKey::new`]
    /// builds one with a name, `INCLUDE`, deferrability or
    /// [`nulls_not_distinct`](TableKey::nulls_not_distinct).
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Table::create(Glyph::Table)
    ///         .col(ColumnDef::new(Glyph::Aspect).integer())
    ///         .col(ColumnDef::new(Glyph::Image).text())
    ///         .unique(Glyph::Aspect)
    ///         .unique((Glyph::Image, Glyph::Aspect))
    ///         .to_string(),
    ///     [
    ///         r#"CREATE TABLE "glyph" ( "aspect" integer, "image" text,"#,
    ///         r#"UNIQUE ("aspect"), UNIQUE ("image", "aspect") )"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.create-table+16]
    pub fn unique<K>(&mut self, key: K) -> &mut Self
    where
        K: IntoTableKey<Unique>,
    {
        self.unique_keys.push(key.into_table_key());
        self
    }

    /// Add a foreign key
    ///
    /// The key is consumed, as `col()` consumes its column, and is restamped
    /// onto this statement's table: an embedded key constrains the table it
    /// sits inside and cannot name another. Pass an owned value —
    /// `.to_owned()` a builder chain you mean to reuse.
    pub fn foreign_key<F>(&mut self, foreign_key: F) -> &mut Self
    where
        F: Into<ForeignKeyCreateStatement>,
    {
        let mut foreign_key = foreign_key.into();
        foreign_key.foreign_key.retarget(self.table.clone());
        self.foreign_keys.push(foreign_key);
        self
    }

    pub fn get_table_name(&self) -> &TableName {
        &self.table
    }

    pub fn get_columns(&self) -> &Vec<ColumnDef> {
        self.columns.as_ref()
    }

    pub fn get_comment(&self) -> Option<&String> {
        self.comment.as_ref()
    }

    /// Every comment this statement carries, as the statements that render it
    ///
    /// On PostgreSQL a comment is a statement of its own, so the text handed
    /// to [`comment`](Self::comment) and to [`ColumnDef::comment`] cannot ride
    /// inside `CREATE TABLE`: this renders it out as the `COMMENT ON`
    /// statements to execute after the create, table comment first and then
    /// one per commented column in column order.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let table = Table::create(Char::Table)
    ///     .comment("one row per character")
    ///     .col(ColumnDef::new(Char::Id).integer().not_null())
    ///     .col(ColumnDef::new(Char::FontSize).integer().comment("in points"))
    ///     .primary_key(Char::Id)
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     table.comments().iter().map(ToString::to_string).collect::<Vec<_>>(),
    ///     [
    ///         r#"COMMENT ON TABLE "character" IS 'one row per character'"#,
    ///         r#"COMMENT ON COLUMN "character"."font_size" IS 'in points'"#,
    ///     ]
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.comment+5]
    pub fn comments(&self) -> Vec<CommentStatement> {
        let mut statements = Vec::new();
        if let Some(comment) = &self.comment {
            statements.push(Comment::on_table(self.table.clone(), comment.as_str()));
        }
        for column in &self.columns {
            for spec in column.get_column_spec() {
                if let ColumnSpec::Comment(text) = spec {
                    statements.push(Comment::on_column(
                        self.table.clone(),
                        column.name.clone(),
                        text.as_str(),
                    ));
                }
            }
        }
        statements
    }

    pub fn get_foreign_key_create_stmts(&self) -> &Vec<ForeignKeyCreateStatement> {
        self.foreign_keys.as_ref()
    }

    /// The table's primary key, if it declares one.
    // [spec:pgorm:req:sql.ddl.create-table+16]
    pub fn get_primary_key(&self) -> Option<&TableKey<Primary>> {
        self.primary_key.as_ref()
    }

    /// The table's unique keys, in the order they were added.
    // [spec:pgorm:req:sql.ddl.create-table+16]
    pub fn get_unique_keys(&self) -> &[TableKey<Unique>] {
        &self.unique_keys
    }

    /// The table's `CHECK` constraints, in the order they were added.
    // [spec:pgorm:req:sql.ddl.create-table+16]
    pub fn get_checks(&self) -> &[Check] {
        &self.check
    }

    /// Append verbatim SQL after the table's own clauses — the escape hatch
    /// for table options this vocabulary has no spelling for. One suffix per
    /// statement: a second call replaces the first, so compose the whole tail
    /// yourself.
    ///
    /// The `&'static str` bound is the contract, the same one
    /// [`Expr::raw`](crate::Expr::raw) carries: only program text can be
    /// appended, never a runtime string a value could have reached.
    ///
    /// Example for PostgresSQL [Citus](https://github.com/citusdata/citus) extension:
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    /// let table = Table::create(Char::Table)
    ///     .col(
    ///         ColumnDef::new(Char::Id)
    ///             .uuid()
    ///             .raw_suffix("DEFAULT uuid_generate_v4()")
    ///             .not_null(),
    ///     )
    ///     .col(
    ///         ColumnDef::new(Char::CreatedAt)
    ///             .timestamp_with_time_zone()
    ///             .raw_suffix("DEFAULT NOW()")
    ///             .not_null(),
    ///     )
    ///     .col(ColumnDef::new(Char::UserData).json_binary().not_null())
    ///     .primary_key(Char::Id)
    ///     .raw_suffix("USING columnar")
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     table.to_string(),
    ///     [
    ///         r#"CREATE TABLE "character" ("#,
    ///         r#""id" uuid DEFAULT uuid_generate_v4() NOT NULL,"#,
    ///         r#""created_at" timestamp with time zone DEFAULT NOW() NOT NULL,"#,
    ///         r#""user_data" jsonb NOT NULL,"#,
    ///         r#"PRIMARY KEY ("id")"#,
    ///         r#") USING columnar"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    pub fn raw_suffix(&mut self, sql: &'static str) -> &mut Self {
        self.raw_suffix = Some(sql);
        self
    }

    pub fn get_raw_suffix(&self) -> Option<&'static str> {
        self.raw_suffix
    }

    /// Clone this statement out of a builder chain.
    ///
    /// The table is copied rather than moved: moving it out would leave the
    /// targetless statement this type exists to rule out.
    pub fn take(&mut self) -> Self {
        Self {
            table: self.table.clone(),
            columns: std::mem::take(&mut self.columns),
            primary_key: self.primary_key.take(),
            unique_keys: std::mem::take(&mut self.unique_keys),
            foreign_keys: std::mem::take(&mut self.foreign_keys),
            if_not_exists: self.if_not_exists,
            check: std::mem::take(&mut self.check),
            comment: std::mem::take(&mut self.comment),
            raw_suffix: self.raw_suffix.take(),
        }
    }
}

/// Renders the statement with every value inlined as an escaped SQL literal.
/// This is its only rendering: it exposes no placeholder-emitting build, so
/// nothing here is left to bind.
// [spec:pgorm:req:sql.ddl+12]
impl std::fmt::Display for TableCreateStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sql = String::with_capacity(256);
        QueryBuilder.prepare_table_create_statement(self, &mut sql);
        f.write_str(&sql)
    }
}
