#![allow(non_camel_case_types)]

use pgorm::{
    ActiveEnum,
    entity::prelude::{ColumnType, StringLen},
};

#[derive(Debug, ActiveEnum, Eq, PartialEq)]
#[pgorm(enum_name = "test_enum", rename_all = "camelCase")]
enum TestEnum {
    DefaultVariant,
    #[pgorm(rename = "camelCase")]
    VariantCamelCase,
    #[pgorm(rename = "kebab-case")]
    VariantKebabCase,
    #[pgorm(rename = "mixed_case")]
    VariantMixedCase,
    #[pgorm(rename = "SCREAMING_SNAKE_CASE")]
    VariantShoutySnakeCase,
    #[pgorm(rename = "snake_case")]
    VariantSnakeCase,
    #[pgorm(rename = "title_case")]
    VariantTitleCase,
    #[pgorm(rename = "UPPERCASE")]
    VariantUpperCase,
    #[pgorm(rename = "lowercase")]
    VariantLowerCase,
    #[pgorm(rename = "SCREAMING-KEBAB-CASE")]
    VariantScreamingKebabCase,
    #[pgorm(rename = "PascalCase")]
    VariantPascalCase,
    #[pgorm(string_value = "CuStOmStRiNgVaLuE")]
    CustomStringValue,
}

#[derive(Debug, ActiveEnum, Eq, PartialEq)]
#[pgorm(
    rs_type = String,
    db_type = ColumnType::String(StringLen::None),
    rename_all = "snake_case"
)]
pub enum TestEnum2 {
    HelloWorld,
    #[pgorm(rename = "camelCase")]
    HelloWorldTwo,
}

#[derive(Debug, ActiveEnum, Eq, PartialEq)]
#[pgorm(
    rs_type = String,
    db_type = ColumnType::String(StringLen::None),
    rename_all = "snake_case"
)]
pub enum TestEnum3 {
    HelloWorld,
}

#[test]
fn derive_active_enum_value() {
    assert_eq!(TestEnum::DefaultVariant.to_value(), "defaultVariant");
    assert_eq!(TestEnum::VariantCamelCase.to_value(), "variantCamelCase");
    assert_eq!(TestEnum::VariantKebabCase.to_value(), "variant-kebab-case");
    assert_eq!(TestEnum::VariantMixedCase.to_value(), "variantMixedCase");
    assert_eq!(
        TestEnum::VariantShoutySnakeCase.to_value(),
        "VARIANT_SHOUTY_SNAKE_CASE"
    );
    assert_eq!(TestEnum::VariantSnakeCase.to_value(), "variant_snake_case");
    assert_eq!(TestEnum::VariantTitleCase.to_value(), "Variant Title Case");
    assert_eq!(TestEnum::VariantUpperCase.to_value(), "VARIANTUPPERCASE");
    assert_eq!(TestEnum::VariantLowerCase.to_value(), "variantlowercase");
    assert_eq!(
        TestEnum::VariantScreamingKebabCase.to_value(),
        "VARIANT-SCREAMING-KEBAB-CASE"
    );
    assert_eq!(TestEnum::VariantPascalCase.to_value(), "VariantPascalCase");
    assert_eq!(TestEnum::CustomStringValue.to_value(), "CuStOmStRiNgVaLuE");
}

#[test]
fn derive_active_enum_from_value() {
    assert_eq!(
        TestEnum::try_from_value(&"defaultVariant".to_string()),
        Ok(TestEnum::DefaultVariant)
    );
    assert_eq!(
        TestEnum::try_from_value(&"variantCamelCase".to_string()),
        Ok(TestEnum::VariantCamelCase)
    );
    assert_eq!(
        TestEnum::try_from_value(&"variant-kebab-case".to_string()),
        Ok(TestEnum::VariantKebabCase)
    );
    assert_eq!(
        TestEnum::try_from_value(&"variantMixedCase".to_string()),
        Ok(TestEnum::VariantMixedCase)
    );
    assert_eq!(
        TestEnum::try_from_value(&"VARIANT_SHOUTY_SNAKE_CASE".to_string()),
        Ok(TestEnum::VariantShoutySnakeCase),
    );
    assert_eq!(
        TestEnum::try_from_value(&"variant_snake_case".to_string()),
        Ok(TestEnum::VariantSnakeCase)
    );
    assert_eq!(
        TestEnum::try_from_value(&"Variant Title Case".to_string()),
        Ok(TestEnum::VariantTitleCase)
    );
    assert_eq!(
        TestEnum::try_from_value(&"VARIANTUPPERCASE".to_string()),
        Ok(TestEnum::VariantUpperCase)
    );
    assert_eq!(
        TestEnum::try_from_value(&"variantlowercase".to_string()),
        Ok(TestEnum::VariantLowerCase)
    );
    assert_eq!(
        TestEnum::try_from_value(&"VARIANT-SCREAMING-KEBAB-CASE".to_string()),
        Ok(TestEnum::VariantScreamingKebabCase),
    );
    assert_eq!(
        TestEnum::try_from_value(&"VariantPascalCase".to_string()),
        Ok(TestEnum::VariantPascalCase)
    );
    assert_eq!(
        TestEnum::try_from_value(&"CuStOmStRiNgVaLuE".to_string()),
        Ok(TestEnum::CustomStringValue)
    );
}

#[test]
fn derive_active_enum_value_2() {
    assert_eq!(TestEnum2::HelloWorld.to_value(), "hello_world");
    assert_eq!(TestEnum2::HelloWorldTwo.to_value(), "helloWorldTwo");

    assert_eq!(TestEnum3::HelloWorld.to_value(), "hello_world");
}

/// `enum_name` is optional: it defaults to the UpperCamelCase of the enum ident.
#[derive(Debug, ActiveEnum, Eq, PartialEq)]
#[pgorm(rs_type = String, db_type = ColumnType::String(StringLen::None))]
pub enum tea_kind {
    #[pgorm(string_value = "E")]
    Everyday,
}

/// `num_value` selects the integer flavour for the whole enum.
#[derive(Debug, ActiveEnum, Eq, PartialEq)]
#[pgorm(rs_type = i32, db_type = ColumnType::Integer)]
pub enum Numbered {
    #[pgorm(num_value = 1)]
    One,
    #[pgorm(num_value = 22)]
    TwentyTwo,
}

/// A variant with no attribute falls back to its integer discriminant,
/// including negative literals written with a unary minus.
#[derive(Debug, ActiveEnum, Eq, PartialEq)]
#[pgorm(rs_type = i32, db_type = ColumnType::Integer)]
pub enum Discriminants {
    Below = -3,
    Zero = 0,
    Above = 9,
}

/// `display` implements `Display`, writing a variant's `display_value` or else
/// its name.
#[derive(Debug, ActiveEnum, Eq, PartialEq)]
#[pgorm(rs_type = String, db_type = ColumnType::String(StringLen::None), display)]
pub enum Displayed {
    #[pgorm(string_value = "S", display_value = "Small")]
    Small,
    #[pgorm(string_value = "L")]
    Large,
}

// [spec:pgorm:syn:macros.derive.active-enum+6/test]    rename_all + per-variant rename + string_value
#[test]
fn container_and_variant_string_markers() {
    // Covered exhaustively by `derive_active_enum_value` above; this asserts the
    // container `enum_name` and its default, the enum's name in snake_case.
    assert_eq!(
        pgorm::SqlName::to_string(&*<TestEnum as ActiveEnum>::name()),
        "test_enum"
    );
    assert_eq!(
        pgorm::SqlName::to_string(&*<tea_kind as ActiveEnum>::name()),
        "tea_kind"
    );
    assert_eq!(
        pgorm::SqlName::to_string(&*<TestEnum2 as ActiveEnum>::name()),
        "test_enum2"
    );
}

// [spec:pgorm:syn:macros.derive.active-enum+6/test]    an omitted `db_type` is the PostgreSQL enum
// [spec:pgorm:req:macros.derive.code-values+2/test]    `db_type` is an expression taken as written
#[test]
fn omitted_db_type_is_the_postgres_enum() {
    let col = <TestEnum as ActiveEnum>::db_type();
    assert_eq!(
        col.get_column_type(),
        &ColumnType::Enum {
            schema: None,
            name: <TestEnum as ActiveEnum>::name(),
            variants: TestEnum::name_values(),
        }
    );
    // A variant named by `rename_all` alone is one of the type's values.
    assert!(
        TestEnum::name_values()
            .iter()
            .any(|value| pgorm::SqlName::to_string(&**value) == "defaultVariant")
    );

    // A `db_type` is the `ColumnType` expression as written.
    assert_eq!(
        <TestEnum2 as ActiveEnum>::db_type().get_column_type(),
        &ColumnType::String(StringLen::None)
    );
    assert_eq!(
        <Numbered as ActiveEnum>::db_type().get_column_type(),
        &ColumnType::Integer
    );
}

// [spec:pgorm:syn:macros.derive.active-enum+6/test]    num_value
#[test]
fn num_value_variants() {
    assert_eq!(Numbered::One.to_value(), 1);
    assert_eq!(Numbered::TwentyTwo.to_value(), 22);
    assert_eq!(Numbered::try_from_value(&22), Ok(Numbered::TwentyTwo));
    assert!(Numbered::try_from_value(&23).is_err());
}

// [spec:pgorm:syn:macros.derive.active-enum+6/test]    discriminant fallback, incl. unary minus
#[test]
fn variants_without_attributes_fall_back_to_their_discriminant() {
    assert_eq!(Discriminants::Below.to_value(), -3);
    assert_eq!(Discriminants::Zero.to_value(), 0);
    assert_eq!(Discriminants::Above.to_value(), 9);
    assert_eq!(Discriminants::try_from_value(&-3), Ok(Discriminants::Below));
}

// [spec:pgorm:syn:macros.derive.active-enum+6/test]    `display` and `display_value`
#[test]
fn display_writes_the_display_value_or_name() {
    // It plays no part in the stored value...
    assert_eq!(Displayed::Small.to_value(), "S");
    assert_eq!(Displayed::Large.to_value(), "L");
    // ...it is what `display` writes, the variant's name where it is absent.
    assert_eq!(Displayed::Small.to_string(), "Small");
    assert_eq!(Displayed::Large.to_string(), "Large");
}

/// The longest values a PostgreSQL enum holds, 63 bytes, one of them spelt
/// in 62 characters.
#[derive(Debug, ActiveEnum, Eq, PartialEq)]
pub enum Longest {
    #[pgorm(string_value = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")]
    Ascii,
    #[pgorm(string_value = "éaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")]
    Accented,
}

/// A value stored as text, which no label limit bounds.
#[derive(Debug, ActiveEnum, Eq, PartialEq)]
#[pgorm(rs_type = String, db_type = ColumnType::Text)]
pub enum Texted {
    #[pgorm(string_value = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")]
    Long,
}

// [spec:pgorm:syn:macros.derive.active-enum+6/test]    a PostgreSQL enum's value may be 63 bytes, however many characters spell it; a value stored as text is not bounded
#[test]
fn values_up_to_the_label_limit_are_kept() {
    assert_eq!(Longest::Ascii.to_value().len(), 63);
    assert_eq!(Longest::Accented.to_value().len(), 63);
    assert_eq!(Longest::Accented.to_value().chars().count(), 62);
    assert_eq!(Texted::Long.to_value().len(), 64);
}

/// Variants a `#[cfg]` keeps or removes, and one whose value `cfg_attr`
/// gives.
#[derive(Debug, ActiveEnum, Eq, PartialEq)]
pub enum Configured {
    Kept,
    #[cfg(not(test))]
    Removed,
    #[cfg(test)]
    Included,
    #[cfg_attr(test, pgorm(string_value = "given"))]
    Attributed,
}

// [spec:pgorm:syn:macros.derive.active-enum+6/test]    a variant `#[cfg]` removes is no value of the type, and a `cfg_attr` key is read as written
#[test]
fn configured_variants_are_the_values() {
    use pgorm::entity::prelude::Iterable;
    assert_eq!(
        Configured::iter().collect::<Vec<_>>(),
        [
            Configured::Kept,
            Configured::Included,
            Configured::Attributed
        ]
    );
    assert_eq!(Configured::values(), ["kept", "included", "given"]);
    assert_eq!(
        Configured::name_values()
            .iter()
            .map(|value| pgorm::SqlName::to_string(&**value))
            .collect::<Vec<_>>(),
        ["kept", "included", "given"]
    );
    assert!(Configured::try_from_value(&"removed".to_owned()).is_err());
}
