//! Column type mapping, the unsupported-type policy, and how primary keys
//! surface in each format.

mod common;

use common::*;
use pgorm_codegen::{Column, EntityTransformer, Error};
use pgorm_query::{
    ColumnDef, ColumnType, IdentityGeneration, IntervalSpec, Name, PgInterval, SequenceOption,
    StringLen, TypeName,
};
use proc_macro2::{TokenStream, TokenTree};
use quote::quote;
use std::sync::Arc;

// [spec:pgorm:sem:codegen.entity.types+11/test]    `Column::get_rs_type` follows
// the mapping table, wrapping nullable columns in `Option`
#[test]
fn column_rust_types_follow_the_mapping_table() {
    let generated = generate(
        vec![keyed_with(
            "sample",
            &["id"],
            vec![
                serial("id"),
                typed("c_char", ColumnType::Char(Some(1))),
                typed("c_string", ColumnType::String(StringLen::N(10))),
                typed("c_text", ColumnType::Text),
                typed("c_named", ColumnType::named("citext")),
                typed("c_small", ColumnType::SmallInteger),
                typed("c_int", ColumnType::Integer),
                typed("c_big", ColumnType::BigInteger),
                typed("c_json", ColumnType::Json),
                typed("c_jsonb", ColumnType::JsonBinary),
                typed("c_decimal", ColumnType::Decimal(None)),
                typed("c_money", ColumnType::Money),
                typed("c_uuid", ColumnType::Uuid),
                typed("c_bytea", ColumnType::Bytea),
                typed("c_bool", ColumnType::Boolean),
                typed("c_interval", ColumnType::Interval(IntervalSpec::Any(None))),
                typed(
                    "c_months",
                    ColumnType::Interval(IntervalSpec::Fields(PgInterval::YearToMonth)),
                ),
                enum_col("c_enum", "tea_kind", &["black", "green"]),
                ColumnDef::new(Name::runtime("c_array"))
                    .array(ColumnType::Integer)
                    .not_null()
                    .to_owned(),
                ColumnDef::new(Name::runtime("c_nested_array"))
                    .array(ColumnType::Array(Arc::new(ColumnType::Integer)))
                    .not_null()
                    .to_owned(),
                // nullable: the same mapping, wrapped in `Option`
                typed_null("c_null_int", ColumnType::Integer),
                typed_null("c_null_uuid", ColumnType::Uuid),
                typed_null(
                    "c_null_intervals",
                    ColumnType::Array(Arc::new(ColumnType::Interval(IntervalSpec::Any(None)))),
                ),
            ],
        )],
        Opts::default(),
    );
    let sample = generated.file("sample.rs");

    for (field, rust_type) in [
        ("c_char", "String"),
        ("c_string", "String"),
        ("c_text", "String"),
        ("c_named", "String"),
        ("c_small", "i16"),
        ("c_int", "i32"),
        ("c_big", "i64"),
        ("c_json", "Json"),
        ("c_jsonb", "Json"),
        ("c_decimal", "Decimal"),
        ("c_money", "Decimal"),
        ("c_uuid", "Uuid"),
        ("c_bytea", "Vec<u8>"),
        ("c_bool", "bool"),
        ("c_interval", "Interval"),
        ("c_months", "Interval"),
        ("c_enum", "TeaKind"),
        ("c_array", "Vec<i32>"),
        ("c_nested_array", "Vec<Vec<i32>>"),
        ("c_null_int", "Option<i32>"),
        ("c_null_uuid", "Option<Uuid>"),
        ("c_null_intervals", "Option<Vec<Interval> >"),
    ] {
        assert_contains(sample, &format!("pub {field}: {rust_type}"));
    }
}

// [spec:pgorm:sem:codegen.entity.types+11/test]    `Float` and `Double` also map
// to `f32` / `f64`, and either one suppresses the Model's `Eq` derive —
// recursively through `Array`
#[test]
fn float_and_double_columns_suppress_the_eq_derive() {
    let no_floats = generate(
        vec![keyed_with("sample", &["id"], vec![serial("id")])],
        Opts::default(),
    );
    assert_contains(
        no_floats.file("sample.rs"),
        "#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]",
    );

    let with_float = generate(
        vec![keyed_with(
            "sample",
            &["id"],
            vec![serial("id"), typed("ratio", ColumnType::Float)],
        )],
        Opts::default(),
    );
    assert_contains(with_float.file("sample.rs"), "pub ratio: f32,");
    assert_contains(
        with_float.file("sample.rs"),
        "#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]",
    );

    let with_double = generate(
        vec![keyed_with(
            "sample",
            &["id"],
            vec![serial("id"), typed("ratio", ColumnType::Double)],
        )],
        Opts::default(),
    );
    assert_contains(with_double.file("sample.rs"), "pub ratio: f64,");
    assert_contains(
        with_double.file("sample.rs"),
        "#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]",
    );

    // through an array element type
    let with_float_array = generate(
        vec![keyed_with(
            "sample",
            &["id"],
            vec![
                serial("id"),
                ColumnDef::new(Name::runtime("ratios"))
                    .array(ColumnType::Float)
                    .not_null()
                    .to_owned(),
            ],
        )],
        Opts::default(),
    );
    assert_contains(
        with_float_array.file("sample.rs"),
        "#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]",
    );
}

// [spec:pgorm:sem:codegen.entity.types.datetime+2/test]    date/time columns map
// to the prelude's temporal aliases, and to nothing else — there is no option
// selecting a different set, so the mapping is the whole surface
#[test]
fn date_time_columns_map_to_prelude_aliases() {
    let generated = generate(
        vec![keyed_with(
            "moment",
            &["id"],
            vec![
                serial("id"),
                typed("d", ColumnType::Date),
                typed("t", ColumnType::Time),
                typed("ts", ColumnType::Timestamp),
                typed("tstz", ColumnType::TimestampWithTimeZone),
                typed("many", ColumnType::Array(Arc::new(ColumnType::Timestamp))),
            ],
        )],
        Opts::default(),
    );
    let moment = generated.file("moment.rs");
    for (field, rust_type) in [
        ("d", "Date"),
        ("t", "Time"),
        ("ts", "DateTime"),
        ("tstz", "DateTimeWithTimeZone"),
        ("many", "Vec<DateTime>"),
    ] {
        assert_contains(moment, &format!("pub {field}: {rust_type},"));
    }
    // No crate in the workspace defines a `TimeDate` family, so generated source
    // naming one cannot compile against pgorm. Nothing may reach these spellings.
    for absent in [
        "TimeDate",
        "TimeTime",
        "TimeDateTime",
        "TimeDateTimeWithTimeZone",
    ] {
        assert!(
            !moment.contains(absent),
            "generated source names `{absent}`, which no crate defines:\n{moment}"
        );
    }
}

/// A type name Postgres accepts and Rust source does not: it holds a double
/// quote and a space, so pasting it between two quote characters ends the
/// attribute's literal in the middle of the name.
const HOSTILE_NAME: &str = "ev\"il type";

/// The compact attribute codegen emits for [`HOSTILE_NAME`], exactly as it
/// appears in the generated file. `hostile_entity` below carries this same
/// text through the real derive, so the two halves of the round trip are
/// pinned to one spelling.
const HOSTILE_ATTR: &str = r#"#[pgorm(column_type = ColumnType::named("ev\"il type"))]"#;

/// A generated entity written out by hand, carrying [`HOSTILE_ATTR`] verbatim.
/// That it compiles at all is half the claim; `Column::Odd`'s definition is the
/// other half.
mod hostile_entity {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[pgorm(table_name = "sample")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(column_type = ColumnType::named("ev\"il type"))]
        pub odd: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// The value of the sole string literal in an attribute — the name the
/// `ColumnType::named(..)` expression `column_type` takes is called with.
fn attribute_string_value(attr: &str) -> String {
    let mut literal = None;
    fn walk(stream: TokenStream, found: &mut Option<String>) {
        for tree in stream {
            match tree {
                TokenTree::Group(group) => walk(group.stream(), found),
                TokenTree::Literal(lit) => *found = Some(lit.to_string()),
                _ => {}
            }
        }
    }
    walk(
        attr.parse().expect("the attribute should lex as Rust"),
        &mut literal,
    );
    let literal = literal.expect("the attribute should carry a literal");
    syn::parse_str::<syn::LitStr>(&literal)
        .expect("the literal should be a string")
        .value()
}

// [spec:pgorm:sem:codegen.entity.compact.attrs+10/test]    the compact attribute
// carries a named type's name as a rendered string literal, so a name holding a
// quote arrives at the derive as the name that was described rather than as
// tokens that escaped the literal
#[test]
fn hostile_named_type_survives_the_compact_attribute() {
    let generated = generate(
        vec![keyed_with(
            "sample",
            &["id"],
            vec![serial("id"), typed("odd", ColumnType::named(HOSTILE_NAME))],
        )],
        Opts::default(),
    );

    assert_contains(
        generated.file("sample.rs"),
        &format!("{HOSTILE_ATTR} pub odd: String,"),
    );

    // The derive takes the expression as written, so the name it is called
    // with is the literal's value: the described name, whole.
    assert_eq!(attribute_string_value(HOSTILE_ATTR), HOSTILE_NAME);

    // The control the rest of this test rests on: pasting the same name
    // between two quote characters, as `format!` would, does not produce the
    // same program. It fails in one of two ways, and both are here because
    // only one of them is loud.
    //
    // This name ends the literal at its own quote and leaves a trailing `")`
    // that never closes, so the derive cannot parse the attribute at all — a
    // compile error inside generated source, pointing at an attribute nobody
    // wrote by hand.
    assert!(
        syn::parse_str::<syn::Expr>(&format!("ColumnType::named(\"{HOSTILE_NAME}\")")).is_err(),
        "the pasted spelling of {HOSTILE_NAME:?} should not parse",
    );
    // A name carrying a comment opener parses perfectly well, and names a
    // different type than the one described — the tail of the name is
    // commented out, and nothing anywhere reports it.
    let truncating = "x\") //";
    let pasted: syn::Expr = syn::parse_str(&format!("ColumnType::named(\"{truncating}\")"))
        .expect("the pasted spelling of a comment-bearing name parses");
    assert_eq!(
        quote!(#pasted).to_string(),
        quote!(ColumnType::named("x")).to_string(),
        "the pasted spelling should silently lose the rest of the name",
    );
    // Rendered as a literal, that same name survives whole.
    let rendered: syn::Expr = syn::parse_str(&format!(
        "ColumnType::named({})",
        proc_macro2::Literal::string(truncating)
    ))
    .expect("the rendered spelling parses");
    assert_eq!(
        quote!(#rendered).to_string(),
        quote!(ColumnType::named("x\") //")).to_string(),
    );
}

// [spec:pgorm:sem:codegen.entity.compact.attrs+10/test]    and the entity the
// derive builds from it names the type that was described, character for
// character
#[test]
fn derived_entity_names_the_hostile_type_exactly() {
    use pgorm::entity::prelude::ColumnTrait;

    assert_eq!(
        hostile_entity::Column::Odd.def().get_column_type(),
        &ColumnType::named(HOSTILE_NAME),
    );
}

// [spec:pgorm:req:codegen.entity.types.unsupported+9/test]    a named type the
// generated `ColumnType::named("..")` could not rebuild — schema-qualified, an
// array, or a type expression — is refused by name rather than respelled as a
// different type
#[test]
fn column_conversion_rejects_an_unrespellable_named_type() {
    for (col_type, spelled) in [
        (
            ColumnType::Named(TypeName::new(Name::runtime("status")).schema(Name::runtime("app"))),
            "app.status",
        ),
        (
            ColumnType::Named(TypeName::new(Name::runtime("citext")).array()),
            "citext[]",
        ),
        (
            ColumnType::Named(TypeName::raw("numeric(12, 2)")),
            "numeric(12, 2)",
        ),
    ] {
        let col_def = ColumnDef::new_with_type(Name::runtime("odd"), col_type).to_owned();
        match Column::try_from(&col_def) {
            Err(Error::TransformError(msg)) => assert_eq!(
                msg,
                format!(
                    "column `odd`: named column type `{spelled}` is not supported by codegen; \
                     only a bare type name survives the generated `ColumnType::named(\"..\")`"
                )
            ),
            other => panic!("expected a TransformError, got {other:?}"),
        }
    }
}

// [spec:pgorm:req:codegen.entity.types.unsupported+9/test]    a type outside the
// mapping fails the whole run with a `TransformError` naming table, column and
// type — no placeholder code, and no panic
#[test]
fn transform_rejects_column_type_outside_mapping() {
    let unsupported = EntityTransformer::transform(vec![keyed_with(
        "device",
        &["id"],
        vec![serial("id"), typed("address", ColumnType::Inet)],
    )]);

    match unsupported {
        Err(Error::TransformError(msg)) => assert_eq!(
            msg,
            "table `device` column `address`: column type Inet is not supported by codegen"
        ),
        other => panic!("expected a TransformError, got {other:?}"),
    }
}

// [spec:pgorm:req:codegen.entity.types.unsupported+9/test]    the check sits at
// `Column` construction, so the writer never meets an unmapped type; `Array`
// element types are checked through
#[test]
fn column_conversion_rejects_unsupported_type() {
    for col_type in [
        ColumnType::Inet,
        ColumnType::Array(Arc::new(ColumnType::Inet)),
    ] {
        let col_def = ColumnDef::new_with_type(Name::runtime("address"), col_type).to_owned();
        match Column::try_from(&col_def) {
            Err(Error::TransformError(msg)) => assert_eq!(
                msg,
                "column `address`: column type Inet is not supported by codegen"
            ),
            other => panic!("expected a TransformError, got {other:?}"),
        }
    }
}

// [spec:pgorm:sem:codegen.entity.transform+16/test]    an identity is read off
// the column definition as its form, and makes the column NOT NULL; one
// carrying sequence options is refused, since an entity declares no options
#[test]
fn column_conversion_reads_an_identity() {
    let plain = ColumnDef::new(Name::runtime("seq"))
        .integer()
        .identity_by_default()
        .to_owned();
    let column = Column::try_from(&plain).expect("an identity converts");
    assert_eq!(
        column.get_info(),
        "Column `seq`: i32, identity_by_default, not_null"
    );

    let with_options = ColumnDef::new(Name::runtime("seq"))
        .integer()
        .identity_with(IdentityGeneration::Always, SequenceOption::StartWith(10))
        .to_owned();
    match Column::try_from(&with_options) {
        Err(Error::TransformError(msg)) => assert_eq!(
            msg,
            "column `seq`: an entity declares an identity's form, not its sequence options"
        ),
        other => panic!("expected a TransformError, got {other:?}"),
    }
}

// [spec:pgorm:sem:codegen.entity.pk+4/test]    in the compact format the same facts
// surface as the `primary_key` / `auto_increment = false` field attributes
#[test]
fn compact_primary_key_facts_surface_as_field_attributes() {
    let auto = generate(vec![cake()], Opts::default());
    assert_contains(auto.file("cake.rs"), "#[pgorm(primary_key)] pub id: i32,");
    assert_not_contains(auto.file("cake.rs"), "auto_increment = false");

    let composite = generate(vec![cake(), cake_filling(), filling()], Opts::default());
    assert_contains(
        composite.file("cake_filling.rs"),
        "#[pgorm(primary_key, auto_increment = false)] pub cake_id: i32,
         #[pgorm(primary_key, auto_increment = false)] pub filling_id: i32,",
    );
}

// [spec:pgorm:sem:codegen.entity.types+11/test]    a range column is the `Range` of its
// subtype's Rust type, which the derive infers the range from
// [spec:pgorm:req:codegen.entity.types.unsupported+9/test]    an array of ranges is supported
#[test]
fn range_columns_are_typed_by_their_subtype() {
    use pgorm_query::RangeType;

    for (col_type, rust_type) in [
        (ColumnType::Range(RangeType::Int4), "Range < i32 >"),
        (ColumnType::Range(RangeType::Numeric), "Range < Decimal >"),
        (
            ColumnType::Multirange(RangeType::TimestampTz),
            "Multirange < DateTimeWithTimeZone >",
        ),
        (
            ColumnType::Array(std::sync::Arc::new(ColumnType::Range(RangeType::Int4))),
            "Vec < Range < i32 >>",
        ),
        (
            ColumnType::Array(std::sync::Arc::new(ColumnType::Multirange(RangeType::Date))),
            "Vec < Multirange < Date >>",
        ),
    ] {
        let col_def = ColumnDef::new_with_type(Name::runtime("span"), col_type)
            .not_null()
            .to_owned();
        let column = Column::try_from(&col_def).expect("a range column is supported");
        assert_eq!(column.get_rs_type().to_string(), rust_type);
        assert!(column.get_col_type_attrs().is_none(), "{rust_type}");
    }
}

// [spec:pgorm:sem:codegen.entity.compact.attrs+10/test]    an array needs a
// `column_type` exactly when its element does, since the derive infers an
// array as an array of what the element's Rust type infers: `text[]` would be
// created as `varchar[]`, and `citext[]` as `varchar[]` too
#[test]
fn arrays_name_the_element_type_they_cannot_infer() {
    let array = |name: &str, element: ColumnType| {
        ColumnDef::new(Name::runtime(name))
            .array(element)
            .not_null()
            .to_owned()
    };
    let generated = generate(
        vec![keyed_with(
            "sample",
            &["id"],
            vec![
                serial("id"),
                array("tags", ColumnType::Text),
                array("prices", ColumnType::Decimal(Some((10, 2)))),
                array("names", ColumnType::named("citext")),
                array("grid", ColumnType::Array(Arc::new(ColumnType::Text))),
                array("counts", ColumnType::Integer),
                array("labels", ColumnType::String(StringLen::None)),
                ColumnDef::new(Name::runtime("kinds"))
                    .array(ColumnType::Enum {
                        schema: None,
                        name: Name::runtime("tea_kind"),
                        variants: vec![Name::runtime("black")],
                    })
                    .to_owned(),
            ],
        )],
        Opts::default(),
    );
    let sample = generated.file("sample.rs");
    for field in [
        r#"#[pgorm(column_type = ColumnType::Array(Arc::new(ColumnType::Text)))] pub tags: Vec<String>,"#,
        r#"#[pgorm(column_type = ColumnType::Array(Arc::new(ColumnType::Decimal(Some((10u32, 2u32))))))]
           pub prices: Vec<Decimal>,"#,
        r#"#[pgorm(column_type = ColumnType::Array(Arc::new(ColumnType::named("citext"))))]
           pub names: Vec<String>,"#,
        // One run of fields with nothing between them: the arrays whose
        // element the derive infers carry no attribute.
        r#"#[pgorm(column_type = ColumnType::Array(Arc::new(ColumnType::Array(Arc::new(ColumnType::Text)))))]
           pub grid: Vec<Vec<String> >,
           pub counts: Vec<i32>,
           pub labels: Vec<String>,
           pub kinds: Option<Vec<TeaKind> >,"#,
    ] {
        assert_contains(sample, field);
    }
}

/// The attribute [`arrays_name_the_element_type_they_cannot_infer`] pins for
/// `text[]`, carried through the real derive.
mod text_array_entity {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
    #[pgorm(table_name = "sample")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(column_type = ColumnType::Array(Arc::new(ColumnType::Text)))]
        pub tags: Vec<String>,
        #[pgorm(
            column_type = ColumnType::Array(Arc::new(ColumnType::named("citext"))),
            nullable
        )]
        pub names: Option<Vec<String>>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

// [spec:pgorm:sem:codegen.entity.compact.attrs+10/test]    and the derive builds
// the array of the element type named, not of the one `String` infers
#[test]
fn derived_array_columns_keep_their_element_type() {
    use pgorm::entity::prelude::{ColumnTrait, ColumnTypeTrait};

    assert_eq!(
        text_array_entity::Column::Tags.def(),
        ColumnType::Array(Arc::new(ColumnType::Text)).def()
    );
    assert_eq!(
        text_array_entity::Column::Names.def(),
        ColumnType::Array(Arc::new(ColumnType::named("citext")))
            .def()
            .null()
    );
}
