//! The kinds a binding's values carry: one of pgorm's value kinds, or the
//! identity a plain value of the language cannot carry, an enum's type and a
//! created range's type and subtype, and an array of either.

use std::sync::Arc;

use pgorm::{
    ColumnTypeTrait, CreatedRangeCodec, TableColumn,
    pgorm_query::{ArrayType, ColumnType, Name, SqlName, TypeName as QueryTypeName, Value},
};

use crate::{Dialect, Failure};

/// A PostgreSQL type named by identifier: an enum, or a range type a schema
/// created.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub struct TypeName {
    pub name: String,
    /// Its schema, or `None` for the search path's.
    pub schema: Option<String>,
}

impl TypeName {
    /// The name double-quoted, its own quotes doubled, behind its quoted
    /// schema when it has one.
    pub fn quoted(&self) -> String {
        let part = |part: &str| format!("\"{}\"", part.replace('"', "\"\""));
        match &self.schema {
            Some(schema) => format!("{}.{}", part(schema), part(&self.name)),
            None => part(&self.name),
        }
    }

    /// The type as pgorm-query names one in a cast, never verbatim SQL.
    pub fn rust_type(&self) -> QueryTypeName {
        let name = QueryTypeName::new(Name::runtime(&self.name));
        match &self.schema {
            Some(schema) => name.schema(Name::runtime(schema)),
            None => name,
        }
    }

    fn of(name: &Name, schema: &Option<Name>) -> Self {
        Self {
            name: SqlName::to_string(&**name),
            schema: schema.as_ref().map(|schema| SqlName::to_string(&**schema)),
        }
    }
}

/// A range or multirange type a schema created: its name, the value kind of
/// its subtype, and which of the two it is.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub struct CreatedKind {
    pub name: TypeName,
    /// The value kind each bound is.
    pub subtype: ArrayType,
    /// Whether it is the multirange of the type.
    pub multirange: bool,
}

impl CreatedKind {
    /// pgorm's codec of this kind's values, `None` for a subtype no
    /// `RangeSubtype` is, which no kind a binding makes has.
    pub fn codec(&self) -> Option<CreatedRangeCodec> {
        CreatedRangeCodec::new(&self.subtype, self.multirange)
    }

    /// The kind's name: `created_range` or `created_multirange`.
    pub fn kind_name(&self) -> &'static str {
        if self.multirange {
            "created_multirange"
        } else {
            "created_range"
        }
    }
}

/// The kind a value is.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum Tag {
    /// One of pgorm's `Value` kinds, which needs no type name.
    Scalar(ArrayType),
    /// A label of the enum type named.
    Enum(TypeName),
    /// A value of a range type a schema created.
    Created(CreatedKind),
    Array(Box<Tag>),
}

impl Tag {
    /// The kind a pgorm value declares itself.
    pub fn of_value(value: &Value) -> Self {
        let kind = Self::Scalar(value.array_type());
        if value.is_array() {
            Self::Array(Box::new(kind))
        } else {
            kind
        }
    }

    /// The kind a column's values bind and read as, or `None` for a type a
    /// binding has no kind for, whose values a caller gives as tagged
    /// values. A domain's values travel as its base type's do, so a domain
    /// column is the kind of the type it is over.
    // [spec:pgorm:req:bindings.entities+6]
    // [spec:pgorm:req:bindings.schema-domains+2]
    pub fn of_column(ty: &ColumnType) -> Option<Self> {
        match ty {
            ColumnType::Enum { name, schema, .. } => Some(Self::Enum(TypeName::of(name, schema))),
            // [spec:pgorm:req:bindings.created-ranges+5]
            ColumnType::CreatedRange { name, schema, .. }
            | ColumnType::CreatedMultirange { name, schema, .. } => {
                let codec = CreatedRangeCodec::of_column(ty)?;
                Some(Self::Created(CreatedKind {
                    name: TypeName::of(name, schema),
                    subtype: codec.subtype().clone(),
                    multirange: codec.is_multirange(),
                }))
            }
            ColumnType::Array(member) => {
                Self::of_column(member).map(|tag| Self::Array(Box::new(tag)))
            }
            ColumnType::Domain { base, .. } => Self::of_column(base),
            // A `money` column is read as `numeric` and written cast to
            // `money` by the entity's own casts, so its values are decimals;
            // no range a binding knows is over it, so `scalar` does not name
            // it.
            ColumnType::Money => Some(Self::Scalar(ArrayType::Decimal)),
            other => scalar(other).map(Self::Scalar),
        }
    }

    /// The kind's name: a scalar's own, or `enum`, `created_range`,
    /// `created_multirange` or `array`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Scalar(kind) => kind.kind_name(),
            Self::Enum(_) => "enum",
            Self::Created(kind) => kind.kind_name(),
            Self::Array(_) => "array",
        }
    }

    /// The kind as a model's and a registered entity's `describe()` spell
    /// it, as a decoded column's is compared: a scalar's name, an enum's
    /// quoted type, a created range's type and subtype, and an array's
    /// element followed by `[]`.
    // [spec:pgorm:req:bindings.models+6]
    // [spec:pgorm:req:bindings.model-records+1]
    pub fn key(&self) -> String {
        match self {
            Self::Scalar(kind) => kind.kind_name().to_owned(),
            Self::Enum(name) => format!("enum {}", name.quoted()),
            Self::Created(kind) => format!(
                "{} {} of {}",
                if kind.multirange {
                    "multirange"
                } else {
                    "range"
                },
                kind.name.quoted(),
                kind.subtype.kind_name()
            ),
            Self::Array(element) => format!("{}[]", element.key()),
        }
    }

    /// The column type a model column of the kind is written with: the
    /// type it declares where it declares one, made an array where the
    /// column is; otherwise a scalar kind's own (`ArrayType::column_type`),
    /// an enum of the labels `values` lists, a created range over its
    /// subtype's, and an array of its element's.
    // [spec:pgorm:req:bindings.schema-from-model]
    pub fn column_type(
        &self,
        values: &[String],
        declared: Option<&ColumnType>,
    ) -> Result<ColumnType, Failure> {
        if let Some(declared) = declared {
            return Ok(match self {
                Self::Array(_) => ColumnType::Array(Arc::new(declared.clone())),
                _ => declared.clone(),
            });
        }
        Ok(match self {
            Self::Scalar(kind) => kind.column_type().ok_or_else(|| {
                Failure::Construction(format!(
                    "no column decodes as `{}`, so a model's column cannot be one",
                    kind.kind_name()
                ))
            })?,
            Self::Enum(name) => ColumnType::Enum {
                name: Name::runtime(&name.name),
                schema: name.schema.as_ref().map(Name::runtime),
                variants: values.iter().map(Name::runtime).collect(),
            },
            Self::Created(kind) => {
                let subtype = Arc::new(Self::Scalar(kind.subtype.clone()).column_type(&[], None)?);
                let (name, schema) = (
                    Name::runtime(&kind.name.name),
                    kind.name.schema.as_ref().map(Name::runtime),
                );
                match kind.multirange {
                    true => ColumnType::CreatedMultirange {
                        name,
                        schema,
                        subtype,
                    },
                    false => ColumnType::CreatedRange {
                        name,
                        schema,
                        subtype,
                    },
                }
            }
            Self::Array(element) => ColumnType::Array(Arc::new(element.column_type(values, None)?)),
        })
    }

    /// Refuse a `column_type` the kind cannot be written as: a created
    /// range's is its own, and an enum's is a domain over that enum alone,
    /// which the column is read and compared as.
    // [spec:pgorm:req:bindings.model-casts]
    pub fn refuse_declared(&self, declared: &ColumnType, dialect: Dialect) -> Result<(), Failure> {
        let element = match self {
            Self::Array(element) => element,
            other => other,
        };
        let option = dialect.option("column_type");
        match element {
            Self::Created(_) => Err(Failure::Construction(format!(
                "a created range column's type is its kind's own: it takes no {option}"
            ))),
            Self::Enum(name) => {
                let column = TableColumn::new("column", declared.clone().def());
                match column.compares_as() == Some(name.rust_type()) {
                    true => Ok(()),
                    false => Err(Failure::Construction(format!(
                        "an enum column's {option} is a domain over its enum"
                    ))),
                }
            }
            _ => Ok(()),
        }
    }

    /// SQL NULL of the kind.
    pub fn null(&self) -> Value {
        match self {
            Self::Scalar(kind) => kind.null(),
            Self::Enum(_) | Self::Created(_) => Value::String(None),
            Self::Array(element) => match element.as_ref() {
                Self::Scalar(kind) => Value::Array(kind.clone(), None),
                _ => Value::Array(ArrayType::String, None),
            },
        }
    }
}

/// The scalar kind a column type's values are, where it has one.
fn scalar(ty: &ColumnType) -> Option<ArrayType> {
    Some(match ty {
        ColumnType::Char(Some(1)) => ArrayType::Char,
        ColumnType::Char(_) | ColumnType::String(_) | ColumnType::Text => ArrayType::String,
        ColumnType::Bytea => ArrayType::Bytes,
        ColumnType::SmallInteger => ArrayType::SmallInt,
        ColumnType::Integer => ArrayType::Int,
        ColumnType::BigInteger => ArrayType::BigInt,
        ColumnType::Float => ArrayType::Float,
        ColumnType::Double => ArrayType::Double,
        ColumnType::Decimal(_) => ArrayType::Decimal,
        ColumnType::Boolean => ArrayType::Bool,
        ColumnType::Date => ArrayType::Date,
        ColumnType::Time => ArrayType::Time,
        ColumnType::Timestamp => ArrayType::DateTime,
        ColumnType::TimestampWithTimeZone => ArrayType::DateTimeWithTimeZone,
        ColumnType::Json | ColumnType::JsonBinary => ArrayType::Json,
        ColumnType::Uuid => ArrayType::Uuid,
        ColumnType::Vector(_) => ArrayType::Vector,
        ColumnType::Inet | ColumnType::Cidr => ArrayType::IpNetwork,
        ColumnType::MacAddr => ArrayType::MacAddress,
        ColumnType::Interval(_) => ArrayType::Interval,
        ColumnType::Range(range) => ArrayType::Range(*range),
        ColumnType::Multirange(range) => ArrayType::Multirange(*range),
        _ => return None,
    })
}

/// An enum column's labels, in declaration order, an array of one's and a
/// domain over one included: what a generated declaration types its values
/// as.
// [spec:pgorm:req:bindings.schema-domains+2]
pub fn labels(ty: &ColumnType) -> Option<Vec<String>> {
    match ty {
        ColumnType::Enum { variants, .. } => Some(
            variants
                .iter()
                .map(|variant| SqlName::to_string(&**variant))
                .collect(),
        ),
        ColumnType::Array(member) | ColumnType::Domain { base: member, .. } => labels(member),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use pgorm::pgorm_query::{IntervalSpec, RangeType, StringLen, tests_cfg::Variants};

    use super::*;

    fn key(ty: &ColumnType) -> Option<String> {
        Tag::of_column(ty).as_ref().map(Tag::key)
    }

    fn name(name: &str, schema: Option<&str>) -> TypeName {
        TypeName {
            name: name.to_owned(),
            schema: schema.map(str::to_owned),
        }
    }

    // [spec:pgorm:req:bindings.models+6/test]    a kind is spelled as a decoded column's is
    // compared: a scalar's name, an enum's quoted type, a created range's type and subtype,
    // and an array's element followed by `[]`
    #[test]
    fn kinds_are_spelled_as_results_compare_them() {
        assert_eq!(Tag::Scalar(ArrayType::BigInt).key(), "i64");
        assert_eq!(
            Tag::Scalar(ArrayType::Range(RangeType::Int4)).key(),
            "int4range"
        );
        assert_eq!(
            Tag::Array(Box::new(Tag::Scalar(ArrayType::String))).key(),
            "text[]"
        );
        assert_eq!(
            Tag::Enum(name("mood", Some("app"))).key(),
            "enum \"app\".\"mood\""
        );
        assert_eq!(
            Tag::Array(Box::new(Tag::Enum(name("a\"b", None)))).key(),
            "enum \"a\"\"b\"[]"
        );
        let created = |multirange| CreatedKind {
            name: name("floatrange", Some("app")),
            subtype: ArrayType::Double,
            multirange,
        };
        assert_eq!(
            Tag::Created(created(false)).key(),
            "range \"app\".\"floatrange\" of f64"
        );
        assert_eq!(
            Tag::Created(created(true)).key(),
            "multirange \"app\".\"floatrange\" of f64"
        );
    }

    // [spec:pgorm:req:bindings.entities+6/test]    each column type maps to the kind a decoded
    // column of it carries, so a registered column's values bind and read as the binding's
    // own; a type it has no kind for takes a tagged value
    #[test]
    fn column_types_map_to_decoded_kinds() {
        assert_eq!(key(&ColumnType::Integer).as_deref(), Some("i32"));
        assert_eq!(
            key(&ColumnType::String(StringLen::None)).as_deref(),
            Some("text")
        );
        assert_eq!(key(&ColumnType::Char(Some(1))).as_deref(), Some("char"));
        assert_eq!(
            key(&ColumnType::TimestampWithTimeZone).as_deref(),
            Some("datetime_utc")
        );
        assert_eq!(
            key(&ColumnType::Array(Arc::new(ColumnType::BigInteger))).as_deref(),
            Some("i64[]")
        );
        assert_eq!(
            key(&ColumnType::Range(RangeType::Int4)).as_deref(),
            Some("int4range")
        );
        assert_eq!(
            key(&ColumnType::Multirange(RangeType::TimestampTz)).as_deref(),
            Some("tstzmultirange")
        );
        assert_eq!(
            key(&ColumnType::Array(Arc::new(ColumnType::Range(
                RangeType::Date
            ))))
            .as_deref(),
            Some("daterange[]")
        );
        let mood = ColumnType::Enum {
            name: Name::runtime("mood"),
            schema: Some(Name::runtime("app")),
            variants: Vec::new(),
        };
        assert_eq!(key(&mood).as_deref(), Some("enum \"app\".\"mood\""));
        let created = ColumnType::CreatedRange {
            name: Name::runtime("floatrange"),
            schema: Some(Name::runtime("measure")),
            subtype: Arc::new(ColumnType::Double),
        };
        assert_eq!(
            key(&created).as_deref(),
            Some("range \"measure\".\"floatrange\" of f64")
        );
        let multirange = ColumnType::CreatedMultirange {
            name: Name::runtime("slot_multirange"),
            schema: None,
            subtype: Arc::new(ColumnType::Integer),
        };
        assert_eq!(
            key(&multirange).as_deref(),
            Some("multirange \"slot_multirange\" of i32")
        );
        // [spec:pgorm:req:bindings.created-ranges+5/test]    a created range over `char(n)` is
        // a range of text, and one over a domain a range of the domain's base kind, as
        // pgorm's column-type mapping reads them
        let over_domain = ColumnType::CreatedRange {
            name: Name::runtime("heightrange"),
            schema: None,
            subtype: Arc::new(ColumnType::Domain {
                name: Name::runtime("height"),
                schema: None,
                base: Arc::new(ColumnType::Double),
            }),
        };
        assert_eq!(
            key(&over_domain).as_deref(),
            Some("range \"heightrange\" of f64")
        );
        for length in [None, Some(1), Some(4)] {
            let over_char = ColumnType::CreatedRange {
                name: Name::runtime("letters"),
                schema: None,
                subtype: Arc::new(ColumnType::Char(length)),
            };
            assert_eq!(
                key(&over_char).as_deref(),
                Some("range \"letters\" of text"),
                "{length:?}"
            );
        }
        assert_eq!(key(&ColumnType::Money).as_deref(), Some("decimal"));
        assert_eq!(
            key(&ColumnType::Array(Arc::new(ColumnType::Money))).as_deref(),
            Some("decimal[]")
        );
        assert_eq!(
            key(&ColumnType::Interval(IntervalSpec::Any(None))).as_deref(),
            Some("interval")
        );
    }

    // [spec:pgorm:req:bindings.schema-domains+2/test]    a domain column is the kind of the type
    // it is over, through a domain over a domain, an array of one and a domain over an enum,
    // whose labels it carries
    #[test]
    fn domain_columns_take_their_base_kind() {
        let over = |name: &str, base: ColumnType| ColumnType::Domain {
            name: Name::runtime(name),
            schema: Some(Name::runtime("billing")),
            base: Arc::new(base),
        };
        let quantity = over("quantity", ColumnType::Integer);
        assert_eq!(key(&quantity).as_deref(), Some("i32"));
        assert_eq!(
            key(&over("small", quantity.clone())).as_deref(),
            Some("i32")
        );
        assert_eq!(
            key(&ColumnType::Array(Arc::new(quantity.clone()))).as_deref(),
            Some("i32[]")
        );
        assert_eq!(
            key(&over("tags", ColumnType::Array(Arc::new(ColumnType::Text)))).as_deref(),
            Some("text[]")
        );
        let mood = ColumnType::Enum {
            name: Name::runtime("mood"),
            schema: None,
            variants: vec![Name::runtime("calm"), Name::runtime("tense")],
        };
        let calm = over("calm_mood", mood);
        assert_eq!(key(&calm).as_deref(), Some("enum \"mood\""));
        assert_eq!(
            labels(&ColumnType::Array(Arc::new(calm))),
            Some(vec!["calm".to_owned(), "tense".to_owned()])
        );
        assert_eq!(labels(&quantity), None);
    }

    // [spec:pgorm:req:bindings.entities+6/test]
    #[test]
    fn only_types_without_a_kind_have_no_tag() {
        let untagged: Vec<&str> = ColumnType::variants()
            .iter()
            .filter(|ty| Tag::of_column(ty).is_none())
            .map(Variants::variant_name)
            .collect();
        assert_eq!(untagged, ["Bit", "VarBit", "Named", "LTree"]);
    }
}
