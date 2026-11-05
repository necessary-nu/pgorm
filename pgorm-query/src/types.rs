//! Base types used throughout pgorm-query.

use crate::keywords::{self, Position};
use crate::{FunctionCall, ValueTuple, Values, expr::*, query::*, template::SqlTemplate};
use std::{any::Any, fmt, ops, sync::Arc};

/// A name in SQL: what an identifier position renders.
///
/// Spelled as the fork spells its other text contracts — `SqlText` is trusted
/// SQL, `SqlName` is a name — so the two positions a string can land in are
/// told apart by the trait it satisfies rather than by convention.
// [spec:pgorm:def:sql.types+9]
#[doc(alias = "Iden")]
pub trait SqlName: Any + Send + Sync {
    /// Write the identifier as PostgreSQL spells one: wrapped in double
    /// quotes, with any embedded double quote doubled.
    // [spec:pgorm:req:sql.render.ident-quoting+7]
    // [spec:pgorm:req:security.ident-oracle+16] (the quoting every registered
    // name position renders through, held to the identifier render oracle)
    fn prepare(&self, s: &mut dyn fmt::Write) {
        write!(s, "\"{}\"", self.quoted()).unwrap();
    }

    /// The identifier's text with embedded double quotes doubled, ready to sit
    /// between the quotes [`prepare`](Self::prepare) writes.
    // [spec:pgorm:req:sql.render.ident-quoting+7]
    fn quoted(&self) -> String {
        self.to_string().replace('"', "\"\"")
    }

    fn to_string(&self) -> String {
        let mut s = String::new();
        self.unquoted(&mut s);
        s
    }

    fn unquoted(&self, s: &mut dyn fmt::Write);

    /// The schema that qualifies this name wherever it names a table, so
    /// that [`IntoTableName`] reads it as `"schema"."name"`; `None` for a
    /// name that names its table bare. A name that knows its table's schema
    /// answers it here: pgorm's `EntityTrait` derive answers the entity's
    /// `schema_name`, which is how an entity handed straight to `from`, a
    /// join or a write target reads the table its own queries do, rather than
    /// whichever one `search_path` finds first.
    // [spec:pgorm:def:sql.types.table-ref+7]
    fn table_schema(&self) -> Option<&str> {
        None
    }
}

/// A name whose text is fixed by the program, and which therefore hands that
/// text out as a `&str` without rendering it.
///
/// This is the base identifier contract of anything that stands where a column
/// does: entities, columns and primary keys implement it through pgorm's
/// derives, the [`AliasName`] token implements it, and pgorm's key-column sets
/// are built from it. `Copy` and `'static` say the name is part of the shape of
/// the program; `Debug` is what makes a column printable in the generic code
/// that takes one.
// [spec:pgorm:def:sql.types+9]
#[doc(alias = "IdenStatic")]
pub trait StaticName: SqlName + Copy + fmt::Debug + 'static {
    /// The name as an unquoted string.
    fn as_str(&self) -> &str;
}

/// A name, type-erased and shared: one `Arc<dyn SqlName>` wide, cheap to
/// clone, and comparable.
///
/// Every identifier position in the AST holds one of these, so the type is
/// deliberately two words and no more — widening it widens every node.
#[derive(Debug)]
#[repr(transparent)]
#[doc(alias("DynIden", "SeaRc", "SharedIden"))]
pub struct Name(Arc<dyn SqlName>);

impl ops::Deref for Name {
    type Target = dyn SqlName;

    fn deref(&self) -> &Self::Target {
        ops::Deref::deref(&self.0)
    }
}

impl Clone for Name {
    fn clone(&self) -> Name {
        Name(Arc::clone(&self.0))
    }
}

/// Two identifiers are equal when their concrete types and their rendered text
/// both are.
///
/// The type is asked of the identifier by [`TypeId`](std::any::TypeId), not
/// read off the trait object's vtable address: Rust guarantees a vtable
/// neither unique per type nor stable across codegen units, so an address
/// comparison can answer that two runtime names both spelling `"id"` differ.
/// `SqlName` is bounded on [`Any`] so the erased value can still be asked, and
/// asking costs the identifier no width — a `Name` sits in nearly every
/// node of the AST.
// [spec:pgorm:def:sql.types+9]
impl PartialEq for Name {
    fn eq(&self, other: &Self) -> bool {
        let (this, that): (&dyn Any, &dyn Any) = (&*self.0, &*other.0);
        this.type_id() == that.type_id() && self.to_string() == other.to_string()
    }
}

impl Name {
    /// Erase a name the program already spells as a type.
    pub fn new<I>(i: I) -> Name
    where
        I: SqlName + 'static,
    {
        Name(Arc::new(i))
    }

    /// Mint a name from text computed at run time.
    ///
    /// The one route from a `String` into identifier position — there is no
    /// `impl IntoName for &str` — so a grep for `Name::runtime` finds every
    /// place a value becomes a name, which is the set a reader auditing for
    /// injection has to look at. A name written in the program says so with
    /// its type: an [`alias`] token, a derived enum, an entity column.
    ///
    /// The text is rendered as a QUOTED identifier like every other name, so
    /// this is not an escape hatch into SQL — only into naming.
    // [spec:pgorm:def:sql.types+9]
    pub fn runtime<T>(text: T) -> Name
    where
        T: Into<String>,
    {
        Name(Arc::new(RuntimeName(text.into())))
    }
}

#[doc(alias = "IntoIden")]
pub trait IntoName {
    fn into_name(self) -> Name;
}

impl fmt::Debug for dyn SqlName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.unquoted(formatter);
        Ok(())
    }
}

/// Column references
// [spec:pgorm:def:sql.types.column-ref+1]
// [spec:pgorm:def:sql.ast.keywords+6]
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ColumnRef {
    Column(Name),
    TableColumn(Name, Name),
    SchemaTableColumn(Name, Name, Name),
    Asterisk,
    TableAsterisk(Name),
    /// A column of a written row's old or new version, `old."col"`, which
    /// only a RETURNING list resolves.
    // [spec:pgorm:def:sql.ast.returning+3]
    RowColumn(crate::ReturningRow, Name),
    /// Every column of a written row's old or new version, `old.*`.
    // [spec:pgorm:def:sql.ast.returning+3]
    RowAsterisk(crate::ReturningRow),
}

// [spec:pgorm:def:sql.types.column-ref+1]
pub trait IntoColumnRef {
    fn into_column_ref(self) -> ColumnRef;
}

/// The name of a table, as a DDL statement targets it.
///
/// Every form denotes a table that exists in the catalogue, so this is what
/// `CREATE`/`ALTER`/`DROP`/`RENAME`/`TRUNCATE`, index and foreign-key targets
/// and comment targets accept. The query-position forms that name no table —
/// a subquery, a values list, a function call — live on [`FromItem`] and
/// cannot reach a DDL position.
///
/// A name reads in either position:
///
/// ```
/// use pgorm_query::{*, tests_cfg::*};
///
/// let name = (Name::runtime("public"), Glyph::Table).into_table_name();
/// assert_eq!(
///     Table::truncate(name.clone()).to_string(),
///     r#"TRUNCATE TABLE "public"."glyph""#
/// );
/// assert_eq!(
///     Query::select().column(Asterisk).from(name).to_string(),
///     r#"SELECT * FROM "public"."glyph""#
/// );
/// ```
///
/// A subquery is a [`FromItem`] and not a name, so it does not typecheck as a
/// DDL target:
///
/// ```compile_fail,E0277
/// use pgorm_query::{*, tests_cfg::*};
///
/// let sub = FromItem::SubQuery(
///     Query::select().column(Glyph::Id).from(Glyph::Table).take(),
///     Name::runtime("q"),
/// );
/// Table::truncate(sub);
/// ```
///
/// Nor does an aliased table: binding an alias makes it a [`NamedTable`], and
/// an alias means nothing to a statement that only names the catalogue entry:
///
/// ```compile_fail,E0277
/// use pgorm_query::{*, tests_cfg::*};
///
/// Table::truncate(Glyph::Table.into_named_table().alias(Name::runtime("g")));
/// ```
// [spec:pgorm:def:sql.types.table-ref+7]
// [spec:pgorm:sem:sql.ddl.panics+4/test]    the DDL-position panics are gone because the shapes
// that reached them no longer typecheck
/// A type name in cast or column-type position: optionally
/// schema-qualified, optionally an array. Each part renders bare or quoted
/// under the policy [`to_sql_string`](Self::to_sql_string) documents —
/// `tenant_a.status[]`, `"Tenant"."select"[]` — so a name is a name, never
/// SQL.
///
/// This is the *only* thing a cast carries as its type: one node shape, the
/// quoted-or-verbatim question answered inside the type rather than by
/// picking a different node.
// [spec:pgorm:def:sql.types.type-name+8]
// [spec:pgorm:req:sql.ast.cast-shape+2]
#[derive(Debug, Clone, PartialEq)]
pub struct TypeName {
    pub schema: Option<Name>,
    pub name: Name,
    pub array: bool,
    /// The type expression [`raw`](Self::raw) was given, which renders as
    /// written in place of `name`.
    ///
    /// Private, and holding the `&'static str` itself rather than a flag: a
    /// public flag could turn a quoted runtime name into SQL by assignment,
    /// and a flag over `name` would render whatever `name` was reassigned to.
    /// Only program text reaches here, through `raw`'s bound.
    verbatim: Option<&'static str>,
}

impl TypeName {
    /// A bare, non-array type name.
    pub fn new<T>(name: T) -> Self
    where
        T: IntoName,
    {
        Self {
            schema: None,
            name: name.into_name(),
            array: false,
            verbatim: None,
        }
    }

    /// A type EXPRESSION — `BIT(8)`, `numeric(12, 2)` — rendered verbatim,
    /// nothing quoted or escaped.
    ///
    /// The only constructor of a verbatim type, and the one
    /// [`Expr::cast_as_raw`](crate::Expr::cast_as_raw) and the derive's
    /// `select_as` / `save_as` attributes reach. Its argument is a literal
    /// written in the calling source: the text is program text the author
    /// already controls, never data, so rendering it as SQL adds no reach that
    /// writing the SQL by hand would not have. The `&'static str` bound is
    /// what enforces that — a runtime `String` cannot reach this constructor,
    /// so no value-derived text can become SQL here. Only that text is
    /// verbatim: a [`schema`](Self::schema) added to it is a name and renders
    /// under the part policy like any other, and reassigning `name`
    /// afterwards does not change what renders. Any type that arrives as a
    /// *name* — from a schema, a derive attribute, or anything a value could
    /// reach — takes [`new`](Self::new) and is quoted.
    ///
    /// The flag cannot be set any other way, so runtime text cannot be made
    /// verbatim after the fact:
    ///
    /// ```compile_fail,E0616
    /// use pgorm_query::{Name, TypeName};
    ///
    /// let mut ty = TypeName::new(Name::runtime("int4) + 1 --"));
    /// ty.verbatim = Some("int4");
    /// ```
    pub fn raw(type_expr: &'static str) -> Self {
        Self {
            schema: None,
            name: Name::runtime(type_expr),
            array: false,
            verbatim: Some(type_expr),
        }
    }

    /// Whether this is a [`raw`](Self::raw) type expression, rendered as
    /// written rather than as a name.
    pub fn is_verbatim(&self) -> bool {
        self.verbatim.is_some()
    }

    /// Qualify with a schema.
    pub fn schema<S>(mut self, schema: S) -> Self
    where
        S: IntoName,
    {
        self.schema = Some(schema.into_name());
        self
    }

    /// Mark as the array of the named type.
    pub fn array(mut self) -> Self {
        self.array = true;
        self
    }

    /// The SQL spelling: parts dot-joined, a structural `[]` suffix for
    /// arrays.
    ///
    /// A part renders bare when it is a lowercase identifier
    /// (`^[a-z_][a-z0-9_]*$`) and not a keyword PostgreSQL restricts —
    /// PostgreSQL folds unquoted names to lowercase, so the bare and quoted
    /// spellings are the same name there. Every other part renders as a
    /// QUOTED identifier, case preserved — so a name is a name, and neither
    /// text that is not one (`int4) + 100 --`) nor a keyword (`select`)
    /// becomes SQL. The one bare keyword is the grammar's own type spelling
    /// in an unqualified type (`integer`, which resolves to `int4`), because
    /// that is what a caller naming it means. A [`raw`](Self::raw) type
    /// expression is the one exception to all of this and renders as
    /// written — that text alone, the schema qualifying it still a name.
    // [spec:pgorm:def:sql.types.type-name+8]
    pub fn to_sql_string(&self) -> String {
        let mut out = String::new();
        let name = match &self.schema {
            Some(schema) => {
                Self::prepare_part(schema, Position::QualifiedType, &mut out);
                out.push('.');
                Position::QualifiedType
            }
            None => Position::Type,
        };
        match self.verbatim {
            Some(type_expr) => out.push_str(type_expr),
            None => Self::prepare_part(&self.name, name, &mut out),
        }
        if self.array {
            out.push_str("[]");
        }
        out
    }

    /// Write one name part at `position` under the policy
    /// [`to_sql_string`](Self::to_sql_string) documents: bare when
    /// [`keywords::bare`] allows it there, quoted otherwise.
    ///
    /// Shared with the render sites that emit a single caller-supplied name
    /// which is not a type — a function's name
    /// ([`Func::named`](crate::Func::named)) and an index access method
    /// ([`IndexType::Named`](crate::IndexType::Named)) — so one policy covers
    /// every name-shaped position, each position saying only which of the
    /// grammar's own forms it reads.
    // [spec:pgorm:req:sql.render.ident-quoting+7]
    // [spec:pgorm:def:sql.types.type-name+8]
    pub(crate) fn prepare_part(part: &Name, position: Position, out: &mut String) {
        let text = part.to_string();
        if keywords::bare(&text, position) {
            out.push_str(&text);
        } else {
            part.prepare(out);
        }
    }

    /// The unquoted dotted spelling — `tenant_a.status[]` — as a
    /// description of the type rather than SQL: a part that needs quoting is
    /// not quoted here, so this text must never be written into a statement.
    /// [`to_sql_string`](Self::to_sql_string) is the SQL spelling.
    pub fn raw_text(&self) -> String {
        let mut out = String::new();
        if let Some(schema) = &self.schema {
            out.push_str(&schema.to_string());
            out.push('.');
        }
        match self.verbatim {
            Some(type_expr) => out.push_str(type_expr),
            None => out.push_str(&self.name.to_string()),
        }
        if self.array {
            out.push_str("[]");
        }
        out
    }
}

impl<T> From<T> for TypeName
where
    T: IntoName,
{
    fn from(name: T) -> Self {
        Self::new(name)
    }
}

#[derive(Debug, Clone, PartialEq)]
#[doc(alias = "TableRef")]
pub enum TableName {
    /// Table identifier without any schema prefix
    Table(Name),
    /// Table identifier with schema prefix
    SchemaTable(Name, Name),
}

/// Conversion into the [`TableName`] a DDL statement targets.
// [spec:pgorm:def:sql.types.table-ref+7]
pub trait IntoTableName {
    /// Consume `self` and produce a [`TableName`]
    fn into_table_name(self) -> TableName;
}

/// A table name with an optional alias: the target of an `INSERT`, `UPDATE` or
/// `DELETE`, and the named form of a [`FromItem`].
///
/// PostgreSQL's write statements target a relation by name and may bind an
/// alias to it, and nothing else — so this is the type they take, and the
/// alias reads back in the statement:
///
/// ```
/// use pgorm_query::{*, tests_cfg::*};
///
/// let target = Glyph::Table.into_named_table().alias(Name::runtime("g"));
/// assert_eq!(
///     Query::delete()
///         .from_table(target)
///         .and_where(Expr::col((Name::runtime("g"), Glyph::Id)).eq(1))
///         .to_string(),
///     r#"DELETE FROM "glyph" AS "g" WHERE "g"."id" = 1"#
/// );
/// ```
///
/// The value-producing [`FromItem`] forms name no relation, so none of them
/// reaches a write target. A subquery does not:
///
/// ```compile_fail,E0277
/// use pgorm_query::{*, tests_cfg::*};
///
/// let sub = FromItem::SubQuery(
///     Query::select().column(Glyph::Id).from(Glyph::Table).take(),
///     Name::runtime("q"),
/// );
/// Query::insert().into_table(sub);
/// ```
///
/// nor a values list:
///
/// ```compile_fail,E0277
/// use pgorm_query::{*, tests_cfg::*};
///
/// let values = FromItem::ValuesList(
///     vec![(1i32,).into_value_tuple()],
///     Name::runtime("v"),
/// );
/// Query::update().table(values);
/// ```
///
/// nor a function call:
///
/// ```compile_fail,E0277
/// use pgorm_query::{*, tests_cfg::*};
///
/// let func = FromItem::FunctionCall(
///     Func::named(Name::runtime("generate_series")).arg(1i32),
///     Name::runtime("f"),
/// );
/// Query::delete().from_table(func);
/// ```
// [spec:pgorm:def:sql.types.table-ref+7]
#[derive(Debug, Clone, PartialEq)]
pub struct NamedTable {
    /// The table this reference names
    pub name: TableName,
    /// The alias bound to the name, when one is
    pub alias: Option<Name>,
}

/// Conversion into the [`NamedTable`] a DML statement targets.
// [spec:pgorm:def:sql.types.table-ref+7]
pub trait IntoNamedTable {
    /// Consume `self` and produce a [`NamedTable`]
    fn into_named_table(self) -> NamedTable;
}

/// An entry in a `FROM` clause or a join.
///
/// A named table carries its alias beside it rather than in the variant, so
/// aliasing is orthogonal to how the name is qualified; the value-producing
/// forms carry the alias Postgres requires of them.
// [spec:pgorm:def:sql.types.table-ref+7]
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
#[doc(alias = "TableRef")]
#[non_exhaustive]
pub enum FromItem {
    /// A table name with an optional alias
    Table(NamedTable),
    /// Subquery with alias
    SubQuery(SelectStatement, Name),
    /// Values list with alias
    ValuesList(Vec<ValueTuple>, Name),
    /// Function call with alias
    FunctionCall(FunctionCall, Name),
    /// `JSON_TABLE` with alias, built by
    /// [`JsonTable::alias`](crate::JsonTable::alias).
    // [spec:pgorm:def:sql.ast.json-table]
    JsonTable(Box<crate::JsonTable>, Name),
    /// A function with an ordinal column, column names or `ROWS FROM`, with
    /// alias, built by [`TableFunction::alias`](crate::TableFunction::alias).
    // [spec:pgorm:req:sql.ast.select.table-function+2]
    TableFunction(Box<crate::TableFunction>, Name),
    /// A table read through `TABLESAMPLE`, its alias on the table it samples.
    // [spec:pgorm:req:sql.ast.select.table-sample]
    TableSample(Box<crate::TableSample>),
    /// A validated SQL fragment with alias: text this crate did not build,
    /// standing where a relation stands.
    ///
    /// The fragment is a [`SqlTemplate`], so it arrives already paired with
    /// the values its `$N` markers number — usually through
    /// [`SqlTemplate::from_sql`], which reads the `$` grammar of real SQL.
    /// Rendering re-emits each paired value as a parameter of the enclosing
    /// statement, so the fragment's markers renumber into that statement's
    /// space and a fragment may sit in a query that binds values of its own:
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let fragment = SqlTemplate::from_sql(
    ///     r#"SELECT "id" FROM "glyph" WHERE "aspect" > $1"#,
    ///     [1i32.into()],
    /// )?;
    ///
    /// let (sql, values) = Query::select()
    ///     .column(Asterisk)
    ///     .from(FromItem::Template(fragment, Name::runtime("g")))
    ///     .and_where(Expr::col((Name::runtime("g"), Glyph::Id)).lt(9))
    ///     .build();
    ///
    /// assert_eq!(
    ///     sql,
    ///     "SELECT * FROM (SELECT \"id\" FROM \"glyph\" WHERE \"aspect\" > $1\n\
    ///      ) AS \"g\" WHERE \"g\".\"id\" < $2"
    /// );
    /// assert_eq!(values.0, vec![1i32.into(), 9i32.into()]);
    /// # Ok::<(), pgorm_query::error::Error>(())
    /// ```
    Template(SqlTemplate, Name),
}

/// Conversion into a [`FromItem`].
// [spec:pgorm:def:sql.types.table-ref+7]
pub trait IntoFromItem {
    /// Consume `self` and produce a [`FromItem`]
    fn into_from_item(self) -> FromItem;
}

/// Unary operator
// [spec:pgorm:def:sql.types.opers+6]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnOper {
    Not,
}

/// Binary operator
// [spec:pgorm:def:sql.types.opers+6]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BinOper {
    And,
    Or,
    Like,
    NotLike,
    /// `SIMILAR TO`: a match of the whole string against a pattern written in
    /// SQL's regular-expression dialect, LIKE's `%` and `_` beside `|`, `*`,
    /// `+`, `?`, `{m,n}`, `()` and `[]`.
    SimilarTo,
    /// `NOT SIMILAR TO`. The complement of [`BinOper::SimilarTo`].
    NotSimilarTo,
    Is,
    IsNot,
    /// `IS DISTINCT FROM`. Inequality that reads NULL as a value rather than
    /// as unknown: two NULLs are not distinct, a NULL and a non-NULL are, and
    /// the answer is never NULL.
    IsDistinctFrom,
    /// `IS NOT DISTINCT FROM`. The complement of
    /// [`BinOper::IsDistinctFrom`], and so null-safe equality.
    IsNotDistinctFrom,
    In,
    NotIn,
    Between,
    NotBetween,
    /// `BETWEEN SYMMETRIC`. `BETWEEN` with its bounds sorted first, so the
    /// range holds whichever order the two are given in.
    BetweenSymmetric,
    /// `NOT BETWEEN SYMMETRIC`. The complement of
    /// [`BinOper::BetweenSymmetric`].
    NotBetweenSymmetric,
    Equal,
    NotEqual,
    SmallerThan,
    GreaterThan,
    SmallerThanOrEqual,
    GreaterThanOrEqual,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    /// `<<`. A left shift, and over ranges and multiranges the test that the
    /// left lies strictly left of the right: one operator, so one variant.
    LShift,
    /// `>>`. A right shift, and over ranges and multiranges the test that the
    /// left lies strictly right of the right.
    RShift,
    As,
    ILike,
    NotILike,
    Matches,
    Contains,
    Contained,
    Concatenate,
    Overlap,
    Similarity,
    WordSimilarity,
    StrictWordSimilarity,
    SimilarityDistance,
    WordSimilarityDistance,
    StrictWordSimilarityDistance,
    /// `->`. Retrieves JSON field as JSON value.
    GetJsonField,
    /// `->>`. Retrieves JSON field and casts it to an appropriate SQL type.
    CastJsonField,
    /// `#>`. Retrieves the JSON value at a `text[]` path.
    GetJsonPath,
    /// `#>>`. Retrieves the value at a `text[]` path as text.
    CastJsonPath,
    /// `?`. Whether a top-level key or string element exists.
    HasJsonKey,
    /// `?|`. Whether any key in a `text[]` list exists.
    HasAnyJsonKeys,
    /// `?&`. Whether every key in a `text[]` list exists.
    HasAllJsonKeys,
    /// `@?`. Whether a `jsonpath` returns any item for the `jsonb` on its
    /// left.
    JsonPathExists,
    /// `~` Regex operator.
    Regex,
    /// `~*`. Regex operator with case insensitive matching.
    RegexCaseInsensitive,
    /// `!~`. Whether the text does not match a regular expression,
    /// case-sensitively: the complement of [`BinOper::Regex`].
    NotRegex,
    /// `!~*`. Whether the text does not match a regular expression when case
    /// is ignored: the complement of [`BinOper::RegexCaseInsensitive`].
    NotRegexCaseInsensitive,
    /// `&<`. Whether a range or multirange does not extend to the right of
    /// the other.
    NotExtendRightOf,
    /// `&>`. Whether a range or multirange does not extend to the left of the
    /// other.
    NotExtendLeftOf,
    /// `-|-`. Whether two ranges or multiranges are adjacent: they share no
    /// value, and nothing lies between them.
    AdjacentTo,
    /// `AT TIME ZONE`. Reinterprets a timestamp in the zone its right operand
    /// names, converting between `timestamp` and `timestamptz` in whichever
    /// direction the left operand's type calls for.
    AtTimeZone,
    EuclideanDistance,
    NegativeInnerProduct,
    CosineDistance,
    /// An operator this enum has no variant for, written verbatim.
    Raw(&'static str),
}

/// Join types that carry an `ON` constraint
// [spec:pgorm:req:sql.ast.select.join+1]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinType {
    Join,
    InnerJoin,
    LeftJoin,
    RightJoin,
    FullOuterJoin,
}

/// A join and the constraint it carries.
///
/// `CROSS JOIN` is the one join PostgreSQL takes without an `ON` clause and
/// every other join requires one, so the two travel together: a constrained
/// cross join and an unconstrained inner join both fail to construct.
// [spec:pgorm:req:sql.ast.select.join+1]
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum JoinKind {
    Cross,
    Qualified(JoinType, JoinOn),
}

/// Nulls order
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NullOrdering {
    First,
    Last,
}

/// Order expression
#[derive(Debug, Clone, PartialEq)]
pub struct OrderExpr {
    pub(crate) expr: SimpleExpr,
    pub(crate) order: Order,
    pub(crate) nulls: Option<NullOrdering>,
}

/// Join on types
// [spec:pgorm:req:sql.render.joins+2]
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum JoinOn {
    Condition(Box<ConditionHolder>),
}

/// Ordering options
// [spec:pgorm:req:sql.ast.order+3]
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Order {
    Asc,
    Desc,
    Field(Values),
}

#[path = "types_alias.rs"]
mod types_alias;
pub use types_alias::*;

#[path = "types_collation.rs"]
mod types_collation;
pub use types_collation::*;

/// A name computed at run time, reachable only through
/// [`Name::runtime`].
///
/// Private on purpose: `Name::runtime` is the one door a `String` walks
/// through to reach identifier position, so every entry of
/// possibly-untrusted text into a name is one grep away.
#[derive(Debug, Clone)]
struct RuntimeName(String);

/// Asterisk ("*")
///
/// Express the asterisk without table prefix.
///
/// # Examples
///
/// ```
/// use pgorm_query::{tests_cfg::*, *};
///
/// let query = Query::select()
///     .column(Asterisk)
///     .from(Char::Table)
///     .to_owned();
///
/// assert_eq!(
///     query.to_string(),
///     r#"SELECT * FROM "character""#
/// );
/// ```
///
/// Express the asterisk with table prefix.
///
/// Examples
///
/// ```
/// use pgorm_query::{tests_cfg::*, *};
///
/// let query = Query::select()
///     .column((Char::Table, Asterisk))
///     .from(Char::Table)
///     .to_owned();
///
/// assert_eq!(
///     query.to_string(),
///     r#"SELECT "character".* FROM "character""#
/// );
/// ```
#[derive(Default, Debug, Clone, Copy)]
#[doc(alias = "asterisk")]
pub struct Asterisk;

/// SQL Keywords
// [spec:pgorm:def:sql.ast.keywords+6]
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Keyword {
    Null,
    CurrentDate,
    CurrentTime,
    CurrentTimestamp,
    /// `VALUE`: the value a domain's `CHECK` constraint is tested against
    /// ([`Expr::domain_value`](crate::Expr::domain_value)).
    // [spec:pgorm:req:sql.ddl.domain+3]
    Value,
}

/// A `LIKE`, `ILIKE` or `SIMILAR TO` pattern, with the `ESCAPE` tail the
/// grammar admits only there.
// [spec:pgorm:req:sql.ast.expr.similar-to+1]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LikeExpr {
    pub(crate) pattern: String,
    pub(crate) escape: Option<LikeEscape>,
}

/// What a pattern's `ESCAPE` holds: one character, or nothing, which turns
/// escaping off. PostgreSQL refuses anything longer (`22025`), so nothing
/// longer has a spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LikeEscape {
    Char(char),
    Off,
}

pub trait IntoLikeExpr {
    fn into_like_expr(self) -> LikeExpr;
}

/// SubQuery operators
// [spec:pgorm:def:sql.ast.expr+7]
#[derive(Debug, Copy, Clone, PartialEq)]
#[non_exhaustive]
pub enum SubQueryOper {
    Exists,
    Any,
    Some,
    All,
    /// `ARRAY(..)`: the one column a subquery yields, collected into an array.
    Array,
}

// Impl begins

impl<T: 'static> IntoName for T
where
    T: SqlName,
{
    fn into_name(self) -> Name {
        Name::new(self)
    }
}

impl IntoName for Name {
    fn into_name(self) -> Name {
        self
    }
}

impl IntoColumnRef for ColumnRef {
    fn into_column_ref(self) -> ColumnRef {
        self
    }
}

impl<T: 'static> IntoColumnRef for T
where
    T: IntoName,
{
    fn into_column_ref(self) -> ColumnRef {
        ColumnRef::Column(self.into_name())
    }
}

impl IntoColumnRef for Asterisk {
    fn into_column_ref(self) -> ColumnRef {
        ColumnRef::Asterisk
    }
}

impl<S: 'static, T: 'static> IntoColumnRef for (S, T)
where
    S: IntoName,
    T: IntoName,
{
    fn into_column_ref(self) -> ColumnRef {
        ColumnRef::TableColumn(self.0.into_name(), self.1.into_name())
    }
}

impl<T: 'static> IntoColumnRef for (T, Asterisk)
where
    T: IntoName,
{
    fn into_column_ref(self) -> ColumnRef {
        ColumnRef::TableAsterisk(self.0.into_name())
    }
}

impl<S: 'static, T: 'static, U: 'static> IntoColumnRef for (S, T, U)
where
    S: IntoName,
    T: IntoName,
    U: IntoName,
{
    fn into_column_ref(self) -> ColumnRef {
        ColumnRef::SchemaTableColumn(self.0.into_name(), self.1.into_name(), self.2.into_name())
    }
}

impl IntoTableName for TableName {
    fn into_table_name(self) -> TableName {
        self
    }
}

// [spec:pgorm:def:sql.types.table-ref+7]
impl<T: 'static> IntoTableName for T
where
    T: IntoName,
{
    fn into_table_name(self) -> TableName {
        let name = self.into_name();
        match name.table_schema().map(Name::runtime) {
            Some(schema) => TableName::SchemaTable(schema, name),
            None => TableName::Table(name),
        }
    }
}

impl<S: 'static, T: 'static> IntoTableName for (S, T)
where
    S: IntoName,
    T: IntoName,
{
    fn into_table_name(self) -> TableName {
        TableName::SchemaTable(self.0.into_name(), self.1.into_name())
    }
}

// [spec:pgorm:def:sql.types.table-ref+7]
impl TableName {
    /// The table identifier, without its schema
    pub fn table(&self) -> &Name {
        match self {
            Self::Table(table) | Self::SchemaTable(_, table) => table,
        }
    }

    /// The schema identifier, when the name carries one
    pub fn schema(&self) -> Option<&Name> {
        match self {
            Self::Table(_) => None,
            Self::SchemaTable(schema, _) => Some(schema),
        }
    }
}

impl IntoNamedTable for NamedTable {
    fn into_named_table(self) -> NamedTable {
        self
    }
}

impl<T: 'static> IntoNamedTable for T
where
    T: IntoTableName,
{
    fn into_named_table(self) -> NamedTable {
        NamedTable {
            name: self.into_table_name(),
            alias: None,
        }
    }
}

// [spec:pgorm:def:sql.types.table-ref+7]
impl NamedTable {
    /// Bind an alias to the name, replacing any alias already bound
    pub fn alias<A>(self, alias: A) -> Self
    where
        A: IntoName,
    {
        Self {
            name: self.name,
            alias: Some(alias.into_name()),
        }
    }

    /// The identifier a column of this table is qualified by: the alias when
    /// one is bound, otherwise the table identifier.
    pub fn qualifier(&self) -> &Name {
        self.alias.as_ref().unwrap_or_else(|| self.name.table())
    }
}

impl From<TableName> for NamedTable {
    fn from(name: TableName) -> Self {
        Self { name, alias: None }
    }
}

impl IntoFromItem for FromItem {
    fn into_from_item(self) -> FromItem {
        self
    }
}

impl<T: 'static> IntoFromItem for T
where
    T: IntoNamedTable,
{
    fn into_from_item(self) -> FromItem {
        FromItem::Table(self.into_named_table())
    }
}

impl From<NamedTable> for FromItem {
    fn from(table: NamedTable) -> Self {
        Self::Table(table)
    }
}

impl From<TableName> for FromItem {
    fn from(name: TableName) -> Self {
        Self::Table(name.into())
    }
}

// [spec:pgorm:def:sql.types.table-ref+7]
impl FromItem {
    /// Add or replace the current alias
    pub fn alias<A>(self, alias: A) -> Self
    where
        A: IntoName,
    {
        match self {
            Self::Table(table) => Self::Table(table.alias(alias)),
            Self::SubQuery(statement, _) => Self::SubQuery(statement, alias.into_name()),
            Self::ValuesList(values, _) => Self::ValuesList(values, alias.into_name()),
            Self::FunctionCall(func, _) => Self::FunctionCall(func, alias.into_name()),
            Self::Template(template, _) => Self::Template(template, alias.into_name()),
            Self::JsonTable(table, _) => Self::JsonTable(table, alias.into_name()),
            Self::TableFunction(function, _) => Self::TableFunction(function, alias.into_name()),
            Self::TableSample(mut sample) => {
                sample.table = sample.table.alias(alias);
                Self::TableSample(sample)
            }
        }
    }

    /// The name of the table this item reads, when it names one
    pub fn table_name(&self) -> Option<&TableName> {
        match self {
            Self::Table(table) => Some(&table.name),
            Self::TableSample(sample) => Some(&sample.table.name),
            Self::SubQuery(_, _)
            | Self::ValuesList(_, _)
            | Self::FunctionCall(_, _)
            | Self::Template(_, _)
            | Self::JsonTable(_, _)
            | Self::TableFunction(_, _) => None,
        }
    }

    /// The identifier a column of this item is qualified by: the alias when
    /// one is bound, otherwise the table identifier.
    pub fn qualifier(&self) -> &Name {
        match self {
            Self::Table(table) => table.qualifier(),
            Self::TableSample(sample) => sample.table.qualifier(),
            Self::SubQuery(_, alias)
            | Self::ValuesList(_, alias)
            | Self::FunctionCall(_, alias)
            | Self::Template(_, alias)
            | Self::JsonTable(_, alias)
            | Self::TableFunction(_, alias) => alias,
        }
    }
}

// [spec:pgorm:def:sql.types+9]
impl SqlName for RuntimeName {
    fn unquoted(&self, s: &mut dyn fmt::Write) {
        write!(s, "{}", self.0).unwrap();
    }
}

impl LikeExpr {
    pub fn new<T>(pattern: T) -> Self
    where
        T: Into<String>,
    {
        Self {
            pattern: pattern.into(),
            escape: None,
        }
    }

    /// Escape with `c`, `ESCAPE 'c'`, in place of the default backslash.
    pub fn escape(self, c: char) -> Self {
        Self {
            pattern: self.pattern,
            escape: Some(LikeEscape::Char(c)),
        }
    }

    /// Turn escaping off, `ESCAPE ''`: every character of the pattern, a
    /// backslash included, is then literal or a wildcard.
    ///
    /// ```
    /// use pgorm_query::{*, tests_cfg::*};
    ///
    /// let query = Query::select()
    ///     .column(Glyph::Image)
    ///     .from(Glyph::Table)
    ///     .and_where(Expr::col(Glyph::Image).like(LikeExpr::new(r"a").no_escape()))
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     query.to_string(),
    ///     r#"SELECT "image" FROM "glyph" WHERE "image" LIKE E'a\b' ESCAPE ''"#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ast.expr.similar-to+1]
    pub fn no_escape(self) -> Self {
        Self {
            pattern: self.pattern,
            escape: Some(LikeEscape::Off),
        }
    }
}

impl IntoLikeExpr for LikeExpr {
    fn into_like_expr(self) -> LikeExpr {
        self
    }
}

impl<T> IntoLikeExpr for T
where
    T: Into<String>,
{
    fn into_like_expr(self) -> LikeExpr {
        LikeExpr::new(self)
    }
}

#[cfg(test)]
mod tests {
    pub use crate::{tests_cfg::*, *};
    pub use Character as CharReexport;
    use pretty_assertions::assert_eq;

    // [spec:pgorm:def:sql.types.type-name+8/test]    only the text `raw` was
    // given renders verbatim: a schema on a raw type is a name, and
    // reassigning `name` afterwards changes nothing that renders
    #[test]
    fn raw_type_renders_only_its_static_text() {
        let schema = Name::runtime("x\"; DROP TABLE t; --");
        let qualified = TypeName::raw("numeric(12, 2)").schema(schema).array();
        assert_eq!(
            qualified.to_sql_string(),
            r#""x""; DROP TABLE t; --".numeric(12, 2)[]"#
        );
        let mut renamed = TypeName::raw("numeric(12, 2)");
        renamed.name = Name::runtime("int4) + 1 --");
        assert!(renamed.is_verbatim());
        assert_eq!(renamed.to_sql_string(), "numeric(12, 2)");
        assert_eq!(renamed.raw_text(), "numeric(12, 2)");
        assert!(!TypeName::new(Name::runtime("numeric")).is_verbatim());
    }

    #[test]
    fn test_identifier() {
        let query = Query::select()
            .column(Name::runtime("hello-World_"))
            .to_owned();

        assert_eq!(query.to_string(), r#"SELECT "hello-World_""#);
    }

    // [spec:pgorm:def:sql.types+9/test]
    #[test]
    fn test_quoted_identifier_1() {
        let query = Query::select().column(Name::runtime("hel\"lo")).to_owned();

        assert_eq!(query.to_string(), r#"SELECT "hel""lo""#);
    }

    #[test]
    fn test_quoted_identifier_2() {
        let query = Query::select()
            .column(Name::runtime("hel\"\"lo"))
            .to_owned();

        assert_eq!(query.to_string(), r#"SELECT "hel""""lo""#);
    }

    // [spec:pgorm:def:sql.types+9/test]
    #[test]
    fn test_cmp_identifier() {
        type CharLocal = Character;

        assert_eq!(
            ColumnRef::Column(Character::Id.into_name()),
            ColumnRef::Column(Character::Id.into_name())
        );
        assert_eq!(
            ColumnRef::Column(Character::Id.into_name()),
            ColumnRef::Column(Char::Id.into_name())
        );
        assert_eq!(
            ColumnRef::Column(Character::Id.into_name()),
            ColumnRef::Column(CharLocal::Id.into_name())
        );
        assert_eq!(
            ColumnRef::Column(Character::Id.into_name()),
            ColumnRef::Column(CharReexport::Id.into_name())
        );
        assert_eq!(
            ColumnRef::Column(Name::runtime("id")),
            ColumnRef::Column(Name::runtime("id"))
        );
        assert_ne!(
            ColumnRef::Column(Name::runtime("id")),
            ColumnRef::Column(Name::runtime("id_"))
        );
        assert_ne!(
            ColumnRef::Column(Character::Id.into_name()),
            ColumnRef::Column(Name::runtime("id"))
        );
        assert_ne!(
            ColumnRef::Column(Character::Id.into_name()),
            ColumnRef::Column(Character::Table.into_name())
        );
        assert_ne!(
            ColumnRef::Column(Character::Id.into_name()),
            ColumnRef::Column(Font::Id.into_name())
        );
    }
}
