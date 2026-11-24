//! `CREATE TYPE ... AS (composite)`: a row type and its attributes. The type
//! statement's file is over its function cap, so the composite form sits
//! beside it here.

use crate::{Collation, ColumnType, IntoCollation, IntoName, Name};

use super::{TypeAs, TypeCreateStatement};

/// One attribute of a composite type: `"name" <type>[ COLLATE "collation"]`.
///
/// It is the part of a column definition a composite attribute can carry and
/// no more: PostgreSQL's grammar gives an attribute a name, a type and a
/// collation, and refuses `NOT NULL`, `DEFAULT`, a constraint or `GENERATED`
/// after it as a syntax error, so the attribute is not a
/// [`ColumnDef`](crate::ColumnDef) whose specs could spell them.
// [spec:pgorm:req:sql.ddl.type-composite+3]
#[derive(Debug, Clone)]
pub struct CompositeAttribute {
    pub(crate) name: Name,
    pub(crate) column_type: ColumnType,
    pub(crate) collation: Option<Collation>,
}

impl TypeCreateStatement {
    /// Define the type as a composite, whose attributes are appended by
    /// [`attribute`](Self::attribute)
    ///
    /// A composite with no attributes is a type PostgreSQL accepts, and
    /// renders with its parentheses:
    ///
    /// ```
    /// use pgorm_query::{extension::Type, *};
    ///
    /// assert_eq!(
    ///     Type::create(Name::runtime("nothing")).as_composite().to_string(),
    ///     r#"CREATE TYPE "nothing" AS ()"#
    /// );
    /// ```
    ///
    /// The type is an enumeration or a composite, never both: what it is is
    /// one slot, so this replaces an enumeration's labels, as
    /// [`as_enum`](Self::as_enum) replaces a composite's attributes.
    // [spec:pgorm:req:sql.ddl.type-composite+3]
    pub fn as_composite(&mut self) -> &mut Self {
        if !matches!(self.as_type, Some(TypeAs::Composite(_))) {
            self.as_type = Some(TypeAs::Composite(Vec::new()));
        }
        self
    }

    /// Append an attribute, defining the type as a composite if it is not one
    /// already
    ///
    /// The type is written as a column's is, so any [`ColumnType`] serves —
    /// an array, an enumeration, another composite by
    /// [`ColumnType::named`].
    ///
    /// ```
    /// use pgorm_query::{extension::Type, *};
    ///
    /// assert_eq!(
    ///     Type::create(Name::runtime("address"))
    ///         .attribute(Name::runtime("street"), ColumnType::Text)
    ///         .attribute(Name::runtime("no"), ColumnType::Integer)
    ///         .to_string(),
    ///     r#"CREATE TYPE "address" AS ("street" text, "no" integer)"#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.type-composite+3]
    pub fn attribute<N>(&mut self, name: N, column_type: ColumnType) -> &mut Self
    where
        N: IntoName,
    {
        self.push_attribute(CompositeAttribute {
            name: name.into_name(),
            column_type,
            collation: None,
        })
    }

    /// Append an attribute under a named collation, `"name" <type> COLLATE
    /// "collation"`
    ///
    /// ```
    /// use pgorm_query::{extension::Type, *};
    ///
    /// assert_eq!(
    ///     Type::create(Name::runtime("label"))
    ///         .attribute_collated(Name::runtime("text"), ColumnType::Text, Name::runtime("C"))
    ///         .to_string(),
    ///     r#"CREATE TYPE "label" AS ("text" text COLLATE "C")"#
    /// );
    /// ```
    // [spec:pgorm:req:sql.ddl.type-composite+3]
    pub fn attribute_collated<N, C>(
        &mut self,
        name: N,
        column_type: ColumnType,
        collation: C,
    ) -> &mut Self
    where
        N: IntoName,
        C: IntoCollation,
    {
        self.push_attribute(CompositeAttribute {
            name: name.into_name(),
            column_type,
            collation: Some(collation.into_collation()),
        })
    }

    fn push_attribute(&mut self, attribute: CompositeAttribute) -> &mut Self {
        self.as_composite();
        if let Some(TypeAs::Composite(attributes)) = self.as_type.as_mut() {
            attributes.push(attribute);
        }
        self
    }
}
