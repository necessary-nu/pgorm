//! `COPY … FROM STDIN` and `COPY … TO STDOUT`: the statements that open COPY's
//! data flow, with the format and its options chosen from what PostgreSQL
//! accepts in each direction.

use std::{fmt, num::NonZeroU32};

use inherent::inherent;

use crate::{
    ConditionHolder, ConditionalStatement, DeleteStatement, InsertStatement, IntoCondition,
    IntoName, IntoTableName, MergeStatement, Name, QueryBuilder, SelectStatement, SimpleExpr,
    SubQueryStatement, TableName, UpdateStatement, error::Error,
};

mod sealed {
    pub trait Sealed {
        /// The `FORMAT` option's value.
        const KEYWORD: &'static str;
    }
}

/// One of COPY's three data formats: [`CopyText`], [`CopyCsv`] or
/// [`CopyBinary`].
///
/// The set is closed. A statement carries its format as a type parameter, so
/// an option the format does not accept is a method the statement does not
/// have.
// [spec:pgorm:def:sql.ast.copy+3]
pub trait CopyFormat: sealed::Sealed + Clone + fmt::Debug + Send + Sync + 'static {}

/// The two formats that are lines of delimited text, [`CopyText`] and
/// [`CopyCsv`], and so take a delimiter, a NULL string, a header line and an
/// encoding. [`CopyBinary`] takes none of them.
pub trait CopyDelimitedFormat: CopyFormat {}

/// `FORMAT text`: PostgreSQL's own line format, tab-separated by default, with
/// backslash escapes and `\N` for NULL.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CopyText;

/// `FORMAT csv`: comma-separated values with quoting, and an unquoted empty
/// field for NULL by default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CopyCsv;

/// `FORMAT binary`: PostgreSQL's binary tuple format — a signature header, then
/// each row as its fields' binary representations, then a trailer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CopyBinary;

impl sealed::Sealed for CopyText {
    const KEYWORD: &'static str = "text";
}
impl sealed::Sealed for CopyCsv {
    const KEYWORD: &'static str = "csv";
}
impl sealed::Sealed for CopyBinary {
    const KEYWORD: &'static str = "binary";
}
impl CopyFormat for CopyText {}
impl CopyFormat for CopyCsv {}
impl CopyFormat for CopyBinary {}
impl CopyDelimitedFormat for CopyText {}
impl CopyDelimitedFormat for CopyCsv {}

/// `ON_ERROR ignore`: skip a row whose field does not convert to its column's
/// type, where the default fails the whole COPY.
///
/// `ON_ERROR stop` is the server's default, and selecting it says nothing, so
/// there is no value for it; neither `REJECT_LIMIT` nor `LOG_VERBOSITY` means
/// anything without `ignore`, so both live here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CopyIgnore {
    /// `REJECT_LIMIT n`: fail the COPY after all once more than `n` rows have
    /// been skipped. `None` skips without limit.
    pub reject_limit: Option<NonZeroU32>,
    /// `LOG_VERBOSITY`: what the server reports about skipped rows. `None`
    /// is its default, one `NOTICE` with the number skipped.
    pub log_verbosity: Option<CopyLogVerbosity>,
}

/// `LOG_VERBOSITY`: how much a COPY that ignores errors reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CopyLogVerbosity {
    /// `verbose`: a `NOTICE` per skipped row, naming its line and column.
    Verbose,
    /// `silent`: no `NOTICE` at all, not even the count.
    Silent,
}

/// A column list an option forces, or `*` for every column.
#[derive(Debug, Clone)]
pub(crate) enum CopyColumns {
    All,
    Named(Vec<Name>),
}

impl CopyColumns {
    fn named<C, I>(columns: I) -> Option<Self>
    where
        C: IntoName,
        I: IntoIterator<Item = C>,
    {
        let named: Vec<Name> = columns.into_iter().map(IntoName::into_name).collect();
        // `FORCE_QUOTE ()` is a syntax error; forcing no column is the option
        // left out.
        (!named.is_empty()).then_some(Self::Named(named))
    }
}

/// Whether a header line is written or read, and on COPY FROM whether it is
/// checked against the column names.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum CopyHeader {
    #[default]
    Absent,
    Present,
    Match,
}

/// The options a statement has been given. Which fields a statement can set is
/// decided by the methods its direction and format offer, so a field the
/// server would refuse for that direction or format stays at its default.
#[derive(Debug, Clone, Default)]
pub(crate) struct CopyOptions {
    pub(crate) freeze: bool,
    pub(crate) delimiter: Option<char>,
    pub(crate) null: Option<String>,
    pub(crate) default: Option<String>,
    pub(crate) header: CopyHeader,
    pub(crate) quote: Option<char>,
    pub(crate) escape: Option<char>,
    pub(crate) force_quote: Option<CopyColumns>,
    pub(crate) force_not_null: Option<CopyColumns>,
    pub(crate) force_null: Option<CopyColumns>,
    pub(crate) on_error: Option<CopyIgnore>,
    pub(crate) encoding: Option<String>,
}

/// `COPY table [(column, …)] FROM STDIN WITH (FORMAT …)`: rows sent by the
/// client into a table.
///
/// The format is chosen when the statement is made, and is its type
/// parameter: the options [`CopyText`] and [`CopyCsv`] share are methods on
/// both, the ones only CSV has are methods on `CopyFromStatement<CopyCsv>`, and
/// [`CopyBinary`] has neither. Every option here is one PostgreSQL accepts on
/// COPY FROM; the ones only COPY TO takes (`FORCE_QUOTE`) are on
/// [`CopyToStatement`].
///
/// COPY takes no bind parameters, so the statement has one rendering, its
/// [`Display`](fmt::Display): a value in its `WHERE` is written into the SQL as
/// an escaped literal.
///
/// ```
/// use pgorm_query::{tests_cfg::*, *};
///
/// let copy = CopyFromStatement::new(Glyph::Table, CopyCsv)
///     .columns([Glyph::Id, Glyph::Image])
///     .header()
///     .null_string("")
///     .force_null([Glyph::Image])
///     .to_owned();
///
/// assert_eq!(
///     copy.to_string(),
///     [
///         r#"COPY "glyph" ("id", "image") FROM STDIN"#,
///         r#"WITH (FORMAT csv, NULL '', HEADER true, FORCE_NULL ("image"))"#,
///     ]
///     .join(" ")
/// );
/// ```
///
/// A CSV-only option is not a method of a text-format statement:
///
/// ```compile_fail,E0599
/// use pgorm_query::{tests_cfg::*, *};
///
/// CopyFromStatement::new(Glyph::Table, CopyText).quote('\'');
/// ```
///
/// and a binary statement has no delimited-text option at all:
///
/// ```compile_fail,E0599
/// use pgorm_query::{tests_cfg::*, *};
///
/// CopyFromStatement::new(Glyph::Table, CopyBinary).header();
/// ```
// [spec:pgorm:def:sql.ast.copy+3]
#[derive(Debug, Clone)]
pub struct CopyFromStatement<F: CopyFormat> {
    pub(crate) table: TableName,
    pub(crate) columns: Vec<Name>,
    pub(crate) format: F,
    pub(crate) options: CopyOptions,
    pub(crate) r#where: ConditionHolder,
}

impl<F: CopyFormat> CopyFromStatement<F> {
    /// COPY into `table` from the client, in `format`.
    pub fn new<T>(table: T, format: F) -> Self
    where
        T: IntoTableName,
    {
        Self {
            table: table.into_table_name(),
            columns: Vec::new(),
            format,
            options: CopyOptions::default(),
            r#where: ConditionHolder::new(),
        }
    }

    /// The columns each row supplies, in order; a column left out takes its
    /// default. A later call adds to an earlier one, and with none the row
    /// supplies every column in the table's order.
    pub fn columns<C, I>(&mut self, columns: I) -> &mut Self
    where
        C: IntoName,
        I: IntoIterator<Item = C>,
    {
        self.columns
            .extend(columns.into_iter().map(IntoName::into_name));
        self
    }

    /// `FREEZE true`: write the rows already frozen, as if `VACUUM FREEZE`
    /// had run. PostgreSQL refuses it (`55000`) unless the table was created
    /// or truncated in the current subtransaction.
    pub fn freeze(&mut self) -> &mut Self {
        self.options.freeze = true;
        self
    }

    /// The table the rows go into.
    // [spec:pgorm:def:sql.ast.copy.parts]
    pub fn get_table(&self) -> &TableName {
        &self.table
    }

    /// The columns each row supplies, in order. Empty when the statement
    /// names none, and each row then supplies every column of the table the
    /// server does not generate, in the table's order.
    // [spec:pgorm:def:sql.ast.copy.parts]
    pub fn get_columns(&self) -> &[Name] {
        &self.columns
    }
}

impl<F: CopyDelimitedFormat> CopyFromStatement<F> {
    /// `DELIMITER`: the character between fields, tab for text and comma for
    /// CSV by default. The server takes a single one-byte character and
    /// refuses a newline, a carriage return, and in text format a backslash,
    /// a period, a lowercase letter or a digit.
    pub fn delimiter(&mut self, delimiter: char) -> &mut Self {
        self.options.delimiter = Some(delimiter);
        self
    }

    /// `NULL`: the field that reads as SQL NULL — `\N` in text format and an
    /// unquoted empty field in CSV by default.
    pub fn null_string<S>(&mut self, null: S) -> &mut Self
    where
        S: Into<String>,
    {
        self.options.null = Some(null.into());
        self
    }

    /// `DEFAULT`: the field that reads as the column's default value.
    /// PostgreSQL refuses it when it equals the `NULL` string.
    pub fn default_string<S>(&mut self, default: S) -> &mut Self
    where
        S: Into<String>,
    {
        self.options.default = Some(default.into());
        self
    }

    /// `HEADER true`: the first line is a header, and is skipped.
    pub fn header(&mut self) -> &mut Self {
        self.options.header = CopyHeader::Present;
        self
    }

    /// `HEADER match`: the first line is a header, and its field names must
    /// equal the columns' names in order, or the COPY fails.
    pub fn header_match(&mut self) -> &mut Self {
        self.options.header = CopyHeader::Match;
        self
    }

    /// `ENCODING`: the encoding the data is in, when it is not the client
    /// encoding. It is the server's name for one (`'UTF8'`, `'LATIN1'`).
    pub fn encoding<S>(&mut self, encoding: S) -> &mut Self
    where
        S: Into<String>,
    {
        self.options.encoding = Some(encoding.into());
        self
    }

    /// `ON_ERROR ignore`: skip the rows whose fields do not convert rather
    /// than failing the COPY, under `ignore`'s limit and reporting.
    pub fn on_error_ignore(&mut self, ignore: CopyIgnore) -> &mut Self {
        self.options.on_error = Some(ignore);
        self
    }
}

impl CopyFromStatement<CopyCsv> {
    /// `QUOTE`: the character that quotes a field, `"` by default.
    pub fn quote(&mut self, quote: char) -> &mut Self {
        self.options.quote = Some(quote);
        self
    }

    /// `ESCAPE`: the character that escapes a quote inside a quoted field,
    /// the `QUOTE` character by default.
    pub fn escape(&mut self, escape: char) -> &mut Self {
        self.options.escape = Some(escape);
        self
    }

    /// `FORCE_NOT_NULL (…)`: a field of these columns that matches the `NULL`
    /// string reads as that string rather than NULL, quoted or not. Each must
    /// be one the COPY supplies. An empty list forces nothing.
    pub fn force_not_null<C, I>(&mut self, columns: I) -> &mut Self
    where
        C: IntoName,
        I: IntoIterator<Item = C>,
    {
        self.options.force_not_null = CopyColumns::named(columns);
        self
    }

    /// `FORCE_NOT_NULL *`, for every column the COPY supplies.
    pub fn force_not_null_all(&mut self) -> &mut Self {
        self.options.force_not_null = Some(CopyColumns::All);
        self
    }

    /// `FORCE_NULL (…)`: a field of these columns that matches the `NULL`
    /// string reads as NULL even when quoted. Each must be one the COPY
    /// supplies. An empty list forces nothing.
    pub fn force_null<C, I>(&mut self, columns: I) -> &mut Self
    where
        C: IntoName,
        I: IntoIterator<Item = C>,
    {
        self.options.force_null = CopyColumns::named(columns);
        self
    }

    /// `FORCE_NULL *`, for every column the COPY supplies.
    pub fn force_null_all(&mut self) -> &mut Self {
        self.options.force_null = Some(CopyColumns::All);
        self
    }
}

/// `WHERE`: only the rows the condition holds for are inserted. PostgreSQL
/// refuses a subquery, an aggregate, a window function or a set-returning
/// function in it.
#[inherent]
impl<F: CopyFormat> ConditionalStatement for CopyFromStatement<F> {
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

/// Renders the statement with every value inlined as an escaped SQL literal.
/// This is its only rendering: COPY takes no bind parameters.
// [spec:pgorm:req:sql.render.copy]
impl<F: CopyFormat> fmt::Display for CopyFromStatement<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut sql = String::with_capacity(256);
        QueryBuilder.prepare_copy_from_statement(self, &mut sql);
        f.write_str(&sql)
    }
}

/// What a COPY TO reads: a whole table, or the rows a statement yields.
///
/// Its contents are private, so it holds only what a conversion into it
/// accepted. A table and a `SELECT` convert through [`IntoCopySource`]. A
/// write converts with [`TryFrom`], which refuses one without a `RETURNING`
/// list ([`Error::CopyWithoutReturning`]): COPY TO copies the rows a write
/// returns, and PostgreSQL refuses a write that returns none (`0A000`).
///
/// ```
/// use pgorm_query::{error::Error, tests_cfg::*, *};
///
/// let mut insert = Query::insert()
///     .into_table(Glyph::Table)
///     .columns([Glyph::Image])
///     .values_panic(["new".into()])
///     .to_owned();
/// assert_eq!(
///     CopySource::try_from(insert.clone()).err(),
///     Some(Error::CopyWithoutReturning { write: "INSERT" })
/// );
///
/// insert.returning_col(Glyph::Id);
/// let copy = CopyToStatement::new(CopySource::try_from(insert)?, CopyText);
/// assert_eq!(
///     copy.to_string(),
///     [
///         r#"COPY (INSERT INTO "glyph" ("image") VALUES ('new') RETURNING "id")"#,
///         r#"TO STDOUT WITH (FORMAT text)"#,
///     ]
///     .join(" ")
/// );
/// # Ok::<(), Error>(())
/// ```
///
/// A write is not an [`IntoCopySource`], so one goes to
/// [`CopyToStatement::new`] only through that check:
///
/// ```compile_fail,E0277
/// use pgorm_query::{tests_cfg::*, *};
///
/// CopyToStatement::new(Query::delete().from_table(Glyph::Table).to_owned(), CopyText);
/// ```
// [spec:pgorm:def:sql.ast.copy+3]
#[derive(Debug, Clone)]
pub struct CopySource(pub(crate) Source);

/// A [`CopySource`]'s contents. A `Query` is a `SELECT`, or a write carrying
/// a `RETURNING` list.
#[derive(Debug, Clone)]
pub(crate) enum Source {
    /// `COPY table TO STDOUT`: every column of the table, in its order.
    Table(TableName),
    /// `COPY (query) TO STDOUT`: the rows of a `SELECT`, or of a write's
    /// `RETURNING` list.
    Query(Box<SubQueryStatement>),
}

/// Conversion into the [`CopySource`] a [`CopyToStatement`] reads, for the
/// sources that always convert: a table name, a `SELECT`, and a
/// [`CopySource`] itself.
///
/// A column subset of a table is a `SELECT` of those columns: COPY TO's column
/// list says nothing that query does not. A write is a source only with a
/// `RETURNING` list, so it converts through [`CopySource`]'s [`TryFrom`]
/// instead.
// [spec:pgorm:def:sql.ast.copy+3]
pub trait IntoCopySource {
    fn into_copy_source(self) -> CopySource;
}

impl CopySource {
    /// The table a `COPY table TO STDOUT` reads, every column of it the
    /// server does not generate; `None` for a statement's rows.
    // [spec:pgorm:def:sql.ast.copy.parts]
    pub fn get_table(&self) -> Option<&TableName> {
        match &self.0 {
            Source::Table(table) => Some(table),
            Source::Query(_) => None,
        }
    }

    /// The statement whose rows a `COPY (query) TO STDOUT` reads: a `SELECT`,
    /// or a write carrying a `RETURNING` list; `None` for a table.
    // [spec:pgorm:def:sql.ast.copy.parts]
    pub fn get_query(&self) -> Option<&SubQueryStatement> {
        match &self.0 {
            Source::Table(_) => None,
            Source::Query(query) => Some(query),
        }
    }
}

impl IntoCopySource for CopySource {
    fn into_copy_source(self) -> CopySource {
        self
    }
}

impl<T> IntoCopySource for T
where
    T: IntoTableName,
{
    fn into_copy_source(self) -> CopySource {
        CopySource(Source::Table(self.into_table_name()))
    }
}

impl IntoCopySource for SelectStatement {
    fn into_copy_source(self) -> CopySource {
        CopySource(Source::Query(Box::new(SubQueryStatement::SelectStatement(
            self,
        ))))
    }
}

/// The write keyword `statement` names when it is a write that returns
/// nothing, which COPY TO cannot read; `None` for a `SELECT` and for a write
/// with a `RETURNING` list.
fn returns_nothing(statement: &SubQueryStatement) -> Option<&'static str> {
    match statement {
        SubQueryStatement::SelectStatement(_) => None,
        SubQueryStatement::InsertStatement(insert) => {
            insert.returning.is_none().then_some("INSERT")
        }
        SubQueryStatement::UpdateStatement(update) => {
            update.returning.is_none().then_some("UPDATE")
        }
        SubQueryStatement::DeleteStatement(delete) => {
            delete.returning.is_none().then_some("DELETE")
        }
        // `merge_action()` alone is a RETURNING list.
        SubQueryStatement::MergeStatement(merge) => {
            (merge.returning.is_none() && !merge.returns_action).then_some("MERGE")
        }
    }
}

/// A `SELECT` converts, and a write converts when it carries a `RETURNING`
/// list; a write without one is [`Error::CopyWithoutReturning`].
// [spec:pgorm:def:sql.ast.copy+3]
impl TryFrom<SubQueryStatement> for CopySource {
    type Error = Error;

    fn try_from(statement: SubQueryStatement) -> Result<Self, Error> {
        match returns_nothing(&statement) {
            Some(write) => Err(Error::CopyWithoutReturning { write }),
            None => Ok(Self(Source::Query(Box::new(statement)))),
        }
    }
}

/// Converts when the INSERT carries a `RETURNING` list.
impl TryFrom<InsertStatement> for CopySource {
    type Error = Error;

    fn try_from(insert: InsertStatement) -> Result<Self, Error> {
        Self::try_from(SubQueryStatement::InsertStatement(insert))
    }
}

/// Converts when the UPDATE carries a `RETURNING` list.
impl TryFrom<UpdateStatement> for CopySource {
    type Error = Error;

    fn try_from(update: UpdateStatement) -> Result<Self, Error> {
        Self::try_from(SubQueryStatement::UpdateStatement(update))
    }
}

/// Converts when the DELETE carries a `RETURNING` list.
impl TryFrom<DeleteStatement> for CopySource {
    type Error = Error;

    fn try_from(delete: DeleteStatement) -> Result<Self, Error> {
        Self::try_from(SubQueryStatement::DeleteStatement(delete))
    }
}

/// Converts when the MERGE carries a `RETURNING` list, which
/// [`returning_action`](MergeStatement::returning_action) alone also makes.
impl TryFrom<MergeStatement> for CopySource {
    type Error = Error;

    fn try_from(merge: MergeStatement) -> Result<Self, Error> {
        Self::try_from(SubQueryStatement::MergeStatement(Box::new(merge)))
    }
}

/// `COPY { table | (query) } TO STDOUT WITH (FORMAT …)`: rows sent by the
/// server to the client.
///
/// The format is the type parameter, as on [`CopyFromStatement`], and every
/// option here is one PostgreSQL accepts on COPY TO: `DEFAULT`, `FREEZE`,
/// `HEADER match`, `FORCE_NOT_NULL`, `FORCE_NULL` and `ON_ERROR` are COPY FROM's
/// alone.
///
/// A query's values are written into the SQL as escaped literals, because COPY
/// takes no bind parameters: [`Display`](fmt::Display) is the only rendering. A
/// write statement is a source only with a `RETURNING` list, which converting
/// it into a [`CopySource`] checks.
///
/// ```
/// use pgorm_query::{tests_cfg::*, *};
///
/// let whole = CopyToStatement::new(Glyph::Table, CopyBinary);
/// assert_eq!(whole.to_string(), r#"COPY "glyph" TO STDOUT WITH (FORMAT binary)"#);
///
/// let subset = CopyToStatement::new(
///     Query::select()
///         .columns([Glyph::Id, Glyph::Image])
///         .from(Glyph::Table)
///         .and_where(Expr::col(Glyph::Aspect).gt(2))
///         .take(),
///     CopyCsv,
/// )
/// .header()
/// .force_quote_all()
/// .to_owned();
/// assert_eq!(
///     subset.to_string(),
///     [
///         r#"COPY (SELECT "id", "image" FROM "glyph" WHERE "aspect" > 2) TO STDOUT"#,
///         r#"WITH (FORMAT csv, HEADER true, FORCE_QUOTE *)"#,
///     ]
///     .join(" ")
/// );
/// ```
///
/// `FORCE_NULL` reads a field, so it is COPY FROM's:
///
/// ```compile_fail,E0599
/// use pgorm_query::{tests_cfg::*, *};
///
/// CopyToStatement::new(Glyph::Table, CopyCsv).force_null_all();
/// ```
// [spec:pgorm:def:sql.ast.copy+3]
#[derive(Debug, Clone)]
pub struct CopyToStatement<F: CopyFormat> {
    pub(crate) source: CopySource,
    pub(crate) format: F,
    pub(crate) options: CopyOptions,
}

impl<F: CopyFormat> CopyToStatement<F> {
    /// COPY `source` to the client, in `format`.
    pub fn new<S>(source: S, format: F) -> Self
    where
        S: IntoCopySource,
    {
        Self {
            source: source.into_copy_source(),
            format,
            options: CopyOptions::default(),
        }
    }

    /// Refuse a `SELECT` source PostgreSQL's grammar would refuse: a tie
    /// limit with no ORDER BY or beside `SKIP LOCKED`
    /// ([`check_tie_limit`](SelectStatement::check_tie_limit)) and a WINDOW
    /// clause, or a reference to one, it refuses
    /// ([`check_windows`](SelectStatement::check_windows)).
    ///
    /// Rendering leaves both to its caller, as a `SELECT`'s own rendering
    /// does, and pgorm runs this before it sends a COPY TO. A table has
    /// nothing to check, and a write's `RETURNING` list was checked when it
    /// became a source. A `SELECT` nested inside the source is the server's
    /// to refuse, as it is for the two checks on their own.
    ///
    /// ```
    /// use pgorm_query::{error::Error, tests_cfg::*, *};
    ///
    /// let mut select = Query::select()
    ///     .column(Glyph::Image)
    ///     .from(Glyph::Table)
    ///     .limit_with_ties(3)
    ///     .to_owned();
    /// let copy = CopyToStatement::new(select.clone(), CopyText);
    /// assert_eq!(copy.check_source(), Err(Error::TiesWithoutOrder));
    ///
    /// select.order_by(Glyph::Aspect, Order::Asc);
    /// let copy = CopyToStatement::new(select, CopyText);
    /// assert_eq!(copy.check_source(), Ok(()));
    /// ```
    // [spec:pgorm:def:sql.ast.copy+3]
    pub fn check_source(&self) -> Result<(), Error> {
        match &self.source.0 {
            Source::Query(query) => match query.as_ref() {
                SubQueryStatement::SelectStatement(select) => select.check_clauses(),
                _ => Ok(()),
            },
            Source::Table(_) => Ok(()),
        }
    }

    /// What the rows are read from: a table, or a statement's rows.
    // [spec:pgorm:def:sql.ast.copy.parts]
    pub fn get_source(&self) -> &CopySource {
        &self.source
    }
}

impl<F: CopyDelimitedFormat> CopyToStatement<F> {
    /// `DELIMITER`: the character between fields, tab for text and comma for
    /// CSV by default. The server takes a single one-byte character and
    /// refuses a newline, a carriage return, and in text format a backslash,
    /// a period, a lowercase letter or a digit.
    pub fn delimiter(&mut self, delimiter: char) -> &mut Self {
        self.options.delimiter = Some(delimiter);
        self
    }

    /// `NULL`: the field written for SQL NULL — `\N` in text format and an
    /// unquoted empty field in CSV by default.
    pub fn null_string<S>(&mut self, null: S) -> &mut Self
    where
        S: Into<String>,
    {
        self.options.null = Some(null.into());
        self
    }

    /// `HEADER true`: write the column names as the first line.
    pub fn header(&mut self) -> &mut Self {
        self.options.header = CopyHeader::Present;
        self
    }

    /// `ENCODING`: the encoding to write the data in, when it is not the
    /// client encoding. It is the server's name for one (`'UTF8'`,
    /// `'LATIN1'`).
    pub fn encoding<S>(&mut self, encoding: S) -> &mut Self
    where
        S: Into<String>,
    {
        self.options.encoding = Some(encoding.into());
        self
    }
}

impl CopyToStatement<CopyCsv> {
    /// `QUOTE`: the character that quotes a field, `"` by default.
    pub fn quote(&mut self, quote: char) -> &mut Self {
        self.options.quote = Some(quote);
        self
    }

    /// `ESCAPE`: the character that escapes a quote inside a quoted field,
    /// the `QUOTE` character by default.
    pub fn escape(&mut self, escape: char) -> &mut Self {
        self.options.escape = Some(escape);
        self
    }

    /// `FORCE_QUOTE (…)`: quote every non-NULL value of these columns, not
    /// only the ones that need it. Each must be one the COPY writes. An empty
    /// list forces nothing.
    pub fn force_quote<C, I>(&mut self, columns: I) -> &mut Self
    where
        C: IntoName,
        I: IntoIterator<Item = C>,
    {
        self.options.force_quote = CopyColumns::named(columns);
        self
    }

    /// `FORCE_QUOTE *`, for every column.
    pub fn force_quote_all(&mut self) -> &mut Self {
        self.options.force_quote = Some(CopyColumns::All);
        self
    }
}

/// Renders the statement with every value inlined as an escaped SQL literal.
/// This is its only rendering: COPY takes no bind parameters.
// [spec:pgorm:req:sql.render.copy]
impl<F: CopyFormat> fmt::Display for CopyToStatement<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut sql = String::with_capacity(256);
        QueryBuilder.prepare_copy_to_statement(self, &mut sql);
        f.write_str(&sql)
    }
}

pub(crate) fn format_keyword<F: CopyFormat>(_: &F) -> &'static str {
    <F as sealed::Sealed>::KEYWORD
}
