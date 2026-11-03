use core::fmt;

use crate::{
    IntoName, Name, PgInterval, QueryBuilder, SqlName, SqlWriter, SqlWriterValues, value::Values,
};

#[path = "extension_composite.rs"]
mod composite;
#[path = "extension_composite_alter.rs"]
mod composite_alter;
#[path = "extension_range.rs"]
mod range;

pub use composite::CompositeAttribute;
pub use composite_alter::{AttributeChange, AttributeRenameStatement, CompositeAlterStatement};
pub use range::RangeDefinition;

/// Creates a new "CREATE or DROP EXTENSION" statement for PostgreSQL
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Extension;

impl Extension {
    /// Creates a new [`ExtensionCreateStatement`] over the extension it creates
    pub fn create<T>(name: T) -> ExtensionCreateStatement
    where
        T: IntoName,
    {
        ExtensionCreateStatement::new(name)
    }

    /// Creates a new [`ExtensionDropStatement`] over the extension it drops
    pub fn drop<T>(name: T) -> ExtensionDropStatement
    where
        T: IntoName,
    {
        ExtensionDropStatement::new(name)
    }
}

/// Creates a new "CREATE EXTENSION" statement for PostgreSQL
///
/// # Synopsis
///
/// ```ignore
/// CREATE EXTENSION [ IF NOT EXISTS ] extension_name
///     [ WITH ] [ SCHEMA schema_name ]
///              [ VERSION version ]
///              [ CASCADE ]
/// ```
///
/// # Example
///
/// Creates the "ltree" extension if it doesn't exists.
///
/// ```
/// use pgorm_query::{extension::Extension, *};
///
/// assert_eq!(
///     Extension::create(Name::runtime("ltree"))
///         .schema(Name::runtime("public"))
///         .version("v0.1.0")
///         .cascade()
///         .if_not_exists()
///         .to_string(),
///     r#"CREATE EXTENSION IF NOT EXISTS "ltree" WITH SCHEMA "public" VERSION 'v0.1.0' CASCADE"#
/// );
/// ```
///
/// The extension name is taken by the constructor, because a statement that
/// never names one renders the zero-length delimited identifier PostgreSQL
/// rejects (`[dec:pgorm:invalid-states-unrepresentable]`):
///
/// ```compile_fail,E0061
/// use pgorm_query::{extension::Extension, *};
///
/// Extension::create().schema(Name::runtime("public"));
/// ```
///
/// # References
///
/// [Refer to the PostgreSQL Documentation][1]
///
/// [1]: https://www.postgresql.org/docs/current/sql-createextension.html
// [spec:pgorm:req:sql.ddl.extension+5]
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionCreateStatement {
    pub(crate) name: Name,
    /// The schema the extension's objects are created in — an identifier, the
    /// same type every other schema qualifier in the crate is, because that is
    /// what it renders as.
    pub(crate) schema: Option<Name>,
    /// The version, a string literal in the grammar and so genuinely text.
    pub(crate) version: Option<String>,

    /// Conditional to execute query based on existance of the extension.
    pub(crate) if_not_exists: bool,

    /// Determines the presence of the `RESTRICT` statement
    pub(crate) cascade: bool,
}

impl ExtensionCreateStatement {
    /// Construct a new statement over the extension it creates
    pub fn new<T>(name: T) -> Self
    where
        T: IntoName,
    {
        Self {
            name: name.into_name(),
            schema: None,
            version: None,
            if_not_exists: false,
            cascade: false,
        }
    }

    /// Uses "WITH SCHEMA" on Create Extension Statement.
    ///
    /// The schema is a name and renders as a quoted identifier, so the bound
    /// is the identifier bound every other schema position takes.
    // [spec:pgorm:req:sql.render.ident-quoting+7]
    pub fn schema<T: IntoName>(&mut self, schema: T) -> &mut Self {
        self.schema = Some(schema.into_name());
        self
    }

    /// Uses "VERSION" on Create Extension Statement.
    pub fn version<T: Into<String>>(&mut self, version: T) -> &mut Self {
        self.version = Some(version.into());
        self
    }

    /// Uses "CASCADE" on Create Extension Statement.
    pub fn cascade(&mut self) -> &mut Self {
        self.cascade = true;
        self
    }

    /// Uses "IF NOT EXISTS" on Create Extension Statement.
    pub fn if_not_exists(&mut self) -> &mut Self {
        self.if_not_exists = true;
        self
    }
}

/// Creates a new "DROP EXTENSION" statement for PostgreSQL
///
/// # Synopsis
///
/// ```ignore
/// DROP EXTENSION [ IF EXISTS ] name [, ...] [ CASCADE | RESTRICT ]
/// ```
///
/// # Example
///
/// Drops the "ltree" extension if it exists.
///
/// ```
/// use pgorm_query::{extension::Extension, *};
///
/// assert_eq!(
///     Extension::drop(Name::runtime("ltree")).cascade().if_exists().to_string(),
///     r#"DROP EXTENSION IF EXISTS "ltree" CASCADE"#
/// );
/// ```
///
/// The extension name is taken by the constructor, for the reason it is on
/// [`ExtensionCreateStatement`]:
///
/// ```compile_fail,E0061
/// use pgorm_query::{extension::Extension, *};
///
/// Extension::drop().if_exists();
/// ```
///
/// # References
///
/// [Refer to the PostgreSQL Documentation][1]
///
/// [1]: https://www.postgresql.org/docs/current/sql-createextension.html
// [spec:pgorm:req:sql.ddl.extension+5]
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionDropStatement {
    pub(crate) name: Name,

    /// Conditional to execute query based on existance of the extension.
    pub(crate) if_exists: bool,

    /// The drop behaviour, at most one of `CASCADE` and `RESTRICT`.
    pub(crate) option: Option<ExtensionDropOpt>,
}

/// The drop behaviour of a `DROP EXTENSION` statement.
///
/// PostgreSQL takes one of `CASCADE` and `RESTRICT`, never both, so the two
/// spellings share one slot.
// [spec:pgorm:req:sql.ddl.extension+5]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionDropOpt {
    Cascade,
    Restrict,
}

impl ExtensionDropStatement {
    /// Construct a new statement over the extension it drops
    pub fn new<T>(name: T) -> Self
    where
        T: IntoName,
    {
        Self {
            name: name.into_name(),
            if_exists: false,
            option: None,
        }
    }

    /// Uses "IF EXISTS" on Drop Extension Statement.
    pub fn if_exists(&mut self) -> &mut Self {
        self.if_exists = true;
        self
    }

    /// Uses "CASCADE" on Drop Extension Statement, replacing any "RESTRICT".
    pub fn cascade(&mut self) -> &mut Self {
        self.option = Some(ExtensionDropOpt::Cascade);
        self
    }

    /// Uses "RESTRICT" on Drop Extension Statement, replacing any "CASCADE".
    pub fn restrict(&mut self) -> &mut Self {
        self.option = Some(ExtensionDropOpt::Restrict);
        self
    }
}

macro_rules! impl_extension_statement_builder {
    ( $struct_name: ident, $func_name: ident ) => {
        impl $struct_name {
            /// Build the SQL statement into the given sink, returning the sink's text
            pub fn build_collect(&self, sql: &mut dyn SqlWriter) -> String {
                QueryBuilder.$func_name(self, sql);
                sql.to_string()
            }
        }

        /// Renders the statement with every value inlined as an escaped SQL
        /// literal. This is its only rendering: it binds nothing, so there is
        /// no placeholder form to choose between.
        // [spec:pgorm:req:sql.ddl+12]
        impl fmt::Display for $struct_name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                let mut sql = String::with_capacity(256);
                QueryBuilder.$func_name(self, &mut sql);
                f.write_str(&sql)
            }
        }
    };
}

impl_extension_statement_builder!(ExtensionCreateStatement, prepare_extension_create_statement);
impl_extension_statement_builder!(ExtensionDropStatement, prepare_extension_drop_statement);

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn creates_a_stmt_for_create_extension() {
        let create_extension_stmt = Extension::create(PgLTree)
            .schema(Name::runtime("public"))
            .version("v0.1.0")
            .cascade()
            .if_not_exists()
            .to_owned();

        assert_eq!(create_extension_stmt.name.to_string(), "ltree");
        assert_eq!(
            create_extension_stmt.schema.as_ref().map(|s| s.to_string()),
            Some("public".to_string())
        );
        assert_eq!(create_extension_stmt.version, Some("v0.1.0".to_string()));
        assert!(create_extension_stmt.cascade);
        assert!(create_extension_stmt.if_not_exists);
    }

    #[test]
    fn creates_a_stmt_for_drop_extension() {
        let drop_extension_stmt = Extension::drop(PgLTree)
            .cascade()
            .if_exists()
            .restrict()
            .to_owned();

        assert_eq!(drop_extension_stmt.name.to_string(), "ltree");
        assert!(drop_extension_stmt.if_exists);
        assert_eq!(drop_extension_stmt.option, Some(ExtensionDropOpt::Restrict));
    }
}

// [spec:pgorm:def:sql.types.column-type+11]
impl fmt::Display for PgInterval {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let (fields, precision) = match self {
            PgInterval::Year => ("YEAR", None),
            PgInterval::Month => ("MONTH", None),
            PgInterval::Day => ("DAY", None),
            PgInterval::Hour => ("HOUR", None),
            PgInterval::Minute => ("MINUTE", None),
            PgInterval::Second(precision) => ("SECOND", *precision),
            PgInterval::YearToMonth => ("YEAR TO MONTH", None),
            PgInterval::DayToHour => ("DAY TO HOUR", None),
            PgInterval::DayToMinute => ("DAY TO MINUTE", None),
            PgInterval::DayToSecond(precision) => ("DAY TO SECOND", *precision),
            PgInterval::HourToMinute => ("HOUR TO MINUTE", None),
            PgInterval::HourToSecond(precision) => ("HOUR TO SECOND", *precision),
            PgInterval::MinuteToSecond(precision) => ("MINUTE TO SECOND", *precision),
        };
        write!(f, "{fields}")?;
        match precision {
            Some(precision) => write!(f, "({precision})"),
            None => Ok(()),
        }
    }
}

impl TryFrom<String> for PgInterval {
    type Error = String;

    fn try_from(field: String) -> Result<Self, Self::Error> {
        PgInterval::try_from(field.as_str())
    }
}

impl TryFrom<&String> for PgInterval {
    type Error = String;

    fn try_from(field: &String) -> Result<Self, Self::Error> {
        PgInterval::try_from(field.as_str())
    }
}

impl TryFrom<&str> for PgInterval {
    type Error = String;

    fn try_from(field: &str) -> Result<Self, Self::Error> {
        match field.trim_start().trim_end().to_uppercase().as_ref() {
            "YEAR" => Ok(PgInterval::Year),
            "MONTH" => Ok(PgInterval::Month),
            "DAY" => Ok(PgInterval::Day),
            "HOUR" => Ok(PgInterval::Hour),
            "MINUTE" => Ok(PgInterval::Minute),
            "SECOND" => Ok(PgInterval::Second(None)),
            "YEAR TO MONTH" => Ok(PgInterval::YearToMonth),
            "DAY TO HOUR" => Ok(PgInterval::DayToHour),
            "DAY TO MINUTE" => Ok(PgInterval::DayToMinute),
            "DAY TO SECOND" => Ok(PgInterval::DayToSecond(None)),
            "HOUR TO MINUTE" => Ok(PgInterval::HourToMinute),
            "HOUR TO SECOND" => Ok(PgInterval::HourToSecond(None)),
            "MINUTE TO SECOND" => Ok(PgInterval::MinuteToSecond(None)),
            field => Err(format!(
                "Cannot turn \"{field}\" into a Postgres interval field",
            )),
        }
    }
}

/// PostgreSQL `ltree` extension type.
///
/// `ltree` stores a raber path which in this struct is represented as the
/// tuple's first value.
///
/// # PostcreSQL Reference
///
/// The following set of SQL statements can be used to create a table with
/// a `ltree` column. Here the `ltree` column is called `path`.
///
/// The `path` column is then populated to generate the tree.
///
/// ```sql
/// CREATE TABLE test (path ltree);
/// INSERT INTO test VALUES ('Top');
/// INSERT INTO test VALUES ('Top.Science');
/// INSERT INTO test VALUES ('Top.Science.Astronomy');
/// INSERT INTO test VALUES ('Top.Science.Astronomy.Astrophysics');
/// INSERT INTO test VALUES ('Top.Science.Astronomy.Cosmology');
/// INSERT INTO test VALUES ('Top.Hobbies');
/// INSERT INTO test VALUES ('Top.Hobbies.Amateurs_Astronomy');
/// INSERT INTO test VALUES ('Top.Collections');
/// INSERT INTO test VALUES ('Top.Collections.Pictures');
/// INSERT INTO test VALUES ('Top.Collections.Pictures.Astronomy');
/// INSERT INTO test VALUES ('Top.Collections.Pictures.Astronomy.Stars');
/// INSERT INTO test VALUES ('Top.Collections.Pictures.Astronomy.Galaxies');
/// INSERT INTO test VALUES ('Top.Collections.Pictures.Astronomy.Astronauts');
/// CREATE INDEX path_gist_idx ON test USING GIST (path);
/// CREATE INDEX path_idx ON test USING BTREE (path);
/// ```
///
/// The set of queries above will generate the following tree:
///
/// ```text
///                        Top
///                     /   |  \
///              Science Hobbies Collections
///                /       |              \
///       Astronomy   Amateurs_Astronomy Pictures
///            /  \                            |
/// Astrophysics  Cosmology                Astronomy
///                                       /    |    \
///                                Galaxies  Stars  Astronauts
/// ```
/// [Source][1]
///
/// [1]: https://www.postgresql.org/docs/current/ltree.html
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PgLTree;

impl SqlName for PgLTree {
    fn unquoted(&self, s: &mut dyn std::fmt::Write) {
        write!(s, "ltree").unwrap();
    }
}

impl From<PgLTree> for String {
    fn from(l: PgLTree) -> Self {
        l.to_string()
    }
}

/// Helper for constructing any type statement
#[derive(Debug)]
pub struct Type;

#[derive(Clone, Debug)]
pub enum TypeRef {
    Type(Name),
    SchemaType(Name, Name),
    DatabaseSchemaType(Name, Name, Name),
}

pub trait IntoTypeRef {
    fn into_type_ref(self) -> TypeRef;
}

impl IntoTypeRef for TypeRef {
    fn into_type_ref(self) -> TypeRef {
        self
    }
}

impl<I> IntoTypeRef for I
where
    I: IntoName,
{
    fn into_type_ref(self) -> TypeRef {
        TypeRef::Type(self.into_name())
    }
}

impl<A, B> IntoTypeRef for (A, B)
where
    A: IntoName,
    B: IntoName,
{
    fn into_type_ref(self) -> TypeRef {
        TypeRef::SchemaType(self.0.into_name(), self.1.into_name())
    }
}

impl<A, B, C> IntoTypeRef for (A, B, C)
where
    A: IntoName,
    B: IntoName,
    C: IntoName,
{
    fn into_type_ref(self) -> TypeRef {
        TypeRef::DatabaseSchemaType(self.0.into_name(), self.1.into_name(), self.2.into_name())
    }
}

/// Create a type
///
/// The type name is taken by [`Type::create`], because `CREATE TYPE ` is
/// rejected at the token after it (`[dec:pgorm:invalid-states-unrepresentable]`):
///
/// ```compile_fail,E0061
/// use pgorm_query::{extension::Type, tests_cfg::*, *};
///
/// Type::create().values(["name"]);
/// ```
///
/// The name alone is a shell type, which PostgreSQL accepts; `as_enum` and
/// `values` make it an enumeration, and the parenthesised value list is always
/// rendered once it is one, because `CREATE TYPE "t" AS ENUM ()` is an accepted
/// spelling of the empty enum while `CREATE TYPE "t" AS ENUM` is not a
/// statement at all. `as_composite` and `attribute` make it a composite
/// instead, `AS (...)`; what the type is, is one slot, so choosing one kind
/// replaces the other's list.
// [spec:pgorm:req:sql.ddl.type-enum+7]
#[derive(Debug, Clone)]
pub struct TypeCreateStatement {
    pub(crate) name: TypeRef,
    pub(crate) as_type: Option<TypeAs>,
}

/// What a `CREATE TYPE` defines, when it defines more than a shell type.
// [spec:pgorm:req:sql.ddl.type-enum+7]
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum TypeAs {
    /// `AS (..)`, a composite carrying its attributes, the marker and the
    /// attributes one fact as an enumeration's marker and labels are.
    // [spec:pgorm:req:sql.ddl.type-composite+2]
    Composite(Vec<CompositeAttribute>),
    /// `AS ENUM (..)`, carrying its labels: the marker and the values are one
    /// fact, so no value list survives without the `AS ENUM` that renders it.
    ///
    /// A label is *data* — it renders as a string literal, never as an
    /// identifier — so it is carried as a `String` rather than as a name.
    // [spec:pgorm:req:sql.render.ddl.enum-type+6]
    Enum(Vec<String>),
    /// `AS RANGE (..)`, carrying the subtype and the options beside it.
    // [spec:pgorm:req:sql.ddl.type-range+2]
    Range(RangeDefinition),
}

/// Drop one or more types
///
/// The first name is taken by [`Type::drop`] and every further one is appended,
/// so the name list is non-empty: PostgreSQL rejects `DROP TYPE ` at end of
/// input (`[dec:pgorm:invalid-states-unrepresentable]`).
///
/// ```compile_fail,E0061
/// use pgorm_query::{extension::Type, *};
///
/// Type::drop().if_exists();
/// ```
// [spec:pgorm:req:sql.ddl.type-alter-drop+7]
#[derive(Debug, Clone)]
pub struct TypeDropStatement {
    pub(crate) first: TypeRef,
    pub(crate) rest: Vec<TypeRef>,
    pub(crate) option: Option<TypeDropOpt>,
    pub(crate) if_exists: bool,
}

/// A type awaiting its first alter option.
///
/// PostgreSQL has no spelling for an `ALTER TYPE` that does nothing, so this is
/// what [`Type::alter`] returns: naming the type is not yet a statement. Each
/// option method consumes it and yields a [`TypeAlterStatement`].
///
/// ```compile_fail,E0599
/// use pgorm_query::{extension::Type, tests_cfg::*, *};
///
/// Type::alter(Font::Table).to_string();
/// ```
// [spec:pgorm:req:sql.ddl.type-alter-drop+7]
#[derive(Debug, Clone)]
pub struct PendingTypeAlter {
    pub(crate) name: TypeRef,
}

/// Alter a type
///
/// A statement of this type always names a type and always carries exactly one
/// option: it is reachable only by choosing an option on a
/// [`PendingTypeAlter`], so the `ALTER TYPE "font"` PostgreSQL rejects has no
/// constructor.
// [spec:pgorm:req:sql.ddl.type-alter-drop+7]
#[derive(Debug, Clone)]
pub struct TypeAlterStatement {
    pub(crate) name: TypeRef,
    pub(crate) option: TypeAlterOpt,
}

#[derive(Debug, Clone)]
pub enum TypeDropOpt {
    Cascade,
    Restrict,
}

/// What an `ALTER TYPE` does.
///
/// Enum labels are data and are carried as `String`; `Rename`'s payload is the
/// type's own name and is carried as an identifier, because that is what it
/// renders as.
// [spec:pgorm:req:sql.render.ddl.enum-type+6]
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum TypeAlterOpt {
    Add(String, Option<TypeAlterAddOpt>),
    Rename(Name),
    RenameValue(String, String),
}

/// Where an added enum label goes relative to an existing one — both labels,
/// so both data.
// [spec:pgorm:req:sql.render.ddl.enum-type+6]
#[derive(Debug, Clone)]
pub enum TypeAlterAddOpt {
    Before(String),
    After(String),
}

impl Type {
    /// Construct type [`TypeCreateStatement`] over the type it creates
    pub fn create<T>(name: T) -> TypeCreateStatement
    where
        T: IntoTypeRef,
    {
        TypeCreateStatement::new(name)
    }

    /// Construct type [`TypeDropStatement`] over the first type it drops
    pub fn drop<T>(name: T) -> TypeDropStatement
    where
        T: IntoTypeRef,
    {
        TypeDropStatement::new(name)
    }

    /// Name the type a [`TypeAlterStatement`] will alter
    pub fn alter<T>(name: T) -> PendingTypeAlter
    where
        T: IntoTypeRef,
    {
        PendingTypeAlter::new(name)
    }
}

impl TypeCreateStatement {
    /// Construct a new statement over the type it creates
    pub fn new<T>(name: T) -> Self
    where
        T: IntoTypeRef,
    {
        Self {
            name: name.into_type_ref(),
            as_type: None,
        }
    }

    /// Define the type as an enumeration, whose values are appended by
    /// [`TypeCreateStatement::values`]
    ///
    /// This replaces a composite's attributes, as
    /// [`as_composite`](Self::as_composite) replaces an enumeration's labels:
    /// a type is one kind or the other.
    ///
    /// ```
    /// use pgorm_query::{*, extension::Type, tests_cfg::*};
    ///
    /// assert_eq!(
    ///     Type::create(Font::Table).as_enum().to_string(),
    ///     r#"CREATE TYPE "font" AS ENUM ()"#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.type-enum+7]
    pub fn as_enum(&mut self) -> &mut Self {
        if !matches!(self.as_type, Some(TypeAs::Enum(_))) {
            self.as_type = Some(TypeAs::Enum(Vec::new()));
        }
        self
    }

    /// Append enum values, defining the type as an enumeration if it is not one
    /// already
    ///
    /// A label is data, not a name: it renders as a string literal — a bound
    /// parameter in [`build`](Self::build), a quoted literal in `Display` —
    /// so the bound is `Into<String>` and not an identifier type.
    ///
    /// ```
    /// use pgorm_query::{*, extension::Type};
    ///
    /// enum FontFamily {
    ///     Type,
    /// }
    ///
    /// impl SqlName for FontFamily {
    ///     fn unquoted(&self, s: &mut dyn std::fmt::Write) {
    ///         write!(s, "font_family").unwrap();
    ///     }
    /// }
    ///
    /// assert_eq!(
    ///     Type::create(FontFamily::Type)
    ///         .values(["serif", "sans", "monospace"])
    ///         .to_string(),
    ///     r#"CREATE TYPE "font_family" AS ENUM ('serif', 'sans', 'monospace')"#
    /// );
    /// ```
    // [spec:pgorm:req:sql.render.ddl.enum-type+6]
    pub fn values<T, I>(&mut self, values: I) -> &mut Self
    where
        T: Into<String>,
        I: IntoIterator<Item = T>,
    {
        self.as_enum();
        if let Some(TypeAs::Enum(existing)) = self.as_type.as_mut() {
            existing.extend(values.into_iter().map(Into::into));
        }
        self
    }
}

impl TypeDropStatement {
    /// Construct a new statement over the first type it drops
    pub fn new<T>(name: T) -> Self
    where
        T: IntoTypeRef,
    {
        Self {
            first: name.into_type_ref(),
            rest: Vec::new(),
            option: None,
            if_exists: false,
        }
    }

    /// Drop a further type
    ///
    /// ```
    /// use pgorm_query::{*, extension::Type};
    ///
    /// struct FontFamily;
    ///
    /// impl SqlName for FontFamily {
    ///     fn unquoted(&self, s: &mut dyn std::fmt::Write) {
    ///         write!(s, "{}", "font_family").unwrap();
    ///     }
    /// }
    ///
    /// assert_eq!(
    ///     Type::drop(FontFamily).if_exists().restrict().to_string(),
    ///     r#"DROP TYPE IF EXISTS "font_family" RESTRICT"#
    /// );
    /// ```
    pub fn name<T>(&mut self, name: T) -> &mut Self
    where
        T: IntoTypeRef,
    {
        self.rest.push(name.into_type_ref());
        self
    }

    /// Drop further types
    ///
    /// ```
    /// use pgorm_query::{*, extension::Type};
    ///
    /// #[derive(SqlName)]
    /// enum KycStatus {
    ///     #[iden = "kyc_status"]
    ///     Type,
    ///     Pending,
    ///     Approved,
    /// }
    ///
    /// #[derive(SqlName)]
    /// enum FontFamily {
    ///     #[iden = "font_family"]
    ///     Type,
    ///     Aerial,
    ///     Forte,
    /// }
    ///
    /// assert_eq!(
    ///     Type::drop(Name::new(KycStatus::Type))
    ///         .if_exists()
    ///         .names([Name::new(FontFamily::Type)])
    ///         .cascade()
    ///         .to_string(),
    ///     r#"DROP TYPE IF EXISTS "kyc_status", "font_family" CASCADE"#
    /// );
    /// ```
    pub fn names<T, I>(&mut self, names: I) -> &mut Self
    where
        T: IntoTypeRef,
        I: IntoIterator<Item = T>,
    {
        self.rest
            .extend(names.into_iter().map(IntoTypeRef::into_type_ref));
        self
    }

    /// The types dropped, in declaration order, of which there is at least one
    pub fn names_iter(&self) -> impl Iterator<Item = &TypeRef> {
        std::iter::once(&self.first).chain(self.rest.iter())
    }

    /// Set `IF EXISTS`
    pub fn if_exists(&mut self) -> &mut Self {
        self.if_exists = true;
        self
    }

    /// Set `CASCADE`
    pub fn cascade(&mut self) -> &mut Self {
        self.option = Some(TypeDropOpt::Cascade);
        self
    }

    /// Set `RESTRICT`
    pub fn restrict(&mut self) -> &mut Self {
        self.option = Some(TypeDropOpt::Restrict);
        self
    }
}

impl PendingTypeAlter {
    fn new<T>(name: T) -> Self
    where
        T: IntoTypeRef,
    {
        Self {
            name: name.into_type_ref(),
        }
    }

    fn with(self, option: TypeAlterOpt) -> TypeAlterStatement {
        TypeAlterStatement {
            name: self.name,
            option,
        }
    }

    /// Add an enum value
    ///
    /// The label is data and renders as a string literal, so the bound is
    /// `Into<String>`; contrast [`rename_to`](Self::rename_to), whose
    /// argument is the type's name and renders as an identifier.
    ///
    /// ```
    /// use pgorm_query::{*, extension::Type};
    ///
    /// enum FontFamily {
    ///     Type,
    /// }
    ///
    /// impl SqlName for FontFamily {
    ///     fn unquoted(&self, s: &mut dyn std::fmt::Write) {
    ///         write!(s, "font_family").unwrap();
    ///     }
    /// }
    ///
    /// assert_eq!(
    ///     Type::alter(FontFamily::Type)
    ///         .add_value("cursive")
    ///         .to_string(),
    ///     r#"ALTER TYPE "font_family" ADD VALUE 'cursive'"#
    /// );
    /// ```
    // [spec:pgorm:req:sql.render.ddl.enum-type+6]
    pub fn add_value<T>(self, value: T) -> TypeAlterStatement
    where
        T: Into<String>,
    {
        self.with(TypeAlterOpt::Add(value.into(), None))
    }

    /// Rename the type
    ///
    /// The argument is the type's new *name* and renders as a quoted
    /// identifier, so this keeps the identifier bound the label methods drop.
    pub fn rename_to<T>(self, name: T) -> TypeAlterStatement
    where
        T: IntoName,
    {
        self.with(TypeAlterOpt::Rename(name.into_name()))
    }

    /// Rename an enum value
    ///
    /// Both labels are data and render as string literals.
    ///
    /// ```
    /// use pgorm_query::{*, extension::Type, tests_cfg::*};
    ///
    /// assert_eq!(
    ///     Type::alter(Font::Table)
    ///         .rename_value("variant", "language")
    ///         .to_string(),
    ///     r#"ALTER TYPE "font" RENAME VALUE 'variant' TO 'language'"#
    /// )
    /// ```
    // [spec:pgorm:req:sql.render.ddl.enum-type+6]
    pub fn rename_value<T, V>(self, existing: T, new_name: V) -> TypeAlterStatement
    where
        T: Into<String>,
        V: Into<String>,
    {
        self.with(TypeAlterOpt::RenameValue(existing.into(), new_name.into()))
    }
}

impl TypeAlterStatement {
    /// Add a enum value before an existing value
    ///
    /// ```
    /// use pgorm_query::{*, extension::Type, tests_cfg::*};
    ///
    /// assert_eq!(
    ///     Type::alter(Font::Table)
    ///         .add_value("weight")
    ///         .before("variant")
    ///         .to_string(),
    ///     r#"ALTER TYPE "font" ADD VALUE 'weight' BEFORE 'variant'"#
    /// )
    /// ```
    // [spec:pgorm:req:sql.render.ddl.enum-type+6]
    #[must_use]
    pub fn before<T>(mut self, value: T) -> Self
    where
        T: Into<String>,
    {
        self.option = self.option.before(value);
        self
    }

    /// Add a enum value after an existing value
    // [spec:pgorm:req:sql.render.ddl.enum-type+6]
    #[must_use]
    pub fn after<T>(mut self, value: T) -> Self
    where
        T: Into<String>,
    {
        self.option = self.option.after(value);
        self
    }
}

impl TypeAlterOpt {
    /// Changes only `ADD VALUE x` options into `ADD VALUE x BEFORE` options, does nothing otherwise
    pub fn before<T>(self, value: T) -> Self
    where
        T: Into<String>,
    {
        match self {
            TypeAlterOpt::Add(label, _) => {
                Self::Add(label, Some(TypeAlterAddOpt::Before(value.into())))
            }
            _ => self,
        }
    }

    /// Changes only `ADD VALUE x` options into `ADD VALUE x AFTER` options, does nothing otherwise
    pub fn after<T>(self, value: T) -> Self
    where
        T: Into<String>,
    {
        match self {
            TypeAlterOpt::Add(label, _) => {
                Self::Add(label, Some(TypeAlterAddOpt::After(value.into())))
            }
            _ => self,
        }
    }
}

macro_rules! impl_type_statement_builder {
    ( $struct_name: ident, $func_name: ident, $display_doc: literal ) => {
        impl $struct_name {
            /// Build the SQL statement into the given sink, returning the sink's text
            pub fn build_collect(&self, sql: &mut dyn SqlWriter) -> String {
                QueryBuilder.$func_name(self, sql);
                sql.to_string()
            }
        }

        #[doc = $display_doc]
        // [spec:pgorm:req:sql.ddl+12]
        impl fmt::Display for $struct_name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                let mut sql = String::with_capacity(256);
                QueryBuilder.$func_name(self, &mut sql);
                f.write_str(&sql)
            }
        }
    };
}

/// Give a label-carrying type statement the placeholder rendering its
/// `push_param` calls already make reachable, so the capability has a name
/// rather than only an `into_parts` on a sink the caller had to build.
macro_rules! impl_type_statement_build {
    ( $struct_name: ident, $doc: literal ) => {
        impl $struct_name {
            #[doc = $doc]
            ///
            /// PostgreSQL accepts no bind parameter in DDL, so the pair this
            /// returns is for inspection — logging the labels apart from the
            /// SQL, feeding a proxy that expands them — and the statement you
            /// execute is the inlined `Display` rendering.
            // [spec:pgorm:req:sql.ddl+12]
            // [spec:pgorm:req:sql.render.ddl.enum-type+6]
            pub fn build(&self) -> (String, Values) {
                let mut sql = SqlWriterValues::new("$", true);
                self.build_collect(&mut sql);
                sql.into_parts()
            }
        }
    };
}

impl_type_statement_builder!(
    TypeCreateStatement,
    prepare_type_create_statement,
    "Renders every enum label inlined as an escaped SQL literal rather than \
     bound. This is the rendering to execute: PostgreSQL takes no bind \
     parameter in DDL, so [`build`](Self::build)'s `$N` form is for \
     inspection."
);
impl_type_statement_builder!(
    TypeAlterStatement,
    prepare_type_alter_statement,
    "Renders every enum label inlined as an escaped SQL literal rather than \
     bound. This is the rendering to execute: PostgreSQL takes no bind \
     parameter in DDL, so [`build`](Self::build)'s `$N` form is for \
     inspection."
);
impl_type_statement_builder!(
    TypeDropStatement,
    prepare_type_drop_statement,
    "Renders the statement with every value inlined as an escaped SQL \
     literal. This is its only rendering: it binds nothing, so there is no \
     placeholder form to choose between."
);

impl_type_statement_build!(
    TypeCreateStatement,
    "Build the statement with its enum labels as numbered placeholders, \
     returning the SQL and the labels to bind."
);
impl_type_statement_build!(
    TypeAlterStatement,
    "Build the statement with its enum label operands as numbered \
     placeholders, returning the SQL and the labels to bind."
);
