//! `CREATE TYPE ... AS RANGE`: a range type over a subtype, and the options
//! that change how its values are ordered and indexed. The type statement's
//! file is over its function cap, so the range form sits beside it here.

use crate::{Collation, ColumnType, IntoCollation, IntoName, Name};

use super::{IntoTypeRef, TypeAs, TypeCreateStatement, TypeRef};

/// What a range type ranges over: `SUBTYPE = <type>`, with the options
/// PostgreSQL lets a range type set beside it.
///
/// The subtype is the one option PostgreSQL requires (`42601` without it), so
/// it is what constructs the definition, and each other option is written
/// only when set:
///
/// - [`subtype_opclass`](Self::subtype_opclass), the b-tree operator class
///   that orders the subtype when its default is not the ordering wanted;
/// - [`collation`](Self::collation), the collation a collatable subtype is
///   ordered under (`42809` on a subtype that has none);
/// - [`subtype_diff`](Self::subtype_diff), the function that measures the
///   distance between two subtype values, which a GiST index over the range
///   is much faster for having;
/// - [`multirange_type_name`](Self::multirange_type_name), the name of the
///   multirange PostgreSQL creates beside the range, which otherwise derives
///   it from the range's name.
///
/// `CANONICAL` is not among them. A canonical function takes and returns the
/// range type itself, so it has to be created against a shell type before the
/// range exists, and only a C function can be: PostgreSQL refuses a shell
/// type as an SQL function's argument (`42P13`) and as a PL/pgSQL function's
/// result (`0A000`), and refuses the option on a type with no shell
/// (`42P17`). A function body is outside what this crate builds.
///
/// ```
/// use pgorm_query::{extension::{RangeDefinition, Type}, *};
///
/// assert_eq!(
///     Type::create(Name::runtime("floatrange"))
///         .as_range(RangeDefinition::new(ColumnType::Double).subtype_diff(Name::runtime("float8mi")))
///         .to_string(),
///     r#"CREATE TYPE "floatrange" AS RANGE (SUBTYPE = double precision, SUBTYPE_DIFF = "float8mi")"#
/// );
/// ```
// [spec:pgorm:req:sql.ddl.type-range+3]
#[derive(Debug, Clone)]
pub struct RangeDefinition {
    pub(crate) subtype: ColumnType,
    pub(crate) subtype_opclass: Option<Name>,
    pub(crate) collation: Option<Collation>,
    pub(crate) subtype_diff: Option<Name>,
    pub(crate) multirange_type_name: Option<TypeRef>,
}

impl RangeDefinition {
    /// A range over `subtype`, written as a column's type is, so any
    /// [`ColumnType`] serves — a type created elsewhere by
    /// [`ColumnType::named`]. PostgreSQL drops a type modifier here: a range
    /// over `varchar(10)` ranges over `varchar`.
    // [spec:pgorm:req:sql.ddl.type-range+3]
    pub fn new(subtype: ColumnType) -> Self {
        Self {
            subtype,
            subtype_opclass: None,
            collation: None,
            subtype_diff: None,
            multirange_type_name: None,
        }
    }

    /// `SUBTYPE_OPCLASS = "opclass"`: the b-tree operator class ordering the
    /// subtype. The name is unqualified, as a function name is everywhere
    /// in this crate, and is found on the search path.
    // [spec:pgorm:req:sql.ddl.type-range+3]
    #[must_use]
    pub fn subtype_opclass<N>(mut self, opclass: N) -> Self
    where
        N: IntoName,
    {
        self.subtype_opclass = Some(opclass.into_name());
        self
    }

    /// `COLLATION = "collation"`: the collation a collatable subtype is
    /// ordered under.
    // [spec:pgorm:req:sql.ddl.type-range+3]
    #[must_use]
    pub fn collation<C>(mut self, collation: C) -> Self
    where
        C: IntoCollation,
    {
        self.collation = Some(collation.into_collation());
        self
    }

    /// `SUBTYPE_DIFF = "function"`: the subtype's difference function, which
    /// takes two subtype values and returns `double precision`. Unqualified,
    /// as [`subtype_opclass`](Self::subtype_opclass) is.
    // [spec:pgorm:req:sql.ddl.type-range+3]
    #[must_use]
    pub fn subtype_diff<N>(mut self, function: N) -> Self
    where
        N: IntoName,
    {
        self.subtype_diff = Some(function.into_name());
        self
    }

    /// `MULTIRANGE_TYPE_NAME = <name>`: what the multirange PostgreSQL
    /// creates beside the range is called, schema-qualified as the range's
    /// own name can be.
    // [spec:pgorm:req:sql.ddl.type-range+3]
    #[must_use]
    pub fn multirange_type_name<T>(mut self, name: T) -> Self
    where
        T: IntoTypeRef,
    {
        self.multirange_type_name = Some(name.into_type_ref());
        self
    }
}

impl TypeCreateStatement {
    /// Define the type as a range: `CREATE TYPE <name> AS RANGE (SUBTYPE =
    /// ..., ...)`.
    ///
    /// What a type is, is one slot, so this replaces an enumeration's labels
    /// or a composite's attributes, and [`as_enum`](Self::as_enum) or
    /// [`as_composite`](Self::as_composite) replaces the range.
    ///
    /// ```
    /// use pgorm_query::{extension::{RangeDefinition, Type}, *};
    ///
    /// assert_eq!(
    ///     Type::create((Name::runtime("app"), Name::runtime("textrange")))
    ///         .as_range(
    ///             RangeDefinition::new(ColumnType::Text)
    ///                 .collation(Name::runtime("C"))
    ///                 .multirange_type_name((Name::runtime("app"), Name::runtime("textranges"))),
    ///         )
    ///         .to_string(),
    ///     concat!(
    ///         r#"CREATE TYPE "app"."textrange" AS RANGE (SUBTYPE = text, COLLATION = "C", "#,
    ///         r#"MULTIRANGE_TYPE_NAME = "app"."textranges")"#,
    ///     )
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.type-range+3]
    pub fn as_range(&mut self, range: RangeDefinition) -> &mut Self {
        self.as_type = Some(TypeAs::Range(range));
        self
    }
}
