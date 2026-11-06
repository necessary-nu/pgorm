use super::ident::safe_ident;
use crate::{
    Column, CreatedRangeType, EnumHome, Error,
    entity::column::{checked_range_spelling, enum_rust_name, rs_type_text},
    util::escape_rust_keyword,
};
use heck::{ToSnakeCase, ToUpperCamelCase};
use pgorm_query::{
    ColumnType, GeneratedKind, IdentityGeneration, IntervalPrecision, IntervalSpec, Name,
    PgInterval, RangeType, StringLen,
};
use proc_macro2::{Ident, Literal, TokenStream};
use quote::{format_ident, quote};
use std::fmt::Write as FmtWrite;

impl Column {
    /// Every Rust identifier the writer derives from the column's DB names:
    /// the enum or created range its type names, its field and its `Column`
    /// variant.
    // [spec:pgorm:req:codegen.entity.types.unsupported+12]
    // [spec:pgorm:sem:codegen.entity.keywords+2]
    pub(crate) fn validate_idents(&self) -> Result<(), Error> {
        let context = format!("column `{}`", self.name);
        validate_col_type_idents(&context, &self.col_type)?;
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

    // [spec:pgorm:sem:codegen.entity.types+13]
    // [spec:pgorm:sem:codegen.entity.types.datetime+2]
    // [spec:pgorm:req:codegen.entity.types.unsupported+12]
    pub fn get_rs_type(&self) -> TokenStream {
        let ident: TokenStream = rs_type_text(&self.col_type, self.enum_home.as_ref())
            .parse()
            .expect("mapped Rust type names are token text");
        match self.not_null {
            true => quote! { #ident },
            false => quote! { Option<#ident> },
        }
    }

    /// The `column_type = ..` part of the compact `#[pgorm(..)]` attribute,
    /// the expression the derive builds the column's type from.
    // [spec:pgorm:sem:codegen.entity.compact.attrs+14]
    pub fn get_col_type_attrs(&self) -> Option<TokenStream> {
        col_type_attr(&self.col_type, self.enum_home.as_ref())
            .map(|ty| quote! { column_type = #ty })
    }

    /// The Rust name of the enum the column's type names, if it names one,
    /// and the module declaring it: `pgorm_active_enums`, the base's module
    /// for a disjoint entity's kind type, or `None` for the column's own.
    // [spec:pgorm:sem:codegen.entity.imports+5]
    // [spec:pgorm:sem:codegen.entity.disjoint+3]
    pub(crate) fn enum_import(&self) -> Option<(String, Option<String>)> {
        let ColumnType::Enum { name, .. } = self.get_inner_col_type() else {
            return None;
        };
        Some(match &self.enum_home {
            Some(home) => (home.name.clone(), home.module.clone()),
            None => (
                enum_rust_name(name, None),
                Some("pgorm_active_enums".to_owned()),
            ),
        })
    }

    /// The Rust name of the created range's newtype the column's type
    /// names, if it names one, which `pgorm_range_types` declares.
    // [spec:pgorm:sem:codegen.entity.imports+5]
    pub(crate) fn range_import(&self) -> Option<String> {
        match self.get_inner_col_type() {
            ColumnType::CreatedRange { name, .. } | ColumnType::CreatedMultirange { name, .. } => {
                Some(CreatedRangeType::rs_type(name).to_string())
            }
            _ => None,
        }
    }

    /// The field attribute carrying the column's `DEFAULT` or generation
    /// expression: `default_value = ..` for a literal default, written as the
    /// value it is, and otherwise the Rust that builds the expression written
    /// bare, `default_expr = ..`, `generated_stored = ..` or
    /// `generated_virtual = ..`.
    // [spec:pgorm:sem:codegen.entity.expressions+3]
    // [spec:pgorm:req:macros.derive.code-values+4]
    pub fn get_expression_attr(&self) -> Option<TokenStream> {
        if let Some(default) = &self.default {
            if let Some(value) = default.to_value() {
                return Some(quote! { default_value = #value });
            }
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
    // [spec:pgorm:sem:codegen.entity.compact.attrs+14]
    // [spec:pgorm:sem:codegen.entity.pk+5]
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
}

/// The enum and created range names `col_type` names, through `Array`
/// element types and domains, each of which `get_rs_type` and
/// `write_col_type` emit as a Rust identifier.
// [spec:pgorm:req:codegen.entity.types.unsupported+12]
fn validate_col_type_idents(context: &str, col_type: &ColumnType) -> Result<(), Error> {
    match col_type {
        ColumnType::Enum { name, .. }
        | ColumnType::CreatedRange { name, .. }
        | ColumnType::CreatedMultirange { name, .. } => {
            safe_ident(context, &name.to_string().to_upper_camel_case())?;
            Ok(())
        }
        ColumnType::Array(inner_col_type) => validate_col_type_idents(context, inner_col_type),
        ColumnType::Domain { base, .. } => validate_col_type_idents(context, base),
        _ => Ok(()),
    }
}

/// The Rust that builds a `ColumnType`. An enum reaches it only inside
/// another type — an array's element, a domain's base — and is its
/// `ActiveEnum`'s column type itself, `db_type().get_column_type().clone()`,
/// since the `Arc` holding it holds a `ColumnType` where `db_type()` returns
/// a `ColumnDef`.
// [spec:pgorm:req:codegen.entity.types.unsupported+12]
// [spec:pgorm:sem:codegen.entity.types+13]
fn write_col_type(col_type: &ColumnType, home: Option<&EnumHome>) -> TokenStream {
    let write_col_type = |col_type: &ColumnType| write_col_type(col_type, home);
    match col_type {
        ColumnType::Char(s) => match s {
            Some(s) => {
                let s = Literal::u32_unsuffixed(*s);
                quote! { ColumnType::Char(Some(#s)) }
            }
            None => quote! { ColumnType::Char(None) },
        },
        ColumnType::String(s) => match s {
            StringLen::N(s) => {
                let s = Literal::u32_unsuffixed(*s);
                quote! { ColumnType::String(StringLen::N(#s)) }
            }
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
            Some((s1, s2)) => {
                let (s1, s2) = (Literal::u32_unsuffixed(*s1), Literal::u32_unsuffixed(*s2));
                quote! { ColumnType::Decimal(Some((#s1, #s2))) }
            }
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
            let enum_ident = format_ident!("{}", enum_rust_name(name, home));
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
        ColumnType::Domain { name, schema, base } => domain_tokens(name, schema, base, home),
        #[allow(unreachable_patterns)]
        other => unreachable!(
            "column type {other:?} reached the writer; \
                 every Column is type-checked when it is built"
        ),
    }
}

/// A domain column's `ColumnType::Domain`, its name and schema as runtime
/// names and its base built as a `ColumnType`.
// [spec:pgorm:sem:codegen.entity.types+13]
fn domain_tokens(
    name: &Name,
    schema: &Option<Name>,
    base: &ColumnType,
    home: Option<&EnumHome>,
) -> TokenStream {
    let name = name.to_string();
    let schema = match schema {
        Some(schema) => {
            let schema = schema.to_string();
            quote! { Some(Name::runtime(#schema)) }
        }
        None => quote! { None },
    };
    let base = write_col_type(base, home);
    quote! {
        ColumnType::Domain {
            name: Name::runtime(#name),
            schema: #schema,
            base: Arc::new(#base)
        }
    }
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
// [spec:pgorm:sem:codegen.entity.compact.attrs+14]
fn col_type_attr(col_type: &ColumnType, home: Option<&EnumHome>) -> Option<TokenStream> {
    let stated = matches!(
        col_type,
        ColumnType::Float | ColumnType::Double | ColumnType::Bytea
    ) || !infers_itself(col_type);
    stated.then(|| write_col_type(col_type, home))
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
// [spec:pgorm:sem:codegen.entity.compact.attrs+14]
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
// [spec:pgorm:sem:codegen.entity.types+13]
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

#[cfg(test)]
mod tests {
    use crate::Column;
    use crate::entity::column::tests::to_column;
    use pgorm_query::{ColumnDef, ColumnType, Name, StringLen};
    use proc_macro2::TokenStream;
    use quote::quote;

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
                    enum_home: None,
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
}
