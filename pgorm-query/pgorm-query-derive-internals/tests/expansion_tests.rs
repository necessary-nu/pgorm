//! The expansion read as tokens: what each derive writes for a given input and
//! root path, and what it refuses. The derives built on it are compiled and run
//! by pgorm-query-derive's trybuild fixtures and pgorm-macros' tests; these pin
//! the parts a fixture shows only indirectly, the path the traits are named by
//! above all.

use pgorm_query_derive_internals::{sql_name, static_name};
use proc_macro2::TokenStream;
use quote::quote;
use syn::DeriveInput;

fn input(source: &str) -> DeriveInput {
    syn::parse_str(source).expect("a derive input")
}

fn expand_sql_name(source: &str, root: TokenStream) -> String {
    sql_name(&input(source), &root)
        .expect("the input is accepted")
        .to_string()
}

fn expand_static_name(source: &str, root: TokenStream) -> String {
    static_name(&input(source), &root)
        .expect("the input is accepted")
        .to_string()
}

fn refusal(source: &str) -> String {
    sql_name(&input(source), &quote!(pgorm_query))
        .expect_err("the input is refused")
        .to_string()
}

// [spec:pgorm:sem:macros.derive.iden+3/test]    the traits are named through the root each crate passes
#[test]
fn the_traits_are_named_through_the_root() {
    let query = expand_static_name("enum Post { Table, Id }", quote!(pgorm_query));
    assert!(query.contains("impl pgorm_query :: SqlName for Post"));
    assert!(query.contains("impl pgorm_query :: StaticName for Post"));

    let pgorm = expand_static_name("enum Post { Table, Id }", quote!(pgorm::pgorm_query));
    assert!(pgorm.contains("impl pgorm :: pgorm_query :: SqlName for Post"));
    assert!(pgorm.contains("impl pgorm :: pgorm_query :: StaticName for Post"));
    assert!(
        !pgorm.contains(":: pgorm ::"),
        "the root is never `::`-prefixed"
    );
}

// [spec:pgorm:sem:macros.derive.iden.query+1/test]    the snake_case default, `Table`, and the renames
#[test]
fn names_follow_the_type_variant_and_renames() {
    let unit = expand_sql_name("struct GlyphToken;", quote!(pgorm_query));
    assert!(unit.contains("\"glyph_token\""));

    let renamed = expand_sql_name("#[iden = \"weRd\"] struct Word;", quote!(pgorm_query));
    assert!(renamed.contains("\"weRd\""));

    let variants = expand_static_name(
        "enum BookShelf { Table, FirstName, #[iden = \"turtle\"] Title, #[iden(rename = \"t\")] Text }",
        quote!(pgorm_query),
    );
    assert!(variants.contains("Self :: Table => \"book_shelf\""));
    assert!(variants.contains("Self :: FirstName => \"first_name\""));
    assert!(variants.contains("Self :: Title => \"turtle\""));
    assert!(variants.contains("Self :: Text => \"t\""));
}

// [spec:pgorm:sem:macros.derive.iden.query+1/test]    `method` and `flatten` read the name at run time
#[test]
fn method_and_flatten_name_variants_at_run_time() {
    let expanded = expand_sql_name(
        "enum Named { #[method = \"label\"] Custom, #[iden(flatten)] Inner(Other) }",
        quote!(pgorm_query),
    );
    assert!(expanded.contains("self . label ()"));
    assert!(expanded.contains("Self :: Inner (delegated) => delegated . unquoted (s)"));
    assert!(
        !expanded.contains("fn prepare"),
        "a name read at run time is not known to be a valid identifier"
    );
}

// [spec:pgorm:sem:macros.derive.iden.query+1/test]    `prepare` is written only for statically valid names
#[test]
fn prepare_is_written_only_for_valid_names() {
    let valid = expand_sql_name("enum Valid { Table, Id }", quote!(pgorm_query));
    assert!(valid.contains("fn prepare"));

    let invalid = expand_sql_name(
        "enum Invalid { Table, #[iden = \"we\\\"ird\"] Weird }",
        quote!(pgorm_query),
    );
    assert!(!invalid.contains("fn prepare"));

    let unit = expand_sql_name(
        "#[iden = \"has space\"] struct Spaced;",
        quote!(pgorm_query),
    );
    assert!(!unit.contains("fn prepare"));
}

// [spec:pgorm:sem:macros.derive.iden.query+1/test]    a name is a format argument, never the format string
#[test]
fn a_name_is_never_the_format_string() {
    let unit = expand_sql_name("#[iden = \"a{b}\"] struct Braced;", quote!(pgorm_query));
    assert!(unit.contains("write ! (s , \"{}\" , \"a{b}\")"));

    let variant = expand_sql_name("enum Braced { #[iden = \"{x}\"] X }", quote!(pgorm_query));
    assert!(variant.contains("write ! (s , \"{}\" , \"{x}\")"));
}

// [spec:pgorm:sem:macros.derive.iden.query+1/test]    an empty enum expands to nothing
#[test]
fn an_empty_enum_expands_to_nothing() {
    assert!(expand_sql_name("enum Empty {}", quote!(pgorm_query)).is_empty());
    assert!(expand_static_name("enum Empty {}", quote!(pgorm_query)).is_empty());
}

// [spec:pgorm:sem:macros.derive.iden.query+1/test]    other shapes and malformed attributes are errors, not panics
#[test]
fn malformed_input_is_an_error_not_a_panic() {
    assert_eq!(
        refusal("struct Fields { a: i32 }"),
        "you can only derive SqlName on enums or unit structs"
    );
    assert!(refusal("#[iden(flatten)] struct Listed;").contains("Only the attributes"));
    assert!(refusal("enum Wide { #[iden(flatten)] Two(A, B) }").contains("single field"));
    assert!(refusal("enum Pathed { #[iden(a::b)] X }").contains("flatten"));
    assert!(refusal("enum Odd { #[iden(nope)] X }").contains("nope is not a supported keyword"));
}
