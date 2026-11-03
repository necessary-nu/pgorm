use super::interval::IntervalSpec;
use super::{Check, GeneratedKind, IntoCheck};
use crate::{RangeType, SequenceOptions, expr::*, types::*};
use std::sync::Arc;

/// Specification of a table column
// [spec:pgorm:req:sql.ddl.column-def+13]
#[derive(Debug, Clone)]
pub struct ColumnDef {
    pub(crate) table: Option<TableName>,
    pub(crate) name: Name,
    pub(crate) types: Option<ColumnType>,
    /// The collation written after the type, `COLLATE "name"`: one slot and
    /// not a spec, because PostgreSQL refuses a second `COLLATE` on a column.
    // [spec:pgorm:req:sql.ddl.column-def+13]
    pub(crate) collation: Option<Collation>,
    pub(crate) spec: Vec<ColumnSpec>,
}

pub trait IntoColumnDef {
    fn into_column_def(self) -> ColumnDef;
}

/// All column types
///
/// | ColumnType            | PostgreSQL data type     |
/// |-----------------------|--------------------------|
/// | Char                  | char(n), bpchar          |
/// | String                | varchar                  |
/// | Text                  | text                     |
/// | SmallInteger          | smallint                 |
/// | Integer               | integer                  |
/// | BigInteger            | bigint                   |
/// | Float                 | real                     |
/// | Double                | double precision         |
/// | Decimal               | decimal                  |
/// | Timestamp             | timestamp                |
/// | TimestampWithTimeZone | timestamp with time zone |
/// | Time                  | time                     |
/// | Date                  | date                     |
/// | Interval              | interval                 |
/// | Bytea                 | bytea                    |
/// | Bit                   | bit                      |
/// | VarBit                | varbit                   |
/// | Boolean               | bool                     |
/// | Money                 | money                    |
/// | Json                  | json                     |
/// | JsonBinary            | jsonb                    |
/// | Uuid                  | uuid                     |
/// | Enum                  | ENUM_NAME                |
/// | Array                 | DATA_TYPE[]              |
/// | Vector                | vector                   |
/// | Cidr                  | cidr                     |
/// | Inet                  | inet                     |
/// | MacAddr               | macaddr                  |
/// | LTree                 | ltree                    |
/// | Range                 | int4range, ..., tstzrange |
/// | Multirange            | int4multirange, ...      |
/// | CreatedRange          | RANGE_TYPE_NAME          |
/// | CreatedMultirange     | MULTIRANGE_TYPE_NAME     |
/// | Domain                | DOMAIN_NAME              |
// [spec:pgorm:def:sql.types.column-type+11]
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ColumnType {
    Char(Option<u32>),
    String(StringLen),
    Text,
    Bytea,
    SmallInteger,
    Integer,
    BigInteger,
    Float,
    Double,
    Decimal(Option<(u32, u32)>),
    Timestamp,
    TimestampWithTimeZone,
    Time,
    Date,
    Interval(IntervalSpec),
    Bit(Option<u32>),
    VarBit(u32),
    Boolean,
    Money,
    Json,
    JsonBinary,
    Uuid,
    /// A type this crate has no variant for, named rather than spelled: the
    /// payload is a [`TypeName`], so it renders through the same
    /// quoted-or-safe-bare part policy every other type name does and a
    /// hostile catalogue name becomes a name PostgreSQL refuses, never SQL.
    Named(TypeName),
    Enum {
        name: Name,
        schema: Option<Name>,
        variants: Vec<Name>,
    },
    Array(Arc<ColumnType>),
    Vector(Option<u32>),
    Cidr,
    Inet,
    MacAddr,
    LTree,
    /// One of the built-in range types, `int4range` through `tstzrange`. A
    /// range type created by `CREATE TYPE ... AS RANGE` is
    /// [`CreatedRange`](Self::CreatedRange).
    // [spec:pgorm:def:sql.value.range+5]
    Range(RangeType),
    /// The multirange over one of the built-in range types,
    /// `int4multirange` through `tstzmultirange`.
    // [spec:pgorm:def:sql.value.range+5]
    Multirange(RangeType),
    /// A range type a schema created with `CREATE TYPE ... AS RANGE`, named
    /// in full as an enum type is, and the subtype it ranges over. The name
    /// is what DDL writes and what a value is cast to; the subtype is what a
    /// value's bounds are, which no name says.
    // [spec:pgorm:def:sql.value.created-range+5]
    CreatedRange {
        name: Name,
        schema: Option<Name>,
        subtype: Arc<ColumnType>,
    },
    /// The multirange PostgreSQL creates beside a range type a schema
    /// created, named in full — its own name, which is not the range's — and
    /// the subtype its ranges range over.
    // [spec:pgorm:def:sql.value.created-range+5]
    CreatedMultirange {
        name: Name,
        schema: Option<Name>,
        subtype: Arc<ColumnType>,
    },
    /// A domain a schema created, named in full as an enum type is, and the
    /// type it is over. The name is what DDL writes, and what a value is cast
    /// to where the base type needs a cast to be compared; the base is what a
    /// value is, since a domain's values travel as its base type's do.
    ///
    /// This is the one spelling of a domain column. [`Named`](Self::Named)
    /// renders the same DDL but says nothing of what the column holds, which
    /// is what an entity reads and writes by.
    // [spec:pgorm:def:sql.types.column-type+11]
    // [spec:pgorm:req:sql.ddl.domain+3]
    Domain {
        name: Name,
        schema: Option<Name>,
        base: Arc<ColumnType>,
    },
}

/// Length for var-char; default to 255
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum StringLen {
    /// String size
    N(u32),
    Max,
    #[default]
    None,
}

// [spec:pgorm:def:sql.types.column-type+11]
impl PartialEq for ColumnType {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Char(l0), Self::Char(r0)) => l0 == r0,
            (Self::String(l0), Self::String(r0)) => l0 == r0,
            (Self::Decimal(l0), Self::Decimal(r0)) => l0 == r0,
            (Self::Interval(l0), Self::Interval(r0)) => l0 == r0,
            (Self::Bit(l0), Self::Bit(r0)) => l0 == r0,
            (Self::VarBit(l0), Self::VarBit(r0)) => l0 == r0,
            (Self::Named(l0), Self::Named(r0)) => l0.raw_text() == r0.raw_text(),
            (
                Self::Enum {
                    name: l_name,
                    schema: l_schema,
                    variants: l_variants,
                },
                Self::Enum {
                    name: r_name,
                    schema: r_schema,
                    variants: r_variants,
                },
            ) => {
                l_name.to_string() == r_name.to_string()
                    && l_schema.as_ref().map(|s| s.to_string())
                        == r_schema.as_ref().map(|s| s.to_string())
                    && l_variants
                        .iter()
                        .map(|v| v.to_string())
                        .eq(r_variants.iter().map(|v| v.to_string()))
            }
            (Self::Array(l0), Self::Array(r0)) => l0 == r0,
            (Self::Range(l0), Self::Range(r0)) => l0 == r0,
            (
                Self::CreatedRange {
                    name: l_name,
                    schema: l_schema,
                    subtype: l_subtype,
                },
                Self::CreatedRange {
                    name: r_name,
                    schema: r_schema,
                    subtype: r_subtype,
                },
            )
            | (
                Self::CreatedMultirange {
                    name: l_name,
                    schema: l_schema,
                    subtype: l_subtype,
                },
                Self::CreatedMultirange {
                    name: r_name,
                    schema: r_schema,
                    subtype: r_subtype,
                },
            ) => {
                l_name.to_string() == r_name.to_string()
                    && l_schema.as_ref().map(|s| s.to_string())
                        == r_schema.as_ref().map(|s| s.to_string())
                    && l_subtype == r_subtype
            }
            (Self::Multirange(l0), Self::Multirange(r0)) => l0 == r0,
            (
                Self::Domain {
                    name: l_name,
                    schema: l_schema,
                    base: l_base,
                },
                Self::Domain {
                    name: r_name,
                    schema: r_schema,
                    base: r_base,
                },
            ) => {
                l_name.to_string() == r_name.to_string()
                    && l_schema.as_ref().map(|s| s.to_string())
                        == r_schema.as_ref().map(|s| s.to_string())
                    && l_base == r_base
            }
            _ => core::mem::discriminant(self) == core::mem::discriminant(other),
        }
    }
}

impl ColumnType {
    /// A type named rather than spelled — `citext`, `tenant_a.status`.
    ///
    /// The name is data, not program text: it renders through
    /// [`TypeName`]'s part policy, so a runtime `String` is safe here.
    /// A type *expression* — `numeric(12, 2)` — is grammar rather than a
    /// name and belongs in
    /// [`Expr::cast_as_raw`](crate::Expr::cast_as_raw).
    // [spec:pgorm:req:sql.render.ident-quoting+7]
    pub fn named<T>(ty: T) -> ColumnType
    where
        T: Into<String>,
    {
        ColumnType::Named(TypeName::new(Name::runtime(ty)))
    }

    pub fn string(length: Option<u32>) -> ColumnType {
        match length {
            Some(s) => ColumnType::String(StringLen::N(s)),
            None => ColumnType::String(StringLen::None),
        }
    }

    /// The `serial` spelling this type is replaced by when the column carries
    /// [`ColumnSpec::AutoIncrement`], or `None` when Postgres has no serial
    /// form for it.
    // [spec:pgorm:req:sql.ddl.column-def+13]
    pub fn serial_spelling(&self) -> Option<&'static str> {
        match self {
            ColumnType::SmallInteger => Some("smallserial"),
            ColumnType::Integer => Some("serial"),
            ColumnType::BigInteger => Some("bigserial"),
            _ => None,
        }
    }
}

/// All column specification keywords
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ColumnSpec {
    Null,
    /// `[CONSTRAINT "name" ]NOT NULL[ NO INHERIT]`: the column's one not-null
    /// constraint, whose name PostgreSQL 18 records in its catalog — derived
    /// when `name` is `None` — and which a child table inherits unless
    /// `no_inherit`. Set by [`ColumnDef::not_null`],
    /// [`ColumnDef::not_null_named`] and [`ColumnDef::not_null_no_inherit`],
    /// which share the one spec a column holds.
    // [spec:pgorm:req:sql.ddl.column-def+13]
    NotNull {
        name: Option<Name>,
        no_inherit: bool,
    },
    Default(SimpleExpr),
    AutoIncrement,
    /// `[CONSTRAINT "name" ]CHECK (<expr>)[ ENFORCED | NOT ENFORCED]`. It
    /// carries no deferrability: PostgreSQL never defers a check constraint.
    // [spec:pgorm:req:sql.ddl.column-def+13]
    Check(Check),
    /// `GENERATED ALWAYS AS (<expr>) { STORED | VIRTUAL }`: a column computed
    /// from the others in its row, and which of PostgreSQL's two kinds it is,
    /// always written out.
    // [spec:pgorm:req:sql.ddl.column-def+13]
    Generated {
        expr: SimpleExpr,
        kind: GeneratedKind,
    },
    /// `GENERATED { ALWAYS | BY DEFAULT } AS IDENTITY` — PostgreSQL's
    /// standard-SQL replacement for the serial family — and the options of the
    /// sequence it owns, written in parentheses after it when there are any.
    // [spec:pgorm:req:sql.ddl.column-def+13]
    Identity(IdentityGeneration, Option<SequenceOptions>),
    /// Verbatim SQL appended after the column's own clauses.
    RawSuffix(&'static str),
    /// Metadata, not a rendered clause: PostgreSQL has no column-comment
    /// clause of `CREATE TABLE`, so the renderer skips this spec and the text
    /// is carried for
    /// [`TableCreateStatement::comments`](crate::TableCreateStatement::comments)
    /// — and for consumers reading it back off `get_column_spec` — to turn
    /// into the `COMMENT ON COLUMN` statement it really is.
    // [spec:pgorm:req:sql.ddl.comment+5]
    Comment(String),
}

/// Which of PostgreSQL's two identity forms supplies a column's values.
///
/// The distinction is only about what happens when a statement *does* name the
/// column: under [`Always`](IdentityGeneration::Always) the server rejects the
/// supplied value, under [`ByDefault`](IdentityGeneration::ByDefault) it takes
/// it and the sequence fills in only where a statement stays silent. Both draw
/// from the same implicit sequence otherwise.
///
/// There is no third state and no flag: an identity column is generated one way
/// or the other, never both and never neither, so the choice is a closed pair
/// rather than a `bool` that reads backwards at the call site
/// (`[dec:pgorm:invalid-states-unrepresentable]`).
// [spec:pgorm:req:sql.ddl.column-def+13]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityGeneration {
    /// `GENERATED ALWAYS AS IDENTITY`.
    Always,
    /// `GENERATED BY DEFAULT AS IDENTITY`.
    ByDefault,
}

impl IdentityGeneration {
    /// The keyword PostgreSQL spells this form with — `ALWAYS` or
    /// `BY DEFAULT`, the words `information_schema.columns.identity_generation`
    /// reports it under.
    // [spec:pgorm:req:sql.ddl.column-def+13]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Always => "ALWAYS",
            Self::ByDefault => "BY DEFAULT",
        }
    }
}

impl ColumnDef {
    /// Construct a table column
    pub fn new<T>(name: T) -> Self
    where
        T: IntoName,
    {
        Self {
            table: None,
            name: name.into_name(),
            types: None,
            collation: None,
            spec: Vec::new(),
        }
    }

    /// Construct a table column with column type
    pub fn new_with_type<T>(name: T, types: ColumnType) -> Self
    where
        T: IntoName,
    {
        Self {
            table: None,
            name: name.into_name(),
            types: Some(types),
            collation: None,
            spec: Vec::new(),
        }
    }

    /// Set column null
    pub fn null(&mut self) -> &mut Self {
        self.spec.push(ColumnSpec::Null);
        self
    }

    /// Set default expression of a column
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// let table = Table::create(Char::Table)
    ///     .col(ColumnDef::new(Char::FontId).integer().default(12i32))
    ///     .col(
    ///         ColumnDef::new(Char::CreatedAt)
    ///             .timestamp()
    ///             .default(Expr::current_timestamp())
    ///             .not_null(),
    ///     )
    ///     .to_owned();
    ///
    /// assert_eq!(
    ///     table.to_string(),
    ///     [
    ///         r#"CREATE TABLE "character" ("#,
    ///         r#""font_id" integer DEFAULT 12,"#,
    ///         r#""created_at" timestamp DEFAULT CURRENT_TIMESTAMP NOT NULL"#,
    ///         r#")"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    pub fn default<T>(&mut self, value: T) -> &mut Self
    where
        T: Into<SimpleExpr>,
    {
        self.spec.push(ColumnSpec::Default(value.into()));
        self
    }

    /// Draw the column's values from the serial family — `serial`,
    /// `smallserial`, `bigserial` — in place of its declared type.
    ///
    /// This is the legacy spelling. PostgreSQL has recommended
    /// [`identity`](Self::identity) since 10: an identity column is standard
    /// SQL, owns its sequence outright rather than merely defaulting to one,
    /// and cannot be overwritten by accident. `auto_increment` stays because it
    /// is what the entity derive's `auto_increment` attribute means today;
    /// prefer `identity` in new schemas.
    // [spec:pgorm:req:sql.ddl.column-def+13]
    pub fn auto_increment(&mut self) -> &mut Self {
        self.spec.push(ColumnSpec::AutoIncrement);
        self
    }

    /// Generate the column's values `ALWAYS` — `GENERATED ALWAYS AS IDENTITY`.
    ///
    /// The server refuses an explicit value for the column, which is what makes
    /// this the safe default: a row cannot silently claim a key the sequence
    /// does not know it handed out. The deliberate override is spelled on the
    /// statement rather than here — [`crate::InsertStatement::overriding`]
    /// with [`crate::Overriding::SystemValue`] — so one insert is exempted, not
    /// the column's declaration that is weakened
    /// (`[spec:pgorm:def:sql.ast.insert+3]`).
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Table::create(Glyph::Table)
    ///         .col(ColumnDef::new(Glyph::Id).integer().identity())
    ///         .primary_key(Glyph::Id)
    ///         .to_string(),
    ///     r#"CREATE TABLE "glyph" ( "id" integer GENERATED ALWAYS AS IDENTITY, PRIMARY KEY ("id") )"#,
    /// );
    /// ```
    ///
    /// The sequence's own options — `START WITH`, `INCREMENT BY`, `CACHE` and
    /// the rest — are [`identity_with`](Self::identity_with)'s.
    // [spec:pgorm:req:sql.ddl.column-def+13]
    pub fn identity(&mut self) -> &mut Self {
        self.spec
            .push(ColumnSpec::Identity(IdentityGeneration::Always, None));
        self
    }

    /// Generate the column's values `BY DEFAULT` —
    /// `GENERATED BY DEFAULT AS IDENTITY`.
    ///
    /// The sequence supplies a value only where the statement gives none, so an
    /// explicit value is accepted — the behaviour a `serial` column has, with
    /// an owned sequence instead of a `DEFAULT nextval(..)`. Reach for it where
    /// rows arrive carrying keys already, such as a restore or a migration;
    /// otherwise prefer [`identity`](Self::identity).
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    ///
    /// assert_eq!(
    ///     Table::create(Glyph::Table)
    ///         .col(
    ///             ColumnDef::new(Glyph::Id)
    ///                 .big_integer()
    ///                 .identity_by_default()
    ///                 .not_null()
    ///         )
    ///         .to_string(),
    ///     r#"CREATE TABLE "glyph" ( "id" bigint GENERATED BY DEFAULT AS IDENTITY NOT NULL )"#,
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.column-def+13]
    pub fn identity_by_default(&mut self) -> &mut Self {
        self.spec
            .push(ColumnSpec::Identity(IdentityGeneration::ByDefault, None));
        self
    }

    /// Set column type as char with an explicit length
    pub fn char_len(&mut self, length: u32) -> &mut Self {
        self.types = Some(ColumnType::Char(Some(length)));
        self
    }

    /// Set column type as SQL's `char`, which PostgreSQL reads as `char(1)`.
    /// The blank-padded type with no length limit is
    /// `ColumnType::Char(None)`, spelled `bpchar`.
    // [spec:pgorm:req:sql.ddl.column-types+7]
    pub fn char(&mut self) -> &mut Self {
        self.types = Some(ColumnType::Char(Some(1)));
        self
    }

    /// Set column type as string with an explicit length
    pub fn string_len(&mut self, length: u32) -> &mut Self {
        self.types = Some(ColumnType::String(StringLen::N(length)));
        self
    }

    /// Set column type as string
    pub fn string(&mut self) -> &mut Self {
        self.types = Some(ColumnType::String(Default::default()));
        self
    }

    /// Set column type as text
    pub fn text(&mut self) -> &mut Self {
        self.types = Some(ColumnType::Text);
        self
    }

    /// Set column type as small_integer
    pub fn small_integer(&mut self) -> &mut Self {
        self.types = Some(ColumnType::SmallInteger);
        self
    }

    /// Set column type as integer
    pub fn integer(&mut self) -> &mut Self {
        self.types = Some(ColumnType::Integer);
        self
    }

    /// Set column type as big_integer
    pub fn big_integer(&mut self) -> &mut Self {
        self.types = Some(ColumnType::BigInteger);
        self
    }

    /// Set column type as float
    pub fn float(&mut self) -> &mut Self {
        self.types = Some(ColumnType::Float);
        self
    }

    /// Set column type as double
    pub fn double(&mut self) -> &mut Self {
        self.types = Some(ColumnType::Double);
        self
    }

    /// Set column type as decimal with an explicit precision and scale
    pub fn decimal_len(&mut self, precision: u32, scale: u32) -> &mut Self {
        self.types = Some(ColumnType::Decimal(Some((precision, scale))));
        self
    }

    /// Set column type as decimal
    pub fn decimal(&mut self) -> &mut Self {
        self.types = Some(ColumnType::Decimal(None));
        self
    }

    /// Set column type as interval type. Postgres only
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    /// assert_eq!(
    ///     Table::create(Glyph::Table)
    ///         .col(
    ///             ColumnDef::new(Name::runtime("I1"))
    ///                 .interval(IntervalSpec::Any(None))
    ///                 .not_null()
    ///         )
    ///         .col(
    ///             ColumnDef::new(Name::runtime("I2"))
    ///                 .interval(IntervalSpec::Fields(PgInterval::YearToMonth))
    ///                 .not_null()
    ///         )
    ///         .col(
    ///             ColumnDef::new(Name::runtime("I3"))
    ///                 .interval(IntervalSpec::Any(Some(IntervalPrecision::P4)))
    ///                 .not_null()
    ///         )
    ///         .col(
    ///             ColumnDef::new(Name::runtime("I4"))
    ///                 .interval(IntervalSpec::Fields(PgInterval::HourToSecond(Some(
    ///                     IntervalPrecision::P3
    ///                 ))))
    ///                 .not_null()
    ///         )
    ///         .to_string(),
    ///     [
    ///         r#"CREATE TABLE "glyph" ("#,
    ///         r#""I1" interval NOT NULL,"#,
    ///         r#""I2" interval YEAR TO MONTH NOT NULL,"#,
    ///         r#""I3" interval(4) NOT NULL,"#,
    ///         r#""I4" interval HOUR TO SECOND(3) NOT NULL"#,
    ///         r#")"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    // [spec:pgorm:def:sql.types.column-type+11]
    pub fn interval(&mut self, spec: IntervalSpec) -> &mut Self {
        self.types = Some(ColumnType::Interval(spec));
        self
    }

    pub fn vector(&mut self, size: Option<u32>) -> &mut Self {
        self.types = Some(ColumnType::Vector(size));
        self
    }

    /// Set column type as timestamp
    pub fn timestamp(&mut self) -> &mut Self {
        self.types = Some(ColumnType::Timestamp);
        self
    }

    /// Set column type as timestamp with time zone. Postgres only
    pub fn timestamp_with_time_zone(&mut self) -> &mut Self {
        self.types = Some(ColumnType::TimestampWithTimeZone);
        self
    }

    /// Set column type as time
    pub fn time(&mut self) -> &mut Self {
        self.types = Some(ColumnType::Time);
        self
    }

    /// Set column type as date
    pub fn date(&mut self) -> &mut Self {
        self.types = Some(ColumnType::Date);
        self
    }

    /// Set column type as bit with variable length
    pub fn bit(&mut self, length: Option<u32>) -> &mut Self {
        self.types = Some(ColumnType::Bit(length));
        self
    }

    /// Set column type as varbit with variable length
    pub fn varbit(&mut self, length: u32) -> &mut Self {
        self.types = Some(ColumnType::VarBit(length));
        self
    }

    /// Set column type as bytea
    pub fn bytea(&mut self) -> &mut Self {
        self.types = Some(ColumnType::Bytea);
        self
    }

    /// Set column type as boolean
    pub fn boolean(&mut self) -> &mut Self {
        self.types = Some(ColumnType::Boolean);
        self
    }

    /// Set column type as money
    pub fn money(&mut self) -> &mut Self {
        self.types = Some(ColumnType::Money);
        self
    }

    /// Set column type as json.
    pub fn json(&mut self) -> &mut Self {
        self.types = Some(ColumnType::Json);
        self
    }

    /// Set column type as json binary.
    pub fn json_binary(&mut self) -> &mut Self {
        self.types = Some(ColumnType::JsonBinary);
        self
    }

    /// Set column type as uuid
    pub fn uuid(&mut self) -> &mut Self {
        self.types = Some(ColumnType::Uuid);
        self
    }

    /// Use a type this vocabulary has no spelling for, named rather than
    /// spelled.
    ///
    /// Takes anything a [`TypeName`] is built from — a bare name, or a
    /// `TypeName` carrying a schema qualifier or an array suffix — and every
    /// part renders quoted-or-safe-bare, never as SQL.
    // [spec:pgorm:req:sql.render.ident-quoting+7]
    pub fn named<T>(&mut self, name: T) -> &mut Self
    where
        T: Into<TypeName>,
    {
        self.types = Some(ColumnType::Named(name.into()));
        self
    }

    /// Set column type as enum.
    pub fn enumeration<N, S, V>(&mut self, name: N, variants: V) -> &mut Self
    where
        N: IntoName,
        S: IntoName,
        V: IntoIterator<Item = S>,
    {
        self.types = Some(ColumnType::Enum {
            name: name.into_name(),
            schema: None,
            variants: variants.into_iter().map(IntoName::into_name).collect(),
        });
        self
    }

    /// Set column type as an array with a specified element type.
    /// This is only supported on Postgres.
    pub fn array(&mut self, elem_type: ColumnType) -> &mut Self {
        self.types = Some(ColumnType::Array(Arc::new(elem_type)));
        self
    }

    /// Set columnt type as cidr.
    /// This is only supported on Postgres.
    pub fn cidr(&mut self) -> &mut Self {
        self.types = Some(ColumnType::Cidr);
        self
    }

    /// Set columnt type as inet.
    /// This is only supported on Postgres.
    pub fn inet(&mut self) -> &mut Self {
        self.types = Some(ColumnType::Inet);
        self
    }

    /// Set columnt type as macaddr.
    /// This is only supported on Postgres.
    pub fn mac_address(&mut self) -> &mut Self {
        self.types = Some(ColumnType::MacAddr);
        self
    }

    /// Set column type as `ltree`
    /// This is only supported on Postgres.
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    /// assert_eq!(
    ///     Table::create(Glyph::Table)
    ///         .col(
    ///             ColumnDef::new(Glyph::Id)
    ///                 .integer()
    ///                 .not_null()
    ///                 .auto_increment()
    ///         )
    ///         .col(ColumnDef::new(Glyph::Tokens).ltree())
    ///         .primary_key(Glyph::Id)
    ///         .to_string(),
    ///     [
    ///         r#"CREATE TABLE "glyph" ("#,
    ///         r#""id" serial NOT NULL,"#,
    ///         r#""tokens" ltree,"#,
    ///         r#"PRIMARY KEY ("id")"#,
    ///         r#")"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    pub fn ltree(&mut self) -> &mut Self {
        self.types = Some(ColumnType::LTree);
        self
    }

    /// Hold the column's rows to a condition: `CHECK (<expr>)`.
    ///
    /// An expression is the unnamed, enforced constraint; a [`Check`] names it
    /// or says it is `NOT ENFORCED` (`[spec:pgorm:req:sql.ddl.enforcement]`).
    ///
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    /// assert_eq!(
    ///     Table::create(Glyph::Table)
    ///         .col(
    ///             ColumnDef::new(Glyph::Id)
    ///                 .integer()
    ///                 .not_null()
    ///                 .check(Expr::col(Glyph::Id).gt(10))
    ///                 .check(
    ///                     Check::new(Expr::col(Glyph::Id).lt(1000))
    ///                         .name(Name::runtime("id_small"))
    ///                         .enforcement(Enforcement::NotEnforced)
    ///                 )
    ///         )
    ///         .to_string(),
    ///     [
    ///         r#"CREATE TABLE "glyph" ( "id" integer NOT NULL CHECK ("id" > 10)"#,
    ///         r#"CONSTRAINT "id_small" CHECK ("id" < 1000) NOT ENFORCED )"#,
    ///     ]
    ///     .join(" "),
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.column-def+13]
    pub fn check<C>(&mut self, check: C) -> &mut Self
    where
        C: IntoCheck,
    {
        self.spec.push(ColumnSpec::Check(check.into_check()));
        self
    }

    /// Append verbatim SQL after this column's own clauses — the escape hatch
    /// for column specs this vocabulary has no spelling for.
    ///
    /// The `&'static str` bound is the contract, the same one
    /// [`Expr::raw`](crate::Expr::raw) carries: only program text can be
    /// appended, never a runtime string a value could have reached.
    /// ```
    /// use pgorm_query::{tests_cfg::*, *};
    /// let table = Table::create(Char::Table)
    ///     .col(
    ///         ColumnDef::new(Char::Id)
    ///             .uuid()
    ///             .raw_suffix("DEFAULT gen_random_uuid()")
    ///             .not_null(),
    ///     )
    ///     .col(
    ///         ColumnDef::new(Char::CreatedAt)
    ///             .timestamp_with_time_zone()
    ///             .raw_suffix("DEFAULT NOW()")
    ///             .not_null(),
    ///     )
    ///     .primary_key(Char::Id)
    ///     .to_owned();
    /// assert_eq!(
    ///     table.to_string(),
    ///     [
    ///         r#"CREATE TABLE "character" ("#,
    ///         r#""id" uuid DEFAULT gen_random_uuid() NOT NULL,"#,
    ///         r#""created_at" timestamp with time zone DEFAULT NOW() NOT NULL,"#,
    ///         r#"PRIMARY KEY ("id")"#,
    ///         r#")"#,
    ///     ]
    ///     .join(" ")
    /// );
    /// ```
    pub fn raw_suffix(&mut self, sql: &'static str) -> &mut Self {
        self.spec.push(ColumnSpec::RawSuffix(sql));
        self
    }

    /// Record a comment for this column.
    ///
    /// The spec is skipped when the column renders: on Postgres a column
    /// comment is a `COMMENT ON` statement of its own. Render it off the
    /// create statement this column goes into, with
    /// [`TableCreateStatement::comments`](crate::TableCreateStatement::comments),
    /// or build it directly with [`Comment::on_column`](crate::Comment::on_column).
    // [spec:pgorm:req:sql.ddl.comment+5]
    pub fn comment<T>(&mut self, string: T) -> &mut Self
    where
        T: Into<String>,
    {
        self.spec.push(ColumnSpec::Comment(string.into()));
        self
    }

    pub fn get_column_name(&self) -> String {
        self.name.to_string()
    }

    pub fn get_column_type(&self) -> Option<&ColumnType> {
        self.types.as_ref()
    }

    pub fn get_column_spec(&self) -> &Vec<ColumnSpec> {
        self.spec.as_ref()
    }

    pub fn take(&mut self) -> Self {
        Self {
            table: self.table.take(),
            name: self.name.clone(),
            types: self.types.take(),
            collation: self.collation.take(),
            spec: std::mem::take(&mut self.spec),
        }
    }
}

impl IntoColumnDef for &mut ColumnDef {
    fn into_column_def(self) -> ColumnDef {
        self.take()
    }
}

impl IntoColumnDef for ColumnDef {
    fn into_column_def(self) -> ColumnDef {
        self
    }
}
