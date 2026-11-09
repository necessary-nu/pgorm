use crate::{ColumnExpr, EnumHome, Error, entity::created_range::created_range_subtype};
use heck::{ToSnakeCase, ToUpperCamelCase};
use pgorm_query::{
    ColumnDef, ColumnSpec, ColumnType, GeneratedKind, IdentityGeneration, Name, RangeType,
    StringLen,
};

#[derive(Clone, Debug)]
pub struct Column {
    pub(crate) name: String,
    pub(crate) col_type: ColumnType,
    pub(crate) auto_increment: bool,
    /// The identity form generating the column, if it is one.
    pub(crate) identity: Option<IdentityGeneration>,
    pub(crate) not_null: bool,
    pub(crate) unique: bool,
    /// The column's `DEFAULT`, if it has one.
    // [spec:pgorm:sem:codegen.entity.expressions+3]
    pub(crate) default: Option<ColumnExpr>,
    /// The expression a generated column is computed from, and its kind.
    // [spec:pgorm:sem:codegen.entity.expressions+3]
    pub(crate) generated: Option<Generation>,
    /// Where the enum the column's type names is declared, when that is not
    /// `pgorm_active_enums`: a disjoint entity's kind type, which its derive
    /// declares in the base's module.
    // [spec:pgorm:sem:codegen.entity.disjoint+3]
    pub(crate) enum_home: Option<EnumHome>,
}

/// A generated column's expression and kind.
pub(crate) type Generation = (ColumnExpr, GeneratedKind);

impl Column {
    /// Reject a type outside the mapping table, which no writer could
    /// render. Whether the DB name's case-converted forms are Rust
    /// identifiers is the Rust writer's gate's to check.
    // [spec:pgorm:req:codegen.entity.types.unsupported+12]
    pub(crate) fn validate(&self) -> Result<(), Error> {
        let context = format!("column `{}`", self.name);
        validate_col_type(&context, &self.col_type)
    }

    pub fn is_snake_case_name(&self) -> bool {
        self.name.to_snake_case() == self.name
    }

    /// The type the column's values are made of: an array's element, under
    /// any domains either is typed with.
    pub fn get_inner_col_type(&self) -> &ColumnType {
        match under_domains(&self.col_type) {
            ColumnType::Array(inner_col_type) => under_domains(inner_col_type),
            col_type => col_type,
        }
    }
}

/// The Rust type `col_type`'s values are, as generated source names it,
/// whether or not its column is nullable: an enum by the name `home` gives
/// it, where it is declared away from `pgorm_active_enums`.
// [spec:pgorm:sem:codegen.entity.types+13]
pub(crate) fn rs_type_text(col_type: &ColumnType, home: Option<&EnumHome>) -> String {
    #[allow(unreachable_patterns)]
    match col_type {
        ColumnType::Char(_) | ColumnType::String(_) | ColumnType::Text | ColumnType::Named(_) => {
            "String".to_owned()
        }
        ColumnType::SmallInteger => "i16".to_owned(),
        ColumnType::Integer => "i32".to_owned(),
        ColumnType::BigInteger => "i64".to_owned(),
        ColumnType::Float => "f32".to_owned(),
        ColumnType::Double => "f64".to_owned(),
        ColumnType::Json | ColumnType::JsonBinary => "Json".to_owned(),
        ColumnType::Date => "Date".to_owned(),
        ColumnType::Time => "Time".to_owned(),
        ColumnType::Timestamp => "DateTime".to_owned(),
        ColumnType::TimestampWithTimeZone => "DateTimeWithTimeZone".to_owned(),
        ColumnType::Decimal(_) | ColumnType::Money => "Decimal".to_owned(),
        ColumnType::Uuid => "Uuid".to_owned(),
        ColumnType::Bytea => "Vec<u8>".to_owned(),
        ColumnType::Boolean => "bool".to_owned(),
        ColumnType::Interval(_) => "Interval".to_owned(),
        ColumnType::Enum { name, .. } => enum_rust_name(name, home),
        ColumnType::CreatedRange { name, .. } | ColumnType::CreatedMultirange { name, .. } => {
            name.to_string().to_upper_camel_case()
        }
        ColumnType::Array(column_type) => {
            format!("Vec<{}>", rs_type_text(column_type, home))
        }
        ColumnType::Range(range) => format!("Range<{}>", checked_range_spelling(*range).1),
        ColumnType::Multirange(range) => {
            format!("Multirange<{}>", checked_range_spelling(*range).1)
        }
        ColumnType::Domain { base, .. } => rs_type_text(base, home),
        other => unreachable!(
            "column type {other:?} reached the writer; \
             every Column is type-checked when it is built"
        ),
    }
}

/// The Rust name of the enum `name`: the UpperCamelCase of the DB name, or
/// the name a disjoint entity's derive gives its kind type.
// [spec:pgorm:sem:codegen.entity.types+13]
pub(crate) fn enum_rust_name(name: &Name, home: Option<&EnumHome>) -> String {
    match home {
        Some(home) => home.name.clone(),
        None => name.to_string().to_upper_camel_case(),
    }
}

/// The type a domain's values are, under any domains it is over; any other
/// type itself.
pub(crate) fn under_domains(col_type: &ColumnType) -> &ColumnType {
    match col_type {
        ColumnType::Domain { base, .. } => under_domains(base),
        _ => col_type,
    }
}

/// A built-in range as generated source spells it: the `RangeType` variant
/// naming it, and the Rust type it ranges over as the prelude names it.
/// `None` for one codegen has no spelling of, which `validate_col_type`
/// refuses, so the writers meet only ranges this answers for.
// [spec:pgorm:sem:codegen.entity.types+13]
fn range_spelling(range: RangeType) -> Option<(&'static str, &'static str)> {
    Some(match range {
        RangeType::Int4 => ("Int4", "i32"),
        RangeType::Int8 => ("Int8", "i64"),
        RangeType::Numeric => ("Numeric", "Decimal"),
        RangeType::Date => ("Date", "Date"),
        RangeType::Timestamp => ("Timestamp", "DateTime"),
        RangeType::TimestampTz => ("TimestampTz", "DateTimeWithTimeZone"),
        _ => return None,
    })
}

/// [`range_spelling`] of a range a built `Column` holds.
pub(crate) fn checked_range_spelling(range: RangeType) -> (&'static str, &'static str) {
    range_spelling(range).unwrap_or_else(|| {
        unreachable!(
            "range type {range:?} reached the writer; \
             every Column is type-checked when it is built"
        )
    })
}

/// The set of `ColumnType`s the Rust writer's `get_rs_type` and
/// `write_col_type` can render, checked through `Array` element types and over
/// the named types they will respell. The enum and range names they will emit
/// are the Rust writer's gate's to check.
// [spec:pgorm:req:codegen.entity.types.unsupported+12]
fn validate_col_type(context: &str, col_type: &ColumnType) -> Result<(), Error> {
    match col_type {
        ColumnType::Char(_)
        | ColumnType::String(_)
        | ColumnType::Text
        | ColumnType::SmallInteger
        | ColumnType::Integer
        | ColumnType::BigInteger
        | ColumnType::Float
        | ColumnType::Double
        | ColumnType::Json
        | ColumnType::JsonBinary
        | ColumnType::Date
        | ColumnType::Time
        | ColumnType::Timestamp
        | ColumnType::TimestampWithTimeZone
        | ColumnType::Decimal(_)
        | ColumnType::Money
        | ColumnType::Uuid
        | ColumnType::Bytea
        | ColumnType::Boolean
        | ColumnType::Interval(_)
        | ColumnType::Enum { .. } => Ok(()),
        // The writer respells a named type as `ColumnType::named("<name>")`,
        // whose argument rebuilds one unqualified, non-array, quoted name. The
        // characters of that name are unconstrained — it becomes a string
        // literal, never a Rust identifier, and `citext`, `CamelType` and
        // `my type` are all type names Postgres accepts — but a `TypeName`
        // carrying anything the one-name spelling cannot hold would be
        // generated as a different type than the one described, so it is
        // refused here rather than silently respelled.
        ColumnType::Named(type_name) => {
            if type_name.is_verbatim() || type_name.schema.is_some() || type_name.array {
                return Err(Error::TransformError(format!(
                    "{context}: named column type `{}` is not supported by codegen; \
                     only a bare type name survives the generated \
                     `ColumnType::named(\"..\")`",
                    type_name.to_sql_string()
                )));
            }
            Ok(())
        }
        ColumnType::Range(range) | ColumnType::Multirange(range) => match range_spelling(*range) {
            Some(_) => Ok(()),
            None => Err(Error::TransformError(format!(
                "{context}: range type {range:?} is not supported by codegen"
            ))),
        },
        // A range type a schema created is generated as a newtype over
        // `Range<T>`, which only `RangeSubtype`'s subtypes can be.
        ColumnType::CreatedRange { name, subtype, .. }
        | ColumnType::CreatedMultirange { name, subtype, .. } => {
            if created_range_subtype(subtype).is_none() {
                return Err(Error::TransformError(format!(
                    "{context}: range type `{name}` over {subtype:?} is not supported by \
                     codegen; a created range's subtype is one pgorm's `RangeSubtype` covers",
                    name = name.to_string(),
                )));
            }
            // A modifier would claim a length or precision the range's
            // bounds are not held to.
            if matches!(
                subtype.as_ref(),
                ColumnType::Decimal(Some(_))
                    | ColumnType::Char(Some(_))
                    | ColumnType::String(StringLen::N(_))
            ) {
                return Err(Error::TransformError(format!(
                    "{context}: range type `{name}` over {subtype:?} is not supported by \
                     codegen; PostgreSQL keeps no type modifier on a range's subtype",
                    name = name.to_string(),
                )));
            }
            Ok(())
        }
        ColumnType::Array(inner_col_type) => match under_domains(inner_col_type) {
            // A field is one `Vec` of its element's Rust type, and no `Vec`
            // but `Vec<u8>` is an element (`NotU8`), so `Vec<Vec<T>>` has no
            // `ValueType` and an entity holding it would not compile.
            ColumnType::Array(_) => {
                let element = match inner_col_type.as_ref() {
                    ColumnType::Domain { .. } => "a domain over an array",
                    _ => "an array",
                };
                Err(Error::TransformError(format!(
                    "{context}: an array of {element} is not supported by codegen; \
                     an entity field holds one dimension"
                )))
            }
            _ => validate_col_type(context, inner_col_type),
        },
        // A domain's column holds its base type's values, so it is generated
        // as its base is; the name rides in the column's definition.
        ColumnType::Domain { base, .. } => validate_col_type(context, base),
        other => Err(Error::TransformError(format!(
            "{context}: column type {other:?} is not supported by codegen"
        ))),
    }
}

impl TryFrom<ColumnDef> for Column {
    type Error = Error;

    fn try_from(col_def: ColumnDef) -> Result<Self, Self::Error> {
        Self::try_from(&col_def)
    }
}

// [spec:pgorm:sem:codegen.entity.transform+18]
impl TryFrom<&ColumnDef> for Column {
    type Error = Error;

    fn try_from(col_def: &ColumnDef) -> Result<Self, Self::Error> {
        let name = col_def.get_column_name();
        let Some(col_type) = col_def.get_column_type().cloned() else {
            return Err(Error::TransformError(format!(
                "column `{name}`: column type should not be empty"
            )));
        };
        let auto_increment = col_def
            .get_column_spec()
            .iter()
            .any(|spec| matches!(spec, ColumnSpec::AutoIncrement));
        let mut identity = None;
        for spec in col_def.get_column_spec() {
            if let ColumnSpec::Identity(generation, options) = spec {
                if options.is_some() {
                    return Err(Error::TransformError(format!(
                        "column `{name}`: an entity declares an identity's form, \
                         not its sequence options"
                    )));
                }
                identity = Some(*generation);
            }
        }
        // An identity column is NOT NULL whether or not it says so.
        let not_null = identity.is_some()
            || col_def
                .get_column_spec()
                .iter()
                .any(|spec| matches!(spec, ColumnSpec::NotNull { .. }));
        let (default, generated) = expressions(col_def, &name, auto_increment, identity)?;
        // Uniqueness is a key of the table's, which the transform reads from
        // the table's unique keys.
        let column = Self {
            name,
            col_type,
            auto_increment,
            identity,
            not_null,
            unique: false,
            default,
            generated,
            enum_home: None,
        };
        column.validate()?;
        Ok(column)
    }
}

/// A column's `DEFAULT` and generation expression, read into the subset, and
/// refused where PostgreSQL refuses the pair (`42601`): a column takes one of
/// a default, an identity, a generation expression and the serial family.
// [spec:pgorm:sem:codegen.entity.expressions+3]
fn expressions(
    col_def: &ColumnDef,
    name: &str,
    auto_increment: bool,
    identity: Option<IdentityGeneration>,
) -> Result<(Option<ColumnExpr>, Option<Generation>), Error> {
    let unread = |what: &str, problem: String| {
        Error::TransformError(format!(
            "column `{name}`: {what} codegen cannot read: {problem}"
        ))
    };
    let mut default = None;
    let mut generated = None;
    for spec in col_def.get_column_spec() {
        match spec {
            ColumnSpec::Default(expr) => {
                default = Some(ColumnExpr::read(expr).map_err(|p| unread("a DEFAULT", p))?);
            }
            ColumnSpec::Generated { expr, kind } => {
                let expr =
                    ColumnExpr::read(expr).map_err(|p| unread("a generation expression", p))?;
                generated = Some((expr, *kind));
            }
            _ => {}
        }
    }
    let ways = [
        default.is_some(),
        generated.is_some(),
        identity.is_some(),
        auto_increment,
    ];
    if ways.iter().filter(|way| **way).count() > 1 {
        return Err(Error::TransformError(format!(
            "column `{name}`: a column takes one of a DEFAULT, a generation expression, an \
             identity and the serial family; PostgreSQL refuses two"
        )));
    }
    Ok((default, generated))
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::Column;
    use pgorm_query::{ColumnDef, ColumnType, Name, StringLen};

    pub(crate) fn to_column(col_def: ColumnDef) -> Column {
        Column::try_from(col_def).expect("column def should convert")
    }

    #[test]
    fn test_from_column_def() {
        let column = to_column(ColumnDef::new(Name::runtime("id")).string().to_owned());
        assert_eq!(column.col_type, ColumnType::String(StringLen::None));
        assert!(!column.not_null);

        let column = to_column(
            ColumnDef::new(Name::runtime("id"))
                .string()
                .not_null()
                .to_owned(),
        );
        assert!(column.not_null);

        let column = to_column(
            ColumnDef::new(Name::runtime("id"))
                .string()
                .auto_increment()
                .not_null()
                .to_owned(),
        );
        assert!(column.auto_increment);
        assert!(!column.unique);
        assert!(column.not_null);
    }
}
