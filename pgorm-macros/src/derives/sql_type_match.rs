use proc_macro2::{Ident, Span, TokenStream};
use quote::{quote, quote_spanned};
use syn::{Expr, GenericArgument, PathArguments, PathSegment, Type, TypePath};

/// The `T` of a field written as a bare, single-segment `Option<T>`.
///
/// A qualified spelling (`std::option::Option<T>`) is not an `Option` for the
/// purposes of the inference tables.
pub fn unwrap_option(ty: &Type) -> Option<&Type> {
    let segment = sole_segment(ty)?;
    if segment.ident != "Option" {
        return None;
    }
    sole_type_argument(&segment.arguments)
}

fn sole_segment(ty: &Type) -> Option<&PathSegment> {
    let Type::Path(TypePath {
        qself: None, path, ..
    }) = ty
    else {
        return None;
    };
    if path.leading_colon.is_some() || path.segments.len() != 1 {
        return None;
    }
    path.segments.first()
}

fn sole_type_argument(arguments: &PathArguments) -> Option<&Type> {
    let PathArguments::AngleBracketed(arguments) = arguments else {
        return None;
    };
    match arguments.args.first() {
        Some(GenericArgument::Type(ty)) if arguments.args.len() == 1 => Some(ty),
        _ => None,
    }
}

/// The name of a bare, single-segment path carrying no generic arguments:
/// `i32`, `String`, `Uuid`.
fn bare_name(ty: &Type) -> Option<&Ident> {
    let segment = sole_segment(ty)?;
    matches!(segment.arguments, PathArguments::None).then_some(&segment.ident)
}

/// `&str`, written without a lifetime and without `mut`.
fn is_shared_str(ty: &Type) -> bool {
    let Type::Reference(reference) = ty else {
        return false;
    };
    reference.lifetime.is_none()
        && reference.mutability.is_none()
        && bare_name(&reference.elem).is_some_and(|name| name == "str")
}

/// `Vec<u8>`.
fn is_byte_vec(ty: &Type) -> bool {
    let Some(segment) = sole_segment(ty) else {
        return false;
    };
    segment.ident == "Vec"
        && sole_type_argument(&segment.arguments)
            .and_then(bare_name)
            .is_some_and(|name| name == "u8")
}

// [spec:pgorm:sem:macros.derive.entity-model.column-def+10]
pub fn col_type_match(
    col_type: Option<Expr>,
    field_type: &Type,
    field_span: Span,
) -> syn::Result<TokenStream> {
    if let Some(col_type) = col_type {
        return Ok(quote! { #col_type });
    }
    if let Some(name) = bare_name(field_type)
        && let Some(reason) = refused_col_type(&name.to_string())
    {
        return Err(syn::Error::new(
            field_span,
            format!(
                "{reason}, so no column type is inferred for it; \
                 write `#[pgorm(column_type = ColumnType::...)]` to name one explicitly"
            ),
        ));
    }
    Ok(match inferred_col_type(field_type) {
        Some(col_type) => quote! { pgorm::prelude::ColumnType::#col_type },
        // Assumed to be an ActiveEnum if none of the above types matched.
        None => quote_spanned! { field_span =>
            std::convert::Into::<pgorm::pgorm_query::ColumnType>::into(
                <#field_type as pgorm::pgorm_query::ValueType>::column_type()
            )
        },
    })
}

/// Rust types whose `Value` binding names a column type their decode cannot read back.
/// Inferring one would generate DDL no `SELECT` could ever fill, so the inference
/// refuses instead — the same posture as `u8` and `u16`, which have no `ValueType` at
/// all and so fail on the fallback path.
// [spec:pgorm:sem:macros.derive.entity-model.column-def+10]
fn refused_col_type(name: &str) -> Option<&'static str> {
    Some(match name {
        "i8" => "`i8` decodes only from the Postgres `\"char\"` type, never from `smallint`",
        "u32" => "`u32` decodes only from `OID`, never from `int8`",
        "u64" => "`u64` cannot be decoded from a row at all",
        "char" => "`char` cannot be decoded from a row at all",
        _ => return None,
    })
}

fn inferred_col_type(field_type: &Type) -> Option<TokenStream> {
    if is_shared_str(field_type) {
        return Some(quote! { string(None) });
    }
    if is_byte_vec(field_type) {
        return Some(quote! { Bytea });
    }
    Some(match bare_name(field_type)?.to_string().as_str() {
        "String" => quote! { string(None) },
        "i16" => quote! { SmallInteger },
        "i32" => quote! { Integer },
        "i64" => quote! { BigInteger },
        "f32" => quote! { Float },
        "f64" => quote! { Double },
        "bool" => quote! { Boolean },
        "Date" => quote! { Date },
        "Time" => quote! { Time },
        "DateTime" => quote! { Timestamp },
        "DateTimeWithTimeZone" => quote! { TimestampWithTimeZone },
        "Uuid" => quote! { Uuid },
        "Json" => quote! { Json },
        "Decimal" => quote! { Decimal(None) },
        _ => return None,
    })
}

// [spec:pgorm:sem:macros.derive.entity-model.column-def+10]
pub fn arr_type_match(arr_type: Option<Expr>, field_type: &Type, field_span: Span) -> TokenStream {
    if let Some(arr_type) = arr_type {
        return quote! { #arr_type };
    }
    match inferred_arr_type(field_type) {
        Some(arr_type) => quote! { pgorm::pgorm_query::ArrayType::#arr_type },
        // Assumed to be an ActiveEnum if none of the above types matched.
        None => quote_spanned! { field_span =>
            std::convert::Into::<pgorm::pgorm_query::ArrayType>::into(
                <#field_type as pgorm::pgorm_query::ValueType>::array_type()
            )
        },
    }
}

fn inferred_arr_type(field_type: &Type) -> Option<TokenStream> {
    if is_shared_str(field_type) {
        return Some(quote! { String });
    }
    Some(match bare_name(field_type)?.to_string().as_str() {
        "String" => quote! { String },
        "i16" => quote! { SmallInt },
        "i32" => quote! { Int },
        "i64" => quote! { BigInt },
        "f32" => quote! { Float },
        "f64" => quote! { Double },
        "bool" => quote! { Bool },
        "Date" => quote! { Date },
        "Time" => quote! { Time },
        "DateTime" => quote! { DateTime },
        "DateTimeWithTimeZone" => quote! { DateTimeWithTimeZone },
        "Uuid" => quote! { Uuid },
        "Json" => quote! { Json },
        "Decimal" => quote! { Decimal },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_str;

    fn col(ty: &str) -> String {
        let ty = parse_str::<Type>(ty).expect("test type parses");
        col_type_match(None, &ty, Span::call_site())
            .expect("type is not refused")
            .to_string()
    }

    fn col_refusal(ty: &str) -> String {
        let ty = parse_str::<Type>(ty).expect("test type parses");
        col_type_match(None, &ty, Span::call_site())
            .expect_err("type is refused")
            .to_string()
    }

    fn arr(ty: &str) -> String {
        let ty = parse_str::<Type>(ty).expect("test type parses");
        arr_type_match(None, &ty, Span::call_site()).to_string()
    }

    // [spec:pgorm:sem:macros.derive.entity-model.column-def+10/test]    types whose decode
    // cannot read back what their `Value` binds are refused rather than inferred
    #[test]
    fn undecodable_integer_widths_are_refused() {
        assert!(col_refusal("i8").contains("`i8` decodes only from the Postgres"));
        assert!(col_refusal("u32").contains("`u32` decodes only from `OID`"));
        assert!(col_refusal("u64").contains("`u64` cannot be decoded from a row"));
        assert!(col_refusal("char").contains("`char` cannot be decoded from a row"));

        // An explicit `column_type` is the escape hatch and is never second-guessed.
        let ty = parse_str::<Type>("u32").expect("test type parses");
        let explicit = parse_str::<Expr>("ColumnType::BigInteger").expect("test expression parses");
        let pinned = col_type_match(Some(explicit), &ty, Span::call_site())
            .expect("an explicit column type is taken as written");
        assert!(pinned.to_string().contains("BigInteger"));

        // The neighbouring widths still infer.
        assert!(col("i16").contains("SmallInteger"));
        assert!(col("i64").contains("BigInteger"));
    }

    // [spec:pgorm:sem:macros.derive.entity-model.column-def+10/test]    only bare `&str` is in the table
    #[test]
    fn shared_str_matches_without_a_lifetime() {
        assert!(col("&str").contains("ColumnType :: string"));
        assert!(arr("&str").contains("ArrayType :: String"));

        assert!(col("&'static str").contains("ValueType"));
        assert!(col("&mut str").contains("ValueType"));
    }

    // [spec:pgorm:sem:macros.derive.entity-model.column-def+10/test]    the fallback keeps the written type
    #[test]
    fn fallback_reproduces_the_type_verbatim() {
        assert!(col("&'a str").contains("& 'a str"));
        assert!(col("Box<dyn Send>").contains("dyn Send"));
        assert!(col("<T as Trait>::Assoc").contains("< T as Trait > :: Assoc"));
    }

    // [spec:pgorm:sem:macros.derive.entity-model.column-def+10/test]    `Vec<u8>` is the only byte row
    #[test]
    fn byte_vec_matches_only_vec_of_u8() {
        assert!(col("Vec<u8>").contains("Bytea"));
        assert!(col("Vec<u16>").contains("ValueType"));
        assert!(col("Vec<u8, A>").contains("ValueType"));
        assert!(arr("Vec<u8>").contains("ValueType"));
    }

    // [spec:pgorm:sem:macros.derive.entity-model.column-def+10/test]    a wall clock and
    // an instant are different columns and different array members, so neither table
    // may merge them; each array tag agrees with its `ValueType::array_type()`
    #[test]
    fn naive_and_zoned_datetimes_stay_distinct() {
        assert!(col("DateTime").ends_with("Timestamp"));
        assert!(arr("DateTime").ends_with("DateTime"));

        assert!(col("DateTimeWithTimeZone").ends_with("TimestampWithTimeZone"));
        assert!(arr("DateTimeWithTimeZone").ends_with("DateTimeWithTimeZone"));
    }

    // [spec:pgorm:sem:macros.derive.entity-model.column-def+10/test]    the tables hold only
    // bare names, so a qualified spelling or a user alias resolves through `ValueType`
    // rather than failing — the tables are a fast path, not the mechanism
    #[test]
    fn qualified_temporal_paths_reach_the_fallback() {
        for ty in ["civil::Date", "jiff::civil::DateTime", "jiff::Timestamp"] {
            assert!(col(ty).contains("ValueType"));
            assert!(arr(ty).contains("ValueType"));
        }

        // A bare alias is equally outside the tables and takes the same route.
        assert!(col("Stamp").contains("ValueType"));
        assert!(arr("Stamp").contains("ValueType"));
    }

    // [spec:pgorm:sem:macros.derive.entity-model.column-def+10/test]    `Option<T>` unwrapping is structural
    #[test]
    fn option_unwraps_only_when_bare() {
        let bare = parse_str::<Type>("Option<i64>").expect("test type parses");
        assert_eq!(
            unwrap_option(&bare).map(|ty| quote! { #ty }.to_string()),
            Some("i64".to_owned())
        );

        for ty in [
            "std::option::Option<i64>",
            "i64",
            "Option",
            "OptionLike<i64>",
        ] {
            let ty = parse_str::<Type>(ty).expect("test type parses");
            assert!(unwrap_option(&ty).is_none());
        }
    }
}
