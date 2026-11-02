use crate::{
    ColumnExpr, Error,
    entity::created_range::created_range_subtype,
    util::{escape_rust_keyword, safe_ident},
};
use heck::{ToSnakeCase, ToUpperCamelCase};
use pgorm_query::{
    ColumnDef, ColumnSpec, ColumnType, GeneratedKind, IdentityGeneration, IntervalPrecision,
    IntervalSpec, Name, PgInterval, RangeType, StringLen,
};
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};
use std::fmt::Write as FmtWrite;

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
    // [spec:pgorm:sem:codegen.entity.expressions+1]
    pub(crate) default: Option<ColumnExpr>,
    /// The expression a generated column is computed from, and its kind.
    // [spec:pgorm:sem:codegen.entity.expressions+1]
    pub(crate) generated: Option<Generation>,
}

/// A generated column's expression and kind.
pub(crate) type Generation = (ColumnExpr, GeneratedKind);

impl Column {
    /// Reject anything the writer could not render: a type outside the mapping
    /// table, and a DB name whose case-converted forms are not Rust
    /// identifiers.
    // [spec:pgorm:req:codegen.entity.types.unsupported+9]
    // [spec:pgorm:sem:codegen.entity.keywords+2]
    pub(crate) fn validate(&self) -> Result<(), Error> {
        let context = format!("column `{}`", self.name);
        validate_col_type(&context, &self.col_type)?;
        safe_ident(&context, &escape_rust_keyword(self.name.to_snake_case()))?;
        safe_ident(
            &context,
            &escape_rust_keyword(self.name.to_upper_camel_case()),
        )?;
        Ok(())
    }

    // [spec:pgorm:sem:codegen.entity.keywords+2]
    pub fn get_name_snake_case(&self) -> Ident {
        format_ident!("{}", escape_rust_keyword(self.name.to_snake_case()))
    }

    pub fn get_name_camel_case(&self) -> Ident {
        format_ident!("{}", escape_rust_keyword(self.name.to_upper_camel_case()))
    }

    pub fn is_snake_case_name(&self) -> bool {
        self.name.to_snake_case() == self.name
    }

    // [spec:pgorm:sem:codegen.entity.types+12]
    // [spec:pgorm:sem:codegen.entity.types.datetime+2]
    // [spec:pgorm:req:codegen.entity.types.unsupported+9]
    pub fn get_rs_type(&self) -> TokenStream {
        fn write_rs_type(col_type: &ColumnType) -> String {
            #[allow(unreachable_patterns)]
            match col_type {
                ColumnType::Char(_)
                | ColumnType::String(_)
                | ColumnType::Text
                | ColumnType::Named(_) => "String".to_owned(),
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
                ColumnType::Enum { name, .. }
                | ColumnType::CreatedRange { name, .. }
                | ColumnType::CreatedMultirange { name, .. } => {
                    name.to_string().to_upper_camel_case()
                }
                ColumnType::Array(column_type) => {
                    format!("Vec<{}>", write_rs_type(column_type))
                }
                ColumnType::Range(range) => format!("Range<{}>", checked_range_spelling(*range).1),
                ColumnType::Multirange(range) => {
                    format!("Multirange<{}>", checked_range_spelling(*range).1)
                }
                ColumnType::Domain { base, .. } => write_rs_type(base),
                other => unreachable!(
                    "column type {other:?} reached the writer; \
                     every Column is type-checked when it is built"
                ),
            }
        }
        let ident: TokenStream = write_rs_type(&self.col_type)
            .parse()
            .expect("mapped Rust type names are token text");
        match self.not_null {
            true => quote! { #ident },
            false => quote! { Option<#ident> },
        }
    }

    /// The `column_type = ..` part of the compact `#[pgorm(..)]` attribute,
    /// the expression the derive builds the column's type from.
    // [spec:pgorm:sem:codegen.entity.compact.attrs+12]
    pub fn get_col_type_attrs(&self) -> Option<TokenStream> {
        col_type_attr(&self.col_type).map(|ty| quote! { column_type = #ty })
    }

    /// The field attribute carrying the column's `DEFAULT` or generation
    /// expression, the Rust that builds it written bare: `default_expr = ..`,
    /// `generated_stored = ..` or `generated_virtual = ..`.
    // [spec:pgorm:sem:codegen.entity.expressions+1]
    // [spec:pgorm:req:macros.derive.code-values+2]
    pub fn get_expression_attr(&self) -> Option<TokenStream> {
        if let Some(default) = &self.default {
            let source = default.to_rust();
            return Some(quote! { default_expr = #source });
        }
        let (expr, kind) = self.generated.as_ref()?;
        let source = expr.to_rust();
        Some(match kind {
            GeneratedKind::Stored => quote! { generated_stored = #source },
            GeneratedKind::Virtual => quote! { generated_virtual = #source },
        })
    }

    /// The field attribute declaring the column's identity, if it is one.
    // [spec:pgorm:sem:codegen.entity.compact.attrs+12]
    pub fn get_identity_attr(&self) -> Option<TokenStream> {
        self.identity.map(|identity| match identity {
            IdentityGeneration::Always => quote! { identity },
            IdentityGeneration::ByDefault => quote! { identity_by_default },
        })
    }

    pub fn get_info(&self) -> String {
        let mut info = String::new();
        let type_info = self.get_rs_type().to_string().replace(' ', "");
        let col_info = self.col_info();
        write!(
            &mut info,
            "Column `{}`: {}{}",
            self.name, type_info, col_info
        )
        .unwrap();
        info
    }

    fn col_info(&self) -> String {
        let mut info = String::new();
        if self.auto_increment {
            write!(&mut info, ", auto_increment").unwrap();
        }
        match self.identity {
            Some(IdentityGeneration::Always) => write!(&mut info, ", identity").unwrap(),
            Some(IdentityGeneration::ByDefault) => {
                write!(&mut info, ", identity_by_default").unwrap()
            }
            None => {}
        }
        if self.not_null {
            write!(&mut info, ", not_null").unwrap();
        }
        if self.unique {
            write!(&mut info, ", unique").unwrap();
        }
        info
    }

    // [spec:pgorm:sem:codegen.entity.serde.skip]
    pub fn get_serde_attribute(
        &self,
        is_primary_key: bool,
        serde_skip_deserializing_primary_key: bool,
        serde_skip_hidden_column: bool,
    ) -> TokenStream {
        if self.name.starts_with('_') && serde_skip_hidden_column {
            quote! {
                #[serde(skip)]
            }
        } else if serde_skip_deserializing_primary_key && is_primary_key {
            quote! {
                #[serde(skip_deserializing)]
            }
        } else {
            quote! {}
        }
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

/// The Rust that builds a `ColumnType`. An enum reaches it only inside
/// another type — an array's element, a domain's base — and is its
/// `ActiveEnum`'s column type itself, `db_type().get_column_type().clone()`,
/// since the `Arc` holding it holds a `ColumnType` where `db_type()` returns
/// a `ColumnDef`.
// [spec:pgorm:req:codegen.entity.types.unsupported+9]
// [spec:pgorm:sem:codegen.entity.types+12]
fn write_col_type(col_type: &ColumnType) -> TokenStream {
    match col_type {
        ColumnType::Char(s) => match s {
            Some(s) => quote! { ColumnType::Char(Some(#s)) },
            None => quote! { ColumnType::Char(None) },
        },
        ColumnType::String(s) => match s {
            StringLen::N(s) => quote! { ColumnType::String(StringLen::N(#s)) },
            StringLen::None => quote! { ColumnType::String(StringLen::None) },
            StringLen::Max => quote! { ColumnType::String(StringLen::Max) },
        },
        ColumnType::Text => quote! { ColumnType::Text },
        ColumnType::SmallInteger => quote! { ColumnType::SmallInteger },
        ColumnType::Integer => quote! { ColumnType::Integer },
        ColumnType::BigInteger => quote! { ColumnType::BigInteger },
        ColumnType::Float => quote! { ColumnType::Float },
        ColumnType::Double => quote! { ColumnType::Double },
        ColumnType::Decimal(s) => match s {
            Some((s1, s2)) => quote! { ColumnType::Decimal(Some((#s1, #s2))) },
            None => quote! { ColumnType::Decimal(None) },
        },
        ColumnType::Timestamp => quote! { ColumnType::Timestamp },
        ColumnType::TimestampWithTimeZone => {
            quote! { ColumnType::TimestampWithTimeZone }
        }
        ColumnType::Time => quote! { ColumnType::Time },
        ColumnType::Date => quote! { ColumnType::Date },
        ColumnType::Interval(spec) => {
            let spec = interval_spec_tokens(spec);
            quote! { ColumnType::Interval(#spec) }
        }
        ColumnType::Bytea => quote! { ColumnType::Bytea },
        ColumnType::Boolean => quote! { ColumnType::Boolean },
        ColumnType::Money => quote! { ColumnType::Money },
        ColumnType::Json => quote! { ColumnType::Json },
        ColumnType::JsonBinary => quote! { ColumnType::JsonBinary },
        ColumnType::Uuid => quote! { ColumnType::Uuid },
        ColumnType::Named(type_name) => {
            let s = type_name.raw_text();
            quote! { ColumnType::named(#s) }
        }
        ColumnType::Enum { name, .. } => {
            let enum_ident = format_ident!("{}", name.to_string().to_upper_camel_case());
            quote! { #enum_ident::db_type().get_column_type().clone() }
        }
        ColumnType::Array(column_type) => {
            let column_type = write_col_type(column_type);
            quote! { ColumnType::Array(Arc::new(#column_type)) }
        }
        ColumnType::Range(range) => {
            let range = range_type_tokens(*range);
            quote! { ColumnType::Range(#range) }
        }
        ColumnType::Multirange(range) => {
            let range = range_type_tokens(*range);
            quote! { ColumnType::Multirange(#range) }
        }
        ColumnType::CreatedRange {
            name,
            schema,
            subtype,
        }
        | ColumnType::CreatedMultirange {
            name,
            schema,
            subtype,
        } => {
            let variant = match col_type {
                ColumnType::CreatedRange { .. } => quote! { CreatedRange },
                _ => quote! { CreatedMultirange },
            };
            let name = name.to_string();
            let schema = match schema {
                Some(schema) => {
                    let schema = schema.to_string();
                    quote! { Some(Name::runtime(#schema)) }
                }
                None => quote! { None },
            };
            let subtype = write_col_type(subtype);
            quote! {
                ColumnType::#variant {
                    name: Name::runtime(#name),
                    schema: #schema,
                    subtype: Arc::new(#subtype),
                }
            }
        }
        ColumnType::Domain { name, schema, base } => domain_tokens(name, schema, base),
        #[allow(unreachable_patterns)]
        other => unreachable!(
            "column type {other:?} reached the writer; \
                 every Column is type-checked when it is built"
        ),
    }
}

/// A domain column's `ColumnType::Domain`, its name and schema as runtime
/// names and its base built as a `ColumnType`.
// [spec:pgorm:sem:codegen.entity.types+12]
fn domain_tokens(name: &Name, schema: &Option<Name>, base: &ColumnType) -> TokenStream {
    let name = name.to_string();
    let schema = match schema {
        Some(schema) => {
            let schema = schema.to_string();
            quote! { Some(Name::runtime(#schema)) }
        }
        None => quote! { None },
    };
    let base = write_col_type(base);
    quote! {
        ColumnType::Domain {
            name: Name::runtime(#name),
            schema: #schema,
            base: Arc::new(#base)
        }
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
// [spec:pgorm:sem:codegen.entity.types+12]
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
fn checked_range_spelling(range: RangeType) -> (&'static str, &'static str) {
    range_spelling(range).unwrap_or_else(|| {
        unreachable!(
            "range type {range:?} reached the writer; \
             every Column is type-checked when it is built"
        )
    })
}

/// The `ColumnType` expression the compact derive's `column_type` takes: the
/// column's whole type, as `write_col_type` spells it, wherever the derive's
/// inference from the field's Rust type is not that type, and on a `Float`,
/// `Double` or `Bytea` scalar besides; `None` elsewhere.
///
/// Every name the described schema contributes to it — a named type's, a
/// domain's or a created range's — reaches it as a string-literal token,
/// `quote!`'s own escaping of a `&str`, never as text placed between two
/// quote characters, so a name holding a `"` comes back out of the derive as
/// the name it went in as. `validate_col_type` has already refused any
/// `Named` shape the one-name spelling could not carry.
// [spec:pgorm:req:macros.derive.one-form]
// [spec:pgorm:sem:codegen.entity.compact.attrs+12]
fn col_type_attr(col_type: &ColumnType) -> Option<TokenStream> {
    let stated = matches!(
        col_type,
        ColumnType::Float | ColumnType::Double | ColumnType::Bytea
    ) || !infers_itself(col_type);
    stated.then(|| write_col_type(col_type))
}

/// Whether the `Entity` derive, given no `column_type`, builds exactly
/// `col_type` from the Rust type `get_rs_type` writes for it: from its
/// table of bare names, else from the type's `ValueType::column_type()`
/// (`macros.derive.entity-model.column-def`).
///
/// `String` infers an unbounded `varchar` and `Decimal` an unconstrained
/// `numeric`, so a length, a precision and scale, `char`, `text`, a named
/// type and `money` are each beyond what the field says; `Json` infers
/// `json`, `Interval` an unqualified `interval`. An array infers the array of
/// what its element infers, and a created range's newtype the range over what
/// its `Range<E>`'s `E` infers, so each infers itself exactly when its
/// element or subtype does. No Rust type infers a domain. A type this table
/// does not know is taken not to infer itself, so it is stated rather than
/// lost.
// [spec:pgorm:sem:codegen.entity.compact.attrs+12]
fn infers_itself(col_type: &ColumnType) -> bool {
    match col_type {
        ColumnType::String(StringLen::None)
        | ColumnType::SmallInteger
        | ColumnType::Integer
        | ColumnType::BigInteger
        | ColumnType::Float
        | ColumnType::Double
        | ColumnType::Json
        | ColumnType::Date
        | ColumnType::Time
        | ColumnType::Timestamp
        | ColumnType::TimestampWithTimeZone
        | ColumnType::Decimal(None)
        | ColumnType::Uuid
        | ColumnType::Bytea
        | ColumnType::Boolean
        | ColumnType::Interval(IntervalSpec::Any(None))
        | ColumnType::Enum { .. }
        | ColumnType::Range(_)
        | ColumnType::Multirange(_) => true,
        ColumnType::Array(element) => infers_itself(element),
        ColumnType::CreatedRange { subtype, .. }
        | ColumnType::CreatedMultirange { subtype, .. } => infers_itself(subtype),
        _ => false,
    }
}

/// The `IntervalSpec` an interval column carries, as generated source spells
/// it: the qualifier vocabulary resolves through the prelude, as `RangeType`
/// does.
// [spec:pgorm:sem:codegen.entity.types+12]
fn interval_spec_tokens(spec: &IntervalSpec) -> TokenStream {
    let precision = |precision: &Option<IntervalPrecision>| match precision {
        Some(precision) => {
            let variant = format_ident!("P{}", precision.digits());
            quote! { Some(IntervalPrecision::#variant) }
        }
        None => quote! { None },
    };
    let fields = match spec {
        IntervalSpec::Any(digits) => {
            let digits = precision(digits);
            return quote! { IntervalSpec::Any(#digits) };
        }
        IntervalSpec::Fields(fields) => fields,
    };
    let fields = match fields {
        PgInterval::Year => quote! { Year },
        PgInterval::Month => quote! { Month },
        PgInterval::Day => quote! { Day },
        PgInterval::Hour => quote! { Hour },
        PgInterval::Minute => quote! { Minute },
        PgInterval::YearToMonth => quote! { YearToMonth },
        PgInterval::DayToHour => quote! { DayToHour },
        PgInterval::DayToMinute => quote! { DayToMinute },
        PgInterval::HourToMinute => quote! { HourToMinute },
        PgInterval::Second(digits) => {
            let digits = precision(digits);
            quote! { Second(#digits) }
        }
        PgInterval::DayToSecond(digits) => {
            let digits = precision(digits);
            quote! { DayToSecond(#digits) }
        }
        PgInterval::HourToSecond(digits) => {
            let digits = precision(digits);
            quote! { HourToSecond(#digits) }
        }
        PgInterval::MinuteToSecond(digits) => {
            let digits = precision(digits);
            quote! { MinuteToSecond(#digits) }
        }
    };
    quote! { IntervalSpec::Fields(PgInterval::#fields) }
}

/// The `RangeType` naming a built-in range, as generated source spells it.
fn range_type_tokens(range: RangeType) -> TokenStream {
    let variant = format_ident!("{}", checked_range_spelling(range).0);
    quote! { RangeType::#variant }
}

/// The set of `ColumnType`s `get_rs_type` and `write_col_type` can render, checked
/// through `Array` element types, over the enum names they will emit, and over
/// the named types they will respell.
// [spec:pgorm:req:codegen.entity.types.unsupported+9]
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
        | ColumnType::Interval(_) => Ok(()),
        ColumnType::Enum { name, .. } => {
            safe_ident(context, &name.to_string().to_upper_camel_case())?;
            Ok(())
        }
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
        // `Range<T>`, which only `RangeSubtype`'s subtypes can be; an array of
        // one has no conversion, as the newtype has no `NotU8`.
        ColumnType::CreatedRange { name, subtype, .. }
        | ColumnType::CreatedMultirange { name, subtype, .. } => {
            safe_ident(context, &name.to_string().to_upper_camel_case())?;
            if created_range_subtype(subtype).is_none() {
                return Err(Error::TransformError(format!(
                    "{context}: range type `{name}` over {subtype:?} is not supported by \
                     codegen; a created range's subtype is one pgorm's `RangeSubtype` covers",
                    name = name.to_string(),
                )));
            }
            Ok(())
        }
        ColumnType::Array(inner_col_type) => match under_domains(inner_col_type) {
            ColumnType::CreatedRange { name, .. } | ColumnType::CreatedMultirange { name, .. } => {
                Err(Error::TransformError(format!(
                    "{context}: an array of range type `{name}` is not supported by codegen; \
                     the newtype a created range generates has no array conversion",
                    name = name.to_string(),
                )))
            }
            ColumnType::Array(_)
                if matches!(inner_col_type.as_ref(), ColumnType::Domain { .. }) =>
            {
                Err(Error::TransformError(format!(
                    "{context}: an array of a domain over an array is not supported by codegen; \
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

// [spec:pgorm:sem:codegen.entity.transform+16]
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
        };
        column.validate()?;
        Ok(column)
    }
}

/// A column's `DEFAULT` and generation expression, read into the subset, and
/// refused where PostgreSQL refuses the pair (`42601`): a column takes one of
/// a default, an identity, a generation expression and the serial family.
// [spec:pgorm:sem:codegen.entity.expressions+1]
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
mod tests {
    use crate::Column;
    use pgorm_query::{ColumnDef, ColumnType, Name, StringLen};
    use proc_macro2::TokenStream;
    use quote::quote;

    fn to_column(col_def: ColumnDef) -> Column {
        Column::try_from(col_def).expect("column def should convert")
    }

    fn setup() -> Vec<Column> {
        macro_rules! make_col {
            ($name:expr, $col_type:expr) => {
                Column {
                    name: $name.to_owned(),
                    col_type: $col_type,
                    auto_increment: false,
                    identity: None,
                    not_null: false,
                    unique: false,
                    default: None,
                    generated: None,
                }
            };
        }
        vec![
            make_col!("id", ColumnType::String(StringLen::N(255))),
            make_col!("id", ColumnType::String(StringLen::None)),
            make_col!("cake_id", ColumnType::named("cus_col")),
            make_col!("CakeId", ColumnType::SmallInteger),
            make_col!("CakeId", ColumnType::Integer),
            make_col!("CakeFillingId", ColumnType::BigInteger),
            make_col!("cake-filling-id", ColumnType::Float),
            make_col!("CAKE_FILLING_ID", ColumnType::Double),
            make_col!("CAKE-FILLING-ID", ColumnType::Bytea),
            make_col!("CAKE", ColumnType::Boolean),
            make_col!("date", ColumnType::Date),
            make_col!("time", ColumnType::Time),
            make_col!("timestamp", ColumnType::Timestamp),
            make_col!("timestamp_tz", ColumnType::TimestampWithTimeZone),
        ]
    }

    #[test]
    fn test_get_name_snake_case() {
        let columns = setup();
        let snack_cases = vec![
            "id",
            "id",
            "cake_id",
            "cake_id",
            "cake_id",
            "cake_filling_id",
            "cake_filling_id",
            "cake_filling_id",
            "cake_filling_id",
            "cake",
            "date",
            "time",
            "timestamp",
            "timestamp_tz",
        ];
        for (col, snack_case) in columns.into_iter().zip(snack_cases) {
            assert_eq!(col.get_name_snake_case().to_string(), snack_case);
        }
    }

    #[test]
    fn test_get_name_camel_case() {
        let columns = setup();
        let camel_cases = vec![
            "Id",
            "Id",
            "CakeId",
            "CakeId",
            "CakeId",
            "CakeFillingId",
            "CakeFillingId",
            "CakeFillingId",
            "CakeFillingId",
            "Cake",
            "Date",
            "Time",
            "Timestamp",
            "TimestampTz",
        ];
        for (col, camel_case) in columns.into_iter().zip(camel_cases) {
            assert_eq!(col.get_name_camel_case().to_string(), camel_case);
        }
    }

    #[test]
    fn test_get_rs_type() {
        let columns = setup();
        let rs_types = vec![
            "String",
            "String",
            "String",
            "i16",
            "i32",
            "i64",
            "f32",
            "f64",
            "Vec<u8>",
            "bool",
            "Date",
            "Time",
            "DateTime",
            "DateTimeWithTimeZone",
        ];
        for (mut col, rs_type) in columns.into_iter().zip(rs_types) {
            let rs_type: TokenStream = rs_type.parse().unwrap();

            col.not_null = true;
            assert_eq!(col.get_rs_type().to_string(), quote!(#rs_type).to_string());

            col.not_null = false;
            assert_eq!(
                col.get_rs_type().to_string(),
                quote!(Option<#rs_type>).to_string()
            );
        }
    }

    #[test]
    fn test_get_info() {
        let column = to_column(ColumnDef::new(Name::runtime("id")).string().to_owned());
        assert_eq!(column.get_info().as_str(), "Column `id`: Option<String>");

        let column = to_column(
            ColumnDef::new(Name::runtime("id"))
                .string()
                .not_null()
                .to_owned(),
        );
        assert_eq!(column.get_info().as_str(), "Column `id`: String, not_null");

        // Uniqueness comes from the table's unique keys, which the transform
        // reads; a column converted alone is set unique the way it sets one.
        let mut column = to_column(
            ColumnDef::new(Name::runtime("id"))
                .string()
                .not_null()
                .to_owned(),
        );
        column.unique = true;
        assert_eq!(
            column.get_info().as_str(),
            "Column `id`: String, not_null, unique"
        );

        let mut column = to_column(
            ColumnDef::new(Name::runtime("id"))
                .string()
                .not_null()
                .auto_increment()
                .to_owned(),
        );
        column.unique = true;
        assert_eq!(
            column.get_info().as_str(),
            "Column `id`: String, auto_increment, not_null, unique"
        );

        let column = to_column(
            ColumnDef::new(Name::runtime("date_field"))
                .date()
                .not_null()
                .to_owned(),
        );
        assert_eq!(
            column.get_info().as_str(),
            "Column `date_field`: Date, not_null"
        );

        let column = to_column(
            ColumnDef::new(Name::runtime("time_field"))
                .time()
                .not_null()
                .to_owned(),
        );
        assert_eq!(
            column.get_info().as_str(),
            "Column `time_field`: Time, not_null"
        );

        let column = to_column(
            ColumnDef::new(Name::runtime("timestamp_field"))
                .timestamp()
                .not_null()
                .to_owned(),
        );
        assert_eq!(
            column.get_info().as_str(),
            "Column `timestamp_field`: DateTime, not_null"
        );

        let column = to_column(
            ColumnDef::new(Name::runtime("timestamp_with_timezone_field"))
                .timestamp_with_time_zone()
                .not_null()
                .to_owned(),
        );
        assert_eq!(
            column.get_info().as_str(),
            "Column `timestamp_with_timezone_field`: DateTimeWithTimeZone, not_null"
        );
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
