//! Verification for the `Entity` derive and the pieces it composes: the `Column`
//! enum and its `ColumnTrait` impl, the `Entity` unit struct and its `EntityName`
//! impl, and the `PrimaryKey` enum and its `PrimaryKeyTrait` impl.

#![allow(dead_code, non_snake_case)]

use pgorm::entity::prelude::*;
use pgorm::pgorm_query::{ColumnType, Expr, StringLen};
use pgorm::{Iterable, PrimaryKeyToColumn};

/// The kitchen-sink entity: every struct-level and field-level attribute the
/// entity-model derive recognises appears here at least once.
mod filling {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, Entity)]
    #[pgorm(
        table_name = "filling",
        schema_name = "baking",
        comment = "cake fillings",
        table_iden
    )]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(column_type = ColumnType::Text, unique, indexed, comment = "display name")]
        pub name: String,
        pub vendor_id: Option<i32>,
        #[pgorm(nullable, default_value = 0)]
        pub weight: i32,
        #[pgorm(default_expr = "Expr::current_date()")]
        pub baked_on: Date,
        #[pgorm(column_name = "SKU")]
        pub sku: String,
        #[pgorm(enum_name = "Renamed")]
        pub original: i32,
        #[pgorm(select_as = "text", save_as = "citext")]
        pub casted: String,
        #[pgorm(ignore)]
        pub not_a_column: i32,
        pub plain: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// What a hand-written `ColumnTrait` method could say, said by attributes: a
/// `column_type` that is any `ColumnType` expression, a helper's call
/// included, and a column read and written through functions of its
/// expression.
mod written_by_functions {
    use pgorm::entity::prelude::*;
    use pgorm::pgorm_query::{SimpleExpr, TypeName};

    fn varchar(length: u32) -> ColumnType {
        ColumnType::String(StringLen::N(length))
    }

    fn label() -> TypeName {
        TypeName::new(Name::runtime("Label"))
    }

    pub fn read_label(expr: Expr) -> SimpleExpr {
        expr.cast_as_type(label())
    }

    pub fn write_label(val: Expr) -> SimpleExpr {
        val.cast_as_type(label())
    }

    pub fn write_labels(val: Expr) -> SimpleExpr {
        val.cast_as_type(label().array())
    }

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "written_by_functions")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(column_type = varchar(40))]
        pub code: String,
        #[pgorm(
            column_type = ColumnType::named("label"),
            select_with = read_label,
            save_with = write_label,
            save_array_with = write_labels
        )]
        pub label: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Every branch of the Rust-type -> `ColumnType` inference table. `char`, `i8`,
/// `u32` and `u64` are not in it: each is refused outright, because what its
/// `Value` binds is not what its decode can read back (see the `sql_type_match`
/// unit tests).
mod scalars {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "scalars")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub txt: String,
        pub small: i16,
        pub big: i64,
        pub single: f32,
        pub dbl: f64,
        pub flag: bool,
        pub d: Date,
        pub t: Time,
        pub dt: DateTime,
        pub dttz: DateTimeWithTimeZone,
        pub uid: Uuid,
        pub js: Json,
        pub dec: Decimal,
        pub blob: Vec<u8>,
        pub maybe: Option<i64>,
        pub tea: Tea,
    }

    #[derive(Clone, Debug, PartialEq, Eq, EnumIter, ActiveEnum)]
    #[pgorm(rs_type = "String", db_type = "String(StringLen::N(1))")]
    pub enum Tea {
        #[pgorm(string_value = "E")]
        EverydayTea,
        #[pgorm(string_value = "B")]
        BreakfastTea,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Casing: raw-identifier trimming, `enum_name` substitution, keyword escaping,
/// and the conditional pinning of the SQL column name.
mod casing {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "casing")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        // Raw identifier: the leading `r#` is trimmed before UpperCamelCase.
        pub r#type: i32,
        // Already-clean snake_case: no `column_name` attribute is pinned.
        pub first_name: i32,
        // Does not survive the UpperCamelCase -> snake_case round trip, so the
        // snake_case of the *original* field name is pinned instead.
        pub abc_1: i32,
        // Upper-cases to the special keyword `Self`, which is escaped by
        // appending an underscore.
        pub SELF: i32,
        // `enum_name` replaces the computed variant name entirely.
        #[pgorm(enum_name = "Wholly")]
        pub renamed_away: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// `rename_all` drives the SQL column names for every field that has no
/// explicit `column_name`.
mod renamed {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "renamed", rename_all = "SCREAMING_SNAKE_CASE")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub first_name: i32,
        #[pgorm(column_name = "explicit")]
        pub second_name: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// A single primary key with the default (true) auto-increment flag, plus a
/// `column_name` override on the key itself.
mod single_pk {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "single_pk")]
    pub struct Model {
        #[pgorm(primary_key, column_name = "ID")]
        pub id: i32,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// A composite primary key: the `ValueType` becomes a tuple and
/// `auto_increment()` is false because there is more than one key column.
mod composite_pk {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "composite_pk")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub cake_id: i32,
        #[pgorm(primary_key)]
        pub filling_id: i32,
        pub qty: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// `auto_increment` is a single shared flag: setting it false on a field that
/// is not even a primary key flips it for the whole entity.
mod shared_auto_increment {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "shared_auto_increment")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        #[pgorm(auto_increment = false)]
        pub not_a_key: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// A temporal key: the room and the period a rate holds over, the period last
/// and compared for overlap.
mod temporal_key {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "room_rate")]
    pub struct Model {
        #[pgorm(primary_key, auto_increment = false)]
        pub room_id: i32,
        #[pgorm(primary_key, without_overlaps)]
        pub valid_at: Range<Date>,
        pub rate: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// The multi-tenant key: the tenant is supplied, the row number generated. The
/// generated part is the column's identity, so the key as a whole is not
/// generated and `auto_increment()` stays false.
mod tenant_identity {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "tenant_identity")]
    pub struct Model {
        #[pgorm(primary_key)]
        pub tenant_id: i32,
        #[pgorm(primary_key, identity)]
        pub id: i64,
        #[pgorm(identity_by_default)]
        pub seq: i16,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Every column of the key is an identity, so the database generates the whole
/// key and `auto_increment()` is true for a composite key.
mod all_identity {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "all_identity")]
    pub struct Model {
        #[pgorm(primary_key, identity)]
        pub a: i32,
        #[pgorm(primary_key, identity_by_default)]
        pub b: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// A one-column identity key: generated whole, and not by the serial family.
mod single_identity {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "single_identity")]
    pub struct Model {
        #[pgorm(primary_key, identity)]
        pub id: i32,
        #[pgorm(auto_increment = false)]
        pub not_a_key: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Both kinds of generated column, a nullable one, and a stored one as the
/// whole key: the database computes the key, so `auto_increment()` is true.
mod generated_columns {
    use pgorm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Entity)]
    #[pgorm(table_name = "generated_columns")]
    pub struct Model {
        #[pgorm(primary_key, generated_stored = "Expr::col(Column::Base).mul(10)")]
        pub id: i32,
        pub base: i32,
        #[pgorm(generated_virtual = "Expr::col(Column::Base).add(1)")]
        pub next: i32,
        #[pgorm(generated_virtual = "Expr::col(Column::Base).mul(2)", nullable)]
        pub doubled: Option<i32>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// No `table_name`, so no `Entity` struct is generated: the entity's name is
/// computed, its `Entity` written by hand under `EntityTrait`, and
/// the `Entity` derive contributes only `Column`, `PrimaryKey`, `Model` and
/// `ActiveModel`.
mod no_table_name {
    use pgorm::entity::prelude::*;

    #[derive(Copy, Clone, Default, Debug, EntityTrait)]
    pub struct Entity;

    impl EntityName for Entity {
        fn table_name(&self) -> &str {
            "hand_written"
        }
    }

    #[derive(Clone, Debug, PartialEq, Entity)]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// The `serde` side of the naming, with enough unrelated `serde` parameters
/// around the two the derive reads to prove the attribute walk steps over the
/// rest instead of failing on it.
mod serde_keys {
    use pgorm::entity::prelude::*;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Eq, Entity, Serialize, Deserialize)]
    #[pgorm(table_name = "serde_keys")]
    #[serde(rename_all(deserialize = "camelCase"), deny_unknown_fields)]
    pub struct Model {
        #[pgorm(primary_key)]
        pub id: i32,
        pub baked_on: String,
        #[serde(rename(serialize = "ignored", deserialize = "NOTE"))]
        pub note: String,
        #[pgorm(column_name = "SKU")]
        #[serde(skip_serializing_if = "Option::is_none")]
        pub sku: Option<String>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, Relation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

// [spec:pgorm:sem:macros.derive.entity-model+7/test]
// [spec:pgorm:syn:macros.derive.entity-model.attrs+6/test]    struct-level table_name / schema_name / comment
#[test]
fn struct_attributes_drive_entity_and_entity_name() {
    // (3) `table_name` present, so `pub struct Entity;` plus a hand-rolled
    //     `EntityName` carrying table_name / schema_name / comment.
    assert_eq!(filling::Entity.table_name(), "filling");
    assert_eq!(filling::Entity.schema_name(), Some("baking"));
    assert_eq!(filling::Entity.comment(), Some("cake fillings"));
    // Entity is `Copy, Clone, Default, Debug` and derives `EntityTrait`.
    let entity = filling::Entity;
    let copied = entity;
    assert_eq!(format!("{:?}", copied), "Entity");
    assert_eq!(format!("{:?}", filling::Entity {}), "Entity");
}

// [spec:pgorm:sem:macros.derive.entity-model+7/test]    one derive yields the whole entity module
#[test]
fn one_derive_yields_the_whole_entity_module() {
    // (1) the `Column` enum, with `EnumIter` + the `Column` derive's behaviour.
    let columns: Vec<String> = filling::Column::iter().map(|c| c.to_string()).collect();
    assert_eq!(
        columns,
        vec![
            "id",
            "name",
            "vendor_id",
            "weight",
            "baked_on",
            "SKU",
            "renamed",
            "casted",
            "plain",
        ]
    );

    // (2) `impl ColumnTrait for Column` with `type EntityName = Entity`.
    fn assert_entity_name<C: ColumnTrait<EntityName = filling::Entity>>(_: C) {}
    assert_entity_name(filling::Column::Id);

    // (4) the `PrimaryKey` enum.
    let keys: Vec<String> = filling::PrimaryKey::iter().map(|k| k.to_string()).collect();
    assert_eq!(keys, vec!["id"]);

    // The `Model` derive is re-run on the same input and the ActiveModel written
    // beside it, so `Model: ModelTrait` and `ActiveModel` both exist.
    let model = filling::Model {
        id: 1,
        name: "raspberry".to_owned(),
        vendor_id: None,
        weight: 3,
        baked_on: Date::new(2024, 1, 1).unwrap(),
        sku: "RB-1".to_owned(),
        original: 7,
        casted: "x".to_owned(),
        not_a_column: 99,
        plain: 0,
    };
    assert_eq!(model.get(filling::Column::Id), 1i32.into());
    let active: filling::ActiveModel = model.into();
    assert_eq!(active.id, pgorm::ActiveValue::unchanged(1));
}

// [spec:pgorm:sem:macros.derive.entity-model+7/test]    select_as / save_as overrides and their fallback
#[test]
fn select_as_and_save_as_cast_columns() {
    let casted = filling::Column::Casted;
    // The attribute spells a type *expression*, not an identifier, so it rides
    // the verbatim escape hatch rather than the quoting `TypeName` path.
    assert_eq!(
        casted.select_as(Expr::col(casted)),
        Expr::col(casted).cast_as_raw("text")
    );
    assert_eq!(
        casted.save_as(Expr::val("v")),
        Expr::val("v").cast_as_raw("citext")
    );

    // Columns without the attributes fall through to the trait defaults.
    let plain = filling::Column::Plain;
    assert_eq!(
        plain.select_as(Expr::col(plain)),
        ColumnTrait::select_enum_as(&plain, Expr::col(plain))
    );
    assert_eq!(
        plain.save_as(Expr::val(1)),
        ColumnTrait::save_enum_as(&plain, Expr::val(1))
    );
}

// [spec:pgorm:req:macros.derive.one-form/test]    `column_type` is any `ColumnType`
// expression, a helper's call included, and `select_with` / `save_with` /
// `save_array_with` name the functions a column is read and written through
#[test]
fn attributes_say_what_column_methods_said() {
    use written_by_functions::{Column, read_label, write_label, write_labels};

    assert_eq!(
        Column::Code.def().get_column_type(),
        &ColumnType::String(StringLen::N(40))
    );
    assert_eq!(
        Column::Label.def().get_column_type(),
        &ColumnType::named("label")
    );

    let label = Column::Label;
    assert_eq!(
        label.select_as(Expr::col(label)),
        read_label(Expr::col(label))
    );
    assert_eq!(label.save_as(Expr::val("v")), write_label(Expr::val("v")));
    assert_eq!(
        label.save_array_as(Expr::val(vec!["v".to_owned()])),
        write_labels(Expr::val(vec!["v".to_owned()]))
    );

    // Columns naming none of the three keep the trait's casts.
    let code = Column::Code;
    assert_eq!(
        code.select_as(Expr::col(code)),
        ColumnTrait::select_enum_as(&code, Expr::col(code))
    );
    assert_eq!(
        code.save_array_as(Expr::val(vec!["v".to_owned()])),
        ColumnTrait::save_enum_array_as(&code, Expr::val(vec!["v".to_owned()]))
    );
}

// [spec:pgorm:sem:macros.derive.entity-model+7/test]    the `json_key` arm: the
// field's own name under `serde`'s renames, with the SQL naming attributes kept
// out of it and the rest of `serde` stepped over
// [spec:pgorm:def:entity.traits.column+7/test]    the namespace `json_key` names,
// beside the SQL one `as_str` names
#[test]
fn json_key_reports_the_serde_key() {
    // Renamed on neither side, so the two namespaces agree.
    assert_eq!(filling::Column::Name.as_str(), "name");
    assert_eq!(filling::Column::Name.json_key(), "name");

    // `column_name` moves the SQL name alone; `enum_name` moves the variant, and
    // with it the SQL name that falls out of the variant — neither reaches the key.
    assert_eq!(filling::Column::Sku.as_str(), "SKU");
    assert_eq!(filling::Column::Sku.json_key(), "sku");
    assert_eq!(filling::Column::Renamed.as_str(), "renamed");
    assert_eq!(filling::Column::Renamed.json_key(), "original");

    // `serde`'s renames move the key alone, in either spelling.
    assert_eq!(serde_keys::Column::BakedOn.as_str(), "baked_on");
    assert_eq!(serde_keys::Column::BakedOn.json_key(), "bakedOn");
    assert_eq!(serde_keys::Column::Note.as_str(), "note");
    assert_eq!(serde_keys::Column::Note.json_key(), "NOTE");
    assert_eq!(serde_keys::Column::Sku.as_str(), "SKU");
    assert_eq!(serde_keys::Column::Sku.json_key(), "sku");

    // And each computed key is the key `serde` actually reads: an object keyed
    // by `json_key` deserializes into the model, which is the whole point of
    // computing it.
    let mut object: serde_json::Map<String, serde_json::Value> = serde_keys::Column::iter()
        .map(|col| (col.json_key().to_owned(), serde_json::json!("x")))
        .collect();
    object["id"] = serde_json::json!(1);
    let model: serde_keys::Model = serde_json::from_value(object.into()).unwrap();
    assert_eq!(
        model,
        serde_keys::Model {
            id: 1,
            baked_on: "x".to_owned(),
            note: "x".to_owned(),
            sku: Some("x".to_owned()),
        }
    );
}

// [spec:pgorm:syn:macros.derive.entity-model.attrs+6/test]    `table_iden` adds a Table variant
#[test]
fn table_iden_variant_is_skipped_by_enum_iter() {
    // The variant exists...
    assert_eq!(filling::Column::Table.to_string(), "table");
    // ...but `#[strum(disabled)]` keeps it out of iteration.
    assert!(
        !filling::Column::iter()
            .map(|c| c.to_string())
            .any(|c| c == "table")
    );
}

// [spec:pgorm:syn:macros.derive.entity-model.attrs+6/test]    the Table variant has no column def
#[test]
#[should_panic(expected = "Table cannot be used as a column")]
fn table_iden_variant_has_no_column_def() {
    let _ = filling::Column::Table.def();
}

// [spec:pgorm:syn:macros.derive.entity-model.attrs+6/test]    field-level keys
#[test]
fn field_level_attributes_shape_the_column_defs() {
    use pgorm::ColumnTypeTrait;

    // bare `primary_key`
    assert_eq!(filling::Column::Id.def(), ColumnType::Integer.def());
    // `column_type` + `unique` + `indexed` + `comment`
    assert_eq!(
        filling::Column::Name.def(),
        ColumnType::Text
            .def()
            .unique()
            .indexed()
            .comment("display name")
    );
    // bare `nullable` + `default_value`
    assert_eq!(
        filling::Column::Weight.def(),
        ColumnType::Integer.def().nullable().default_value(0)
    );
    // `default_expr`
    assert_eq!(
        filling::Column::BakedOn.def(),
        ColumnType::Date.def().default(Expr::current_date())
    );
    // `column_name`
    assert_eq!(filling::Column::Sku.to_string(), "SKU");
    // `enum_name`
    assert_eq!(filling::Column::Renamed.to_string(), "renamed");
    // `ignore` keeps the field out of `Column` entirely; a field carrying no
    // attribute at all is an ordinary column.
    assert_eq!(filling::Column::Plain.def(), ColumnType::Integer.def());
}

// [spec:pgorm:sem:macros.derive.entity-model.column-def+9/test]    the Rust-type inference table
#[test]
fn column_types_inferred_from_rust_type_name() {
    use pgorm::ColumnTypeTrait;
    use scalars::Column as C;

    assert_eq!(C::Txt.def(), ColumnType::string(None).def());
    assert_eq!(C::Small.def(), ColumnType::SmallInteger.def());
    assert_eq!(C::Id.def(), ColumnType::Integer.def());
    assert_eq!(C::Big.def(), ColumnType::BigInteger.def());
    assert_eq!(C::Single.def(), ColumnType::Float.def());
    assert_eq!(C::Dbl.def(), ColumnType::Double.def());
    assert_eq!(C::Flag.def(), ColumnType::Boolean.def());
    assert_eq!(C::D.def(), ColumnType::Date.def());
    assert_eq!(C::T.def(), ColumnType::Time.def());
    assert_eq!(C::Dt.def(), ColumnType::Timestamp.def());
    assert_eq!(C::Dttz.def(), ColumnType::TimestampWithTimeZone.def());
    assert_eq!(C::Uid.def(), ColumnType::Uuid.def());
    assert_eq!(C::Js.def(), ColumnType::Json.def());
    assert_eq!(C::Dec.def(), ColumnType::Decimal(None).def());
    assert_eq!(C::Blob.def(), ColumnType::Bytea.def());

    // `Option<T>` is unwrapped to `T` and forces nullability.
    assert_eq!(C::Maybe.def(), ColumnType::BigInteger.def().nullable());

    // Anything else resolves through `<T as ValueType>::column_type()`.
    assert_eq!(
        C::Tea.def(),
        <scalars::Tea as pgorm::pgorm_query::ValueType>::column_type().def()
    );
    assert_eq!(C::Tea.def(), ColumnType::String(StringLen::N(1)).def());
}

// [spec:pgorm:sem:macros.derive.entity-model.column-def+9/test]    explicit column_type wins
#[test]
fn an_explicit_column_type_overrides_the_inferred_one() {
    // `name: String` would infer `string(None)`; the attribute pins `Text`.
    assert_ne!(
        filling::Column::Name.def().get_column_type(),
        &ColumnType::string(None)
    );
    assert_eq!(
        filling::Column::Name.def().get_column_type(),
        &ColumnType::Text
    );
}

// [spec:pgorm:sem:macros.derive.entity-model.casing+3/test]
#[test]
fn column_variant_names_follow_the_casing_rules() {
    // Raw identifier trimmed, then UpperCamelCase.
    assert_eq!(casing::Column::Type.to_string(), "type");
    // Escaped keyword: `SELF` upper-camel-cases to `Self`, which gets a
    // trailing underscore to stay a legal identifier.
    assert_eq!(casing::Column::Self_.to_string(), "self");
    // `enum_name` replaces the variant name entirely.
    assert_eq!(casing::Column::Wholly.to_string(), "wholly");
}

// [spec:pgorm:sem:macros.derive.entity-model.casing+3/test]    when the SQL name is pinned
#[test]
fn sql_column_names_are_pinned_only_when_needed() {
    // Clean snake_case: no attribute, the name falls out of the `Column` derive.
    assert_eq!(casing::Column::FirstName.to_string(), "first_name");
    // Does not survive the round trip, so the original snake_case is pinned
    // (`Abc1` would otherwise render as `abc1`).
    assert_eq!(casing::Column::Abc1.to_string(), "abc_1");
    // With `rename_all`, the variant name is converted per the case style...
    assert_eq!(renamed::Column::FirstName.to_string(), "FIRST_NAME");
    // ...unless an explicit `column_name` wins.
    assert_eq!(renamed::Column::SecondName.to_string(), "explicit");
}

// [spec:pgorm:sem:macros.derive.entity-model.primary-key+7/test]
#[test]
fn primary_key_value_type_and_auto_increment() {
    // A single key contributes a bare type...
    let _: <single_pk::PrimaryKey as PrimaryKeyTrait>::ValueType = 1i32;
    assert!(single_pk::PrimaryKey::auto_increment());

    // ...composite keys contribute a tuple, and disable auto-increment.
    let _: <composite_pk::PrimaryKey as PrimaryKeyTrait>::ValueType = (1i32, 2i32);
    assert!(!composite_pk::PrimaryKey::auto_increment());
    let keys: Vec<String> = composite_pk::PrimaryKey::iter()
        .map(|k| k.to_string())
        .collect();
    assert_eq!(keys, vec!["cake_id", "filling_id"]);

    // The flag is shared across the whole struct: `auto_increment = false` on a
    // field that is not a primary key still flips it globally.
    assert!(!shared_auto_increment::PrimaryKey::auto_increment());
}

// [spec:pgorm:sem:macros.derive.entity-model.primary-key+7/test]    `without_overlaps` on the
// key's last field makes the key temporal, its period part of the key value; a plain key is not
#[test]
fn without_overlaps_marks_the_period() {
    let _: <temporal_key::PrimaryKey as PrimaryKeyTrait>::ValueType = (1i32, Range::<Date>::Empty);
    assert!(temporal_key::PrimaryKey::without_overlaps());
    assert!(!temporal_key::PrimaryKey::auto_increment());
    assert!(!composite_pk::PrimaryKey::without_overlaps());
    assert!(!single_pk::PrimaryKey::without_overlaps());
}

// [spec:pgorm:sem:macros.derive.entity-model.primary-key+7/test]    an identity
// column inside a composite key: its def carries the identity, the other key
// column's does not, and the key is not generated whole
#[test]
fn identity_inside_a_composite_key() {
    let _: <tenant_identity::PrimaryKey as PrimaryKeyTrait>::ValueType = (1i32, 2i64);
    assert!(!tenant_identity::PrimaryKey::auto_increment());
    assert_eq!(
        tenant_identity::Column::TenantId.def(),
        ColumnType::Integer.def()
    );
    assert_eq!(
        tenant_identity::Column::Id.def(),
        ColumnType::BigInteger.def().identity()
    );
    assert_eq!(
        tenant_identity::Column::Seq.def(),
        ColumnType::SmallInteger.def().identity_by_default()
    );
}

// [spec:pgorm:sem:macros.derive.entity-model.primary-key+7/test]    the key is
// generated whole when every key column is an identity, whatever its arity and
// whatever a non-key field says about `auto_increment`
#[test]
fn every_identity_key_column_generates_the_key() {
    assert!(all_identity::PrimaryKey::auto_increment());
    assert_eq!(
        all_identity::Column::A.def(),
        ColumnType::Integer.def().identity()
    );
    assert!(single_identity::PrimaryKey::auto_increment());
    assert_eq!(
        single_identity::Column::Id.def(),
        ColumnType::Integer.def().identity()
    );
}

// [spec:pgorm:sem:macros.derive.entity-model.column-def+9/test]    each generated key
// becomes the column's generation expression of its kind, after the modifiers it
// sits beside, and a stored generated column alone in the key generates the key
#[test]
fn generated_keys_carry_their_kind() {
    use generated_columns::Column;
    use pgorm::pgorm_query::GeneratedKind;

    assert!(generated_columns::PrimaryKey::auto_increment());
    assert_eq!(
        Column::Id.def(),
        ColumnType::Integer
            .def()
            .generated(Expr::col(Column::Base).mul(10), GeneratedKind::Stored)
    );
    assert_eq!(Column::Base.def(), ColumnType::Integer.def());
    assert_eq!(
        Column::Next.def(),
        ColumnType::Integer
            .def()
            .generated(Expr::col(Column::Base).add(1), GeneratedKind::Virtual)
    );
    assert_eq!(
        Column::Doubled.def(),
        ColumnType::Integer
            .def()
            .nullable()
            .generated(Expr::col(Column::Base).mul(2), GeneratedKind::Virtual)
    );
}

// [spec:pgorm:sem:macros.derive.entity-model.primary-key+7/test]    the names and column
// mapping the composite writes for its `PrimaryKey`
#[test]
fn primary_key_emits_iden_and_mapping() {
    // `StaticName` maps the variant to its snake_case name, or a `column_name`
    // override; `SqlName` delegates to it.
    assert_eq!(pgorm::StaticName::as_str(&single_pk::PrimaryKey::Id), "ID");
    assert_eq!(single_pk::PrimaryKey::Id.to_string(), "ID");
    assert_eq!(
        pgorm::StaticName::as_str(&composite_pk::PrimaryKey::CakeId),
        "cake_id"
    );

    // `PrimaryKeyToColumn` maps to the same-named variants of `Column`.
    assert!(matches!(
        single_pk::PrimaryKey::Id.into_column(),
        single_pk::Column::Id
    ));
    assert!(matches!(
        single_pk::PrimaryKey::from_column(single_pk::Column::Id),
        Some(single_pk::PrimaryKey::Id)
    ));
    assert!(
        single_pk::PrimaryKey::from_column(single_pk::Column::Name).is_none(),
        "a non-key column has no PrimaryKey variant"
    );
}

// [spec:pgorm:sem:macros.derive.entity+5/test]    EntityTrait associated types and the SqlName pair
#[test]
fn derive_entity_wires_up_the_entity_trait() {
    // The five associated types default to the conventional names.
    let _: <filling::Entity as EntityTrait>::Model = filling::Model {
        id: 1,
        name: String::new(),
        vendor_id: None,
        weight: 0,
        baked_on: Date::new(2024, 1, 1).unwrap(),
        sku: String::new(),
        original: 0,
        casted: String::new(),
        not_a_column: 0,
        plain: 0,
    };
    let _: <filling::Entity as EntityTrait>::ActiveModel =
        <filling::ActiveModel as Default>::default();
    let _: <filling::Entity as EntityTrait>::Column = filling::Column::Id;
    let _: <filling::Entity as EntityTrait>::PrimaryKey = filling::PrimaryKey::Id;
    fn assert_relation<E: EntityTrait<Relation = filling::Relation>>() {}
    assert_relation::<filling::Entity>();

    // `SqlName` and `StaticName` both render `EntityName::table_name`, and the
    // `SqlName` answers `EntityName::schema_name` as its table's schema.
    assert_eq!(pgorm::StaticName::as_str(&filling::Entity), "filling");
    assert_eq!(filling::Entity.to_string(), "filling");
    assert_eq!(
        pgorm::SqlName::table_schema(&filling::Entity),
        Some("baking")
    );
}

// [spec:pgorm:req:macros.derive.one-form/test]    an entity whose name is computed keeps
// its `Entity` and `EntityName` by hand, beside a composite derive given no `table_name`
// [spec:pgorm:sem:macros.derive.entity+5/test]    the entity-trait derive writes no EntityName
#[test]
fn derive_entity_leaves_entity_name_to_hand() {
    // `no_table_name::Entity` carries the `EntityTrait` derive and a hand-written
    // `EntityName` impl right beside it. If the derive had emitted its own
    // `EntityName` this module would not compile at all.
    assert_eq!(no_table_name::Entity.table_name(), "hand_written");
    assert_eq!(no_table_name::Entity.schema_name(), None);
    assert_eq!(no_table_name::Entity.to_string(), "hand_written");
    assert_eq!(pgorm::SqlName::table_schema(&no_table_name::Entity), None);
    // The composite still produced Column / PrimaryKey without an Entity struct
    // of its own.
    assert_eq!(no_table_name::Column::Id.to_string(), "id");
}
